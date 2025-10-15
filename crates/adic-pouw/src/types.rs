use adic_crypto::BLSSignature;
use adic_economics::types::AdicAmount;
use adic_types::PublicKey;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub type Hash = [u8; 32];
pub type TaskId = [u8; 32];

/// Types of tasks that can be executed in the PoUW framework
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskType {
    /// General computation task (hash verification, data processing)
    Compute {
        computation_type: ComputationType,
        resource_requirements: ResourceRequirements,
    },
    /// Validate other nodes' work
    Validation {
        target_task_id: TaskId,
        validation_criteria: String,
    },
    /// Store and retrieve data (preliminary for storage market)
    Storage {
        data_cid: String,
        duration_epochs: u64,
        redundancy: u8,
    },
    /// Aggregate results from multiple workers
    Aggregation {
        source_task_ids: Vec<TaskId>,
        aggregation_function: String,
    },
}

/// Types of computation that can be performed in PoUW tasks
///
/// # Security Notice for ModelInference
///
/// `ModelInference` is **EXPERIMENTAL** and gated behind the `ml-inference` feature flag.
/// The current implementation uses a placeholder and is NOT suitable for production.
///
/// To use in development/testing:
/// ```bash
/// cargo build --features ml-inference
/// ```
///
/// For production, integrate a real ML runtime:
/// - ONNX Runtime: https://onnxruntime.ai/
/// - TensorFlow Lite: https://www.tensorflow.org/lite
/// - Candle (Rust-native): https://github.com/huggingface/candle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComputationType {
    /// Hash-based computation and verification (production-ready)
    HashVerification,
    /// General data transformation and processing (production-ready)
    DataProcessing,
    /// Machine learning model inference (EXPERIMENTAL - requires ml-inference feature)
    ///
    /// ⚠️ WARNING: Not available in production builds without ml-inference feature.
    /// Current implementation is a placeholder that returns fake results.
    ModelInference,
    /// Custom computation defined by string identifier
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRequirements {
    pub max_cpu_ms: u64,
    pub max_memory_mb: u64,
    pub max_storage_mb: u64,
    pub max_network_kb: u64,
}

impl Default for ResourceRequirements {
    fn default() -> Self {
        Self {
            max_cpu_ms: 10_000,      // 10 seconds
            max_memory_mb: 512,       // 512 MB
            max_storage_mb: 1024,     // 1 GB
            max_network_kb: 10_240,   // 10 MB
        }
    }
}

/// Main task structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub task_id: TaskId,
    pub sponsor: PublicKey,
    pub task_type: TaskType,
    pub input_cid: String,  // Content-addressed input data
    pub expected_output_schema: Option<String>,
    pub reward: AdicAmount,
    pub collateral_requirement: AdicAmount,
    pub deadline_epoch: u64,
    pub min_reputation: f64,
    pub worker_count: u8,  // Number of workers for redundancy
    pub created_at: DateTime<Utc>,
    pub status: TaskStatus,
    pub finality_status: FinalityStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Submitted,       // Task submitted, awaiting finality
    Finalized,       // Task finalized, ready for worker selection
    Assigned,        // Workers selected and assigned
    InProgress,      // Workers executing
    ResultSubmitted, // Results submitted, awaiting validation
    Validating,      // Quorum validation in progress
    Challenging,     // In challenge period
    Completed,       // Successfully completed
    Failed,          // Failed validation or deadline
    Disputed,        // Under dispute resolution
    Slashed,         // Worker slashed for fraud
    Expired,         // Deadline passed without completion
}

impl adic_app_common::LifecycleState for TaskStatus {
    fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Slashed | Self::Expired)
    }

    fn can_transition_to(&self, next: &Self) -> bool {
        use TaskStatus::*;
        match (self, next) {
            // From Submitted
            (Submitted, Finalized) => true,
            (Submitted, Failed) => true, // Can fail if invalid
            (Submitted, Expired) => true, // Can expire while waiting for finality

            // From Finalized
            (Finalized, Assigned) => true,
            (Finalized, Expired) => true,

            // From Assigned
            (Assigned, InProgress) => true,
            (Assigned, Expired) => true,

            // From InProgress
            (InProgress, ResultSubmitted) => true,
            (InProgress, Failed) => true,
            (InProgress, Expired) => true,

            // From ResultSubmitted
            (ResultSubmitted, Validating) => true,
            (ResultSubmitted, Failed) => true,

            // From Validating
            (Validating, Challenging) => true,
            (Validating, Completed) => true, // Direct completion if no challenges
            (Validating, Failed) => true,

            // From Challenging
            (Challenging, Completed) => true,
            (Challenging, Disputed) => true,
            (Challenging, Failed) => true,

            // From Disputed
            (Disputed, Completed) => true, // Dispute resolved in favor
            (Disputed, Slashed) => true,   // Dispute resolved against worker
            (Disputed, Failed) => true,

            // Terminal states cannot transition
            (Completed, _) | (Failed, _) | (Slashed, _) | (Expired, _) => false,

            // All other transitions are invalid
            _ => false,
        }
    }
}

