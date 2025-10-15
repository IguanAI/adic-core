pub mod selector;
pub mod trace;
pub mod weights;

#[cfg(test)]
mod selector_tests;
#[cfg(test)]
mod weights_tests;

pub use selector::{MrwSelector, ParentCandidate, SelectionParams};
pub use trace::{MrwTrace, SelectionStep};
pub use weights::{MrwWeights, WeightCalculator};

use adic_consensus::ConsensusEngine;
use adic_storage::StorageEngine;
use adic_types::{AdicFeatures, AdicParams, MessageId, QpDigits};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct MrwEngine {
    _params: Arc<RwLock<AdicParams>>,
    selector: Arc<RwLock<MrwSelector>>,
    weight_calc: Arc<RwLock<WeightCalculator>>,
    traces: Arc<RwLock<HashMap<String, MrwTrace>>>,
    // Metrics counters - updated externally by incrementing directly
    pub mrw_attempts_total: Option<Arc<prometheus::IntCounter>>,
    pub mrw_widens_total: Option<Arc<prometheus::IntCounter>>,
    pub mrw_selection_duration: Option<Arc<prometheus::Histogram>>,
}

impl MrwEngine {
    pub fn new(params: AdicParams) -> Self {
        let selection_params = SelectionParams::from_adic_params(&params);
        Self {
            _params: Arc::new(RwLock::new(params.clone())),
            selector: Arc::new(RwLock::new(MrwSelector::new(selection_params))),
            weight_calc: Arc::new(RwLock::new(WeightCalculator::new_with_alpha(
                params.lambda,
                params.alpha,
                params.beta,
                params.mu,
            ))),
            traces: Arc::new(RwLock::new(HashMap::new())),
            mrw_attempts_total: None,
            mrw_widens_total: None,
            mrw_selection_duration: None,
        }
    }

    /// Set metrics for MRW tracking
    pub fn set_metrics(
        &mut self,
        mrw_attempts_total: Arc<prometheus::IntCounter>,
        mrw_widens_total: Arc<prometheus::IntCounter>,
        mrw_selection_duration: Arc<prometheus::Histogram>,
    ) {
        self.mrw_attempts_total = Some(mrw_attempts_total);
        self.mrw_widens_total = Some(mrw_widens_total);
        self.mrw_selection_duration = Some(mrw_selection_duration);
    }

    /// Update MRW parameters for hot-reload support
    pub async fn update_params(&self, new_params: &AdicParams) {
        *self._params.write().await = new_params.clone();
        *self.weight_calc.write().await = WeightCalculator::new_with_alpha(
            new_params.lambda,
            new_params.alpha,
            new_params.beta,
            new_params.mu,
        );
        let selection_params = SelectionParams::from_adic_params(new_params);
        *self.selector.write().await = MrwSelector::new(selection_params);
        tracing::info!(
            lambda = new_params.lambda,
            alpha = new_params.alpha,
            beta = new_params.beta,
            mu = new_params.mu,
            "✅ MRW parameters updated"
        );
    }

    /// Select parents from the current tips using MRW algorithm
    pub async fn select_parents(
        &self,
        features: &AdicFeatures,
        tips: &[MessageId],
        storage: &StorageEngine,
        consensus: &ConsensusEngine,
    ) -> Result<Vec<MessageId>> {
        let start = std::time::Instant::now();

        // Update metrics - MRW selection attempt
        if let Some(ref counter) = self.mrw_attempts_total {
            counter.inc();
        }

        // Build parent candidates from tips
        let mut candidates = Vec::new();

        tracing::debug!("Building parent candidates from {} tips", tips.len());

        for tip_id in tips {
            // Get tip message from storage
            match storage.get_message(tip_id).await {
                Ok(Some(tip_msg)) => {
                    // Get LIVE reputation for the tip proposer
                    let reputation = consensus.reputation
                        .get_reputation(&tip_msg.proposer_pk)
                        .await;
                    tracing::debug!(
                        "Tip {} proposer reputation: {:.2}",
                        hex::encode(&tip_id.as_bytes()[..8]),
                        reputation
                    );

                    // Calculate conflict penalty using paper-accurate energy descent
                    let conflict_penalty = if !tip_msg.meta.conflict.is_none() {
                        consensus
                            .get_conflict_penalty(tip_id, &tip_msg.meta.conflict)
                            .await
                    } else {
                        0.0
                    };

                    // Extract features as Vec<QpDigits>
                    let features: Vec<QpDigits> = tip_msg
                        .features
                        .phi
                        .iter()
                        .map(|axis_phi| axis_phi.qp_digits.clone())
                        .collect();

                    // Calculate message age in seconds for trust decay
                    let now = chrono::Utc::now();
                    let age_duration = now.signed_duration_since(tip_msg.meta.timestamp);
                    let age = age_duration.num_seconds() as f64;

                    candidates.push(ParentCandidate {
                        message_id: *tip_id,
                        features,
                        reputation,
                        conflict_penalty,
                        weight: 0.0,                  // Will be calculated later
                        axis_weights: HashMap::new(), // Will be filled by MRW
                        age,                          // Age in seconds for trust decay
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

        tracing::debug!(
            "Built {} parent candidates from {} tips",
            candidates.len(),
            tips.len()
        );

        // If no candidates were built, check if this is a genesis scenario
        if candidates.is_empty() {
            // Allow genesis message (no parents) only when there are no tips
            if tips.is_empty() {
                tracing::info!("No tips available - allowing genesis message with no parents");
                return Ok(vec![]); // Return empty parent list for genesis
            }
            return Err(anyhow::anyhow!(
                "No valid parent candidates found from {} tips",
                tips.len()
            ));
        }

        // Select parents using MRW
        let selector = self.selector.read().await;
        let weight_calc = self.weight_calc.read().await;
        let (selected, trace) = selector
            .select_parents(features, candidates, &*weight_calc)
            .await?;

        // Store the trace for debugging
        let trace_id = format!("mrw_{}", chrono::Utc::now().timestamp_millis());

        // Track widens from trace
        let widen_count = trace.widen_count;

        {
            let mut traces = self.traces.write().await;
            traces.insert(trace_id.clone(), trace);

            // Limit trace storage to 100 most recent traces
            if traces.len() > 100 {
                let oldest_key = traces.keys().min().unwrap().clone();
                traces.remove(&oldest_key);
            }
        }

        // Update metrics - widens and duration
        if widen_count > 0 {
            if let Some(ref counter) = self.mrw_widens_total {
                counter.inc_by(widen_count as u64);
            }
        }
        if let Some(ref histogram) = self.mrw_selection_duration {
            histogram.observe(start.elapsed().as_secs_f64());
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
        entries
            .into_iter()
            .take(limit)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_storage::{StorageConfig, StorageEngine};
    use adic_types::features::{AxisPhi, QpDigits};
    use chrono::Utc;

    #[tokio::test]
    async fn test_mrw_engine_select_parents() {
        let params = AdicParams::default();
        let engine = MrwEngine::new(params.clone());

        // Prepare in-memory storage with candidate tip messages
        let storage = std::sync::Arc::new(StorageEngine::new(StorageConfig::default()).unwrap());

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

        let consensus = adic_consensus::ConsensusEngine::new(params.clone(), storage.clone());
        let selected = engine
            .select_parents(&features, &tips, &storage, &consensus)
            .await
            .expect("selection should succeed");

        assert_eq!(selected.len(), (params.d + 1) as usize);
    }
}
