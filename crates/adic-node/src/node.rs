use crate::config::NodeConfig;
use crate::events::{
    AxisData, BalanceChangeType, EventBus, FinalityType, NodeEvent, RejectionType,
};
use crate::genesis::{account_address_from_hex, GenesisManifest};
use crate::metrics::Metrics;
use crate::update_manager;
use crate::wallet::NodeWallet;
use crate::wallet_registry::WalletRegistry;
use adic_consensus::{ConsensusEngine, ReputationChangeEvent};
use adic_crypto::{BLSThresholdSigner, Keypair, ThresholdConfig};
use adic_economics::balance::{BalanceChangeEvent, BalanceChangeTypeEvent};
use adic_economics::{AccountAddress, AdicAmount, EconomicsEngine};
use adic_finality::{FinalityConfig, FinalityEngine};
use adic_mrw::MrwEngine;
use adic_network::peer::PeerEvent;
use adic_network::sync::SyncEvent;
use adic_network::{NetworkConfig, NetworkEngine};
use adic_pouw::{BLSCoordinator, BLSCoordinatorConfig, DKGManager, DKGOrchestrator, DKGOrchestratorConfig};
use adic_storage::store::BackendType;
use adic_storage::{MessageIndex, StorageConfig, StorageEngine, TipManager};
use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AxisId, AxisPhi, EncoderData, EncoderSet, MessageId,
    PublicKey, QpDigits,
};
use adic_vrf::{VRFService, VRFConfig};
use adic_app_common::{NetworkMetadataRegistry, ParameterStore};
use anyhow::Result;
use chrono::Utc;
use libp2p::identity::Keypair as LibP2pKeypair;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

pub struct AdicNode {
    config: NodeConfig,
    wallet: Arc<NodeWallet>,
    pub wallet_registry: Arc<WalletRegistry>,
    keypair: Keypair,
    pub consensus: Arc<ConsensusEngine>,
    pub mrw: Arc<MrwEngine>,
    pub storage: Arc<StorageEngine>,
    pub finality: Arc<FinalityEngine>,
    pub economics: Arc<EconomicsEngine>,
    pub network: Option<Arc<RwLock<NetworkEngine>>>,
    pub update_manager: Option<Arc<update_manager::UpdateManager>>,
    pub event_bus: Arc<EventBus>,
    pub vrf_service: Arc<VRFService>,
    pub quorum_selector: Arc<adic_quorum::QuorumSelector>,
    pub parameter_store: Option<Arc<ParameterStore>>,
    pub metrics: Arc<Metrics>,
    /// Network metadata registry for node ASN/region tracking
    pub network_metadata: Arc<NetworkMetadataRegistry>,
    /// DKG manager for distributed key generation
    pub dkg_manager: Option<Arc<DKGManager>>,
    /// BLS coordinator for threshold signatures
    pub bls_coordinator: Option<Arc<BLSCoordinator>>,
    /// DKG orchestrator for network-aware DKG ceremonies
    pub dkg_orchestrator: Option<Arc<DKGOrchestrator>>,

    // ========== Application Layer ==========
    /// PoUW task manager for useful work coordination
    pub task_manager: Option<Arc<adic_pouw::TaskManager>>,
    /// Governance lifecycle manager for proposals and voting
    pub governance_manager: Option<Arc<adic_governance::ProposalLifecycleManager>>,
    /// Governance network protocol handler
    pub governance_protocol: Option<Arc<adic_network::protocol::GovernanceProtocol>>,
    /// Storage market coordinator for decentralized storage deals
    pub storage_market: Option<Arc<adic_storage_market::StorageMarketCoordinator>>,

    index: Arc<MessageIndex>,
    tip_manager: Arc<TipManager>,
    running: Arc<RwLock<bool>>,
    /// Feature encoders for all 4 axes per ADIC-DAG paper Appendix A
    encoders: Arc<EncoderSet>,
    /// Current epoch for VRF commit-reveal protocol
    current_epoch: Arc<RwLock<u64>>,
}

