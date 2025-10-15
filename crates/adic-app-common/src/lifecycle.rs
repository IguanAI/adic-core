use crate::{AppError, AppMessage, EscrowManager, LockId, Result, StateEvent};
use adic_finality::{FinalityArtifact, FinalityEngine, FinalityGate};
use std::marker::PhantomData;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

pub use crate::message_trait::LifecycleState;

/// State transition result
#[derive(Debug, Clone)]
pub struct StateTransition<S: LifecycleState> {
    pub from: S,
    pub to: S,
    pub event: StateEvent,
    pub finality_artifact: Option<FinalityArtifact>,
    pub escrow_action: Option<EscrowAction>,
}

/// Lifecycle event emitted after successful state transition
#[derive(Debug, Clone)]
pub struct LifecycleEvent<S: LifecycleState> {
    pub message_id: adic_types::MessageId,
    pub from_state: S,
    pub to_state: S,
    pub event: StateEvent,
    pub lock_id: Option<LockId>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Escrow action to be performed during lifecycle transition
#[derive(Debug, Clone)]
pub enum EscrowAction {
    /// Lock funds in escrow (returns LockId)
    Lock {
        escrow_type: crate::EscrowType,
        amount: adic_economics::types::AdicAmount,
        epoch: u64,
    },
    /// Release escrowed funds to recipient
    Release {
        lock_id: LockId,
        to: adic_economics::types::AccountAddress,
    },
    /// Refund escrowed funds to owner
    Refund { lock_id: LockId },
    /// Slash escrowed funds to treasury
    Slash {
        lock_id: LockId,
        treasury: adic_economics::types::AccountAddress,
    },
    /// No escrow action needed
    None,
}

/// Generic lifecycle manager for application messages
/// Handles state transitions, finality checking, and escrow management
pub struct LifecycleManager<T: AppMessage> {
    finality_service: Arc<FinalityEngine>,
    escrow_service: Arc<EscrowManager>,
    event_tx: Option<mpsc::UnboundedSender<LifecycleEvent<T::Lifecycle>>>,
    _phantom: PhantomData<T>,
}

impl<T: AppMessage> LifecycleManager<T> {
    pub fn new(finality_service: Arc<FinalityEngine>, escrow_service: Arc<EscrowManager>) -> Self {
        Self {
            finality_service,
            escrow_service,
            event_tx: None,
            _phantom: PhantomData,
        }
    }

    /// Create a new lifecycle manager with event emission
    pub fn with_events(
        finality_service: Arc<FinalityEngine>,
        escrow_service: Arc<EscrowManager>,
    ) -> (Self, mpsc::UnboundedReceiver<LifecycleEvent<T::Lifecycle>>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let manager = Self {
            finality_service,
            escrow_service,
            event_tx: Some(tx),
            _phantom: PhantomData,
        };
        (manager, rx)
    }

    /// Attempt to transition a message to a new state based on an event
    pub async fn transition(
        &self,
        msg: &T,
        event: StateEvent,
    ) -> Result<StateTransition<T::Lifecycle>> {
        self.transition_with_escrow(msg, event, None).await
    }

    /// Attempt to transition a message to a new state with optional escrow action
    pub async fn transition_with_escrow(
        &self,
        msg: &T,
        event: StateEvent,
        escrow_action: Option<EscrowAction>,
    ) -> Result<StateTransition<T::Lifecycle>> {
        let current_state = msg.lifecycle_state();

        // Check if already in terminal state
        if current_state.is_terminal() {
            return Err(AppError::InvalidTransition(
                "Cannot transition from terminal state".to_string(),
            ));
        }

        // Determine next state based on event
        let next_state = self.compute_next_state(msg, &event).await?;

        // Validate transition
        if !current_state.can_transition_to(&next_state) {
            return Err(AppError::InvalidTransition(format!(
                "Invalid transition from {:?} to {:?}",
                current_state, next_state
            )));
        }

        // Check finality if required
        let finality_artifact = if matches!(event, StateEvent::FinalityAchieved) {
            Some(self.check_finality(msg).await?)
        } else {
            None
        };

        // Execute escrow action if provided
        let lock_id = if let Some(ref action) = escrow_action {
            self.execute_escrow_action(action).await?
        } else {
            None
        };

        // Emit lifecycle event if event channel is configured
        if let Some(ref tx) = self.event_tx {
            let lifecycle_event = LifecycleEvent {
                message_id: msg.id(),
                from_state: current_state.clone(),
                to_state: next_state.clone(),
                event: event.clone(),
                lock_id: lock_id.clone(),
                timestamp: chrono::Utc::now(),
            };

            if let Err(e) = tx.send(lifecycle_event) {
                warn!(
                    msg_id = %msg.id(),
                    error = %e,
                    "Failed to emit lifecycle event"
                );
            } else {
                debug!(
                    msg_id = %msg.id(),
                    from = ?current_state,
                    to = ?next_state,
                    "Lifecycle event emitted"
                );
            }
        }

        Ok(StateTransition {
            from: current_state,
            to: next_state,
            event,
            finality_artifact,
            escrow_action,
        })
    }

