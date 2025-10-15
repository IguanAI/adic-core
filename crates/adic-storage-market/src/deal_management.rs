//! Deal Management
//!
//! This module implements Sprint 8 from storage-market-design.md:
//! - Deal renewal with extended duration
//! - Early termination with refund calculations
//! - Deal migration between providers
//! - Deal parameter updates

use crate::error::{Result, StorageMarketError};
use crate::types::{FinalityStatus, Hash, MessageHash, StorageDeal, StorageDealStatus};
use adic_economics::types::{AccountAddress, AdicAmount};
use adic_types::Signature;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Deal renewal message (extends deal duration)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DealRenewal {
    // ========== Protocol Fields ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Renewal Fields ==========
    /// Deal being renewed
    pub deal_id: u64,
    /// Client requesting renewal
    pub client: AccountAddress,
    /// Provider accepting renewal
    pub provider: AccountAddress,
    /// Additional epochs to extend
    pub additional_epochs: u64,
    /// Additional payment for extension
    pub additional_payment: AdicAmount,
    /// Additional collateral (if needed)
    pub additional_collateral: AdicAmount,
    /// New price per epoch (if different)
    pub new_price_per_epoch: Option<AdicAmount>,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
    pub renewal_id: Hash,
}

impl DealRenewal {
    pub fn new(
        deal_id: u64,
        client: AccountAddress,
        provider: AccountAddress,
        additional_epochs: u64,
        additional_payment: AdicAmount,
        additional_collateral: AdicAmount,
        new_price_per_epoch: Option<AdicAmount>,
    ) -> Self {
        let now = chrono::Utc::now().timestamp();
        let mut renewal_data = Vec::new();
        renewal_data.extend_from_slice(&deal_id.to_le_bytes());
        renewal_data.extend_from_slice(client.as_bytes());
        renewal_data.extend_from_slice(&additional_epochs.to_le_bytes());
        renewal_data.extend_from_slice(&now.to_le_bytes());

        Self {
            approvals: [MessageHash::default(); 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: now,
            deal_id,
            client,
            provider,
            additional_epochs,
            additional_payment,
            additional_collateral,
            new_price_per_epoch,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
            renewal_id: blake3::hash(&renewal_data).into(),
        }
    }

    pub fn is_finalized(&self) -> bool {
        self.finality_status.is_finalized()
    }
}

/// Deal termination message (early exit)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DealTermination {
    // ========== Protocol Fields ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Termination Fields ==========
    /// Deal being terminated
    pub deal_id: u64,
    /// Party initiating termination
    pub initiator: AccountAddress,
    /// Reason for termination
    pub reason: TerminationReason,
    /// Epoch when termination is requested
    pub termination_epoch: u64,
    /// Client refund amount (calculated)
    pub client_refund: AdicAmount,
    /// Provider payment amount (calculated)
    pub provider_payment: AdicAmount,
    /// Collateral return amount (calculated)
    pub collateral_return: AdicAmount,
    /// Penalty amount (if applicable)
    pub penalty: AdicAmount,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
    pub termination_id: Hash,
}

/// Reasons for deal termination
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminationReason {
    /// Client no longer needs storage
    ClientRequest,
    /// Provider cannot continue service
    ProviderRequest,
    /// Mutual agreement
    MutualAgreement,
    /// Contract breach (provider failed proofs)
    ProviderBreach,
    /// Contract breach (client payment issues)
    ClientBreach,
    /// Data corruption detected
    DataCorruption,
}

impl DealTermination {
    pub fn new(
        deal_id: u64,
        initiator: AccountAddress,
        reason: TerminationReason,
        termination_epoch: u64,
        client_refund: AdicAmount,
        provider_payment: AdicAmount,
        collateral_return: AdicAmount,
        penalty: AdicAmount,
    ) -> Self {
        let now = chrono::Utc::now().timestamp();
        let mut termination_data = Vec::new();
        termination_data.extend_from_slice(&deal_id.to_le_bytes());
        termination_data.extend_from_slice(initiator.as_bytes());
        termination_data.extend_from_slice(&termination_epoch.to_le_bytes());
        termination_data.extend_from_slice(&now.to_le_bytes());

        Self {
            approvals: [MessageHash::default(); 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: now,
            deal_id,
            initiator,
            reason,
            termination_epoch,
            client_refund,
            provider_payment,
            collateral_return,
            penalty,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
            termination_id: blake3::hash(&termination_data).into(),
        }
    }

    pub fn is_finalized(&self) -> bool {
        self.finality_status.is_finalized()
    }
}

/// Deal migration message (transfer between providers)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DealMigration {
    // ========== Protocol Fields ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Migration Fields ==========
    /// Deal being migrated
    pub deal_id: u64,
    /// Client authorizing migration
    pub client: AccountAddress,
    /// Current provider
    pub old_provider: AccountAddress,
    /// New provider
    pub new_provider: AccountAddress,
    /// Migration deadline (epochs)
    pub migration_deadline: u64,
    /// New provider's collateral
    pub new_collateral: AdicAmount,
    /// Migration proof (data transfer verification)
    pub migration_proof: Option<MigrationProof>,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
    pub migration_id: Hash,
}

