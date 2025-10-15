use prometheus::{Histogram, HistogramOpts, IntCounter, IntGauge, Registry, TextEncoder};
use std::sync::Arc;

#[derive(Clone)]
pub struct Metrics {
    registry: Arc<Registry>,
    pub messages_submitted: IntCounter,
    pub messages_processed: IntCounter,
    pub messages_failed: IntCounter,

    // MRW Metrics
    pub mrw_attempts_total: IntCounter,
    pub mrw_widens_total: IntCounter,
    pub mrw_selection_duration: Histogram,

    // Admissibility Metrics
    pub admissibility_checks_total: IntCounter,
    pub admissibility_s_failures: IntCounter,
    pub admissibility_c2_failures: IntCounter,
    pub admissibility_c3_failures: IntCounter,

    // Finality Metrics
    pub finalizations_total: IntCounter,
    pub kcore_size: IntGauge,
    pub finality_depth: Histogram,

    // F2 (Persistent Homology) Finality Metrics
    pub f2_computation_time: Histogram,
    pub f2_timeout_count: IntCounter,
    pub f1_fallback_count: IntCounter,
    pub finality_method_used: IntGauge, // 0=pending, 1=F1, 2=F2

    // Validation Metrics
    pub signature_verifications: IntCounter,
    pub signature_failures: IntCounter,

    // DAG State Metrics
    pub current_tips: IntGauge,
    pub dag_messages: IntGauge,

    // Event Streaming Metrics
    pub events_emitted_total: IntCounter,
    pub websocket_connections: IntGauge,
    pub sse_connections: IntGauge,
    pub websocket_messages_sent: IntCounter,
    pub sse_messages_sent: IntCounter,

    // Foundation Layer Metrics - VRF
    pub vrf_commits_total: IntCounter,
    pub vrf_reveals_total: IntCounter,
    pub vrf_finalizations_total: IntCounter,
    pub vrf_commit_duration: Histogram,
    pub vrf_reveal_duration: Histogram,
    pub vrf_finalize_duration: Histogram,

    // Foundation Layer Metrics - Quorum Selection
    pub quorum_selections_total: IntCounter,
    pub quorum_selection_duration: Histogram,
    pub quorum_committee_size: IntGauge,
    // Note: quorum_votes_* metrics are passed via Arc to DisputeAdjudicator (node.rs:1144-1147)
    // Rust's dead code analysis doesn't track usage through .clone() + Arc wrapper
    #[allow(dead_code)]
    pub quorum_votes_total: IntCounter,
    #[allow(dead_code)]
    pub quorum_votes_passed: IntCounter,
    #[allow(dead_code)]
    pub quorum_votes_failed: IntCounter,
    #[allow(dead_code)]
    pub quorum_vote_duration: Histogram,

    // Foundation Layer Metrics - Challenges
    // Note: These metrics are passed via Arc to sub-components:
    // - challenge_windows_* → ChallengeWindowManager (node.rs:1122-1126)
    // - fraud_proofs_* → DisputeAdjudicator (node.rs:1138-1147)
    // - arbitrations_* → DisputeAdjudicator (node.rs:1138-1147)
    // Rust's dead code analysis doesn't track usage through .clone() + Arc wrapper
    #[allow(dead_code)]
    pub challenge_windows_opened: IntCounter,
    #[allow(dead_code)]
    pub challenges_submitted: IntCounter,
    #[allow(dead_code)]
    pub fraud_proofs_submitted: IntCounter,
    #[allow(dead_code)]
    pub fraud_proofs_verified: IntCounter,
    #[allow(dead_code)]
    pub fraud_proofs_rejected: IntCounter,
    #[allow(dead_code)]
    pub arbitrations_started: IntCounter,
    #[allow(dead_code)]
    pub arbitrations_completed: IntCounter,
    #[allow(dead_code)]
    pub challenge_windows_active: IntGauge,

    // Foundation Layer Metrics - Escrow
    pub escrow_locks_total: IntCounter,
    pub escrow_releases_total: IntCounter,
    pub escrow_slashes_total: IntCounter,
    pub escrow_refunds_total: IntCounter,
    pub escrow_lock_duration: Histogram,
    pub escrow_locked_amount: IntGauge,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Arc::new(Registry::new());

        let messages_submitted =
            IntCounter::new("adic_messages_submitted_total", "Total messages submitted").unwrap();
        let messages_processed =
            IntCounter::new("adic_messages_processed_total", "Total messages processed").unwrap();
        let messages_failed =
            IntCounter::new("adic_messages_failed_total", "Total messages failed").unwrap();

