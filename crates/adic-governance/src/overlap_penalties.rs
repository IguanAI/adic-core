//! Overlap Penalty Detection and Enforcement
//!
//! Per PoUW III §4.2, coordinated voting rings are detected and penalized
//! with an attenuation factor η ∈ (0, 1] to prevent Sybil attacks.
//!
//! # Algorithm
//!
//! 1. **Overlap Detection**: Track co-voting patterns across proposals
//! 2. **Similarity Calculation**: Compute Jaccard similarity for voter pairs
//! 3. **Attenuation Factor**: η = 1 / (1 + λ · overlap_score)
//!    - λ: sensitivity parameter (default 2.0)
//!    - overlap_score: weighted average of recent co-voting frequency
//!
//! # Example
//!
//! ```ignore
//! use adic_governance::OverlapPenaltyTracker;
//!
//! let tracker = OverlapPenaltyTracker::new(2.0, 0.8, 20);
//!
//! // Record votes from a proposal
//! tracker.record_votes(&proposal_id, &voter_ids).await;
//!
//! // Get attenuated credits
//! let original_credits = 10.0;
//! let attenuated = tracker.apply_penalty(&voter_id, original_credits).await;
//! // If voter is in coordinated ring: attenuated < original_credits
//! ```

use crate::metrics;
use adic_types::PublicKey;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Default sensitivity parameter (λ)
const DEFAULT_LAMBDA: f64 = 2.0;

/// Default decay factor for historical co-voting
const DEFAULT_DECAY: f64 = 0.8;

/// Default window size for tracking proposals
const DEFAULT_WINDOW_SIZE: usize = 20;

/// Minimum proposals needed before applying penalties
const MIN_PROPOSALS_FOR_PENALTY: usize = 5;

/// Co-voting record for a single proposal
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProposalVotingPattern {
    proposal_id: [u8; 32],
    voters: HashSet<PublicKey>,
    timestamp: u64,
}

/// Overlap statistics for a voter
#[derive(Debug, Clone)]
struct OverlapStats {
    /// Number of proposals this voter participated in
    participation_count: usize,
    /// Map of co-voters and co-voting frequency
    co_voting_frequency: HashMap<PublicKey, f64>,
    /// Current attenuation factor (η)
    attenuation_factor: f64,
}

/// Overlap penalty tracker
///
/// Detects coordinated voting rings and applies attenuation per PoUW III §4.2
pub struct OverlapPenaltyTracker {
    /// Sensitivity parameter (λ) - higher values = stronger penalties
    lambda: f64,

    /// Decay factor for historical co-voting (β ∈ [0, 1])
    /// Recent proposals weighted more heavily
    decay_factor: f64,

    /// Window size for tracking proposals
    window_size: usize,

    /// Recent voting patterns (sliding window)
    voting_history: Arc<RwLock<VecDeque<ProposalVotingPattern>>>,

    /// Per-voter overlap statistics
    voter_stats: Arc<RwLock<HashMap<PublicKey, OverlapStats>>>,

    /// Epoch counter for decay
    current_epoch: Arc<RwLock<u64>>,
}

impl OverlapPenaltyTracker {
    /// Create new overlap penalty tracker
    ///
    /// # Parameters
    /// - `lambda`: Sensitivity (default 2.0)
    /// - `decay_factor`: Historical decay (default 0.8)
    /// - `window_size`: Proposal window (default 20)
    pub fn new(lambda: f64, decay_factor: f64, window_size: usize) -> Self {
        Self {
            lambda,
            decay_factor,
            window_size,
            voting_history: Arc::new(RwLock::new(VecDeque::with_capacity(window_size))),
            voter_stats: Arc::new(RwLock::new(HashMap::new())),
            current_epoch: Arc::new(RwLock::new(0)),
        }
    }

    /// Create with default parameters
    pub fn default() -> Self {
        Self::new(DEFAULT_LAMBDA, DEFAULT_DECAY, DEFAULT_WINDOW_SIZE)
    }

