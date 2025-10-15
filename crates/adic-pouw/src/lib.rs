/*!
# ADIC PoUW (Proof of Useful Work) Framework

Distributed task execution framework for ADIC-DAG with verifiable computation,
VRF-based worker selection, quorum validation, and reputation-based incentives.

## Design Principles

- **Verifiable Computation**: All work results include cryptographic proofs
- **VRF Worker Selection**: Deterministic, unpredictable worker lottery
- **Quorum Validation**: Threshold BLS signatures for result verification
- **Reputation-Based**: Work quality affects ADIC-Rep scores
- **Economic Security**: Collateral requirements and slashing for fraud
- **Challenge Period**: Time window for disputes before finalization

## Module Structure

- **types**: Core data structures (Task, WorkResult, ValidationReport, etc.)
- **task_manager**: Task submission and lifecycle management
- **worker_selection**: VRF-based worker lottery with diversity
- **executor**: Task execution engine with resource limits
- **validator**: Quorum-based result validation
- **reputation**: Rep updates from work quality
- **rewards**: Payment distribution (atomic, vested, milestone)
- **collateral**: Escrow management and slashing
- **disputes**: Challenge resolution and arbitration
- **error**: PoUW-specific errors

## Integration with Foundation Layer

This framework builds on:
- **adic-vrf**: VRF service for worker selection randomness
- **adic-quorum**: Quorum selection and threshold BLS signatures
- **adic-challenges**: Challenge windows and fraud proof verification
- **adic-app-common**: Escrow management and lifecycle state machines
- **adic-governance**: Treasury execution for grant-funded tasks
- **adic-economics**: Balance management and reputation tracking

## Example Usage

```rust,ignore
use adic_pouw::{TaskManager, Task, TaskType, ComputationType, ResourceRequirements};
use adic_economics::types::AdicAmount;

// Create task manager
let task_manager = TaskManager::new(
    escrow_manager,
    reputation_tracker,
    TaskManagerConfig::default(),
);

// Submit a computation task
let task = Task {
    task_id: [0u8; 32],  // Generated from content hash
    sponsor: sponsor_pubkey,
    task_type: TaskType::Compute {
        computation_type: ComputationType::HashVerification,
        resource_requirements: ResourceRequirements::default(),
    },
    input_cid: "QmXyz123...".to_string(),
    expected_output_schema: Some("hash:sha256".to_string()),
    reward: AdicAmount::from_adic(100.0),
    collateral_requirement: AdicAmount::from_adic(50.0),
    deadline_epoch: current_epoch + 10,
    min_reputation: 1000.0,
    worker_count: 3,  // Triple redundancy
    // ... other fields
};

// Submit task (escrows reward + anti-spam deposit)
let task_id = task_manager.submit_task(task, current_epoch).await?;

// VRF worker selection happens automatically when task is finalized

// Worker executes and submits result
let result = task_manager.submit_work_result(work_result).await?;

// Quorum validates result
let validation = task_manager.validate_result(result_id, epoch).await?;

// Challenge period for disputes
// ... (24-hour window for fraud proofs)

// Finalize task (distribute rewards, update reputation)
let receipt = task_manager.finalize_task(task_id, epoch).await?;
```

## Task Lifecycle

```text
TaskSubmission
    ↓ (escrow reward)
Finalized
    ↓ (VRF selection)
Assigned
    ↓ (workers execute)
ResultSubmitted
    ↓ (quorum validation)
Validating
    ↓ (challenge period)
Challenging
    ↓ (no valid disputes)
Completed
    ↓ (distribute rewards, update rep)
TaskReceipt
```

## Reputation Updates

Per PoUW III §4.3:

```text
Accepted work: R' = R * (1 + α * quality_score)
Rejected work: R' = R * (1 - β * severity)
Per-epoch cap: ΔR_max per epoch
```

Where:
- α = multiplicative bonus factor (e.g., 0.1 for 10% max increase)
- β = multiplicative penalty factor (e.g., 0.2 for 20% decrease)
- quality_score ∈ [0, 1]: speed, correctness, challenge resistance
- severity ∈ [0, 1]: impact of failure

## Security Model

- **Collateral Requirements**: Workers escrow collateral (function of task value)
- **Fraud Detection**: Challenge period + fraud proofs
- **Slashing Conditions**:
  - Incorrect result verified by quorum
  - Missed deadline without valid excuse
  - Proven fraud attempt
- **Dispute Resolution**: High-reputation arbitration committee
- **Appeal Mechanism**: Governance proposal for complex cases

## Reference

- **ADIC PoUW I**: Sponsor Hooks and Epoch Quorums for Outsourced Distributed Computing
- **ADIC PoUW III**: Governance, Parameterization, Treasury Execution, and Conflict Resolution
- **Yellow Paper**: ADIC-Rep, MRW, Dual Finality
*/

pub mod bls_coordinator;
pub mod committee;
pub mod dkg_manager;
pub mod dkg_orchestrator;
pub mod disputes;
pub mod error;
pub mod executor;
pub mod input_storage;
pub mod receipt_validator;
pub mod reputation;
pub mod rewards;
pub mod task_manager;
pub mod task_storage;
pub mod types;
pub mod validator;
pub mod worker_selection;

pub use bls_coordinator::{
    BLSCoordinator, BLSCoordinatorConfig, SigningRequest, SigningRequestId, SigningStatus,
    DST_COMMITTEE_CERT, DST_TASK_RECEIPT,
};
pub use committee::{AggregatorSelector, CommitteeCertConfig, CommitteeCertGenerator};
pub use dkg_manager::DKGManager;
pub use dkg_orchestrator::{DKGOrchestrator, DKGOrchestratorConfig};
pub use disputes::{ArbitrationDecision, DisputeConfig, DisputeManager, DisputeOutcome, DisputeStats};
pub use error::{PoUWError, Result};
pub use executor::{ExecutorConfig, ExecutionStats, TaskExecutor};
pub use receipt_validator::{
    ReceiptValidationResult, ReceiptValidator, ReExecutionEvidence, ValidatedReceipt,
    ValidationStatus, WorkResultFraudProof,
};
pub use reputation::{EpochReputationStats, PoUWReputationManager, ReputationConfig};
pub use rewards::{RewardConfig, RewardManager, RewardStats};
pub use task_manager::{TaskManager, TaskManagerConfig};
pub use types::*;
pub use validator::{
    ResultValidator, ValidationStats, ValidationVote, ValidatorConfig, ZKProofVerifier,
};

// Only export PlaceholderZKVerifier in tests or when dev-placeholders feature is enabled
#[cfg(any(test, feature = "dev-placeholders"))]
pub use validator::PlaceholderZKVerifier;
pub use worker_selection::{
    EligibleWorker, WorkerPerformance, WorkerSelectionConfig, WorkerSelector,
};
