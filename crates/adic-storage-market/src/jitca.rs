//! JITCA (Just-in-Time Compiled Agreements) Compiler
//!
//! Compiles zero-value intents and acceptances into value-bearing storage deals.
//! The JITCA compiler ensures that:
//! - Intent and acceptance semantically match
//! - Both messages have reached F1 finality
//! - Funds are properly locked in escrow
//! - Provider collateral is sufficient
//! - Deal parameters satisfy market constraints

use crate::error::{Result, StorageMarketError};
use crate::types::{
    Hash, ProviderAcceptance, StorageDeal, StorageDealIntent,
    StorageDealStatus,
};
use adic_economics::{AccountAddress, AdicAmount, BalanceManager};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration for JITCA compilation
#[derive(Debug, Clone)]
pub struct JitcaConfig {
    /// Minimum collateral ratio (provider_collateral / total_payment)
    pub min_collateral_ratio: f64,

    /// Maximum compilation window after acceptance finality (epochs)
    pub max_compilation_window: u64,

    /// Activation deadline after deal compilation (epochs)
    pub activation_deadline: u64,

    /// Minimum F1 finality requirement
    pub require_f1_finality: bool,
}

impl Default for JitcaConfig {
    fn default() -> Self {
        Self {
            min_collateral_ratio: 1.0, // 1:1 collateral
            max_compilation_window: 100, // 100 epochs
            activation_deadline: 50, // 50 epochs
            require_f1_finality: true,
        }
    }
}

/// JITCA compiler for storage deals
pub struct JitcaCompiler {
    config: JitcaConfig,
    balance_manager: Arc<BalanceManager>,
    deals: Arc<RwLock<std::collections::HashMap<Hash, StorageDeal>>>,
    next_deal_id: Arc<RwLock<u64>>,
}

impl JitcaCompiler {
    /// Create new JITCA compiler
    pub fn new(config: JitcaConfig, balance_manager: Arc<BalanceManager>) -> Self {
        Self {
            config,
            balance_manager,
            deals: Arc::new(RwLock::new(std::collections::HashMap::new())),
            next_deal_id: Arc::new(RwLock::new(1)),
        }
    }

    /// Compile intent and acceptance into a storage deal
    ///
    /// This is the core JITCA operation that:
    /// 1. Validates semantic compatibility
    /// 2. Checks finality requirements
    /// 3. Locks client payment in escrow
    /// 4. Locks provider collateral
    /// 5. Creates the deal with proper state machine
    pub async fn compile(
        &self,
        intent: &StorageDealIntent,
        acceptance: &ProviderAcceptance,
        current_epoch: u64,
    ) -> Result<StorageDeal> {
        // 1. Validate semantic compatibility
        self.validate_compatibility(intent, acceptance)?;

        // 2. Check finality requirements
        self.validate_finality(intent, acceptance)?;

        // 3. Check compilation window
        self.validate_compilation_window(acceptance, current_epoch)?;

        // 4. Calculate deal economics
        let total_payment = self.calculate_total_payment(intent, acceptance)?;
        let required_collateral = self.calculate_required_collateral(&total_payment)?;

        // 5. Validate provider collateral
        if acceptance.collateral_commitment < required_collateral {
            return Err(StorageMarketError::InsufficientCollateral {
                required: required_collateral.to_string(),
                provided: acceptance.collateral_commitment.to_string(),
            });
        }

        // 6. Lock funds in escrow
        self.lock_client_payment(intent.client, total_payment)
            .await?;
        self.lock_provider_collateral(acceptance.provider, acceptance.collateral_commitment)
            .await?;

        // 7. Generate deal ID
        let deal_id = self.next_deal_id().await;

        // 8. Create the deal using the existing constructor
        let mut deal = StorageDeal::from_intent_and_acceptance(
            intent,
            acceptance,
            deal_id,
            current_epoch,
            self.config.activation_deadline,
        );

        // 9. Update status to PendingActivation after compilation
        deal.transition_to(StorageDealStatus::PendingActivation)?;

        // 10. Store the deal
        self.deals.write().await.insert(intent.intent_id, deal.clone());

        Ok(deal)
    }

