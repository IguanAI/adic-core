use crate::{ChallengeError, DisputeRuling, FraudProof, Result};
use adic_quorum::{QuorumConfig, QuorumResult, QuorumSelector, QuorumVerifier, QuorumVote};
use adic_types::{MessageId, PublicKey};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

/// Result of dispute adjudication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdjudicationResult {
    pub fraud_proof_id: MessageId,
    pub ruling: DisputeRuling,
    pub quorum: QuorumResult,
    pub votes_for: Vec<ArbitratorVote>,
    pub votes_against: Vec<ArbitratorVote>,
    pub total_votes: usize,
    pub threshold_met: bool,
    pub adjudicated_at_epoch: u64,
}

/// Vote from an arbitrator on a fraud proof
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitratorVote {
    pub voter: PublicKey,
    pub vote: VoteDecision,
    pub submitted_at_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoteDecision {
    /// Fraud proof is valid (challenger wins)
    Valid,
    /// Fraud proof is invalid (subject wins)
    Invalid,
}

impl QuorumVote for ArbitratorVote {
    fn voter(&self) -> PublicKey {
        self.voter
    }

    fn is_for(&self) -> bool {
        self.vote == VoteDecision::Valid
    }

    fn is_against(&self) -> bool {
        self.vote == VoteDecision::Invalid
    }

    fn is_abstain(&self) -> bool {
        false // No abstention in fraud proof adjudication
    }
}

/// Configuration for dispute adjudication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdjudicationConfig {
    /// Quorum configuration for arbitrator selection
    pub quorum_config: QuorumConfig,

    /// Threshold for ruling (e.g., 0.66 for 2/3 majority)
    pub ruling_threshold: f64,

    /// Window for collecting votes (in epochs)
    pub voting_window: u64,
}

impl Default for AdjudicationConfig {
    fn default() -> Self {
        Self {
            quorum_config: QuorumConfig::default(),
            ruling_threshold: 0.66, // 2/3 majority
            voting_window: 50,      // 50 epochs to vote
        }
    }
}

/// Dispute adjudicator using VRF-based quorum selection
pub struct DisputeAdjudicator {
    quorum_selector: Arc<QuorumSelector>,
    config: AdjudicationConfig,
    // Metrics counters - updated externally by incrementing directly
    pub fraud_proofs_submitted: Option<Arc<prometheus::IntCounter>>,
    pub fraud_proofs_verified: Option<Arc<prometheus::IntCounter>>,
    pub fraud_proofs_rejected: Option<Arc<prometheus::IntCounter>>,
    pub arbitrations_started: Option<Arc<prometheus::IntCounter>>,
    pub arbitrations_completed: Option<Arc<prometheus::IntCounter>>,
    pub quorum_votes_total: Option<Arc<prometheus::IntCounter>>,
    pub quorum_votes_passed: Option<Arc<prometheus::IntCounter>>,
    pub quorum_votes_failed: Option<Arc<prometheus::IntCounter>>,
    pub quorum_vote_duration: Option<Arc<prometheus::Histogram>>,
}

impl DisputeAdjudicator {
    pub fn new(quorum_selector: Arc<QuorumSelector>, config: AdjudicationConfig) -> Self {
        Self {
            quorum_selector,
            config,
            fraud_proofs_submitted: None,
            fraud_proofs_verified: None,
            fraud_proofs_rejected: None,
            arbitrations_started: None,
            arbitrations_completed: None,
            quorum_votes_total: None,
            quorum_votes_passed: None,
            quorum_votes_failed: None,
            quorum_vote_duration: None,
        }
    }

    /// Set metrics for fraud proof tracking
    pub fn set_metrics(
        &mut self,
        fraud_proofs_submitted: Arc<prometheus::IntCounter>,
        fraud_proofs_verified: Arc<prometheus::IntCounter>,
        fraud_proofs_rejected: Arc<prometheus::IntCounter>,
        arbitrations_started: Arc<prometheus::IntCounter>,
        arbitrations_completed: Arc<prometheus::IntCounter>,
        quorum_votes_total: Arc<prometheus::IntCounter>,
        quorum_votes_passed: Arc<prometheus::IntCounter>,
        quorum_votes_failed: Arc<prometheus::IntCounter>,
        quorum_vote_duration: Arc<prometheus::Histogram>,
    ) {
        self.fraud_proofs_submitted = Some(fraud_proofs_submitted);
        self.fraud_proofs_verified = Some(fraud_proofs_verified);
        self.fraud_proofs_rejected = Some(fraud_proofs_rejected);
        self.arbitrations_started = Some(arbitrations_started);
        self.arbitrations_completed = Some(arbitrations_completed);
        self.quorum_votes_total = Some(quorum_votes_total);
        self.quorum_votes_passed = Some(quorum_votes_passed);
        self.quorum_votes_failed = Some(quorum_votes_failed);
        self.quorum_vote_duration = Some(quorum_vote_duration);
    }

