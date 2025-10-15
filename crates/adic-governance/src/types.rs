use adic_crypto::BLSSignature;
use adic_economics::types::AdicAmount;
use adic_types::PublicKey;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Hash type for governance artifacts
pub type Hash = [u8; 32];

/// Governance proposal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceProposal {
    pub proposal_id: Hash,
    pub class: ProposalClass,
    pub proposer_pk: PublicKey,
    pub param_keys: Vec<String>,
    pub new_values: serde_json::Value,
    pub axis_changes: Option<AxisCatalogChange>,
    pub treasury_grant: Option<TreasuryGrant>,
    pub enact_epoch: u64,
    pub rationale_cid: String,
    pub creation_timestamp: DateTime<Utc>,
    pub voting_end_timestamp: DateTime<Utc>,
    pub status: ProposalStatus,
    pub tally_yes: f64,
    pub tally_no: f64,
    pub tally_abstain: f64,
}

/// Proposal priority classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposalClass {
    /// Higher priority, longer timelock, requires supermajority (≥66.7%)
    Constitutional,
    /// Lower priority, shorter timelock, requires simple majority (>50%)
    Operational,
}

/// Proposal lifecycle status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposalStatus {
    /// Active voting period
    Voting,
    /// Voting ended, awaiting quorum check
    Tallying,
    /// Passed quorum and threshold
    Succeeded,
    /// Timelock period before execution
    Enacting,
    /// Failed quorum or threshold
    Rejected,
    /// Successfully applied
    Enacted,
    /// Execution error
    Failed,
}

impl adic_app_common::LifecycleState for ProposalStatus {
    fn is_terminal(&self) -> bool {
        matches!(self, Self::Enacted | Self::Failed | Self::Rejected)
    }

    fn can_transition_to(&self, next: &Self) -> bool {
        use ProposalStatus::*;
        match (self, next) {
            // From Voting
            (Voting, Tallying) => true,

            // From Tallying
            (Tallying, Succeeded) => true,
            (Tallying, Rejected) => true,

            // From Succeeded
            (Succeeded, Enacting) => true,
            (Succeeded, Rejected) => true, // Can be rejected if conditions fail

            // From Enacting
            (Enacting, Enacted) => true,
            (Enacting, Failed) => true,

            // Terminal states cannot transition
            (Enacted, _) | (Failed, _) | (Rejected, _) => false,

            // All other transitions are invalid
            _ => false,
        }
    }
}

#[cfg(test)]
mod proposal_lifecycle_tests {
    use super::*;
    use adic_app_common::LifecycleState;

    #[test]
    fn test_proposal_terminal_states() {
        assert!(ProposalStatus::Enacted.is_terminal());
        assert!(ProposalStatus::Failed.is_terminal());
        assert!(ProposalStatus::Rejected.is_terminal());

        assert!(!ProposalStatus::Voting.is_terminal());
        assert!(!ProposalStatus::Tallying.is_terminal());
        assert!(!ProposalStatus::Succeeded.is_terminal());
        assert!(!ProposalStatus::Enacting.is_terminal());
    }

    #[test]
    fn test_proposal_valid_transitions() {
        // Voting → Tallying
        assert!(ProposalStatus::Voting.can_transition_to(&ProposalStatus::Tallying));

        // Tallying → Succeeded or Rejected
        assert!(ProposalStatus::Tallying.can_transition_to(&ProposalStatus::Succeeded));
        assert!(ProposalStatus::Tallying.can_transition_to(&ProposalStatus::Rejected));

        // Succeeded → Enacting or Rejected
        assert!(ProposalStatus::Succeeded.can_transition_to(&ProposalStatus::Enacting));
        assert!(ProposalStatus::Succeeded.can_transition_to(&ProposalStatus::Rejected));

        // Enacting → Enacted or Failed
        assert!(ProposalStatus::Enacting.can_transition_to(&ProposalStatus::Enacted));
        assert!(ProposalStatus::Enacting.can_transition_to(&ProposalStatus::Failed));
    }

    #[test]
    fn test_proposal_invalid_transitions() {
        // Cannot skip states
        assert!(!ProposalStatus::Voting.can_transition_to(&ProposalStatus::Succeeded));
        assert!(!ProposalStatus::Tallying.can_transition_to(&ProposalStatus::Enacted));

        // Cannot transition from terminal states
        assert!(!ProposalStatus::Enacted.can_transition_to(&ProposalStatus::Voting));
        assert!(!ProposalStatus::Failed.can_transition_to(&ProposalStatus::Enacting));
        assert!(!ProposalStatus::Rejected.can_transition_to(&ProposalStatus::Tallying));

        // Cannot go backwards
        assert!(!ProposalStatus::Tallying.can_transition_to(&ProposalStatus::Voting));
        assert!(!ProposalStatus::Enacting.can_transition_to(&ProposalStatus::Succeeded));
    }