        let mrw_attempts_total =
            IntCounter::new("adic_mrw_attempts_total", "Total MRW attempts").unwrap();
        let mrw_widens_total =
            IntCounter::new("adic_mrw_widens_total", "Total MRW widens").unwrap();
        let mrw_selection_duration = Histogram::with_opts(HistogramOpts::new(
            "adic_mrw_selection_duration_seconds",
            "MRW selection duration",
        ))
        .unwrap();

        let admissibility_checks_total = IntCounter::new(
            "adic_admissibility_checks_total",
            "Total admissibility checks",
        )
        .unwrap();
        let admissibility_s_failures = IntCounter::new(
            "adic_admissibility_s_failures_total",
            "Admissibility S failures",
        )
        .unwrap();
        let admissibility_c2_failures = IntCounter::new(
            "adic_admissibility_c2_failures_total",
            "Admissibility C2 failures",
        )
        .unwrap();
        let admissibility_c3_failures = IntCounter::new(
            "adic_admissibility_c3_failures_total",
            "Admissibility C3 failures",
        )
        .unwrap();

        let finalizations_total =
            IntCounter::new("adic_finalizations_total", "Total finalizations").unwrap();
        let kcore_size = IntGauge::new("adic_kcore_size", "K-core size").unwrap();
        let finality_depth = Histogram::with_opts(HistogramOpts::new(
            "adic_finality_depth_seconds",
            "Finality depth",
        ))
        .unwrap();

        // F2 finality metrics
        let f2_computation_time = Histogram::with_opts(HistogramOpts::new(
            "adic_f2_computation_time_seconds",
            "F2 persistent homology computation time",
        ))
        .unwrap();
        let f2_timeout_count =
            IntCounter::new("adic_f2_timeout_total", "Total F2 computation timeouts").unwrap();
        let f1_fallback_count = IntCounter::new(
            "adic_f1_fallback_total",
            "Total F1 fallbacks after F2 timeout",
        )
        .unwrap();
        let finality_method_used = IntGauge::new(
            "adic_finality_method_used",
            "Current finality method (0=pending, 1=F1, 2=F2)",
        )
        .unwrap();

        let signature_verifications = IntCounter::new(
            "adic_signature_verifications_total",
            "Total signature verifications",
        )
        .unwrap();
        let signature_failures =
            IntCounter::new("adic_signature_failures_total", "Total signature failures").unwrap();

        let current_tips = IntGauge::new("adic_current_tips", "Current tips").unwrap();
        let dag_messages = IntGauge::new("adic_dag_messages", "Total DAG messages").unwrap();

        let events_emitted_total = IntCounter::new(
            "adic_events_emitted_total",
            "Total events emitted to subscribers",
        )
        .unwrap();
        let websocket_connections =
            IntGauge::new("adic_websocket_connections", "Active WebSocket connections").unwrap();
        let sse_connections =
            IntGauge::new("adic_sse_connections", "Active SSE connections").unwrap();
        let websocket_messages_sent = IntCounter::new(
            "adic_websocket_messages_sent_total",
            "Total WebSocket messages sent",
        )
        .unwrap();
        let sse_messages_sent =
            IntCounter::new("adic_sse_messages_sent_total", "Total SSE messages sent").unwrap();

        // Foundation Layer - VRF Metrics
        let vrf_commits_total = IntCounter::new(
            "adic_vrf_commits_total",
            "Total VRF commits submitted",
        )
        .unwrap();
        let vrf_reveals_total = IntCounter::new(
            "adic_vrf_reveals_total",
            "Total VRF reveals submitted",
        )
        .unwrap();
        let vrf_finalizations_total = IntCounter::new(
            "adic_vrf_finalizations_total",
            "Total VRF epoch finalizations",
        )
        .unwrap();
        let vrf_commit_duration = Histogram::with_opts(HistogramOpts::new(
            "adic_vrf_commit_duration_seconds",
            "VRF commit operation duration",
        ))
        .unwrap();
        let vrf_reveal_duration = Histogram::with_opts(HistogramOpts::new(
            "adic_vrf_reveal_duration_seconds",
            "VRF reveal operation duration",
        ))
        .unwrap();
        let vrf_finalize_duration = Histogram::with_opts(HistogramOpts::new(
            "adic_vrf_finalize_duration_seconds",
            "VRF finalization operation duration",
        ))
        .unwrap();