impl AdicNode {
    pub async fn new(config: NodeConfig) -> Result<Self> {
        info!("Initializing ADIC node...");

        // Load or create wallet
        let data_dir = config.node.data_dir.clone();
        let node_id = config.node.name.clone();
        let wallet = Arc::new(NodeWallet::load_or_create(&data_dir, &node_id)?);

        // Get keypair from wallet
        let keypair = wallet.keypair().clone();

        info!(
            address = %hex::encode(wallet.address().as_bytes()),
            public_key = %hex::encode(wallet.public_key().as_bytes()),
            "🔐 Node wallet loaded"
        );

        // Initialize metrics
        let metrics = Arc::new(Metrics::new());
        info!("📊 Metrics initialized for Prometheus");

        // Initialize event bus early for real-time event streaming
        let mut event_bus_instance = EventBus::new();
        event_bus_instance.set_metrics(Arc::new(metrics.events_emitted_total.clone()));
        let event_bus = Arc::new(event_bus_instance);
        info!("📡 Event bus initialized for WebSocket/SSE streaming");

        // Create storage engine using configured backend
        let storage_config = StorageConfig {
            backend_type: match config.storage.backend.as_str() {
                "rocksdb" => {
                    #[cfg(feature = "rocksdb")]
                    {
                        BackendType::RocksDB {
                            path: config
                                .node
                                .data_dir
                                .join("storage")
                                .to_string_lossy()
                                .to_string(),
                        }
                    }
                    #[cfg(not(feature = "rocksdb"))]
                    {
                        warn!(
                            requested_backend = "rocksdb",
                            fallback_backend = "memory",
                            "⚠️ RocksDB feature not enabled, using memory backend"
                        );
                        BackendType::Memory
                    }
                }
                "memory" => BackendType::Memory,
                _ => {
                    warn!(
                        unknown_backend = %config.storage.backend,
                        fallback_backend = "memory",
                        "⚠️ Unknown storage backend, using memory"
                    );
                    BackendType::Memory
                }
            },
            cache_size: config.storage.cache_size,
            flush_interval_ms: config.storage.snapshot_interval * 1000,
            max_batch_size: 100,
        };

        let storage = Arc::new(StorageEngine::new(storage_config)?);

        // Create consensus engine
        let adic_params = config.adic_params();
        let mut consensus_engine = ConsensusEngine::new(adic_params.clone(), storage.clone());

        // Wire metrics to deposit manager
        consensus_engine.deposits.set_metrics(
            Arc::new(metrics.escrow_locks_total.clone()),
            Arc::new(metrics.escrow_releases_total.clone()),
            Arc::new(metrics.escrow_slashes_total.clone()),
            Arc::new(metrics.escrow_refunds_total.clone()),
            Arc::new(metrics.escrow_lock_duration.clone()),
            Arc::new(metrics.escrow_locked_amount.clone()),
        );

        // Wire metrics to admissibility checker
        consensus_engine.admissibility.set_metrics(
            Arc::new(metrics.admissibility_checks_total.clone()),
            Arc::new(metrics.admissibility_s_failures.clone()),
            Arc::new(metrics.admissibility_c2_failures.clone()),
            Arc::new(metrics.admissibility_c3_failures.clone()),
        );

        // Wire metrics to message validator
        consensus_engine.validator.set_metrics(
            Arc::new(metrics.signature_verifications.clone()),
            Arc::new(metrics.signature_failures.clone()),
        );

        let consensus = Arc::new(consensus_engine);

        // Register reputation event callback to bridge consensus events to node events
        {
            let event_bus_clone = event_bus.clone();
            consensus
                .reputation
                .set_event_callback(Arc::new(move |reputation_event: ReputationChangeEvent| {
                    event_bus_clone.emit(NodeEvent::ReputationChanged {
                        validator_address: hex::encode(
                            reputation_event.validator_pubkey.as_bytes(),
                        ),
                        old_reputation: reputation_event.old_reputation,
                        new_reputation: reputation_event.new_reputation,
                        reason: reputation_event.reason,
                        timestamp: Utc::now(),
                    });
                }))
                .await;
        }

        // Create MRW engine
        let mut mrw_engine = MrwEngine::new(adic_params.clone());

        // Wire metrics to MRW engine
        mrw_engine.set_metrics(
            Arc::new(metrics.mrw_attempts_total.clone()),
            Arc::new(metrics.mrw_widens_total.clone()),
            Arc::new(metrics.mrw_selection_duration.clone()),
        );

        let mrw = Arc::new(mrw_engine);

        // Create finality engine
        let finality_config = FinalityConfig::from(&adic_params);
        let mut finality_engine = FinalityEngine::new(
            finality_config,
            consensus.clone(),
            storage.clone(),
        ).await;

        // Wire metrics to finality engine
        finality_engine.set_metrics(
            Arc::new(metrics.finalizations_total.clone()),
            Arc::new(metrics.kcore_size.clone()),
            Arc::new(metrics.finality_depth.clone()),
            Arc::new(metrics.f2_computation_time.clone()),
            Arc::new(metrics.f2_timeout_count.clone()),
            Arc::new(metrics.f1_fallback_count.clone()),
            Arc::new(metrics.finality_method_used.clone()),
        );

        let finality = Arc::new(finality_engine);

        // Create VRF service for commit-reveal protocol
        let reveal_depth = (adic_params.depth_star + 5) as u64; // D* + δ

        // Derive genesis root deterministically from chain ID
        // This provides a canonical root for VRF commit-reveal protocol
        // In production with persistent storage, this could be loaded from genesis config
        let genesis_root = *blake3::hash(b"adic-dag-genesis-v1").as_bytes();

        let vrf_config = VRFConfig {
            min_committer_reputation: 50.0,
            reveal_depth,
            genesis_root,
        };
        let mut vrf_service_instance = VRFService::new(
            vrf_config,
            Arc::new(consensus.reputation.clone()),
        );
        vrf_service_instance.set_metrics(
            Arc::new(metrics.vrf_commits_total.clone()),
            Arc::new(metrics.vrf_reveals_total.clone()),
            Arc::new(metrics.vrf_finalizations_total.clone()),
            Arc::new(metrics.vrf_commit_duration.clone()),
            Arc::new(metrics.vrf_reveal_duration.clone()),
            Arc::new(metrics.vrf_finalize_duration.clone()),
        );
        let vrf_service = Arc::new(vrf_service_instance);
        info!("🎲 VRF service initialized with reveal depth {}", reveal_depth);

        // Create quorum selector for VRF-based committee selection
        let mut quorum_selector_instance = adic_quorum::QuorumSelector::new(
            Arc::clone(&vrf_service),
            Arc::new(consensus.reputation.clone()),
        );
        quorum_selector_instance.set_metrics(
            Arc::new(metrics.quorum_selections_total.clone()),
            Arc::new(metrics.quorum_selection_duration.clone()),
            Arc::new(metrics.quorum_committee_size.clone()),
        );
        let quorum_selector = Arc::new(quorum_selector_instance);
        info!("🗳️ Quorum selector initialized");

        // Create network metadata registry for tracking node ASN/region
        let network_metadata = Arc::new(NetworkMetadataRegistry::new());
        info!("🌐 Network metadata registry initialized");

        // Create wallet registry
        let wallet_registry = Arc::new(WalletRegistry::new(storage.clone()));
        wallet_registry.load_from_storage().await?;

        // Create economics engine
        let economics_storage = Arc::new(adic_economics::storage::MemoryStorage::new());
        let economics = Arc::new(EconomicsEngine::new(economics_storage.clone()).await?);

        // Register balance event callback to bridge economics events to node events
        {
            let event_bus_clone = event_bus.clone();
            economics
                .balances
                .set_event_callback(Arc::new(move |balance_event: BalanceChangeEvent| {
                    tracing::info!(
                        address = ?balance_event.address,
                        balance_before = %balance_event.balance_before.to_adic(),
                        balance_after = %balance_event.balance_after.to_adic(),
                        change_amount = %balance_event.change_amount.to_adic(),
                        change_type = ?balance_event.change_type,
                        "💰 Balance changed - emitting event"
                    );
                    event_bus_clone.emit(NodeEvent::BalanceChanged {
                        address: hex::encode(balance_event.address.as_bytes()),
                        balance_before: balance_event.balance_before.to_adic().to_string(),
                        balance_after: balance_event.balance_after.to_adic().to_string(),
                        change_amount: balance_event.change_amount.to_adic().to_string(),
                        change_type: match balance_event.change_type {
                            BalanceChangeTypeEvent::Credit => BalanceChangeType::Credit,
                            BalanceChangeTypeEvent::Debit => BalanceChangeType::Debit,
                            BalanceChangeTypeEvent::TransferIn => BalanceChangeType::TransferIn,
                            BalanceChangeTypeEvent::TransferOut => BalanceChangeType::TransferOut,
                        },
                        timestamp: Utc::now(),
                    });
                }))
                .await;
        }

        // Register balance transfer event callback to emit TransferRecorded events
        {
            let event_bus_clone = event_bus.clone();
            economics
                .balances
                .set_transfer_event_callback(Arc::new(
                    move |transfer_event: adic_economics::types::TransferEvent| {
                        tracing::info!(
                            from = %transfer_event.from,
                            to = %transfer_event.to,
                            amount = %transfer_event.amount.to_adic(),
                            reason = ?transfer_event.reason,
                            "💸 Transfer recorded (balances) - emitting event"
                        );
                        event_bus_clone.emit(NodeEvent::TransferRecorded {
                            from_address: hex::encode(transfer_event.from.as_bytes()),
                            to_address: hex::encode(transfer_event.to.as_bytes()),
                            amount: transfer_event.amount.to_adic().to_string(),
                            reason: format!("{:?}", transfer_event.reason),
                            tx_hash: None,
                            timestamp: chrono::DateTime::from_timestamp(
                                transfer_event.timestamp,
                                0,
                            )
                            .unwrap_or_else(Utc::now),
                        });
                    },
                ))
                .await;
        }

        // Register supply transfer event callback to emit TransferRecorded events
        {
            let event_bus_clone = event_bus.clone();
            economics
                .supply
                .set_event_callback(Arc::new(
                    move |transfer_event: adic_economics::types::TransferEvent| {
                        tracing::info!(
                            from = %transfer_event.from,
                            to = %transfer_event.to,
                            amount = %transfer_event.amount.to_adic(),
                            reason = ?transfer_event.reason,
                            "💸 Transfer recorded - emitting event"
                        );
                        event_bus_clone.emit(NodeEvent::TransferRecorded {
                            from_address: hex::encode(transfer_event.from.as_bytes()),
                            to_address: hex::encode(transfer_event.to.as_bytes()),
                            amount: transfer_event.amount.to_adic().to_string(),
                            reason: format!("{:?}", transfer_event.reason),
                            tx_hash: None, // Will be generated by backend if needed
                            timestamp: chrono::DateTime::from_timestamp(
                                transfer_event.timestamp,
                                0,
                            )
                            .unwrap_or_else(Utc::now),
                        });
                    },
                ))
                .await;
        }

        // Only bootstrap nodes should create genesis
        // Other nodes will sync genesis from the network
        let is_bootstrap = config.node.bootstrap.unwrap_or(false);

        if is_bootstrap {
            info!("🚀 Bootstrap node: Initializing genesis state");

            // Initialize genesis in economics engine
            economics
                .initialize_genesis()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to initialize economics genesis: {}", e))?;

            // Apply genesis allocations on first startup
            // Check if genesis has been applied by looking for genesis marker file
            let genesis_marker = config.node.data_dir.join(".genesis_applied");
            let genesis_applied = genesis_marker.exists();

            if !genesis_applied {
                // Use genesis from config if provided, otherwise use default
                let genesis_config = config.genesis.clone().unwrap_or_default();

                // Get network type for security logging
                let network_type = genesis_config.network_type();

                // SECURITY: Use enhanced validation with canonical hash enforcement
                // This prevents the attack where someone modifies TOML allocations or uses
                // a different chain_id to bypass canonical hash validation
                genesis_config
                    .verify_with_canonical(true) // true = is_bootstrap
                    .map_err(|e| anyhow::anyhow!("Genesis validation failed: {}", e))?;

                // Calculate genesis hash for logging
                let genesis_hash = genesis_config.calculate_hash();

                // Log security information
                match network_type {
                    crate::genesis::NetworkType::Mainnet => {
                        info!(
                            "🔒 MAINNET bootstrap: Using canonical genesis (hash verified)"
                        );
                    }
                    crate::genesis::NetworkType::Testnet => {
                        info!("🧪 TESTNET bootstrap: Using custom genesis configuration");

                        // Production guard: Block testnet in release builds
                        #[cfg(all(not(debug_assertions), not(test)))]
                        {
                            anyhow::bail!(
                                "PRODUCTION ERROR: Cannot run TESTNET in release builds. \
                                Use mainnet genesis or build in debug mode for testing."
                            );
                        }
                    }
                    crate::genesis::NetworkType::Devnet => {
                        info!("🛠️  DEVNET bootstrap: Using custom genesis configuration");

                        // Production guard: Block devnet in release builds
                        #[cfg(all(not(debug_assertions), not(test)))]
                        {
                            anyhow::bail!(
                                "PRODUCTION ERROR: Cannot run DEVNET in release builds. \
                                Use mainnet genesis or build in debug mode for development."
                            );
                        }
                    }
                    crate::genesis::NetworkType::Custom => {
                        warn!(
                            "⚠️  CUSTOM NETWORK bootstrap: Using non-standard chain_id '{}'. \
                             This network will not be compatible with official ADIC networks.",
                            genesis_config.chain_id
                        );

                        // Production guard: Warn strongly about custom networks in release builds
                        #[cfg(all(not(debug_assertions), not(test)))]
                        {
                            error!(
                                "⚠️  WARNING: Running CUSTOM NETWORK '{}' in production release build! \
                                This is not a standard ADIC network and may not be compatible with official infrastructure.",
                                genesis_config.chain_id
                            );
                        }
                    }
                }

                // Apply allocations
                let balance_manager = &economics.balances;
                for (address_hex, amount_adic) in genesis_config.allocations.iter() {
                    let address = account_address_from_hex(address_hex).map_err(|e| {
                        anyhow::anyhow!("Invalid genesis address {}: {}", address_hex, e)
                    })?;
                    let amount = AdicAmount::from_adic(*amount_adic as f64);

                    balance_manager.credit(address, amount).await.map_err(|e| {
                        anyhow::anyhow!("Failed to credit genesis allocation: {}", e)
                    })?;

                    debug!(
                        address = &address_hex[..16.min(address_hex.len())],
                        amount_adic = *amount_adic,
                        "Genesis allocation credited"
                    );
                }

                // Create and save genesis manifest from the config we used
                let genesis_manifest = GenesisManifest {
                    config: genesis_config.clone(),
                    hash: genesis_hash.clone(),
                    version: "1.0.0".to_string(),
                };
                let genesis_path = config.node.data_dir.join("genesis.json");
                let genesis_json = serde_json::to_string_pretty(&genesis_manifest)?;
                std::fs::write(&genesis_path, genesis_json)?;

                // Mark genesis as applied
                std::fs::write(
                    &genesis_marker,
                    format!(
                        "hash: {}\ntimestamp: {}\n",
                        genesis_manifest.hash,
                        chrono::Utc::now().to_rfc3339()
                    ),
                )?;

                info!(
                    genesis_hash = %genesis_manifest.hash,
                    total_supply_adic = genesis_config.total_supply().to_adic(),
                    allocation_count = genesis_config.allocations.len(),
                    "✅ Genesis manifest created and applied"
                );

                // Emit economics updated event after genesis
                let total_supply = economics.get_total_supply().await;
                let circulating_supply = economics.get_circulating_supply().await;
                let treasury_balance = economics
                    .treasury
                    .get_treasury_balance()
                    .await
                    .unwrap_or(AdicAmount::ZERO);

                event_bus.emit(NodeEvent::EconomicsUpdated {
                    total_supply: total_supply.to_adic().to_string(),
                    circulating_supply: circulating_supply.to_adic().to_string(),
                    treasury_balance: treasury_balance.to_adic().to_string(),
                    timestamp: Utc::now(),
                });
            } else {
                info!(
                    genesis_applied = true,
                    marker_exists = true,
                    "🧬 Genesis already applied, skipping"
                );
            }
        } else {
            info!("📡 Non-bootstrap node: Checking for genesis");

            // Non-bootstrap nodes must have genesis provided
            let genesis_path = config.node.data_dir.join("genesis.json");
            if !genesis_path.exists() {
                return Err(anyhow::anyhow!(
                    "Non-bootstrap node requires genesis.json. Please obtain genesis from bootstrap node or network."
                ));
            }

            // Load and verify genesis
            info!("Loading genesis from {:?}", genesis_path);
            let genesis_data = std::fs::read_to_string(&genesis_path)?;
            let genesis_manifest: GenesisManifest = serde_json::from_str(&genesis_data)
                .map_err(|e| anyhow::anyhow!("Failed to parse genesis: {}", e))?;

            // Verify genesis integrity
            genesis_manifest
                .verify()
                .map_err(|e| anyhow::anyhow!("Invalid genesis manifest: {}", e))?;

            // SECURITY: Use enhanced validation with canonical hash enforcement
            genesis_manifest.config
                .verify_with_canonical(false) // false = not bootstrap
                .map_err(|e| anyhow::anyhow!("Genesis validation failed: {}", e))?;

            // Get network type for logging
            let network_type = genesis_manifest.config.network_type();

            match network_type {
                crate::genesis::NetworkType::Mainnet => {
                    info!(
                        genesis_hash = %genesis_manifest.hash,
                        chain_id = %genesis_manifest.config.chain_id,
                        "✅ Genesis loaded, verified against canonical hash"
                    );
                }
                crate::genesis::NetworkType::Custom => {
                    warn!(
                        genesis_hash = %genesis_manifest.hash,
                        chain_id = %genesis_manifest.config.chain_id,
                        "⚠️  Joining CUSTOM network with non-standard chain_id. \
                         Ensure this is intentional."
                    );
                }
                _ => {
                    info!(
                        genesis_hash = %genesis_manifest.hash,
                        chain_id = %genesis_manifest.config.chain_id,
                        "✅ Genesis loaded and verified"
                    );
                }
            }

            // Emit genesis loaded event
            event_bus.emit(NodeEvent::GenesisLoaded {
                chain_id: genesis_manifest.config.chain_id.clone(),
                genesis_hash: genesis_manifest.hash.clone(),
                total_supply: genesis_manifest.config.total_supply().to_adic().to_string(),
                timestamp: Utc::now(),
            });

            // Initialize economics with genesis state
            economics
                .initialize_genesis()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to initialize economics: {}", e))?;

            // Apply genesis allocations
            let balance_manager = &economics.balances;
            for (address_hex, amount_adic) in genesis_manifest.config.allocations.iter() {
                let address = account_address_from_hex(address_hex).map_err(|e| {
                    anyhow::anyhow!("Invalid genesis address {}: {}", address_hex, e)
                })?;
                let amount = AdicAmount::from_adic(*amount_adic as f64);

                balance_manager
                    .credit(address, amount)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to credit genesis allocation: {}", e))?;
            }

            info!("✅ Genesis state applied from manifest");

            // Emit economics updated event after genesis
            let total_supply = economics.get_total_supply().await;
            let circulating_supply = economics.get_circulating_supply().await;
            let treasury_balance = economics
                .treasury
                .get_treasury_balance()
                .await
                .unwrap_or(AdicAmount::ZERO);

            event_bus.emit(NodeEvent::EconomicsUpdated {
                total_supply: total_supply.to_adic().to_string(),
                circulating_supply: circulating_supply.to_adic().to_string(),
                treasury_balance: treasury_balance.to_adic().to_string(),
                timestamp: Utc::now(),
            });
        }

        // Log node wallet balance
        let wallet_balance = economics
            .balances
            .get_balance(wallet.address())
            .await
            .unwrap_or(AdicAmount::ZERO);
        info!(
            balance_adic = wallet_balance.to_adic(),
            address = %hex::encode(wallet.address().as_bytes()),
            "💎 Node wallet balance loaded"
        );

        // Create indices
        let index = Arc::new(MessageIndex::new());
        let tip_manager = Arc::new(TipManager::new());

        // Initialize network if enabled
        let (network, update_manager) = if config.network.enabled {
            info!("🌐 Initializing P2P network...");

            // Parse listen addresses
            let listen_addr = format!("/ip4/0.0.0.0/tcp/{}", config.network.p2p_port);
            let listen_addresses = vec![listen_addr
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid listen address: {}", e))?];

            // Parse bootstrap peers as multiaddrs
            let bootstrap_peers = config
                .network
                .bootstrap_peers
                .clone()
                .into_iter()
                .filter_map(|s| {
                    // Try to parse as multiaddr, or convert from host:port format
                    if s.starts_with("/") {
                        s.parse().ok()
                    } else {
                        // Convert "host:port" to "/ip4/host/tcp/port"
                        if let Ok(addr) = s.parse::<SocketAddr>() {
                            match addr {
                                SocketAddr::V4(v4) => {
                                    format!("/ip4/{}/tcp/{}", v4.ip(), v4.port()).parse().ok()
                                }
                                SocketAddr::V6(v6) => {
                                    format!("/ip6/{}/tcp/{}", v6.ip(), v6.port()).parse().ok()
                                }
                            }
                        } else {
                            None
                        }
                    }
                })
                .collect();

            let network_config = NetworkConfig {
                listen_addresses,
                bootstrap_peers,
                max_peers: config.network.max_peers,
                enable_metrics: false,
                data_dir: data_dir.clone(),
                transport: adic_network::TransportConfig {
                    quic_listen_addr: format!("0.0.0.0:{}", config.network.quic_port)
                        .parse()
                        .unwrap(),
                    use_production_tls: config.network.use_production_tls,
                    ca_cert_path: config.network.ca_cert_path.clone(),
                    node_cert_path: config.network.node_cert_path.clone(),
                    node_key_path: config.network.node_key_path.clone(),
                    ..Default::default()
                },
                asn: config.network.asn,
                region: config.network.region.clone(),
                ..Default::default()
            };

            // Convert adic_crypto::Keypair to libp2p::identity::Keypair
            let libp2p_keypair = LibP2pKeypair::ed25519_from_bytes(&mut keypair.to_bytes().clone())
                .map_err(|e| anyhow::anyhow!("Failed to convert keypair: {}", e))?;

            let network = NetworkEngine::new(
                network_config,
                libp2p_keypair,
                storage.clone(),
                consensus.clone(),
                finality.clone(),
            )
            .await?;

            // Initialize update manager if network is available
            let network_arc = Arc::new(RwLock::new(network));

            // Bridge P2P peer events to node events and register nodes
            {
                let event_bus_clone = event_bus.clone();
                let network_clone = network_arc.clone();
                let _metadata_registry = network_metadata.clone();
                tokio::spawn(async move {
                    let net = network_clone.read().await;
                    let peer_stream = net.peer_manager().event_stream();
                    drop(net); // Release the read lock

                    loop {
                        let event_opt = {
                            let mut stream = peer_stream.write().await;
                            stream.recv().await
                        };

                        match event_opt {
                            Some(peer_event) => {
                                match peer_event {
                                    PeerEvent::PeerConnected(peer_id) => {
                                        // Get peer info to extract address and public key
                                        let (address, public_key_opt) = {
                                            let net = network_clone.read().await;
                                            if let Some(peer_info) =
                                                net.peer_manager().get_peer(&peer_id).await
                                            {
                                                let addr = peer_info
                                                    .addresses
                                                    .first()
                                                    .map(|addr| addr.to_string())
                                                    .unwrap_or_else(|| "unknown".to_string());
                                                (addr, Some(peer_info.public_key))
                                            } else {
                                                ("unknown".to_string(), None)
                                            }
                                        };

                                        // Node metadata (ASN/region) will be received via handshake
                                        // and registered when the handshake is processed
                                        if public_key_opt.is_some() {
                                            debug!(
                                                peer_id = %peer_id,
                                                "✅ Peer connected, awaiting handshake metadata"
                                            );
                                        }

                                        event_bus_clone.emit(NodeEvent::PeerConnected {
                                            peer_id: peer_id.to_string(),
                                            address,
                                            timestamp: Utc::now(),
                                        });
                                    }
                                    PeerEvent::PeerDisconnected(peer_id) => {
                                        event_bus_clone.emit(NodeEvent::PeerDisconnected {
                                            peer_id: peer_id.to_string(),
                                            reason: None,
                                            timestamp: Utc::now(),
                                        });
                                    }
                                    _ => {} // Ignore other peer events for now
                                }
                            }
                            None => break, // Channel closed
                        }
                    }
                });
            }

            // Bridge sync events to node events
            {
                let event_bus_clone = event_bus.clone();
                let network_clone = network_arc.clone();
                tokio::spawn(async move {
                    let net = network_clone.read().await;
                    let sync_stream = net.state_sync().event_stream();
                    drop(net); // Release the read lock

                    loop {
                        let event_opt = {
                            let mut stream = sync_stream.write().await;
                            stream.recv().await
                        };

                        match event_opt {
                            Some(sync_event) => {
                                match sync_event {
                                    SyncEvent::SyncStarted(peer_id, target_height) => {
                                        event_bus_clone.emit(NodeEvent::SyncStarted {
                                            peer_id: peer_id.to_string(),
                                            target_height,
                                            timestamp: Utc::now(),
                                        });
                                    }
                                    SyncEvent::SyncProgress(peer_id, synced_messages, progress) => {
                                        event_bus_clone.emit(NodeEvent::SyncProgress {
                                            peer_id: peer_id.to_string(),
                                            synced_messages,
                                            progress_percent: progress * 100.0,
                                            timestamp: Utc::now(),
                                        });
                                    }
                                    SyncEvent::SyncCompleted(
                                        peer_id,
                                        synced_messages,
                                        duration_ms,
                                    ) => {
                                        event_bus_clone.emit(NodeEvent::SyncCompleted {
                                            peer_id: peer_id.to_string(),
                                            synced_messages,
                                            duration_ms,
                                            timestamp: Utc::now(),
                                        });
                                    }
                                    _ => {} // Ignore other sync events for now
                                }
                            }
                            None => break, // Channel closed
                        }
                    }
                });
            }

            let update_manager = if config.network.enabled {
                let network_clone = {
                    let net = network_arc.read().await;
                    net.clone()
                };

                let update_config = update_manager::UpdateConfig {
                    auto_update: config.network.auto_update,
                    dns_domain: "adic.network.adicl1.com".to_string(),
                    ..Default::default()
                };

                match update_manager::UpdateManager::new(
                    env!("CARGO_PKG_VERSION").to_string(),
                    Arc::new(network_clone),
                    data_dir.clone(),
                    update_config,
                ) {
                    Ok(manager) => {
                        info!("✅ Update manager initialized");
                        Some(Arc::new(manager))
                    }
                    Err(e) => {
                        warn!("Failed to initialize update manager: {}", e);
                        None
                    }
                }
            } else {
                None
            };

            (Some(network_arc), update_manager)
        } else {
            info!("P2P network disabled");
            (None, None)
        };

        // Initialize DKG components if network is enabled
        let (dkg_manager, bls_coordinator, dkg_orchestrator) = if network.is_some() {
            info!("🔐 Initializing DKG components for committee key generation...");

            // Create DKG manager
            let dkg_manager = Arc::new(DKGManager::new());

            // Create BLS coordinator (default committee size 15, threshold 10)
            let bls_config = BLSCoordinatorConfig::default();
            let threshold_config = ThresholdConfig::new(
                bls_config.total_members,
                bls_config.threshold,
            ).expect("Valid threshold config");
            let bls_signer = Arc::new(BLSThresholdSigner::new(threshold_config));
            let bls_coordinator = Arc::new(BLSCoordinator::with_dkg(
                bls_config,
                bls_signer,
                dkg_manager.clone(),
            ));

            // Create DKG orchestrator
            let orchestrator_config = DKGOrchestratorConfig {
                our_public_key: PublicKey::from_bytes(*wallet.public_key().as_bytes()),
            };
            let dkg_orchestrator = Arc::new(DKGOrchestrator::new(
                orchestrator_config,
                dkg_manager.clone(),
                bls_coordinator.clone(),
            ));

            info!("✅ DKG components initialized (BLS threshold 10-of-15)");

            // Start DKG message handling background task
            if let Some(ref _network_arc) = network {
                let _orchestrator_clone = dkg_orchestrator.clone();
                tokio::spawn(async move {
                    info!("🔐 DKG message handler started");
                    loop {
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                        // Check for DKG messages via the network's DKG protocol
                        // In production, this would be event-driven via the DKG protocol handler
                        // For now, this is a placeholder for the message handling loop

                        // The actual message handling is done through NetworkEngine's
                        // handle_dkg_message() which is called when DKG messages arrive
                    }
                });
            }

            (Some(dkg_manager), Some(bls_coordinator), Some(dkg_orchestrator))
        } else {
            info!("DKG components disabled (network not enabled)");
            (None, None, None)
        };

        // Initialize encoder set with all 4 axes
        let encoders = Arc::new(EncoderSet::default_full());
        info!("📊 Initialized encoder set: Time, Topic, Region, StakeTier");

        // Initialize parameter store for governance hot-reload
        let parameter_store = Arc::new(ParameterStore::new());
        info!("📝 Parameter store initialized for governance integration");

        // Bridge ParameterStore changes to EventBus for external subscribers
        {
            let event_bus_clone = event_bus.clone();
            let param_store_clone = parameter_store.clone();
            tokio::spawn(async move {
                let mut rx = param_store_clone.subscribe();
                loop {
                    match rx.recv().await {
                        Ok(change) => {
                            debug!(
                                key = %change.key,
                                old_value = %change.old_value,
                                new_value = %change.new_value,
                                "📝 Parameter change detected, emitting to EventBus"
                            );
                            event_bus_clone.emit(NodeEvent::ParameterChanged {
                                proposal_id: change.proposal_id
                                    .map(|id| hex::encode(&id[..8]))
                                    .unwrap_or_else(|| "direct".to_string()),
                                parameter_key: change.key.clone(),
                                old_value: change.old_value.to_string(),
                                new_value: change.new_value.to_string(),
                                timestamp: Utc::now(),
                            });
                        }
                        Err(e) => {
                            // Channel closed or lagged
                            if matches!(e, tokio::sync::broadcast::error::RecvError::Closed) {
                                debug!("ParameterStore channel closed, stopping event bridge");
                                break;
                            }
                            // Lagged error - subscriber fell behind, continue
                        }
                    }
                }
            });
        }

        let node = Self {
            config,
            wallet,
            wallet_registry,
            keypair,
            consensus,
            mrw,
            storage,
            finality,
            economics,
            network,
            update_manager,
            event_bus,
            vrf_service,
            quorum_selector,
            parameter_store: Some(parameter_store),
            metrics,
            network_metadata,
            dkg_manager,
            bls_coordinator,
            dkg_orchestrator,
            // Application layer managers (initialized to None, configured later)
            task_manager: None,
            governance_manager: None,
            governance_protocol: None,
            storage_market: None,
            index,
            tip_manager,
            running: Arc::new(RwLock::new(false)),
            encoders,
            current_epoch: Arc::new(RwLock::new(0)),
        };

        // Perform cache warmup
        node.warmup_caches().await?;

        // Emit node started event
        node.event_bus.emit(NodeEvent::NodeStarted {
            node_id: node.node_id(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            timestamp: Utc::now(),
        });

        Ok(node)
    }

    // ========== Application Layer Configuration ==========

    /// Configure PoUW TaskManager
    ///
    /// Requires EscrowManager (built from economics) and ReputationTracker (from consensus).
    #[allow(dead_code)] // Used in tests and available for runtime configuration
    pub fn with_task_manager(
        mut self,
        config: adic_pouw::TaskManagerConfig,
    ) -> Result<Self> {
        use adic_app_common::EscrowManager;

        // Create EscrowManager using economics balance manager
        let escrow_mgr = Arc::new(EscrowManager::new(Arc::clone(&self.economics.balances)));

        // Create TaskManager using consensus reputation tracker
        let task_manager = Arc::new(adic_pouw::TaskManager::new(
            escrow_mgr,
            Arc::new(self.consensus.reputation.clone()),
            config,
        ));

        self.task_manager = Some(task_manager);
        info!("✅ TaskManager configured");

        Ok(self)
    }

    /// Configure Governance ProposalLifecycleManager
    ///
    /// Requires ReputationTracker, and optionally FinalityAdapter, QuorumSelector,
    /// BLS components for threshold signatures.
    pub fn with_governance_manager(
        mut self,
        lifecycle_config: adic_governance::LifecycleConfig,
        protocol_config: adic_network::protocol::GovernanceConfig,
    ) -> Result<Self> {
        use adic_app_common::FinalityAdapter;

        // Create base manager with reputation tracker
        let mut manager = adic_governance::ProposalLifecycleManager::new(
            lifecycle_config,
            Arc::new(self.consensus.reputation.clone()),
        );

        // Wire finality adapter
        let finality_adapter = Arc::new(FinalityAdapter::with_defaults(
            Arc::clone(&self.finality),
        ));
        manager = manager.with_finality_adapter(finality_adapter);

        // Wire parameter store if available
        if let Some(ref param_store) = self.parameter_store {
            manager = manager.with_parameter_store(Arc::clone(param_store));
        }

        // Wire BLS coordinator if available
        if let Some(ref bls_coord) = self.bls_coordinator {
            manager = manager.with_bls_coordinator(Arc::clone(bls_coord));
        }

        // Create treasury executor for grant execution
        let treasury_executor = Arc::new(adic_governance::TreasuryExecutor::new(
            Arc::clone(&self.economics.balances),
            adic_economics::AccountAddress::treasury(),
        ));
        manager = manager.with_treasury_executor(treasury_executor);

        // Create governance network protocol
        let governance_protocol = Arc::new(adic_network::protocol::GovernanceProtocol::new(
            protocol_config,
        ));

        self.governance_manager = Some(Arc::new(manager));
        self.governance_protocol = Some(governance_protocol);
        info!("✅ Governance ProposalLifecycleManager, treasury executor, and protocol configured");

        Ok(self)
    }

    /// Configure StorageMarketCoordinator
    ///
    /// Requires multiple sub-managers for intent matching, JITCA compilation,
    /// proof verification, and balance management.
    #[allow(dead_code)] // Used in tests and available for runtime configuration
    pub fn with_storage_market(
        mut self,
        config: adic_storage_market::MarketConfig,
        intent_config: adic_storage_market::IntentConfig,
        jitca_config: adic_storage_market::JitcaConfig,
        proof_config: adic_storage_market::ProofCycleConfig,
        challenge_config: adic_challenges::ChallengeConfig,
    ) -> Result<Self> {
        use adic_challenges::ChallengeWindowManager;
        use adic_storage_market::{
            IntentManager, JitcaCompiler, ProofCycleManager, StorageMarketCoordinator,
        };

        // Create sub-managers with proper dependencies
        let intent_manager = Arc::new(IntentManager::new(
            intent_config,
            Arc::clone(&self.economics.balances),
            Arc::new(self.consensus.reputation.clone()),
        ));

        let jitca_compiler = Arc::new(JitcaCompiler::new(
            jitca_config,
            Arc::clone(&self.economics.balances),
        ));

        // Create challenge window manager
        let mut challenge_manager_instance = ChallengeWindowManager::new(challenge_config);
        challenge_manager_instance.set_metrics(
            Arc::new(self.metrics.challenge_windows_opened.clone()),
            Arc::new(self.metrics.challenges_submitted.clone()),
            Arc::new(self.metrics.challenge_windows_active.clone()),
        );
        let challenge_manager = Arc::new(challenge_manager_instance);

        // Create dispute adjudicator with metrics
        let quorum_selector = Arc::new(adic_quorum::QuorumSelector::new(
            Arc::clone(&self.vrf_service),
            Arc::new(self.consensus.reputation.clone()),
        ));
        let mut dispute_adjudicator_instance = adic_challenges::DisputeAdjudicator::new(
            quorum_selector,
            adic_challenges::AdjudicationConfig::default(),
        );
        dispute_adjudicator_instance.set_metrics(
            Arc::new(self.metrics.fraud_proofs_submitted.clone()),
            Arc::new(self.metrics.fraud_proofs_verified.clone()),
            Arc::new(self.metrics.fraud_proofs_rejected.clone()),
            Arc::new(self.metrics.arbitrations_started.clone()),
            Arc::new(self.metrics.arbitrations_completed.clone()),
            Arc::new(self.metrics.quorum_votes_total.clone()),
            Arc::new(self.metrics.quorum_votes_passed.clone()),
            Arc::new(self.metrics.quorum_votes_failed.clone()),
            Arc::new(self.metrics.quorum_vote_duration.clone()),
        );
        let dispute_adjudicator = Arc::new(dispute_adjudicator_instance);

        // Build ProofCycleManager and wire FinalityAdapter for production finality checks
        let proof_manager = {
            let base = ProofCycleManager::new(
                proof_config,
                Arc::clone(&self.economics.balances),
                challenge_manager,
                Arc::clone(&self.vrf_service),
                dispute_adjudicator,
            );
            // Inject finality adapter so storage proofs wait on F1 (operational signals)
            let adapter = adic_app_common::FinalityAdapter::with_defaults(Arc::clone(&self.finality));
            Arc::new(base.with_finality_adapter(Arc::new(adapter)))
        };

        // Create coordinator
        let coordinator = Arc::new(StorageMarketCoordinator::new(
            config,
            intent_manager,
            jitca_compiler,
            proof_manager,
            Arc::clone(&self.economics.balances),
        ));

        self.storage_market = Some(coordinator);
        info!("✅ StorageMarketCoordinator configured");

        Ok(self)
    }

    pub fn node_id(&self) -> String {
        hex::encode(&self.keypair.public_key().as_bytes()[..8])
    }

    pub fn public_key(&self) -> PublicKey {
        *self.keypair.public_key()
    }

    pub fn get_deposit_amount(&self) -> AdicAmount {
        // Get from genesis config - 0.1 ADIC per paper
        AdicAmount::from_adic(0.1)
    }

    pub fn wallet_address(&self) -> AccountAddress {
        self.wallet.address()
    }

    /// Get access to the event bus for real-time notifications
    pub fn event_bus(&self) -> Arc<EventBus> {
        Arc::clone(&self.event_bus)
    }

    pub fn wallet(&self) -> &NodeWallet {
        &self.wallet
    }

    /// Sync peer network metadata from PeerManager to NetworkMetadataRegistry
    ///
    /// Should be called periodically or when peers update to ensure registry
    /// has current ASN/region information for quorum selection.
    pub async fn sync_peer_metadata_to_registry(&self) -> anyhow::Result<()> {
        let network = match &self.network {
            Some(net) => net,
            None => return Ok(()), // No network, nothing to sync
        };

        let net = network.read().await;
        let peers = net.get_all_peers_info().await;

        for peer_info in peers {
            if peer_info.asn.is_some() || peer_info.region.is_some() {
                let metadata = adic_app_common::NodeNetworkInfo {
                    asn: peer_info.asn.unwrap_or(0),
                    region: peer_info.region.clone().unwrap_or_else(|| "unknown".to_string()),
                    metadata: std::collections::HashMap::new(),
                };

                self.network_metadata
                    .register(peer_info.public_key, metadata)
                    .await;

                debug!(
                    peer = ?peer_info.peer_id,
                    asn = ?peer_info.asn,
                    region = ?peer_info.region,
                    "Synced peer metadata to registry"
                );
            }
        }

        Ok(())
    }

    /// Get all eligible nodes for quorum selection from connected peers
    ///
    /// Returns nodes with their metadata (ASN, region, reputation) for use
    /// in quorum selection. Only includes peers that:
    /// - Are alive (connected and seen within last 5 minutes)
    /// - Have network metadata (ASN, region)
    /// - Meet minimum reputation threshold from consensus
    pub async fn get_eligible_quorum_nodes(
        &self,
    ) -> anyhow::Result<Vec<adic_quorum::NodeInfo>> {
        let network = match &self.network {
            Some(net) => net,
            None => return Ok(Vec::new()), // No network, no peers
        };

        let net = network.read().await;

        // Get alive peers with metadata (filters by liveness and metadata presence)
        let max_age = std::time::Duration::from_secs(300); // 5 minutes
        let alive_peers = net.peer_manager().get_alive_peers_with_metadata(max_age).await;

        let mut eligible_nodes = Vec::new();

        for peer_info in alive_peers {
            // unwrap is safe because get_alive_peers_with_metadata filters for Some
            let asn = peer_info.asn.unwrap();
            let region = peer_info.region.clone().unwrap();

            let node = adic_quorum::NodeInfo {
                public_key: peer_info.public_key,
                reputation: peer_info.reputation_score,
                asn: Some(asn),
                region: Some(region),
                axis_balls: vec![], // p-adic balls computed by QuorumSelector
            };
            eligible_nodes.push(node);
        }

        Ok(eligible_nodes)
    }

    /// Start parameter hot-reload listener
    ///
    /// Subscribes to ParameterStore changes and automatically updates
    /// ConsensusEngine, MrwEngine, and FinalityEngine when operational
    /// parameters change via governance.
    async fn start_parameter_reload_listener(&self) {
        let Some(param_store) = self.parameter_store.as_ref() else {
            info!("⚙️  Parameter store not configured, hot-reload disabled");
            return;
        };

        let mut rx = param_store.subscribe();
        let consensus = Arc::clone(&self.consensus);
        let mrw = Arc::clone(&self.mrw);
        let finality = Arc::clone(&self.finality);
        let param_store = Arc::clone(param_store);

        tokio::spawn(async move {
            info!("🔄 Parameter hot-reload listener started");

            while let Ok(change) = rx.recv().await {
                info!(
                    param = %change.key,
                    old = %change.old_value,
                    new = %change.new_value,
                    epoch = change.applied_at_epoch,
                    "📝 Parameter changed, reloading engines"
                );

                // Build updated params from store
                match param_store.build_adic_params().await {
                    Ok(new_params) => {
                        // Update ConsensusEngine operational params
                        consensus.update_operational_params(&new_params).await;

                        // Update MrwEngine params
                        mrw.update_params(&new_params).await;

                        // Update FinalityEngine params
                        // Note: F2 params (max_betti, min_persistence) use defaults for now
                        finality.update_params(&new_params, 2, 0.01).await;

                        info!("✅ All engines updated with new parameters");
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to build AdicParams from store");
                    }
                }
            }

            info!("🔄 Parameter hot-reload listener stopped");
        });
    }

    /// Validate production readiness
    ///
    /// Checks that critical infrastructure components are configured
    /// and warns about missing optional components.
    ///
    /// This should be called before starting the node to ensure
    /// production-critical features are available.
    pub fn validate_production_readiness(&self) -> Result<()> {
        info!("🔍 Validating node production readiness...");

        let mut warnings = Vec::new();
        let mut errors = Vec::new();

        // Check DKG manager (required for committee operations)
        if self.dkg_manager.is_none() {
            warnings.push("DKG manager not configured - distributed key generation disabled");
        }

        // Check BLS coordinator (required for governance threshold signatures)
        if self.bls_coordinator.is_none() {
            // If governance manager is enabled, missing BLS coordinator is a hard error
            if self.governance_manager.is_some() {
                errors.push("BLS coordinator not configured but governance is enabled - threshold receipt signatures required");
            } else {
                warnings.push("BLS coordinator not configured - threshold signatures disabled");
            }
        }

        // Check DKG orchestrator (required for network-wide DKG ceremonies)
        if self.dkg_orchestrator.is_none() {
            warnings.push("DKG orchestrator not configured - committee key generation disabled");
        }

        // Check parameter store (required for governance parameter updates)
        if self.parameter_store.is_none() {
            warnings.push("Parameter store not configured - dynamic parameter updates disabled");
        }

        // Check network engine (required for p2p communication)
        if self.network.is_none() {
            errors.push("Network engine not configured - node cannot communicate with peers");
        }

        // Check application layer managers (optional but recommended)
        if self.task_manager.is_none() {
            warnings.push("TaskManager not configured - PoUW functionality disabled");
        }

        if self.governance_manager.is_none() {
            warnings.push("GovernanceManager not configured - on-chain governance disabled");
        }

        if self.storage_market.is_none() {
            warnings.push("StorageMarket not configured - decentralized storage disabled");
        }

        // Log warnings
        for warning in &warnings {
            warn!("⚠️ {}", warning);
        }

        // Fail on errors
        if !errors.is_empty() {
            for error in &errors {
                error!("❌ {}", error);
            }
            return Err(anyhow::anyhow!(
                "Production readiness validation failed: {}",
                errors.join(", ")
            ));
        }

        if warnings.is_empty() {
            info!("✅ Production readiness validation passed - all components configured");
        } else {
            warn!(
                "⚠️ Production readiness validation passed with {} warnings",
                warnings.len()
            );
        }

        Ok(())
    }

    pub async fn run(&self) -> Result<()> {
        // Validate production readiness before starting
        self.validate_production_readiness()?;

        {
            let mut running = self.running.write().await;
            *running = true;
        }

        info!("Node started and running");

        // Start parameter hot-reload listener
        self.start_parameter_reload_listener().await;

        // Start network engine if enabled
        if let Some(ref network) = self.network {
            network.read().await.start().await?;

            // Connect to bootstrap peers
            for peer in &self.config.network.bootstrap_peers {
                // Try to parse as SocketAddr first, then try to resolve hostname
                let addr_result = if let Ok(addr) = peer.parse::<SocketAddr>() {
                    Ok(addr)
                } else {
                    // Try to resolve hostname:port
                    use tokio::net::lookup_host;
                    match lookup_host(peer).await {
                        Ok(mut addrs) => {
                            if let Some(addr) = addrs.next() {
                                Ok(addr)
                            } else {
                                Err(anyhow::anyhow!("No addresses found for {}", peer))
                            }
                        }
                        Err(e) => Err(anyhow::anyhow!("Failed to resolve {}: {}", peer, e)),
                    }
                };

                match addr_result {
                    Ok(addr) => {
                        info!(
                            peer = %peer,
                            resolved_addr = %addr,
                            "🔍 Connecting to bootstrap peer"
                        );
                        if let Err(e) = network.read().await.connect_peer(addr).await {
                            warn!("Failed to connect to bootstrap peer {}: {}", addr, e);
                        }
                    }
                    Err(e) => {
                        warn!("Failed to resolve bootstrap peer: {}", e);
                    }
                }
            }
        }

        // Start update manager if enabled
        if let Some(ref update_manager) = self.update_manager {
            if let Err(e) = update_manager.start().await {
                warn!("Failed to start update manager: {}", e);
            }
        }

        // Start finality checker
        self.finality.start_checker().await;

        // Main loop
        while *self.running.read().await {
            // Process incoming messages from network (if any)
            self.process_incoming_messages().await?;

            // Check for finalized messages
            self.check_finality().await?;

            // Check for epoch transitions (VRF finalization)
            self.check_epoch_transition().await?;

            // Update tips
            self.update_tips().await?;

            // Apply reputation decay periodically
            if self.should_apply_reputation_decay() {
                self.consensus.reputation.apply_decay().await;
            }

            // Cleanup old conflicts
            if self.should_cleanup_conflicts() {
                self.consensus
                    .energy_tracker()
                    .cleanup_resolved(86400)
                    .await; // 1 day old conflicts
            }

            // Emit energy metrics periodically
            if self.should_emit_energy_metrics() {
                self.emit_energy_event().await;
            }

            // Emit admissibility metrics periodically
            if self.should_emit_admissibility_metrics() {
                self.emit_admissibility_event().await;
            }

            // Update DAG state metrics (tips and messages count)
            self.update_dag_metrics().await;

            // Sync peer metadata to registry for quorum selection
            if let Err(e) = self.sync_peer_metadata_to_registry().await {
                warn!("Failed to sync peer metadata to registry: {}", e);
            }

            // Periodically cleanup stale peers (every 60 seconds)
            if self.should_cleanup_stale_peers() {
                if let Some(ref network) = self.network {
                    let net = network.read().await;
                    let stale_age = std::time::Duration::from_secs(600); // 10 minutes
                    let removed = net.peer_manager().remove_stale_peers(stale_age).await;
                    if removed > 0 {
                        info!("🧹 Cleaned up {} stale peers", removed);
                    }
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            // - Check for new messages
            // - Run consensus
            // - Update finality
            // - Create snapshots
        }

        info!("Node stopped");
        Ok(())
    }

    pub async fn _stop(&self) -> Result<()> {
        info!("Stopping node...");
        let mut running = self.running.write().await;
        *running = false;
        Ok(())
    }

    /// Generate features using the encoder set with actual metadata
    async fn create_encoded_features(&self, topic: &str, region: &str) -> Result<AdicFeatures> {
        // Get current stake balance for StakeTierEncoder
        let proposer_addr = AccountAddress::from_public_key(&self.wallet.public_key());
        let balance = self.economics.balances.get_balance(proposer_addr).await?;
        let stake_amount = balance.to_adic() as u64;

        // Create encoder data with all required fields
        let encoder_data = EncoderData::new(
            Utc::now().timestamp(),
            topic.to_string(),
            region.to_string(),
        )
        .with_stake(stake_amount);

        // Use encoders to create all 4 axes
        let params = self.config.adic_params();
        let precision = 10; // Default p-adic precision
        let axis_phi = self.encoders.encode_all(&encoder_data, params.p, precision);

        Ok(AdicFeatures::new(axis_phi))
    }

    #[allow(dead_code)]
    pub async fn submit_message(&self, content: Vec<u8>) -> Result<MessageId> {
        self.submit_message_with_transfer(content, None).await
    }

    pub async fn submit_message_with_transfer(
        &self,
        content: Vec<u8>,
        transfer: Option<adic_types::ValueTransfer>,
    ) -> Result<MessageId> {
        debug!(
            "Submitting new message with transfer: {}",
            transfer.is_some()
        );

        // 1. Get tips first to inform feature generation
        let tips = self.tip_manager.get_tips().await;

        // 2. Create features with good p-adic proximity to existing tips
        let features = if !tips.is_empty() {
            // Get a reference tip to base features on for better proximity
            if let Ok(Some(tip_msg)) = self.storage.get_message(&tips[0]).await {
                // Create features with good p-adic proximity to the tip
                let mut new_features = Vec::new();
                for (i, axis_phi) in tip_msg.features.phi.iter().enumerate() {
                    let mut new_digits = axis_phi.qp_digits.digits.clone();

                    // For p-adic proximity, we need to keep most digits the same
                    // Only modify higher-order digits to maintain closeness
                    // This ensures high vp (valuation) differences, resulting in scores close to 1.0 per axis

                    // Strategy: Keep first ρ[i] digits mostly the same (for ball membership)
                    // and only perturb higher digits
                    let params = self.config.adic_params();
                    let radius = params.rho.get(i).copied().unwrap_or(2) as usize;

                    // Modify a digit beyond the radius to maintain proximity
                    if new_digits.len() > radius {
                        // Modify a digit at position radius or higher (less significant in p-adic metric)
                        // This gives vp >= radius, ensuring term = 1.0 in admissibility score
                        let modify_pos = radius; // Modify the digit just beyond the ball radius
                        if modify_pos < new_digits.len() {
                            new_digits[modify_pos] = (new_digits[modify_pos] + 1) % 3;
                        }
                    } else if !new_digits.is_empty() {
                        // If we don't have enough digits, add variation at the end
                        // This maintains p-adic closeness
                        let last_pos = new_digits.len() - 1;
                        new_digits[last_pos] = (new_digits[last_pos] + 1) % 3;
                    }

                    // Create new QpDigits with modified digits
                    let new_qp = QpDigits {
                        digits: new_digits,
                        p: axis_phi.qp_digits.p,
                    };

                    new_features.push(AxisPhi::new(i as u32, new_qp));
                }
                AdicFeatures::new(new_features)
            } else {
                // Fallback: use encoded features with all 4 axes
                self.create_encoded_features("default", "unknown").await?
            }
        } else {
            // No tips available: use encoded features for genesis-like messages (all 4 axes)
            self.create_encoded_features("default", "unknown").await?
        };

        // 3. Select d+1 parents using MRW
        let parents = self
            .mrw
            .select_parents(
                &features,
                &tips,
                &self.storage,
                &self.consensus,
            )
            .await?;

        // 3. Validate transfer if provided (before escrowing deposit)
        if let Some(ref t) = transfer {
            self.economics
                .balances
                .validate_transfer(&t.from, &t.to, t.amount, t.nonce)
                .await?;
        }

        // 4. Create the message
        let mut message = if let Some(t) = transfer {
            AdicMessage::new_with_transfer(
                parents.clone(),
                features,
                AdicMeta::new(Utc::now()),
                *self.keypair.public_key(),
                t,
                content,
            )
        } else {
            AdicMessage::new(
                parents.clone(),
                features,
                AdicMeta::new(Utc::now()),
                *self.keypair.public_key(),
                content,
            )
        };

        // Sign the message before validation
        let signature = self.keypair.sign(&message.to_bytes());
        message.signature = signature;

        // 4. Escrow deposit
        let proposer_pk = *self.keypair.public_key();
        let deposit_amount = self.consensus.deposits.get_deposit_amount().await;
        self.consensus
            .deposits
            .escrow(message.id, proposer_pk)
            .await?;

        // Emit deposit escrowed event
        self.event_bus.emit(NodeEvent::DepositEscrowed {
            message_id: hex::encode(message.id.as_bytes()),
            validator_address: hex::encode(proposer_pk.as_bytes()),
            amount: deposit_amount.to_adic().to_string(),
            timestamp: Utc::now(),
        });

        // 5. Perform C1-C3 admissibility checks BEFORE validation
        // This is critical for consensus security
        if !parents.is_empty() {
            // Non-genesis messages must pass C1-C3
            let mut parent_features = Vec::new();
            let mut parent_reputations = Vec::new();

            for parent_id in &parents {
                match self.storage.get_message(parent_id).await {
                    Ok(Some(parent)) => {
                        // Extract parent features for C1 check
                        let mut features = Vec::new();
                        for axis_phi in &parent.features.phi {
                            features.push(axis_phi.qp_digits.clone());
                        }
                        parent_features.push(features);

                        // Get parent reputation for C3 check
                        parent_reputations.push(
                            self.consensus
                                .reputation
                                .get_reputation(&parent.proposer_pk)
                                .await,
                        );
                    }
                    Ok(None) => {
                        warn!(
                            "Parent message {} not found in storage - may not be synced yet",
                            hex::encode(parent_id.as_bytes())
                        );
                        return Err(anyhow::anyhow!("Missing parent message"));
                    }
                    Err(e) => {
                        warn!(
                            "Error fetching parent message {}: {}",
                            hex::encode(parent_id.as_bytes()),
                            e
                        );
                        return Err(anyhow::anyhow!("Error fetching parent message: {}", e));
                    }
                }
            }

            // Check C1-C3 admissibility
            debug!(
                parent_count = parent_reputations.len(),
                reputations = ?parent_reputations,
                "C1-C3 admissibility check"
            );
            let admissibility_result = self
                .consensus
                .admissibility()
                .check_message(&message, &parent_features, &parent_reputations)
                .map_err(|e| anyhow::anyhow!("C1-C3 check failed: {}", e))?;

            if !admissibility_result.is_admissible() {
                warn!(
                    "Message failed C1-C3 admissibility: {}",
                    admissibility_result.details
                );

                // Slash the deposit for failing admissibility
                self.consensus
                    .deposits
                    .slash(&message.id, &admissibility_result.details)
                    .await?;

                // Emit deposit slashed event
                self.event_bus.emit(NodeEvent::DepositSlashed {
                    message_id: hex::encode(message.id.as_bytes()),
                    validator_address: hex::encode(proposer_pk.as_bytes()),
                    amount: deposit_amount.to_adic().to_string(),
                    reason: admissibility_result.details.clone(),
                    timestamp: Utc::now(),
                });

                // Emit message rejected event
                self.event_bus.emit(NodeEvent::MessageRejected {
                    message_id: hex::encode(message.id.as_bytes()),
                    reason: admissibility_result.details.clone(),
                    rejection_type: RejectionType::AdmissibilityFailed,
                    timestamp: Utc::now(),
                });

                return Err(anyhow::anyhow!(
                    "C1-C3 admissibility failed: {}",
                    admissibility_result.details
                ));
            }

            info!(
                "Message passed C1-C3 checks (score: {:.2})",
                admissibility_result.score
            );
        }

        // 6. Validate the message (additional validation beyond C1-C3)
        let validation_result = self.consensus.validate_and_slash(&message).await?;
        if !validation_result.is_valid {
            warn!(
                "Message failed validation and was slashed: {:?}",
                validation_result.errors
            );

            // Emit deposit slashed event (deposit was already slashed by validate_and_slash)
            self.event_bus.emit(NodeEvent::DepositSlashed {
                message_id: hex::encode(message.id.as_bytes()),
                validator_address: hex::encode(proposer_pk.as_bytes()),
                amount: deposit_amount.to_adic().to_string(),
                reason: format!("{:?}", validation_result.errors),
                timestamp: Utc::now(),
            });

            // Emit message rejected event
            self.event_bus.emit(NodeEvent::MessageRejected {
                message_id: hex::encode(message.id.as_bytes()),
                reason: format!("{:?}", validation_result.errors),
                rejection_type: RejectionType::ValidationFailed,
                timestamp: Utc::now(),
            });

            return Err(anyhow::anyhow!(
                "Message failed validation: {:?}",
                validation_result.errors
            ));
        }
        // Store message
        self.storage.store_message(&message).await?;

        // Update indices - fetch actual parent depths
        let mut parent_depths: Vec<u32> = Vec::new();
        for parent_id in &message.parents {
            if let Some(depth) = self.index.get_depth(parent_id).await {
                parent_depths.push(depth);
            } else {
                // Genesis messages or unknown parents have depth 0
                parent_depths.push(0);
            }
        }
        self.index.add_message(&message, parent_depths).await;

        // Add to tip manager
        self.tip_manager.add_tip(message.id, 1.0).await;

        // Remove only the selected parents from tips
        for parent_id in &parents {
            self.tip_manager.remove_tip(parent_id).await;
        }

        // Emit tips updated event for real-time monitoring
        let tips = self.tip_manager.get_tips().await;
        self.event_bus.emit(NodeEvent::TipsUpdated {
            tips: tips.iter().map(|id| hex::encode(id.as_bytes())).collect(),
            count: tips.len(),
            timestamp: Utc::now(),
        });

        // Emit diversity event immediately when message is added
        self.emit_diversity_event(&tips).await;

        // Emit message added event
        self.event_bus.emit(NodeEvent::MessageAdded {
            message_id: hex::encode(message.id.as_bytes()),
            depth: self.index.get_depth(&message.id).await.unwrap_or(0) as u64,
            timestamp: Utc::now(),
        });

        // Add to finality engine
        let mut ball_ids = std::collections::HashMap::new();
        for axis_phi in &message.features.phi {
            ball_ids.insert(axis_phi.axis.0, axis_phi.qp_digits.ball_id(3));
        }

        self.finality
            .add_message(
                message.id,
                parents.clone(), // Use selected parents, not all tips
                1.0,             // Initial reputation
                ball_ids,
            )
            .await?;

        // Broadcast to network if enabled
        if let Some(ref network) = self.network {
            network
                .read()
                .await
                .broadcast_message(message.clone())
                .await?;
            debug!("Message broadcast to network");
        }

        info!("Message submitted: {}", hex::encode(message.id.as_bytes()));

        Ok(message.id)
    }

    #[allow(dead_code)]
    pub async fn submit_message_with_parents(
        &self,
        content: Vec<u8>,
        parents: Vec<MessageId>,
        features_opt: Option<crate::api::SubmitFeatures>,
    ) -> Result<MessageId> {
        self.submit_message_with_parents_and_transfer(content, parents, features_opt, None)
            .await
    }

    pub async fn submit_message_with_parents_and_transfer(
        &self,
        content: Vec<u8>,
        parents: Vec<MessageId>,
        features_opt: Option<crate::api::SubmitFeatures>,
        transfer: Option<adic_types::ValueTransfer>,
    ) -> Result<MessageId> {
        debug!(
            "Submitting message with explicit parents: {} parents, transfer: {}",
            parents.len(),
            transfer.is_some()
        );

        // Create features - either from provided data or generate default
        let features = if let Some(submit_features) = features_opt {
            // Convert simplified {axis, value} format to proper p-adic QpDigits
            let mut axis_features = Vec::new();
            for axis_value in submit_features.axes {
                let qp_digits = QpDigits::from_u64(axis_value.value, 3, 10);
                axis_features.push(AxisPhi::new(axis_value.axis, qp_digits));
            }
            AdicFeatures::new(axis_features)
        } else if parents.is_empty() {
            // Genesis message: use encoded features with all 4 axes
            self.create_encoded_features("default", "unknown").await?
        } else {
            // CRITICAL FIX: Generate features that are p-adic close to ALL parents, not just first parent
            // The admissibility formula requires proximity to ALL parents for each axis

            // Collect all parent messages and their features
            let mut all_parent_features = Vec::new();
            for parent_id in &parents {
                if let Ok(Some(parent_msg)) = self.storage.get_message(parent_id).await {
                    all_parent_features.push(parent_msg.features.clone());
                }
            }

            if !all_parent_features.is_empty() {
                let params = self.config.adic_params();
                let mut new_features = Vec::new();

                // For each axis, generate a feature that is close to ALL parents
                for axis_idx in 0..params.d {
                    // Collect features from all parents for this axis
                    let parent_axis_features: Vec<&QpDigits> = all_parent_features
                        .iter()
                        .filter_map(|pf| {
                            pf.get_axis(AxisId(axis_idx as u32)).map(|ap| &ap.qp_digits)
                        })
                        .collect();

                    if !parent_axis_features.is_empty() {
                        // New strategy: Find the geometric median of the parent features on this axis.
                        // This ensures the new feature is centrally located with respect to all parents.
                        let max_digits = parent_axis_features.iter().map(|d| d.digits.len()).max().unwrap_or(0);
                        let mut new_digits = Vec::with_capacity(max_digits);

                        for i in 0..max_digits {
                            let mut digits_at_pos: Vec<u8> = parent_axis_features.iter()
                                .map(|d| d.digits.get(i).copied().unwrap_or(0))
                                .collect();
                            digits_at_pos.sort();
                            let median = digits_at_pos[digits_at_pos.len() / 2];
                            new_digits.push(median);
                        }

                        let new_qp = QpDigits {
                            digits: new_digits,
                            p: parent_axis_features[0].p,
                        };
                        new_features.push(AxisPhi::new(axis_idx as u32, new_qp));
                    } else {
                        // Fallback if no parent features found for this axis
                        let default_qp = QpDigits::from_u64(1 + axis_idx as u64, params.p, 10);
                        new_features.push(AxisPhi::new(axis_idx as u32, default_qp));
                    }
                }

                AdicFeatures::new(new_features)
            } else {
                // Fallback: use encoded features with all 4 axes
                self.create_encoded_features("default", "unknown").await?
            }
        };

        // Validate transfer if provided (before escrowing deposit)
        if let Some(ref t) = transfer {
            self.economics
                .balances
                .validate_transfer(&t.from, &t.to, t.amount, t.nonce)
                .await?;
        }

        // Create the message
        let mut message = if let Some(t) = transfer {
            AdicMessage::new_with_transfer(
                parents.clone(),
                features,
                AdicMeta::new(Utc::now()),
                *self.keypair.public_key(),
                t,
                content,
            )
        } else {
            AdicMessage::new(
                parents.clone(),
                features,
                AdicMeta::new(Utc::now()),
                *self.keypair.public_key(),
                content,
            )
        };

        // Sign the message
        let signature = self.keypair.sign(&message.to_bytes());
        message.signature = signature;

        // Escrow deposit
        let proposer_pk = *self.keypair.public_key();
        let deposit_amount = self.consensus.deposits.get_deposit_amount().await;
        self.consensus
            .deposits
            .escrow(message.id, proposer_pk)
            .await?;

        // Emit deposit escrowed event
        self.event_bus.emit(NodeEvent::DepositEscrowed {
            message_id: hex::encode(message.id.as_bytes()),
            validator_address: hex::encode(proposer_pk.as_bytes()),
            amount: deposit_amount.to_adic().to_string(),
            timestamp: Utc::now(),
        });

        // IMPORTANT: Only perform C1-C3 checks if this is NOT a genesis message
        if !parents.is_empty() {
            // Debug: Log the generated features for analysis
            debug!(
                features = ?message
                    .features
                    .phi
                    .iter()
                    .map(|ap| format!(
                        "axis_{}: digits={:?}",
                        ap.axis.0,
                        &ap.qp_digits.digits[..3.min(ap.qp_digits.digits.len())]
                    ))
                    .collect::<Vec<_>>(),
                "Generated message features"
            );

            // Non-genesis messages must pass C1-C3
            let mut parent_features = Vec::new();
            let mut parent_reputations = Vec::new();

            debug!(
                parent_count = parents.len(),
                parents = ?parents
                    .iter()
                    .map(|p| hex::encode(p.as_bytes()))
                    .collect::<Vec<_>>(),
                "Processing parents for C1-C3 check"
            );

            for parent_id in &parents {
                debug!(
                    parent_id = %hex::encode(parent_id.as_bytes()),
                    "Looking up parent message"
                );
                match self.storage.get_message(parent_id).await {
                    Ok(Some(parent)) => {
                        debug!(
                            parent_features = ?parent
                                .features
                                .phi
                                .iter()
                                .map(|ap| format!(
                                    "axis_{}: digits={:?}",
                                    ap.axis.0,
                                    &ap.qp_digits.digits[..3.min(ap.qp_digits.digits.len())]
                                ))
                                .collect::<Vec<_>>(),
                            "Parent message features"
                        );

                        let mut features = Vec::new();
                        for axis_phi in &parent.features.phi {
                            features.push(axis_phi.qp_digits.clone());
                        }
                        parent_features.push(features);

                        parent_reputations.push(
                            self.consensus
                                .reputation
                                .get_reputation(&parent.proposer_pk)
                                .await,
                        );
                    }
                    Ok(None) => {
                        warn!(
                            "Parent message {:?} not found in storage - may not be synced yet",
                            hex::encode(parent_id.as_bytes())
                        );
                        // Parent not in storage - fail early
                        return Err(anyhow::anyhow!(
                            "Parent message {:?} not found in storage",
                            hex::encode(parent_id.as_bytes())
                        ));
                    }
                    Err(e) => {
                        warn!(
                            "Error fetching parent message {:?}: {}",
                            hex::encode(parent_id.as_bytes()),
                            e
                        );
                        return Err(anyhow::anyhow!("Error fetching parent message: {}", e));
                    }
                }
            }

            debug!(
                parent_count = parent_reputations.len(),
                reputations = ?parent_reputations,
                "C1-C3 admissibility check (submit_message_with_parents)"
            );
            let admissibility_result = self.consensus.admissibility().check_message(
                &message,
                &parent_features,
                &parent_reputations,
            )?;

            if !admissibility_result.is_admissible() {
                warn!(
                    "Message failed C1-C3 admissibility: {}",
                    admissibility_result.details
                );
                self.consensus
                    .deposits
                    .slash(&message.id, &admissibility_result.details)
                    .await?;

                // Emit deposit slashed event
                self.event_bus.emit(NodeEvent::DepositSlashed {
                    message_id: hex::encode(message.id.as_bytes()),
                    validator_address: hex::encode(proposer_pk.as_bytes()),
                    amount: deposit_amount.to_adic().to_string(),
                    reason: admissibility_result.details.clone(),
                    timestamp: Utc::now(),
                });

                // Emit message rejected event
                self.event_bus.emit(NodeEvent::MessageRejected {
                    message_id: hex::encode(message.id.as_bytes()),
                    reason: admissibility_result.details.clone(),
                    rejection_type: RejectionType::AdmissibilityFailed,
                    timestamp: Utc::now(),
                });

                return Err(anyhow::anyhow!(
                    "C1-C3 admissibility failed: {}",
                    admissibility_result.details
                ));
            }

            info!(
                "Message passed C1-C3 checks (score: {:.2})",
                admissibility_result.score
            );
        } else {
            info!("Genesis message - skipping C1-C3 checks");
        }

        // Validate the message
        let validation_result = self.consensus.validate_and_slash(&message).await?;
        if !validation_result.is_valid {
            warn!(
                "Message failed validation and was slashed: {:?}",
                validation_result.errors
            );

            // Emit deposit slashed event (deposit was already slashed by validate_and_slash)
            self.event_bus.emit(NodeEvent::DepositSlashed {
                message_id: hex::encode(message.id.as_bytes()),
                validator_address: hex::encode(proposer_pk.as_bytes()),
                amount: deposit_amount.to_adic().to_string(),
                reason: format!("{:?}", validation_result.errors),
                timestamp: Utc::now(),
            });

            // Emit message rejected event
            self.event_bus.emit(NodeEvent::MessageRejected {
                message_id: hex::encode(message.id.as_bytes()),
                reason: format!("{:?}", validation_result.errors),
                rejection_type: RejectionType::ValidationFailed,
                timestamp: Utc::now(),
            });

            return Err(anyhow::anyhow!(
                "Message failed validation: {:?}",
                validation_result.errors
            ));
        }

        // Store message
        self.storage.store_message(&message).await?;

        // Update indices - calculate parent depths
        let mut parent_depths: Vec<u32> = Vec::new();
        for parent_id in &parents {
            if let Some(depth) = self.index.get_depth(parent_id).await {
                parent_depths.push(depth);
            }
        }
        self.index.add_message(&message, parent_depths).await;

        // Add to tip manager
        self.tip_manager.add_tip(message.id, 1.0).await;

        // Remove parents from tips
        for parent_id in &parents {
            self.tip_manager.remove_tip(parent_id).await;
        }

        // Add to finality engine
        let mut ball_ids = std::collections::HashMap::new();
        for axis_phi in &message.features.phi {
            ball_ids.insert(axis_phi.axis.0, axis_phi.qp_digits.ball_id(3));
        }

        self.finality
            .add_message(message.id, parents, 1.0, ball_ids)
            .await?;

        // Broadcast to network if available
        if let Some(ref network) = self.network {
            let net = network.read().await;
            net.broadcast_message(message.clone()).await?;
        }

        info!("Successfully submitted message: {}", message.id);

        Ok(message.id)
    }

    pub async fn get_message(&self, id: &MessageId) -> Result<Option<AdicMessage>> {
        self.storage
            .get_message(id)
            .await
            .map_err(|e| anyhow::anyhow!(e))
    }

    pub async fn get_tips(&self) -> Result<Vec<MessageId>> {
        self.storage
            .get_tips()
            .await
            .map_err(|e| anyhow::anyhow!(e))
    }

    pub async fn get_stats(&self) -> Result<NodeStats> {
        let storage_stats = self
            .storage
            .get_stats()
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        let finality_stats = self.finality.get_stats().await;

        Ok(NodeStats {
            message_count: storage_stats.message_count,
            tip_count: storage_stats.tip_count,
            finalized_count: finality_stats.finalized_count,
            pending_finality: finality_stats.pending_count,
        })
    }

    pub async fn get_finality_artifact(
        &self,
        id: &MessageId,
    ) -> Option<adic_finality::artifact::FinalityArtifact> {
        self.finality.get_artifact(id).await
    }

    /// Get the depth of a message from the index
    pub async fn get_message_depth(&self, id: &MessageId) -> Option<u32> {
        self.index.get_depth(id).await
    }

    /// Warmup caches on startup to improve initial performance
    /// This pre-loads frequently accessed data into memory
    async fn warmup_caches(&self) -> Result<()> {
        info!("🔥 Warming up caches...");

        // 1. Load recent finalized messages into finality cache
        let recent_finalized = self
            .storage
            .get_recently_finalized(100)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get recent finalized: {}", e))?;

        info!(
            finalized_loaded = recent_finalized.len(),
            "Loaded recent finalized messages"
        );

        // 2. Load tips to warm up storage cache
        let tips = self
            .storage
            .get_tips()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get tips: {}", e))?;

        info!(tips_loaded = tips.len(), "Loaded current tips");

        // 3. Pre-load recent messages for validation cache warmup
        let mut loaded_count = 0;
        for tip_id in tips.iter().take(50) {
            if let Ok(Some(_msg)) = self.storage.get_message(tip_id).await {
                loaded_count += 1;
            }
        }

        info!(
            messages_loaded = loaded_count,
            "Pre-loaded recent messages for cache warmup"
        );

        // 4. Trigger DAG index warmup by loading recent message depths
        for finalized_id in recent_finalized.iter().take(20) {
            let _ = self.index.get_depth(finalized_id).await;
        }

        info!("✅ Cache warmup complete");
        Ok(())
    }

    async fn process_incoming_messages(&self) -> Result<()> {
        // Process messages from the network if enabled
        if let Some(ref network) = self.network {
            // Get pending messages from the network buffer
            let messages = network.read().await.get_pending_messages().await;

            for msg in messages {
                debug!(
                    message_id = %hex::encode(msg.id.as_bytes()),
                    "Processing message from network"
                );

                // Validate the message
                let validation_result = self.consensus.validate_and_slash(&msg).await?;
                if validation_result.is_valid {
                    // Store valid messages
                    self.storage
                        .store_message(&msg)
                        .await
                        .map_err(|e| anyhow::anyhow!(e))?;

                    // Update tips
                    self.tip_manager.add_tip(msg.id, 1.0).await;

                    // Add to finality engine
                    let mut ball_ids = std::collections::HashMap::new();
                    for axis_phi in &msg.features.phi {
                        ball_ids.insert(axis_phi.axis.0, axis_phi.qp_digits.ball_id(3));
                    }

                    self.finality
                        .add_message(
                            msg.id,
                            msg.parents.clone(),
                            1.0, // Initial reputation
                            ball_ids,
                        )
                        .await?;

                    // Process VRF messages (commit-reveal protocol)
                    self.process_vrf_message(&msg).await;

                    // Broadcast to other peers
                    network.read().await.broadcast_message(msg).await?;
                } else {
                    warn!(
                        "Received invalid message from network: {:?}",
                        validation_result.errors
                    );
                }
            }
        }

        Ok(())
    }

    /// Process VRF commit and reveal messages
    async fn process_vrf_message(&self, msg: &adic_types::AdicMessage) {
        use adic_vrf::{VRFCommit, VRFOpen};

        // Try to deserialize as VRFCommit
        if let Ok(vrf_commit) = serde_json::from_slice::<VRFCommit>(&msg.data) {
            match self.vrf_service.submit_commit(vrf_commit.clone()).await {
                Ok(()) => {
                    // Emit VRFCommitSubmitted event
                    self.event_bus.emit(NodeEvent::VRFCommitSubmitted {
                        commit_id: hex::encode(vrf_commit.id().as_bytes()),
                        committer: hex::encode(vrf_commit.committer.as_bytes()),
                        target_epoch: vrf_commit.target_epoch,
                        commitment_hash: hex::encode(vrf_commit.commitment),
                        committer_reputation: vrf_commit.committer_reputation,
                        timestamp: Utc::now(),
                    });
                }
                Err(e) => {
                    debug!(
                        message_id = %hex::encode(msg.id.as_bytes()),
                        error = %e,
                        "VRF commit rejected"
                    );
                }
            }
            return;
        }

        // Try to deserialize as VRFOpen
        if let Ok(vrf_reveal) = serde_json::from_slice::<VRFOpen>(&msg.data) {
            // Get current epoch
            let current_epoch = *self.current_epoch.read().await;

            match self
                .vrf_service
                .submit_reveal(vrf_reveal.clone(), current_epoch)
                .await
            {
                Ok(()) => {
                    // Emit VRFRevealOpened event
                    self.event_bus.emit(NodeEvent::VRFRevealOpened {
                        reveal_id: hex::encode(vrf_reveal.id().as_bytes()),
                        revealer: hex::encode(vrf_reveal.public_key.as_bytes()),
                        commit_id: hex::encode(vrf_reveal.ref_commit.as_bytes()),
                        target_epoch: vrf_reveal.target_epoch,
                        vrf_proof_hash: hex::encode(
                            blake3::hash(&vrf_reveal.vrf_proof).as_bytes(),
                        ),
                        timestamp: Utc::now(),
                    });
                }
                Err(e) => {
                    debug!(
                        message_id = %hex::encode(msg.id.as_bytes()),
                        error = %e,
                        "VRF reveal rejected"
                    );
                }
            }
        }
    }

    /// Check for epoch transitions and finalize VRF randomness
    async fn check_epoch_transition(&self) -> Result<()> {
        // Determine new epoch based on checkpoint height
        // Each checkpoint represents a new epoch
        let new_epoch = if let Some(checkpoint) = self.finality.get_latest_checkpoint().await {
            checkpoint.height
        } else {
            0
        };

        let mut current_epoch = self.current_epoch.write().await;

        // Check if we've advanced to a new epoch
        if new_epoch > *current_epoch {
            let old_epoch = *current_epoch;

            info!(
                old_epoch = old_epoch,
                new_epoch = new_epoch,
                "📅 Epoch transition detected (checkpoint-based)"
            );

            // Finalize VRF for the completed epoch
            match self.vrf_service.finalize_epoch(old_epoch, new_epoch).await {
                Ok(randomness) => {
                    // Get actual contributor count from VRF service
                    let contributor_count = self.vrf_service.get_reveal_count(old_epoch).await;

                    // Emit VRFRandomnessFinalized event
                    self.event_bus.emit(NodeEvent::VRFRandomnessFinalized {
                        epoch: old_epoch,
                        randomness_hash: hex::encode(blake3::hash(&randomness).as_bytes()),
                        contributor_count,
                        timestamp: Utc::now(),
                    });

                    info!(
                        epoch = old_epoch,
                        randomness_hash = hex::encode(&randomness[..8]),
                        "✨ VRF randomness finalized for epoch"
                    );
                }
                Err(e) => {
                    warn!(
                        epoch = old_epoch,
                        error = %e,
                        "Failed to finalize VRF for epoch (this is expected if no reveals)"
                    );
                }
            }

            // Initialize DKG ceremony for new epoch (if enabled)
            if let Err(e) = self.initiate_dkg_ceremony(new_epoch).await {
                warn!(
                    epoch = new_epoch,
                    error = %e,
                    "Failed to initiate DKG ceremony for new epoch"
                );
            }

            // Update current epoch
            *current_epoch = new_epoch;
        }

        Ok(())
    }

    async fn check_finality(&self) -> Result<()> {
        let finalized = self.finality.check_finality().await?;
        let has_finalized = !finalized.is_empty();

        for msg_id in finalized {
            debug!("Message finalized: {}", hex::encode(msg_id.as_bytes()));

            // Determine finality type from artifact
            use adic_finality::FinalityGate;

            let finality_type = if let Some(artifact) = self.finality.get_artifact(&msg_id).await {
                match artifact.gate {
                    FinalityGate::F1KCore => FinalityType::KCore,
                    FinalityGate::F2PersistentHomology => FinalityType::Homology,
                    FinalityGate::F1AndF2Composite => FinalityType::Composite,
                }
            } else {
                // If no artifact found, default to KCore as that's what check_finality uses
                FinalityType::KCore
            };

            // Emit finality event
            self.event_bus.emit(NodeEvent::MessageFinalized {
                message_id: hex::encode(msg_id.as_bytes()),
                finality_type,
                timestamp: Utc::now(),
            });

            // CRITICAL: Broadcast finality artifact to network
            // This ensures all peers learn about finality for consensus verification
            if let Some(artifact) = self.finality.get_artifact(&msg_id).await {
                if let Some(ref network) = self.network {
                    let network_guard = network.read().await;
                    let transport = network_guard.transport().await;
                    let gate = artifact.gate.clone();
                    if let Err(e) = transport.broadcast_finality_artifact(artifact).await {
                        warn!(
                            message_id = %hex::encode(msg_id.as_bytes()),
                            error = %e,
                            "Failed to broadcast finality artifact to network"
                        );
                    } else {
                        info!(
                            message_id = %hex::encode(msg_id.as_bytes()),
                            gate = ?gate,
                            "✅ Broadcast finality artifact to network"
                        );
                    }
                }
            }

            // Execute transfer if present (only on finality for safety)
            if let Ok(Some(message)) = self.storage.get_message(&msg_id).await {
                if let Some(transfer) = message.get_transfer() {
                    info!(
                        message_id = %hex::encode(msg_id.as_bytes()),
                        from = %hex::encode(&transfer.from),
                        to = %hex::encode(&transfer.to),
                        amount = transfer.amount,
                        "💰 Executing finalized transfer"
                    );

                    match self
                        .economics
                        .balances
                        .process_message_transfer(
                            &hex::encode(msg_id.as_bytes()),
                            &transfer.from,
                            &transfer.to,
                            transfer.amount,
                            transfer.nonce,
                        )
                        .await
                    {
                        Ok(tx_hash) => {
                            debug!(
                                message_id = %hex::encode(msg_id.as_bytes()),
                                tx_hash = %tx_hash,
                                "✅ Transfer executed successfully"
                            );
                        }
                        Err(e) => {
                            // Log error but don't fail finality - the message is already finalized
                            // This could happen if the sender's balance was already spent elsewhere
                            tracing::error!(
                                message_id = %hex::encode(msg_id.as_bytes()),
                                error = %e,
                                "❌ Failed to execute transfer from finalized message"
                            );
                        }
                    }
                }
            }

            // Update tip manager - remove finalized messages from tips
            if let Ok(tips) = self.storage.get_tips().await {
                if tips.contains(&msg_id) {
                    self.tip_manager.remove_tip(&msg_id).await;
                }
            }
        }

        // Emit k-core metrics after finality checks (throttled, or immediately if messages were finalized)
        if has_finalized || self.should_emit_kcore_metrics() {
            self.emit_kcore_event().await;
        }

        Ok(())
    }

    pub async fn update_tips(&self) -> Result<()> {
        // First clear the TipManager
        self.tip_manager.clear().await;

        // Get current tips from storage
        let tips = self
            .storage
            .get_tips()
            .await
            .map_err(|e| anyhow::anyhow!(e))?;

        // Add all tips from storage to TipManager
        for tip in tips.clone() {
            self.tip_manager.add_tip(tip, 1.0).await;
        }

        // Emit tips updated event (throttled)
        if self.should_emit_tips_metrics() {
            self.event_bus.emit(NodeEvent::TipsUpdated {
                tips: tips.iter().map(|id| hex::encode(id.as_bytes())).collect(),
                count: tips.len(),
                timestamp: Utc::now(),
            });

            // Calculate and emit diversity metrics when tips are emitted
            if self.should_emit_diversity_metrics() {
                self.emit_diversity_event(&tips).await;
            }
        }

        Ok(())
    }

    /// Initialize tips from storage (useful for tests)
    #[allow(dead_code)]
    pub async fn sync_tips_from_storage(&self) -> Result<()> {
        self.update_tips().await
    }

    /// Emit diversity metrics event based on current tips
    async fn emit_diversity_event(&self, tips: &[MessageId]) {
        use adic_math::ball_id;
        use std::collections::HashSet;

        let params = self.consensus.params().await;
        let mut axes_data = Vec::new();

        // Calculate diversity for each axis
        for (axis_idx, &radius) in params.rho.iter().enumerate() {
            let mut distinct_balls = HashSet::new();

            for tip_id in tips {
                if let Ok(Some(msg)) = self.storage.get_message(tip_id).await {
                    if let Some(axis_phi) =
                        msg.features.get_axis(adic_types::AxisId(axis_idx as u32))
                    {
                        let ball = ball_id(&axis_phi.qp_digits, radius as usize);
                        distinct_balls.insert(ball);
                    }
                }
            }

            let meets_requirement = distinct_balls.len() >= params.q as usize;
            axes_data.push(AxisData {
                axis: axis_idx as u32,
                radius,
                distinct_balls: distinct_balls.len(),
                required_diversity: params.q as usize,
                meets_requirement,
            });
        }

        // Calculate overall diversity score
        let compliant_axes = axes_data.iter().filter(|a| a.meets_requirement).count();
        let diversity_score = if !axes_data.is_empty() {
            (compliant_axes as f64) / (axes_data.len() as f64)
        } else {
            0.0
        };

        self.event_bus.emit(NodeEvent::DiversityUpdated {
            diversity_score,
            axes: axes_data,
            total_tips: tips.len(),
            timestamp: Utc::now(),
        });
    }

    /// Emit k-core finality metrics event
    async fn emit_kcore_event(&self) {
        let finality_stats = self.finality.get_stats().await;
        let params = self.consensus.params().await;

        self.event_bus.emit(NodeEvent::KCoreUpdated {
            finalized_count: finality_stats.finalized_count as u32,
            pending_count: finality_stats.pending_count as u32,
            current_k_value: Some(params.k),
            timestamp: Utc::now(),
        });
    }

    /// Emit energy conflict metrics event
    pub async fn emit_energy_event(&self) {
        let metrics = self.consensus.energy_tracker.get_metrics().await;
        let active_conflicts = metrics
            .total_conflicts
            .saturating_sub(metrics.resolved_conflicts);

        self.event_bus.emit(NodeEvent::EnergyUpdated {
            total_conflicts: metrics.total_conflicts as u32,
            resolved_conflicts: metrics.resolved_conflicts as u32,
            active_conflicts: active_conflicts as u32,
            total_energy: metrics.total_energy,
            timestamp: Utc::now(),
        });
    }

    fn should_apply_reputation_decay(&self) -> bool {
        // Apply decay every hour
        use std::sync::atomic::{AtomicU64, Ordering};
        static LAST_DECAY: AtomicU64 = AtomicU64::new(0);

        let now = chrono::Utc::now().timestamp() as u64;
        let last = LAST_DECAY.load(Ordering::Relaxed);

        if now - last > 3600 {
            LAST_DECAY.store(now, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn should_cleanup_conflicts(&self) -> bool {
        // Cleanup conflicts every 10 minutes
        use std::sync::atomic::{AtomicU64, Ordering};
        static LAST_CLEANUP: AtomicU64 = AtomicU64::new(0);

        let now = chrono::Utc::now().timestamp() as u64;
        let last = LAST_CLEANUP.load(Ordering::Relaxed);

        if now - last > 600 {
            LAST_CLEANUP.store(now, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn should_cleanup_stale_peers(&self) -> bool {
        // Cleanup stale peers every 60 seconds
        use std::sync::atomic::{AtomicU64, Ordering};
        static LAST_PEER_CLEANUP: AtomicU64 = AtomicU64::new(0);

        let now = chrono::Utc::now().timestamp() as u64;
        let last = LAST_PEER_CLEANUP.load(Ordering::Relaxed);

        if now - last > 60 {
            LAST_PEER_CLEANUP.store(now, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn should_emit_energy_metrics(&self) -> bool {
        // Emit energy metrics every 5 seconds
        use std::sync::atomic::{AtomicU64, Ordering};
        static LAST_EMIT: AtomicU64 = AtomicU64::new(0);

        let now = chrono::Utc::now().timestamp() as u64;
        let last = LAST_EMIT.load(Ordering::Relaxed);

        if now - last > 5 {
            LAST_EMIT.store(now, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn should_emit_admissibility_metrics(&self) -> bool {
        // Emit admissibility metrics every 30 seconds
        use std::sync::atomic::{AtomicU64, Ordering};
        static LAST_EMIT: AtomicU64 = AtomicU64::new(0);

        let now = chrono::Utc::now().timestamp() as u64;
        let last = LAST_EMIT.load(Ordering::Relaxed);

        if now - last > 30 {
            LAST_EMIT.store(now, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn should_emit_tips_metrics(&self) -> bool {
        // Emit tips metrics every 2 seconds
        use std::sync::atomic::{AtomicU64, Ordering};
        static LAST_EMIT: AtomicU64 = AtomicU64::new(0);

        let now = chrono::Utc::now().timestamp() as u64;
        let last = LAST_EMIT.load(Ordering::Relaxed);

        if now - last > 2 {
            LAST_EMIT.store(now, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn should_emit_kcore_metrics(&self) -> bool {
        // Emit k-core metrics every 5 seconds
        use std::sync::atomic::{AtomicU64, Ordering};
        static LAST_EMIT: AtomicU64 = AtomicU64::new(0);

        let now = chrono::Utc::now().timestamp() as u64;
        let last = LAST_EMIT.load(Ordering::Relaxed);

        if now - last > 5 {
            LAST_EMIT.store(now, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn should_emit_diversity_metrics(&self) -> bool {
        // Emit diversity metrics every 5 seconds
        use std::sync::atomic::{AtomicU64, Ordering};
        static LAST_EMIT: AtomicU64 = AtomicU64::new(0);

        let now = chrono::Utc::now().timestamp() as u64;
        let last = LAST_EMIT.load(Ordering::Relaxed);

        if now - last > 5 {
            LAST_EMIT.store(now, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// Emit admissibility compliance metrics event
    async fn emit_admissibility_event(&self) {
        // Sample recent tips for admissibility analysis
        let tips = match self.storage.get_tips().await {
            Ok(tips) => tips,
            Err(_) => return,
        };

        let mut total_checks = 0;
        let mut c1_passes = 0;
        let mut c2_passes = 0;
        let mut c3_passes = 0;
        let mut fully_admissible = 0;

        // Sample up to 50 tips for performance
        for tip_id in tips.iter().take(50) {
            if let Ok(Some(msg)) = self.storage.get_message(tip_id).await {
                let mut parent_features = Vec::new();
                let mut parent_reputations = Vec::new();

                for parent_id in &msg.parents {
                    if let Ok(Some(parent)) = self.storage.get_message(parent_id).await {
                        let features: Vec<QpDigits> = parent
                            .features
                            .phi
                            .iter()
                            .map(|axis_phi| axis_phi.qp_digits.clone())
                            .collect();
                        parent_features.push(features);

                        let rep = self
                            .consensus
                            .reputation
                            .get_reputation(&parent.proposer_pk)
                            .await;
                        parent_reputations.push(rep);
                    }
                }

                if let Ok(result) = self.consensus.admissibility().check_message(
                    &msg,
                    &parent_features,
                    &parent_reputations,
                ) {
                    total_checks += 1;
                    if result.score_passed {
                        c1_passes += 1;
                    }
                    if result.c2_passed {
                        c2_passes += 1;
                    }
                    if result.c3_passed {
                        c3_passes += 1;
                    }
                    if result.is_admissible {
                        fully_admissible += 1;
                    }
                }
            }
        }

        if total_checks > 0 {
            let c1_rate = (c1_passes as f64) / (total_checks as f64);
            let c2_rate = (c2_passes as f64) / (total_checks as f64);
            let c3_rate = (c3_passes as f64) / (total_checks as f64);
            let overall_rate = (fully_admissible as f64) / (total_checks as f64);

            self.event_bus.emit(NodeEvent::AdmissibilityUpdated {
                c1_rate,
                c2_rate,
                c3_rate,
                overall_rate,
                sample_size: total_checks,
                timestamp: Utc::now(),
            });
        }
    }

    /// Update DAG state metrics (tips count and total messages count)
    async fn update_dag_metrics(&self) {
        // Get storage stats to update both tips and messages count
        if let Ok(stats) = self.storage.get_stats().await {
            self.metrics.current_tips.set(stats.tip_count as i64);
            self.metrics.dag_messages.set(stats.message_count as i64);
        }
    }

    // ========== Governance Methods ==========

    /// Submit a governance proposal
    ///
    /// Creates a proposal message and submits it to the network.
    /// Requires governance_manager and governance_protocol to be configured.
    pub async fn submit_governance_proposal(
        &self,
        param_keys: Vec<String>,
        new_values: serde_json::Value,
        class: adic_types::ProposalClass,
        rationale_cid: String,
        enact_epoch: u64,
    ) -> Result<([u8; 32], MessageId)> {
        let gov_manager = self
            .governance_manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Governance manager not configured"))?;

        let gov_protocol = self
            .governance_protocol
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Governance protocol not configured"))?;

        // Convert adic_types::ProposalClass to adic_governance::types::ProposalClass
        let gov_class = match class {
            adic_types::ProposalClass::Constitutional => adic_governance::types::ProposalClass::Constitutional,
            adic_types::ProposalClass::Operational => adic_governance::types::ProposalClass::Operational,
        };

        // Compute proposal ID using canonical JSON for deterministic hashing
        use adic_types::canonical_hash;
        use serde::Serialize;

        #[derive(Serialize)]
        struct ProposalCanonical<'a> {
            proposer_pk: &'a [u8; 32],
            param_keys: &'a [String],
            new_values: &'a serde_json::Value,
            class: &'a str,
            enact_epoch: u64,
            rationale_cid: &'a str,
        }

        let canonical = ProposalCanonical {
            proposer_pk: self.keypair.public_key().as_bytes(),
            param_keys: &param_keys,
            new_values: &new_values,
            class: match class {
                adic_types::ProposalClass::Constitutional => "constitutional",
                adic_types::ProposalClass::Operational => "operational",
            },
            enact_epoch,
            rationale_cid: &rationale_cid,
        };

        let proposal_id = canonical_hash(&canonical)
            .map_err(|e| anyhow::anyhow!("Failed to compute proposal ID: {}", e))?;

        // Calculate voting end timestamp (voting_duration from config)
        let voting_duration = std::time::Duration::from_secs(7 * 24 * 3600); // 7 days
        let voting_end_timestamp = chrono::Utc::now() + chrono::Duration::from_std(voting_duration)?;

        // Create proposal struct
        let proposal = adic_governance::types::GovernanceProposal {
            proposal_id,
            class: gov_class,
            proposer_pk: *self.keypair.public_key(),
            param_keys: param_keys.clone(),
            new_values: new_values.clone(),
            axis_changes: None, // AxisCatalogChange type
            treasury_grant: None,
            enact_epoch,
            rationale_cid: rationale_cid.clone(),
            creation_timestamp: chrono::Utc::now(),
            voting_end_timestamp,
            status: adic_governance::types::ProposalStatus::Voting,
            tally_yes: 0.0,
            tally_no: 0.0,
            tally_abstain: 0.0,
        };

        // Submit to governance manager
        gov_manager.submit_proposal(proposal.clone()).await?;

        // Serialize proposal to MessageContent
        use adic_types::{GovernanceProposalContent, MessageContent};
        let content = MessageContent::GovernanceProposal(Box::new(GovernanceProposalContent {
            proposal_id,
            class, // Use original adic_types::ProposalClass
            param_keys: param_keys.clone(),
            new_values: proposal.new_values.clone(),
            axis_changes: None, // TODO: convert AxisCatalogChange if needed
            enact_epoch,
            rationale_cid: proposal.rationale_cid.clone(),
            submitter_pk: *self.keypair.public_key(),
        }));

        let content_bytes = content.to_bytes()?;

        // Submit message to DAG
        let message_id = self.submit_message(content_bytes).await?;

        // Track proposal in protocol
        let gov_message = adic_network::protocol::GovernanceMessage::Proposal {
            message_id,
            proposal_id,
            submitter: *self.keypair.public_key(),
            class: format!("{:?}", class),
            param_keys: param_keys.clone(),
            timestamp: chrono::Utc::now().timestamp() as u64,
        };

        gov_protocol.handle_message(gov_message).await?;

        // Get proposer reputation for event
        let proposer_reputation = self
            .consensus
            .reputation
            .get_reputation(&self.keypair.public_key())
            .await;

        // Emit event
        self.event_bus.emit(NodeEvent::ProposalSubmitted {
            proposal_id: hex::encode(&proposal_id[..8]),
            proposer: hex::encode(self.keypair.public_key().as_bytes()),
            proposal_class: format!("{:?}", class),
            param_keys: param_keys.clone(),
            enact_epoch,
            proposer_reputation,
            timestamp: Utc::now(),
        });

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            "📋 Governance proposal submitted"
        );

        Ok((proposal_id, message_id))
    }

    /// Cast a vote on a governance proposal
    pub async fn vote_on_proposal(
        &self,
        proposal_id: [u8; 32],
        ballot: adic_types::Ballot,
    ) -> Result<MessageId> {
        let gov_manager = self
            .governance_manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Governance manager not configured"))?;

        let gov_protocol = self
            .governance_protocol
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Governance protocol not configured"))?;

        // Get reputation to compute voting credits
        let reputation = self
            .consensus
            .reputation
            .get_reputation(&self.keypair.public_key())
            .await;

        // Compute voting credits (w(P) = sqrt(min(R(P), Rmax)))
        let rmax = 100_000.0; // Should come from config
        let credits = (reputation.min(rmax)).sqrt();

        // Convert adic_types::Ballot to adic_governance::types::Ballot
        let gov_ballot = match ballot {
            adic_types::Ballot::Yes => adic_governance::types::Ballot::Yes,
            adic_types::Ballot::No => adic_governance::types::Ballot::No,
            adic_types::Ballot::Abstain => adic_governance::types::Ballot::Abstain,
        };

        // Create vote struct
        let vote = adic_governance::types::GovernanceVote {
            proposal_id,
            voter_pk: *self.keypair.public_key(),
            credits,
            ballot: gov_ballot,
            timestamp: chrono::Utc::now(),
        };

        // Submit vote to governance manager
        gov_manager.cast_vote(vote.clone()).await?;

        // Serialize vote to MessageContent
        use adic_types::{GovernanceVoteContent, MessageContent};
        let content = MessageContent::GovernanceVote(Box::new(GovernanceVoteContent {
            proposal_id,
            voter_pk: *self.keypair.public_key(),
            credits,
            ballot,
            epoch: *self.current_epoch.read().await,
        }));

        let content_bytes = content.to_bytes()?;

        // Submit message to DAG
        let message_id = self.submit_message(content_bytes).await?;

        // Track vote in protocol
        let gov_message = adic_network::protocol::GovernanceMessage::Vote {
            message_id,
            proposal_id,
            voter: *self.keypair.public_key(),
            ballot: format!("{:?}", ballot),
            credits,
            timestamp: chrono::Utc::now().timestamp() as u64,
        };

        gov_protocol.handle_message(gov_message).await?;

        // Emit event
        self.event_bus.emit(NodeEvent::VoteCast {
            proposal_id: hex::encode(&proposal_id[..8]),
            voter: hex::encode(self.keypair.public_key().as_bytes()),
            vote_credits: credits,
            ballot: format!("{:?}", ballot),
            voter_reputation: reputation,
            timestamp: Utc::now(),
        });

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            ballot = ?ballot,
            credits = credits,
            "🗳️ Vote cast on proposal"
        );

        Ok(message_id)
    }

    /// Finalize governance voting and emit receipt to DAG
    ///
    /// This method handles the full workflow:
    /// 1. Finalize voting via lifecycle manager (tallies votes, creates BLS-signed receipt)
    /// 2. Retrieve the created receipt
    /// 3. Emit receipt to DAG as governance message
    /// 4. Track receipt in governance protocol
    ///
    /// Returns the vote result and the receipt message ID
    #[allow(dead_code)] // Used in tests and available for governance operations
    pub async fn finalize_governance_voting(
        &self,
        proposal_id: &[u8; 32],
    ) -> Result<(adic_governance::types::VoteResult, MessageId)> {
        // Get governance manager
        let manager = self
            .governance_manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Governance not enabled"))?;

        // Finalize voting (this creates the receipt with BLS signature)
        let result = manager.finalize_voting(proposal_id).await?;

        // Get the created receipt
        let receipts = manager.get_receipts(proposal_id).await;
        let receipt = receipts
            .last()
            .ok_or_else(|| anyhow::anyhow!("Receipt not found after finalization"))?;

        // Get current epoch from finality engine
        let current_epoch = self.finality.current_epoch().await;

        // Emit receipt to DAG
        let message_id = self.emit_governance_receipt(receipt, current_epoch).await?;

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            result = ?result,
            message_id = ?message_id,
            "✅ Governance voting finalized and receipt emitted"
        );

        Ok((result, message_id))
    }

    /// Emit governance receipt to DAG with BLS signature
    #[allow(dead_code)] // Used by finalize_governance_voting
    pub async fn emit_governance_receipt(
        &self,
        receipt: &adic_governance::types::GovernanceReceipt,
        finalized_epoch: u64,
    ) -> Result<MessageId> {
        use adic_types::{GovernanceReceiptContent, MessageContent, QuorumStats as TypesQuorumStats, VoteResult as TypesVoteResult};

        // Convert BLS signature to bytes
        let threshold_bls_sig = receipt
            .quorum_signature
            .as_ref()
            .map(|sig| hex::decode(sig.as_hex()).unwrap_or_default())
            .unwrap_or_default();

        // Create MessageContent for receipt
        let content = MessageContent::GovernanceReceipt(Box::new(GovernanceReceiptContent {
            proposal_id: receipt.proposal_id,
            quorum_stats: TypesQuorumStats {
                total_eligible_credits: 0.0, // TODO: Track this in governance
                credits_voted: receipt.quorum_stats.total_participation,
                credits_yes: receipt.quorum_stats.yes,
                credits_no: receipt.quorum_stats.no,
                credits_abstain: receipt.quorum_stats.abstain,
                participation_rate: 0.0, // Will be calculated from credits_voted/total_eligible
            },
            result: match receipt.result {
                adic_governance::types::VoteResult::Pass => TypesVoteResult::Pass,
                adic_governance::types::VoteResult::Fail => TypesVoteResult::Fail,
            },
            receipt_seq: receipt.receipt_seq,
            prev_receipt_hash: receipt.prev_receipt_hash,
            threshold_bls_sig: threshold_bls_sig.clone(),
            finalized_epoch,
        }));

        let content_bytes = content.to_bytes()?;

        // Submit receipt message to DAG
        let message_id = self.submit_message(content_bytes).await?;

        // Track receipt in governance protocol
        if let Some(ref gov_protocol) = self.governance_protocol {
            let gov_message = adic_network::protocol::GovernanceMessage::Receipt {
                message_id,
                proposal_id: receipt.proposal_id,
                result: format!("{:?}", receipt.result).to_lowercase(),
                credits_yes: receipt.quorum_stats.yes,
                credits_no: receipt.quorum_stats.no,
                threshold_sig: threshold_bls_sig,
                timestamp: receipt.timestamp.timestamp() as u64,
            };

            gov_protocol.handle_message(gov_message).await?;
        }

        // Emit event
        self.event_bus.emit(NodeEvent::GovernanceReceiptEmitted {
            proposal_id: hex::encode(&receipt.proposal_id[..8]),
            result: format!("{:?}", receipt.result),
            quorum_yes: receipt.quorum_stats.yes,
            quorum_no: receipt.quorum_stats.no,
            quorum_abstain: receipt.quorum_stats.abstain,
            has_bls_signature: receipt.quorum_signature.is_some(),
            committee_size: receipt.committee_members.len(),
            timestamp: Utc::now(),
        });

        info!(
            proposal_id = hex::encode(&receipt.proposal_id[..8]),
            result = ?receipt.result,
            has_bls = receipt.quorum_signature.is_some(),
            committee_members = receipt.committee_members.len(),
            "📜 Governance receipt emitted to DAG"
        );

        Ok(message_id)
    }

    /// Initiate DKG ceremony for a new epoch
    ///
    /// Determines committee membership, starts the ceremony via DKG orchestrator,
    /// and broadcasts commitment messages to committee members via gossip.
    async fn initiate_dkg_ceremony(&self, epoch_id: u64) -> Result<()> {
        // Check if DKG orchestrator is configured
        let dkg_orchestrator = match &self.dkg_orchestrator {
            Some(orchestrator) => orchestrator,
            None => {
                debug!("DKG orchestrator not configured, skipping ceremony initialization");
                return Ok(());
            }
        };

        // Check if network is enabled for message propagation
        let network = match &self.network {
            Some(net) => net,
            None => {
                warn!("Network not configured, cannot run DKG ceremony");
                return Ok(());
            }
        };

        // Select committee members using VRF-based quorum selection
        // This provides unpredictable, reputation-weighted committee formation
        // with diversity enforcement (ASN/region caps)

        // Get eligible nodes from network with metadata
        let eligible_nodes = match self.get_eligible_quorum_nodes().await {
            Ok(nodes) => nodes,
            Err(e) => {
                warn!(epoch_id, error = %e, "Failed to get eligible nodes for committee");
                return Ok(());
            }
        };

        if eligible_nodes.is_empty() {
            debug!(epoch_id, "No eligible nodes available for DKG committee");
            return Ok(());
        }

        // Configure committee selection parameters
        let config = adic_quorum::QuorumConfig {
            min_reputation: 50.0,      // Minimum reputation for eligibility
            members_per_axis: 5,       // 5 members per axis (3 axes = 15 total)
            total_size: 15,            // DKG committee size
            max_per_asn: 2,            // Max 2 members from same ASN
            max_per_region: 3,         // Max 3 members from same region
            domain_separator: "dkg_committee".to_string(),
            num_axes: 3,               // 3-axis selection per ADIC-DAG
        };

        // Use VRF-based selection for unpredictable, fair committee formation
        let quorum_result = match self.quorum_selector.select_committee(
            epoch_id,
            &config,
            eligible_nodes,
        ).await {
            Ok(result) => result,
            Err(e) => {
                warn!(
                    epoch_id,
                    error = %e,
                    "Failed to select committee via VRF - skipping DKG ceremony"
                );
                return Ok(());
            }
        };

        // Extract PublicKeys for DKG ceremony
        let committee_members: Vec<PublicKey> = quorum_result
            .members
            .iter()
            .map(|m| m.public_key)
            .collect();

        if committee_members.is_empty() {
            debug!(epoch_id, "No committee members selected after diversity enforcement");
            return Ok(());
        }

        info!(
            epoch_id,
            committee_size = committee_members.len(),
            "🔐 Initiating DKG ceremony for new epoch"
        );

        // Start DKG ceremony via orchestrator
        let dkg_messages = dkg_orchestrator
            .start_ceremony(epoch_id, committee_members)
            .await?;

        if dkg_messages.is_empty() {
            debug!(epoch_id, "Not a committee member, no DKG messages to send");
            return Ok(());
        }

        // Broadcast DKG commitment via gossip protocol
        let network_guard = network.read().await;
        let gossip = network_guard.get_gossip_protocol();
        for (_peer_id, dkg_msg) in dkg_messages {
            // Use gossip broadcast instead of direct peer messaging
            if let Err(e) = gossip.broadcast_dkg_message(dkg_msg).await {
                warn!(
                    epoch_id,
                    error = %e,
                    "Failed to broadcast DKG message"
                );
            }
        }

        info!(
            epoch_id,
            "📤 DKG commitment broadcasted via gossip"
        );

        Ok(())
    }

    /// Handle incoming governance message from network
    #[allow(dead_code)] // Used in tests and available for governance protocol
    pub async fn handle_governance_message(
        &self,
        content: adic_types::MessageContent,
        message_id: MessageId,
    ) -> Result<()> {
        let gov_protocol = self
            .governance_protocol
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Governance protocol not configured"))?;

        match content {
            adic_types::MessageContent::GovernanceProposal(proposal) => {
                let gov_message = adic_network::protocol::GovernanceMessage::Proposal {
                    message_id,
                    proposal_id: proposal.proposal_id,
                    submitter: proposal.submitter_pk,
                    class: format!("{:?}", proposal.class),
                    param_keys: proposal.param_keys,
                    timestamp: chrono::Utc::now().timestamp() as u64,
                };
                gov_protocol.handle_message(gov_message).await?;
            }
            adic_types::MessageContent::GovernanceVote(vote) => {
                let gov_message = adic_network::protocol::GovernanceMessage::Vote {
                    message_id,
                    proposal_id: vote.proposal_id,
                    voter: vote.voter_pk,
                    ballot: format!("{:?}", vote.ballot),
                    credits: vote.credits,
                    timestamp: vote.epoch,
                };
                gov_protocol.handle_message(gov_message).await?;
            }
            adic_types::MessageContent::GovernanceReceipt(receipt) => {
                let gov_message = adic_network::protocol::GovernanceMessage::Receipt {
                    message_id,
                    proposal_id: receipt.proposal_id,
                    result: format!("{:?}", receipt.result),
                    credits_yes: receipt.quorum_stats.credits_yes,
                    credits_no: receipt.quorum_stats.credits_no,
                    threshold_sig: receipt.threshold_bls_sig,
                    timestamp: receipt.finalized_epoch,
                };
                gov_protocol.handle_message(gov_message).await?;
            }
            _ => {
                warn!("Unexpected governance message type");
            }
        }

        Ok(())
    }

    /// Handle incoming DKG message from gossip protocol
    ///
    /// Routes DKG messages (commitments, shares, completion) to the DKG orchestrator,
    /// which manages the distributed key generation ceremony lifecycle.
    #[allow(dead_code)] // Used in tests and available for DKG protocol
    pub async fn handle_dkg_message(
        &self,
        dkg_msg: adic_network::protocol::DKGMessage,
        from_peer: libp2p::PeerId,
    ) -> Result<()> {
        let dkg_orchestrator = match &self.dkg_orchestrator {
            Some(orchestrator) => orchestrator,
            None => {
                debug!("DKG orchestrator not configured, ignoring DKG message");
                return Ok(());
            }
        };

        let network = match &self.network {
            Some(net) => net,
            None => {
                warn!("Network not configured, cannot propagate DKG responses");
                return Ok(());
            }
        };

        match dkg_msg {
            adic_network::protocol::DKGMessage::Commitment { epoch_id, commitment } => {
                debug!(
                    epoch_id,
                    participant_id = commitment.participant_id,
                    from_peer = %from_peer,
                    "🔐 Processing DKG commitment"
                );

                // Handle commitment and get response messages (if ready for share exchange)
                let response_messages = dkg_orchestrator
                    .handle_commitment(epoch_id, commitment, from_peer)
                    .await?;

                // Broadcast share messages via gossip
                if !response_messages.is_empty() {
                    let network_guard = network.read().await;
                    let gossip = network_guard.get_gossip_protocol();
                    for (_peer_id, dkg_msg) in response_messages {
                        if let Err(e) = gossip.broadcast_dkg_message(dkg_msg).await {
                            warn!(
                                epoch_id,
                                error = %e,
                                "Failed to broadcast DKG share"
                            );
                        }
                    }
                }
            }
            adic_network::protocol::DKGMessage::Share { epoch_id, share } => {
                debug!(
                    epoch_id,
                    from_participant = share.from,
                    to_participant = share.to,
                    from_peer = %from_peer,
                    "🔐 Processing DKG share"
                );

                // Handle share and get completion messages (if ceremony is done)
                if let Some(completion_messages) = dkg_orchestrator
                    .handle_share(epoch_id, share, from_peer)
                    .await?
                {
                    // Broadcast completion messages via gossip
                    let network_guard = network.read().await;
                    let gossip = network_guard.get_gossip_protocol();
                    for (_peer_id, dkg_msg) in completion_messages {
                        if let Err(e) = gossip.broadcast_dkg_message(dkg_msg).await {
                            warn!(
                                epoch_id,
                                error = %e,
                                "Failed to broadcast DKG completion"
                            );
                        }
                    }

                    info!(epoch_id, "✨ DKG ceremony completed successfully");
                }
            }
            adic_network::protocol::DKGMessage::Complete {
                epoch_id,
                participant_id,
            } => {
                debug!(
                    epoch_id,
                    participant_id,
                    from_peer = %from_peer,
                    "✅ DKG completion notification received"
                );

                dkg_orchestrator
                    .handle_complete(epoch_id, participant_id, from_peer)
                    .await;
            }
        }

        Ok(())
    }
}

impl Clone for AdicNode {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            wallet: Arc::clone(&self.wallet),
            wallet_registry: Arc::clone(&self.wallet_registry),
            keypair: self.keypair.clone(),
            event_bus: Arc::clone(&self.event_bus),
            consensus: Arc::clone(&self.consensus),
            mrw: Arc::clone(&self.mrw),
            storage: Arc::clone(&self.storage),
            finality: Arc::clone(&self.finality),
            economics: Arc::clone(&self.economics),
            network: self.network.as_ref().map(Arc::clone),
            update_manager: self.update_manager.as_ref().map(Arc::clone),
            vrf_service: Arc::clone(&self.vrf_service),
            quorum_selector: Arc::clone(&self.quorum_selector),
            parameter_store: self.parameter_store.as_ref().map(Arc::clone),
            metrics: Arc::clone(&self.metrics),
            network_metadata: Arc::clone(&self.network_metadata),
            dkg_manager: self.dkg_manager.as_ref().map(Arc::clone),
            bls_coordinator: self.bls_coordinator.as_ref().map(Arc::clone),
            dkg_orchestrator: self.dkg_orchestrator.as_ref().map(Arc::clone),
            task_manager: self.task_manager.as_ref().map(Arc::clone),
            governance_manager: self.governance_manager.as_ref().map(Arc::clone),
            governance_protocol: self.governance_protocol.as_ref().map(Arc::clone),
            storage_market: self.storage_market.as_ref().map(Arc::clone),
            index: Arc::clone(&self.index),
            tip_manager: Arc::clone(&self.tip_manager),
            running: Arc::clone(&self.running),
            encoders: Arc::clone(&self.encoders),
            current_epoch: Arc::clone(&self.current_epoch),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NodeStats {
    pub message_count: usize,
    pub tip_count: usize,
    pub finalized_count: usize,
    pub pending_finality: usize,
}
