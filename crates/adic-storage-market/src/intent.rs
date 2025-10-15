//! Intent Layer - Zero-value OCIM message management
//!
//! This module implements the intent-driven deal discovery layer where:
//! - Clients publish StorageDealIntents (zero-value messages)
//! - Providers respond with ProviderAcceptances (zero-value messages)
//! - All messages inherit ADIC's admissibility and finality guarantees
//! - Deposits are refunded on finality

use crate::error::{Result, StorageMarketError};
use crate::types::{
    FinalityStatus, Hash, ProviderAcceptance, StorageDealIntent,
};
use adic_consensus::ReputationTracker;
use adic_economics::types::{AccountAddress, AdicAmount};
use adic_economics::BalanceManager;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Configuration for intent layer
#[derive(Debug, Clone)]
pub struct IntentConfig {
    /// Minimum anti-spam deposit for intents
    pub min_deposit: AdicAmount,
    /// Minimum client reputation to publish intent
    pub min_client_reputation: f64,
    /// Maximum intent duration (seconds)
    pub max_intent_duration: i64,
    /// Maximum number of active intents per client
    pub max_intents_per_client: usize,
}

impl Default for IntentConfig {
    fn default() -> Self {
        Self {
            min_deposit: AdicAmount::from_adic(0.1),
            min_client_reputation: 10.0,
            max_intent_duration: 86400, // 24 hours
            max_intents_per_client: 10,
        }
    }
}

/// Intent manager handles intent lifecycle
pub struct IntentManager {
    config: IntentConfig,
    balance_manager: Arc<BalanceManager>,
    reputation_tracker: Arc<ReputationTracker>,
    /// Active intents indexed by intent_id
    intents: Arc<RwLock<HashMap<Hash, StorageDealIntent>>>,
    /// Acceptances indexed by acceptance_id
    acceptances: Arc<RwLock<HashMap<Hash, ProviderAcceptance>>>,
    /// Map from intent_id to list of acceptance_ids
    intent_acceptances: Arc<RwLock<HashMap<Hash, Vec<Hash>>>>,
    /// Intents by client for rate limiting
    client_intents: Arc<RwLock<HashMap<AccountAddress, Vec<Hash>>>>,
}

