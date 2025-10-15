//! Provider Lifecycle Management
//!
//! This module implements Sprint 7 from storage-market-design.md:
//! - Provider registration with capacity declaration
//! - Heartbeat monitoring (100 epoch intervals)
//! - Graceful exit with data migration
//! - Auto-repair on provider failure

use crate::error::{Result, StorageMarketError};
use crate::types::{FinalityStatus, Hash, MessageHash};
use adic_economics::types::{AccountAddress, AdicAmount};
use adic_types::Signature;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Provider registration message (zero-value OCIM)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRegistration {
    // ========== Protocol Fields ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Registration Fields ==========
    /// Provider account address
    pub provider: AccountAddress,
    /// Total storage capacity (bytes)
    pub total_capacity: u64,
    /// Available storage capacity (bytes)
    pub available_capacity: u64,
    /// Collateral locked for registration
    pub collateral: AdicAmount,
    /// Network endpoint for data transfer
    pub endpoint: String,
    /// Geographic region (optional)
    pub region: Option<String>,
    /// ASN for diversity (optional)
    pub asn: Option<u32>,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
    pub registration_id: Hash,
}

impl ProviderRegistration {
    pub fn new(
        provider: AccountAddress,
        total_capacity: u64,
        available_capacity: u64,
        collateral: AdicAmount,
        endpoint: String,
        region: Option<String>,
        asn: Option<u32>,
    ) -> Self {
        let now = chrono::Utc::now().timestamp();
        let mut reg_data = Vec::new();
        reg_data.extend_from_slice(provider.as_bytes());
        reg_data.extend_from_slice(&total_capacity.to_le_bytes());
        reg_data.extend_from_slice(&now.to_le_bytes());

        Self {
            approvals: [MessageHash::default(); 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: now,
            provider,
            total_capacity,
            available_capacity,
            collateral,
            endpoint,
            region,
            asn,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
            registration_id: blake3::hash(&reg_data).into(),
        }
    }

    pub fn is_finalized(&self) -> bool {
        self.finality_status.is_finalized()
    }
}

/// Provider heartbeat message (zero-value OCIM)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHeartbeat {
    // ========== Protocol Fields ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Heartbeat Fields ==========
    pub provider: AccountAddress,
    pub heartbeat_epoch: u64,
    /// Current storage utilization (bytes used)
    pub storage_used: u64,
    /// Current available capacity (bytes)
    pub available_capacity: u64,
    /// Number of active deals
    pub active_deals: u32,
    /// Health status
    pub health: ProviderHealth,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
}

/// Provider health status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderHealth {
    /// Fully operational
    Healthy,
    /// Performance degraded but operational
    Degraded,
    /// Offline or unreachable
    Offline,
    /// In process of exiting
    Exiting,
}

impl ProviderHeartbeat {
    pub fn new(
        provider: AccountAddress,
        heartbeat_epoch: u64,
        storage_used: u64,
        available_capacity: u64,
        active_deals: u32,
        health: ProviderHealth,
    ) -> Self {
        Self {
            approvals: [MessageHash::default(); 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: chrono::Utc::now().timestamp(),
            provider,
            heartbeat_epoch,
            storage_used,
            available_capacity,
            active_deals,
            health,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
        }
    }
}

/// Provider exit message (initiates graceful shutdown)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderExit {
    // ========== Protocol Fields ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Exit Fields ==========
    pub provider: AccountAddress,
    /// Epoch when exit was initiated
    pub exit_epoch: u64,
    /// Epoch by which migration must complete
    pub migration_deadline: u64,
    /// Reason for exit
    pub exit_reason: ExitReason,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExitReason {
    /// Voluntary exit
    Voluntary,
    /// Forced exit due to poor performance
    PerformanceBreach,
    /// Forced exit due to slashing
    Slashed,
    /// Maintenance/upgrade
    Maintenance,
}

impl ProviderExit {
    pub fn new(
        provider: AccountAddress,
        exit_epoch: u64,
        migration_deadline: u64,
        exit_reason: ExitReason,
    ) -> Self {
        Self {
            approvals: [MessageHash::default(); 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: chrono::Utc::now().timestamp(),
            provider,
            exit_epoch,
            migration_deadline,
            exit_reason,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
        }
    }
}

/// Provider lifecycle state
#[derive(Debug, Clone)]
pub struct ProviderState {
    pub provider: AccountAddress,
    pub registration: ProviderRegistration,
    pub health: ProviderHealth,
    pub last_heartbeat_epoch: u64,
    pub storage_used: u64,
    pub available_capacity: u64,
    pub active_deals: Vec<u64>,
    pub exit_info: Option<ProviderExit>,
}

/// Configuration for provider lifecycle management
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Minimum collateral required for registration
    pub min_collateral: AdicAmount,
    /// Heartbeat interval (epochs)
    pub heartbeat_interval: u64,
    /// Maximum heartbeat miss before marking offline
    pub max_missed_heartbeats: u64,
    /// Migration grace period (epochs)
    pub migration_grace_period: u64,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            min_collateral: AdicAmount::from_adic(100.0),
            heartbeat_interval: 100,
            max_missed_heartbeats: 3,
            migration_grace_period: 1000,
        }
    }
}