    #[test]
    fn test_proposal_happy_path() {
        // Happy path: Voting → Tallying → Succeeded → Enacting → Enacted
        let voting = ProposalStatus::Voting;
        assert!(voting.can_transition_to(&ProposalStatus::Tallying));

        let tallying = ProposalStatus::Tallying;
        assert!(tallying.can_transition_to(&ProposalStatus::Succeeded));

        let succeeded = ProposalStatus::Succeeded;
        assert!(succeeded.can_transition_to(&ProposalStatus::Enacting));

        let enacting = ProposalStatus::Enacting;
        assert!(enacting.can_transition_to(&ProposalStatus::Enacted));

        let enacted = ProposalStatus::Enacted;
        assert!(enacted.is_terminal());
    }

    #[test]
    fn test_proposal_rejection_paths() {
        // Early rejection: Voting → Tallying → Rejected
        assert!(ProposalStatus::Tallying.can_transition_to(&ProposalStatus::Rejected));

        // Late rejection: Voting → Tallying → Succeeded → Rejected
        assert!(ProposalStatus::Succeeded.can_transition_to(&ProposalStatus::Rejected));

        // Execution failure: Voting → Tallying → Succeeded → Enacting → Failed
        assert!(ProposalStatus::Enacting.can_transition_to(&ProposalStatus::Failed));
    }
}

/// Governance vote
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceVote {
    pub proposal_id: Hash,
    pub voter_pk: PublicKey,
    /// Computed as √min{R(voter), Rmax}
    pub credits: f64,
    pub ballot: Ballot,
    pub timestamp: DateTime<Utc>,
}

/// Vote choice
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ballot {
    Yes,
    No,
    Abstain,
}

/// Governance receipt with quorum attestation
///
/// Per PoUW III §10.3: Includes BLS threshold signature from governance committee
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceReceipt {
    pub proposal_id: Hash,
    pub quorum_stats: QuorumStats,
    pub result: VoteResult,
    pub receipt_seq: u32,
    pub prev_receipt_hash: Hash,
    pub timestamp: DateTime<Utc>,
    /// BLS threshold signature (sig_Qk) from governance committee quorum
    /// Signed using DST ADIC-GOV-R-v1 per PoUW III §10.3
    pub quorum_signature: Option<BLSSignature>,
    /// Public keys of committee members who signed
    pub committee_members: Vec<PublicKey>,
}

/// Vote tallying statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuorumStats {
    pub yes: f64,
    pub no: f64,
    pub abstain: f64,
    pub total_participation: f64,
}

/// Final vote result
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoteResult {
    Pass,
    Fail,
}

/// Axis catalog modification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxisCatalogChange {
    pub action: AxisAction,
    pub axis_id: u8,
    pub encoder_spec_cid: String,
    pub ultrametric_proof_cid: String,
    pub security_analysis_cid: String,
    pub migration_plan: MigrationPlan,
}

/// Axis catalog action
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisAction {
    Add,
    Deprecate,
    Modify,
}

/// Migration ramp schedule for axis changes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub ramp_epochs: u64,
    /// (epoch, weight) pairs
    pub weight_schedule: Vec<(u64, f64)>,
}

/// Treasury grant configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreasuryGrant {
    pub recipient: PublicKey,
    pub total_amount: AdicAmount,
    pub schedule: GrantSchedule,
    pub proposal_id: Hash,
}

/// Grant disbursement schedule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GrantSchedule {
    /// Single immediate payment
    Atomic,
    /// Regular payments over time
    Streamed {
        rate_per_epoch: AdicAmount,
        duration_epochs: u64,
    },
    /// Milestone-gated payments
    Milestone {
        milestones: Vec<Milestone>,
    },
}

/// Milestone requirement for grant tranches
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub id: u32,
    pub amount: AdicAmount,
    pub deliverable_cid: String,
    pub deadline_epoch: u64,
    pub verification_scheme: VerificationScheme,
}

/// How milestone completion is verified
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerificationScheme {
    /// Quorum review with BLS threshold signature
    QuorumAttestation { threshold: f64 },
    /// Automated check against criteria
    AutomatedCheck { criteria: String },
    /// External oracle verification
    OracleVerification { oracle_id: String },
    /// PoUW task execution verification
    /// Milestone is verified by successful completion of a PoUW task
    PoUWTaskVerification {
        /// Task ID that must be completed
        task_id: Hash,
        /// Minimum quality score required (0.0 - 1.0)
        min_quality: f64,
    },
}

/// Milestone attestation with cryptographic proof
///
/// Provides verifiable evidence that a milestone has been completed.
/// Different verification schemes produce different proof structures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilestoneAttestation {
    pub milestone_id: u32,
    pub grant_id: Hash,
    pub deliverable_cid: String,
    pub proof: AttestationProof,
    pub timestamp: DateTime<Utc>,
}

