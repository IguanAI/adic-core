use crate::overlap_penalties::OverlapPenaltyTracker;
use crate::types::{Ballot, GovernanceVote, Hash, QuorumStats, VoteResult};
use crate::{GovernanceError, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

/// Voting engine for tallying governance votes with Rep-based bounded credits
pub struct VotingEngine {
    /// Maximum reputation cap for voting (Rmax)
    rmax: f64,
    /// Minimum quorum participation threshold
    min_quorum: f64,
    /// Optional overlap penalty tracker (for Sybil resistance)
    overlap_tracker: Option<Arc<OverlapPenaltyTracker>>,
}

impl VotingEngine {
    pub fn new(rmax: f64, min_quorum: f64) -> Self {
        Self {
            rmax,
            min_quorum,
            overlap_tracker: None,
        }
    }

    /// Set overlap penalty tracker (for Sybil resistance)
    pub fn with_overlap_tracker(mut self, tracker: Arc<OverlapPenaltyTracker>) -> Self {
        self.overlap_tracker = Some(tracker);
        self
    }

    /// Compute bounded voting credits: w(P) = √min{R(P), Rmax}
    ///
    /// This concave cap function ensures:
    /// - Non-transferable influence (based on reputation)
    /// - Diminishing returns (prevents dominance)
    /// - Capped maximum (√Rmax regardless of reputation)
    pub fn compute_voting_credits(&self, reputation: f64) -> f64 {
        let capped_rep = reputation.min(self.rmax);
        capped_rep.sqrt()
    }

    /// Tally votes for a proposal
    ///
    /// Returns QuorumStats with yes/no/abstain totals
    /// Applies overlap penalties if tracker is configured (per PoUW III §4.2)
    pub async fn tally_votes(
        &self,
        votes: &[GovernanceVote],
        proposal_id: &Hash,
    ) -> Result<QuorumStats> {
        let mut yes_total = 0.0;
        let mut no_total = 0.0;
        let mut abstain_total = 0.0;
        let mut voter_set = HashMap::new();

        for vote in votes {
            // Verify vote is for this proposal
            if &vote.proposal_id != proposal_id {
                debug!(
                    voter = hex::encode(vote.voter_pk.as_bytes()),
                    "Vote for different proposal, skipping"
                );
                continue;
            }

            // Check for duplicate votes
            if voter_set.contains_key(&vote.voter_pk) {
                return Err(GovernanceError::DuplicateVote(hex::encode(
                    vote.voter_pk.as_bytes(),
                )));
            }

            voter_set.insert(vote.voter_pk, true);

            // Apply overlap penalty if configured
            let final_credits = if let Some(ref tracker) = self.overlap_tracker {
                tracker.apply_penalty(&vote.voter_pk, vote.credits).await
            } else {
                vote.credits
            };

            // Tally credits according to ballot (with potential penalty applied)
            match vote.ballot {
                Ballot::Yes => yes_total += final_credits,
                Ballot::No => no_total += final_credits,
                Ballot::Abstain => abstain_total += final_credits,
            }

            debug!(
                voter = hex::encode(&vote.voter_pk.as_bytes()[..8]),
                ballot = ?vote.ballot,
                original_credits = vote.credits,
                final_credits = final_credits,
                "Vote counted"
            );
        }

        let stats = QuorumStats {
            yes: yes_total,
            no: no_total,
            abstain: abstain_total,
            total_participation: yes_total + no_total + abstain_total,
        };

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            yes = stats.yes,
            no = stats.no,
            abstain = stats.abstain,
            total = stats.total_participation,
            unique_voters = voter_set.len(),
            "📊 Vote tally completed"
        );

        Ok(stats)
    }

    /// Check if quorum is met
    pub fn check_quorum(
        &self,
        stats: &QuorumStats,
        total_eligible_credits: f64,
    ) -> Result<()> {
        let participation_rate = stats.total_participation / total_eligible_credits;

        if participation_rate < self.min_quorum {
            return Err(GovernanceError::QuorumNotMet {
                required: self.min_quorum,
                actual: participation_rate,
            });
        }

        Ok(())
    }

    /// Determine vote result based on threshold
    ///
    /// - Constitutional: requires ≥66.7% yes / (yes + no)
    /// - Operational: requires >50% yes / (yes + no)
    pub fn determine_result(
        &self,
        stats: &QuorumStats,
        threshold: f64,
    ) -> Result<VoteResult> {
        let total_directional = stats.yes + stats.no;

        if total_directional == 0.0 {
            info!("No directional votes (yes/no), proposal fails");
            return Ok(VoteResult::Fail);
        }

        let yes_percentage = stats.yes / total_directional;

        let passed = if threshold >= 0.667 {
            // Constitutional: ≥ threshold
            yes_percentage >= threshold
        } else {
            // Operational: > threshold
            yes_percentage > threshold
        };

        let result = if passed {
            VoteResult::Pass
        } else {
            VoteResult::Fail
        };

        info!(
            yes_pct = yes_percentage,
            threshold,
            result = ?result,
            "Vote result determined"
        );

        Ok(result)
    }

    /// Validate vote credits match expected reputation
    pub fn validate_credits(&self, credits: f64, reputation: f64) -> Result<()> {
        let expected = self.compute_voting_credits(reputation);
        let tolerance = 0.0001; // Floating point tolerance

        if (credits - expected).abs() > tolerance {
            return Err(GovernanceError::Other(format!(
                "Credits mismatch: expected {}, got {}",
                expected, credits
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::PublicKey;
    use chrono::Utc;

    #[test]
    fn test_voting_credits_calculation() {
        let engine = VotingEngine::new(100_000.0, 0.1);

        // R = 100 → w = √100 = 10
        assert_eq!(engine.compute_voting_credits(100.0), 10.0);

        // R = 10,000 → w = √10,000 = 100
        assert_eq!(engine.compute_voting_credits(10_000.0), 100.0);

        // R = 1,000,000 → w = √min{1,000,000, 100,000} = √100,000 ≈ 316.23
        let credits = engine.compute_voting_credits(1_000_000.0);
        assert!((credits - 316.227766).abs() < 0.0001);

        // Cap is enforced: 10,000x more Rep → only 31.6x more influence
        let ratio = credits / 10.0;
        assert!((ratio - 31.6227766).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_vote_tallying() {
        let engine = VotingEngine::new(100_000.0, 0.1);
        let proposal_id = [1u8; 32];

        let votes = vec![
            GovernanceVote {
                proposal_id,
                voter_pk: PublicKey::from_bytes([1; 32]),
                credits: 10.0, // √100
                ballot: Ballot::Yes,
                timestamp: Utc::now(),
            },
            GovernanceVote {
                proposal_id,
                voter_pk: PublicKey::from_bytes([2; 32]),
                credits: 100.0, // √10,000
                ballot: Ballot::Yes,
                timestamp: Utc::now(),
            },
            GovernanceVote {
                proposal_id,
                voter_pk: PublicKey::from_bytes([3; 32]),
                credits: 50.0, // √2,500
                ballot: Ballot::No,
                timestamp: Utc::now(),
            },
        ];

        let stats = engine.tally_votes(&votes, &proposal_id).await.unwrap();

        assert_eq!(stats.yes, 110.0);
        assert_eq!(stats.no, 50.0);
        assert_eq!(stats.abstain, 0.0);
        assert_eq!(stats.total_participation, 160.0);
    }

    #[tokio::test]
    async fn test_duplicate_vote_detection() {
        let engine = VotingEngine::new(100_000.0, 0.1);
        let proposal_id = [1u8; 32];

        let votes = vec![
            GovernanceVote {
                proposal_id,
                voter_pk: PublicKey::from_bytes([1; 32]),
                credits: 10.0,
                ballot: Ballot::Yes,
                timestamp: Utc::now(),
            },
            GovernanceVote {
                proposal_id,
                voter_pk: PublicKey::from_bytes([1; 32]), // Duplicate!
                credits: 10.0,
                ballot: Ballot::No,
                timestamp: Utc::now(),
            },
        ];

        let result = engine.tally_votes(&votes, &proposal_id).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GovernanceError::DuplicateVote(_)
        ));
    }

    #[tokio::test]
    async fn test_vote_tallying_with_overlap_penalties() {
        use crate::OverlapPenaltyTracker;

        let tracker = Arc::new(OverlapPenaltyTracker::new(2.0, 0.9, 20));
        let engine = VotingEngine::new(100_000.0, 0.1)
            .with_overlap_tracker(tracker.clone());

        let proposal_id = [1u8; 32];
        let voters = vec![
            PublicKey::from_bytes([1; 32]),
            PublicKey::from_bytes([2; 32]),
            PublicKey::from_bytes([3; 32]),
        ];

        // Create voting ring: voters 1,2,3 vote together on multiple proposals
        for i in 0..10 {
            tracker.record_votes([i as u8; 32], &voters).await;
        }

        // Now they vote on proposal_id
        let votes = vec![
            GovernanceVote {
                proposal_id,
                voter_pk: voters[0],
                credits: 100.0,
                ballot: Ballot::Yes,
                timestamp: Utc::now(),
            },
            GovernanceVote {
                proposal_id,
                voter_pk: voters[1],
                credits: 100.0,
                ballot: Ballot::Yes,
                timestamp: Utc::now(),
            },
            GovernanceVote {
                proposal_id,
                voter_pk: voters[2],
                credits: 100.0,
                ballot: Ballot::No,
                timestamp: Utc::now(),
            },
        ];

        let stats = engine.tally_votes(&votes, &proposal_id).await.unwrap();

        // With penalties, total should be less than 300.0
        assert!(stats.total_participation < 300.0);
        assert!(stats.total_participation > 100.0); // But still meaningful

        // Yes votes should be penalized (ring members)
        assert!(stats.yes < 200.0);
    }

    #[test]
    fn test_threshold_determination() {
        let engine = VotingEngine::new(100_000.0, 0.1);

        // Constitutional threshold (≥66.7%)
        let stats = QuorumStats {
            yes: 67.0,
            no: 33.0,
            abstain: 0.0,
            total_participation: 100.0,
        };
        let result = engine.determine_result(&stats, 0.667).unwrap();
        assert_eq!(result, VoteResult::Pass);

        // Just below constitutional threshold
        let stats = QuorumStats {
            yes: 66.0,
            no: 34.0,
            abstain: 0.0,
            total_participation: 100.0,
        };
        let result = engine.determine_result(&stats, 0.667).unwrap();
        assert_eq!(result, VoteResult::Fail);

        // Operational threshold (>50%)
        let stats = QuorumStats {
            yes: 51.0,
            no: 49.0,
            abstain: 0.0,
            total_participation: 100.0,
        };
        let result = engine.determine_result(&stats, 0.5).unwrap();
        assert_eq!(result, VoteResult::Pass);

        // Exactly 50% fails for operational
        let stats = QuorumStats {
            yes: 50.0,
            no: 50.0,
            abstain: 0.0,
            total_participation: 100.0,
        };
        let result = engine.determine_result(&stats, 0.5).unwrap();
        assert_eq!(result, VoteResult::Fail);
    }
}