/// Provider lifecycle manager
pub struct ProviderManager {
    config: ProviderConfig,
    providers: Arc<RwLock<HashMap<AccountAddress, ProviderState>>>,
}

impl ProviderManager {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            config,
            providers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new provider
    pub async fn register_provider(&self, registration: ProviderRegistration) -> Result<()> {
        // Validate collateral
        if registration.collateral < self.config.min_collateral {
            return Err(StorageMarketError::InsufficientFunds {
                required: self.config.min_collateral.to_string(),
                available: registration.collateral.to_string(),
            });
        }

        // Validate capacity
        if registration.available_capacity > registration.total_capacity {
            return Err(StorageMarketError::Other(
                "Available capacity exceeds total capacity".to_string(),
            ));
        }

        // Check if already registered
        let providers = self.providers.read().await;
        if providers.contains_key(&registration.provider) {
            return Err(StorageMarketError::Other(
                "Provider already registered".to_string(),
            ));
        }
        drop(providers);

        // Create provider state
        let state = ProviderState {
            provider: registration.provider,
            registration: registration.clone(),
            health: ProviderHealth::Healthy,
            last_heartbeat_epoch: 0,
            storage_used: 0,
            available_capacity: registration.available_capacity,
            active_deals: Vec::new(),
            exit_info: None,
        };

        // Store provider
        self.providers
            .write()
            .await
            .insert(registration.provider, state);

        info!(
            provider = hex::encode(registration.provider.as_bytes()),
            capacity_gb = registration.total_capacity / (1024 * 1024 * 1024),
            "🔧 Provider registered"
        );

        Ok(())
    }

    /// Process provider heartbeat
    pub async fn process_heartbeat(&self, heartbeat: ProviderHeartbeat) -> Result<()> {
        let mut providers = self.providers.write().await;
        let state = providers
            .get_mut(&heartbeat.provider)
            .ok_or_else(|| StorageMarketError::Other("Provider not registered".to_string()))?;

        // Update state
        state.last_heartbeat_epoch = heartbeat.heartbeat_epoch;
        state.storage_used = heartbeat.storage_used;
        state.available_capacity = heartbeat.available_capacity;
        state.health = heartbeat.health;

        info!(
            provider = hex::encode(heartbeat.provider.as_bytes()),
            epoch = heartbeat.heartbeat_epoch,
            health = ?heartbeat.health,
            utilization_pct = (heartbeat.storage_used as f64 / state.registration.total_capacity as f64 * 100.0) as u64,
            "💓 Heartbeat received"
        );

        Ok(())
    }

    /// Initiate provider exit
    pub async fn initiate_exit(&self, exit: ProviderExit) -> Result<()> {
        let mut providers = self.providers.write().await;
        let state = providers
            .get_mut(&exit.provider)
            .ok_or_else(|| StorageMarketError::Other("Provider not registered".to_string()))?;

        // Set health to Exiting
        state.health = ProviderHealth::Exiting;
        state.exit_info = Some(exit.clone());

        warn!(
            provider = hex::encode(exit.provider.as_bytes()),
            reason = ?exit.exit_reason,
            deadline_epoch = exit.migration_deadline,
            "🚪 Provider exit initiated"
        );

        Ok(())
    }

