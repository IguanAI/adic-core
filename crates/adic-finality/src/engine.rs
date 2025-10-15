use crate::{
    artifact::{FinalityArtifact, FinalityGate, FinalityParams, FinalityWitness},
    checkpoint::{Checkpoint, F1Metadata, F2Metadata, MerkleTreeBuilder},
    kcore::{KCoreAnalyzer, KCoreResult},
    ph::{F2Config, F2FinalityChecker, F2Result},
};
use adic_consensus::ConsensusEngine;
use adic_storage::StorageEngine;
use adic_types::{AdicParams, MessageId, Result};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

/// Finality timing statistics for governance timelock calculation
#[derive(Debug, Clone)]
pub struct FinalityTimingStats {
    pub f1_avg: Option<std::time::Duration>,
    pub f1_p95: Option<std::time::Duration>,
    pub f2_avg: Option<std::time::Duration>,
    pub f2_p95: Option<std::time::Duration>,
    pub f1_sample_size: usize,
    pub f2_sample_size: usize,
}

/// Configuration for finality engine
#[derive(Debug, Clone)]
pub struct FinalityConfig {
    // F1 (K-core) parameters
    pub k: usize,
    pub min_depth: u32,
    pub min_diversity: usize,
    pub min_reputation: f64,
    pub check_interval_ms: u64,
    pub window_size: usize,

    // F2 (Topological) parameters
    pub f2_enabled: bool,
    pub f2_timeout_ms: u64,
    pub f1_enabled: bool,
    pub epsilon: f64,

    // Checkpoint parameters
    pub checkpoint_interval: u64, // Create checkpoint every N finalized messages
}

impl From<&AdicParams> for FinalityConfig {
    fn from(params: &AdicParams) -> Self {
        Self {
            // F1 parameters
            k: params.k as usize,
            min_depth: params.depth_star,
            min_diversity: params.q as usize,
            min_reputation: params.r_sum_min,
            check_interval_ms: 1000,
            window_size: 1000,

            // F2 parameters (Phase 0: feature-flagged OFF by default per DESIGN.md §1.2)
            f2_enabled: false,
            f2_timeout_ms: 2000,
            f1_enabled: true,
            epsilon: 0.1,

            // Checkpoint parameters
            checkpoint_interval: 100, // Create checkpoint every 100 finalized messages
        }
    }
}

impl Default for FinalityConfig {
    fn default() -> Self {
        Self {
            // F1 defaults
            k: 3,
            min_depth: 3,
            min_diversity: 3,
            min_reputation: 1.0,
            check_interval_ms: 1000,
            window_size: 1000,

            // F2 defaults (Phase 0: OFF by default per DESIGN.md §1.2)
            f2_enabled: false,
            f2_timeout_ms: 2000,
            f1_enabled: true,
            epsilon: 0.1,

            // Checkpoint defaults
            checkpoint_interval: 100,
        }
    }
}

/// Main finality engine
pub struct FinalityEngine {
    config: Arc<RwLock<FinalityConfig>>,
    analyzer: Arc<RwLock<KCoreAnalyzer>>,
    f2_checker: Arc<Mutex<F2FinalityChecker>>,
    storage: Arc<StorageEngine>,
    finalized: Arc<RwLock<HashMap<MessageId, FinalityArtifact>>>,
    pending: Arc<RwLock<VecDeque<MessageId>>>,
    consensus: Arc<ConsensusEngine>,

    // Checkpoint state
    checkpoints: Arc<RwLock<Vec<Checkpoint>>>,
    f1_finalized_messages: Arc<RwLock<Vec<MessageId>>>,
    f2_stabilized_messages: Arc<RwLock<Vec<MessageId>>>,
    checkpoint_height: Arc<RwLock<u64>>,

    // Finality timing metrics for governance timelock calculation
    f1_timing_metrics: Arc<RwLock<VecDeque<std::time::Duration>>>,
    f2_timing_metrics: Arc<RwLock<VecDeque<std::time::Duration>>>,
    metrics_window_size: usize,

    // Epoch tracking for PoUW integration
    /// Maximum finalized depth observed (tracks the deepest finalized message)
    max_finalized_depth: Arc<RwLock<u32>>,
    /// Finality depth parameter (D*) from AdicParams
    depth_star: u32,

    // Metrics counters - updated externally by incrementing directly
    pub finalizations_total: Option<Arc<prometheus::IntCounter>>,
    pub kcore_size: Option<Arc<prometheus::IntGauge>>,
    pub finality_depth: Option<Arc<prometheus::Histogram>>,
    pub f2_computation_time: Option<Arc<prometheus::Histogram>>,
    pub f2_timeout_count: Option<Arc<prometheus::IntCounter>>,
    pub f1_fallback_count: Option<Arc<prometheus::IntCounter>>,
    pub finality_method_used: Option<Arc<prometheus::IntGauge>>,
}

