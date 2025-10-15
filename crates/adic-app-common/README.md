# ADIC App Common

Shared utilities and building blocks for ADIC application layer: escrow management, state machines, and lifecycle helpers.

## Overview

The ADIC App Common module provides **reusable infrastructure** for building applications on ADIC-DAG. It abstracts common patterns like escrow locking/unlocking, state machine transitions, and lifecycle management to reduce code duplication and ensure consistency across applications.

## Components

### 1. Escrow Manager (`src/escrow.rs`)

**Purpose**: Lock and unlock funds for multi-stage transactions (deals, tasks, grants).

**Key Features**:
- Lock funds with expiry epochs
- Multiple escrow types (StorageDeal, PoUWTask, GovernanceGrant, Custom)
- Refundable deposits
- Automatic expiry handling
- Balance manager integration

**Usage**:
```rust
use adic_app_common::escrow::{EscrowManager, EscrowType};
use adic_economics::types::AdicAmount;

let escrow_mgr = EscrowManager::new(balance_manager);

// Lock funds for storage deal
let lock_id = escrow_mgr.lock(
    EscrowType::StorageDeal { deal_id: [1; 32] },
    AdicAmount::from_adic(100.0),
    expiry_epoch: current_epoch + 100,
).await?;

// Later: unlock funds (after deal completes)
escrow_mgr.unlock(lock_id).await?;

// Or: extend lock if deal renewed
escrow_mgr.extend_lock(lock_id, additional_epochs: 50).await?;
```

**Escrow Types**:
```rust
pub enum EscrowType {
    /// Storage market deal escrow
    StorageDeal { deal_id: Hash },

    /// PoUW task reward/collateral escrow
    PoUWTask { task_id: Hash },

    /// Governance grant escrow
    GovernanceGrant { proposal_id: Hash },

    /// Custom escrow with identifier
    Custom { lock_id: String, owner: AccountAddress },
}
```

### 2. State Machine (`src/state_machine.rs`)

**Purpose**: Generic state transition validation and enforcement.

**Key Features**:
- Define allowed state transitions
- Validate transitions before applying
- Track transition history
- Event emission on state change

**Usage**:
```rust
use adic_app_common::state_machine::{StateMachine, Transition};

// Define state machine for storage deal
let mut sm = StateMachine::new("storage_deal", DealStatus::PendingActivation);

// Define allowed transitions
sm.allow_transition(DealStatus::PendingActivation, DealStatus::Active);
sm.allow_transition(DealStatus::Active, DealStatus::Completed);
sm.allow_transition(DealStatus::Active, DealStatus::Failed);

// Attempt transition
match sm.transition_to(DealStatus::Active) {
    Ok(()) => println!("Deal activated"),
    Err(e) => println!("Invalid transition: {}", e),
}

// Get current state
assert_eq!(sm.current_state(), &DealStatus::Active);
```

### 3. Lifecycle Manager (`src/lifecycle.rs`)

**Purpose**: Manage multi-phase lifecycle (e.g., task submission → execution → validation → finalization).

**Key Features**:
- Phase tracking
- Deadline enforcement
- Automatic phase transitions
- Callbacks on phase changes

**Usage**:
```rust
use adic_app_common::lifecycle::{LifecycleManager, LifecyclePhase};

let lifecycle = LifecycleManager::new("pouw_task", vec![
    LifecyclePhase {
        name: "Submission",
        max_duration_epochs: 1,
        required: true,
    },
    LifecyclePhase {
        name: "Execution",
        max_duration_epochs: 10,
        required: true,
    },
    LifecyclePhase {
        name: "Validation",
        max_duration_epochs: 3,
        required: true,
    },
    LifecyclePhase {
        name: "Challenge",
        max_duration_epochs: 5,
        required: true,
    },
    LifecyclePhase {
        name: "Finalization",
        max_duration_epochs: 1,
        required: true,
    },
]);

// Advance to next phase
lifecycle.advance_phase(current_epoch).await?;

// Check if current phase expired
if lifecycle.is_phase_expired(current_epoch).await? {
    println!("Phase expired, cancelling task");
    lifecycle.cancel().await?;
}
```

### 4. Event Bus (`src/events.rs`)

**Purpose**: Publish/subscribe for application events.

**Key Features**:
- Type-safe event publishing
- Multiple subscribers per event type
- Async event handlers
- Event filtering

**Usage**:
```rust
use adic_app_common::events::{EventBus, Event};

let bus = EventBus::new();

// Subscribe to DealActivated events
bus.subscribe::<DealActivatedEvent>(|event| {
    println!("Deal {} activated", event.deal_id);
});

// Publish event
bus.publish(DealActivatedEvent {
    deal_id: [1; 32],
    provider: provider_pk,
    activation_epoch: current_epoch,
}).await;
```

### 5. Utilities (`src/utils.rs`)

**Purpose**: Common helper functions.

**Functions**:
- `compute_hash`: Blake3 hashing
- `generate_id`: Content-addressed IDs
- `encode_p_adic`: p-adic encoding helpers
- `format_amount`: Human-readable ADIC amounts
- `parse_cid`: IPFS CID validation
- `timestamp_to_epoch`: Convert timestamps to epochs

## Error Types

```rust
pub enum AppError {
    /// Escrow-related errors
    EscrowNotFound(String),
    EscrowExpired { lock_id: String, expired_at_epoch: u64 },
    InsufficientBalance { required: String, available: String },

    /// State machine errors
    InvalidTransition { from: String, to: String },
    InvalidState(String),

    /// Lifecycle errors
    PhaseExpired { phase: String, deadline_epoch: u64 },
    InvalidPhase(String),

    /// General errors
    InvalidInput(String),
    Other(String),
}
```

