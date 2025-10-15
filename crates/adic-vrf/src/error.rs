use thiserror::Error;

#[derive(Error, Debug)]
pub enum VRFError {
    #[error("Insufficient reputation: required {required}, got {actual}")]
    InsufficientReputation { required: f64, actual: f64 },

    #[error("Commit not found for epoch {0}")]
    CommitNotFound(u64),

    #[error("Reveal verification failed: {0}")]
    RevealVerificationFailed(String),

    #[error("Epoch {0} randomness not finalized")]
    RandomnessNotFinalized(u64),

    #[error("Invalid VRF proof: {0}")]
    InvalidProof(String),

    #[error("Commitment mismatch: expected {expected}, got {actual}")]
    CommitmentMismatch { expected: String, actual: String },

    #[error("Reveal deadline passed for epoch {0}")]
    RevealDeadlinePassed(u64),

    #[error("Duplicate commit from {0} for epoch {1}")]
    DuplicateCommit(String, u64),

    #[error("No reveals for epoch {0}")]
    NoReveals(u64),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),
}

pub type Result<T> = std::result::Result<T, VRFError>;