    /// Select arbitrator committee for a fraud proof
    pub async fn select_arbitrators(
        &self,
        proof: &FraudProof,
        eligible_nodes: Vec<adic_quorum::NodeInfo>,
    ) -> Result<QuorumResult> {
        let start = std::time::Instant::now();

        // Use fraud proof ID as domain for VRF (future enhancement)
        let _domain = format!("fraud_proof:{}", hex::encode(proof.id.as_bytes()));

        let quorum = self
            .quorum_selector
            .select_committee(
                proof.submitted_at_epoch,
                &self.config.quorum_config,
                eligible_nodes,
            )
            .await
            .map_err(|e| ChallengeError::QuorumError(e))?;

        info!(
            fraud_proof_id = hex::encode(proof.id.as_bytes()),
            arbitrators = quorum.members.len(),
            epoch = proof.submitted_at_epoch,
            axes_coverage = quorum.axes_coverage.len(),
            duration_ms = start.elapsed().as_millis() as u64,
            "👨‍⚖️ Arbitrator committee selected"
        );

        Ok(quorum)
    }

    /// Adjudicate a fraud proof based on arbitrator votes
    pub async fn adjudicate(
        &self,
        proof: &FraudProof,
        quorum: QuorumResult,
        votes: Vec<ArbitratorVote>,
        current_epoch: u64,
    ) -> Result<AdjudicationResult> {
        let start = std::time::Instant::now();

        // Update metrics - fraud proof submitted
        if let Some(ref counter) = self.fraud_proofs_submitted {
            counter.inc();
        }

        // Update metrics - arbitration started
        if let Some(ref counter) = self.arbitrations_started {
            counter.inc();
        }

        // Verify voting window hasn't expired
        if current_epoch > proof.submitted_at_epoch + self.config.voting_window {
            return Err(ChallengeError::WindowExpired {
                id: hex::encode(proof.id.as_bytes()),
            });
        }

        // Verify quorum and tally votes
        let vote_start = std::time::Instant::now();
        let vote_result =
            QuorumVerifier::verify_quorum_vote(&quorum, &votes, self.config.ruling_threshold)
                .map_err(|e| ChallengeError::QuorumError(e))?;

        // Update quorum vote metrics
        if let Some(ref counter) = self.quorum_votes_total {
            counter.inc();
        }
        if let Some(ref histogram) = self.quorum_vote_duration {
            histogram.observe(vote_start.elapsed().as_secs_f64());
        }
        if vote_result.passed {
            if let Some(ref counter) = self.quorum_votes_passed {
                counter.inc();
            }
        } else {
            if let Some(ref counter) = self.quorum_votes_failed {
                counter.inc();
            }
        }

        // Separate votes by decision
        let votes_for: Vec<ArbitratorVote> = votes
            .iter()
            .filter(|v| v.vote == VoteDecision::Valid)
            .cloned()
            .collect();

        let votes_against: Vec<ArbitratorVote> = votes
            .iter()
            .filter(|v| v.vote == VoteDecision::Invalid)
            .cloned()
            .collect();

        // Determine ruling based on threshold
        let ruling = if vote_result.passed {
            DisputeRuling::ChallengerWins // Fraud proof valid
        } else if votes.len() >= (quorum.members.len() / 2) {
            DisputeRuling::SubjectWins // Majority rejected fraud proof
        } else {
            DisputeRuling::Inconclusive // Not enough votes
        };

        let emoji = match ruling {
            DisputeRuling::ChallengerWins => "⚔️",
            DisputeRuling::SubjectWins => "🛡️",
            DisputeRuling::Inconclusive => "❓",
            DisputeRuling::Partial => "⚖️",
        };

        info!(
            fraud_proof_id = hex::encode(proof.id.as_bytes()),
            votes_valid = votes_for.len(),
            votes_invalid = votes_against.len(),
            threshold_met = vote_result.passed,
            ruling = ?ruling,
            duration_ms = start.elapsed().as_millis() as u64,
            "{} Dispute adjudication complete",
            emoji
        );

        // Update metrics - arbitration completed
        if let Some(ref counter) = self.arbitrations_completed {
            counter.inc();
        }

        // Update fraud proof verification metrics based on ruling
        match ruling {
            DisputeRuling::ChallengerWins => {
                // Fraud proof was verified as valid
                if let Some(ref counter) = self.fraud_proofs_verified {
                    counter.inc();
                }
            }
            DisputeRuling::SubjectWins => {
                // Fraud proof was rejected as invalid
                if let Some(ref counter) = self.fraud_proofs_rejected {
                    counter.inc();
                }
            }
            _ => {
                // Inconclusive or Partial - don't count as verified or rejected
            }
        }

        Ok(AdjudicationResult {
            fraud_proof_id: proof.id,
            ruling,
            quorum,
            votes_for,
            votes_against,
            total_votes: votes.len(),
            threshold_met: vote_result.passed,
            adjudicated_at_epoch: current_epoch,
        })
    }

