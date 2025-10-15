use thiserror::Error;

/// Storage market error types
#[derive(Error, Debug, Clone)]
pub enum StorageMarketError {
    /// Invalid message format or structure
    #[error("Invalid message: {0}")]
    InvalidMessage(String),

    /// Signature verification failed
    #[error("Invalid signature: {0}")]
    InvalidSignature(String),

    /// Admissibility check failed (C1-C3 violations)
    #[error("Admissibility violation: {0}")]
    AdmissibilityViolation(String),

    /// Finality requirements not met
    #[error("Finality not reached: {0}")]
    FinalityNotReached(String),

    /// Intent expired
    #[error("Intent expired at {0}")]
    IntentExpired(i64),

    /// Insufficient funds for escrow/collateral
    #[error("Insufficient funds: required {required}, available {available}")]
    InsufficientFunds { required: String, available: String },

    /// Reputation below minimum threshold
    #[error("Insufficient reputation: required {required}, actual {actual}")]
    InsufficientReputation { required: f64, actual: f64 },

    /// Provider diversity requirements not met
    #[error("Provider diversity violation: {0}")]
    DiversityViolation(String),

    /// Activation deadline exceeded
    #[error("Activation deadline exceeded: deadline {deadline}, current {current}")]
    ActivationDeadlineExceeded { deadline: u64, current: u64 },

    /// Proof verification failed
    #[error("Proof verification failed: {0}")]
    ProofVerificationFailed(String),

    /// Challenge response missing or invalid
    #[error("Invalid challenge response: {0}")]
    InvalidChallengeResponse(String),

    /// Deal not found
    #[error("Deal not found: {0}")]
    DealNotFound(String),

    /// Invalid deal state transition
    #[error("Invalid state transition: from {from:?} to {to:?}")]
    InvalidStateTransition { from: String, to: String },

    /// Collateral requirements not met
    #[error("Insufficient collateral: required {required}, provided {provided}")]
    InsufficientCollateral { required: String, provided: String },

    /// Settlement rail not supported
    #[error("Unsupported settlement rail: {0}")]
    UnsupportedSettlementRail(String),

    /// Cross-chain settlement failed
    #[error("Settlement failed: {0}")]
    SettlementFailed(String),

    /// Economics/escrow error
    #[error("Economics error: {0}")]
    EconomicsError(String),

    /// VRF error
    #[error("VRF error: {0}")]
    VRFError(String),

    /// Quorum error
    #[error("Quorum error: {0}")]
    QuorumError(String),

    /// Challenge window error
    #[error("Challenge window error: {0}")]
    ChallengeError(String),

    /// Invalid dispute
    #[error("Invalid dispute: {0}")]
    InvalidDispute(String),

    /// Resource not found
    #[error("Not found: {0}")]
    NotFound(String),

    /// Other errors
    #[error("Storage market error: {0}")]
    Other(String),
}

/// Result type for storage market operations
pub type Result<T> = std::result::Result<T, StorageMarketError>;
