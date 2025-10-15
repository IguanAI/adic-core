# ADIC PoUW (Proof of Useful Work)

Distributed task execution framework for ADIC-DAG with verifiable computation, VRF-based worker selection, quorum validation, and reputation-based incentives.

## Overview

The ADIC PoUW Framework enables **feeless, verifiable distributed computing** where sponsors can outsource computational tasks to workers, who are selected via VRF lottery and validated by quorum consensus. All task execution is cryptographically verifiable, reputation-tracked, and economically secured through collateral requirements.

## Architecture

### Core Components

1. **Task Manager** (`src/task_manager.rs` - 700+ lines)
   - Task submission with escrow integration
   - Lifecycle management (Submitted → Completed)
   - Work assignment coordination
   - Result collection and validation orchestration
   - Challenge window management
   - Anti-spam deposits (refundable on finality)

2. **Worker Selection** (`src/worker_selection.rs` - 564 lines)
   - VRF-based deterministic worker lottery
   - Reputation-weighted selection probability
   - Performance tracking (speed, accuracy, availability)
   - Eligibility filtering (reputation, collateral, resource capacity)
   - Multi-axis diversity enforcement

3. **Task Executor** (`src/executor.rs` - 400+ lines)
   - Computation execution with resource limits (CPU, memory, storage, network)
   - Execution proof generation
   - Hash verification, data processing, model inference
   - Timeout enforcement
   - Performance metrics tracking

4. **Result Validator** (`src/validator.rs` - 477 lines)
   - Quorum-based result validation
   - Threshold BLS signature aggregation
   - Validation vote collection (Accept/Reject/Abstain)
   - Caching of validation results
   - Validator reputation updates

5. **Reward Manager** (`src/rewards.rs` - 493 lines)
   - Reward distribution (atomic, vested, milestone-gated)
   - Quality-based bonuses (correctness, speed)
   - Collateral slashing for fraud
   - Vesting schedules (linear unlock over time)
   - Reward statistics tracking

6. **Reputation Manager** (`src/reputation.rs` - 500+ lines)
   - ADIC-Rep updates from work quality
   - Multiplicative-weights algorithm: `R' = R * (1 + α * quality)`
   - Per-epoch reputation caps
   - Reputation floor (minimum non-zero value)
   - Validator reputation tracking
   - Quality score calculation (speed × correctness × challenge_resistance)

7. **Dispute Manager** (`src/disputes.rs` - 700 lines)
   - Challenge submission with evidence (IPFS/Arweave CIDs)
   - Arbitration committee selection (high-reputation nodes)
   - Fraud proof verification
   - Automatic and manual dispute resolution
   - Challenger rewards for valid disputes
   - Challenge statistics tracking

### Task Lifecycle

```
TaskSubmission
    ↓ (sponsor escrows reward + deposit)
Finalized (F1 + F2 finality)
    ↓ (VRF worker selection)
Assigned
    ↓ (workers execute)
ResultSubmitted
    ↓ (quorum validation)
Validating
    ↓ (threshold BLS signature)
Challenging (challenge window)
    ↓ (no valid disputes)
Completed
    ↓ (distribute rewards, update rep)
TaskReceipt
```

**Status Transitions**:
- `Submitted` → `Finalized`: After F1 + F2 finality
- `Finalized` → `Assigned`: After VRF worker selection
- `Assigned` → `InProgress`: Workers begin execution
- `InProgress` → `ResultSubmitted`: Workers submit results
- `ResultSubmitted` → `Validating`: Quorum begins validation
- `Validating` → `Challenging`: Validation complete, challenge window opens
- `Challenging` → `Completed`: No valid disputes, distribute rewards
- `Challenging` → `Disputed`: Valid dispute submitted
- `Disputed` → `Slashed` or `Completed`: Arbitration resolves

## Key Features

### VRF-Based Worker Selection

Workers are selected via **Verifiable Random Function** lottery, ensuring:
- Deterministic selection (same inputs → same workers)
- Unpredictability (sponsors cannot game selection)
- Reputation-weighted probability (higher Rep → higher chance)
- Multi-axis diversity (workers from different regions/ASNs)
- Collateral-aware (only workers with sufficient collateral)