    /// Check for missed heartbeats and mark providers offline
    pub async fn check_heartbeats(&self, current_epoch: u64) -> Result<Vec<AccountAddress>> {
        let mut offline_providers = Vec::new();
        let mut providers = self.providers.write().await;

        for (address, state) in providers.iter_mut() {
            if state.health == ProviderHealth::Exiting {
                continue; // Skip exiting providers
            }

            let epochs_since_heartbeat = current_epoch.saturating_sub(state.last_heartbeat_epoch);
            let max_interval =
                self.config.heartbeat_interval * self.config.max_missed_heartbeats;

            if epochs_since_heartbeat > max_interval {
                if state.health != ProviderHealth::Offline {
                    state.health = ProviderHealth::Offline;
                    offline_providers.push(*address);

                    warn!(
                        provider = hex::encode(address.as_bytes()),
                        last_heartbeat = state.last_heartbeat_epoch,
                        current_epoch = current_epoch,
                        "⚠️  Provider marked offline (missed heartbeats)"
                    );
                }
            }
        }

        Ok(offline_providers)
    }

    /// Get provider state
    pub async fn get_provider(&self, provider: &AccountAddress) -> Option<ProviderState> {
        self.providers.read().await.get(provider).cloned()
    }

    /// Get all providers
    pub async fn get_all_providers(&self) -> Vec<ProviderState> {
        self.providers.read().await.values().cloned().collect()
    }

    /// Get providers by health status
    pub async fn get_providers_by_health(&self, health: ProviderHealth) -> Vec<ProviderState> {
        self.providers
            .read()
            .await
            .values()
            .filter(|p| p.health == health)
            .cloned()
            .collect()
    }

    /// Update provider's active deals
    pub async fn update_active_deals(
        &self,
        provider: &AccountAddress,
        deal_ids: Vec<u64>,
    ) -> Result<()> {
        let mut providers = self.providers.write().await;
        let state = providers
            .get_mut(provider)
            .ok_or_else(|| StorageMarketError::Other("Provider not registered".to_string()))?;

        state.active_deals = deal_ids;
        Ok(())
    }

    /// Get provider statistics
    pub async fn get_stats(&self) -> ProviderStats {
        let providers = self.providers.read().await;

        let total = providers.len();
        let healthy = providers.values().filter(|p| p.health == ProviderHealth::Healthy).count();
        let degraded = providers.values().filter(|p| p.health == ProviderHealth::Degraded).count();
        let offline = providers.values().filter(|p| p.health == ProviderHealth::Offline).count();
        let exiting = providers.values().filter(|p| p.health == ProviderHealth::Exiting).count();

        let total_capacity: u64 = providers
            .values()
            .map(|p| p.registration.total_capacity)
            .sum();

        let used_capacity: u64 = providers.values().map(|p| p.storage_used).sum();

        ProviderStats {
            total_providers: total,
            healthy_providers: healthy,
            degraded_providers: degraded,
            offline_providers: offline,
            exiting_providers: exiting,
            total_capacity_bytes: total_capacity,
            used_capacity_bytes: used_capacity,
            utilization_percent: if total_capacity > 0 {
                (used_capacity as f64 / total_capacity as f64 * 100.0) as u8
            } else {
                0
            },
        }
    }
}

/// Provider statistics
#[derive(Debug, Clone)]
pub struct ProviderStats {
    pub total_providers: usize,
    pub healthy_providers: usize,
    pub degraded_providers: usize,
    pub offline_providers: usize,
    pub exiting_providers: usize,
    pub total_capacity_bytes: u64,
    pub used_capacity_bytes: u64,
    pub utilization_percent: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_provider_registration() {
        let manager = ProviderManager::new(ProviderConfig::default());
        let provider = AccountAddress::from_bytes([1u8; 32]);

        let registration = ProviderRegistration::new(
            provider,
            1024 * 1024 * 1024 * 1000, // 1 TB
            1024 * 1024 * 1024 * 1000,
            AdicAmount::from_adic(100.0),
            "https://provider.example.com".to_string(),
            Some("us-west".to_string()),
            Some(12345),
        );

        manager.register_provider(registration).await.unwrap();

        let state = manager.get_provider(&provider).await.unwrap();
        assert_eq!(state.health, ProviderHealth::Healthy);
        assert_eq!(state.registration.total_capacity, 1024 * 1024 * 1024 * 1000);
    }

