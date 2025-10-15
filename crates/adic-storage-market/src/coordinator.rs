//! Storage Market Coordinator
//!
//! Orchestrates the complete storage market lifecycle:
//! 1. Intent publication and discovery
//! 2. Provider acceptance matching
//! 3. Deal compilation via JITCA
//! 4. Deal activation
//! 5. Proof cycle management
//! 6. Payment settlement and completion

use crate::error::{Result, StorageMarketError};
use crate::intent::{IntentConfig, IntentManager};
use crate::jitca::{JitcaCompiler, JitcaConfig};
use crate::proof::{ProofCycleConfig, ProofCycleManager};
use crate::types::{
    DealActivation, FinalityStatus, ProviderAcceptance, StorageDeal, StorageDealIntent,
    StorageDealStatus, StorageProof,
};
use adic_economics::{AccountAddress, BalanceManager};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration for the storage market coordinator
#[derive(Debug, Clone)]
pub struct MarketConfig {
    pub intent_config: IntentConfig,
    pub jitca_config: JitcaConfig,
    pub proof_config: ProofCycleConfig,
}

impl Default for MarketConfig {
    fn default() -> Self {
        Self {
            intent_config: IntentConfig::default(),
            jitca_config: JitcaConfig::default(),
            proof_config: ProofCycleConfig::default(),
        }
    }
}

/// Central coordinator for the storage market
pub struct StorageMarketCoordinator {
    #[allow(dead_code)]
    config: MarketConfig,
    intent_manager: Arc<IntentManager>,
    jitca_compiler: Arc<JitcaCompiler>,
    pub proof_manager: Arc<ProofCycleManager>,
    pub balance_manager: Arc<BalanceManager>,

    // Track active deals
    active_deals: Arc<RwLock<HashMap<u64, StorageDeal>>>,

    // Track current epoch
    current_epoch: Arc<RwLock<u64>>,
}

impl StorageMarketCoordinator {
    /// Create new storage market coordinator
    pub fn new(
        config: MarketConfig,
        intent_manager: Arc<IntentManager>,
        jitca_compiler: Arc<JitcaCompiler>,
        proof_manager: Arc<ProofCycleManager>,
        balance_manager: Arc<BalanceManager>,
    ) -> Self {
        Self {
            config,
            intent_manager,
            jitca_compiler,
            proof_manager,
            balance_manager,
            active_deals: Arc::new(RwLock::new(HashMap::new())),
            current_epoch: Arc::new(RwLock::new(0)),
        }
    }

    /// Advance the current epoch
    pub async fn advance_epoch(&self) -> u64 {
        let mut epoch = self.current_epoch.write().await;
        *epoch += 1;
        *epoch
    }

    /// Get current epoch
    pub async fn get_current_epoch(&self) -> u64 {
        *self.current_epoch.read().await
    }

    /// Client publishes a storage intent
    pub async fn publish_intent(&self, intent: StorageDealIntent) -> Result<[u8; 32]> {
        // Validate client has sufficient balance for deposit
        let balance = self
            .balance_manager
            .get_balance(intent.client)
            .await
            .map_err(|e| StorageMarketError::EconomicsError(e.to_string()))?;

        if balance < intent.deposit {
            return Err(StorageMarketError::InsufficientFunds {
                required: intent.deposit.to_string(),
                available: balance.to_string(),
            });
        }

        // Publish intent through intent manager
        let intent_id = self.intent_manager.publish_intent(intent).await?;

        tracing::info!(
            "Intent published: {}",
            hex::encode(&intent_id)
        );

        Ok(intent_id)
    }

    /// Provider submits an acceptance for an intent
    pub async fn submit_acceptance(
        &self,
        acceptance: ProviderAcceptance,
    ) -> Result<[u8; 32]> {
        // Validate provider has sufficient collateral
        let balance = self
            .balance_manager
            .get_balance(acceptance.provider)
            .await
            .map_err(|e| StorageMarketError::EconomicsError(e.to_string()))?;

        if balance < acceptance.collateral_commitment {
            return Err(StorageMarketError::InsufficientFunds {
                required: acceptance.collateral_commitment.to_string(),
                available: balance.to_string(),
            });
        }

        // Submit acceptance through intent manager
        let acceptance_id = self
            .intent_manager
            .submit_acceptance(acceptance)
            .await?;

        tracing::info!(
            "Acceptance submitted: {}",
            hex::encode(&acceptance_id)
        );

        Ok(acceptance_id)
    }

