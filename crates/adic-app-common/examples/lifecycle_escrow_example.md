# Lifecycle Manager with Escrow Integration

This document demonstrates how to use the `LifecycleManager` with escrow operations for application messages like governance proposals or storage deals.

## Overview

The `LifecycleManager` provides a generic framework for managing state transitions with integrated escrow operations and event emission:

1. **State Transitions**: Validates and executes state transitions based on lifecycle state machines
2. **Escrow Integration**: Automatically executes escrow operations (lock/release/refund/slash) during transitions
3. **Event Emission**: Emits lifecycle events for external listeners
4. **Finality Checking**: Optionally waits for finality before allowing certain transitions

## Example: Treasury Grant Proposal

### Step 1: Define Your State Machine

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalStatus {
    Voting,
    Succeeded,
    Enacting,
    Enacted,
    Failed,
}

impl LifecycleState for ProposalStatus {
    fn is_terminal(&self) -> bool {
        matches!(self, Self::Enacted | Self::Failed)
    }

    fn can_transition_to(&self, next: &Self) -> bool {
        use ProposalStatus::*;
        matches!(
            (self, next),
            (Voting, Succeeded) |
            (Succeeded, Enacting) |
            (Enacting, Enacted) |
            (Enacting, Failed)
        )
    }
}
```

### Step 2: Lock Funds When Proposal Succeeds

When a proposal succeeds voting, lock the grant amount in escrow:

```rust
// Proposal succeeded - lock grant funds
let escrow_action = EscrowAction::Lock {
    escrow_type: EscrowType::TreasuryGrant {
        proposal_id: proposal.id(),
        recipient: proposal.grant_recipient,
    },
    amount: proposal.grant_amount,
    epoch: current_epoch,
};

// Execute transition with escrow
let transition = lifecycle_mgr
    .transition_with_escrow(
        &proposal,
        StateEvent::Accepted,
        Some(escrow_action)
    )
    .await?;

// Lock ID is available in the transition result
let lock_id = transition.escrow_action
    .and_then(|action| match action {
        EscrowAction::Lock { escrow_type, .. } => Some(escrow_type.to_lock_id()),
        _ => None,
    });
```

### Step 3: Release Funds When Proposal is Enacted

After the proposal passes finality and timelock, release the escrowed funds:

```rust
// Enact proposal - release funds to recipient
let release_action = EscrowAction::Release {
    lock_id: stored_lock_id,
    to: proposal.grant_recipient,
};

lifecycle_mgr
    .transition_with_escrow(
        &proposal,
        StateEvent::Accepted,
        Some(release_action)
    )
    .await?;
```

### Step 4: Refund if Proposal Fails

If the proposal fails to enact, refund the funds to the original owner:

```rust
// Proposal failed - refund escrowed funds
let refund_action = EscrowAction::Refund {
    lock_id: stored_lock_id,
};

lifecycle_mgr
    .transition_with_escrow(
        &proposal,
        StateEvent::Rejected,
        Some(refund_action)
    )
    .await?;
```

## Event Emission

To receive lifecycle events, create the manager with event emission enabled:

```rust
let (lifecycle_mgr, mut event_rx) = LifecycleManager::<GovernanceProposal>::with_events(
    finality_engine,
    escrow_manager,
);

// Spawn a task to handle events
tokio::spawn(async move {
    while let Some(event) = event_rx.recv().await {
        match event.to_state {
            ProposalStatus::Enacted => {
                info!(
                    "Proposal {} enacted with lock {:?}",
                    event.message_id,
                    event.lock_id
                );
                // Trigger follow-up actions...
            }
            ProposalStatus::Failed => {
                warn!("Proposal {} failed", event.message_id);
            }
            _ => {}
        }
    }
});
```

## Escrow Types

The system supports different escrow types with automatic lock ID generation:

- **TreasuryGrant**: `{ proposal_id, recipient }`
- **StorageCollateral**: `{ deal_id, provider }`
- **StoragePayment**: `{ deal_id, client }`
- **PoUWDeposit**: `{ hook_id, worker }`
- **Custom**: `{ lock_id, owner }`

Each type generates a unique lock ID that can be used for later release/refund/slash operations.

## Integration with Existing Systems

### Governance Module

The `ProposalLifecycleManager` in `adic-governance` handles voting and enactment logic. Escrow operations should be integrated at:

1. **Tallying → Succeeded**: Lock treasury grant funds if proposal passes
2. **Enacting → Enacted**: Release funds to grant recipient
3. **Enacting → Failed**: Refund funds to treasury

### Storage Market

Storage deals can use the same pattern:

1. **Pending → Active**: Lock provider collateral and client payment
2. **Active → Completed**: Release payment to provider, refund collateral
3. **Active → Slashed**: Slash provider collateral to treasury

### PoUW Tasks

Worker deposits follow a similar lifecycle:

1. **Submitted → Assigned**: Lock worker deposit
2. **Verified → Completed**: Release deposit back to worker
3. **Disputed → Slashed**: Slash deposit for fraudulent work

## Testing

When testing lifecycle transitions with escrow, use the provided mocks:

```rust
// Mock implementations are available in adic-app-common/src/mocks.rs
use adic_app_common::mocks::{MockEscrowManager, MockFinalityEngine};

let escrow_mgr = Arc::new(MockEscrowManager::new());
let finality_engine = Arc::new(MockFinalityEngine::new());

let lifecycle_mgr = LifecycleManager::<YourMessage>::new(
    finality_engine,
    escrow_mgr,
);

// Test transitions...
```

## Summary

The lifecycle manager with escrow integration provides:

✅ Type-safe state transitions with compile-time validation
✅ Automatic escrow operations during state changes
✅ Event emission for external coordination
✅ Finality checking before critical transitions
✅ Support for multiple escrow types (treasury, storage, PoUW)
✅ Lock/Release/Refund/Slash operations

This pattern enables consistent, safe management of funds throughout the lifecycle of application messages in the ADIC protocol.