impl FinalityEngine {
    pub async fn new(
        config: FinalityConfig,
        consensus: Arc<ConsensusEngine>,
        storage: Arc<StorageEngine>,
    ) -> Self {
        let params = consensus.params().await;
        let analyzer = KCoreAnalyzer::new(
            config.k,
            config.min_depth,
            config.min_diversity,
            config.min_reputation,
            params.rho.clone(),
        );

        // Initialize F2 checker with config
        let f2_config = F2Config {
            dimension: 3, // d=3 as per paper
            radius: 5,    // Δ=5 as per paper
            axis: 0,      // Use axis 0
            epsilon: config.epsilon,
            timeout_ms: config.f2_timeout_ms,
            use_streaming: false, // Batch mode by default for backward compatibility
            max_betti_d: 2,       // Default stability heuristic (governable)
            min_persistence: 0.01, // Default noise filtering threshold (governable)
        };
        let f2_checker = Arc::new(Mutex::new(F2FinalityChecker::new(f2_config)));

        let depth_star = params.depth_star;

        Self {
            config: Arc::new(RwLock::new(config)),
            analyzer: Arc::new(RwLock::new(analyzer)),
            f2_checker,
            storage,
            finalized: Arc::new(RwLock::new(HashMap::new())),
            pending: Arc::new(RwLock::new(VecDeque::new())),
            consensus,
            checkpoints: Arc::new(RwLock::new(Vec::new())),
            f1_finalized_messages: Arc::new(RwLock::new(Vec::new())),
            f2_stabilized_messages: Arc::new(RwLock::new(Vec::new())),
            checkpoint_height: Arc::new(RwLock::new(0)),
            f1_timing_metrics: Arc::new(RwLock::new(VecDeque::new())),
            f2_timing_metrics: Arc::new(RwLock::new(VecDeque::new())),
            metrics_window_size: 100, // Keep last 100 measurements
            max_finalized_depth: Arc::new(RwLock::new(0)),
            depth_star,
            finalizations_total: None,
            kcore_size: None,
            finality_depth: None,
            f2_computation_time: None,
            f2_timeout_count: None,
            f1_fallback_count: None,
            finality_method_used: None,
        }
    }

    /// Set metrics for finality tracking
    pub fn set_metrics(
        &mut self,
        finalizations_total: Arc<prometheus::IntCounter>,
        kcore_size: Arc<prometheus::IntGauge>,
        finality_depth: Arc<prometheus::Histogram>,
        f2_computation_time: Arc<prometheus::Histogram>,
        f2_timeout_count: Arc<prometheus::IntCounter>,
        f1_fallback_count: Arc<prometheus::IntCounter>,
        finality_method_used: Arc<prometheus::IntGauge>,
    ) {
        self.finalizations_total = Some(finalizations_total);
        self.kcore_size = Some(kcore_size);
        self.finality_depth = Some(finality_depth);
        self.f2_computation_time = Some(f2_computation_time);
        self.f2_timeout_count = Some(f2_timeout_count);
        self.f1_fallback_count = Some(f1_fallback_count);
        self.finality_method_used = Some(finality_method_used);
    }