    #[tokio::test]
    async fn test_heartbeat_processing() {
        let manager = ProviderManager::new(ProviderConfig::default());
        let provider = AccountAddress::from_bytes([1u8; 32]);

        // Register provider
        let registration = ProviderRegistration::new(
            provider,
            1000,
            1000,
            AdicAmount::from_adic(100.0),
            "https://provider.example.com".to_string(),
            None,
            None,
        );
        manager.register_provider(registration).await.unwrap();

        // Send heartbeat
        let heartbeat = ProviderHeartbeat::new(provider, 100, 500, 500, 5, ProviderHealth::Healthy);
        manager.process_heartbeat(heartbeat).await.unwrap();

        let state = manager.get_provider(&provider).await.unwrap();
        assert_eq!(state.last_heartbeat_epoch, 100);
        assert_eq!(state.storage_used, 500);
    }

    #[tokio::test]
    async fn test_missed_heartbeats() {
        let config = ProviderConfig {
            heartbeat_interval: 10,
            max_missed_heartbeats: 3,
            ..Default::default()
        };
        let manager = ProviderManager::new(config);
        let provider = AccountAddress::from_bytes([1u8; 32]);

        // Register provider
        let registration = ProviderRegistration::new(
            provider,
            1000,
            1000,
            AdicAmount::from_adic(100.0),
            "https://provider.example.com".to_string(),
            None,
            None,
        );
        manager.register_provider(registration).await.unwrap();

        // Send initial heartbeat
        let heartbeat = ProviderHeartbeat::new(provider, 1, 0, 1000, 0, ProviderHealth::Healthy);
        manager.process_heartbeat(heartbeat).await.unwrap();

        // Check after 31 epochs (should be offline: 10 * 3 = 30)
        let offline = manager.check_heartbeats(32).await.unwrap();
        assert_eq!(offline.len(), 1);
        assert_eq!(offline[0], provider);

        let state = manager.get_provider(&provider).await.unwrap();
        assert_eq!(state.health, ProviderHealth::Offline);
    }

    #[tokio::test]
    async fn test_provider_exit() {
        let manager = ProviderManager::new(ProviderConfig::default());
        let provider = AccountAddress::from_bytes([1u8; 32]);

        // Register provider
        let registration = ProviderRegistration::new(
            provider,
            1000,
            1000,
            AdicAmount::from_adic(100.0),
            "https://provider.example.com".to_string(),
            None,
            None,
        );
        manager.register_provider(registration).await.unwrap();

        // Initiate exit
        let exit = ProviderExit::new(provider, 100, 1100, ExitReason::Voluntary);
        manager.initiate_exit(exit).await.unwrap();

        let state = manager.get_provider(&provider).await.unwrap();
        assert_eq!(state.health, ProviderHealth::Exiting);
        assert!(state.exit_info.is_some());
    }

    #[tokio::test]
    async fn test_provider_stats() {
        let manager = ProviderManager::new(ProviderConfig::default());

        // Register 3 providers
        for i in 0..3 {
            let provider = AccountAddress::from_bytes([i; 32]);
            let registration = ProviderRegistration::new(
                provider,
                1000,
                1000,
                AdicAmount::from_adic(100.0),
                format!("https://provider{}.example.com", i),
                None,
                None,
            );
            manager.register_provider(registration).await.unwrap();
        }

        let stats = manager.get_stats().await;
        assert_eq!(stats.total_providers, 3);
        assert_eq!(stats.healthy_providers, 3);
        assert_eq!(stats.total_capacity_bytes, 3000);
    }
}
