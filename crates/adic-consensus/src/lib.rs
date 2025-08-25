pub mod admissibility;
pub mod deposit;
pub mod reputation;
pub mod validation;
pub mod conflict;

pub use admissibility::{AdmissibilityChecker, AdmissibilityResult};
pub use deposit::{DepositManager, DepositState};
pub use reputation::ReputationTracker;
pub use validation::{MessageValidator, ValidationResult};
pub use conflict::{ConflictResolver, ConflictEnergy};

use adic_types::{AdicParams, AdicMessage, Result};
use std::sync::Arc;

pub struct ConsensusEngine {
    params: Arc<AdicParams>,
    admissibility: AdmissibilityChecker,
    pub deposits: DepositManager,
    pub reputation: ReputationTracker,
    validator: MessageValidator,
    conflicts: ConflictResolver,
}

impl ConsensusEngine {
    pub fn new(params: AdicParams) -> Self {
        let shared_params = Arc::new(params.clone());
        Self {
            admissibility: AdmissibilityChecker::new(params.clone()),
            deposits: DepositManager::new(params.deposit),
            reputation: ReputationTracker::new(params.gamma),
            validator: MessageValidator::new(),
            conflicts: ConflictResolver::new(),
            params: shared_params,
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

    #[test]
    fn it_works() {
        let params = AdicParams::default();
        let engine = ConsensusEngine::new(params);
        assert_eq!(engine.params().d, 3);
    }
}