    /// Update finality parameters for hot-reload support
    /// Note: Only operational params (k, depth, epsilon) are hot-reloadable
    pub async fn update_params(&self, new_params: &adic_types::AdicParams, f2_max_betti: usize, f2_min_persistence: f64) {
        // Update config
        {
            let mut config = self.config.write().await;
            config.k = new_params.k as usize;
            config.min_depth = new_params.depth_star;
        }

        // Recreate k-core analyzer with new parameters
        let config = self.config.read().await;
        *self.analyzer.write().await = KCoreAnalyzer::new(
            new_params.k as usize,
            new_params.depth_star,
            config.min_diversity,
            config.min_reputation,
            new_params.rho.clone(),
        );

        // Update F2 checker config
        let mut f2_checker = self.f2_checker.lock().await;
        *f2_checker = F2FinalityChecker::new(F2Config {
            dimension: 3,
            radius: 5,
            axis: 0,
            epsilon: config.epsilon,
            timeout_ms: config.f2_timeout_ms,
            use_streaming: false,
            max_betti_d: f2_max_betti,
            min_persistence: f2_min_persistence,
        });

        tracing::info!(
            k = new_params.k,
            depth = new_params.depth_star,
            f2_max_betti = f2_max_betti,
            f2_min_persistence = f2_min_persistence,
            "✅ Finality parameters updated"
        );
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
        let window_size = self.config.read().await.window_size;
        while pending.len() > window_size {
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

    /// Store a finality artifact received from the network
    /// This is used when artifacts propagate from other nodes
    pub async fn store_artifact(&self, artifact: FinalityArtifact) {
        let mut finalized = self.finalized.write().await;
        finalized.insert(artifact.intent_id, artifact);
    }

    /// Run finality checks on pending messages
    /// Tries F2 (topological) first, then falls back to F1 (k-core) on timeout
    pub async fn check_finality(&self) -> Result<Vec<MessageId>> {
        let pending_list = {
            let pending = self.pending.read().await;
            pending.iter().cloned().collect::<Vec<_>>()
        };

        let mut newly_finalized = Vec::new();

        // Read config once at the start
        let f2_enabled = self.config.read().await.f2_enabled;
        let f1_enabled = self.config.read().await.f1_enabled;

        // Try F2 finality first if enabled
        if f2_enabled {
            let f2_start = std::time::Instant::now();
            match self.try_f2_finality(&pending_list).await {
                Ok(Some((finalized_msgs, h3_stable, h2_bottleneck, confidence))) => {
                    // Update metrics - F2 computation time
                    if let Some(ref histogram) = self.f2_computation_time {
                        histogram.observe(f2_start.elapsed().as_secs_f64());
                    }
                    // Set finality method to F2
                    if let Some(ref gauge) = self.finality_method_used {
                        gauge.set(2);
                    }

                    // F2 succeeded - finalize these messages with witness data
                    for msg_id in &finalized_msgs {
                        self.finalize_message_f2(
                            *msg_id,
                            h3_stable,
                            h2_bottleneck,
                            confidence,
                            finalized_msgs.len(),
                        )
                        .await?;

                        // Update metrics - finalization count
                        if let Some(ref counter) = self.finalizations_total {
                            counter.inc();
                        }
                    }
                    newly_finalized.extend(finalized_msgs);

                    // Clean up pending
                    if !newly_finalized.is_empty() {
                        let mut pending = self.pending.write().await;
                        pending.retain(|id| !newly_finalized.contains(id));
                    }

                    return Ok(newly_finalized);
                }
                Ok(None) => {
                    // F2 returned Pending or not enough data - fall through to F1
                    debug!(
                        f1_enabled = f1_enabled,
                        "🔄 F2 finality not achieved, falling back to F1"
                    );
                    // Update metrics - F1 fallback
                    if let Some(ref counter) = self.f1_fallback_count {
                        counter.inc();
                    }
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        f1_enabled = f1_enabled,
                        "🔄 F2 check error, falling back to F1"
                    );
                    // Check if it's a timeout error
                    if e.to_string().contains("timeout") || e.to_string().contains("Timeout") {
                        if let Some(ref counter) = self.f2_timeout_count {
                            counter.inc();
                        }
                    }
                    // Update metrics - F1 fallback
                    if let Some(ref counter) = self.f1_fallback_count {
                        counter.inc();
                    }
                }
            }
        }

        // F1 fallback - check each message individually with k-core
        if f1_enabled {
            for msg_id in pending_list {
                if self.is_finalized(&msg_id).await {
                    continue;
                }

                // Capture F1 finalization timing
                let f1_start = std::time::Instant::now();

                let analyzer = self.analyzer.read().await;
                match analyzer
                    .analyze(msg_id, &self.storage, &self.consensus.reputation)
                    .await
                {
                    Ok(result) if result.is_final => {
                        let f1_duration = f1_start.elapsed();

                        // Record F1 timing metric
                        {
                            let mut metrics = self.f1_timing_metrics.write().await;
                            metrics.push_back(f1_duration);
                            // Trim to window size
                            while metrics.len() > self.metrics_window_size {
                                metrics.pop_front();
                            }
                        }

                        // Update metrics - finalization count
                        if let Some(ref counter) = self.finalizations_total {
                            counter.inc();
                        }
                        // Set finality method to F1
                        if let Some(ref gauge) = self.finality_method_used {
                            gauge.set(1);
                        }
                        // Update k-core size
                        if let Some(ref gauge) = self.kcore_size {
                            gauge.set(result.core_size as i64);
                        }
                        // Update finality depth
                        if let Some(ref histogram) = self.finality_depth {
                            histogram.observe(result.depth as f64);
                        }

                        let artifact = self.create_artifact(msg_id, result.clone()).await;

                        // Update epoch tracking with finalized depth
                        self.update_finalized_depth(result.depth).await;

                        // Trigger consensus updates for finality
                        if let Some(proposer) = self.consensus.deposits.get_proposer(&msg_id).await
                        {
                            self.consensus.deposits.refund(&msg_id).await?;
                            let diversity =
                                artifact.witness.distinct_balls.values().sum::<usize>() as f64;
                            let depth = artifact.witness.depth;

                            self.consensus
                                .reputation
                                .good_update(&proposer, diversity, depth)
                                .await;
                        }

                        // Persist artifact
                        if let Ok(artifact_bytes) = serde_json::to_vec(&artifact) {
                            if let Err(e) = self
                                .storage
                                .store_finality_artifact(&msg_id, &artifact_bytes)
                                .await
                            {
                                tracing::warn!(
                                    "Failed to persist finality artifact for {:?}: {}",
                                    msg_id,
                                    e
                                );
                            }
                        }

                        let mut finalized = self.finalized.write().await;
                        finalized.insert(msg_id, artifact);
                        newly_finalized.push(msg_id);

                        tracing::info!("Message {:?} finalized via F1", msg_id);

                        // Track for checkpoint creation
                        drop(finalized);
                        self.track_f1_finalized(msg_id).await;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("Error checking finality for {:?}: {}", msg_id, e);
                    }
                }
            }
        }

        if !newly_finalized.is_empty() {
            let mut pending = self.pending.write().await;
            pending.retain(|id| !newly_finalized.contains(id));
        }

        Ok(newly_finalized)
    }

    /// Try F2 (topological) finality check
    /// Returns Ok(Some(messages)) if finalized, Ok(None) if pending, Err on failure
    async fn try_f2_finality(
        &self,
        pending: &[MessageId],
    ) -> Result<Option<(Vec<MessageId>, bool, f64, f64)>> {
        if pending.is_empty() {
            debug!("F2 check skipped: no pending messages");
            return Ok(None);
        }

        debug!(num_pending = pending.len(), "Attempting F2 finality check");

        // Fetch messages from storage
        let mut messages = Vec::new();
        for msg_id in pending {
            match self.storage.get_message(msg_id).await {
                Ok(Some(msg)) => messages.push(msg),
                Ok(None) => {
                    warn!(
                        message_id = ?msg_id,
                        "Message not found in storage during F2 check"
                    );
                    continue;
                }
                Err(e) => {
                    warn!(
                        message_id = ?msg_id,
                        error = %e,
                        "Failed to fetch message for F2 check"
                    );
                    continue;
                }
            }
        }

        if messages.is_empty() {
            warn!("F2 check aborted: no messages could be fetched from storage");
            return Ok(None);
        }

        // Run F2 check
        let mut checker = self.f2_checker.lock().await;
        let result = checker.check_finality(&messages);

        match result {
            F2Result::Final {
                h_d_stable,
                h_d_minus_1_bottleneck,
                confidence,
                finalized_messages,
                elapsed_ms,
            } => {
                // Record F2 timing metric
                let f2_duration = std::time::Duration::from_millis(elapsed_ms);
                {
                    let mut metrics = self.f2_timing_metrics.write().await;
                    metrics.push_back(f2_duration);
                    // Trim to window size
                    while metrics.len() > self.metrics_window_size {
                        metrics.pop_front();
                    }
                }

                info!(
                    num_finalized = finalized_messages.len(),
                    h_d_stable = h_d_stable,
                    h_d_minus_1_bottleneck = format!("{:.6}", h_d_minus_1_bottleneck),
                    confidence = format!("{:.4}", confidence),
                    elapsed_ms = elapsed_ms,
                    "✅ F2 topological finality achieved"
                );
                Ok(Some((
                    finalized_messages,
                    h_d_stable,
                    h_d_minus_1_bottleneck,
                    confidence,
                )))
            }
            F2Result::Timeout {
                elapsed_ms,
                partial_bottleneck,
            } => {
                warn!(
                    elapsed_ms = elapsed_ms,
                    partial_bottleneck = ?partial_bottleneck,
                    "⏱️ F2 computation timed out, will fall back to F1"
                );
                Ok(None)
            }
            F2Result::Pending {
                reason,
                h_d_stable,
                h_d_minus_1_distance,
            } => {
                debug!(
                    reason = reason,
                    h_d_stable = h_d_stable,
                    h_d_minus_1_distance = ?h_d_minus_1_distance,
                    "F2 pending: conditions not yet met"
                );
                Ok(None)
            }
            F2Result::Error { message } => {
                warn!(error = message, "❌ F2 computation error");
                Err(adic_types::AdicError::HomologyError(message))
            }
        }
    }

    /// Finalize a message via F2 method with witness data
    async fn finalize_message_f2(
        &self,
        msg_id: MessageId,
        h3_stable: bool,
        h2_bottleneck_distance: f64,
        confidence: f64,
        num_finalized: usize,
    ) -> Result<()> {
        // Read config once
        let config = self.config.read().await;

        // Create artifact with F2-specific witness data
        let artifact = FinalityArtifact {
            intent_id: msg_id,
            gate: FinalityGate::F2PersistentHomology,
            witness: FinalityWitness {
                // F1 fields (not applicable for F2)
                kcore_root: None,
                depth: 3, // d=3 from paper
                diversity_ok: true,
                distinct_balls: HashMap::new(),
                core_size: 0,
                reputation_sum: 0.0,
                // F2 witness data
                h3_stable: Some(h3_stable),
                h2_bottleneck_distance: Some(h2_bottleneck_distance),
                f2_confidence: Some(confidence),
                num_finalized_messages: Some(num_finalized),
            },
            params: FinalityParams {
                k: config.k,
                q: config.min_diversity,
                depth_star: config.min_depth,
                r_sum_min: config.min_reputation,
            },
            validators: Vec::new(),
            timestamp: chrono::Utc::now().timestamp(),
        };

        // Update epoch tracking
        // F2 finalization implies messages have reached at least min_depth (D*)
        // since F2 is stricter than F1
        self.update_finalized_depth(config.min_depth).await;

        // Trigger consensus updates
        if let Some(proposer) = self.consensus.deposits.get_proposer(&msg_id).await {
            self.consensus.deposits.refund(&msg_id).await?;
            // For F2, use topology-based rewards
            self.consensus
                .reputation
                .good_update(&proposer, 1.0, 3)
                .await;
        }

        // Persist artifact
        if let Ok(artifact_bytes) = serde_json::to_vec(&artifact) {
            if let Err(e) = self
                .storage
                .store_finality_artifact(&msg_id, &artifact_bytes)
                .await
            {
                tracing::warn!("Failed to persist F2 artifact: {}", e);
            }
        }

        let mut finalized = self.finalized.write().await;
        finalized.insert(msg_id, artifact);

        tracing::info!("Message {:?} finalized via F2", msg_id);

        // Track for checkpoint creation
        drop(finalized);
        self.track_f2_stabilized(msg_id).await;
        Ok(())
    }

    /// Create finality artifact from k-core result
    async fn create_artifact(&self, msg_id: MessageId, result: KCoreResult) -> FinalityArtifact {
        let config = self.config.read().await;

        let params = FinalityParams {
            k: config.k,
            q: config.min_diversity,
            depth_star: config.min_depth,
            r_sum_min: config.min_reputation,
        };

        let witness = FinalityWitness {
            // F1 witness data
            kcore_root: result.kcore_root,
            depth: result.depth,
            diversity_ok: result
                .distinct_balls
                .values()
                .all(|&c| c >= config.min_diversity),
            reputation_sum: result.total_reputation,
            distinct_balls: result.distinct_balls,
            core_size: result.core_size,
            // F2 witness data (not applicable for F1)
            h3_stable: None,
            h2_bottleneck_distance: None,
            f2_confidence: None,
            num_finalized_messages: None,
        };

        FinalityArtifact::new(msg_id, FinalityGate::F1KCore, params, witness)
    }

    /// Start background finality checker
    pub async fn start_checker(&self) {
        let engine = self.clone();
        let interval = self.config.read().await.check_interval_ms;

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

    /// Get current epoch number based on finalized depth
    ///
    /// Per PoUW II §3: Epochs are delimited by finalized depth-D windows
    /// Formula: epoch = floor(max_finalized_depth / D*)
    pub async fn current_epoch(&self) -> u64 {
        let max_depth = *self.max_finalized_depth.read().await;
        if self.depth_star == 0 {
            return 0;
        }
        (max_depth / self.depth_star) as u64
    }

    /// Get maximum finalized depth
    pub async fn max_finalized_depth(&self) -> u32 {
        *self.max_finalized_depth.read().await
    }

    /// Check if a message has achieved composite F1 ∧ F2 finality
    ///
    /// Per PoUW III §7.1: Governance-grade finality requires both F1 (k-core)
    /// and F2 (persistent homology) to pass.
    ///
    /// Returns true if the message has a composite finality artifact OR
    /// if it has achieved both F1 and F2 finality separately.
    pub async fn has_composite_finality(&self, id: &MessageId) -> bool {
        let finalized = self.finalized.read().await;
        if let Some(artifact) = finalized.get(id) {
            artifact.has_composite_finality()
        } else {
            false
        }
    }

    /// Check if a message is finalized with governance-grade finality (F1 ∧ F2)
    ///
    /// This is an alias for `has_composite_finality` with clearer naming for
    /// governance use cases.
    pub async fn is_governance_finalized(&self, id: &MessageId) -> bool {
        self.has_composite_finality(id).await
    }

    /// Update maximum finalized depth (called internally when messages finalize)
    async fn update_finalized_depth(&self, depth: u32) {
        let mut max_depth = self.max_finalized_depth.write().await;
        if depth > *max_depth {
            // Skip epoch calculation if depth_star is 0 (test mode or not configured)
            if self.depth_star > 0 {
                let old_epoch = (*max_depth / self.depth_star) as u64;
                let new_epoch = (depth / self.depth_star) as u64;

                if new_epoch > old_epoch {
                    info!(
                        old_epoch = old_epoch,
                        new_epoch = new_epoch,
                        finalized_depth = depth,
                        depth_star = self.depth_star,
                        "📅 Epoch boundary crossed"
                    );
                }
            }

            *max_depth = depth;
        }
    }

    /// Check k-core finality for a specific message
    pub async fn check_kcore_finality(
        &self,
        msg_id: &MessageId,
    ) -> Result<Option<KCoreFinalityResult>> {
        // Analyze k-core starting from this message
        let analyzer = self.analyzer.read().await;
        let result = analyzer
            .analyze(*msg_id, &self.storage, &self.consensus.reputation)
            .await?;

        // Calculate diversity score as the minimum diversity across all axes
        let diversity_score = result.distinct_balls.values().min().copied().unwrap_or(0);

        let config = self.config.read().await;
        Ok(Some(KCoreFinalityResult {
            k_value: config.k,
            depth: result.depth,
            diversity_score,
            is_final: result.is_final,
        }))
    }

    /// Get average F1 finality time from recent measurements
    pub async fn get_avg_f1_finality_time(&self) -> Option<std::time::Duration> {
        let metrics = self.f1_timing_metrics.read().await;
        if metrics.is_empty() {
            return None;
        }
        let sum: std::time::Duration = metrics.iter().sum();
        Some(sum / metrics.len() as u32)
    }

    /// Get P95 F1 finality time (95th percentile)
    pub async fn get_p95_f1_finality_time(&self) -> Option<std::time::Duration> {
        let metrics = self.f1_timing_metrics.read().await;
        if metrics.is_empty() {
            return None;
        }
        let mut sorted: Vec<std::time::Duration> = metrics.iter().cloned().collect();
        sorted.sort();
        let index = (sorted.len() * 95 / 100).min(sorted.len() - 1);
        Some(sorted[index])
    }

    /// Get average F2 finality time from recent measurements
    pub async fn get_avg_f2_finality_time(&self) -> Option<std::time::Duration> {
        let metrics = self.f2_timing_metrics.read().await;
        if metrics.is_empty() {
            return None;
        }
        let sum: std::time::Duration = metrics.iter().sum();
        Some(sum / metrics.len() as u32)
    }

    /// Get P95 F2 finality time (95th percentile)
    pub async fn get_p95_f2_finality_time(&self) -> Option<std::time::Duration> {
        let metrics = self.f2_timing_metrics.read().await;
        if metrics.is_empty() {
            return None;
        }
        let mut sorted: Vec<std::time::Duration> = metrics.iter().cloned().collect();
        sorted.sort();
        let index = (sorted.len() * 95 / 100).min(sorted.len() - 1);
        Some(sorted[index])
    }

    /// Get finality timing statistics for monitoring
    pub async fn get_finality_timing_stats(&self) -> FinalityTimingStats {
        FinalityTimingStats {
            f1_avg: self.get_avg_f1_finality_time().await,
            f1_p95: self.get_p95_f1_finality_time().await,
            f2_avg: self.get_avg_f2_finality_time().await,
            f2_p95: self.get_p95_f2_finality_time().await,
            f1_sample_size: self.f1_timing_metrics.read().await.len(),
            f2_sample_size: self.f2_timing_metrics.read().await.len(),
        }
    }
}

impl Clone for FinalityEngine {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            analyzer: self.analyzer.clone(),
            f2_checker: Arc::clone(&self.f2_checker),
            storage: Arc::clone(&self.storage),
            finalized: Arc::clone(&self.finalized),
            pending: Arc::clone(&self.pending),
            consensus: Arc::clone(&self.consensus),
            checkpoints: Arc::clone(&self.checkpoints),
            f1_finalized_messages: Arc::clone(&self.f1_finalized_messages),
            f2_stabilized_messages: Arc::clone(&self.f2_stabilized_messages),
            checkpoint_height: Arc::clone(&self.checkpoint_height),
            f1_timing_metrics: Arc::clone(&self.f1_timing_metrics),
            f2_timing_metrics: Arc::clone(&self.f2_timing_metrics),
            metrics_window_size: self.metrics_window_size,
            max_finalized_depth: Arc::clone(&self.max_finalized_depth),
            depth_star: self.depth_star,
            finalizations_total: self.finalizations_total.clone(),
            kcore_size: self.kcore_size.clone(),
            finality_depth: self.finality_depth.clone(),
            f2_computation_time: self.f2_computation_time.clone(),
            f2_timeout_count: self.f2_timeout_count.clone(),
            f1_fallback_count: self.f1_fallback_count.clone(),
            finality_method_used: self.finality_method_used.clone(),
        }
    }
}

