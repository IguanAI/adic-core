use crate::config::NodeConfig;
use adic_consensus::ConsensusEngine;
use adic_crypto::Keypair;
use adic_finality::{FinalityEngine, FinalityConfig};
use adic_mrw::MrwEngine;
use adic_storage::{StorageEngine, StorageConfig, MessageIndex, TipManager};
use adic_storage::store::BackendType;
use adic_types::{AdicMessage, MessageId, AdicFeatures, AdicMeta, AxisPhi, QpDigits};
use anyhow::Result;
use chrono::Utc;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, debug, warn};

pub struct AdicNode {
    config: NodeConfig,
    keypair: Keypair,
    consensus: Arc<ConsensusEngine>,
    pub mrw: Arc<MrwEngine>,
    storage: Arc<StorageEngine>,
    finality: Arc<FinalityEngine>,
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
        
        info!("Node public key: {}", hex::encode(keypair.public_key().as_bytes()));
        
        // Create storage engine
        let storage_config = StorageConfig {
            backend_type: BackendType::Memory, // For now, only memory backend
            // TODO: Add RocksDB support when available
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
        
        // Create indices
        let index = Arc::new(MessageIndex::new());
        let tip_manager = Arc::new(TipManager::new());
        
        Ok(Self {
            config,
            keypair,
            consensus,
            mrw,
            storage,
            finality,
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
        
        // Start finality checker
        self.finality.start_checker().await;
        
        // Main loop
        while *self.running.read().await {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            
            // Periodic tasks would go here
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

        // 1. Create features for the new message (simplified for now)
        let features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(Utc::now().timestamp() as u64, 3, 10)),
            AxisPhi::new(1, QpDigits::from_u64(0, 3, 10)), // Topic axis
            AxisPhi::new(2, QpDigits::from_u64(0, 3, 10)), // Region axis
        ]);

        // 2. Select d+1 parents using MRW
        let tips = self.tip_manager.get_tips().await;
        let parents = self.mrw.select_parents(&features, &tips, &self.storage, &self.consensus.conflicts()).await?;

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
        self.consensus.deposits.escrow(message.id, proposer_pk).await?;

        // 5. Validate the message, which will slash the deposit if invalid
        let validation_result = self.consensus.validate_and_slash(&message).await?;
        if !validation_result.is_valid {
            warn!("Message failed validation and was slashed: {:?}", validation_result.errors);
            return Err(anyhow::anyhow!("Message failed validation"));
        }

        // The admissibility check is now implicitly handled by mrw.select_parents
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
        
        self.finality.add_message(
            message.id,
            parents.clone(),  // Use selected parents, not all tips
            1.0, // Initial reputation
            ball_ids,
        ).await?;
        
        info!("Message submitted: {}", hex::encode(message.id.as_bytes()));
        
        Ok(message.id)
    }
    
    pub async fn get_message(&self, id: &MessageId) -> Result<Option<AdicMessage>> {
        self.storage.get_message(id).await.map_err(|e| anyhow::anyhow!(e))
    }
    
    pub async fn get_tips(&self) -> Result<Vec<MessageId>> {
        self.storage.get_tips().await.map_err(|e| anyhow::anyhow!(e))
    }
    
    pub async fn get_stats(&self) -> Result<NodeStats> {
        let storage_stats = self.storage.get_stats().await.map_err(|e| anyhow::anyhow!(e))?;
        let finality_stats = self.finality.get_stats().await;
        
        Ok(NodeStats {
            message_count: storage_stats.message_count,
            tip_count: storage_stats.tip_count,
            finalized_count: finality_stats.finalized_count,
            pending_finality: finality_stats.pending_count,
        })
    }
    
    pub async fn get_finality_artifact(&self, id: &MessageId) -> Option<adic_finality::artifact::FinalityArtifact> {
        self.finality.get_artifact(id).await
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