use crate::{
    artifact::{FinalityArtifact, FinalityGate, FinalityParams, FinalityWitness},
    homology::{HomologyAnalyzer, HomologyResult},
    kcore::{KCoreAnalyzer, KCoreResult, MessageGraph},
};
use adic_consensus::ConsensusEngine;
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
    graph: Arc<RwLock<MessageGraph>>,
    finalized: Arc<RwLock<HashMap<MessageId, FinalityArtifact>>>,
    pending: Arc<RwLock<VecDeque<MessageId>>>,
    consensus: Arc<ConsensusEngine>,
}

impl FinalityEngine {
    pub fn new(config: FinalityConfig, consensus: Arc<ConsensusEngine>) -> Self {
        let analyzer = KCoreAnalyzer::new(
            config.k,
            config.min_depth,
            config.min_diversity,
            config.min_reputation,
        );
        let homology = HomologyAnalyzer::new(consensus.params().clone());

        Self {
            config,
            analyzer,
            homology,
            graph: Arc::new(RwLock::new(MessageGraph::new())),
            finalized: Arc::new(RwLock::new(HashMap::new())),
            pending: Arc::new(RwLock::new(VecDeque::new())),
            consensus,
        }
    }

    /// Add a message to the graph for finality tracking
    pub async fn add_message(
        &self,
        id: MessageId,
        parents: Vec<MessageId>,
        reputation: f64,
        ball_ids: HashMap<u32, Vec<u8>>,
    ) -> Result<()> {
        let mut graph = self.graph.write().await;
        graph.add_message(
            id,
            parents.clone(),
            crate::kcore::MessageInfo {
                reputation,
                ball_ids,
            },
        );

        // Add to homology complex for F2 analysis
        self.homology
            .add_simplex(
                {
                    let mut vertices = vec![id];
                    vertices.extend(parents);
                    vertices
                },
                reputation,
            )
            .await?;

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
        let graph = self.graph.read().await;

        for msg_id in pending_list {
            if self.is_finalized(&msg_id).await {
                continue;
            }

            match self.analyzer.analyze(msg_id, &graph) {
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
}

impl Clone for FinalityEngine {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            analyzer: self.analyzer.clone(),
            homology: HomologyAnalyzer::new(self.consensus.params().clone()), // Create new instance with same params
            graph: Arc::clone(&self.graph),
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

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::{AdicParams, PublicKey};

    fn make_ball_ids(axis_vals: &[(u32, &[u8])]) -> HashMap<u32, Vec<u8>> {
        axis_vals.iter().map(|(a, v)| (*a, v.to_vec())).collect()
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

        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(params.clone()));
        let finality_cfg = FinalityConfig::from(&params);
        let engine = FinalityEngine::new(finality_cfg, consensus.clone());

        // Create a tiny chain: root -> child
        let root = MessageId::new(b"root");
        let child = MessageId::new(b"child");

        // Add root (no parents)
        engine
            .add_message(
                root,
                vec![],
                1.0,
                make_ball_ids(&[(0, &[0, 0]), (1, &[1, 0])]),
            )
            .await
            .unwrap();

        // Add child referencing root
        engine
            .add_message(
                child,
                vec![root],
                1.0,
                make_ball_ids(&[(0, &[0, 1]), (1, &[1, 0])]),
            )
            .await
            .unwrap();

        // Run finality check; with permissive thresholds root should finalize
        let finalized = engine.check_finality().await.unwrap();
        assert!(!finalized.is_empty());
        assert!(engine.is_finalized(&root).await);
        assert!(engine.get_artifact(&root).await.is_some());
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
        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(params.clone()));
        let mut finality_cfg = FinalityConfig::from(&params);
        finality_cfg.window_size = 5; // Small window for testing

        let engine = FinalityEngine::new(finality_cfg, consensus);

        // Add more messages than window size
        for i in 0..10 {
            let msg_id = MessageId::new(&[i; 32]);
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
        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(params.clone()));
        let finality_cfg = FinalityConfig::from(&params);
        let engine = FinalityEngine::new(finality_cfg, consensus);

        // Add some messages
        for i in 0..3 {
            let msg_id = MessageId::new(&[i; 32]);
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

        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(params.clone()));
        let finality_cfg = FinalityConfig::from(&params);
        let engine = FinalityEngine::new(finality_cfg, consensus.clone());

        let root = MessageId::new(b"root");
        let proposer = PublicKey::from_bytes([1; 32]);

        // Escrow deposit for the message
        consensus.deposits.escrow(root, proposer).await.unwrap();

        // Add message to finality engine
        engine
            .add_message(
                root,
                vec![],
                1.0,
                make_ball_ids(&[(0, &[0, 0]), (1, &[1, 0])]),
            )
            .await
            .unwrap();

        // Add child to trigger finality
        let child = MessageId::new(b"child");
        engine
            .add_message(
                child,
                vec![root],
                1.0,
                make_ball_ids(&[(0, &[0, 1]), (1, &[1, 0])]),
            )
            .await
            .unwrap();

        // Check finality
        let finalized = engine.check_finality().await.unwrap();
        assert!(!finalized.is_empty());

        // Deposit should be refunded
        let deposit_state = consensus.deposits.get_state(&root).await;
        assert_eq!(deposit_state, Some(adic_consensus::DepositState::Refunded));

        // Reputation should be updated
        let reputation = consensus.reputation.get_reputation(&proposer).await;
        assert!(reputation > 1.0);
    }

    #[tokio::test]
    async fn test_finality_artifact_creation() {
        let params = AdicParams::default();
        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(params.clone()));
        let finality_cfg = FinalityConfig::from(&params);
        let engine = FinalityEngine::new(finality_cfg.clone(), consensus);

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
        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(params.clone()));
        let finality_cfg = FinalityConfig::from(&params);
        let engine = FinalityEngine::new(finality_cfg, consensus);

        let msg_id = MessageId::new(b"test");
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

        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(params.clone()));
        let finality_cfg = FinalityConfig::from(&params);
        let engine = FinalityEngine::new(finality_cfg, consensus);

        // Create a chain: root -> middle -> tip
        let root = MessageId::new(b"root");
        let middle = MessageId::new(b"middle");
        let tip = MessageId::new(b"tip");

        engine
            .add_message(root, vec![], 1.0, make_ball_ids(&[(0, &[0])]))
            .await
            .unwrap();

        engine
            .add_message(middle, vec![root], 1.0, make_ball_ids(&[(0, &[1])]))
            .await
            .unwrap();

        // First check might finalize root
        let finalized1 = engine.check_finality().await.unwrap();

        engine
            .add_message(tip, vec![middle], 1.0, make_ball_ids(&[(0, &[2])]))
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

        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(params.clone()));
        let finality_cfg = FinalityConfig::from(&params);
        let engine = FinalityEngine::new(finality_cfg, consensus.clone());

        let root = MessageId::new(b"root");
        let child = MessageId::new(b"child");

        // Add messages without escrowing deposits
        engine
            .add_message(root, vec![], 1.0, make_ball_ids(&[(0, &[0])]))
            .await
            .unwrap();

        engine
            .add_message(child, vec![root], 1.0, make_ball_ids(&[(0, &[1])]))
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