    /// Compile a deal from finalized intent and acceptance
    pub async fn compile_deal(
        &self,
        intent_id: &[u8; 32],
        acceptance_id: &[u8; 32],
    ) -> Result<u64> {
        let current_epoch = self.get_current_epoch().await;

        // Get the finalized intent
        let intent = self
            .intent_manager
            .get_intent(intent_id)
            .await
            .ok_or_else(|| {
                StorageMarketError::Other(format!(
                    "Intent not found: {}",
                    hex::encode(intent_id)
                ))
            })?;

        // Get the finalized acceptance
        let acceptance = self
            .intent_manager
            .get_acceptance(acceptance_id)
            .await
            .ok_or_else(|| {
                StorageMarketError::Other(format!(
                    "Acceptance not found: {}",
                    hex::encode(acceptance_id)
                ))
            })?;

        // Compile the deal
        let deal = self
            .jitca_compiler
            .compile(&intent, &acceptance, current_epoch)
            .await?;

        let deal_id = deal.deal_id;

        // Store the deal
        self.active_deals.write().await.insert(deal_id, deal);

        tracing::info!(
            "Deal compiled: {} (intent: {}, acceptance: {})",
            deal_id,
            hex::encode(intent_id),
            hex::encode(acceptance_id)
        );

        Ok(deal_id)
    }

    /// Provider activates a deal after receiving data
    pub async fn activate_deal(
        &self,
        deal_id: u64,
        activation: DealActivation,
    ) -> Result<()> {
        let current_epoch = self.get_current_epoch().await;

        // Get the deal
        let mut deals = self.active_deals.write().await;
        let deal = deals.get_mut(&deal_id).ok_or_else(|| {
            StorageMarketError::DealNotFound(deal_id.to_string())
        })?;

        // Verify deal is in correct state
        if deal.status != StorageDealStatus::PendingActivation {
            return Err(StorageMarketError::InvalidStateTransition {
                from: format!("{:?}", deal.status),
                to: "Active".to_string(),
            });
        }

        // Set Merkle root from activation
        deal.proof_merkle_root = Some(activation.data_merkle_root);

        // Activate through JITCA compiler
        drop(deals); // Release lock before calling compiler
        self.jitca_compiler
            .activate_deal(deal_id, current_epoch)
            .await?;

        // Update local copy
        let mut deals = self.active_deals.write().await;
        if let Some(deal) = deals.get_mut(&deal_id) {
            deal.transition_to(StorageDealStatus::Active)?;
            deal.start_epoch = Some(current_epoch);
        }

        tracing::info!(
            "Deal activated: {} at epoch {}",
            deal_id,
            current_epoch
        );

        Ok(())
    }

    /// Provider submits a storage proof
    pub async fn submit_proof(&self, proof: StorageProof) -> Result<()> {
        let current_epoch = self.get_current_epoch().await;

        // Get the deal
        let mut deals = self.active_deals.write().await;
        let deal = deals.get_mut(&proof.deal_id).ok_or_else(|| {
            StorageMarketError::DealNotFound(proof.deal_id.to_string())
        })?;

        // Verify deal is active
        if deal.status != StorageDealStatus::Active {
            return Err(StorageMarketError::InvalidStateTransition {
                from: format!("{:?}", deal.status),
                to: "Proof submission".to_string(),
            });
        }

        // Generate and verify challenges
        let challenges = self
            .proof_manager
            .generate_challenge(deal, proof.proof_epoch, current_epoch)
            .await?;

        // Verify proof matches challenges
        if proof.challenge_indices != challenges {
            return Err(StorageMarketError::ProofVerificationFailed(
                "Proof challenges do not match expected challenges".to_string(),
            ));
        }

        // Submit proof for verification
        self.proof_manager
            .submit_proof(proof.clone(), deal, current_epoch)
            .await?;

        tracing::info!(
            "Proof submitted for deal {} at epoch {}",
            proof.deal_id,
            current_epoch
        );

        Ok(())
    }

