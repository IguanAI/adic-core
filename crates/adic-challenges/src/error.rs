use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChallengeError {
    #[error("Challenge window expired for {id}")]
    WindowExpired { id: String },

    #[error("Challenge window not found: {0}")]
    WindowNotFound(String),

    #[error("Fraud proof verification failed: {0}")]
    VerificationFailed(String),

    #[error("Invalid fraud proof: {0}")]
    InvalidProof(String),

    #[error("Quorum error: {0}")]
    QuorumError(#[from] adic_quorum::QuorumError),

    #[error("Insufficient evidence: {0}")]
    InsufficientEvidence(String),

    #[error("Duplicate challenge from {0}")]
    DuplicateChallenge(String),

    #[error("Challenge not in active state: {0}")]
    NotActive(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ChallengeError>;
