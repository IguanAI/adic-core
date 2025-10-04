use crate::config::NodeConfig;
use crate::events::{
    AxisData, BalanceChangeType, EventBus, FinalityType, NodeEvent, RejectionType,
};
use crate::genesis::{account_address_from_hex, GenesisManifest};
use crate::update_manager;
use crate::wallet::NodeWallet;
use crate::wallet_registry::WalletRegistry;
use adic_consensus::{ConsensusEngine, ReputationChangeEvent};
use adic_crypto::Keypair;
use adic_economics::balance::{BalanceChangeEvent, BalanceChangeTypeEvent};
use adic_economics::{AccountAddress, AdicAmount, EconomicsEngine};
use adic_finality::{FinalityConfig, FinalityEngine};
use adic_mrw::MrwEngine;
use adic_network::peer::PeerEvent;
use adic_network::sync::SyncEvent;
use adic_network::{NetworkConfig, NetworkEngine};
use adic_storage::store::BackendType;
use adic_storage::{MessageIndex, StorageConfig, StorageEngine, TipManager};
use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AxisId, AxisPhi, MessageId, PublicKey, QpDigits,
};
use anyhow::Result;
use chrono::Utc;
use libp2p::identity::Keypair as LibP2pKeypair;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

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
    index: Arc<MessageIndex>,
    tip_manager: Arc<TipManager>,
    running: Arc<RwLock<bool>>,
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

        // Initialize event bus early for real-time event streaming
        let event_bus = Arc::new(EventBus::new());
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
        let consensus = Arc::new(ConsensusEngine::new(adic_params.clone(), storage.clone()));

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
        let mrw = Arc::new(MrwEngine::new(adic_params.clone()));

        // Create finality engine
        let finality_config = FinalityConfig::from(&adic_params);
        let finality = Arc::new(FinalityEngine::new(
            finality_config,
            consensus.clone(),
            storage.clone(),
        ));

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

                // Verify genesis config
                genesis_config
                    .verify()
                    .map_err(|e| anyhow::anyhow!("Invalid genesis config: {}", e))?;

                // Calculate genesis hash for consistency
                let genesis_hash = genesis_config.calculate_hash();

                // Verify against canonical hash for mainnet
                if genesis_config.chain_id == "adic-dag-v1" {
                    let canonical_hash = GenesisManifest::canonical_hash();
                    if genesis_hash != canonical_hash {
                        return Err(anyhow::anyhow!(
                            "Genesis hash mismatch for mainnet (adic-dag-v1). Expected canonical hash {}, got {}",
                            canonical_hash,
                            genesis_hash
                        ));
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

            // Verify against canonical hash for mainnet
            if genesis_manifest.config.chain_id == "adic-dag-v1" {
                let canonical_hash = GenesisManifest::canonical_hash();
                if genesis_manifest.hash != canonical_hash {
                    return Err(anyhow::anyhow!(
                        "Genesis hash mismatch for mainnet (adic-dag-v1). Expected canonical hash {}, got {}",
                        canonical_hash,
                        genesis_manifest.hash
                    ));
                }
            }

            info!(
                genesis_hash = %genesis_manifest.hash,
                chain_id = %genesis_manifest.config.chain_id,
                "✅ Genesis loaded, verified, and matches canonical hash"
            );

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

            // Bridge P2P peer events to node events
            {
                let event_bus_clone = event_bus.clone();
                let network_clone = network_arc.clone();
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
                                        // Get peer info to extract address
                                        let address = {
                                            let net = network_clone.read().await;
                                            if let Some(peer_info) =
                                                net.peer_manager().get_peer(&peer_id).await
                                            {
                                                peer_info
                                                    .addresses
                                                    .first()
                                                    .map(|addr| addr.to_string())
                                                    .unwrap_or_else(|| "unknown".to_string())
                                            } else {
                                                "unknown".to_string()
                                            }
                                        };

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
            index,
            tip_manager,
            running: Arc::new(RwLock::new(false)),
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

    pub async fn run(&self) -> Result<()> {
        {
            let mut running = self.running.write().await;
            *running = true;
        }

        info!("Node started and running");

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

            // Update tips
            self.update_tips().await?;

            // Apply reputation decay periodically
            if self.should_apply_reputation_decay() {
                self.consensus.reputation.apply_decay().await;
            }

            // Cleanup old conflicts
            if self.should_cleanup_conflicts() {
                self.consensus
                    .conflicts()
                    .cleanup_resolved(0.5, 86400)
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
                // Fallback to timestamp-based if tip not found
                AdicFeatures::new(vec![
                    AxisPhi::new(
                        0,
                        QpDigits::from_u64((Utc::now().timestamp() % 243) as u64, 3, 10),
                    ),
                    AxisPhi::new(1, QpDigits::from_u64(0, 3, 10)),
                    AxisPhi::new(2, QpDigits::from_u64(0, 3, 10)),
                ])
            }
        } else {
            // No tips available, use simple features for genesis-like messages
            AdicFeatures::new(vec![
                AxisPhi::new(0, QpDigits::from_u64(1, 3, 10)),
                AxisPhi::new(1, QpDigits::from_u64(1, 3, 10)),
                AxisPhi::new(2, QpDigits::from_u64(1, 3, 10)),
            ])
        };

        // 3. Select d+1 parents using MRW
        let parents = self
            .mrw
            .select_parents(
                &features,
                &tips,
                &self.storage,
                self.consensus.conflicts(),
                &self.consensus.reputation,
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
        let deposit_amount = self.consensus.deposits.get_deposit_amount();
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
            // Genesis message - use simple default features
            AdicFeatures::new(vec![
                AxisPhi::new(0, QpDigits::from_u64(1, 3, 10)),
                AxisPhi::new(1, QpDigits::from_u64(1, 3, 10)),
                AxisPhi::new(2, QpDigits::from_u64(1, 3, 10)),
            ])
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
                for axis_idx in 0..3 {
                    let radius = params.rho.get(axis_idx).copied().unwrap_or(2) as usize;

                    // Collect features from all parents for this axis
                    let parent_axis_features: Vec<&QpDigits> = all_parent_features
                        .iter()
                        .filter_map(|pf| {
                            pf.get_axis(AxisId(axis_idx as u32)).map(|ap| &ap.qp_digits)
                        })
                        .collect();

                    if !parent_axis_features.is_empty() {
                        // Strategy: Find the common p-adic ball that contains all parents
                        // Use the first parent as a base, but ensure compatibility with all others
                        let base_feature = &parent_axis_features[0];
                        let mut new_digits = base_feature.digits.clone();

                        // Ensure we have enough digits for the ball radius
                        while new_digits.len() <= radius {
                            new_digits.push(0);
                        }

                        // For p-adic closeness to ALL parents:
                        // 1. Keep the first 'radius' digits to ensure we're in the same balls
                        // 2. Only modify digits beyond position 'radius'

                        // Check if all parents share the same ball (same first 'radius' digits)
                        let mut all_in_same_ball = true;
                        let check_len = radius.min(new_digits.len());
                        for parent_feature in &parent_axis_features {
                            let cmp_len = check_len.min(parent_feature.digits.len());
                            if parent_feature.digits[..cmp_len] != new_digits[..cmp_len] {
                                all_in_same_ball = false;
                                break;
                            }
                        }

                        if all_in_same_ball {
                            // All parents are in the same ball - safe to modify beyond radius
                            if new_digits.len() > radius {
                                let modify_pos = radius;
                                let old_digit = new_digits[modify_pos];
                                new_digits[modify_pos] = (new_digits[modify_pos] + 1) % 3;
                                log::debug!("Axis {}: Modified digit beyond radius at pos {} from {} to {} (radius={})", 
                                    axis_idx, modify_pos, old_digit, new_digits[modify_pos], radius);
                            }
                        } else {
                            // Parents are in different balls - use conservative approach
                            // Find the longest common prefix among all parents
                            let mut common_prefix_len = 0;
                            let max_check_len = parent_axis_features
                                .iter()
                                .map(|pf| pf.digits.len())
                                .min()
                                .unwrap_or(0);

                            for pos in 0..max_check_len {
                                let first_digit = parent_axis_features[0].digits[pos];
                                let all_same = parent_axis_features
                                    .iter()
                                    .all(|pf| pf.digits.get(pos) == Some(&first_digit));

                                if all_same {
                                    common_prefix_len = pos + 1;
                                } else {
                                    break;
                                }
                            }

                            // Use the common prefix, then add variation beyond it
                            new_digits = parent_axis_features[0].digits
                                [..common_prefix_len.min(new_digits.len())]
                                .to_vec();

                            // Ensure we have enough length and add variation safely
                            while new_digits.len() <= common_prefix_len {
                                new_digits.push(0);
                            }

                            if new_digits.len() > common_prefix_len {
                                let modify_pos = common_prefix_len;
                                new_digits[modify_pos] =
                                    (new_digits[modify_pos] + axis_idx as u8) % 3;
                                log::debug!("Axis {}: Used common prefix len={}, modified pos {} to {} for diversity", 
                                    axis_idx, common_prefix_len, modify_pos, new_digits[modify_pos]);
                            }
                        }

                        log::debug!(
                            "Axis {}: Generated digits={:?} (first 5) from {} parents",
                            axis_idx,
                            &new_digits[..5.min(new_digits.len())],
                            parent_axis_features.len()
                        );

                        let new_qp = QpDigits {
                            digits: new_digits,
                            p: base_feature.p,
                        };
                        new_features.push(AxisPhi::new(axis_idx as u32, new_qp));
                    } else {
                        // Fallback if no parent features found for this axis
                        let default_qp = QpDigits::from_u64(1 + axis_idx as u64, 3, 10);
                        new_features.push(AxisPhi::new(axis_idx as u32, default_qp));
                    }
                }

                AdicFeatures::new(new_features)
            } else {
                // Fallback to default if no parents found
                AdicFeatures::new(vec![
                    AxisPhi::new(0, QpDigits::from_u64(1, 3, 10)),
                    AxisPhi::new(1, QpDigits::from_u64(2, 3, 10)),
                    AxisPhi::new(2, QpDigits::from_u64(3, 3, 10)),
                ])
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
        let deposit_amount = self.consensus.deposits.get_deposit_amount();
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
                    FinalityGate::SSF => FinalityType::KCore,
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

        let params = self.consensus.params();
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
        let params = self.consensus.params();

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
            index: Arc::clone(&self.index),
            tip_manager: Arc::clone(&self.tip_manager),
            running: Arc::clone(&self.running),
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