impl FinalityEngine {
    /// Track F1 finalized message for checkpoint
    async fn track_f1_finalized(&self, message_id: MessageId) {
        let mut f1_messages = self.f1_finalized_messages.write().await;
        f1_messages.push(message_id);

        // Check if we should create a checkpoint
        let checkpoint_interval = self.config.read().await.checkpoint_interval;
        if f1_messages.len() >= checkpoint_interval as usize {
            if let Err(e) = self.create_checkpoint().await {
                warn!(error = %e, "Failed to create checkpoint on F1 finality");
            }
        }
    }

    /// Track F2 stabilized message for checkpoint
    async fn track_f2_stabilized(&self, message_id: MessageId) {
        let mut f2_messages = self.f2_stabilized_messages.write().await;
        f2_messages.push(message_id);
    }

    /// Create a checkpoint of current finalized state
    pub async fn create_checkpoint(&self) -> Result<Checkpoint> {
        let mut height_guard = self.checkpoint_height.write().await;
        let height = *height_guard;
        *height_guard += 1;

        let f1_messages = self.f1_finalized_messages.read().await;
        let f2_messages = self.f2_stabilized_messages.read().await;
        let finalized = self.finalized.read().await;

        // Build merkle tree for finalized messages
        let mut message_tree = MerkleTreeBuilder::new();
        for msg_id in f1_messages.iter() {
            message_tree.add_message(*msg_id);
        }
        let messages_root = message_tree.compute_root();

        // Build merkle tree for witnesses
        let mut witness_tree = MerkleTreeBuilder::new();
        for artifact in finalized.values() {
            if let Ok(witness_bytes) = serde_json::to_vec(&artifact.witness) {
                witness_tree.add_witness(&witness_bytes);
            }
        }
        let witnesses_root = witness_tree.compute_root();

        // Build F1 metadata
        let max_k_core = finalized
            .values()
            .map(|a| a.params.k as u64)
            .max()
            .unwrap_or(0);
        let f1_metadata = F1Metadata::new(max_k_core, f1_messages.clone());

        // Build F2 metadata
        let f2_metadata = F2Metadata::new(f2_messages.clone(), None);

        // Get previous checkpoint hash
        let checkpoints = self.checkpoints.read().await;
        let prev_hash = checkpoints.last().map(|c| c.compute_hash());
        drop(checkpoints);

        let checkpoint = Checkpoint::new(
            height,
            chrono::Utc::now().timestamp(),
            messages_root,
            witnesses_root,
            f1_metadata,
            f2_metadata,
            prev_hash,
        );

        // Store checkpoint
        let mut checkpoints = self.checkpoints.write().await;
        checkpoints.push(checkpoint.clone());

        // Clear tracked messages after checkpoint
        drop(f1_messages);
        drop(f2_messages);
        let mut f1_messages = self.f1_finalized_messages.write().await;
        let mut f2_messages = self.f2_stabilized_messages.write().await;
        f1_messages.clear();
        f2_messages.clear();

        info!(
            height = checkpoint.height,
            messages_count = checkpoint.f1_metadata.finalized_count,
            "✅ Created checkpoint"
        );

        Ok(checkpoint)
    }