        // Foundation Layer - Quorum Selection Metrics
        let quorum_selections_total = IntCounter::new(
            "adic_quorum_selections_total",
            "Total quorum committee selections",
        )
        .unwrap();
        let quorum_selection_duration = Histogram::with_opts(HistogramOpts::new(
            "adic_quorum_selection_duration_seconds",
            "Quorum committee selection duration",
        ))
        .unwrap();
        let quorum_committee_size = IntGauge::new(
            "adic_quorum_committee_size",
            "Current quorum committee size",
        )
        .unwrap();
        let quorum_votes_total = IntCounter::new(
            "adic_quorum_votes_total",
            "Total quorum votes processed",
        )
        .unwrap();
        let quorum_votes_passed = IntCounter::new(
            "adic_quorum_votes_passed",
            "Total quorum votes that passed",
        )
        .unwrap();
        let quorum_votes_failed = IntCounter::new(
            "adic_quorum_votes_failed",
            "Total quorum votes that failed",
        )
        .unwrap();
        let quorum_vote_duration = Histogram::with_opts(HistogramOpts::new(
            "adic_quorum_vote_duration_seconds",
            "Quorum vote processing duration",
        ))
        .unwrap();

        // Foundation Layer - Challenge Metrics
        let challenge_windows_opened = IntCounter::new(
            "adic_challenge_windows_opened_total",
            "Total challenge windows opened",
        )
        .unwrap();
        let challenges_submitted = IntCounter::new(
            "adic_challenges_submitted_total",
            "Total challenges submitted",
        )
        .unwrap();
        let fraud_proofs_submitted = IntCounter::new(
            "adic_fraud_proofs_submitted_total",
            "Total fraud proofs submitted",
        )
        .unwrap();
        let fraud_proofs_verified = IntCounter::new(
            "adic_fraud_proofs_verified_total",
            "Total fraud proofs verified as valid",
        )
        .unwrap();
        let fraud_proofs_rejected = IntCounter::new(
            "adic_fraud_proofs_rejected_total",
            "Total fraud proofs rejected as invalid",
        )
        .unwrap();
        let arbitrations_started = IntCounter::new(
            "adic_arbitrations_started_total",
            "Total arbitrations started",
        )
        .unwrap();
        let arbitrations_completed = IntCounter::new(
            "adic_arbitrations_completed_total",
            "Total arbitrations completed",
        )
        .unwrap();
        let challenge_windows_active = IntGauge::new(
            "adic_challenge_windows_active",
            "Number of active challenge windows",
        )
        .unwrap();

        // Foundation Layer - Escrow Metrics
        let escrow_locks_total = IntCounter::new(
            "adic_escrow_locks_total",
            "Total escrow locks created",
        )
        .unwrap();
        let escrow_releases_total = IntCounter::new(
            "adic_escrow_releases_total",
            "Total escrow funds released",
        )
        .unwrap();
        let escrow_slashes_total = IntCounter::new(
            "adic_escrow_slashes_total",
            "Total escrow funds slashed",
        )
        .unwrap();
        let escrow_refunds_total = IntCounter::new(
            "adic_escrow_refunds_total",
            "Total escrow funds refunded",
        )
        .unwrap();
        let escrow_lock_duration = Histogram::with_opts(HistogramOpts::new(
            "adic_escrow_lock_duration_seconds",
            "Escrow lock operation duration",
        ))
        .unwrap();
        let escrow_locked_amount = IntGauge::new(
            "adic_escrow_locked_amount_adic",
            "Total amount currently locked in escrow (in ADIC)",
        )
        .unwrap();