/// Cryptographic proof of milestone completion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AttestationProof {
    /// Quorum attestation with BLS threshold signature
    QuorumSignature {
        /// BLS threshold signature from committee
        signature: BLSSignature,
        /// Committee members who participated
        signers: Vec<PublicKey>,
        /// Total committee size
        quorum_size: usize,
        /// Message that was signed (typically hash of milestone details)
        signed_message: Vec<u8>,
    },
    /// Automated check result with evidence
    AutomatedCheck {
        /// Check result (passed/failed)
        passed: bool,
        /// IPFS CID of evidence/logs
        evidence_cid: String,
        /// Optional signature from automation service
        service_signature: Option<Vec<u8>>,
    },
    /// Oracle verification result
    OracleAttestation {
        /// Oracle identifier
        oracle_id: String,
        /// Verification result
        result: bool,
        /// Oracle's cryptographic signature
        oracle_signature: Vec<u8>,
        /// Optional additional data from oracle
        oracle_data: Option<String>,
    },
    /// PoUW task completion proof
    PoUWTaskCompletion {
        /// Task ID that was completed
        task_id: Hash,
        /// Task status
        completed: bool,
        /// Quality score achieved
        quality_score: f64,
        /// PoUW receipt ID proving completion
        receipt_id: Hash,
        /// BLS signature from PoUW quorum
        receipt_signature: Option<BLSSignature>,
    },
}

impl GovernanceProposal {
    /// Compute proposal hash (content-addressed ID)
    ///
    /// Uses canonical JSON for deterministic hashing across all nodes
    pub fn compute_id(
        proposer: &PublicKey,
        param_keys: &[String],
        new_values: &serde_json::Value,
        creation_timestamp: DateTime<Utc>,
    ) -> Hash {
        use adic_types::canonical_hash;
        use serde::Serialize;

        #[derive(Serialize)]
        struct CanonicalProposal<'a> {
            proposer: &'a PublicKey,
            param_keys: &'a [String],
            new_values: &'a serde_json::Value,
            creation_timestamp: i64,
        }

        let canonical = CanonicalProposal {
            proposer,
            param_keys,
            new_values,
            creation_timestamp: creation_timestamp.timestamp(),
        };

        let start = std::time::Instant::now();
        let hash = canonical_hash(&canonical).expect("Failed to compute canonical hash");
        let elapsed = start.elapsed();

        // Metrics
        crate::metrics::PROPOSAL_ID_COMPUTATIONS.inc();
        crate::metrics::CANONICAL_HASH_COMPUTATION_TIME.observe(elapsed.as_secs_f64());

        hash
    }

    /// Check if proposal requires supermajority
    pub fn requires_supermajority(&self) -> bool {
        matches!(self.class, ProposalClass::Constitutional)
    }

    /// Get required threshold for passage
    pub fn required_threshold(&self) -> f64 {
        match self.class {
            ProposalClass::Constitutional => 0.667, // ≥66.7%
            ProposalClass::Operational => 0.5,      // >50%
        }
    }

    /// Check if voting period has ended
    pub fn voting_ended(&self, current_time: DateTime<Utc>) -> bool {
        current_time >= self.voting_end_timestamp
    }

    /// Calculate vote percentage (yes / (yes + no))
    pub fn vote_percentage(&self) -> Option<f64> {
        let total_directional = self.tally_yes + self.tally_no;
        if total_directional > 0.0 {
            Some(self.tally_yes / total_directional)
        } else {
            None
        }
    }

    /// Check if proposal has passed threshold
    pub fn passed_threshold(&self) -> bool {
        if let Some(pct) = self.vote_percentage() {
            match self.class {
                ProposalClass::Constitutional => pct >= 0.667,
                ProposalClass::Operational => pct > 0.5,
            }
        } else {
            false
        }
    }
}

impl GovernanceVote {
    /// Create new vote
    pub fn new(
        proposal_id: Hash,
        voter_pk: PublicKey,
        credits: f64,
        ballot: Ballot,
    ) -> Self {
        Self {
            proposal_id,
            voter_pk,
            credits,
            ballot,
            timestamp: Utc::now(),
        }
    }
}

impl QuorumStats {
    /// Calculate total participation (yes + no + abstain)
    pub fn total(&self) -> f64 {
        self.yes + self.no + self.abstain
    }

    /// Calculate quorum percentage (total / total eligible voters)
    /// Note: This requires knowing total eligible voters, which should be tracked separately
    pub fn participation_rate(&self, total_eligible_credits: f64) -> f64 {
        if total_eligible_credits > 0.0 {
            self.total() / total_eligible_credits
        } else {
            0.0
        }
    }
}
