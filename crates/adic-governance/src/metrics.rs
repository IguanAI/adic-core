//! Prometheus metrics for governance module
//!
//! Tracks proposal lifecycle, voting patterns, overlap penalties, and treasury operations.

use once_cell::sync::Lazy;
use prometheus::{
    register_histogram, register_histogram_vec, register_int_counter, register_int_counter_vec,
    register_int_gauge, Histogram, HistogramVec, IntCounter, IntCounterVec, IntGauge,
};

// ========== Core Governance Metrics ==========

/// Number of active proposals by status
pub static ACTIVE_PROPOSALS: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "adic_governance_active_proposals",
        "Number of active governance proposals"
    )
    .unwrap()
});

/// Proposal lifecycle transitions
pub static PROPOSAL_TRANSITIONS: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "adic_governance_proposal_transitions_total",
        "Total proposal lifecycle transitions",
        &["from_status", "to_status"]
    )
    .unwrap()
});

/// Vote counts
pub static VOTES_CAST: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "adic_governance_votes_cast_total",
        "Total votes cast",
        &["ballot"]
    )
    .unwrap()
});

/// Voting credits tallied
pub static VOTING_CREDITS_TALLIED: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "adic_governance_voting_credits",
        "Voting credits distribution",
        &["ballot"]
    )
    .unwrap()
});

/// Treasury operations
pub static TREASURY_OPERATIONS: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "adic_governance_treasury_operations_total",
        "Total treasury operations",
        &["operation"]
    )
    .unwrap()
});

// ========== Phase 2 Metrics: Overlap Penalties ==========

/// Total overlap penalties applied
pub static OVERLAP_PENALTIES_APPLIED: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "adic_governance_overlap_penalties_applied_total",
        "Total number of overlap penalties applied to votes"
    )
    .unwrap()
});

/// Overlap penalty attenuation factors (0.0 - 1.0)
pub static OVERLAP_PENALTY_FACTOR: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "adic_governance_overlap_penalty_factor",
        "Distribution of overlap penalty attenuation factors (η)",
        vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0]
    )
    .unwrap()
});

/// Credits before and after penalty
pub static OVERLAP_PENALTY_CREDITS_REDUCTION: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "adic_governance_overlap_penalty_credits_reduction",
        "Credits reduction percentage due to overlap penalties"
    )
    .unwrap()
});

/// Ring detection events
pub static VOTING_RINGS_DETECTED: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "adic_governance_voting_rings_detected_total",
        "Total number of coordinated voting rings detected"
    )
    .unwrap()
});

/// Ring size distribution
pub static VOTING_RING_SIZE: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "adic_governance_voting_ring_size",
        "Size of detected voting rings",
        vec![2.0, 3.0, 5.0, 10.0, 20.0, 50.0, 100.0]
    )
    .unwrap()
});

/// Co-voting proposals recorded
pub static OVERLAP_PROPOSALS_RECORDED: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "adic_governance_overlap_proposals_recorded_total",
        "Total number of proposals recorded for overlap tracking"
    )
    .unwrap()
});

/// Overlap score computation time
pub static OVERLAP_SCORE_COMPUTATION_TIME: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "adic_governance_overlap_score_computation_seconds",
        "Time to compute overlap scores for a voter"
    )
    .unwrap()
});

// ========== Phase 2 Metrics: Canonical JSON ==========

/// Canonical JSON serialization time
pub static CANONICAL_JSON_SERIALIZATION_TIME: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "adic_governance_canonical_json_serialization_seconds",
        "Time to serialize to canonical JSON",
        vec![0.000001, 0.000005, 0.00001, 0.00005, 0.0001, 0.0005, 0.001]
    )
    .unwrap()
});

/// Canonical hash computation time
pub static CANONICAL_HASH_COMPUTATION_TIME: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "adic_governance_canonical_hash_computation_seconds",
        "Time to compute canonical hash",
        vec![0.000001, 0.000005, 0.00001, 0.00005, 0.0001, 0.0005, 0.001]
    )
    .unwrap()
});

/// Proposal ID computations
pub static PROPOSAL_ID_COMPUTATIONS: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "adic_governance_proposal_id_computations_total",
        "Total canonical proposal ID computations"
    )
    .unwrap()
});

/// Canonical hash mismatches (should be zero)
pub static CANONICAL_HASH_MISMATCHES: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "adic_governance_canonical_hash_mismatches_total",
        "Total canonical hash mismatches detected (security alert)"
    )
    .unwrap()
});

// ========== Quorum and Voting Metrics ==========

/// Quorum checks
pub static QUORUM_CHECKS: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "adic_governance_quorum_checks_total",
        "Total quorum checks performed",
        &["result"]
    )
    .unwrap()
});

/// Proposal tallying time
pub static PROPOSAL_TALLY_TIME: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "adic_governance_proposal_tally_seconds",
        "Time to tally votes for a proposal"
    )
    .unwrap()
});

/// Vote validation failures
pub static VOTE_VALIDATION_FAILURES: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "adic_governance_vote_validation_failures_total",
        "Total vote validation failures",
        &["reason"]
    )
    .unwrap()
});

// ========== Conflict Resolution Metrics ==========

/// Conflicts detected
pub static CONFLICTS_DETECTED: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "adic_governance_conflicts_detected_total",
        "Total governance proposal conflicts detected"
    )
    .unwrap()
});

/// Conflicts resolved
pub static CONFLICTS_RESOLVED: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "adic_governance_conflicts_resolved_total",
        "Total conflicts resolved",
        &["method"]
    )
    .unwrap()
});