        registry
            .register(Box::new(messages_submitted.clone()))
            .unwrap();
        registry
            .register(Box::new(messages_processed.clone()))
            .unwrap();
        registry
            .register(Box::new(messages_failed.clone()))
            .unwrap();
        registry
            .register(Box::new(mrw_attempts_total.clone()))
            .unwrap();
        registry
            .register(Box::new(mrw_widens_total.clone()))
            .unwrap();
        registry
            .register(Box::new(mrw_selection_duration.clone()))
            .unwrap();
        registry
            .register(Box::new(admissibility_checks_total.clone()))
            .unwrap();
        registry
            .register(Box::new(admissibility_s_failures.clone()))
            .unwrap();
        registry
            .register(Box::new(admissibility_c2_failures.clone()))
            .unwrap();
        registry
            .register(Box::new(admissibility_c3_failures.clone()))
            .unwrap();
        registry
            .register(Box::new(finalizations_total.clone()))
            .unwrap();
        registry.register(Box::new(kcore_size.clone())).unwrap();
        registry.register(Box::new(finality_depth.clone())).unwrap();
        registry
            .register(Box::new(f2_computation_time.clone()))
            .unwrap();
        registry
            .register(Box::new(f2_timeout_count.clone()))
            .unwrap();
        registry
            .register(Box::new(f1_fallback_count.clone()))
            .unwrap();
        registry
            .register(Box::new(finality_method_used.clone()))
            .unwrap();
        registry
            .register(Box::new(signature_verifications.clone()))
            .unwrap();
        registry
            .register(Box::new(signature_failures.clone()))
            .unwrap();
        registry.register(Box::new(current_tips.clone())).unwrap();
        registry.register(Box::new(dag_messages.clone())).unwrap();
        registry
            .register(Box::new(events_emitted_total.clone()))
            .unwrap();
        registry
            .register(Box::new(websocket_connections.clone()))
            .unwrap();
        registry
            .register(Box::new(sse_connections.clone()))
            .unwrap();
        registry
            .register(Box::new(websocket_messages_sent.clone()))
            .unwrap();
        registry
            .register(Box::new(sse_messages_sent.clone()))
            .unwrap();

        // Register Foundation Layer - VRF metrics
        registry
            .register(Box::new(vrf_commits_total.clone()))
            .unwrap();
        registry
            .register(Box::new(vrf_reveals_total.clone()))
            .unwrap();
        registry
            .register(Box::new(vrf_finalizations_total.clone()))
            .unwrap();
        registry
            .register(Box::new(vrf_commit_duration.clone()))
            .unwrap();
        registry
            .register(Box::new(vrf_reveal_duration.clone()))
            .unwrap();
        registry
            .register(Box::new(vrf_finalize_duration.clone()))
            .unwrap();

        // Register Foundation Layer - Quorum metrics
        registry
            .register(Box::new(quorum_selections_total.clone()))
            .unwrap();
        registry
            .register(Box::new(quorum_selection_duration.clone()))
            .unwrap();
        registry
            .register(Box::new(quorum_committee_size.clone()))
            .unwrap();

        // Register Foundation Layer - Challenge metrics
        registry
            .register(Box::new(challenge_windows_opened.clone()))
            .unwrap();
        registry
            .register(Box::new(challenges_submitted.clone()))
            .unwrap();
        registry
            .register(Box::new(fraud_proofs_submitted.clone()))
            .unwrap();
        registry
            .register(Box::new(fraud_proofs_verified.clone()))
            .unwrap();
        registry
            .register(Box::new(fraud_proofs_rejected.clone()))
            .unwrap();
        registry
            .register(Box::new(arbitrations_started.clone()))
            .unwrap();
        registry
            .register(Box::new(arbitrations_completed.clone()))
            .unwrap();
        registry
            .register(Box::new(challenge_windows_active.clone()))
            .unwrap();

        // Register Foundation Layer - Escrow metrics
        registry
            .register(Box::new(escrow_locks_total.clone()))
            .unwrap();
        registry
            .register(Box::new(escrow_releases_total.clone()))
            .unwrap();
        registry
            .register(Box::new(escrow_slashes_total.clone()))
            .unwrap();
        registry
            .register(Box::new(escrow_refunds_total.clone()))
            .unwrap();
        registry
            .register(Box::new(escrow_lock_duration.clone()))
            .unwrap();
        registry
            .register(Box::new(escrow_locked_amount.clone()))
            .unwrap();