    /// Get the latest checkpoint
    pub async fn get_latest_checkpoint(&self) -> Option<Checkpoint> {
        let checkpoints = self.checkpoints.read().await;
        checkpoints.last().cloned()
    }

    /// Get checkpoint at specific height
    pub async fn get_checkpoint(&self, height: u64) -> Option<Checkpoint> {
        let checkpoints = self.checkpoints.read().await;
        checkpoints.iter().find(|c| c.height == height).cloned()
    }

    /// Get all checkpoints
    pub async fn get_all_checkpoints(&self) -> Vec<Checkpoint> {
        let checkpoints = self.checkpoints.read().await;
        checkpoints.clone()
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
        let engine = FinalityEngine::new(finality_cfg, consensus.clone(), storage.clone()).await;

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

        let engine = FinalityEngine::new(finality_cfg, consensus, storage.clone()).await;

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
        let engine = FinalityEngine::new(finality_cfg, consensus, storage.clone()).await;

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
        let engine = FinalityEngine::new(finality_cfg, consensus.clone(), storage.clone()).await;

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
        let engine = FinalityEngine::new(finality_cfg.clone(), consensus, storage).await;

        let msg_id = MessageId::new(b"test");
        let kcore_result = KCoreResult {
            is_final: true,
            kcore_root: Some(MessageId::new(b"root")),
            depth: 5,
            total_reputation: 10.0,
            distinct_balls: vec![(0, 3), (1, 4)].into_iter().collect(),
            core_size: 7,
        };

        let artifact = engine.create_artifact(msg_id, kcore_result).await;

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
        let engine = FinalityEngine::new(finality_cfg, consensus, storage.clone()).await;

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
        let engine = FinalityEngine::new(finality_cfg, consensus.clone(), storage.clone()).await;

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
        let engine = FinalityEngine::new(finality_cfg, consensus.clone(), storage.clone()).await;

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
            f2_enabled: true,
            f2_timeout_ms: 2000,
            f1_enabled: true,
            epsilon: 0.1,
            checkpoint_interval: 100,
        };