## Configuration

Each component has its own configuration:

```rust
// Escrow configuration
pub struct EscrowConfig {
    pub default_expiry_epochs: u64,
    pub min_lock_amount: AdicAmount,
    pub max_locks_per_address: usize,
}

// State machine configuration
pub struct StateMachineConfig {
    pub enable_history: bool,
    pub max_history_size: usize,
    pub emit_events: bool,
}

// Lifecycle configuration
pub struct LifecycleConfig {
    pub strict_deadlines: bool,
    pub auto_advance: bool,
    pub enable_callbacks: bool,
}
```

## Integration Examples

### Storage Market Deal Flow

```rust
use adic_app_common::escrow::EscrowManager;
use adic_app_common::state_machine::StateMachine;

// 1. Lock client payment + provider collateral
let client_escrow = escrow_mgr.lock(
    EscrowType::StorageDeal { deal_id },
    deal.payment,
    expiry: deal.deadline_epoch,
).await?;

let provider_escrow = escrow_mgr.lock(
    EscrowType::StorageDeal { deal_id },
    deal.collateral,
    expiry: deal.deadline_epoch,
).await?;

// 2. Track deal state
let mut deal_sm = StateMachine::new("storage_deal", DealStatus::PendingActivation);
deal_sm.allow_transition(DealStatus::PendingActivation, DealStatus::Active);
deal_sm.allow_transition(DealStatus::Active, DealStatus::Completed);

// 3. Provider activates deal
deal_sm.transition_to(DealStatus::Active).await?;

// 4. Deal completes successfully
deal_sm.transition_to(DealStatus::Completed).await?;

// 5. Release escrowed funds
escrow_mgr.unlock(client_escrow).await?;  // Pay provider
escrow_mgr.unlock(provider_escrow).await?;  // Return collateral
```

### PoUW Task Lifecycle

```rust
use adic_app_common::lifecycle::LifecycleManager;
use adic_app_common::events::EventBus;

// 1. Create task lifecycle
let lifecycle = LifecycleManager::new("pouw_task", task_phases);

// 2. Subscribe to phase changes
event_bus.subscribe::<PhaseChangedEvent>(|event| {
    match event.new_phase.as_str() {
        "Execution" => start_worker_execution(),
        "Validation" => start_quorum_validation(),
        "Challenge" => open_challenge_window(),
        "Finalization" => distribute_rewards(),
        _ => {}
    }
});

// 3. Advance through phases
lifecycle.advance_phase(current_epoch).await?;  // Submission → Execution
// ... worker executes ...
lifecycle.advance_phase(current_epoch).await?;  // Execution → Validation
// ... quorum validates ...
lifecycle.advance_phase(current_epoch).await?;  // Validation → Challenge
// ... challenge window ...
lifecycle.advance_phase(current_epoch).await?;  // Challenge → Finalization
```

### Governance Grant Vesting

```rust
use adic_app_common::escrow::EscrowManager;

// 1. Lock total grant amount
let grant_escrow = escrow_mgr.lock(
    EscrowType::GovernanceGrant { proposal_id },
    grant.total_amount,
    expiry: grant.final_milestone_epoch,
).await?;

// 2. Release tranches over time
for milestone in grant.milestones {
    // Wait for milestone completion
    wait_for_milestone(milestone.id).await?;

    // Unlock tranche
    let tranche_escrow = escrow_mgr.create_sub_lock(
        grant_escrow,
        milestone.amount,
    ).await?;

    escrow_mgr.unlock(tranche_escrow).await?;  // Pay to recipient
}
```

## Design Principles

### 1. Separation of Concerns

Each component has a single responsibility:
- Escrow: Fund locking
- State Machine: Transition validation
- Lifecycle: Phase management
- Events: Communication

### 2. Composability

Components work together but can be used independently:
```rust
// Use escrow without state machine
let lock = escrow_mgr.lock(...).await?;

// Use state machine without lifecycle
let mut sm = StateMachine::new(...);

// Combine as needed
lifecycle.on_phase_change(|phase| {
    sm.transition_to(phase.to_state())?;
    if phase == "Finalization" {
        escrow_mgr.unlock(lock)?;
    }
});
```

### 3. Type Safety

Leverage Rust's type system:
- Strongly-typed states
- Generic state machines
- Compile-time validation

### 4. Async-First

All I/O operations are async:
- Non-blocking escrow operations
- Async event handlers
- Parallel phase processing

## Testing

### Unit Tests

```bash
cargo test -p adic-app-common --lib
```

Coverage:
- Escrow lock/unlock/extend operations
- State machine transition validation
- Lifecycle phase advancement
- Event publishing and subscription
- Utility function correctness

**Result: All tests passing ✅**

## Performance Characteristics

- **Escrow Lock**: O(1) hash map insert
- **Escrow Unlock**: O(1) hash map remove
- **State Transition**: O(1) validation lookup
- **Event Publish**: O(n) for n subscribers
- **Phase Advance**: O(1) phase counter increment

## Future Enhancements

1. **Persistent Storage**: Save escrow state to disk
2. **Event Replay**: Reconstruct state from event log
3. **Parallel State Machines**: Manage multiple SMs efficiently
4. **Advanced Escrow**: Conditional unlocks, multi-sig
5. **Lifecycle Templates**: Pre-defined common patterns

## References

- Used by: Storage Market, PoUW, Governance modules
- Integrates with: adic-economics (balance management)

## License

Same as parent ADIC project (check repository root).
