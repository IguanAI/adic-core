use crate::{QuorumError, QuorumResult, QuorumVote, Result};
use adic_types::PublicKey;
use std::collections::HashSet;
use tracing::{debug, info};

/// Voting result
#[derive(Debug, Clone)]
pub struct VotingResult {
    pub votes_for: usize,
    pub votes_against: usize,
    pub votes_abstain: usize,
    pub total_votes: usize,
    pub quorum_size: usize,
    pub threshold: f64,
    pub passed: bool,
}

/// Quorum verifier for tallying votes
pub struct QuorumVerifier;

impl QuorumVerifier {
    /// Verify quorum vote and tally results
    /// threshold: e.g., 0.66 for 2/3 majority
    pub fn verify_quorum_vote<V: QuorumVote>(
        quorum: &QuorumResult,
        votes: &[V],
        threshold: f64,
    ) -> Result<VotingResult> {
        let start = std::time::Instant::now();

        // Validate quorum membership
        let quorum_members: HashSet<_> = quorum.members.iter().map(|m| m.public_key).collect();

        let mut votes_for = 0;
        let mut votes_against = 0;
        let mut votes_abstain = 0;
        let mut voted = HashSet::new();

        for vote in votes {
            let voter = vote.voter();

            // Check if voter is in quorum
            if !quorum_members.contains(&voter) {
                debug!(
                    voter = hex::encode(voter.as_bytes()),
                    "Vote from non-quorum member ignored"
                );
                continue;
            }

            // Check for duplicate votes
            if voted.contains(&voter) {
                debug!(
                    voter = hex::encode(voter.as_bytes()),
                    "Duplicate vote ignored"
                );
                continue;
            }

            voted.insert(voter);

            // Tally vote
            if vote.is_for() {
                votes_for += 1;
            } else if vote.is_against() {
                votes_against += 1;
            } else if vote.is_abstain() {
                votes_abstain += 1;
            }
        }

        let total_votes = votes_for + votes_against; // Abstentions don't count
        let quorum_size = quorum.members.len();

        // Check if threshold is met
        // passed if: votes_for / (votes_for + votes_against) >= threshold
        let passed = if total_votes > 0 {
            (votes_for as f64) / (total_votes as f64) >= threshold
        } else {
            false
        };

        let result = VotingResult {
            votes_for,
            votes_against,
            votes_abstain,
            total_votes,
            quorum_size,
            threshold,
            passed,
        };

        let emoji = if passed { "✅" } else { "❌" };

        info!(
            votes_for,
            votes_against,
            votes_abstain,
            total_votes,
            quorum_size,
            threshold,
            passed,
            duration_ms = start.elapsed().as_millis() as u64,
            "{} Quorum vote tallied",
            emoji
        );

        Ok(result)
    }

    /// Check if minimum participation is met
    pub fn check_participation(
        result: &VotingResult,
        min_participation_ratio: f64,
    ) -> Result<()> {
        let participation = (result.total_votes as f64) / (result.quorum_size as f64);

        if participation < min_participation_ratio {
            return Err(QuorumError::ThresholdNotMet {
                votes_for: result.votes_for,
                total: result.quorum_size,
                threshold: min_participation_ratio,
            });
        }

        Ok(())
    }

    /// Get voters who voted for
    pub fn get_voters_for<V: QuorumVote>(votes: &[V]) -> Vec<PublicKey> {
        votes
            .iter()
            .filter(|v| v.is_for())
            .map(|v| v.voter())
            .collect()
    }

    /// Get voters who voted against
    pub fn get_voters_against<V: QuorumVote>(votes: &[V]) -> Vec<PublicKey> {
        votes
            .iter()
            .filter(|v| v.is_against())
            .map(|v| v.voter())
            .collect()
    }

    /// Calculate vote distribution as percentages
    pub fn vote_distribution(result: &VotingResult) -> (f64, f64, f64) {
        let total = result.quorum_size as f64;
        let for_pct = (result.votes_for as f64) / total;
        let against_pct = (result.votes_against as f64) / total;
        let abstain_pct = (result.votes_abstain as f64) / total;

        (for_pct, against_pct, abstain_pct)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{QuorumMember, SimpleQuorumVote, VoteType};
    use adic_types::PublicKey;

    fn make_quorum_member(id: u8) -> QuorumMember {
        QuorumMember {
            public_key: PublicKey::from_bytes([id; 32]),
            reputation: 50.0,
            vrf_score: [id; 32],
            axis_id: 0,
            asn: None,
            region: None,
        }
    }

    #[test]
    fn test_quorum_verification() {
        let quorum = QuorumResult {
            members: vec![
                make_quorum_member(1),
                make_quorum_member(2),
                make_quorum_member(3),
            ],
            total_size: 3,
            axes_coverage: vec![1, 1, 1],
        };

        let votes = vec![
            SimpleQuorumVote {
                voter: PublicKey::from_bytes([1; 32]),
                vote_type: VoteType::For,
            },
            SimpleQuorumVote {
                voter: PublicKey::from_bytes([2; 32]),
                vote_type: VoteType::For,
            },
            SimpleQuorumVote {
                voter: PublicKey::from_bytes([3; 32]),
                vote_type: VoteType::Against,
            },
        ];

        let result = QuorumVerifier::verify_quorum_vote(&quorum, &votes, 0.66).unwrap();

        assert_eq!(result.votes_for, 2);
        assert_eq!(result.votes_against, 1);
        assert!(result.passed); // 2/3 >= 0.66
    }

    #[test]
    fn test_quorum_verification_fails() {
        let quorum = QuorumResult {
            members: vec![
                make_quorum_member(1),
                make_quorum_member(2),
                make_quorum_member(3),
            ],
            total_size: 3,
            axes_coverage: vec![1, 1, 1],
        };

        let votes = vec![
            SimpleQuorumVote {
                voter: PublicKey::from_bytes([1; 32]),
                vote_type: VoteType::For,
            },
            SimpleQuorumVote {
                voter: PublicKey::from_bytes([2; 32]),
                vote_type: VoteType::Against,
            },
            SimpleQuorumVote {
                voter: PublicKey::from_bytes([3; 32]),
                vote_type: VoteType::Against,
            },
        ];

        let result = QuorumVerifier::verify_quorum_vote(&quorum, &votes, 0.66).unwrap();

        assert_eq!(result.votes_for, 1);
        assert_eq!(result.votes_against, 2);
        assert!(!result.passed); // 1/3 < 0.66
    }

    #[test]
    fn test_non_quorum_votes_ignored() {
        let quorum = QuorumResult {
            members: vec![make_quorum_member(1)],
            total_size: 1,
            axes_coverage: vec![1],
        };

        let votes = vec![
            SimpleQuorumVote {
                voter: PublicKey::from_bytes([1; 32]),
                vote_type: VoteType::For,
            },
            SimpleQuorumVote {
                voter: PublicKey::from_bytes([99; 32]), // Not in quorum
                vote_type: VoteType::Against,
            },
        ];

        let result = QuorumVerifier::verify_quorum_vote(&quorum, &votes, 0.5).unwrap();

        assert_eq!(result.votes_for, 1);
        assert_eq!(result.votes_against, 0); // Non-quorum vote ignored
        assert!(result.passed);
    }
}
