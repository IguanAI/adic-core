use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Invalid state transition: {0}")]
    InvalidTransition(String),

    #[error("Finality requirement not met: {0}")]
    FinalityNotMet(String),

    #[error("Finality check failed: {0}")]
    FinalityCheckFailed(String),

    #[error("Finality timeout: {0}")]
    FinalityTimeout(String),

    #[error("Insufficient reputation: required {required}, got {actual}")]
    InsufficientReputation { required: f64, actual: f64 },

    #[error("Escrow operation failed: {0}")]
    EscrowError(String),

    #[error("Lock not found: {0}")]
    LockNotFound(String),

    #[error("Insufficient balance: needed {needed}, available {available}")]
    InsufficientBalance { needed: String, available: String },

    #[error("Invalid message type: {0}")]
    InvalidMessageType(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Economics error: {0}")]
    EconomicsError(#[from] anyhow::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error("Parameter not found: {0}")]
    ParameterNotFound(String),

    #[error("Parameter validation failed for {param}: {reason}")]
    ParameterValidationFailed { param: String, reason: String },
}

pub type Result<T> = std::result::Result<T, AppError>;