    /// Validate that intent and acceptance are semantically compatible
    fn validate_compatibility(
        &self,
        intent: &StorageDealIntent,
        acceptance: &ProviderAcceptance,
    ) -> Result<()> {
        // Check that acceptance references the intent
        if acceptance.ref_intent != intent.intent_id {
            return Err(StorageMarketError::InvalidMessage(
                "Acceptance does not reference the intent".to_string(),
            ));
        }

        // Check price acceptability (must be <= max_price_per_epoch)
        if acceptance.offered_price_per_epoch > intent.max_price_per_epoch {
            return Err(StorageMarketError::InvalidMessage(format!(
                "Offered price {} exceeds maximum price {}",
                acceptance.offered_price_per_epoch, intent.max_price_per_epoch
            )));
        }

        Ok(())
    }

    /// Validate finality requirements
    fn validate_finality(
        &self,
        intent: &StorageDealIntent,
        acceptance: &ProviderAcceptance,
    ) -> Result<()> {
        if !self.config.require_f1_finality {
            return Ok(());
        }

        // Check intent finality
        if !intent.finality_status.is_finalized() {
            return Err(StorageMarketError::FinalityNotReached(
                "Intent has not reached F1 finality".to_string(),
            ));
        }

        // Check acceptance finality
        if !acceptance.finality_status.is_finalized() {
            return Err(StorageMarketError::FinalityNotReached(
                "Acceptance has not reached F1 finality".to_string(),
            ));
        }

        Ok(())
    }

    /// Validate compilation window
    fn validate_compilation_window(
        &self,
        acceptance: &ProviderAcceptance,
        current_epoch: u64,
    ) -> Result<()> {
        if let Some(finalized_epoch) = acceptance.finalized_at_epoch {
            let elapsed = current_epoch.saturating_sub(finalized_epoch);
            if elapsed > self.config.max_compilation_window {
                return Err(StorageMarketError::Other(format!(
                    "Compilation window exceeded: {} epochs since finality",
                    elapsed
                )));
            }
        }
        Ok(())
    }

    /// Calculate total payment for the deal
    fn calculate_total_payment(
        &self,
        intent: &StorageDealIntent,
        acceptance: &ProviderAcceptance,
    ) -> Result<AdicAmount> {
        let price_base = acceptance.offered_price_per_epoch.to_base_units();
        let duration = intent.duration_epochs;
        let total_base = price_base.checked_mul(duration).ok_or_else(|| {
            StorageMarketError::Other("Payment calculation overflow".to_string())
        })?;
        Ok(AdicAmount::from_base_units(total_base))
    }

    /// Calculate required provider collateral
    fn calculate_required_collateral(&self, total_payment: &AdicAmount) -> Result<AdicAmount> {
        let payment_base = total_payment.to_base_units();
        let collateral_base = (payment_base as f64 * self.config.min_collateral_ratio) as u64;
        Ok(AdicAmount::from_base_units(collateral_base))
    }

    /// Lock client payment in escrow
    async fn lock_client_payment(
        &self,
        client: AccountAddress,
        amount: AdicAmount,
    ) -> Result<()> {
        self.balance_manager
            .debit(client, amount)
            .await
            .map_err(|e| StorageMarketError::EconomicsError(e.to_string()))
    }

    /// Lock provider collateral in escrow
    async fn lock_provider_collateral(
        &self,
        provider: AccountAddress,
        amount: AdicAmount,
    ) -> Result<()> {
        self.balance_manager
            .debit(provider, amount)
            .await
            .map_err(|e| StorageMarketError::EconomicsError(e.to_string()))
    }

    /// Get next deal ID
    async fn next_deal_id(&self) -> u64 {
        let mut id = self.next_deal_id.write().await;
        let current = *id;
        *id += 1;
        current
    }

    /// Get a compiled deal by intent ID
    pub async fn get_deal(&self, intent_id: &Hash) -> Option<StorageDeal> {
        self.deals.read().await.get(intent_id).cloned()
    }

