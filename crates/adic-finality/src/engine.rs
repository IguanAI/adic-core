use crate::{
    artifact::{FinalityArtifact, FinalityGate, FinalityParams, FinalityWitness},
    homology::{HomologyAnalyzer, HomologyResult},
    kcore::{KCoreAnalyzer, KCoreResult},
};
use adic_consensus::ConsensusEngine;
use adic_storage::StorageEngine;
use adic_types::{AdicParams, MessageId, Result};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration for finality engine
#[derive(Debug, Clone)]
pub struct FinalityConfig {
    pub k: usize,
    pub min_depth: u32,
    pub min_diversity: usize,
    pub min_reputation: f64,
    pub check_interval_ms: u64,
    pub window_size: usize,
}

impl From<&AdicParams> for FinalityConfig {
    fn from(params: &AdicParams) -> Self {
        Self {
            k: params.k as usize,
            min_depth: params.depth_star,
            min_diversity: params.q as usize,
            min_reputation: params.r_sum_min,
            check_interval_ms: 1000,
            window_size: 1000,
        }
    }
}

/// Main finality engine
pub struct FinalityEngine {
    config: FinalityConfig,
    analyzer: KCoreAnalyzer,
    homology: HomologyAnalyzer,
    storage: Arc<StorageEngine>,
    finalized: Arc<RwLock<HashMap<MessageId, FinalityArtifact>>>,
    pending: Arc<RwLock<VecDeque<MessageId>>>,
    consensus: Arc<ConsensusEngine>,
}

impl FinalityEngine {
    pub fn new(
        config: FinalityConfig,
        consensus: Arc<ConsensusEngine>,
        storage: Arc<StorageEngine>,
    ) -> Self {
        let analyzer = KCoreAnalyzer::new(
            config.k,
            config.min_depth,
            config.min_diversity,
            config.min_reputation,
            consensus.params().rho.clone(),
        );
        let homology = HomologyAnalyzer::new(consensus.params().clone());

        Self {
            config,
            analyzer,
            homology,
            storage,
            finalized: Arc::new(RwLock::new(HashMap::new())),
            pending: Arc::new(RwLock::new(VecDeque::new())),
            consensus,
        }
    }

    /// Add a message to the graph for finality tracking
    pub async fn add_message(
        &self,
        id: MessageId,
        _parents: Vec<MessageId>,
        _reputation: f64,
        _ball_ids: HashMap<u32, Vec<u8>>,
    ) -> Result<()> {
        // Add to homology complex for F2 analysis
        // self.homology
        //     .add_simplex(
        //         {
        //             let mut vertices = vec![id];
        //             vertices.extend(parents);
        //             vertices
        //         },
        //         reputation,
        //     )
        //     .await?;

        let mut pending = self.pending.write().await;
        pending.push_back(id);

        // Keep window size limited
        while pending.len() > self.config.window_size {
            pending.pop_front();
        }

        Ok(())
    }

    /// Check if a message is finalized
    pub async fn is_finalized(&self, id: &MessageId) -> bool {
        let finalized = self.finalized.read().await;
        finalized.contains_key(id)
    }

    /// Get finality artifact if available
    pub async fn get_artifact(&self, id: &MessageId) -> Option<FinalityArtifact> {
        let finalized = self.finalized.read().await;
        finalized.get(id).cloned()
    }