impl IntentManager {
    /// Create new intent manager
    pub fn new(
        config: IntentConfig,
        balance_manager: Arc<BalanceManager>,
        reputation_tracker: Arc<ReputationTracker>,
    ) -> Self {
        Self {
            config,
            balance_manager,
            reputation_tracker,
            intents: Arc::new(RwLock::new(HashMap::new())),
            acceptances: Arc::new(RwLock::new(HashMap::new())),
            intent_acceptances: Arc::new(RwLock::new(HashMap::new())),
            client_intents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Publish a new storage intent
    pub async fn publish_intent(
        &self,
        intent: StorageDealIntent,
    ) -> Result<Hash> {
        // Validate client reputation
        let client_pubkey = adic_types::PublicKey::from_bytes(*intent.client.as_bytes());
        let client_rep = self.reputation_tracker.get_reputation(&client_pubkey).await;
        if client_rep < self.config.min_client_reputation {
            return Err(StorageMarketError::InsufficientReputation {
                required: self.config.min_client_reputation,
                actual: client_rep,
            });
        }

        // Validate deposit
        if intent.deposit < self.config.min_deposit {
            return Err(StorageMarketError::InsufficientFunds {
                required: self.config.min_deposit.to_string(),
                available: intent.deposit.to_string(),
            });
        }

        // Check client rate limit
        let client_intents = self.client_intents.read().await;
        if let Some(intents) = client_intents.get(&intent.client) {
            if intents.len() >= self.config.max_intents_per_client {
                return Err(StorageMarketError::Other(format!(
                    "Client has too many active intents: {} >= {}",
                    intents.len(),
                    self.config.max_intents_per_client
                )));
            }
        }
        drop(client_intents);

        // Validate expiration
        let duration = intent.expires_at - intent.timestamp;
        if duration > self.config.max_intent_duration {
            return Err(StorageMarketError::Other(format!(
                "Intent duration too long: {} > {}",
                duration, self.config.max_intent_duration
            )));
        }

        // Lock deposit (will be refunded on finality)
        self.balance_manager
            .debit(intent.client, intent.deposit)
            .await
            .map_err(|e| StorageMarketError::EconomicsError(e.to_string()))?;

        let intent_id = intent.intent_id;

        // Store intent
        let mut intents = self.intents.write().await;
        intents.insert(intent_id, intent.clone());

        // Track by client
        let mut client_intents = self.client_intents.write().await;
        client_intents
            .entry(intent.client)
            .or_insert_with(Vec::new)
            .push(intent_id);

        info!(
            intent_id = hex::encode(&intent_id),
            client = hex::encode(intent.client.as_bytes()),
            data_size = intent.data_size,
            duration_epochs = intent.duration_epochs,
            redundancy = intent.required_redundancy,
            "📋 Storage intent published"
        );

        Ok(intent_id)
    }

    /// Update intent finality status (called by consensus layer)
    pub async fn update_intent_finality(
        &self,
        intent_id: &Hash,
        finality_status: FinalityStatus,
        finalized_at_epoch: u64,
    ) -> Result<()> {
        let mut intents = self.intents.write().await;
        let intent = intents
            .get_mut(intent_id)
            .ok_or_else(|| StorageMarketError::Other("Intent not found".to_string()))?;

        intent.finality_status = finality_status;
        intent.finalized_at_epoch = Some(finalized_at_epoch);

        // Refund deposit on finality
        if intent.finality_status.is_finalized() {
            self.balance_manager
                .credit(intent.client, intent.deposit)
                .await
                .map_err(|e| StorageMarketError::EconomicsError(e.to_string()))?;

            info!(
                intent_id = hex::encode(intent_id),
                epoch = finalized_at_epoch,
                "✅ Intent finalized, deposit refunded"
            );
        }

        Ok(())
    }

    /// Submit provider acceptance for a finalized intent
    pub async fn submit_acceptance(
        &self,
        mut acceptance: ProviderAcceptance,
    ) -> Result<Hash> {
        // Verify intent exists and is finalized
        let intents = self.intents.read().await;
        let intent = intents
            .get(&acceptance.ref_intent)
            .ok_or_else(|| StorageMarketError::Other("Referenced intent not found".to_string()))?;

        if !intent.is_finalized() {
            return Err(StorageMarketError::FinalityNotReached(
                "Intent must be finalized before acceptance".to_string(),
            ));
        }

        if intent.is_expired() {
            return Err(StorageMarketError::IntentExpired(intent.expires_at));
        }
        drop(intents);

        // Validate provider reputation
        let provider_pubkey = adic_types::PublicKey::from_bytes(*acceptance.provider.as_bytes());
        let provider_rep = self
            .reputation_tracker
            .get_reputation(&provider_pubkey)
            .await;
        if provider_rep < self.config.min_client_reputation {
            return Err(StorageMarketError::InsufficientReputation {
                required: self.config.min_client_reputation,
                actual: provider_rep,
            });
        }

        // Update acceptance with actual reputation
        acceptance.provider_reputation = provider_rep;

        // Validate deposit
        if acceptance.deposit < self.config.min_deposit {
            return Err(StorageMarketError::InsufficientFunds {
                required: self.config.min_deposit.to_string(),
                available: acceptance.deposit.to_string(),
            });
        }

        // Lock deposit
        self.balance_manager
            .debit(acceptance.provider, acceptance.deposit)
            .await
            .map_err(|e| StorageMarketError::EconomicsError(e.to_string()))?;

        let acceptance_id = acceptance.acceptance_id;

        // Store acceptance
        let mut acceptances = self.acceptances.write().await;
        acceptances.insert(acceptance_id, acceptance.clone());

        // Link to intent
        let mut intent_acceptances = self.intent_acceptances.write().await;
        intent_acceptances
            .entry(acceptance.ref_intent)
            .or_insert_with(Vec::new)
            .push(acceptance_id);

        info!(
            acceptance_id = hex::encode(&acceptance_id),
            intent_id = hex::encode(&acceptance.ref_intent),
            provider = hex::encode(acceptance.provider.as_bytes()),
            offered_price = acceptance.offered_price_per_epoch.to_adic(),
            "👷 Provider acceptance submitted"
        );

        Ok(acceptance_id)
    }

    /// Update acceptance finality status
    pub async fn update_acceptance_finality(
        &self,
        acceptance_id: &Hash,
        finality_status: FinalityStatus,
        finalized_at_epoch: u64,
    ) -> Result<()> {
        let mut acceptances = self.acceptances.write().await;
        let acceptance = acceptances
            .get_mut(acceptance_id)
            .ok_or_else(|| StorageMarketError::Other("Acceptance not found".to_string()))?;

        acceptance.finality_status = finality_status;
        acceptance.finalized_at_epoch = Some(finalized_at_epoch);

        // Refund deposit on finality
        if acceptance.finality_status.is_finalized() {
            self.balance_manager
                .credit(acceptance.provider, acceptance.deposit)
                .await
                .map_err(|e| StorageMarketError::EconomicsError(e.to_string()))?;

            info!(
                acceptance_id = hex::encode(acceptance_id),
                epoch = finalized_at_epoch,
                "✅ Acceptance finalized, deposit refunded"
            );
        }

        Ok(())
    }

    /// Get finalized acceptances for an intent
    pub async fn get_acceptances_for_intent(
        &self,
        intent_id: &Hash,
    ) -> Result<Vec<ProviderAcceptance>> {
        let intent_acceptances = self.intent_acceptances.read().await;
        let acceptance_ids = intent_acceptances
            .get(intent_id)
            .ok_or_else(|| StorageMarketError::Other("No acceptances found".to_string()))?;

        let acceptances = self.acceptances.read().await;
        let mut result = Vec::new();

        for acceptance_id in acceptance_ids {
            if let Some(acceptance) = acceptances.get(acceptance_id) {
                if acceptance.is_finalized() {
                    result.push(acceptance.clone());
                }
            }
        }

        Ok(result)
    }

    /// Get intent by ID
    pub async fn get_intent(&self, intent_id: &Hash) -> Option<StorageDealIntent> {
        let intents = self.intents.read().await;
        intents.get(intent_id).cloned()
    }

    /// Get acceptance by ID
    pub async fn get_acceptance(&self, acceptance_id: &Hash) -> Option<ProviderAcceptance> {
        let acceptances = self.acceptances.read().await;
        acceptances.get(acceptance_id).cloned()
    }

    /// Get all finalized intents (for provider discovery)
    pub async fn get_finalized_intents(&self) -> Vec<StorageDealIntent> {
        let intents = self.intents.read().await;
        intents
            .values()
            .filter(|intent| intent.is_finalized() && !intent.is_expired())
            .cloned()
            .collect()
    }

    /// Cleanup expired intents
    pub async fn cleanup_expired_intents(&self) -> Result<usize> {
        let mut intents = self.intents.write().await;
        let mut client_intents = self.client_intents.write().await;
        let mut intent_acceptances = self.intent_acceptances.write().await;

        let expired: Vec<Hash> = intents
            .iter()
            .filter(|(_, intent)| intent.is_expired())
            .map(|(id, _)| *id)
            .collect();

        let count = expired.len();

        for intent_id in expired {
            if let Some(intent) = intents.remove(&intent_id) {
                // Remove from client tracking
                if let Some(client_intent_list) = client_intents.get_mut(&intent.client) {
                    client_intent_list.retain(|id| id != &intent_id);
                }

                // Remove acceptance tracking
                intent_acceptances.remove(&intent_id);

                warn!(
                    intent_id = hex::encode(&intent_id),
                    "🗑️ Expired intent removed"
                );
            }
        }

        Ok(count)
    }

    /// Get statistics
    pub async fn get_stats(&self) -> IntentStats {
        let intents = self.intents.read().await;
        let acceptances = self.acceptances.read().await;

        let mut stats = IntentStats {
            total_intents: intents.len() as u64,
            finalized_intents: 0,
            pending_intents: 0,
            expired_intents: 0,
            total_acceptances: acceptances.len() as u64,
            finalized_acceptances: 0,
        };

        for intent in intents.values() {
            if intent.is_expired() {
                stats.expired_intents += 1;
            } else if intent.is_finalized() {
                stats.finalized_intents += 1;
            } else {
                stats.pending_intents += 1;
            }
        }

        for acceptance in acceptances.values() {
            if acceptance.is_finalized() {
                stats.finalized_acceptances += 1;
            }
        }

        stats
    }

    /// Get the number of acceptances for a given provider
    pub async fn get_provider_acceptance_count(&self, provider: AccountAddress) -> usize {
        self.acceptances
            .read()
            .await
            .values()
            .filter(|a| a.provider == provider)
            .count()
    }

    /// Get the number of intents for a given client
    pub async fn get_client_intent_count(&self, client: AccountAddress) -> usize {
        self.client_intents
            .read()
            .await
            .get(&client)
            .map(|intents| intents.len())
            .unwrap_or(0)
    }
}

/// Intent layer statistics
#[derive(Debug, Clone)]
pub struct IntentStats {
    pub total_intents: u64,
    pub finalized_intents: u64,
    pub pending_intents: u64,
    pub expired_intents: u64,
    pub total_acceptances: u64,
    pub finalized_acceptances: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_economics::storage::MemoryStorage;
    use adic_types::PublicKey;
    use crate::types::SettlementRail;

    async fn setup() -> IntentManager {
        let storage = Arc::new(MemoryStorage::new());
        let balance_mgr = Arc::new(BalanceManager::new(storage));
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));

        IntentManager::new(IntentConfig::default(), balance_mgr, rep_tracker)
    }

