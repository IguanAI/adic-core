pub mod admissibility;
pub mod conflict;
pub mod deposit;
pub mod energy_descent;
pub mod reputation;
pub mod validation;

#[cfg(test)]
mod c1_security_test;

pub use admissibility::{AdmissibilityChecker, AdmissibilityResult};
pub use conflict::{ConflictEnergy, ConflictResolver};
pub use deposit::{
    Deposit, DepositManager, DepositState, DepositStats, DepositStatus, DEFAULT_DEPOSIT_AMOUNT,
};
pub use energy_descent::{EnergyDescentTracker, EnergyMetrics};
pub use reputation::{ReputationChangeEvent, ReputationTracker};
pub use validation::{MessageValidator, ValidationResult};

use adic_storage::StorageEngine;
use adic_types::{AdicMessage, AdicParams, Result};
use std::sync::Arc;

pub struct ConsensusEngine {
    params: Arc<AdicParams>,
    admissibility: AdmissibilityChecker,
    pub deposits: DepositManager,
    pub reputation: ReputationTracker,
    validator: MessageValidator,
    conflicts: ConflictResolver,
    pub energy_tracker: EnergyDescentTracker,
    storage: Arc<StorageEngine>,
}

impl ConsensusEngine {
    pub fn new(params: AdicParams, storage: Arc<StorageEngine>) -> Self {
        let shared_params = Arc::new(params.clone());
        Self {
            admissibility: AdmissibilityChecker::new(params.clone()),
            deposits: DepositManager::new(params.deposit),
            reputation: ReputationTracker::new(params.gamma),
            validator: MessageValidator::new(),
            conflicts: ConflictResolver::new(),
            energy_tracker: EnergyDescentTracker::new(params.lambda, params.mu),
            params: shared_params,
            storage,
        }
    }

    pub async fn validate_and_slash(&self, message: &AdicMessage) -> Result<ValidationResult> {
        let result = self.validator.validate_message(message);
        if !result.is_valid {
            let reason = result.errors.join(", ");
            // Slash the deposit for the invalid message
            self.deposits.slash(&message.id, &reason).await?;

            // Penalize the proposer's reputation
            if let Some(proposer) = self.deposits.get_proposer(&message.id).await {
                // The penalty can be a fixed value or scaled by the severity of the error
                self.reputation.bad_update(&proposer, 1.0).await;
            }
        }
        Ok(result)
    }

    pub fn params(&self) -> &AdicParams {
        &self.params
    }

    pub fn admissibility(&self) -> &AdmissibilityChecker {
        &self.admissibility
    }

    /// Track a conflict and update energy
    pub async fn track_conflict(
        &self,
        conflict_id: adic_types::ConflictId,
        message_id: adic_types::MessageId,
    ) -> Result<()> {
        // Register conflict if new
        self.energy_tracker
            .register_conflict(conflict_id.clone())
            .await;

        // Update support for this message
        self.energy_tracker
            .update_support(&conflict_id, message_id, &self.storage, &self.reputation)
            .await?;

        // Also update legacy conflict resolver for compatibility
        self.conflicts
            .update_support(&conflict_id, message_id, 0.0, 0)
            .await?;

        Ok(())
    }

    /// Get conflict penalty for MRW
    pub async fn get_conflict_penalty(
        &self,
        message_id: &adic_types::MessageId,
        conflict_id: &adic_types::ConflictId,
    ) -> f64 {
        self.energy_tracker
            .get_conflict_penalty(message_id, conflict_id)
            .await
    }

    /// Check if conflict is resolved
    pub async fn is_conflict_resolved(&self, conflict_id: &adic_types::ConflictId) -> bool {
        self.energy_tracker.is_resolved(conflict_id).await
    }

    /// Get energy metrics for monitoring
    pub async fn get_energy_metrics(&self) -> EnergyMetrics {
        self.energy_tracker.get_metrics().await
    }

    pub fn validator(&self) -> &MessageValidator {
        &self.validator
    }

    pub fn conflicts(&self) -> &ConflictResolver {
        &self.conflicts
    }