    /// Run finality checks on pending messages
    pub async fn check_finality(&self) -> Result<Vec<MessageId>> {
        let pending_list = {
            let pending = self.pending.read().await;
            pending.iter().cloned().collect::<Vec<_>>()
        };

        let mut newly_finalized = Vec::new();

        for msg_id in pending_list {
            if self.is_finalized(&msg_id).await {
                continue;
            }

            match self
                .analyzer
                .analyze(msg_id, &self.storage, &self.consensus.reputation)
                .await
            {
                Ok(result) if result.is_final => {
                    let artifact = self.create_artifact(msg_id, result);

                    // Trigger consensus updates for finality
                    if let Some(proposer) = self.consensus.deposits.get_proposer(&msg_id).await {
                        self.consensus.deposits.refund(&msg_id).await?;
                        // Calculate diversity from distinct balls
                        let diversity =
                            artifact.witness.distinct_balls.values().sum::<usize>() as f64;
                        let depth = artifact.witness.depth;

                        self.consensus
                            .reputation
                            .good_update(&proposer, diversity, depth)
                            .await;
                    } else {
                        tracing::warn!("Could not find proposer for finalized message {}", msg_id);
                    }

                    let mut finalized = self.finalized.write().await;
                    finalized.insert(msg_id, artifact);
                    newly_finalized.push(msg_id);

                    tracing::info!("Message {:?} finalized", msg_id);
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Error checking finality for {:?}: {}", msg_id, e);
                }
            }
        }

        if !newly_finalized.is_empty() {
            let mut pending = self.pending.write().await;
            pending.retain(|id| !newly_finalized.contains(id));
        }

        Ok(newly_finalized)
    }

    /// Create finality artifact from k-core result
    fn create_artifact(&self, msg_id: MessageId, result: KCoreResult) -> FinalityArtifact {
        let params = FinalityParams {
            k: self.config.k,
            q: self.config.min_diversity,
            depth_star: self.config.min_depth,
            r_sum_min: self.config.min_reputation,
        };

        let witness = FinalityWitness {
            kcore_root: result.kcore_root,
            depth: result.depth,
            diversity_ok: result
                .distinct_balls
                .values()
                .all(|&c| c >= self.config.min_diversity),
            reputation_sum: result.total_reputation,
            distinct_balls: result.distinct_balls,
            core_size: result.core_size,
        };

        FinalityArtifact::new(msg_id, FinalityGate::F1KCore, params, witness)
    }

    /// Start background finality checker
    pub async fn start_checker(&self) {
        let engine = self.clone();
        let interval = self.config.check_interval_ms;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(interval));

            loop {
                interval.tick().await;

                if let Err(e) = engine.check_finality().await {
                    tracing::error!("Finality check error: {}", e);
                }
            }
        });
    }

    /// Check F2 homology finality status
    pub async fn check_homology_finality(&self, messages: &[MessageId]) -> Result<HomologyResult> {
        self.homology.check_finality(messages).await
    }

    /// Check if F2 homology finality is enabled
    pub fn is_homology_enabled(&self) -> bool {
        self.homology.is_enabled()
    }

    /// Get statistics about finality
    pub async fn get_stats(&self) -> FinalityStats {
        let finalized_count = self.finalized.read().await.len();
        let pending_count = self.pending.read().await.len();

        FinalityStats {
            finalized_count,
            pending_count,
            total_messages: finalized_count + pending_count,
        }
    }

    /// Check k-core finality for a specific message
    pub async fn check_kcore_finality(
        &self,
        msg_id: &MessageId,
    ) -> Result<Option<KCoreFinalityResult>> {
        // Analyze k-core starting from this message
        let result = self
            .analyzer
            .analyze(*msg_id, &self.storage, &self.consensus.reputation)
            .await?;

        // Calculate diversity score as the minimum diversity across all axes
        let diversity_score = result.distinct_balls.values().min().copied().unwrap_or(0);

        Ok(Some(KCoreFinalityResult {
            k_value: self.config.k,
            depth: result.depth,
            diversity_score,
            is_final: result.is_final,
        }))
    }
}