    /// Activate a compiled deal (provider confirms data receipt)
    pub async fn activate_deal(
        &self,
        deal_id: u64,
        current_epoch: u64,
    ) -> Result<()> {
        let mut deals = self.deals.write().await;

        // Find the deal by ID
        let deal = deals
            .values_mut()
            .find(|d| d.deal_id == deal_id)
            .ok_or_else(|| StorageMarketError::DealNotFound(deal_id.to_string()))?;

        // Check current status
        if deal.status != StorageDealStatus::PendingActivation {
            return Err(StorageMarketError::InvalidStateTransition {
                from: format!("{:?}", deal.status),
                to: "Active".to_string(),
            });
        }

        // Check activation deadline
        if current_epoch > deal.activation_deadline {
            return Err(StorageMarketError::ActivationDeadlineExceeded {
                deadline: deal.activation_deadline,
                current: current_epoch,
            });
        }

        // Activate the deal
        deal.transition_to(StorageDealStatus::Active)?;
        deal.start_epoch = Some(current_epoch);

        Ok(())
    }

    /// Get all deals for a client
    pub async fn get_client_deals(&self, client: AccountAddress) -> Vec<StorageDeal> {
        self.deals
            .read()
            .await
            .values()
            .filter(|d| d.client == client)
            .cloned()
            .collect()
    }

    /// Get all deals for a provider
    pub async fn get_provider_deals(&self, provider: AccountAddress) -> Vec<StorageDeal> {
        self.deals
            .read()
            .await
            .values()
            .filter(|d| d.provider == provider)
            .cloned()
            .collect()
    }

    /// Get deal statistics
    pub async fn get_stats(&self) -> JitcaStats {
        let deals = self.deals.read().await;

        let mut published = 0;
        let mut pending_activation = 0;
        let mut active = 0;
        let mut completed = 0;
        let mut failed = 0;

        for deal in deals.values() {
            match deal.status {
                StorageDealStatus::Published => published += 1,
                StorageDealStatus::PendingActivation => pending_activation += 1,
                StorageDealStatus::Active => active += 1,
                StorageDealStatus::Completed => completed += 1,
                StorageDealStatus::Failed => failed += 1,
                _ => {}
            }
        }

        JitcaStats {
            total_deals: deals.len(),
            published_deals: published,
            pending_activation_deals: pending_activation,
            active_deals: active,
            completed_deals: completed,
            failed_deals: failed,
        }
    }
}

/// JITCA statistics
#[derive(Debug, Clone)]
pub struct JitcaStats {
    pub total_deals: usize,
    pub published_deals: usize,
    pub pending_activation_deals: usize,
    pub active_deals: usize,
    pub completed_deals: usize,
    pub failed_deals: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StorageDealIntent;

    fn create_test_intent() -> StorageDealIntent {
        use crate::types::{FinalityStatus, SettlementRail};

        let mut intent = StorageDealIntent::new(
            AccountAddress::from_bytes([1u8; 32]),
            [0u8; 32],   // data_cid
            1024 * 1024, // 1 MB data_size
            100,         // 100 epochs duration
            AdicAmount::from_adic(1.0), // max price per epoch
            1,           // required redundancy
            vec![SettlementRail::AdicNative],
        );
        intent.finality_status = FinalityStatus::F1Complete {
            k: 3,
            depth: 10,
            rep_weight: 100.0,
        };
        intent.finalized_at_epoch = Some(50);
        intent
    }

    fn create_test_acceptance(intent: &StorageDealIntent) -> ProviderAcceptance {
        use crate::types::FinalityStatus;

        let mut acceptance = ProviderAcceptance::new(
            intent.intent_id,
            AccountAddress::from_bytes([2u8; 32]),
            AdicAmount::from_adic(0.5), // offered price per epoch
            AdicAmount::from_adic(50.0), // collateral commitment
            80.0, // provider reputation
        );
        acceptance.finality_status = FinalityStatus::F1Complete {
            k: 3,
            depth: 10,
            rep_weight: 100.0,
        };
        acceptance.finalized_at_epoch = Some(55);
        acceptance
    }