**Selection Algorithm**:
```rust
// 1. Filter eligible workers (reputation, collateral, capacity)
let eligible = workers.filter(|w|
    w.reputation >= task.min_reputation &&
    w.available_collateral >= task.collateral_requirement &&
    w.meets_resource_requirements(&task)
);

// 2. Generate VRF proof for each worker
let vrf_seed = blake3::hash(&task_id);
for worker in eligible {
    let vrf_output = worker.vrf_service.compute(vrf_seed);
    let score = reputation_weight(worker.reputation) * vrf_output;
    candidates.push((worker, score));
}

// 3. Select top N workers by score
candidates.sort_by_score();
let selected = candidates.take(task.worker_count);
```

### Quorum Validation

Results are validated by a **threshold BLS quorum**:
- `m` validators randomly selected (e.g., 64)
- `t` threshold signatures required (e.g., ⌈2m/3⌉ = 43)
- Each validator votes: Accept, Reject, Abstain
- BLS signature aggregation proves threshold reached
- Validators' Rep updated based on agreement with majority

**Validation Logic**:
```rust
pub async fn validate_result(
    &self,
    result: &WorkResult,
    quorum_votes: Vec<ValidationVote>,
) -> Result<ValidationReport> {
    // Tally votes
    let accepts = votes.filter(|v| v.vote == VoteType::Accept).count();
    let rejects = votes.filter(|v| v.vote == VoteType::Reject).count();

    // Check threshold
    let threshold = (quorum_votes.len() * 2) / 3;
    let outcome = if accepts >= threshold {
        ValidationOutcome::Accepted
    } else if rejects >= threshold {
        ValidationOutcome::Rejected
    } else {
        ValidationOutcome::Inconclusive
    };

    // Aggregate BLS signatures
    let bls_signature = aggregate_signatures(&quorum_votes);

    Ok(ValidationReport {
        result_id: result.result_id,
        outcome,
        vote_counts: (accepts, rejects, abstains),
        quorum_signature: bls_signature,
        finality_status: FinalityStatus::Pending,
    })
}
```

### Reputation Updates

ADIC-Rep is updated via **multiplicative-weights algorithm** (PoUW III §4.3):

```
Accepted work: R' = R * (1 + α * quality_score)
Rejected work: R' = R * (1 - β * severity)
Per-epoch cap: ΔR ≤ ΔR_max
```

**Quality Score** (range [0, 1]):
```rust
quality = (speed_score + correctness_score + challenge_resistance) / 3.0

where:
- speed_score = 1.0 - (actual_time / deadline_time)
- correctness_score = 1.0 if validated, 0.0 if rejected
- challenge_resistance = 1.0 - (valid_challenges / total_challenges)
```

**Reputation Bounds**:
- Minimum: `Rfloor = 1.0` (prevents zero reputation)
- Maximum: Unlimited (but voting caps at `√Rmax` in governance)
- Per-epoch cap: `ΔR_max = 10.0` (prevents rapid farming)

**Validator Rep Updates**:
- Agree with majority: `R' = R * 1.01` (1% bonus)
- Disagree with majority: `R' = R * 0.99` (1% penalty)

### Reward Distribution

Three reward modes:

**1. Atomic** (immediate full payment):
```rust
let reward = RewardDistribution {
    recipient: worker_pubkey,
    amount: task.reward,
    vesting: None,
};
reward_manager.distribute(reward).await?;
```

**2. Vested** (linear unlock over time):
```rust
let reward = RewardDistribution {
    recipient: worker_pubkey,
    amount: task.reward,
    vesting: Some(VestingSchedule {
        start_epoch: current_epoch,
        duration_epochs: 100,  // Unlock over 100 epochs
        cliff_epochs: 10,      // 10 epoch cliff before vesting starts
    }),
};
```

**3. Milestone-Gated** (unlocks on deliverables):
```rust
let milestones = vec![
    RewardMilestone {
        milestone_id: 1,
        amount: AdicAmount::from_adic(40.0),
        verification_criteria: "Phase 1 deliverable",
    },
    RewardMilestone {
        milestone_id: 2,
        amount: AdicAmount::from_adic(60.0),
        verification_criteria: "Phase 2 deliverable",
    },
];
```

**Bonuses**:
- Quality bonus: `bonus = base_reward * (quality_score - 0.8) * quality_multiplier`
- Speed bonus: `bonus = base_reward * (1.0 - time_ratio) * speed_multiplier`
- Combined: `total_reward = base + quality_bonus + speed_bonus`