impl Clone for FinalityEngine {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            analyzer: self.analyzer.clone(),
            homology: HomologyAnalyzer::new(self.consensus.params().clone()), // Create new instance with same params
            storage: Arc::clone(&self.storage),
            finalized: Arc::clone(&self.finalized),
            pending: Arc::clone(&self.pending),
            consensus: Arc::clone(&self.consensus),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FinalityStats {
    pub finalized_count: usize,
    pub pending_count: usize,
    pub total_messages: usize,
}

/// K-core finality result for API responses
#[derive(Debug, Clone)]
pub struct KCoreFinalityResult {
    pub k_value: usize,
    pub depth: u32,
    pub diversity_score: usize,
    pub is_final: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_storage::{store::BackendType, StorageConfig};
    use adic_types::{
        AdicFeatures, AdicMessage, AdicMeta, AdicParams, AxisPhi, PublicKey, QpDigits, Signature,
    };
    use tempfile::tempdir;

    fn make_ball_ids(axis_vals: &[(u32, &[u8])]) -> HashMap<u32, Vec<u8>> {
        axis_vals.iter().map(|(a, v)| (*a, v.to_vec())).collect()
    }

    fn create_test_message(
        id: MessageId,
        parents: Vec<MessageId>,
        ball_vals: &[(u32, u64)],
    ) -> AdicMessage {
        let features = AdicFeatures::new(
            ball_vals
                .iter()
                .map(|(axis, val)| AxisPhi::new(*axis, QpDigits::from_u64(*val, 3, 10)))
                .collect(),
        );

        let mut msg = AdicMessage::new(
            parents,
            features,
            AdicMeta::new(chrono::Utc::now()),
            PublicKey::from_bytes([1; 32]),
            vec![],
        );
        // Override the computed ID with our test ID
        msg.id = id;
        msg.signature = Signature::empty();
        msg
    }

    fn create_test_storage() -> Arc<StorageEngine> {
        let temp_dir = tempdir().unwrap();
        let storage_config = StorageConfig {
            backend_type: BackendType::RocksDB {
                path: temp_dir.path().to_str().unwrap().to_string(),
            },
            cache_size: 10,
            flush_interval_ms: 1000,
            max_batch_size: 100,
        };
        Arc::new(StorageEngine::new(storage_config).unwrap())
    }

    #[tokio::test]
    async fn test_finality_engine_basic_flow() {
        // Configure very permissive thresholds so simple chains can finalize
        let params = AdicParams {
            k: 1,           // minimal core size
            depth_star: 0,  // allow depth 0
            q: 1,           // diversity of 1 per axis
            r_sum_min: 0.0, // no reputation threshold
            ..Default::default()
        };

        let storage = create_test_storage();
        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(
            params.clone(),
            storage.clone(),
        ));
        let finality_cfg = FinalityConfig::from(&params);
        let engine = FinalityEngine::new(finality_cfg, consensus.clone(), storage.clone());

        // Create a tiny chain: root -> child
        let root_id = MessageId::new(b"root");
        let child_id = MessageId::new(b"child");

        // Create and store root message
        let root_msg = create_test_message(root_id, vec![], &[(0, 0), (1, 10)]);
        storage.store_message(&root_msg).await.unwrap();

        // Track reputation for the proposer
        consensus
            .reputation
            .set_reputation(&root_msg.proposer_pk, 1.0)
            .await;

        // Add root to finality engine (no parents)
        engine
            .add_message(
                root_id,
                vec![],
                1.0,
                make_ball_ids(&[(0, &[0, 0]), (1, &[1, 0])]),
            )
            .await
            .unwrap();

        // Create and store child message
        let child_msg = create_test_message(child_id, vec![root_id], &[(0, 1), (1, 11)]);
        storage.store_message(&child_msg).await.unwrap();

        // Track reputation for the child proposer
        consensus
            .reputation
            .set_reputation(&child_msg.proposer_pk, 1.0)
            .await;

        // Add child to finality engine
        engine
            .add_message(
                child_id,
                vec![root_id],
                1.0,
                make_ball_ids(&[(0, &[0, 1]), (1, &[1, 0])]),
            )
            .await
            .unwrap();