    #[tokio::test]
    async fn test_compile_deal() {
        let storage = Arc::new(adic_economics::storage::MemoryStorage::new());
        let balance_manager = Arc::new(BalanceManager::new(storage));
        let compiler = JitcaCompiler::new(JitcaConfig::default(), balance_manager.clone());

        let intent = create_test_intent();
        let acceptance = create_test_acceptance(&intent);

        // Fund the accounts
        balance_manager
            .credit(intent.client, AdicAmount::from_adic(100.0))
            .await
            .unwrap();
        balance_manager
            .credit(acceptance.provider, AdicAmount::from_adic(100.0))
            .await
            .unwrap();

        // Compile the deal
        let deal = compiler.compile(&intent, &acceptance, 60).await.unwrap();

        assert_eq!(deal.client, intent.client);
        assert_eq!(deal.provider, acceptance.provider);
        assert_eq!(deal.status, StorageDealStatus::PendingActivation);
        assert_eq!(deal.data_cid, intent.data_cid);
    }

    #[tokio::test]
    async fn test_compile_mismatched_intent() {
        let storage = Arc::new(adic_economics::storage::MemoryStorage::new());
        let balance_manager = Arc::new(BalanceManager::new(storage));
        let compiler = JitcaCompiler::new(JitcaConfig::default(), balance_manager);

        let intent = create_test_intent();
        let mut acceptance = create_test_acceptance(&intent);

        // Mismatch the intent reference
        acceptance.ref_intent = [9u8; 32];

        let result = compiler.compile(&intent, &acceptance, 60).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_activate_deal() {
        let storage = Arc::new(adic_economics::storage::MemoryStorage::new());
        let balance_manager = Arc::new(BalanceManager::new(storage));
        let compiler = JitcaCompiler::new(JitcaConfig::default(), balance_manager.clone());

        let intent = create_test_intent();
        let acceptance = create_test_acceptance(&intent);

        // Fund the accounts
        balance_manager
            .credit(intent.client, AdicAmount::from_adic(100.0))
            .await
            .unwrap();
        balance_manager
            .credit(acceptance.provider, AdicAmount::from_adic(100.0))
            .await
            .unwrap();

        // Compile the deal
        let deal = compiler.compile(&intent, &acceptance, 60).await.unwrap();

        // Activate the deal
        compiler.activate_deal(deal.deal_id, 65).await.unwrap();

        // Verify activation
        let updated_deal = compiler.get_deal(&intent.intent_id).await.unwrap();
        assert_eq!(updated_deal.status, StorageDealStatus::Active);
        assert_eq!(updated_deal.start_epoch, Some(65));
    }

    #[tokio::test]
    async fn test_activation_deadline_exceeded() {
        let storage = Arc::new(adic_economics::storage::MemoryStorage::new());
        let balance_manager = Arc::new(BalanceManager::new(storage));
        let compiler = JitcaCompiler::new(JitcaConfig::default(), balance_manager.clone());

        let intent = create_test_intent();
        let acceptance = create_test_acceptance(&intent);

        // Fund the accounts
        balance_manager
            .credit(intent.client, AdicAmount::from_adic(100.0))
            .await
            .unwrap();
        balance_manager
            .credit(acceptance.provider, AdicAmount::from_adic(100.0))
            .await
            .unwrap();

        // Compile the deal at epoch 60
        let deal = compiler.compile(&intent, &acceptance, 60).await.unwrap();

        // Try to activate after deadline (60 + 50 = 110)
        let result = compiler.activate_deal(deal.deal_id, 111).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_client_deals() {
        let storage = Arc::new(adic_economics::storage::MemoryStorage::new());
        let balance_manager = Arc::new(BalanceManager::new(storage));
        let compiler = JitcaCompiler::new(JitcaConfig::default(), balance_manager.clone());

        let intent = create_test_intent();
        let acceptance = create_test_acceptance(&intent);

        // Fund the accounts
        balance_manager
            .credit(intent.client, AdicAmount::from_adic(100.0))
            .await
            .unwrap();
        balance_manager
            .credit(acceptance.provider, AdicAmount::from_adic(100.0))
            .await
            .unwrap();

        // Compile a deal
        compiler.compile(&intent, &acceptance, 60).await.unwrap();

        // Get client deals
        let deals = compiler.get_client_deals(intent.client).await;
        assert_eq!(deals.len(), 1);
        assert_eq!(deals[0].client, intent.client);
    }
}
