pub mod selector;
pub mod trace;
pub mod weights;

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
        
        for tip_id in tips {
            // Get tip message from storage
            if let Ok(Some(tip_msg)) = storage.get_message(tip_id).await {
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