        Self {
            registry,
            messages_submitted,
            messages_processed,
            messages_failed,
            mrw_attempts_total,
            mrw_widens_total,
            mrw_selection_duration,
            admissibility_checks_total,
            admissibility_s_failures,
            admissibility_c2_failures,
            admissibility_c3_failures,
            finalizations_total,
            kcore_size,
            finality_depth,
            f2_computation_time,
            f2_timeout_count,
            f1_fallback_count,
            finality_method_used,
            signature_verifications,
            signature_failures,
            current_tips,
            dag_messages,
            events_emitted_total,
            websocket_connections,
            sse_connections,
            websocket_messages_sent,
            sse_messages_sent,
            vrf_commits_total,
            vrf_reveals_total,
            vrf_finalizations_total,
            vrf_commit_duration,
            vrf_reveal_duration,
            vrf_finalize_duration,
            quorum_selections_total,
            quorum_selection_duration,
            quorum_committee_size,
            quorum_votes_total,
            quorum_votes_passed,
            quorum_votes_failed,
            quorum_vote_duration,
            challenge_windows_opened,
            challenges_submitted,
            fraud_proofs_submitted,
            fraud_proofs_verified,
            fraud_proofs_rejected,
            arbitrations_started,
            arbitrations_completed,
            challenge_windows_active,
            escrow_locks_total,
            escrow_releases_total,
            escrow_slashes_total,
            escrow_refunds_total,
            escrow_lock_duration,
            escrow_locked_amount,
        }
    }

    pub fn gather(&self) -> String {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        encoder
            .encode_to_string(&metric_families)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_gather() {
        let m = Metrics::new();
        m.messages_submitted.inc();
        m.finalizations_total.inc_by(2);
        let text = m.gather();
        assert!(text.contains("adic_messages_submitted_total"));
        assert!(text.contains("adic_finalizations_total"));
    }

    // Note: Network propagation metrics (gossiped, duplicate, latency) removed
    // as they require complex gossip protocol instrumentation (Phase 2)

    #[test]
    fn test_all_metrics_registered() {
        let m = Metrics::new();
        let text = m.gather();

        // Verify all core metrics exist
        assert!(text.contains("adic_messages_submitted_total"));
        assert!(text.contains("adic_messages_processed_total"));
        assert!(text.contains("adic_messages_failed_total"));

        // MRW metrics
        assert!(text.contains("adic_mrw_attempts_total"));
        assert!(text.contains("adic_mrw_widens_total"));

        // Admissibility metrics
        assert!(text.contains("adic_admissibility_checks_total"));

        // Finality metrics
        assert!(text.contains("adic_finalizations_total"));
        assert!(text.contains("adic_kcore_size"));

        // Validation metrics
        assert!(text.contains("adic_signature_verifications_total"));

        // DAG state metrics
        assert!(text.contains("adic_current_tips"));
        assert!(text.contains("adic_dag_messages"));

        // Event streaming metrics
        assert!(text.contains("adic_websocket_connections"));
        assert!(text.contains("adic_sse_connections"));

        // Note: Network propagation metrics removed (Phase 2 work)
    }

    #[test]
    fn test_foundation_layer_metrics() {
        let m = Metrics::new();

        // Test VRF metrics
        m.vrf_commits_total.inc();
        m.vrf_reveals_total.inc();
        m.vrf_finalizations_total.inc();
        m.vrf_commit_duration.observe(0.05);

        // Test Quorum selection metrics
        m.quorum_selections_total.inc();
        m.quorum_committee_size.set(15);
        // Note: quorum_votes_* metrics are in DisputeAdjudicator (storage market)

        // Test Challenge metrics
        m.challenge_windows_opened.inc();
        m.challenges_submitted.inc();
        m.fraud_proofs_submitted.inc();
        m.fraud_proofs_verified.inc();
        m.arbitrations_started.inc();
        m.challenge_windows_active.set(5);

        // Test Escrow metrics
        m.escrow_locks_total.inc();
        m.escrow_releases_total.inc();
        m.escrow_slashes_total.inc();
        m.escrow_locked_amount.set(1000);

        let text = m.gather();

        // Verify VRF metrics
        assert!(text.contains("adic_vrf_commits_total"));
        assert!(text.contains("adic_vrf_reveals_total"));
        assert!(text.contains("adic_vrf_finalizations_total"));
        assert!(text.contains("adic_vrf_commit_duration_seconds"));

        // Verify Quorum selection metrics
        assert!(text.contains("adic_quorum_selections_total"));
        assert!(text.contains("adic_quorum_committee_size"));
        // Note: quorum vote metrics are in DisputeAdjudicator, not main Metrics

        // Verify Challenge metrics
        assert!(text.contains("adic_challenge_windows_opened_total"));
        assert!(text.contains("adic_challenges_submitted_total"));
        assert!(text.contains("adic_fraud_proofs_submitted_total"));
        assert!(text.contains("adic_fraud_proofs_verified_total"));
        assert!(text.contains("adic_arbitrations_started_total"));
        assert!(text.contains("adic_challenge_windows_active"));

        // Verify Escrow metrics
        assert!(text.contains("adic_escrow_locks_total"));
        assert!(text.contains("adic_escrow_releases_total"));
        assert!(text.contains("adic_escrow_slashes_total"));
        assert!(text.contains("adic_escrow_locked_amount_adic"));
    }
}