    /// Check all active deals for missed proofs
    pub async fn check_proof_deadlines(&self) -> Result<Vec<u64>> {
        let current_epoch = self.get_current_epoch().await;
        let mut slashed_deals = Vec::new();

        let mut deals = self.active_deals.write().await;

        for (deal_id, deal) in deals.iter_mut() {
            if deal.status == StorageDealStatus::Active {
                // Check for missed proofs
                if let Err(e) = self
                    .proof_manager
                    .check_missed_proofs(deal, current_epoch)
                    .await
                {
                    tracing::warn!(
                        "Deal {} check failed: {}",
                        deal_id,
                        e
                    );
                }

                // If deal was slashed, track it
                if deal.status == StorageDealStatus::Failed {
                    slashed_deals.push(*deal_id);
                }
            }
        }

        if !slashed_deals.is_empty() {
            tracing::warn!(
                "Slashed {} deals at epoch {}: {:?}",
                slashed_deals.len(),
                current_epoch,
                slashed_deals
            );
        }

        Ok(slashed_deals)
    }

    /// Complete a deal after duration elapsed
    pub async fn complete_deal(&self, deal_id: u64) -> Result<()> {
        let current_epoch = self.get_current_epoch().await;

        let mut deals = self.active_deals.write().await;
        let deal = deals.get_mut(&deal_id).ok_or_else(|| {
            StorageMarketError::DealNotFound(deal_id.to_string())
        })?;

        // Complete through proof manager
        self.proof_manager
            .complete_deal(deal, current_epoch)
            .await?;

        tracing::info!(
            "Deal completed: {} at epoch {}",
            deal_id,
            current_epoch
        );

        Ok(())
    }

    /// Get market statistics
    pub async fn get_market_stats(&self) -> MarketStats {
        let intent_stats = self.intent_manager.get_stats().await;
        let jitca_stats = self.jitca_compiler.get_stats().await;

        let deals = self.active_deals.read().await;
        let active_deals = deals
            .values()
            .filter(|d| d.status == StorageDealStatus::Active)
            .count();

        let pending_activation = deals
            .values()
            .filter(|d| d.status == StorageDealStatus::PendingActivation)
            .count();

        let completed_deals = deals
            .values()
            .filter(|d| d.status == StorageDealStatus::Completed)
            .count();

        let failed_deals = deals
            .values()
            .filter(|d| d.status == StorageDealStatus::Failed)
            .count();

        MarketStats {
            total_intents: intent_stats.total_intents as usize,
            finalized_intents: intent_stats.finalized_intents as usize,
            total_acceptances: intent_stats.total_acceptances as usize,
            compiled_deals: jitca_stats.total_deals,
            active_deals,
            pending_activation,
            completed_deals,
            failed_deals,
            current_epoch: *self.current_epoch.read().await,
        }
    }

    /// Get a specific deal
    pub async fn get_deal(&self, deal_id: u64) -> Option<StorageDeal> {
        self.active_deals.read().await.get(&deal_id).cloned()
    }

    /// Get all deals for a client
    pub async fn get_client_deals(&self, client: AccountAddress) -> Vec<StorageDeal> {
        self.active_deals
            .read()
            .await
            .values()
            .filter(|d| d.client == client)
            .cloned()
            .collect()
    }

    /// Get all deals for a provider
    pub async fn get_provider_deals(&self, provider: AccountAddress) -> Vec<StorageDeal> {
        self.active_deals
            .read()
            .await
            .values()
            .filter(|d| d.provider == provider)
            .cloned()
            .collect()
    }

    /// Get the number of acceptances for a given provider
    pub async fn get_provider_acceptance_count(&self, provider: AccountAddress) -> usize {
        self.intent_manager.get_provider_acceptance_count(provider).await
    }

    /// Get the number of intents for a given client
    pub async fn get_client_intent_count(&self, client: AccountAddress) -> usize {
        self.intent_manager.get_client_intent_count(client).await
    }

    /// Update intent finality status (for testing/simulation)
    pub async fn update_intent_finality(
        &self,
        intent_id: &[u8; 32],
        finality_status: FinalityStatus,
        finalized_at_epoch: u64,
    ) -> Result<()> {
        self.intent_manager
            .update_intent_finality(intent_id, finality_status, finalized_at_epoch)
            .await
    }

    /// Update acceptance finality status (for testing/simulation)
    pub async fn update_acceptance_finality(
        &self,
        acceptance_id: &[u8; 32],
        finality_status: FinalityStatus,
        finalized_at_epoch: u64,
    ) -> Result<()> {
        self.intent_manager
            .update_acceptance_finality(acceptance_id, finality_status, finalized_at_epoch)
            .await
    }