#[cfg(test)]
mod task_lifecycle_tests {
    use super::*;
    use adic_app_common::LifecycleState;

    #[test]
    fn test_task_terminal_states() {
        assert!(TaskStatus::Completed.is_terminal());
        assert!(TaskStatus::Failed.is_terminal());
        assert!(TaskStatus::Slashed.is_terminal());
        assert!(TaskStatus::Expired.is_terminal());

        assert!(!TaskStatus::Submitted.is_terminal());
        assert!(!TaskStatus::Finalized.is_terminal());
        assert!(!TaskStatus::Assigned.is_terminal());
        assert!(!TaskStatus::InProgress.is_terminal());
    }

    #[test]
    fn test_task_happy_path() {
        // Happy path: Submitted → Finalized → Assigned → InProgress → ResultSubmitted → Validating → Challenging → Completed
        assert!(TaskStatus::Submitted.can_transition_to(&TaskStatus::Finalized));
        assert!(TaskStatus::Finalized.can_transition_to(&TaskStatus::Assigned));
        assert!(TaskStatus::Assigned.can_transition_to(&TaskStatus::InProgress));
        assert!(TaskStatus::InProgress.can_transition_to(&TaskStatus::ResultSubmitted));
        assert!(TaskStatus::ResultSubmitted.can_transition_to(&TaskStatus::Validating));
        assert!(TaskStatus::Validating.can_transition_to(&TaskStatus::Challenging));
        assert!(TaskStatus::Challenging.can_transition_to(&TaskStatus::Completed));
        assert!(TaskStatus::Completed.is_terminal());
    }

    #[test]
    fn test_task_fast_completion() {
        // Fast path: Validating → Completed (no challenges)
        assert!(TaskStatus::Validating.can_transition_to(&TaskStatus::Completed));
    }

    #[test]
    fn test_task_expiration() {
        // Can expire from multiple states
        assert!(TaskStatus::Submitted.can_transition_to(&TaskStatus::Expired));
        assert!(TaskStatus::Finalized.can_transition_to(&TaskStatus::Expired));
        assert!(TaskStatus::Assigned.can_transition_to(&TaskStatus::Expired));
        assert!(TaskStatus::InProgress.can_transition_to(&TaskStatus::Expired));
    }

    #[test]
    fn test_task_failure_paths() {
        // Can fail from multiple states
        assert!(TaskStatus::Submitted.can_transition_to(&TaskStatus::Failed));
        assert!(TaskStatus::InProgress.can_transition_to(&TaskStatus::Failed));
        assert!(TaskStatus::ResultSubmitted.can_transition_to(&TaskStatus::Failed));
        assert!(TaskStatus::Validating.can_transition_to(&TaskStatus::Failed));
        assert!(TaskStatus::Challenging.can_transition_to(&TaskStatus::Failed));
    }

    #[test]
    fn test_task_dispute_resolution() {
        // Disputed → Completed or Slashed
        assert!(TaskStatus::Disputed.can_transition_to(&TaskStatus::Completed));
        assert!(TaskStatus::Disputed.can_transition_to(&TaskStatus::Slashed));
        assert!(TaskStatus::Disputed.can_transition_to(&TaskStatus::Failed));

        // Challenging → Disputed
        assert!(TaskStatus::Challenging.can_transition_to(&TaskStatus::Disputed));
    }