    #[tokio::test]
    async fn test_publish_intent() {
        let manager = setup().await;
        let client = AccountAddress::from_bytes([1u8; 32]);

        // Fund client
        manager
            .balance_manager
            .credit(client, AdicAmount::from_adic(100.0))
            .await
            .unwrap();

        // Set reputation
        let client_pubkey = PublicKey::from_bytes(*client.as_bytes());
        manager
            .reputation_tracker
            .set_reputation(&client_pubkey, 50.0)
            .await;

        let mut intent = StorageDealIntent::new(
            client,
            [0u8; 32],
            1024 * 1024,
            100,
            AdicAmount::from_adic(10.0),
            3,
            vec![SettlementRail::AdicNative],
        );
        intent.deposit = AdicAmount::from_adic(0.1);

        let intent_id = manager.publish_intent(intent).await.unwrap();

        let stored = manager.get_intent(&intent_id).await.unwrap();
        assert_eq!(stored.client, client);
        assert!(!stored.is_finalized());
    }

    #[tokio::test]
    async fn test_intent_finality_refund() {
        let manager = setup().await;
        let client = AccountAddress::from_bytes([1u8; 32]);

        manager
            .balance_manager
            .credit(client, AdicAmount::from_adic(100.0))
            .await
            .unwrap();

        let client_pubkey = PublicKey::from_bytes(*client.as_bytes());
        manager
            .reputation_tracker
            .set_reputation(&client_pubkey, 50.0)
            .await;

        let mut intent = StorageDealIntent::new(
            client,
            [0u8; 32],
            1000,
            10,
            AdicAmount::from_adic(5.0),
            1,
            vec![SettlementRail::AdicNative],
        );
        intent.deposit = AdicAmount::from_adic(1.0);

        let initial_balance = manager.balance_manager.get_balance(client).await.unwrap();
        let intent_id = manager.publish_intent(intent).await.unwrap();

        // Balance should be reduced by deposit
        let after_publish = manager.balance_manager.get_balance(client).await.unwrap();
        assert!(after_publish < initial_balance);

        // Mark finalized
        manager
            .update_intent_finality(
                &intent_id,
                FinalityStatus::F1Complete {
                    k: 20,
                    depth: 12,
                    rep_weight: 150.0,
                },
                100,
            )
            .await
            .unwrap();

        // Balance should be restored
        let after_finality = manager.balance_manager.get_balance(client).await.unwrap();
        assert_eq!(after_finality, initial_balance);
    }