    pub fn conflict_resolver(&self) -> &ConflictResolver {
        &self.conflicts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_storage::{store::BackendType, StorageConfig};
    use adic_types::features::{AxisPhi, QpDigits};
    use adic_types::{AdicFeatures, AdicMeta, ConflictId, MessageId, PublicKey};
    use chrono::Utc;
    use std::sync::Arc;

    fn create_test_storage() -> Arc<StorageEngine> {
        let storage_config = StorageConfig {
            backend_type: BackendType::Memory,
            ..Default::default()
        };
        Arc::new(StorageEngine::new(storage_config).unwrap())
    }

    #[test]
    fn test_consensus_engine_creation() {
        let params = AdicParams::default();
        let storage = create_test_storage();
        let engine = ConsensusEngine::new(params, storage);
        assert_eq!(engine.params().d, 3);
        assert_eq!(engine.params().k, 20); // Production value from paper
        assert_eq!(engine.params().gamma, 0.9);
    }

    #[tokio::test]
    async fn test_validate_and_slash_with_invalid_signature() {
        let params = AdicParams::default();
        let storage = create_test_storage();
        let engine = ConsensusEngine::new(params, storage);

        let mut message = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![
                AxisPhi::new(0, QpDigits::from_u64(1, 3, 10)),
                AxisPhi::new(1, QpDigits::from_u64(2, 3, 10)),
                AxisPhi::new(2, QpDigits::from_u64(3, 3, 10)),
            ]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([1; 32]),
            vec![],
        );

        // Add a non-empty but invalid signature
        message.signature = adic_types::Signature::new(vec![1; 64]);

        // First escrow deposit for the message
        engine
            .deposits
            .escrow(message.id, PublicKey::from_bytes([1; 32]))
            .await
            .unwrap();

        let result = engine.validate_and_slash(&message).await.unwrap();

        // Message with invalid signature should fail validation
        assert!(!result.is_valid);
        assert!(!result.errors.is_empty());

        // Deposit should be slashed due to invalid message
        let deposit_state = engine.deposits.get_state(&message.id).await;
        assert_eq!(deposit_state, Some(DepositState::Slashed));
    }

    #[tokio::test]
    async fn test_validate_and_slash_invalid_message() {
        let params = AdicParams::default();
        let storage = create_test_storage();
        let engine = ConsensusEngine::new(params, storage);

        // Create an invalid message with too many parents
        let mut parents = vec![];
        for i in 0..10 {
            parents.push(MessageId::new(&[i; 32]));
        }

        let message = AdicMessage::new(
            parents,
            AdicFeatures::new(vec![
                AxisPhi::new(0, QpDigits::from_u64(1, 3, 10)),
                AxisPhi::new(1, QpDigits::from_u64(2, 3, 10)),
                AxisPhi::new(2, QpDigits::from_u64(3, 3, 10)),
            ]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([1; 32]),
            vec![],
        );

        // First escrow a deposit for this message
        engine
            .deposits
            .escrow(message.id, PublicKey::from_bytes([1; 32]))
            .await
            .unwrap();

        let result = engine.validate_and_slash(&message).await.unwrap();
        assert!(!result.is_valid);
        assert!(!result.errors.is_empty());

        // Check deposit was slashed
        let deposit_state = engine.deposits.get_state(&message.id).await;
        assert_eq!(deposit_state, Some(DepositState::Slashed));
    }

    #[test]
    fn test_consensus_engine_getters() {
        let params = AdicParams::default();
        let storage = create_test_storage();
        let engine = ConsensusEngine::new(params.clone(), storage);

        assert_eq!(engine.params().d, params.d);
        assert_eq!(engine.params().k, params.k);
        assert_eq!(engine.params().gamma, params.gamma);

        // Test that getters return proper references
        let _ = engine.admissibility();
        let _ = engine.validator();
        let _ = engine.conflicts();
        let _ = engine.conflict_resolver();
    }

    #[tokio::test]
    async fn test_consensus_engine_reputation_tracking() {
        let params = AdicParams::default();
        let storage = create_test_storage();
        let engine = ConsensusEngine::new(params, storage);

        let proposer = PublicKey::from_bytes([2; 32]);
        let initial_rep = engine.reputation.get_reputation(&proposer).await;
        assert_eq!(initial_rep, 1.0);

        // Good update
        engine.reputation.good_update(&proposer, 5.0, 10).await;
        let new_rep = engine.reputation.get_reputation(&proposer).await;
        assert!(new_rep > initial_rep);

        // Bad update
        engine.reputation.bad_update(&proposer, 2.0).await;
        let final_rep = engine.reputation.get_reputation(&proposer).await;
        assert!(final_rep < new_rep);
    }

    #[tokio::test]
    async fn test_consensus_engine_conflict_resolution() {
        let params = AdicParams::default();
        let storage = create_test_storage();
        let engine = ConsensusEngine::new(params, storage);

        let conflict_id = ConflictId::new("test-conflict".to_string());
        let msg1 = MessageId::new(b"msg1");
        let msg2 = MessageId::new(b"msg2");

        engine
            .conflicts
            .register_conflict(conflict_id.clone())
            .await;
        engine
            .conflicts
            .update_support(&conflict_id, msg1, 3.0, 1)
            .await
            .unwrap();
        engine
            .conflicts
            .update_support(&conflict_id, msg2, 1.0, 1)
            .await
            .unwrap();

        let winner = engine.conflicts.get_winner(&conflict_id).await;
        assert_eq!(winner, Some(msg1));

        let penalty = engine.conflicts.get_penalty(&msg2, &conflict_id).await;
        assert!(penalty > 0.0);
    }
}
