//! Typed message content for ADIC messages
//!
//! This module defines the typed content that can be embedded in the `data` field
//! of an `AdicMessage`. All content types are serialized to/from the raw bytes.

use serde::{Deserialize, Serialize};

/// Typed content that can be embedded in an ADIC message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum MessageContent {
    /// Generic data payload (for application-specific use)
    Generic { data: Vec<u8> },

    /// Governance proposal (PoUW III §3)
    #[cfg(feature = "governance")]
    GovernanceProposal(Box<GovernanceProposalContent>),

    /// Governance vote (PoUW III §3)
    #[cfg(feature = "governance")]
    GovernanceVote(Box<GovernanceVoteContent>),

    /// Governance receipt (PoUW III §3)
    #[cfg(feature = "governance")]
    GovernanceReceipt(Box<GovernanceReceiptContent>),

    /// PoUW task submission (PoUW I §4)
    #[cfg(feature = "pouw")]
    PoUWTask(Box<PoUWTaskContent>),

    /// PoUW work result (PoUW I §5)
    #[cfg(feature = "pouw")]
    PoUWResult(Box<PoUWResultContent>),

    /// PoUW receipt (PoUW II §4)
    #[cfg(feature = "pouw")]
    PoUWReceipt(Box<PoUWReceiptContent>),
}

impl MessageContent {
    /// Serialize content to bytes for embedding in AdicMessage.data
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Deserialize content from AdicMessage.data bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(data)
    }

    /// Check if content is a governance message
    #[cfg(feature = "governance")]
    pub fn is_governance(&self) -> bool {
        matches!(
            self,
            MessageContent::GovernanceProposal(_)
                | MessageContent::GovernanceVote(_)
                | MessageContent::GovernanceReceipt(_)
        )
    }

    /// Check if content is a PoUW message
    #[cfg(feature = "pouw")]
    pub fn is_pouw(&self) -> bool {
        matches!(
            self,
            MessageContent::PoUWTask(_)
                | MessageContent::PoUWResult(_)
                | MessageContent::PoUWReceipt(_)
        )
    }
}

// Re-export governance types when feature is enabled
#[cfg(feature = "governance")]
pub use governance_content::*;

#[cfg(feature = "governance")]
mod governance_content {
    use super::*;
    use crate::PublicKey;

    /// Governance proposal content (PoUW III §3.1)
    ///
    /// This is embedded in AdicMessage.data as serialized MessageContent
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct GovernanceProposalContent {
        /// Unique proposal identifier (hash of proposal data)
        pub proposal_id: [u8; 32],

        /// Proposal class determines voting thresholds
        pub class: ProposalClass,

        /// Parameter keys being modified (e.g., ["rho", "q", "Delta"])
        pub param_keys: Vec<String>,

        /// New values (JSON for flexibility)
        pub new_values: serde_json::Value,

        /// Optional axis catalog changes
        pub axis_changes: Option<AxisChanges>,

        /// Epoch when changes take effect (after timelock)
        pub enact_epoch: u64,

        /// IPFS CID of rationale document
        pub rationale_cid: String,

        /// Submitter's public key
        pub submitter_pk: PublicKey,
    }

    /// Governance vote content (PoUW III §3.2)
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct GovernanceVoteContent {
        /// Proposal being voted on
        pub proposal_id: [u8; 32],

        /// Voter's public key
        pub voter_pk: PublicKey,

        /// Voting credits (computed from ADIC-Rep)
        /// w(P) = sqrt(min(R(P), R_max))
        pub credits: f64,

        /// Ballot choice
        pub ballot: Ballot,

        /// Epoch when vote was cast
        pub epoch: u64,
    }