    #[tokio::test]
    async fn test_submit_acceptance() {
        let manager = setup().await;
        let client = AccountAddress::from_bytes([1u8; 32]);
        let provider = AccountAddress::from_bytes([2u8; 32]);

        // Setup balances
        manager
            .balance_manager
            .credit(client, AdicAmount::from_adic(100.0))
            .await
            .unwrap();
        manager
            .balance_manager
            .credit(provider, AdicAmount::from_adic(100.0))
            .await
            .unwrap();

        // Setup reputations
        let client_pubkey = PublicKey::from_bytes(*client.as_bytes());
        let provider_pubkey = PublicKey::from_bytes(*provider.as_bytes());

        manager
            .reputation_tracker
            .set_reputation(&client_pubkey, 50.0)
            .await;
        manager
            .reputation_tracker
            .set_reputation(&provider_pubkey, 100.0)
            .await;

        // Publish and finalize intent
        let mut intent = StorageDealIntent::new(
            client,
            [0u8; 32],
            1000,
            10,
            AdicAmount::from_adic(5.0),
            1,
            vec![SettlementRail::AdicNative],
        );
        intent.deposit = AdicAmount::from_adic(0.1);

        let intent_id = manager.publish_intent(intent).await.unwrap();
        manager
            .update_intent_finality(
                &intent_id,
                FinalityStatus::F1Complete {
                    k: 20,
                    depth: 12,
                    rep_weight: 150.0,
                },
                100,
            )
            .await
            .unwrap();

        // Submit acceptance
        let mut acceptance = ProviderAcceptance::new(
            intent_id,
            provider,
            AdicAmount::from_adic(4.0),
            AdicAmount::from_adic(50.0),
            0.0, // Will be updated
        );
        acceptance.deposit = AdicAmount::from_adic(0.1);

        let acceptance_id = manager.submit_acceptance(acceptance).await.unwrap();

        let stored = manager.get_acceptance(&acceptance_id).await.unwrap();
        assert_eq!(stored.provider, provider);
        assert_eq!(stored.provider_reputation, 100.0);
    }

