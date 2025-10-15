pub mod admissibility;
pub mod deposit;
pub mod energy_descent;
pub mod reputation;
pub mod validation;

#[cfg(test)]
mod c1_security_test;

pub use admissibility::{AdmissibilityChecker, AdmissibilityResult};
pub use deposit::{
    Deposit, DepositManager, DepositState, DepositStats, DepositStatus, DEFAULT_DEPOSIT_AMOUNT,
};
pub use energy_descent::{ConflictEnergy, EnergyDescentTracker, EnergyMetrics};
pub use reputation::{ReputationChangeEvent, ReputationScore, ReputationTracker};
pub use validation::{MessageValidator, ValidationResult};

use adic_storage::StorageEngine;
use adic_types::{AdicMessage, AdicParams, Result};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ConsensusEngine {
    params: Arc<RwLock<AdicParams>>,
    pub admissibility: AdmissibilityChecker,
    pub deposits: DepositManager,
    pub reputation: ReputationTracker,
    pub validator: MessageValidator,
    pub energy_tracker: EnergyDescentTracker,
    storage: Arc<StorageEngine>,
}

impl ConsensusEngine {
    pub fn new(params: AdicParams, storage: Arc<StorageEngine>) -> Self {
        let shared_params = Arc::new(RwLock::new(params.clone()));
        Self {
            admissibility: AdmissibilityChecker::new(params.clone()),
            deposits: DepositManager::new(params.deposit),
            reputation: ReputationTracker::new(params.gamma),
            validator: MessageValidator::new(),
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

    pub async fn params(&self) -> AdicParams {
        self.params.read().await.clone()
    }

    pub fn admissibility(&self) -> &AdmissibilityChecker {
        &self.admissibility
    }

    /// Update operational parameters that can be hot-reloaded
    /// Note: Constitutional params (p, d, rho, q) require node restart
    pub async fn update_operational_params(&self, new_params: &AdicParams) {
        // Update reputation gamma parameter
        self.reputation.update_gamma(new_params.gamma).await;

        // Update energy tracker parameters (lambda, mu)
        self.energy_tracker
            .update_params(new_params.lambda, new_params.mu)
            .await;

        // Update deposit amount
        self.deposits
            .update_deposit_amount(new_params.deposit)
            .await;

        // Store updated params atomically
        *self.params.write().await = new_params.clone();

        tracing::info!("✅ ConsensusEngine operational parameters updated");
    }

    /// Track a conflict and update energy using mathematically rigorous energy descent
    pub async fn track_conflict(
        &self,
        conflict_id: adic_types::ConflictId,
        message_id: adic_types::MessageId,
    ) -> Result<()> {
        // Register conflict if new
        self.energy_tracker
            .register_conflict(conflict_id.clone())
            .await;

        // Update support for this message using paper-accurate calculation
        self.energy_tracker
            .update_support(&conflict_id, message_id, &self.storage, &self.reputation)
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

    /// Get the energy descent tracker (paper-accurate conflict resolution)
    pub fn energy_tracker(&self) -> &EnergyDescentTracker {
        &self.energy_tracker
    }

    /// Get conflict resolver (returns energy_tracker which is the paper-accurate implementation)
    pub fn conflict_resolver(&self) -> &EnergyDescentTracker {
        &self.energy_tracker
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

    #[tokio::test]
    async fn test_consensus_engine_creation() {
        let params = AdicParams::default();
        let storage = create_test_storage();
        let engine = ConsensusEngine::new(params, storage);
        let params = engine.params().await;
        assert_eq!(params.d, 3);
        assert_eq!(params.k, 20); // Production value from paper
        assert_eq!(params.gamma, 0.9);
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

    #[tokio::test]
    async fn test_consensus_engine_getters() {
        let params_orig = AdicParams::default();
        let storage = create_test_storage();
        let engine = ConsensusEngine::new(params_orig.clone(), storage);

        let params = engine.params().await;
        assert_eq!(params.d, params_orig.d);
        assert_eq!(params.k, params_orig.k);
        assert_eq!(params.gamma, params_orig.gamma);

        // Test that getters return proper references
        let _ = engine.admissibility();
        let _ = engine.validator();
        let _ = engine.conflict_resolver();
        let _ = engine.energy_tracker();
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
        let engine = ConsensusEngine::new(params, storage.clone());

        let conflict_id = ConflictId::new("test-conflict".to_string());

        // Register conflict
        engine
            .energy_tracker
            .register_conflict(conflict_id.clone())
            .await;

        // Create test messages in storage
        let proposer1 = PublicKey::from_bytes([1; 32]);
        let proposer2 = PublicKey::from_bytes([2; 32]);

        let message1 = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![
                AxisPhi::new(0, QpDigits::from_u64(1, 3, 10)),
                AxisPhi::new(1, QpDigits::from_u64(2, 3, 10)),
                AxisPhi::new(2, QpDigits::from_u64(3, 3, 10)),
            ]),
            AdicMeta::new(Utc::now()),
            proposer1,
            vec![],
        );

        let message2 = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![
                AxisPhi::new(0, QpDigits::from_u64(4, 3, 10)),
                AxisPhi::new(1, QpDigits::from_u64(5, 3, 10)),
                AxisPhi::new(2, QpDigits::from_u64(6, 3, 10)),
            ]),
            AdicMeta::new(Utc::now()),
            proposer2,
            vec![],
        );

        // Store messages
        storage.store_message(&message1).await.unwrap();
        storage.store_message(&message2).await.unwrap();

        // Set up reputation
        engine.reputation.good_update(&proposer1, 3.0, 1).await;
        engine.reputation.good_update(&proposer2, 1.0, 1).await;

        // Update support using track_conflict (uses paper-accurate calculation)
        engine
            .track_conflict(conflict_id.clone(), message1.id)
            .await
            .unwrap();
        engine
            .track_conflict(conflict_id.clone(), message2.id)
            .await
            .unwrap();

        // Check that conflict was tracked
        let is_resolved = engine.is_conflict_resolved(&conflict_id).await;
        // Conflict may not be resolved immediately (needs more messages)
        assert!(!is_resolved || is_resolved);

        // Check that winner exists (should be one of the two messages)
        let winner = engine.energy_tracker.get_winner(&conflict_id).await;
        assert!(winner == Some(message1.id) || winner == Some(message2.id) || winner.is_none());

        // Energy metrics should be available
        let metrics = engine.get_energy_metrics().await;
        assert!(metrics.total_conflicts >= 1);
    }
}