**Slashing**:
- Fraud proven: 100% collateral slashed
- Failed validation: 50% collateral slashed
- Missed deadline: 20% collateral slashed

### Dispute Resolution

**Challenge Types**:
- `IncorrectResult`: Worker's output is provably wrong
- `MissedDeadline`: Worker failed to submit by deadline
- `InsufficientQuality`: Result doesn't meet quality standards
- `Fraud`: Malicious behavior (fake proofs, Sybil attacks)

**Challenge Process**:
```rust
// 1. Submit challenge with evidence
let challenge = DisputeChallenge {
    task_id,
    challenger: challenger_pubkey,
    challenge_type: ChallengeType::IncorrectResult,
    evidence_cid: "Qm_fraud_proof",  // IPFS CID
    stake: AdicAmount::from_adic(10.0),  // Anti-spam stake
};
dispute_manager.submit_challenge(challenge).await?;

// 2. Automatic resolution (if deterministic)
let outcome = dispute_manager.resolve_automatic(challenge_id).await?;

// 3. Manual arbitration (if complex)
let committee = dispute_manager.select_arbitration_committee(challenge_id).await?;
let votes = committee.members.iter().map(|m| m.vote()).collect();
let decision = dispute_manager.resolve_via_arbitration(challenge_id, votes).await?;
```

**Challenger Rewards**:
- Valid challenge: 50% of slashed collateral
- Invalid challenge: Lose stake
- Frivolous challenge: Reputation penalty

**Arbitration Committee**:
- High-reputation nodes (top 10%)
- VRF-selected for fairness
- Majority vote (≥66% threshold)
- BLS signature aggregation

## Usage

### Submitting a Task

```rust
use adic_pouw::{TaskManager, Task, TaskType, ComputationType, ResourceRequirements};
use adic_economics::types::AdicAmount;

// Create task manager
let task_manager = TaskManager::new(
    escrow_manager,
    reputation_tracker,
    TaskManagerConfig::default(),
);

// Define task
let task = Task {
    task_id: compute_task_id(...),
    sponsor: sponsor_pubkey,
    task_type: TaskType::Compute {
        computation_type: ComputationType::HashVerification,
        resource_requirements: ResourceRequirements {
            max_cpu_ms: 5_000,      // 5 seconds
            max_memory_mb: 256,      // 256 MB
            max_storage_mb: 100,     // 100 MB
            max_network_kb: 1_024,   // 1 MB
        },
    },
    input_cid: "QmXyz123...",  // Input data on IPFS
    expected_output_schema: Some("hash:sha256"),
    reward: AdicAmount::from_adic(100.0),
    collateral_requirement: AdicAmount::from_adic(50.0),
    deadline_epoch: current_epoch + 10,
    min_reputation: 1000.0,
    worker_count: 3,  // Triple redundancy
    created_at: Utc::now(),
    status: TaskStatus::Submitted,
    finality_status: FinalityStatus::Pending,
};

// Submit task (escrows reward + deposit)
let task_id = task_manager.submit_task(task, current_epoch).await?;
```

### Executing a Task (Worker)

```rust
use adic_pouw::TaskExecutor;

let executor = TaskExecutor::new(ExecutorConfig::default());

// Fetch input data
let input_data = ipfs_fetch(&task.input_cid).await?;

// Execute task
let execution_result = executor.execute_compute_task(
    &task,
    &input_data,
    current_epoch,
).await?;

// Submit result
let work_result = WorkResult {
    result_id: compute_result_id(...),
    task_id: task.task_id,
    worker: worker_pubkey,
    output_cid: "QmResult456...",  // Upload output to IPFS
    execution_proof: execution_result.proof,
    execution_time_ms: execution_result.duration_ms,
    resources_used: execution_result.resources,
    submitted_at: Utc::now(),
    status: ResultStatus::Submitted,
};

task_manager.submit_work_result(work_result).await?;
```

### Validating a Result (Validator)

