use thiserror::Error;

pub type Result<T> = std::result::Result<T, PoUWError>;

#[derive(Debug, Error)]
pub enum PoUWError {
    #[error("Task not found: {0}")]
    TaskNotFound(String),

    #[error("Worker not eligible: {reason}")]
    WorkerNotEligible { reason: String },

    #[error("Insufficient reputation: required {required}, actual {actual}")]
    InsufficientReputation { required: f64, actual: f64 },

    #[error("Insufficient collateral: required {required}, provided {provided}")]
    InsufficientCollateral {
        required: String,
        provided: String,
    },

    #[error("Task deadline exceeded: deadline {deadline}, current {current}")]
    DeadlineExceeded { deadline: u64, current: u64 },

    #[error("Invalid task state: expected {expected}, got {actual}")]
    InvalidTaskState { expected: String, actual: String },

    #[error("Worker already assigned to task: {task_id}")]
    WorkerAlreadyAssigned { task_id: String },

    #[error("Validation failed: {reason}")]
    ValidationFailed { reason: String },

    #[error("Quorum not reached: required {required}, got {actual}")]
    QuorumNotReached { required: usize, actual: usize },

    #[error("Fraud detected: {details}")]
    FraudDetected { details: String },

    #[error("Challenge period active: expires at epoch {expiry}")]
    ChallengePeriodActive { expiry: u64 },

    #[error("Result already submitted for task: {task_id}")]
    ResultAlreadySubmitted { task_id: String },

    #[error("Invalid execution proof: {reason}")]
    InvalidExecutionProof { reason: String },

    #[error("Resource limit exceeded: {resource} exceeded {limit}")]
    ResourceLimitExceeded { resource: String, limit: String },

    #[error("Escrow error: {0}")]
    EscrowError(String),

    #[error("VRF error: {0}")]
    VRFError(String),

    #[error("Quorum error: {0}")]
    QuorumError(String),

    #[error("Challenge error: {0}")]
    ChallengeError(String),

    #[error("Reputation error: {0}")]
    ReputationError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Invalid task input: {0}")]
    InvalidTaskInput(String),

    #[error("App common error: {0}")]
    AppCommonError(#[from] adic_app_common::AppError),

    #[error("VRF service error: {0}")]
    VRFServiceError(#[from] adic_vrf::VRFError),

    #[error("Quorum service error: {0}")]
    QuorumServiceError(#[from] adic_quorum::QuorumError),

    #[error("Challenge service error: {0}")]
    ChallengeServiceError(#[from] adic_challenges::ChallengeError),

    // Storage errors (H5)
    #[error("Storage error: {0}")]
    StorageError(#[from] adic_storage::StorageError),

    #[error("Task not found in storage: {0}")]
    TaskNotFoundInStorage(String),

    #[error("Input data not found in storage: {0}")]
    InputNotFound(String),

    #[error("Storage not configured: {0}")]
    StorageNotConfigured(String),

    #[error("Re-execution required in production: {0}")]
    ReExecutionRequired(String),

    // BLS Coordinator errors
    #[error("Insufficient committee size: required {required}, got {actual}")]
    InsufficientCommitteeSize { required: usize, actual: usize },

    #[error("Signing request not found: {0}")]
    SigningRequestNotFound(String),

    #[error("Signing session already completed: {0}")]
    SigningSessionCompleted(String),

    #[error("Signing deadline expired: {0}")]
    SigningDeadlineExpired(String),

    #[error("Unauthorized signer: {signer} not in committee for request {request_id}")]
    UnauthorizedSigner { signer: String, request_id: String },

    #[error("BLS cryptography error: {0}")]
    BLSError(#[from] adic_crypto::BLSError),

    #[error("Unsupported task type: {0}")]
    UnsupportedTaskType(String),

    #[error("Other error: {0}")]
    Other(String),
}