    /// Record votes from a proposal
    pub async fn record_votes(&self, proposal_id: [u8; 32], voters: &[PublicKey]) {
        let voters_set: HashSet<PublicKey> = voters.iter().cloned().collect();
        let epoch = *self.current_epoch.read().await;

        let pattern = ProposalVotingPattern {
            proposal_id,
            voters: voters_set.clone(),
            timestamp: epoch,
        };

        // Add to history (maintain sliding window)
        {
            let mut history = self.voting_history.write().await;
            if history.len() >= self.window_size {
                history.pop_front();
            }
            history.push_back(pattern);
        }

        // Update voter statistics
        self.update_voter_stats(&voters_set).await;

        // Metrics
        metrics::OVERLAP_PROPOSALS_RECORDED.inc();

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            voter_count = voters.len(),
            "📊 Recorded voting pattern for overlap detection"
        );
    }

    /// Update overlap statistics for voters
    async fn update_voter_stats(&self, current_voters: &HashSet<PublicKey>) {
        let start = Instant::now();
        let history = self.voting_history.read().await;

        // Need sufficient history for meaningful statistics
        if history.len() < MIN_PROPOSALS_FOR_PENALTY {
            debug!("Insufficient history for overlap penalties (need {})", MIN_PROPOSALS_FOR_PENALTY);
            return;
        }

        let mut stats = self.voter_stats.write().await;

        // For each voter in current proposal
        for voter in current_voters {
            let mut overlap_freq: HashMap<PublicKey, f64> = HashMap::new();
            let mut participation = 0;

            // Scan historical proposals with exponential decay
            for (idx, pattern) in history.iter().rev().enumerate() {
                if pattern.voters.contains(voter) {
                    participation += 1;

                    // Weight by recency: w_i = β^i
                    let weight = self.decay_factor.powi(idx as i32);

                    // Find co-voters in this proposal
                    for co_voter in &pattern.voters {
                        if co_voter != voter {
                            *overlap_freq.entry(*co_voter).or_insert(0.0) += weight;
                        }
                    }
                }
            }

            // Normalize by participation count
            if participation > 0 {
                for freq in overlap_freq.values_mut() {
                    *freq /= participation as f64;
                }
            }

            // Calculate overlap score (average co-voting frequency with top co-voters)
            let overlap_score = if overlap_freq.is_empty() {
                0.0
            } else {
                // Take top 5 most frequent co-voters
                let mut frequencies: Vec<f64> = overlap_freq.values().cloned().collect();
                frequencies.sort_by(|a, b| b.partial_cmp(a).unwrap());
                let top_k = frequencies.iter().take(5).copied().collect::<Vec<_>>();
                top_k.iter().sum::<f64>() / top_k.len() as f64
            };

            // Calculate attenuation factor: η = 1 / (1 + λ · overlap_score)
            let attenuation = 1.0 / (1.0 + self.lambda * overlap_score);

            // Store stats
            stats.insert(
                *voter,
                OverlapStats {
                    participation_count: participation,
                    co_voting_frequency: overlap_freq,
                    attenuation_factor: attenuation,
                },
            );

            if attenuation < 1.0 {
                debug!(
                    voter = hex::encode(&voter.as_bytes()[..8]),
                    overlap_score = overlap_score,
                    attenuation = attenuation,
                    participation = participation,
                    "⚠️  Overlap penalty applied"
                );
            }
        }

        // Record computation time
        let elapsed = start.elapsed();
        metrics::OVERLAP_SCORE_COMPUTATION_TIME.observe(elapsed.as_secs_f64());
    }

    /// Apply overlap penalty to voting credits
    ///
    /// Returns attenuated credits: credits_attenuated = η · credits_original
    pub async fn apply_penalty(&self, voter: &PublicKey, original_credits: f64) -> f64 {
        let stats = self.voter_stats.read().await;

        match stats.get(voter) {
            Some(voter_stats) => {
                let attenuated = voter_stats.attenuation_factor * original_credits;

                if attenuated < original_credits {
                    // Metrics
                    metrics::OVERLAP_PENALTIES_APPLIED.inc();
                    metrics::OVERLAP_PENALTY_FACTOR.observe(voter_stats.attenuation_factor);

                    let reduction_pct = (original_credits - attenuated) / original_credits * 100.0;
                    metrics::OVERLAP_PENALTY_CREDITS_REDUCTION.observe(reduction_pct);

                    info!(
                        voter = hex::encode(&voter.as_bytes()[..8]),
                        original = original_credits,
                        attenuated = attenuated,
                        eta = voter_stats.attenuation_factor,
                        reduction_pct = reduction_pct,
                        "🔻 Overlap penalty applied"
                    );
                }

                attenuated
            }
            None => {
                // No history yet - no penalty
                original_credits
            }
        }
    }

    /// Get attenuation factor for a voter
    pub async fn get_attenuation_factor(&self, voter: &PublicKey) -> f64 {
        let stats = self.voter_stats.read().await;
        stats
            .get(voter)
            .map(|s| s.attenuation_factor)
            .unwrap_or(1.0)
    }

    /// Get overlap statistics for a voter
    pub async fn get_voter_stats(&self, voter: &PublicKey) -> Option<(usize, f64)> {
        let stats = self.voter_stats.read().await;
        stats.get(voter).map(|s| (s.participation_count, s.attenuation_factor))
    }

    /// Advance epoch (for decay calculation)
    pub async fn advance_epoch(&self) {
        let mut epoch = self.current_epoch.write().await;
        *epoch += 1;
    }

    /// Get number of tracked proposals in window
    pub async fn get_window_size(&self) -> usize {
        let history = self.voting_history.read().await;
        history.len()
    }

    /// Detect voting rings (groups with high mutual overlap)
    pub async fn detect_rings(&self, min_ring_size: usize, min_overlap: f64) -> Vec<Vec<PublicKey>> {
        let stats = self.voter_stats.read().await;
        let mut rings = Vec::new();

        // Simple ring detection: find cliques of voters with mutual high overlap
        let voters: Vec<PublicKey> = stats.keys().cloned().collect();

        for i in 0..voters.len() {
            let voter_a = &voters[i];
            if let Some(stats_a) = stats.get(voter_a) {
                let mut ring = vec![*voter_a];

                for j in (i + 1)..voters.len() {
                    let voter_b = &voters[j];

                    // Check if voter_b co-votes frequently with voter_a
                    if let Some(freq_ab) = stats_a.co_voting_frequency.get(voter_b) {
                        if *freq_ab >= min_overlap {
                            // Check reverse direction
                            if let Some(stats_b) = stats.get(voter_b) {
                                if let Some(freq_ba) = stats_b.co_voting_frequency.get(voter_a) {
                                    if *freq_ba >= min_overlap {
                                        ring.push(*voter_b);
                                    }
                                }
                            }
                        }
                    }
                }

                if ring.len() >= min_ring_size {
                    // Metrics
                    metrics::VOTING_RINGS_DETECTED.inc();
                    metrics::VOTING_RING_SIZE.observe(ring.len() as f64);

                    rings.push(ring);
                }
            }
        }

        if !rings.is_empty() {
            warn!(
                ring_count = rings.len(),
                "🚨 Detected potential coordinated voting rings"
            );
        }

        rings
    }

    /// Reset all statistics (for testing or governance reset)
    pub async fn reset(&self) {
        let mut history = self.voting_history.write().await;
        let mut stats = self.voter_stats.write().await;
        history.clear();
        stats.clear();
        info!("Reset overlap penalty tracker");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_voters(count: usize) -> Vec<PublicKey> {
        (0..count)
            .map(|i| PublicKey::from_bytes([i as u8; 32]))
            .collect()
    }

    #[tokio::test]
    async fn test_no_penalty_for_isolated_voters() {
        let tracker = OverlapPenaltyTracker::default();
        let voters = create_test_voters(10);

        // Each proposal has different voters (no overlap)
        for i in 0..10 {
            tracker
                .record_votes([i as u8; 32], &[voters[i]])
                .await;
        }

        // No penalty should be applied (η = 1.0)
        for voter in &voters {
            let eta = tracker.get_attenuation_factor(voter).await;
            assert_eq!(eta, 1.0);

            let credits = tracker.apply_penalty(voter, 100.0).await;
            assert_eq!(credits, 100.0);
        }
    }

    #[tokio::test]
    async fn test_penalty_for_coordinated_ring() {
        let tracker = OverlapPenaltyTracker::new(2.0, 0.9, 20);
        let voters = create_test_voters(10);

        // Voters 0, 1, 2 always vote together (coordinated ring)
        let ring = &voters[0..3];

        // Ring votes on proposals 0-7
        for i in 0..8 {
            tracker.record_votes([i as u8; 32], ring).await;
        }

        // Voters 3-9 vote independently on different proposals
        for i in 8..15 {
            tracker
                .record_votes([i as u8; 32], &[voters[3 + ((i - 8) % 7)]])
                .await;
        }

        // Ring members should have penalty (η < 1.0)
        for voter in ring {
            let eta = tracker.get_attenuation_factor(voter).await;
            assert!(eta < 1.0, "Ring member should have penalty, got eta={}", eta);
            assert!(eta > 0.3, "Penalty should not be too severe, got eta={}", eta);
            assert!(eta < 0.7, "Penalty should be meaningful for coordinated ring, got eta={}", eta);

            let credits = tracker.apply_penalty(voter, 100.0).await;
            assert!(credits < 100.0);
            assert!(credits > 30.0); // At least 30% of original
        }

        // Independent voters should have no penalty (only voted once each)
        for i in 3..10 {
            let eta = tracker.get_attenuation_factor(&voters[i]).await;
            // May not have stats yet if not enough history
            if eta < 1.0 {
                assert!(eta > 0.95, "Independent voter should have minimal penalty");
            }
        }
    }

    #[tokio::test]
    async fn test_decay_factor_reduces_old_overlaps() {
        let tracker = OverlapPenaltyTracker::new(2.0, 0.5, 20); // Strong decay
        let voters = create_test_voters(5);

        // Voters 0, 1 voted together in old proposals
        for i in 0..5 {
            tracker.record_votes([i as u8; 32], &voters[0..2]).await;
        }

        // Check penalty after old overlaps
        let eta_old = tracker.get_attenuation_factor(&voters[0]).await;
        assert!(eta_old < 1.0);

        // Now voter 0 votes independently
        for i in 5..15 {
            tracker.record_votes([i as u8; 32], &[voters[0]]).await;
        }

        // Penalty should reduce (old overlaps decayed)
        let eta_new = tracker.get_attenuation_factor(&voters[0]).await;
        assert!(eta_new > eta_old, "Penalty should reduce over time");
    }

    #[tokio::test]
    async fn test_insufficient_history_no_penalty() {
        let tracker = OverlapPenaltyTracker::default();
        let voters = create_test_voters(3);

        // Only 3 proposals (< MIN_PROPOSALS_FOR_PENALTY = 5)
        for i in 0..3 {
            tracker.record_votes([i as u8; 32], &voters).await;
        }

        // No penalty with insufficient history
        for voter in &voters {
            let eta = tracker.get_attenuation_factor(voter).await;
            assert_eq!(eta, 1.0);
        }
    }

    #[tokio::test]
    async fn test_ring_detection() {
        let tracker = OverlapPenaltyTracker::new(2.0, 0.9, 20);
        let voters = create_test_voters(10);

        // Create obvious ring: voters 0, 1, 2 always vote together
        for i in 0..15 {
            tracker.record_votes([i as u8; 32], &voters[0..3]).await;
        }

        // Detect rings with lower overlap threshold (since we normalize by participation)
        let rings = tracker.detect_rings(2, 0.5).await;

        assert!(!rings.is_empty(), "Should detect coordinated ring");

        // Check that ring contains at least 2 of the coordinated voters
        let ring_has_coordinated = rings.iter().any(|ring| {
            let count = ring.iter().filter(|v| {
                **v == voters[0] || **v == voters[1] || **v == voters[2]
            }).count();
            count >= 2
        });

        assert!(ring_has_coordinated, "Ring should contain coordinated voters");
    }

    #[tokio::test]
    async fn test_lambda_sensitivity() {
        let voters = create_test_voters(3);

        // Low lambda (lenient)
        let tracker_lenient = OverlapPenaltyTracker::new(0.5, 0.9, 20);
        for i in 0..10 {
            tracker_lenient
                .record_votes([i as u8; 32], &voters)
                .await;
        }
        let eta_lenient = tracker_lenient.get_attenuation_factor(&voters[0]).await;

        // High lambda (strict)
        let tracker_strict = OverlapPenaltyTracker::new(5.0, 0.9, 20);
        for i in 0..10 {
            tracker_strict
                .record_votes([i as u8; 32], &voters)
                .await;
        }
        let eta_strict = tracker_strict.get_attenuation_factor(&voters[0]).await;

        assert!(eta_lenient > eta_strict, "Higher lambda should result in stronger penalty");
    }
}
