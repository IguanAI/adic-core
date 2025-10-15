use thiserror::Error;

/// Governance operation result type
pub type Result<T> = std::result::Result<T, GovernanceError>;

/// Governance errors
#[derive(Debug, Error)]
pub enum GovernanceError {
    #[error("Insufficient reputation: required {required}, actual {actual}")]
    InsufficientReputation { required: f64, actual: f64 },

    #[error("Proposal not found: {0}")]
    ProposalNotFound(String),

    #[error("Vote not found for voter: {0}")]
    VoteNotFound(String),

    #[error("Duplicate vote from voter: {0}")]
    DuplicateVote(String),

    #[error("Voting period has not ended")]
    VotingNotEnded,

    #[error("Voting period has ended")]
    VotingEnded,

    #[error("Proposal in wrong status: expected {expected}, found {found}")]
    InvalidStatus { expected: String, found: String },

    #[error("Quorum not met: required {required}, actual {actual}")]
    QuorumNotMet { required: f64, actual: f64 },

    #[error("Threshold not met: required {required}, actual {actual}")]
    ThresholdNotMet { required: f64, actual: f64 },

    #[error("Invalid parameter key: {0}")]
    InvalidParameter(String),

    #[error("Parameter value out of range: {param} = {value} (range: {range})")]
    ParameterOutOfRange {
        param: String,
        value: String,
        range: String,
    },

    #[error("Parameter value validation failed: {param} - {reason}")]
    ParameterValidationFailed { param: String, reason: String },

    #[error("Conflicting proposal: {0}")]
    ConflictingProposal(String),

    #[error("Timelock not expired: {remaining_epochs} epochs remaining")]
    TimelockNotExpired { remaining_epochs: u64 },

    #[error("Finality requirement not met: {requirement}")]
    FinalityNotMet { requirement: String },

    #[error("Treasury error: {0}")]
    TreasuryError(String),

    #[error("Milestone verification failed: {milestone_id} - {reason}")]
    MilestoneVerificationFailed { milestone_id: u32, reason: String },

    #[error("Axis catalog error: {0}")]
    AxisCatalogError(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("App common error: {0}")]
    AppCommonError(#[from] adic_app_common::AppError),

    #[error("Quorum error: {0}")]
    QuorumError(#[from] adic_quorum::QuorumError),

    #[error("Challenge error: {0}")]
    ChallengeError(#[from] adic_challenges::ChallengeError),

    #[error("Finality timeout: {0}")]
    FinalityTimeout(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    #[error("Parameter update failed for {param}: {reason}")]
    ParameterUpdateFailed { param: String, reason: String },

    #[error("BLS coordinator required: {0}")]
    BLSCoordinatorRequired(&'static str),

    #[error("Other error: {0}")]
    Other(String),
}
