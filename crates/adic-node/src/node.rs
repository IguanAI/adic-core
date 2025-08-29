use crate::config::NodeConfig;
use adic_consensus::ConsensusEngine;
use adic_crypto::Keypair;
use adic_economics::EconomicsEngine;
use adic_finality::{FinalityConfig, FinalityEngine};
use adic_mrw::MrwEngine;
use adic_network::{NetworkConfig, NetworkEngine};
use adic_storage::store::BackendType;
use adic_storage::{MessageIndex, StorageConfig, StorageEngine, TipManager};
use adic_types::{AdicFeatures, AdicMessage, AdicMeta, AxisPhi, MessageId, QpDigits};
use anyhow::Result;
use chrono::Utc;
use libp2p::identity::Keypair as LibP2pKeypair;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

pub struct AdicNode {
    config: NodeConfig,
    keypair: Keypair,
    pub consensus: Arc<ConsensusEngine>,
    pub mrw: Arc<MrwEngine>,
    pub storage: Arc<StorageEngine>,
    pub finality: Arc<FinalityEngine>,
    pub economics: Arc<EconomicsEngine>,
    pub network: Option<Arc<RwLock<NetworkEngine>>>,
    index: Arc<MessageIndex>,
    tip_manager: Arc<TipManager>,
    running: Arc<RwLock<bool>>,
}