/// Proof that data was successfully migrated
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationProof {
    /// Merkle root from old provider
    pub old_merkle_root: [u8; 32],
    /// Merkle root from new provider
    pub new_merkle_root: [u8; 32],
    /// Epoch when migration completed
    pub completed_epoch: u64,
    /// New provider's signature
    pub signature: Signature,
}

impl DealMigration {
    pub fn new(
        deal_id: u64,
        client: AccountAddress,
        old_provider: AccountAddress,
        new_provider: AccountAddress,
        migration_deadline: u64,
        new_collateral: AdicAmount,
    ) -> Self {
        let now = chrono::Utc::now().timestamp();
        let mut migration_data = Vec::new();
        migration_data.extend_from_slice(&deal_id.to_le_bytes());
        migration_data.extend_from_slice(client.as_bytes());
        migration_data.extend_from_slice(old_provider.as_bytes());
        migration_data.extend_from_slice(new_provider.as_bytes());
        migration_data.extend_from_slice(&now.to_le_bytes());

        Self {
            approvals: [MessageHash::default(); 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: now,
            deal_id,
            client,
            old_provider,
            new_provider,
            migration_deadline,
            new_collateral,
            migration_proof: None,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
            migration_id: blake3::hash(&migration_data).into(),
        }
    }

    pub fn is_finalized(&self) -> bool {
        self.finality_status.is_finalized()
    }
}

/// Configuration for deal management
#[derive(Debug, Clone)]
pub struct DealManagementConfig {
    /// Minimum renewal duration (epochs)
    pub min_renewal_epochs: u64,
    /// Early termination penalty percentage
    pub early_termination_penalty: f64,
    /// Migration grace period (epochs)
    pub migration_grace_period: u64,
}

impl Default for DealManagementConfig {
    fn default() -> Self {
        Self {
            min_renewal_epochs: 100,
            early_termination_penalty: 0.1, // 10%
            migration_grace_period: 1000,
        }
    }
}

/// Deal management coordinator
pub struct DealManagementCoordinator {
    config: DealManagementConfig,
    renewals: Arc<RwLock<HashMap<Hash, DealRenewal>>>,
    terminations: Arc<RwLock<HashMap<Hash, DealTermination>>>,
    migrations: Arc<RwLock<HashMap<Hash, DealMigration>>>,
}

impl DealManagementCoordinator {
    pub fn new(config: DealManagementConfig) -> Self {
        Self {
            config,
            renewals: Arc::new(RwLock::new(HashMap::new())),
            terminations: Arc::new(RwLock::new(HashMap::new())),
            migrations: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Submit a deal renewal request
    pub async fn submit_renewal(
        &self,
        renewal: DealRenewal,
        deal: &StorageDeal,
    ) -> Result<Hash> {
        // Validate deal is active
        if deal.status != StorageDealStatus::Active {
            return Err(StorageMarketError::InvalidStateTransition {
                from: format!("{:?}", deal.status),
                to: "Renewal".to_string(),
            });
        }

        // Validate renewal duration
        if renewal.additional_epochs < self.config.min_renewal_epochs {
            return Err(StorageMarketError::Other(format!(
                "Renewal duration too short: {} < {}",
                renewal.additional_epochs, self.config.min_renewal_epochs
            )));
        }

        // Validate parties match deal
        if renewal.client != deal.client || renewal.provider != deal.provider {
            return Err(StorageMarketError::Other(
                "Renewal parties don't match deal".to_string(),
            ));
        }

        // Store renewal
        self.renewals
            .write()
            .await
            .insert(renewal.renewal_id, renewal.clone());

        info!(
            renewal_id = hex::encode(&renewal.renewal_id),
            deal_id = renewal.deal_id,
            additional_epochs = renewal.additional_epochs,
            additional_payment = %renewal.additional_payment,
            "📝 Deal renewal submitted"
        );

        Ok(renewal.renewal_id)
    }

    /// Submit a deal termination request
    pub async fn submit_termination(
        &self,
        termination: DealTermination,
        deal: &StorageDeal,
    ) -> Result<Hash> {
        // Validate deal is active
        if deal.status != StorageDealStatus::Active {
            return Err(StorageMarketError::InvalidStateTransition {
                from: format!("{:?}", deal.status),
                to: "Termination".to_string(),
            });
        }

        // Validate initiator is party to the deal
        if termination.initiator != deal.client && termination.initiator != deal.provider {
            return Err(StorageMarketError::Other(
                "Termination initiator not party to deal".to_string(),
            ));
        }

        // Store termination
        self.terminations
            .write()
            .await
            .insert(termination.termination_id, termination.clone());

        warn!(
            termination_id = hex::encode(&termination.termination_id),
            deal_id = termination.deal_id,
            reason = ?termination.reason,
            client_refund = %termination.client_refund,
            provider_payment = %termination.provider_payment,
            "🛑 Deal termination submitted"
        );

        Ok(termination.termination_id)
    }

    /// Submit a deal migration request
    pub async fn submit_migration(
        &self,
        migration: DealMigration,
        deal: &StorageDeal,
    ) -> Result<Hash> {
        // Validate deal is active
        if deal.status != StorageDealStatus::Active {
            return Err(StorageMarketError::InvalidStateTransition {
                from: format!("{:?}", deal.status),
                to: "Migration".to_string(),
            });
        }

        // Validate parties
        if migration.client != deal.client || migration.old_provider != deal.provider {
            return Err(StorageMarketError::Other(
                "Migration parties don't match deal".to_string(),
            ));
        }

        // Validate new provider is different
        if migration.new_provider == migration.old_provider {
            return Err(StorageMarketError::Other(
                "New provider same as old provider".to_string(),
            ));
        }

        // Store migration
        self.migrations
            .write()
            .await
            .insert(migration.migration_id, migration.clone());

        info!(
            migration_id = hex::encode(&migration.migration_id),
            deal_id = migration.deal_id,
            old_provider = hex::encode(migration.old_provider.as_bytes()),
            new_provider = hex::encode(migration.new_provider.as_bytes()),
            deadline = migration.migration_deadline,
            "🔄 Deal migration initiated"
        );

        Ok(migration.migration_id)
    }

    /// Calculate termination refunds and penalties
    pub fn calculate_termination_amounts(
        &self,
        deal: &StorageDeal,
        current_epoch: u64,
        reason: TerminationReason,
    ) -> (AdicAmount, AdicAmount, AdicAmount, AdicAmount) {
        let start_epoch = deal.start_epoch.unwrap_or(0);
        let elapsed_epochs = current_epoch.saturating_sub(start_epoch);
        let total_epochs = deal.deal_duration_epochs;
        let remaining_epochs = total_epochs.saturating_sub(elapsed_epochs);

        // Calculate what's been paid/used
        let payment_per_epoch = deal.price_per_epoch;
        let total_payment = deal.client_payment_escrow;
        let payment_base = payment_per_epoch.to_base_units() * remaining_epochs;
        let unused_payment = AdicAmount::from_base_units(payment_base);

        // Calculate refunds and penalties based on reason
        match reason {
            TerminationReason::ClientRequest => {
                // Client terminates: pays penalty, provider gets payment for work done
                let penalty_base = (unused_payment.to_base_units() as f64
                    * self.config.early_termination_penalty) as u64;
                let penalty = AdicAmount::from_base_units(penalty_base);
                let client_refund_base = unused_payment.to_base_units() - penalty_base;
                let client_refund = AdicAmount::from_base_units(client_refund_base);
                let provider_payment = total_payment.to_base_units() - client_refund_base;
                let provider_payment = AdicAmount::from_base_units(provider_payment);
                let collateral_return = deal.provider_collateral;

                (client_refund, provider_payment, collateral_return, penalty)
            }
            TerminationReason::ProviderRequest | TerminationReason::ProviderBreach => {
                // Provider terminates or breaches: client gets full refund + penalty from collateral
                let penalty_base =
                    (deal.provider_collateral.to_base_units() as f64 * 0.2) as u64; // 20% penalty
                let penalty = AdicAmount::from_base_units(penalty_base);
                let client_refund_base = unused_payment.to_base_units() + penalty_base;
                let client_refund = AdicAmount::from_base_units(client_refund_base);
                let provider_payment_base =
                    total_payment.to_base_units() - unused_payment.to_base_units();
                let provider_payment = AdicAmount::from_base_units(provider_payment_base);
                let collateral_return_base = deal.provider_collateral.to_base_units() - penalty_base;
                let collateral_return = AdicAmount::from_base_units(collateral_return_base);

                (client_refund, provider_payment, collateral_return, penalty)
            }
            TerminationReason::MutualAgreement => {
                // Mutual: fair split, no penalties
                let client_refund = unused_payment;
                let provider_payment_base =
                    total_payment.to_base_units() - unused_payment.to_base_units();
                let provider_payment = AdicAmount::from_base_units(provider_payment_base);
                let collateral_return = deal.provider_collateral;

                (
                    client_refund,
                    provider_payment,
                    collateral_return,
                    AdicAmount::ZERO,
                )
            }
            TerminationReason::ClientBreach => {
                // Client breach: provider gets everything
                let client_refund = AdicAmount::ZERO;
                let provider_payment = total_payment;
                let collateral_return = deal.provider_collateral;
                let penalty_base = (unused_payment.to_base_units() as f64 * 0.1) as u64;
                let penalty = AdicAmount::from_base_units(penalty_base);

                (client_refund, provider_payment, collateral_return, penalty)
            }
            TerminationReason::DataCorruption => {
                // Data corruption: client gets full refund, provider loses collateral
                let client_refund = total_payment;
                let provider_payment = AdicAmount::ZERO;
                let collateral_return = AdicAmount::ZERO;
                let penalty = deal.provider_collateral;

                (client_refund, provider_payment, collateral_return, penalty)
            }
        }
    }

    /// Get renewal by ID
    pub async fn get_renewal(&self, renewal_id: &Hash) -> Option<DealRenewal> {
        self.renewals.read().await.get(renewal_id).cloned()
    }

    /// Get termination by ID
    pub async fn get_termination(&self, termination_id: &Hash) -> Option<DealTermination> {
        self.terminations.read().await.get(termination_id).cloned()
    }

    /// Get migration by ID
    pub async fn get_migration(&self, migration_id: &Hash) -> Option<DealMigration> {
        self.migrations.read().await.get(migration_id).cloned()
    }

    /// Get statistics
    pub async fn get_stats(&self) -> DealManagementStats {
        let renewals = self.renewals.read().await;
        let terminations = self.terminations.read().await;
        let migrations = self.migrations.read().await;

        DealManagementStats {
            total_renewals: renewals.len(),
            total_terminations: terminations.len(),
            total_migrations: migrations.len(),
            terminations_by_reason: Self::count_terminations_by_reason(&terminations),
        }
    }

    fn count_terminations_by_reason(
        terminations: &HashMap<Hash, DealTermination>,
    ) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for termination in terminations.values() {
            let reason = format!("{:?}", termination.reason);
            *counts.entry(reason).or_insert(0) += 1;
        }
        counts
    }
}

/// Deal management statistics
#[derive(Debug, Clone)]
pub struct DealManagementStats {
    pub total_renewals: usize,
    pub total_terminations: usize,
    pub total_migrations: usize,
    pub terminations_by_reason: HashMap<String, usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_deal() -> StorageDeal {
        StorageDeal {
            approvals: [[0u8; 32]; 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: chrono::Utc::now().timestamp(),
            deal_id: 1,
            ref_intent: [1u8; 32],
            ref_acceptance: [2u8; 32],
            client: AccountAddress::from_bytes([1u8; 32]),
            provider: AccountAddress::from_bytes([2u8; 32]),
            data_cid: [3u8; 32],
            data_size: 1024 * 1024,
            deal_duration_epochs: 1000,
            price_per_epoch: AdicAmount::from_adic(1.0),
            provider_collateral: AdicAmount::from_adic(1000.0),
            client_payment_escrow: AdicAmount::from_adic(1000.0),
            proof_merkle_root: Some([7u8; 32]),
            start_epoch: Some(100),
            activation_deadline: 200,
            status: StorageDealStatus::Active,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
        }
    }

    #[tokio::test]
    async fn test_deal_renewal() {
        let coordinator = DealManagementCoordinator::new(DealManagementConfig::default());
        let deal = create_test_deal();

        let renewal = DealRenewal::new(
            deal.deal_id,
            deal.client,
            deal.provider,
            500, // Extend by 500 epochs
            AdicAmount::from_adic(500.0),
            AdicAmount::from_adic(500.0),
            None,
        );

        let renewal_id = coordinator.submit_renewal(renewal, &deal).await.unwrap();
        assert_ne!(renewal_id, [0u8; 32]);

        let stored = coordinator.get_renewal(&renewal_id).await.unwrap();
        assert_eq!(stored.additional_epochs, 500);
    }

    #[tokio::test]
    async fn test_deal_termination() {
        let coordinator = DealManagementCoordinator::new(DealManagementConfig::default());
        let deal = create_test_deal();

        let (client_refund, provider_payment, collateral_return, penalty) = coordinator
            .calculate_termination_amounts(&deal, 600, TerminationReason::ClientRequest);

        // After 500 epochs (600 - 100 start), 500 epochs remain
        // Client should get refund minus 10% penalty
        assert!(client_refund.to_base_units() > 0);
        assert!(penalty.to_base_units() > 0);
        assert_eq!(collateral_return, deal.provider_collateral);

        let termination = DealTermination::new(
            deal.deal_id,
            deal.client,
            TerminationReason::ClientRequest,
            600,
            client_refund,
            provider_payment,
            collateral_return,
            penalty,
        );

        let termination_id = coordinator
            .submit_termination(termination, &deal)
            .await
            .unwrap();
        assert_ne!(termination_id, [0u8; 32]);
    }

    #[tokio::test]
    async fn test_deal_migration() {
        let coordinator = DealManagementCoordinator::new(DealManagementConfig::default());
        let deal = create_test_deal();
        let new_provider = AccountAddress::from_bytes([3u8; 32]);

        let migration = DealMigration::new(
            deal.deal_id,
            deal.client,
            deal.provider,
            new_provider,
            2000, // Migration deadline
            AdicAmount::from_adic(1000.0),
        );

        let migration_id = coordinator.submit_migration(migration, &deal).await.unwrap();
        assert_ne!(migration_id, [0u8; 32]);

        let stored = coordinator.get_migration(&migration_id).await.unwrap();
        assert_eq!(stored.new_provider, new_provider);
    }

    #[test]
    fn test_termination_penalty_calculations() {
        let coordinator = DealManagementCoordinator::new(DealManagementConfig::default());
        let deal = create_test_deal();

        // Test client request termination (10% penalty)
        let (client_refund, _, _, penalty) =
            coordinator.calculate_termination_amounts(&deal, 600, TerminationReason::ClientRequest);
        assert!(penalty.to_base_units() > 0);
        assert!(client_refund.to_base_units() > 0);

        // Test provider breach (20% collateral penalty)
        let (client_refund, _, _, penalty) =
            coordinator.calculate_termination_amounts(&deal, 600, TerminationReason::ProviderBreach);
        assert!(penalty.to_base_units() > 0);
        assert!(client_refund.to_base_units() > 0);

        // Test mutual agreement (no penalty)
        let (_, _, _, penalty) = coordinator
            .calculate_termination_amounts(&deal, 600, TerminationReason::MutualAgreement);
        assert_eq!(penalty.to_base_units(), 0);

        // Test data corruption (full collateral penalty)
        let (client_refund, _, collateral_return, penalty) =
            coordinator.calculate_termination_amounts(&deal, 600, TerminationReason::DataCorruption);
        assert_eq!(client_refund, deal.client_payment_escrow);
        assert_eq!(collateral_return.to_base_units(), 0);
        assert_eq!(penalty, deal.provider_collateral);
    }

    #[tokio::test]
    async fn test_stats() {
        let coordinator = DealManagementCoordinator::new(DealManagementConfig::default());
        let deal = create_test_deal();

        // Submit some operations
        let renewal = DealRenewal::new(
            deal.deal_id,
            deal.client,
            deal.provider,
            500,
            AdicAmount::from_adic(500.0),
            AdicAmount::from_adic(500.0),
            None,
        );
        coordinator.submit_renewal(renewal, &deal).await.unwrap();

        let (client_refund, provider_payment, collateral_return, penalty) = coordinator
            .calculate_termination_amounts(&deal, 600, TerminationReason::ClientRequest);
        let termination = DealTermination::new(
            deal.deal_id,
            deal.client,
            TerminationReason::ClientRequest,
            600,
            client_refund,
            provider_payment,
            collateral_return,
            penalty,
        );
        coordinator
            .submit_termination(termination, &deal)
            .await
            .unwrap();

        let stats = coordinator.get_stats().await;
        assert_eq!(stats.total_renewals, 1);
        assert_eq!(stats.total_terminations, 1);
        assert_eq!(stats.total_migrations, 0);
    }
}