    /// Convenience method: adjudicate with automatic arbitrator selection
    pub async fn adjudicate_with_selection(
        &self,
        proof: &FraudProof,
        eligible_nodes: Vec<adic_quorum::NodeInfo>,
        votes: Vec<ArbitratorVote>,
        current_epoch: u64,
    ) -> Result<AdjudicationResult> {
        let quorum = self.select_arbitrators(proof, eligible_nodes).await?;
        self.adjudicate(proof, quorum, votes, current_epoch).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FraudEvidence, FraudProof, FraudType};
    use adic_consensus::ReputationTracker;
    use adic_types::MessageId;
    use adic_vrf::VRFService;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_arbitrator_selection() {
        // This test verifies the adjudicator can be constructed with proper config
        // Actual quorum selection requires VRF randomness to be established via commit-reveal
        let rep_tracker = Arc::new(ReputationTracker::new(0.5)); // gamma = 0.5
        let vrf_service = Arc::new(VRFService::new(
            adic_vrf::VRFConfig::default(),
            rep_tracker.clone(),
        ));
        let quorum_selector = Arc::new(QuorumSelector::new(vrf_service, rep_tracker));

        let adjudicator = DisputeAdjudicator::new(
            quorum_selector,
            AdjudicationConfig {
                quorum_config: QuorumConfig {
                    total_size: 5,
                    min_reputation: 10.0,
                    ..Default::default()
                },
                ruling_threshold: 0.66,
                voting_window: 50,
            },
        );

        // Verify config
        assert_eq!(adjudicator.config.quorum_config.total_size, 5);
        assert_eq!(adjudicator.config.ruling_threshold, 0.66);
        assert_eq!(adjudicator.config.voting_window, 50);
    }

