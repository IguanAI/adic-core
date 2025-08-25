pub mod selector;
pub mod trace;
pub mod weights;

#[cfg(test)]
mod selector_tests;
#[cfg(test)]
mod weights_tests;

pub use selector::{MrwSelector, ParentCandidate, SelectionParams};
pub use trace::{MrwTrace, SelectionStep};
pub use weights::{WeightCalculator, MrwWeights};

use adic_types::{AdicParams, AdicFeatures, MessageId, QpDigits};
use adic_storage::StorageEngine;
use adic_consensus::ConflictResolver;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct MrwEngine {
    _params: AdicParams,
    selector: MrwSelector,
    weight_calc: WeightCalculator,
    traces: Arc<RwLock<HashMap<String, MrwTrace>>>,
}

impl MrwEngine {
    pub fn new(params: AdicParams) -> Self {
        let selection_params = SelectionParams::from_adic_params(&params);
        Self {
            _params: params.clone(),
            selector: MrwSelector::new(selection_params),
            weight_calc: WeightCalculator::new(params.lambda, params.beta, params.mu),
            traces: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn selector(&self) -> &MrwSelector {
        &self.selector
    }

    pub fn weight_calculator(&self) -> &WeightCalculator {
        &self.weight_calc
    }

    /// Select parents from the current tips using MRW algorithm
    pub async fn select_parents(
        &self,
        features: &AdicFeatures,
        tips: &[MessageId],
        storage: &StorageEngine,
        conflict_resolver: &ConflictResolver,
    ) -> Result<Vec<MessageId>> {
        // Build parent candidates from tips
        let mut candidates = Vec::new();
        
        tracing::debug!("Building parent candidates from {} tips", tips.len());
        
        for tip_id in tips {
            // Get tip message from storage
            match storage.get_message(tip_id).await {
                Ok(Some(tip_msg)) => {
                    // Get reputation for the tip proposer
                    // For now, use a default reputation of 1.0
                    let reputation = 1.0;
                    
                    // Calculate conflict penalty
                    let conflict_penalty = if !tip_msg.meta.conflict.is_none() {
                        conflict_resolver.get_penalty(tip_id, &tip_msg.meta.conflict).await
                    } else {
                        0.0
                    };
                    
                    // Extract features as Vec<QpDigits>
                    let features: Vec<QpDigits> = tip_msg.features.phi.iter()
                        .map(|axis_phi| axis_phi.qp_digits.clone())
                        .collect();
                        
                    candidates.push(ParentCandidate {
                        message_id: *tip_id,
                        features,
                        reputation,
                        conflict_penalty,
                        weight: 0.0, // Will be calculated later
                        axis_weights: HashMap::new(), // Will be filled by MRW
                    });
                }
                Ok(None) => {
                    tracing::warn!("Tip {:?} not found in storage", tip_id);
                }
                Err(e) => {
                    tracing::warn!("Error fetching tip {:?}: {}", tip_id, e);
                }
            }
        }
        
        tracing::debug!("Built {} parent candidates from {} tips", candidates.len(), tips.len());
        
        // If no candidates were built, return an error
        if candidates.is_empty() {
            return Err(anyhow::anyhow!(
                "No valid parent candidates found from {} tips", 
                tips.len()
            ));
        }
        
        // Select parents using MRW
        let (selected, trace) = self.selector.select_parents(
            features,
            candidates,
            &self.weight_calc,
        ).await?;
        
        // Store the trace for debugging
        let trace_id = format!("mrw_{}", chrono::Utc::now().timestamp_millis());
        {
            let mut traces = self.traces.write().await;
            traces.insert(trace_id.clone(), trace);
            
            // Limit trace storage to 100 most recent traces
            if traces.len() > 100 {
                let oldest_key = traces.keys().min().unwrap().clone();
                traces.remove(&oldest_key);
            }
        }
        
        tracing::debug!("MRW selection completed, trace stored: {}", trace_id);
        
        Ok(selected)
    }
    
    /// Get a specific MRW trace by ID
    pub async fn get_trace(&self, trace_id: &str) -> Option<MrwTrace> {
        let traces = self.traces.read().await;
        traces.get(trace_id).cloned()
    }
    
    /// Get all stored trace IDs
    pub async fn get_trace_ids(&self) -> Vec<String> {
        let traces = self.traces.read().await;
        let mut ids: Vec<String> = traces.keys().cloned().collect();
        ids.sort();
        ids.reverse(); // Most recent first
        ids
    }
    
    /// Get recent traces (up to specified limit)
    pub async fn get_recent_traces(&self, limit: usize) -> Vec<(String, MrwTrace)> {
        let traces = self.traces.read().await;
        let mut entries: Vec<_> = traces.iter().collect();
        entries.sort_by_key(|(k, _)| *k);
        entries.reverse(); // Most recent first
        entries.into_iter()
            .take(limit)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_storage::{StorageConfig, StorageEngine};
    use chrono::Utc;
    use adic_types::features::{AxisPhi, QpDigits};

    #[tokio::test]
    async fn test_mrw_engine_select_parents() {
        let params = AdicParams::default();
        let engine = MrwEngine::new(params.clone());

        // Prepare in-memory storage with candidate tip messages
        let storage = StorageEngine::new(StorageConfig::default()).unwrap();

        let mut tips: Vec<MessageId> = Vec::new();
        for i in 0..10u64 {
            let msg = adic_types::AdicMessage::new(
                vec![],
                adic_types::AdicFeatures::new(vec![
                    AxisPhi::new(0, QpDigits::from_u64(i, 3, 10)),
                    AxisPhi::new(1, QpDigits::from_u64(i * 2, 3, 10)),
                    AxisPhi::new(2, QpDigits::from_u64(i * 3, 3, 10)),
                ]),
                adic_types::AdicMeta::new(Utc::now()),
                adic_types::PublicKey::from_bytes([0; 32]),
                vec![],
            );
            tips.push(msg.id);
            storage.store_message(&msg).await.unwrap();
        }

        // Message features to select parents for
        let features = adic_types::AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(5, 3, 10)),
            AxisPhi::new(1, QpDigits::from_u64(10, 3, 10)),
            AxisPhi::new(2, QpDigits::from_u64(15, 3, 10)),
        ]);

        let resolver = adic_consensus::ConflictResolver::new();
        let selected = engine
            .select_parents(&features, &tips, &storage, &resolver)
            .await
            .expect("selection should succeed");

        assert_eq!(selected.len(), (params.d + 1) as usize);
    }
}