    #[tokio::test]
    async fn test_get_acceptances_for_intent() {
        let manager = setup().await;
        let client = AccountAddress::from_bytes([1u8; 32]);
        let provider1 = AccountAddress::from_bytes([2u8; 32]);
        let provider2 = AccountAddress::from_bytes([3u8; 32]);

        // Setup
        manager
            .balance_manager
            .credit(client, AdicAmount::from_adic(100.0))
            .await
            .unwrap();
        manager
            .balance_manager
            .credit(provider1, AdicAmount::from_adic(100.0))
            .await
            .unwrap();
        manager
            .balance_manager
            .credit(provider2, AdicAmount::from_adic(100.0))
            .await
            .unwrap();
        let client_pubkey = PublicKey::from_bytes(*client.as_bytes());
        let provider1_pubkey = PublicKey::from_bytes(*provider1.as_bytes());
        let provider2_pubkey = PublicKey::from_bytes(*provider2.as_bytes());

        manager
            .reputation_tracker
            .set_reputation(&client_pubkey, 50.0)
            .await;
        manager
            .reputation_tracker
            .set_reputation(&provider1_pubkey, 100.0)
            .await;
        manager
            .reputation_tracker
            .set_reputation(&provider2_pubkey, 90.0)
            .await;

        // Publish and finalize intent
        let mut intent = StorageDealIntent::new(
            client,
            [0u8; 32],
            1000,
            10,
            AdicAmount::from_adic(5.0),
            2,
            vec![SettlementRail::AdicNative],
        );
        intent.deposit = AdicAmount::from_adic(0.1);

        let intent_id = manager.publish_intent(intent).await.unwrap();
        manager
            .update_intent_finality(
                &intent_id,
                FinalityStatus::F1Complete {
                    k: 20,
                    depth: 12,
                    rep_weight: 150.0,
                },
                100,
            )
            .await
            .unwrap();

        // Submit acceptances
        let mut acceptance1 = ProviderAcceptance::new(
            intent_id,
            provider1,
            AdicAmount::from_adic(4.0),
            AdicAmount::from_adic(50.0),
            0.0,
        );
        acceptance1.deposit = AdicAmount::from_adic(0.1);
        let acc1_id = manager.submit_acceptance(acceptance1).await.unwrap();

        let mut acceptance2 = ProviderAcceptance::new(
            intent_id,
            provider2,
            AdicAmount::from_adic(4.5),
            AdicAmount::from_adic(55.0),
            0.0,
        );
        acceptance2.deposit = AdicAmount::from_adic(0.1);
        let acc2_id = manager.submit_acceptance(acceptance2).await.unwrap();

        // Finalize acceptances
        manager
            .update_acceptance_finality(
                &acc1_id,
                FinalityStatus::F1Complete {
                    k: 20,
                    depth: 12,
                    rep_weight: 150.0,
                },
                110,
            )
            .await
            .unwrap();
        manager
            .update_acceptance_finality(
                &acc2_id,
                FinalityStatus::F1Complete {
                    k: 20,
                    depth: 12,
                    rep_weight: 150.0,
                },
                110,
            )
            .await
            .unwrap();

        // Get acceptances
        let acceptances = manager.get_acceptances_for_intent(&intent_id).await.unwrap();
        assert_eq!(acceptances.len(), 2);
    }

    #[tokio::test]
    async fn test_cleanup_expired_intents() {
        let manager = setup().await;
        let client = AccountAddress::from_bytes([1u8; 32]);

        manager
            .balance_manager
            .credit(client, AdicAmount::from_adic(100.0))
            .await
            .unwrap();

        let client_pubkey = PublicKey::from_bytes(*client.as_bytes());
        manager
            .reputation_tracker
            .set_reputation(&client_pubkey, 50.0)
            .await;

        // Create expired intent
        let mut intent = StorageDealIntent::new(
            client,
            [0u8; 32],
            1000,
            10,
            AdicAmount::from_adic(5.0),
            1,
            vec![SettlementRail::AdicNative],
        );
        intent.deposit = AdicAmount::from_adic(0.1);
        intent.expires_at = chrono::Utc::now().timestamp() - 3600; // Expired 1 hour ago

        let _intent_id = manager.publish_intent(intent).await.unwrap();

        let stats_before = manager.get_stats().await;
        assert_eq!(stats_before.expired_intents, 1);

        let cleaned = manager.cleanup_expired_intents().await.unwrap();
        assert_eq!(cleaned, 1);

        let stats_after = manager.get_stats().await;
        assert_eq!(stats_after.total_intents, 0);
    }
}