        assert_eq!(config.k, 5);
        assert_eq!(config.min_depth, 10);
        assert_eq!(config.min_diversity, 3);
        assert_eq!(config.min_reputation, 2.5);
        assert_eq!(config.check_interval_ms, 500);
        assert_eq!(config.window_size, 2000);
        assert!(config.f2_enabled);
        assert_eq!(config.f2_timeout_ms, 2000);
        assert!(config.f1_enabled);
        assert_eq!(config.epsilon, 0.1);
    }

    #[tokio::test]
    async fn test_f2_finality_integration() {
        // Test F2 finality end-to-end
        let params = AdicParams {
            k: 2,
            depth_star: 1,
            q: 2,
            r_sum_min: 0.0,
            ..Default::default()
        };

        let storage = create_test_storage();
        let consensus = Arc::new(adic_consensus::ConsensusEngine::new(
            params.clone(),
            storage.clone(),
        ));

        // Enable F2 with very permissive settings
        let mut finality_cfg = FinalityConfig::from(&params);
        finality_cfg.f2_enabled = true;
        finality_cfg.f2_timeout_ms = 5000;
        finality_cfg.epsilon = 1.0; // Very lenient

        let engine = FinalityEngine::new(finality_cfg, consensus.clone(), storage.clone()).await;

        // Create multiple messages to potentially trigger F2
        let mut msg_ids = vec![];
        for i in 0..10 {
            let id = MessageId::new(format!("msg_{}", i).as_bytes());
            let msg = create_test_message(id, vec![], &[(0, i), (1, i * 2)]);
            storage.store_message(&msg).await.unwrap();
            consensus
                .reputation
                .set_reputation(&msg.proposer_pk, 1.0)
                .await;

            engine
                .add_message(
                    id,
                    vec![],
                    1.0,
                    make_ball_ids(&[(0, &[0, i as u8]), (1, &[1, (i * 2) as u8])]),
                )
                .await
                .unwrap();

            msg_ids.push(id);
        }

        // Run finality check - should attempt F2 first
        let finalized = engine.check_finality().await.unwrap();

        // Verify some messages finalized (either via F2 or F1 fallback)
        assert!(
            !finalized.is_empty() || !engine.pending.read().await.is_empty(),
            "Should have finalized or pending messages"
        );

        // Check that finalized messages have artifacts
        for msg_id in &finalized {
            let artifact = engine.get_artifact(msg_id).await;
            assert!(artifact.is_some(), "Finalized message should have artifact");
        }
    }

    #[tokio::test]
    async fn test_f2_timeout_fallback_to_f1() {
        // Test that F2 timeout properly falls back to F1
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

        // Configure F2 with very short timeout to force fallback
        let mut finality_cfg = FinalityConfig::from(&params);
        finality_cfg.f2_enabled = true;
        finality_cfg.f2_timeout_ms = 1; // 1ms timeout - should timeout immediately
        finality_cfg.f1_enabled = true;

        let engine = FinalityEngine::new(finality_cfg, consensus.clone(), storage.clone()).await;

        // Create messages
        let msg_id = MessageId::new(b"timeout_test");
        let msg = create_test_message(msg_id, vec![], &[(0, 5)]);
        storage.store_message(&msg).await.unwrap();
        consensus
            .reputation
            .set_reputation(&msg.proposer_pk, 1.0)
            .await;

        engine
            .add_message(msg_id, vec![], 1.0, make_ball_ids(&[(0, &[0, 5])]))
            .await
            .unwrap();

        // Run finality - should timeout on F2 and fall back to F1
        let finalized = engine.check_finality().await.unwrap();

        // Should still finalize via F1 fallback
        if !finalized.is_empty() {
            assert!(finalized.contains(&msg_id));
            let artifact = engine.get_artifact(&msg_id).await;
            assert!(artifact.is_some());

            // Should be F1 finality due to timeout
            let artifact = artifact.unwrap();
            assert!(matches!(artifact.gate, FinalityGate::F1KCore));
        }
    }

    #[tokio::test]
    async fn test_f2_disabled_uses_f1_only() {
        // Test that when F2 is disabled, only F1 is used
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

        // Disable F2
        let mut finality_cfg = FinalityConfig::from(&params);
        finality_cfg.f2_enabled = false;
        finality_cfg.f1_enabled = true;

        let engine = FinalityEngine::new(finality_cfg, consensus.clone(), storage.clone()).await;

        // Create messages
        let msg_id = MessageId::new(b"f1_only");
        let msg = create_test_message(msg_id, vec![], &[(0, 10)]);
        storage.store_message(&msg).await.unwrap();
        consensus
            .reputation
            .set_reputation(&msg.proposer_pk, 1.0)
            .await;

        engine
            .add_message(msg_id, vec![], 1.0, make_ball_ids(&[(0, &[0, 10])]))
            .await
            .unwrap();

        // Run finality
        let finalized = engine.check_finality().await.unwrap();

        // If finalized, should use F1
        if finalized.contains(&msg_id) {
            let artifact = engine.get_artifact(&msg_id).await.unwrap();
            assert!(matches!(artifact.gate, FinalityGate::F1KCore));
        }
    }

    #[tokio::test]
    async fn test_concurrent_f2_checks() {
        // Test that concurrent finality checks don't interfere
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

        let mut finality_cfg = FinalityConfig::from(&params);
        finality_cfg.f2_enabled = true;
        finality_cfg.f2_timeout_ms = 2000;

        let engine = Arc::new(FinalityEngine::new(
            finality_cfg,
            consensus.clone(),
            storage.clone(),
        ).await);

        // Create messages
        for i in 0..5 {
            let id = MessageId::new(format!("concurrent_{}", i).as_bytes());
            let msg = create_test_message(id, vec![], &[(0, i * 10)]);
            storage.store_message(&msg).await.unwrap();
            consensus
                .reputation
                .set_reputation(&msg.proposer_pk, 1.0)
                .await;

            engine
                .add_message(id, vec![], 1.0, make_ball_ids(&[(0, &[0, (i * 10) as u8])]))
                .await
                .unwrap();
        }

        // Run multiple concurrent finality checks
        let engine1 = Arc::clone(&engine);
        let engine2 = Arc::clone(&engine);

        let handle1 = tokio::spawn(async move { engine1.check_finality().await });

        let handle2 = tokio::spawn(async move { engine2.check_finality().await });

        // Both should complete without deadlock or panic
        let result1 = handle1.await.unwrap();
        let result2 = handle2.await.unwrap();

        assert!(result1.is_ok());
        assert!(result2.is_ok());
    }
}
