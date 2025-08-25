use thiserror::Error;

#[derive(Error, Debug)]
pub enum AdicError {
    #[error("Invalid message format: {0}")]
    InvalidMessage(String),

    #[error("Signature verification failed")]
    SignatureVerification,

    #[error("Admissibility check failed: {0}")]
    AdmissibilityFailed(String),

    #[error("Parent selection failed: {0}")]
    ParentSelectionFailed(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Finality check failed: {0}")]
    FinalityFailed(String),

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    #[error("Consensus violation: {0}")]
    ConsensusViolation(String),
}

impl From<serde_json::Error> for AdicError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialization(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AdicError>;