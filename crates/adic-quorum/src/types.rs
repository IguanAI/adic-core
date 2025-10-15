use adic_types::PublicKey;
use serde::{Deserialize, Serialize};

/// Quorum configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuorumConfig {
    /// Minimum reputation for quorum eligibility
    pub min_reputation: f64,

    /// Members selected per axis
    pub members_per_axis: usize,

    /// Total quorum size (after merging axes)
    pub total_size: usize,

    /// Max members from same ASN
    pub max_per_asn: usize,

    /// Max members from same region
    pub max_per_region: usize,

    /// Domain separator for VRF
    pub domain_separator: String,

    /// Number of axes
    pub num_axes: usize,
}

impl Default for QuorumConfig {
    fn default() -> Self {
        Self {
            min_reputation: 50.0,
            members_per_axis: 5,
            total_size: 15,
            max_per_asn: 2,
            max_per_region: 3,
            domain_separator: "quorum_selection".to_string(),
            num_axes: 3,
        }
    }
}

/// Quorum member with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuorumMember {
    pub public_key: PublicKey,
    pub reputation: f64,
    pub vrf_score: [u8; 32],
    pub axis_id: usize,
    pub asn: Option<u32>,
    pub region: Option<String>,
}

/// Result of quorum selection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuorumResult {
    pub members: Vec<QuorumMember>,
    pub total_size: usize,
    pub axes_coverage: Vec<usize>,
}

/// Trait for quorum votes
pub trait QuorumVote: Send + Sync {
    fn voter(&self) -> PublicKey;
    fn is_for(&self) -> bool;
    fn is_against(&self) -> bool;
    fn is_abstain(&self) -> bool;
}

/// Simple quorum vote implementation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleQuorumVote {
    pub voter: PublicKey,
    pub vote_type: VoteType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoteType {
    For,
    Against,
    Abstain,
}

impl QuorumVote for SimpleQuorumVote {
    fn voter(&self) -> PublicKey {
        self.voter
    }

    fn is_for(&self) -> bool {
        matches!(self.vote_type, VoteType::For)
    }

    fn is_against(&self) -> bool {
        matches!(self.vote_type, VoteType::Against)
    }

    fn is_abstain(&self) -> bool {
        matches!(self.vote_type, VoteType::Abstain)
    }
}