    /// Governance receipt content (PoUW III §3.3)
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct GovernanceReceiptContent {
        /// Proposal this receipt is for
        pub proposal_id: [u8; 32],

        /// Quorum statistics
        pub quorum_stats: QuorumStats,

        /// Vote result
        pub result: VoteResult,

        /// Receipt sequence number
        pub receipt_seq: u32,

        /// Previous receipt hash (for chaining)
        pub prev_receipt_hash: [u8; 32],

        /// BLS threshold signature from committee
        pub threshold_bls_sig: Vec<u8>,

        /// Epoch when finalized
        pub finalized_epoch: u64,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ProposalClass {
        /// Core protocol parameters (requires supermajority)
        Constitutional,
        /// Operational parameters (simple majority)
        Operational,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AxisChanges {
        pub action: AxisAction,
        pub axis_id: String,
        pub axis_spec: Option<AxisSpec>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum AxisAction {
        Add,
        Modify,
        Remove,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AxisSpec {
        pub name: String,
        pub encoder_type: String,
        pub config: serde_json::Value,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Ballot {
        Yes,
        No,
        Abstain,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct QuorumStats {
        pub total_eligible_credits: f64,
        pub credits_voted: f64,
        pub credits_yes: f64,
        pub credits_no: f64,
        pub credits_abstain: f64,
        pub participation_rate: f64,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum VoteResult {
        Pass,
        Fail,
        Inconclusive,
    }
}

// Re-export PoUW types when feature is enabled
#[cfg(feature = "pouw")]
pub use pouw_content::*;

#[cfg(feature = "pouw")]
mod pouw_content {
    use super::*;
    use crate::PublicKey;

    /// PoUW task submission content (PoUW I §4)
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PoUWTaskContent {
        pub task_id: [u8; 32],
        pub sponsor: PublicKey,
        pub task_type: String, // "compute" | "storage" | "bandwidth"
        pub input_cid: String,
        pub expected_output_schema: Option<String>,
        pub reward_amount: u64,
        pub collateral_requirement: u64,
        pub deadline_epoch: u64,
        pub min_reputation: f64,
        pub worker_count: u8,
    }

    /// PoUW work result content (PoUW I §5)
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PoUWResultContent {
        pub result_id: [u8; 32],
        pub task_id: [u8; 32],
        pub worker: PublicKey,
        pub output_cid: String,
        pub execution_proof_cid: String,
        pub execution_metrics: ExecutionMetrics,
    }

    /// PoUW receipt content (PoUW II §4)
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PoUWReceiptContent {
        pub hook_id: [u8; 32],
        pub epoch_id: u64,
        pub receipt_seq: u32,
        pub prev_receipt_hash: [u8; 32],
        pub accepted_results: Vec<[u8; 32]>, // result_ids
        pub rejected_results: Vec<[u8; 32]>,
        pub threshold_bls_sig: Vec<u8>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ExecutionMetrics {
        pub cpu_time_ms: u64,
        pub memory_used_mb: u64,
        pub storage_used_mb: u64,
        pub network_used_kb: u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generic_content_roundtrip() {
        let content = MessageContent::Generic {
            data: vec![1, 2, 3, 4, 5],
        };

        let bytes = content.to_bytes().unwrap();
        let decoded = MessageContent::from_bytes(&bytes).unwrap();

        match decoded {
            MessageContent::Generic { data } => {
                assert_eq!(data, vec![1, 2, 3, 4, 5]);
            }
            _ => panic!("Wrong content type"),
        }
    }

    #[cfg(feature = "governance")]
    #[test]
    fn test_governance_proposal_content_roundtrip() {
        use crate::PublicKey;

        let content = MessageContent::GovernanceProposal(Box::new(GovernanceProposalContent {
            proposal_id: [1u8; 32],
            class: ProposalClass::Constitutional,
            param_keys: vec!["rho".to_string()],
            new_values: serde_json::json!({"rho": 0.6}),
            axis_changes: None,
            enact_epoch: 100,
            rationale_cid: "QmTest123".to_string(),
            submitter_pk: PublicKey::from_bytes([2u8; 32]),
        }));

        let bytes = content.to_bytes().unwrap();
        let decoded = MessageContent::from_bytes(&bytes).unwrap();

        match decoded {
            MessageContent::GovernanceProposal(proposal) => {
                assert_eq!(proposal.proposal_id, [1u8; 32]);
                assert_eq!(proposal.class, ProposalClass::Constitutional);
                assert_eq!(proposal.param_keys, vec!["rho".to_string()]);
            }
            _ => panic!("Wrong content type"),
        }
    }

    #[cfg(feature = "governance")]
    #[test]
    fn test_governance_vote_content_roundtrip() {
        use crate::PublicKey;

        let content = MessageContent::GovernanceVote(Box::new(GovernanceVoteContent {
            proposal_id: [1u8; 32],
            voter_pk: PublicKey::from_bytes([3u8; 32]),
            credits: 10.0,
            ballot: Ballot::Yes,
            epoch: 50,
        }));

        let bytes = content.to_bytes().unwrap();
        let decoded = MessageContent::from_bytes(&bytes).unwrap();

        match decoded {
            MessageContent::GovernanceVote(vote) => {
                assert_eq!(vote.proposal_id, [1u8; 32]);
                assert_eq!(vote.ballot, Ballot::Yes);
                assert_eq!(vote.credits, 10.0);
            }
            _ => panic!("Wrong content type"),
        }
    }
}
