use thiserror::Error;

#[derive(Error, Debug)]
pub enum QuorumError {
    #[error("Insufficient eligible nodes: need {needed}, found {available}")]
    InsufficientNodes { needed: usize, available: usize },

    #[error("Quorum threshold not met: {votes_for}/{total} < {threshold}")]
    ThresholdNotMet {
        votes_for: usize,
        total: usize,
        threshold: f64,
    },

    #[error("Invalid quorum configuration: {0}")]
    InvalidConfiguration(String),

    #[error("VRF error: {0}")]
    VRFError(#[from] adic_vrf::VRFError),

    #[error("Node not eligible: {0}")]
    NodeNotEligible(String),

    #[error("Diversity cap exceeded: {cap_type} limit {limit}, trying {actual}")]
    DiversityCapExceeded {
        cap_type: String,
        limit: usize,
        actual: usize,
    },

    #[error("Invalid vote: {0}")]
    InvalidVote(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, QuorumError>;