```rust
use adic_pouw::ResultValidator;

let validator = ResultValidator::new(
    reputation_tracker,
    ValidatorConfig::default(),
);

// Fetch result and re-execute
let result_data = ipfs_fetch(&work_result.output_cid).await?;
let expected = re_execute_task(&task, &input_data).await?;

// Vote on result
let vote = if result_data == expected {
    VoteType::Accept
} else {
    VoteType::Reject
};

let validation_vote = ValidationVote {
    result_id: work_result.result_id,
    validator: validator_pubkey,
    vote,
    signature: sign(&vote),
};

validator.submit_vote(validation_vote).await?;

// Aggregate votes from quorum
let report = validator.validate_result(
    &work_result,
    quorum_votes,
    current_epoch,
).await?;
```

## Configuration

### Task Manager Configuration

```rust
let config = TaskManagerConfig {
    min_deposit: AdicAmount::from_adic(1.0),       // Anti-spam deposit
    max_task_duration_epochs: 100,                  // Max task lifetime
    min_sponsor_reputation: 100.0,                  // Min Rep to submit
    challenge_window_epochs: 5,                     // Challenge window
    max_workers_per_task: 10,                       // Max redundancy
};
```

### Worker Selection Configuration

```rust
let config = WorkerSelectionConfig {
    vrf_seed_domain: "ADIC-POUW-WORKER-SELECTION-v1".to_string(),
    min_worker_reputation: 1000.0,
    reputation_weight_exponent: 2.0,                // w = R^2 for selection
    diversity_enforcement: true,
    max_tasks_per_worker: 5,                        // Concurrent limit
};
```

### Executor Configuration

```rust
let config = ExecutorConfig {
    default_timeout_ms: 30_000,                     // 30 seconds
    max_cpu_cores: 4,
    max_memory_mb: 2048,
    enable_network_isolation: true,
    proof_generation: true,
};
```

### Validator Configuration

```rust
let config = ValidatorConfig {
    quorum_size: 64,                                // Number of validators
    threshold: 43,                                  // BLS threshold (⌈2m/3⌉)
    validation_timeout_epochs: 3,
    cache_validation_results: true,
    validator_reputation_bonus: 0.01,               // 1% for correct vote
    validator_reputation_penalty: 0.01,             // 1% for incorrect vote
};
```

### Reward Configuration

```rust
let config = RewardConfig {
    quality_bonus_threshold: 0.8,                   // Quality > 0.8 for bonus
    quality_bonus_multiplier: 0.2,                  // 20% bonus
    speed_bonus_multiplier: 0.1,                    // 10% bonus
    slash_incorrect_result: 0.5,                    // 50% collateral
    slash_missed_deadline: 0.2,                     // 20% collateral
    slash_fraud: 1.0,                               // 100% collateral
};
```

### Reputation Configuration

```rust
let config = ReputationConfig {
    alpha: 0.1,                                     // 10% max increase
    beta: 0.2,                                      // 20% max decrease
    reputation_floor: 1.0,                          // Minimum Rep
    epoch_cap: 10.0,                                // Max ΔR per epoch
    validator_bonus: 0.01,
    validator_penalty: 0.01,
};
```

### Dispute Configuration

```rust
let config = DisputeConfig {
    min_challenge_stake: AdicAmount::from_adic(10.0),
    challenge_window_epochs: 5,
    arbitration_committee_size: 15,
    arbitration_threshold: 0.66,                    // 66% majority
    challenger_reward_share: 0.5,                   // 50% of slashed
    frivolous_challenge_penalty: 0.05,              // 5% Rep decrease
};
```

## Design Principles

### 1. Verifiable Computation

All task execution includes cryptographic proofs:
- **Execution proofs**: Hash of input, output, resource usage
- **Merkle proofs**: For storage/retrieval tasks
- **Zero-knowledge proofs**: For privacy-preserving computation (future)

### 2. Economic Security

- **Collateral requirements**: Workers escrow collateral ≥ 50% of reward
- **Slashing conditions**: Fraud, incorrect results, missed deadlines
- **Challenge period**: Time window for disputes before finalization
- **Challenger rewards**: Incentivize fraud detection

### 3. Reputation-Based Incentives

- **Higher Rep**: Better task selection probability
- **Good work**: Reputation increases (multiplicative-weights)
- **Bad work**: Reputation decreases + collateral slashed
- **Long-term**: Reputation compounds over time

### 4. Sybil Resistance

- **VRF selection**: Cannot predict or game worker lottery
- **Collateral requirements**: Cost to create multiple identities
- **Reputation tracking**: New identities start with low Rep
- **Multi-axis diversity**: Workers must be geographically/topologically diverse