        // Run finality check; with permissive thresholds root should finalize
        let finalized = engine.check_finality().await.unwrap();
        assert!(!finalized.is_empty());
        assert!(engine.is_finalized(&root_id).await);
        assert!(engine.get_artifact(&root_id).await.is_some());
    }

    #[tokio::test]
    async fn test_finality_config_from_params() {
        let params = AdicParams {
            k: 7,
            depth_star: 10,
            q: 3,
            r_sum_min: 5.0,
            ..Default::default()
        };

        let config = FinalityConfig::from(&params);

        assert_eq!(config.k, 7);
        assert_eq!(config.min_depth, 10);
        assert_eq!(config.min_diversity, 3);
        assert_eq!(config.min_reputation, 5.0);
        assert_eq!(config.check_interval_ms, 1000);
        assert_eq!(config.window_size, 1000);
    }

    #[tokio::test]
    async fn test_finality_engine_window_management() {
        let params = AdicParams::default();
        let storage = create_test_storage();
        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(
            params.clone(),
            storage.clone(),
        ));
        let mut finality_cfg = FinalityConfig::from(&params);
        finality_cfg.window_size = 5; // Small window for testing

        let engine = FinalityEngine::new(finality_cfg, consensus, storage.clone());

        // Add more messages than window size
        for i in 0..10 {
            let msg_id = MessageId::new(&[i; 32]);

            // Create and store the message
            let msg = create_test_message(msg_id, vec![], &[(0, i as u64)]);
            storage.store_message(&msg).await.unwrap();

            engine
                .add_message(msg_id, vec![], 1.0, HashMap::new())
                .await
                .unwrap();
        }