impl AdicNode {
    pub async fn new(config: NodeConfig) -> Result<Self> {
        info!("Initializing ADIC node...");

        // Load or generate keypair
        let keypair = if let Some(key_path) = &config.node.keypair_path {
            let key_bytes = std::fs::read(key_path)?;
            Keypair::from_bytes(&key_bytes)?
        } else {
            info!("Generating new keypair");
            Keypair::generate()
        };

        info!(
            "Node public key: {}",
            hex::encode(keypair.public_key().as_bytes())
        );

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
                        warn!("RocksDB backend requested but feature not enabled, falling back to memory");
                        BackendType::Memory
                    }
                }
                "memory" => BackendType::Memory,
                _ => {
                    warn!(
                        "Unknown storage backend '{}', falling back to memory",
                        config.storage.backend
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
        let consensus = Arc::new(ConsensusEngine::new(adic_params.clone()));

        // Create MRW engine
        let mrw = Arc::new(MrwEngine::new(adic_params.clone()));

        // Create finality engine
        let finality_config = FinalityConfig::from(&adic_params);
        let finality = Arc::new(FinalityEngine::new(finality_config, consensus.clone()));

        // Create economics engine
        let economics_storage = Arc::new(adic_economics::storage::MemoryStorage::new());
        let economics = Arc::new(EconomicsEngine::new(economics_storage).await?);

        // Initialize genesis allocation if needed
        if let Err(e) = economics.initialize_genesis().await {
            warn!("Failed to initialize genesis allocation: {}", e);
        } else {
            info!("Economics engine initialized with genesis allocation");
        }

        // Create indices
        let index = Arc::new(MessageIndex::new());
        let tip_manager = Arc::new(TipManager::new());

        // Initialize network if enabled
        let network = if config.network.enabled {
            info!("Initializing P2P network...");

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

            Some(Arc::new(RwLock::new(network)))
        } else {
            info!("P2P network disabled");
            None
        };

        Ok(Self {
            config,
            keypair,
            consensus,
            mrw,
            storage,
            finality,
            economics,
            network,
            index,
            tip_manager,
            running: Arc::new(RwLock::new(false)),
        })
    }

    pub fn node_id(&self) -> String {
        hex::encode(&self.keypair.public_key().as_bytes()[..8])
    }

    pub async fn run(&self) -> Result<()> {
        {
            let mut running = self.running.write().await;
            *running = true;
        }

        info!("Node started and running");

        // Start network engine if enabled
        if let Some(ref network) = self.network {
            info!("Starting P2P network engine...");
            network.read().await.start().await?;

            // Connect to bootstrap peers
            for peer in &self.config.network.bootstrap_peers {
                if let Ok(addr) = peer.parse::<SocketAddr>() {
                    info!("Connecting to bootstrap peer: {}", addr);
                    network.read().await.connect_peer(addr).await?;
                }
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

    pub async fn submit_message(&self, content: Vec<u8>) -> Result<MessageId> {
        debug!("Submitting new message");

        // 1. Get tips first to inform feature generation
        let tips = self.tip_manager.get_tips().await;

        // 2. Create features with good p-adic proximity to existing tips
        let features = if !tips.is_empty() {
            // Get a reference tip to base features on for better proximity
            if let Ok(Some(tip_msg)) = self.storage.get_message(&tips[0]).await {
                // Create features with good p-adic proximity to the tip
                let mut new_features = Vec::new();
                for (i, axis_phi) in tip_msg.features.phi.iter().enumerate() {
                    // Create a slightly modified version for proximity
                    // Just increment the least significant digit to maintain p-adic closeness
                    let mut new_digits = axis_phi.qp_digits.digits.clone();
                    if !new_digits.is_empty() {
                        new_digits[0] = (new_digits[0] + 1) % 3; // Increment in base 3
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

        // 3. Create the message
        let mut message = AdicMessage::new(
            parents.clone(),
            features,
            AdicMeta::new(Utc::now()),
            *self.keypair.public_key(),
            content,
        );

        // Sign the message before validation
        let signature = self.keypair.sign(&message.to_bytes());
        message.signature = signature;

        // 4. Escrow deposit
        let proposer_pk = *self.keypair.public_key();
        self.consensus
            .deposits
            .escrow(message.id, proposer_pk)
            .await?;

        // 5. Perform C1-C3 admissibility checks BEFORE validation
        // This is critical for consensus security
        if !parents.is_empty() {
            // Non-genesis messages must pass C1-C3
            let mut parent_features = Vec::new();
            let mut parent_reputations = Vec::new();

            for parent_id in &parents {
                if let Ok(Some(parent)) = self.storage.get_message(parent_id).await {
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
                } else {
                    warn!(
                        "Parent message {} not found",
                        hex::encode(parent_id.as_bytes())
                    );
                    return Err(anyhow::anyhow!("Missing parent message"));
                }
            }

            // Check C1-C3 admissibility
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
            return Err(anyhow::anyhow!(
                "Message failed validation: {:?}",
                validation_result.errors
            ));
        }
        // Store message
        self.storage.store_message(&message).await?;

        // Update indices
        let parent_depths: Vec<u32> = Vec::new(); // Would need to fetch actual depths
        self.index.add_message(&message, parent_depths).await;

        // Add to tip manager
        self.tip_manager.add_tip(message.id, 1.0).await;

        // Remove only the selected parents from tips
        for parent_id in &parents {
            self.tip_manager.remove_tip(parent_id).await;
        }

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

    async fn process_incoming_messages(&self) -> Result<()> {
        // Process messages from the network if enabled
        if let Some(ref network) = self.network {
            // Get pending messages from the network buffer
            let messages = network.read().await.get_pending_messages().await;

            for msg in messages {
                debug!(
                    "Processing message from network: {}",
                    hex::encode(msg.id.as_bytes())
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

        for msg_id in finalized {
            debug!("Message finalized: {}", hex::encode(msg_id.as_bytes()));

            // Update tip manager - remove finalized messages from tips
            if let Ok(tips) = self.storage.get_tips().await {
                if tips.contains(&msg_id) {
                    self.tip_manager.remove_tip(&msg_id).await;
                }
            }
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
        for tip in tips {
            self.tip_manager.add_tip(tip, 1.0).await;
        }

        Ok(())
    }

    /// Initialize tips from storage (useful for tests)
    #[allow(dead_code)]
    pub async fn sync_tips_from_storage(&self) -> Result<()> {
        self.update_tips().await
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
}

impl Clone for AdicNode {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            keypair: self.keypair.clone(),
            consensus: Arc::clone(&self.consensus),
            mrw: Arc::clone(&self.mrw),
            storage: Arc::clone(&self.storage),
            finality: Arc::clone(&self.finality),
            economics: Arc::clone(&self.economics),
            network: self.network.as_ref().map(Arc::clone),
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