### 5. Feeless Core

- **OCIM messages**: Task submissions are zero-value messages
- **Refundable deposits**: Anti-spam deposits returned on finality
- **Only escrow**: Rewards locked, not transferred until validation

## Testing

### Unit Tests (32 tests)

```bash
cargo test -p adic-pouw --lib
```

Coverage:
- Task manager: submission, lifecycle, reputation checks (3 tests)
- Worker selection: VRF lottery, eligibility, performance tracking (3 tests)
- Executor: hash verification, data processing, resource limits, proofs (4 tests)
- Validator: quorum voting, BLS aggregation, caching (3 tests)
- Rewards: distribution, bonuses, slashing, vesting (5 tests)
- Reputation: increase/decrease, quality scores, epoch caps, floors (8 tests)
- Disputes: challenges, arbitration, evidence, challenger rewards (6 tests)

### Module Tests

```bash
# Test task manager
cargo test -p adic-pouw task_manager::tests

# Test worker selection
cargo test -p adic-pouw worker_selection::tests

# Test disputes
cargo test -p adic-pouw disputes::tests
```

**Result: 32/32 tests passing ✅**

## Implementation Status

### ✅ Implemented

- Task submission and lifecycle management
- VRF-based worker selection
- Task execution with resource limits
- Quorum-based validation
- Reward distribution (atomic, vested, milestone)
- Reputation updates (multiplicative-weights)
- Dispute resolution (automatic + arbitration)
- Collateral management and slashing
- Performance tracking
- Statistics collection

### ⚠️ Partial Implementation

- BLS threshold signatures (placeholder)
- F1+F2 finality integration (placeholder)
- ADIC message integration (not yet OCIM)
- Cross-chain settlement (not yet connected)

### ❌ Not Yet Implemented

- Zero-knowledge proof support
- Privacy-preserving computation
- Advanced task types (TEE, MPC)
- Task marketplace UI
- Worker node software
- Sponsor dashboard

## Design Documents

**Note:** The following design documents are conceptual references. The actual implementation is in this crate and related PoUW infrastructure crates.

- **ADIC PoUW I** (Conceptual): Sponsor Hooks and Epoch Quorums for Outsourced Distributed Computing
  - *Implemented in*: `src/task_manager.rs`, `src/executor.rs`
- **ADIC PoUW II** (Conceptual): Normative Randomness, Quorum Selection, Challenge/Slashing
  - *Implemented in*: `../adic-vrf/`, `../adic-quorum/`, `../adic-challenges/`
- **ADIC PoUW III** (Conceptual): Governance, Parameterization, Treasury Execution
  - *Implemented in*: `../adic-governance/src/parameters.rs`, `src/rewards.rs`

## Performance Characteristics

- **Worker Selection**: O(n log k) for n eligible workers, k selected
- **Validation**: O(m) for m quorum validators
- **Reputation Update**: O(1) per worker
- **Dispute Resolution**: O(c) for c committee members
- **Reward Distribution**: O(w) for w workers

## Future Enhancements

1. **Zero-Knowledge Proofs**: Privacy-preserving task execution (zk-SNARKs)
2. **Trusted Execution Environments**: Hardware-backed computation (SGX, TrustZone)
3. **Multi-Party Computation**: Distributed computation without revealing inputs
4. **Machine Learning Tasks**: Model training/inference at scale
5. **Cross-Chain Settlement**: Atomic swaps for rewards in other tokens
6. **Advanced Slashing**: Graduated penalties based on severity
7. **Reputation Staking**: Lock Rep for higher task priority
8. **Task Marketplace**: Decentralized exchange for compute resources

## License

Same as parent ADIC project (check repository root).

## References

### Published Papers
- [ADIC-DAG Paper](../../docs/references/adic-dag-paper.pdf)
- [ADIC Yellow Paper](../../docs/references/ADICYellowPaper.pdf)
- [ADIC Applications Paper](../../docs/references/ADICApplications.pdf)

### Related Crates
- [ADIC VRF](../adic-vrf/) - VRF-based worker selection
- [ADIC Quorum](../adic-quorum/) - Quorum selection and validation
- [ADIC Challenges](../adic-challenges/) - Challenge and dispute resolution
- [ADIC App Common](../adic-app-common/) - Shared application primitives
- [ADIC Governance](../adic-governance/) - Parameter governance