        // Check that pending window is limited
        let pending = engine.pending.read().await;
        assert!(pending.len() <= 5);
    }

    #[tokio::test]
    async fn test_finality_engine_stats() {
        let params = AdicParams::default();
        let storage = create_test_storage();
        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(
            params.clone(),
            storage.clone(),
        ));
        let finality_cfg = FinalityConfig::from(&params);
        let engine = FinalityEngine::new(finality_cfg, consensus, storage.clone());

        // Add some messages
        for i in 0..3 {
            let msg_id = MessageId::new(&[i; 32]);

            // Create and store the message
            let msg = create_test_message(msg_id, vec![], &[(0, i as u64)]);
            storage.store_message(&msg).await.unwrap();

            engine
                .add_message(msg_id, vec![], 1.0, HashMap::new())
                .await
                .unwrap();
        }

        let stats = engine.get_stats().await;
        assert_eq!(stats.pending_count, 3);
        assert_eq!(stats.finalized_count, 0);
        assert_eq!(stats.total_messages, 3);
    }

    #[tokio::test]
    async fn test_finality_engine_with_deposits() {
        let params = AdicParams {
            k: 1,
            depth_star: 0,
            q: 1,
            r_sum_min: 0.0,
            ..Default::default()
        };

        let storage = create_test_storage();
        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(
            params.clone(),
            storage.clone(),
        ));
        let finality_cfg = FinalityConfig::from(&params);
        let engine = FinalityEngine::new(finality_cfg, consensus.clone(), storage.clone());

        let root_id = MessageId::new(b"root");
        let proposer = PublicKey::from_bytes([1; 32]);

        // Create and store root message
        let mut root_msg = create_test_message(root_id, vec![], &[(0, 0), (1, 10)]);
        root_msg.proposer_pk = proposer;
        storage.store_message(&root_msg).await.unwrap();

        // Initialize reputation
        consensus.reputation.set_reputation(&proposer, 1.0).await;

        // Escrow deposit for the message
        consensus.deposits.escrow(root_id, proposer).await.unwrap();

        // Add message to finality engine
        engine
            .add_message(
                root_id,
                vec![],
                1.0,
                make_ball_ids(&[(0, &[0, 0]), (1, &[1, 0])]),
            )
            .await
            .unwrap();

        // Create and store child to trigger finality
        let child_id = MessageId::new(b"child");
        let child_msg = create_test_message(child_id, vec![root_id], &[(0, 1), (1, 11)]);
        storage.store_message(&child_msg).await.unwrap();

        // Initialize reputation for child proposer
        consensus
            .reputation
            .set_reputation(&child_msg.proposer_pk, 1.0)
            .await;

        engine
            .add_message(
                child_id,
                vec![root_id],
                1.0,
                make_ball_ids(&[(0, &[0, 1]), (1, &[1, 0])]),
            )
            .await
            .unwrap();

        // Check finality
        let finalized = engine.check_finality().await.unwrap();
        assert!(!finalized.is_empty());

        // Deposit should be refunded
        let deposit_state = consensus.deposits.get_state(&root_id).await;
        assert_eq!(deposit_state, Some(adic_consensus::DepositState::Refunded));

        // Reputation should be updated
        let reputation = consensus.reputation.get_reputation(&proposer).await;
        assert!(reputation > 1.0);
    }

    #[tokio::test]
    async fn test_finality_artifact_creation() {
        let params = AdicParams::default();
        let storage = create_test_storage();
        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(
            params.clone(),
            storage.clone(),
        ));
        let finality_cfg = FinalityConfig::from(&params);
        let engine = FinalityEngine::new(finality_cfg.clone(), consensus, storage);

        let msg_id = MessageId::new(b"test");
        let kcore_result = KCoreResult {
            is_final: true,
            kcore_root: Some(MessageId::new(b"root")),
            depth: 5,
            total_reputation: 10.0,
            distinct_balls: vec![(0, 3), (1, 4)].into_iter().collect(),
            core_size: 7,
        };

        let artifact = engine.create_artifact(msg_id, kcore_result);

        assert_eq!(artifact.intent_id, msg_id);
        assert_eq!(artifact.gate, FinalityGate::F1KCore);
        assert_eq!(artifact.params.k, finality_cfg.k);
        assert_eq!(artifact.params.q, finality_cfg.min_diversity);
        assert_eq!(artifact.witness.depth, 5);
        assert_eq!(artifact.witness.reputation_sum, 10.0);
        assert_eq!(artifact.witness.core_size, 7);
    }

    #[tokio::test]
    async fn test_finality_engine_clone() {
        let params = AdicParams::default();
        let storage = create_test_storage();
        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(
            params.clone(),
            storage.clone(),
        ));
        let finality_cfg = FinalityConfig::from(&params);
        let engine = FinalityEngine::new(finality_cfg, consensus, storage.clone());

        let msg_id = MessageId::new(b"test");
        let msg = create_test_message(msg_id, vec![], &[(0, 1)]);
        storage.store_message(&msg).await.unwrap();

        engine
            .add_message(msg_id, vec![], 1.0, HashMap::new())
            .await
            .unwrap();

        let cloned = engine.clone();

        // Both should see the same pending messages
        let original_pending = engine.pending.read().await;
        let cloned_pending = cloned.pending.read().await;
        assert_eq!(original_pending.len(), cloned_pending.len());
    }

    #[tokio::test]
    async fn test_multiple_finality_checks() {
        let params = AdicParams {
            k: 1,
            depth_star: 0,
            q: 1,
            r_sum_min: 0.0,
            ..Default::default()
        };

        let storage = create_test_storage();
        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(
            params.clone(),
            storage.clone(),
        ));
        let finality_cfg = FinalityConfig::from(&params);
        let engine = FinalityEngine::new(finality_cfg, consensus.clone(), storage.clone());

        // Create a chain: root -> middle -> tip
        let root_id = MessageId::new(b"root");
        let middle_id = MessageId::new(b"middle");
        let tip_id = MessageId::new(b"tip");

        // Store root message
        let root_msg = create_test_message(root_id, vec![], &[(0, 0)]);
        storage.store_message(&root_msg).await.unwrap();
        consensus
            .reputation
            .set_reputation(&root_msg.proposer_pk, 1.0)
            .await;

        engine
            .add_message(root_id, vec![], 1.0, make_ball_ids(&[(0, &[0])]))
            .await
            .unwrap();

        // Store middle message
        let middle_msg = create_test_message(middle_id, vec![root_id], &[(0, 1)]);
        storage.store_message(&middle_msg).await.unwrap();
        consensus
            .reputation
            .set_reputation(&middle_msg.proposer_pk, 1.0)
            .await;

        engine
            .add_message(middle_id, vec![root_id], 1.0, make_ball_ids(&[(0, &[1])]))
            .await
            .unwrap();

        // First check might finalize root
        let finalized1 = engine.check_finality().await.unwrap();

        // Store tip message
        let tip_msg = create_test_message(tip_id, vec![middle_id], &[(0, 2)]);
        storage.store_message(&tip_msg).await.unwrap();
        consensus
            .reputation
            .set_reputation(&tip_msg.proposer_pk, 1.0)
            .await;

        engine
            .add_message(tip_id, vec![middle_id], 1.0, make_ball_ids(&[(0, &[2])]))
            .await
            .unwrap();

        // Second check might finalize middle
        let finalized2 = engine.check_finality().await.unwrap();

        // At least one should be finalized
        assert!(finalized1.len() + finalized2.len() > 0);
    }

    #[tokio::test]
    async fn test_finality_no_consensus_proposer() {
        let params = AdicParams {
            k: 1,
            depth_star: 0,
            q: 1,
            r_sum_min: 0.0,
            ..Default::default()
        };

        let storage = create_test_storage();
        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(
            params.clone(),
            storage.clone(),
        ));
        let finality_cfg = FinalityConfig::from(&params);
        let engine = FinalityEngine::new(finality_cfg, consensus.clone(), storage.clone());

        let root_id = MessageId::new(b"root");
        let child_id = MessageId::new(b"child");

        // Store root message
        let root_msg = create_test_message(root_id, vec![], &[(0, 0)]);
        storage.store_message(&root_msg).await.unwrap();
        consensus
            .reputation
            .set_reputation(&root_msg.proposer_pk, 1.0)
            .await;

        // Add messages without escrowing deposits
        engine
            .add_message(root_id, vec![], 1.0, make_ball_ids(&[(0, &[0])]))
            .await
            .unwrap();

        // Store child message
        let child_msg = create_test_message(child_id, vec![root_id], &[(0, 1)]);
        storage.store_message(&child_msg).await.unwrap();
        consensus
            .reputation
            .set_reputation(&child_msg.proposer_pk, 1.0)
            .await;

        engine
            .add_message(child_id, vec![root_id], 1.0, make_ball_ids(&[(0, &[1])]))
            .await
            .unwrap();

        // Should still finalize even without deposits
        let finalized = engine.check_finality().await.unwrap();
        assert!(!finalized.is_empty());
    }

    #[test]
    fn test_finality_stats_creation() {
        let stats = FinalityStats {
            finalized_count: 10,
            pending_count: 5,
            total_messages: 15,
        };

        assert_eq!(stats.finalized_count, 10);
        assert_eq!(stats.pending_count, 5);
        assert_eq!(stats.total_messages, 15);
    }

    #[test]
    fn test_finality_config_creation() {
        let config = FinalityConfig {
            k: 5,
            min_depth: 10,
            min_diversity: 3,
            min_reputation: 2.5,
            check_interval_ms: 500,
            window_size: 2000,
        };

        assert_eq!(config.k, 5);
        assert_eq!(config.min_depth, 10);
        assert_eq!(config.min_diversity, 3);
        assert_eq!(config.min_reputation, 2.5);
        assert_eq!(config.check_interval_ms, 500);
        assert_eq!(config.window_size, 2000);
    }
}