    #[test]
    fn test_task_invalid_transitions() {
        // Cannot skip states
        assert!(!TaskStatus::Submitted.can_transition_to(&TaskStatus::Assigned));
        assert!(!TaskStatus::Finalized.can_transition_to(&TaskStatus::InProgress));

        // Cannot transition from terminal states
        assert!(!TaskStatus::Completed.can_transition_to(&TaskStatus::Validating));
        assert!(!TaskStatus::Failed.can_transition_to(&TaskStatus::InProgress));
        assert!(!TaskStatus::Expired.can_transition_to(&TaskStatus::Assigned));
        assert!(!TaskStatus::Slashed.can_transition_to(&TaskStatus::Disputed));

        // Cannot go backwards
        assert!(!TaskStatus::Assigned.can_transition_to(&TaskStatus::Finalized));
        assert!(!TaskStatus::Validating.can_transition_to(&TaskStatus::InProgress));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinalityStatus {
    Pending,
    F1Complete,
    F2Complete,
}

/// Worker assignment after VRF selection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkAssignment {
    pub assignment_id: Hash,
    pub task_id: TaskId,
    pub worker: PublicKey,
    pub assigned_epoch: u64,
    pub vrf_proof: Vec<u8>,  // VRF proof of selection
    pub worker_index: u8,     // Index among selected workers
    pub collateral_lock_id: Option<String>,
    pub assignment_timestamp: DateTime<Utc>,
}

/// Worker's submitted result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkResult {
    pub result_id: Hash,
    pub task_id: TaskId,
    pub worker: PublicKey,
    pub output_cid: String,  // Content-addressed output data
    pub execution_proof: ExecutionProof,
    pub execution_metrics: ExecutionMetrics,
    pub worker_signature: Vec<u8>,
    pub submitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionProof {
    pub proof_type: ProofType,
    pub merkle_root: Hash,
    pub intermediate_states: Vec<Hash>,
    pub proof_data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProofType {
    MerkleTree,
    StateTransition,
    ZKProof,
    ReExecution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionMetrics {
    pub cpu_time_ms: u64,
    pub memory_used_mb: u64,
    pub storage_used_mb: u64,
    pub network_used_kb: u64,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
}

/// Validation report from quorum
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub report_id: Hash,
    pub task_id: TaskId,
    pub result_id: Hash,
    pub validators: Vec<PublicKey>,
    pub votes_for: usize,
    pub votes_against: usize,
    pub threshold_bls_sig: Vec<u8>,
    pub validation_result: ValidationResult,
    pub validation_evidence: Vec<ValidationEvidence>,
    pub finalized_at_epoch: u64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationResult {
    Accepted,
    Rejected,
    Inconclusive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationEvidence {
    pub validator: PublicKey,
    pub vote: bool,  // true = accept, false = reject
    pub re_execution_hash: Option<Hash>,
    pub discrepancy_details: Option<String>,
    pub signature: Vec<u8>,
}

/// Final task receipt with reputation updates
///
/// Per PoUW III §10.3: Includes BLS threshold signature from validation quorum
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskReceipt {
    pub receipt_id: Hash,
    pub task_id: TaskId,
    pub outcome: TaskOutcome,
    pub workers: Vec<PublicKey>,
    pub reputation_updates: Vec<ReputationUpdate>,
    pub rewards_distributed: Vec<RewardDistribution>,
    pub slashing_events: Vec<SlashingEvent>,
    pub finalized_at_epoch: u64,
    pub timestamp: DateTime<Utc>,
    /// BLS threshold signature from validation quorum
    /// Signed using DST ADIC-PoUW-RECEIPT-v1 per PoUW III §10.3
    pub quorum_signature: Vec<u8>,
    /// Public keys of quorum members who signed
    pub quorum_members: Vec<PublicKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskOutcome {
    Success,
    Failure,
    PartialSuccess,
    Slashed,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationUpdate {
    pub account: PublicKey,
    pub old_reputation: f64,
    pub new_reputation: f64,
    pub change_reason: ReputationChangeReason,
    pub quality_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReputationChangeReason {
    WorkAccepted { quality: f64 },
    WorkRejected { severity: f64 },
    DeadlineMissed,
    FraudDetected,
    ValidationPerformed { correctness: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardDistribution {
    pub recipient: PublicKey,
    pub amount: AdicAmount,
    pub reward_type: RewardType,
    pub vesting_schedule: Option<VestingSchedule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RewardType {
    BaseReward,
    QualityBonus,
    SpeedBonus,
    ValidationReward,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VestingSchedule {
    pub total_amount: AdicAmount,
    pub vested_per_epoch: AdicAmount,
    pub start_epoch: u64,
    pub duration_epochs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashingEvent {
    pub slashed_account: PublicKey,
    pub amount: AdicAmount,
    pub reason: SlashingReason,
    pub evidence_hash: Hash,
    pub beneficiary: Option<PublicKey>,  // Challenger or treasury
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SlashingReason {
    IncorrectResult,
    MissedDeadline,
    ProvenFraud,
    ResourceAbuse,
}

/// Dispute submission
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dispute {
    pub dispute_id: Hash,
    pub task_id: TaskId,
    pub result_id: Hash,
    pub worker: PublicKey,        // Worker being challenged
    pub challenger: PublicKey,     // Party submitting the challenge
    pub challenger_stake: AdicAmount,
    pub evidence: DisputeEvidence,
    pub status: DisputeStatus,
    pub created_at: DateTime<Utc>,
    pub resolution_deadline: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DisputeEvidence {
    ReExecutionProof {
        different_output_cid: String,
        execution_trace: Vec<Hash>,
    },
    InputOutputMismatch {
        expected_output: String,
        actual_output: String,
    },
    ResourceUsageFraud {
        claimed_metrics: ExecutionMetrics,
        actual_metrics: ExecutionMetrics,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisputeStatus {
    Submitted,
    UnderReview,
    Accepted,
    Rejected,
    Inconclusive,
    Appealed,
}

/// Task statistics for monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStats {
    pub total_tasks: u64,
    pub active_tasks: u64,
    pub completed_tasks: u64,
    pub failed_tasks: u64,
    pub total_rewards_distributed: AdicAmount,
    pub total_slashed: AdicAmount,
    pub average_execution_time_ms: f64,
    pub average_validation_time_ms: f64,
}

impl Default for TaskStats {
    fn default() -> Self {
        Self {
            total_tasks: 0,
            active_tasks: 0,
            completed_tasks: 0,
            failed_tasks: 0,
            total_rewards_distributed: AdicAmount::from_adic(0.0),
            total_slashed: AdicAmount::from_adic(0.0),
            average_execution_time_ms: 0.0,
            average_validation_time_ms: 0.0,
        }
    }
}

// ============================================================================
// Paper-Specified PoUW Hook Message Types (PoUW I §4.2, PoUW II Appendix C)
// ============================================================================

/// HookMessage: On-ledger sponsor hook registration (PoUW I §4.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookMessage {
    pub hook_id: Hash,
    pub epoch_selector: EpochSelector,
    pub budget: Budget,
    pub payment_schedule: PaymentSchedule,
    /// Content ID of off-ledger PoUWManifest
    pub manifest_hash: String,
    pub verify_scheme: VerifyScheme,
    pub inputs: Vec<InputCommitment>,
    pub sla: SLA,
    pub access: Access,
    pub payout_rail: PayoutRail,
    pub sponsor_pk: PublicKey,
    pub signature: Vec<u8>,
}

/// Epoch selection criteria for hook execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EpochSelector {
    /// Execute in specific epochs
    Specific(Vec<u64>),
    /// Execute every N epochs starting from epoch S
    Periodic { interval: u64, start_epoch: u64 },
    /// Execute once in epoch range [start, end]
    Range { start: u64, end: u64 },
}

/// Budget allocation for hook execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub total_amount: AdicAmount,
    pub per_chunk_reward: AdicAmount,
    pub validator_reward_percentage: f64,
}

/// Payment distribution schedule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PaymentSchedule {
    /// Pay immediately upon verification
    Immediate,
    /// Pay in fixed installments over time
    Vested {
        installments: u32,
        epochs_per_installment: u64,
    },
    /// Pay based on milestone completion
    Milestone { milestones: Vec<MilestonePayment> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilestonePayment {
    pub milestone_id: u32,
    pub amount: AdicAmount,
    pub completion_criteria: String,
}

/// Verification scheme for work validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerifyScheme {
    /// Replication-based: k-of-n agreement
    Replication { k: u32, n: u32 },
    /// Zero-knowledge proof verification
    ZKProof { proof_type: String },
    /// TEE attestation
    TEE { enclave_type: String },
    /// Combination of schemes
    Hybrid { schemes: Vec<Box<VerifyScheme>> },
}

/// Input commitment for deterministic execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputCommitment {
    pub input_id: String,
    pub commitment_hash: Hash,
    pub availability_proof: Option<String>,
}

/// Service Level Agreement parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SLA {
    pub max_execution_time_ms: u64,
    pub max_retries: u32,
    pub timeout_penalty: AdicAmount,
}

/// Access control for hook execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Access {
    /// Anyone can execute
    Public,
    /// Only whitelisted workers
    Whitelist { allowed_workers: Vec<PublicKey> },
    /// Requires minimum reputation
    ReputationGated { min_reputation: f64 },
}

/// Payout settlement rail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PayoutRail {
    /// Native ADIC ledger
    AdicNative,
    /// Ethereum ERC-20 (via JITCA)
    EthereumERC20 { token_address: String },
    /// Bitcoin PSBT
    BitcoinPSBT,
    /// Solana SPL token
    SolanaSPL { mint_address: String },
}

/// PoUWManifest: Off-ledger content-addressed program specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoUWManifest {
    /// WASM module bytecode
    pub program: Vec<u8>,
    pub inputs: Vec<InputSpec>,
    pub verifier: VerifierConfig,
    pub replication: ReplicationParams,
    pub challenge: ChallengeLogic,
    pub test_vectors: Vec<TestVector>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputSpec {
    pub name: String,
    pub type_schema: String,
    pub source: InputSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputSource {
    Inline { data: Vec<u8> },
    ContentAddressed { cid: String },
    ChainState { query: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifierConfig {
    pub verification_method: VerifyScheme,
    pub consensus_threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationParams {
    pub replication_factor: u32,
    pub chunk_assignment_strategy: ChunkStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChunkStrategy {
    Random,
    ProximityBased { axis: u32 },
    ReputationWeighted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeLogic {
    pub challenge_window_epochs: u64,
    pub challenge_stake: AdicAmount,
    pub adjudication_method: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestVector {
    pub input: Vec<u8>,
    pub expected_output: Vec<u8>,
}

/// ODCCommitteeCert: On-Demand Committee epoch certificate (PoUW II §2.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ODCCommitteeCert {
    pub epoch_id: u64,
    /// Epoch randomness from VRF mixing
    pub randomness: Hash,
    /// Committee members selected via VRF
    pub members: Vec<PublicKey>,
    /// Axis diversity statistics
    pub axis_stats: AxisStats,
    /// BLS threshold signature from committee
    /// Signed using DST ADIC-PoUW-COMMITTEE-v1
    pub threshold_signature: Option<BLSSignature>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxisStats {
    pub axis_distribution: Vec<(u32, u32)>, // (axis_id, member_count)
    pub diversity_score: f64,
}

pub type ChunkId = u64;

/// WorkClaim: Worker's claim to execute a chunk (PoUW II §3.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkClaim {
    pub hook_id: Hash,
    pub chunk_id: ChunkId,
    pub miner_pk: PublicKey,
    pub signature: Vec<u8>,
}

/// QuorumVerdict: ODC member's verdict on work result (PoUW II §4.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuorumVerdict {
    pub hook_id: Hash,
    pub chunk_id: ChunkId,
    pub member_pk: PublicKey,
    /// true = accept, false = reject
    pub verdict: bool,
    /// Hash of verification evidence
    pub evidence_hash: Hash,
    pub signature: Vec<u8>,
}

/// PoUWReceipt: Chained receipt with BLS aggregation (PoUW II §4.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoUWReceipt {
    pub version: u8,
    pub hook_id: Hash,
    pub epoch_id: u64,
    /// Receipt sequence number for chaining
    pub receipt_seq: u32,
    /// Hash of previous receipt (for chain integrity)
    pub prev_receipt_hash: Hash,
    pub accepted: Vec<ChunkAcceptance>,
    pub rejected: Vec<ChunkRejection>,
    /// Aggregated BLS public key from quorum
    pub agg_pk: Vec<u8>,
    /// BLS threshold signature (sig_Qk)
    /// Signed using DST ADIC-PoUW-RECEIPT-v1
    pub sig_qk: Option<BLSSignature>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkAcceptance {
    pub chunk_id: ChunkId,
    pub worker_pk: PublicKey,
    pub output_hash: Hash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRejection {
    pub chunk_id: ChunkId,
    pub worker_pk: PublicKey,
    pub rejection_reason: String,
}

/// PayoutOrder: Cross-chain settlement instruction (PoUW I §8)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayoutOrder {
    pub hook_id: Hash,
    pub recipient: PublicKey,
    pub amount: AdicAmount,
    pub rail: PayoutRail,
    /// Just-In-Time Cross-Chain Adapter reference
    pub jitca_ref: Option<String>,
}
