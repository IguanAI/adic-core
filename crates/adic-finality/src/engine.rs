use crate::{
    artifact::{FinalityArtifact, FinalityGate, FinalityParams, FinalityWitness},
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

        Self {
            config,
            analyzer,
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
            parents,
            crate::kcore::MessageInfo {
                reputation,
                ball_ids,
            },
        );

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
                        let diversity = artifact.witness.distinct_balls.values().sum::<usize>() as f64;
                        let depth = artifact.witness.depth;
                        
                        self.consensus.reputation.good_update(&proposer, diversity, depth).await;
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
            diversity_ok: result.distinct_balls.values().all(|&c| c >= self.config.min_diversity),
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