    /// Check if message has achieved required finality
    pub async fn check_finality(&self, msg: &T) -> Result<FinalityArtifact> {
        let required_gate = msg.required_finality();
        let msg_id = msg.id();

        debug!(
            msg_id = %msg_id,
            required_gate = ?required_gate,
            "Checking finality for message"
        );

        // Query finality engine for artifact
        let artifact = self
            .finality_service
            .get_artifact(&msg_id)
            .await
            .ok_or_else(|| {
                AppError::FinalityNotMet(format!("Message {} has not achieved finality", msg_id))
            })?;

        // Verify it meets the required gate
        if !self.gate_meets_requirement(&artifact.gate, &required_gate) {
            return Err(AppError::FinalityNotMet(format!(
                "Message {} achieved {:?} but requires {:?}",
                msg_id, artifact.gate, required_gate
            )));
        }

        info!(
            msg_id = %msg_id,
            gate = ?artifact.gate,
            "Message achieved finality"
        );

        Ok(artifact)
    }

    /// Check if achieved gate meets required gate
    fn gate_meets_requirement(&self, achieved: &FinalityGate, required: &FinalityGate) -> bool {
        // F1 (k-core) < F2 (persistent homology) in terms of strength
        // F2 implies F1
        match (achieved, required) {
            (a, r) if a == r => true,
            (FinalityGate::F2PersistentHomology, FinalityGate::F1KCore) => true,
            _ => false,
        }
    }

    /// Execute actions when message reaches finality
    pub async fn execute_on_finality(&self, msg: &T) -> Result<()> {
        let msg_id = msg.id();

        info!(
            msg_id = %msg_id,
            msg_type = ?msg.app_type(),
            "Executing finality actions"
        );

        // Application-specific finality actions are handled by the specific implementation
        // This is a hook point that can be extended

        Ok(())
    }

    /// Compute the next state based on the current state and event
    ///
    /// WARNING: This default implementation is a base placeholder intended for
    /// application-specific managers to override. It returns the current state
    /// unchanged. Production application lifecycles (e.g., Storage, PoUW, Governance)
    /// should provide concrete transition logic and must not rely on this default
    /// behavior for state progression.
    async fn compute_next_state(&self, msg: &T, _event: &StateEvent) -> Result<T::Lifecycle> {
        // This is a placeholder - actual state transition logic
        // should be implemented by specific application types
        // For now, return current state
        Ok(msg.lifecycle_state())
    }

    /// Execute escrow action during state transition
    /// Returns the LockId if a lock was created, otherwise None
    async fn execute_escrow_action(&self, action: &EscrowAction) -> Result<Option<LockId>> {
        match action {
            EscrowAction::Lock {
                escrow_type,
                amount,
                epoch,
            } => {
                info!(
                    escrow_type = ?escrow_type,
                    amount = %amount,
                    epoch = epoch,
                    "Locking funds in escrow"
                );

                let lock_id = self
                    .escrow_service
                    .lock(escrow_type.clone(), *amount, *epoch)
                    .await
                    .map_err(|e| AppError::EscrowError(e.to_string()))?;

                info!(lock_id = %lock_id, "Escrow lock successful");
                Ok(Some(lock_id))
            }

            EscrowAction::Release { lock_id, to } => {
                info!(
                    lock_id = %lock_id,
                    recipient = %to,
                    "Releasing escrowed funds"
                );

                self.escrow_service
                    .release(lock_id, *to)
                    .await
                    .map_err(|e| AppError::EscrowError(e.to_string()))?;

                info!(lock_id = %lock_id, "Escrow release successful");
                Ok(None)
            }

            EscrowAction::Refund { lock_id } => {
                info!(
                    lock_id = %lock_id,
                    "Refunding escrowed funds to owner"
                );

                self.escrow_service
                    .refund(lock_id)
                    .await
                    .map_err(|e| AppError::EscrowError(e.to_string()))?;

                info!(lock_id = %lock_id, "Escrow refund successful");
                Ok(None)
            }

            EscrowAction::Slash { lock_id, treasury } => {
                info!(
                    lock_id = %lock_id,
                    treasury = %treasury,
                    "Slashing escrowed funds to treasury"
                );

                self.escrow_service
                    .slash(lock_id, *treasury)
                    .await
                    .map_err(|e| AppError::EscrowError(e.to_string()))?;

                info!(lock_id = %lock_id, "Escrow slash successful");
                Ok(None)
            }

            EscrowAction::None => {
                debug!("No escrow action required");
                Ok(None)
            }
        }
    }
}

/// Helper trait for managing message collections with lifecycle
pub trait LifecycleCollection<T: AppMessage> {
    /// Get all messages in a specific state
    fn in_state(&self, state: T::Lifecycle) -> Vec<T>;

    /// Update message state
    fn update_state(&mut self, msg: T) -> Result<()>;

    /// Remove message
    fn remove(&mut self, id: &adic_types::MessageId) -> Option<T>;
}

#[cfg(test)]
mod tests {
    // Mock implementations for testing would go here
}