    // ========== Intent/Acceptance Query Methods ==========

    /// Get all finalized intents (for provider discovery)
    pub async fn get_finalized_intents(&self) -> Vec<StorageDealIntent> {
        self.intent_manager.get_finalized_intents().await
    }

    /// Get a specific intent by ID
    pub async fn get_intent(&self, intent_id: &[u8; 32]) -> Option<StorageDealIntent> {
        self.intent_manager.get_intent(intent_id).await
    }

    /// Get a specific acceptance by ID
    pub async fn get_acceptance(&self, acceptance_id: &[u8; 32]) -> Option<ProviderAcceptance> {
        self.intent_manager.get_acceptance(acceptance_id).await
    }

    /// Get all acceptances for a specific intent
    pub async fn get_acceptances_for_intent(
        &self,
        intent_id: &[u8; 32],
    ) -> Result<Vec<ProviderAcceptance>> {
        self.intent_manager
            .get_acceptances_for_intent(intent_id)
            .await
    }
}

/// Market-wide statistics
#[derive(Debug, Clone)]
pub struct MarketStats {
    pub total_intents: usize,
    pub finalized_intents: usize,
    pub total_acceptances: usize,
    pub compiled_deals: usize,
    pub active_deals: usize,
    pub pending_activation: usize,
    pub completed_deals: usize,
    pub failed_deals: usize,
    pub current_epoch: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SettlementRail;
    use adic_challenges::{ChallengeConfig, ChallengeWindowManager};
    use adic_consensus::ReputationTracker;
    use adic_economics::AdicAmount;
    use adic_vrf::VRFService;

    async fn create_test_coordinator() -> StorageMarketCoordinator {
        let storage = Arc::new(adic_economics::storage::MemoryStorage::new());
        let balance_manager = Arc::new(BalanceManager::new(storage));

        let rep_tracker = Arc::new(ReputationTracker::new(0.9));

        let mut intent_config = IntentConfig::default();
        intent_config.min_client_reputation = 0.0; // Disable reputation check for tests

        let intent_manager = Arc::new(IntentManager::new(
            intent_config,
            balance_manager.clone(),
            rep_tracker.clone(),
        ));

        let jitca_compiler = Arc::new(JitcaCompiler::new(
            JitcaConfig::default(),
            balance_manager.clone(),
        ));

        let challenge_manager = Arc::new(ChallengeWindowManager::new(
            ChallengeConfig::default(),
        ));
        let vrf_service = Arc::new(VRFService::new(
            Default::default(),
            rep_tracker,
        ));

        let proof_manager = Arc::new(ProofCycleManager::new_for_testing(
            ProofCycleConfig::default(),
            balance_manager.clone(),
            challenge_manager,
            vrf_service,
        ));

        StorageMarketCoordinator::new(
            MarketConfig::default(),
            intent_manager,
            jitca_compiler,
            proof_manager,
            balance_manager,
        )
    }

    #[tokio::test]
    async fn test_publish_intent() {
        let coordinator = create_test_coordinator().await;

        let client = AccountAddress::from_bytes([1u8; 32]);

        // Fund the client
        coordinator
            .balance_manager
            .credit(client, AdicAmount::from_adic(100.0))
            .await
            .unwrap();

        let mut intent = StorageDealIntent::new(
            client,
            [0u8; 32],
            1024 * 1024,
            100,
            AdicAmount::from_adic(1.0),
            1,
            vec![SettlementRail::AdicNative],
        );
        intent.deposit = AdicAmount::from_adic(0.1);

        let intent_id = coordinator.publish_intent(intent).await.unwrap();
        assert_ne!(intent_id, [0u8; 32]);
    }

    #[tokio::test]
    async fn test_market_stats() {
        let coordinator = create_test_coordinator().await;

        let stats = coordinator.get_market_stats().await;
        assert_eq!(stats.total_intents, 0);
        assert_eq!(stats.current_epoch, 0);
    }

    #[tokio::test]
    async fn test_advance_epoch() {
        let coordinator = create_test_coordinator().await;

        assert_eq!(coordinator.get_current_epoch().await, 0);

        coordinator.advance_epoch().await;
        assert_eq!(coordinator.get_current_epoch().await, 1);

        coordinator.advance_epoch().await;
        assert_eq!(coordinator.get_current_epoch().await, 2);
    }
}