    #[tokio::test]
    async fn test_adjudication_challenger_wins() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.5)); // gamma = 0.5
        let vrf_service = Arc::new(VRFService::new(
            adic_vrf::VRFConfig::default(),
            rep_tracker.clone(),
        ));
        let quorum_selector = Arc::new(QuorumSelector::new(vrf_service, rep_tracker));

        let adjudicator = DisputeAdjudicator::new(
            quorum_selector.clone(),
            AdjudicationConfig {
                ruling_threshold: 0.66,
                ..Default::default()
            },
        );

        let proof = FraudProof::new(
            MessageId::new(b"subject"),
            PublicKey::from_bytes([1; 32]),
            FraudEvidence {
                fraud_type: FraudType::IncorrectProof,
                evidence_cid: "test".to_string(),
                evidence_data: None,
                metadata: HashMap::new(),
            },
            100,
        );

        // Create mock quorum
        let quorum = QuorumResult {
            members: vec![
                adic_quorum::QuorumMember {
                    public_key: PublicKey::from_bytes([1; 32]),
                    reputation: 50.0,
                    vrf_score: [1; 32],
                    axis_id: 0,
                    asn: Some(100),
                    region: Some("us-west".to_string()),
                },
                adic_quorum::QuorumMember {
                    public_key: PublicKey::from_bytes([2; 32]),
                    reputation: 50.0,
                    vrf_score: [2; 32],
                    axis_id: 0,
                    asn: Some(101),
                    region: Some("us-east".to_string()),
                },
                adic_quorum::QuorumMember {
                    public_key: PublicKey::from_bytes([3; 32]),
                    reputation: 50.0,
                    vrf_score: [3; 32],
                    axis_id: 0,
                    asn: Some(102),
                    region: Some("eu-west".to_string()),
                },
            ],
            total_size: 3,
            axes_coverage: vec![3],
        };

        // 3 votes for Valid (challenger wins)
        let votes = vec![
            ArbitratorVote {
                voter: PublicKey::from_bytes([1; 32]),
                vote: VoteDecision::Valid,
                submitted_at_epoch: 101,
            },
            ArbitratorVote {
                voter: PublicKey::from_bytes([2; 32]),
                vote: VoteDecision::Valid,
                submitted_at_epoch: 101,
            },
            ArbitratorVote {
                voter: PublicKey::from_bytes([3; 32]),
                vote: VoteDecision::Valid,
                submitted_at_epoch: 101,
            },
        ];

        let result = adjudicator
            .adjudicate(&proof, quorum, votes, 105)
            .await
            .unwrap();

        assert_eq!(result.ruling, DisputeRuling::ChallengerWins);
        assert_eq!(result.votes_for.len(), 3);
        assert_eq!(result.votes_against.len(), 0);
        assert!(result.threshold_met);
    }

    #[tokio::test]
    async fn test_adjudication_subject_wins() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.5)); // gamma = 0.5
        let vrf_service = Arc::new(VRFService::new(
            adic_vrf::VRFConfig::default(),
            rep_tracker.clone(),
        ));
        let quorum_selector = Arc::new(QuorumSelector::new(vrf_service, rep_tracker));

        let adjudicator = DisputeAdjudicator::new(
            quorum_selector,
            AdjudicationConfig {
                ruling_threshold: 0.66,
                ..Default::default()
            },
        );

        let proof = FraudProof::new(
            MessageId::new(b"subject"),
            PublicKey::from_bytes([1; 32]),
            FraudEvidence {
                fraud_type: FraudType::InvalidResult,
                evidence_cid: "test".to_string(),
                evidence_data: None,
                metadata: HashMap::new(),
            },
            100,
        );

        let quorum = QuorumResult {
            members: vec![
                adic_quorum::QuorumMember {
                    public_key: PublicKey::from_bytes([1; 32]),
                    reputation: 50.0,
                    vrf_score: [1; 32],
                    axis_id: 0,
                    asn: Some(100),
                    region: Some("us-west".to_string()),
                },
                adic_quorum::QuorumMember {
                    public_key: PublicKey::from_bytes([2; 32]),
                    reputation: 50.0,
                    vrf_score: [2; 32],
                    axis_id: 0,
                    asn: Some(101),
                    region: Some("us-east".to_string()),
                },
                adic_quorum::QuorumMember {
                    public_key: PublicKey::from_bytes([3; 32]),
                    reputation: 50.0,
                    vrf_score: [3; 32],
                    axis_id: 0,
                    asn: Some(102),
                    region: Some("eu-west".to_string()),
                },
            ],
            total_size: 3,
            axes_coverage: vec![3],
        };

        // Majority votes Invalid (subject wins)
        let votes = vec![
            ArbitratorVote {
                voter: PublicKey::from_bytes([1; 32]),
                vote: VoteDecision::Invalid,
                submitted_at_epoch: 101,
            },
            ArbitratorVote {
                voter: PublicKey::from_bytes([2; 32]),
                vote: VoteDecision::Invalid,
                submitted_at_epoch: 101,
            },
            ArbitratorVote {
                voter: PublicKey::from_bytes([3; 32]),
                vote: VoteDecision::Valid,
                submitted_at_epoch: 101,
            },
        ];

        let result = adjudicator
            .adjudicate(&proof, quorum, votes, 105)
            .await
            .unwrap();

        assert_eq!(result.ruling, DisputeRuling::SubjectWins);
        assert_eq!(result.votes_for.len(), 1);
        assert_eq!(result.votes_against.len(), 2);
        assert!(!result.threshold_met);
    }
}
