use adic_economics::{AccountAddress, AdicAmount, BalanceManager};
use adic_types::{AdicError, MessageId, PublicKey, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

#[derive(Debug, Clone, PartialEq)]
pub enum DepositStatus {
    Escrowed,
    Refunded,
    Slashed,
}

// Alias for backwards compatibility
pub type DepositState = DepositStatus;

#[derive(Debug, Clone)]
pub struct Deposit {
    pub message_id: MessageId,
    pub proposer_pk: PublicKey,
    pub amount: AdicAmount,
    pub status: DepositStatus,
    pub timestamp: i64,
}

// Backwards compatibility methods
impl Deposit {
    pub fn proposer(&self) -> PublicKey {
        self.proposer_pk
    }

    pub fn state(&self) -> DepositState {
        self.status.clone()
    }
}

#[derive(Debug, Clone)]
pub struct DepositStats {
    pub escrowed_count: usize,
    pub total_escrowed: AdicAmount,
    pub slashed_count: usize,
    pub total_slashed: AdicAmount,
    pub refunded_count: usize,
    pub total_refunded: AdicAmount,
}

pub struct DepositManager {
    deposit_amount: AdicAmount,
    deposits: Arc<RwLock<HashMap<MessageId, Deposit>>>,
    balance_manager: Option<Arc<BalanceManager>>,
}

impl DepositManager {
    pub fn new(deposit_amount: f64) -> Self {
        Self {
            deposit_amount: AdicAmount::from_adic(deposit_amount),
            deposits: Arc::new(RwLock::new(HashMap::new())),
            balance_manager: None,
        }
    }

    pub fn with_balance_manager(
        deposit_amount: AdicAmount,
        balance_manager: Arc<BalanceManager>,
    ) -> Self {
        Self {
            deposit_amount,
            deposits: Arc::new(RwLock::new(HashMap::new())),
            balance_manager: Some(balance_manager),
        }
    }

    pub async fn escrow(&self, message_id: MessageId, proposer: PublicKey) -> Result<()> {
        // If we have a balance manager, check and lock balance
        if let Some(ref balance_mgr) = self.balance_manager {
            let proposer_addr = AccountAddress::from_public_key(&proposer);

            // Check proposer has sufficient balance
            let balance = balance_mgr
                .get_unlocked_balance(proposer_addr)
                .await
                .map_err(|e| {
                    AdicError::InvalidParameter(format!("Failed to get balance: {}", e))
                })?;

            if balance < self.deposit_amount {
                return Err(AdicError::InvalidParameter(format!(
                    "Insufficient balance for deposit: has {}, needs {}",
                    balance, self.deposit_amount
                )));
            }

            // Lock the deposit amount
            balance_mgr
                .lock(proposer_addr, self.deposit_amount)
                .await
                .map_err(|e| {
                    AdicError::InvalidParameter(format!("Failed to lock deposit: {}", e))
                })?;
        }

        // Record the deposit
        let deposit = Deposit {
            message_id,
            proposer_pk: proposer,
            amount: self.deposit_amount,
            status: DepositStatus::Escrowed,
            timestamp: chrono::Utc::now().timestamp(),
        };

        let mut deposits = self.deposits.write().await;
        deposits.insert(message_id, deposit);

        info!(
            "Escrowed {} for message {}",
            self.deposit_amount, message_id
        );

        Ok(())
    }

    pub async fn refund(&self, message_id: &MessageId) -> Result<()> {
        let mut deposits = self.deposits.write().await;

        match deposits.get_mut(message_id) {
            Some(deposit) => {
                if deposit.status != DepositStatus::Escrowed {
                    return Err(AdicError::InvalidParameter(format!(
                        "Deposit for {} is not in escrowed state",
                        message_id
                    )));
                }

                // If we have a balance manager, unlock the deposit
                if let Some(ref balance_mgr) = self.balance_manager {
                    let proposer_addr = AccountAddress::from_public_key(&deposit.proposer_pk);

                    balance_mgr
                        .unlock(proposer_addr, deposit.amount)
                        .await
                        .map_err(|e| {
                            AdicError::InvalidParameter(format!("Failed to unlock deposit: {}", e))
                        })?;
                }

                deposit.status = DepositStatus::Refunded;

                info!("Refunded {} for message {}", deposit.amount, message_id);

                Ok(())
            }
            None => Err(AdicError::InvalidParameter(format!(
                "No deposit found for message {}",
                message_id
            ))),
        }
    }

    pub async fn slash(&self, message_id: &MessageId, reason: &str) -> Result<()> {
        let mut deposits = self.deposits.write().await;

        match deposits.get_mut(message_id) {
            Some(deposit) => {
                if deposit.status != DepositStatus::Escrowed {
                    return Err(AdicError::InvalidParameter(format!(
                        "Deposit for {} is not in escrowed state",
                        message_id
                    )));
                }

                // If we have a balance manager, handle the slashing
                if let Some(ref balance_mgr) = self.balance_manager {
                    let proposer_addr = AccountAddress::from_public_key(&deposit.proposer_pk);

                    // Unlock the deposit (it was locked)
                    balance_mgr
                        .unlock(proposer_addr, deposit.amount)
                        .await
                        .map_err(|e| {
                            AdicError::InvalidParameter(format!(
                                "Failed to unlock for slash: {}",
                                e
                            ))
                        })?;

                    // Transfer slashed amount to treasury
                    let treasury_addr = AccountAddress::treasury();
                    balance_mgr
                        .transfer(proposer_addr, treasury_addr, deposit.amount)
                        .await
                        .map_err(|e| {
                            AdicError::InvalidParameter(format!(
                                "Failed to transfer slashed funds: {}",
                                e
                            ))
                        })?;
                }

                deposit.status = DepositStatus::Slashed;

                warn!("Slashed {} for {}: {}", deposit.amount, message_id, reason);

                Ok(())
            }
            None => Err(AdicError::InvalidParameter(format!(
                "No deposit found for message {}",
                message_id
            ))),
        }
    }

    pub async fn get_state(&self, message_id: &MessageId) -> Option<DepositState> {
        let deposits = self.deposits.read().await;
        deposits.get(message_id).map(|d| d.status.clone())
    }

    pub async fn get_proposer(&self, message_id: &MessageId) -> Option<PublicKey> {
        let deposits = self.deposits.read().await;
        deposits.get(message_id).map(|d| d.proposer_pk)
    }

    pub async fn get_escrowed_count(&self) -> usize {
        let deposits = self.deposits.read().await;
        deposits
            .values()
            .filter(|d| d.status == DepositStatus::Escrowed)
            .count()
    }

    pub async fn get_total_escrowed(&self) -> AdicAmount {
        let deposits = self.deposits.read().await;
        deposits
            .values()
            .filter(|d| d.status == DepositStatus::Escrowed)
            .map(|d| d.amount)
            .fold(AdicAmount::ZERO, |acc, amt| acc.saturating_add(amt))
    }

    pub async fn get_deposit(&self, message_id: &MessageId) -> Result<Option<Deposit>> {
        let deposits = self.deposits.read().await;
        Ok(deposits.get(message_id).cloned())
    }

    pub async fn get_recent_deposits(&self, limit: usize) -> Vec<Deposit> {
        let deposits = self.deposits.read().await;
        let mut recent: Vec<Deposit> = deposits.values().cloned().collect();
        recent.sort_by_key(|d| -d.timestamp);
        recent.truncate(limit);
        recent
    }

    pub async fn get_stats(&self) -> DepositStats {
        let deposits = self.deposits.read().await;

        let mut escrowed_count = 0;
        let mut total_escrowed = AdicAmount::ZERO;
        let mut slashed_count = 0;
        let mut total_slashed = AdicAmount::ZERO;
        let mut refunded_count = 0;
        let mut total_refunded = AdicAmount::ZERO;

        for deposit in deposits.values() {
            match deposit.status {
                DepositStatus::Escrowed => {
                    escrowed_count += 1;
                    total_escrowed = total_escrowed.saturating_add(deposit.amount);
                }
                DepositStatus::Slashed => {
                    slashed_count += 1;
                    total_slashed = total_slashed.saturating_add(deposit.amount);
                }
                DepositStatus::Refunded => {
                    refunded_count += 1;
                    total_refunded = total_refunded.saturating_add(deposit.amount);
                }
            }
        }

        DepositStats {
            escrowed_count,
            total_escrowed,
            slashed_count,
            total_slashed,
            refunded_count,
            total_refunded,
        }
    }

    pub async fn set_deposit_amount(&mut self, amount: AdicAmount) {
        self.deposit_amount = amount;
        info!("Deposit amount updated to {}", amount);
    }

    pub fn get_deposit_amount(&self) -> AdicAmount {
        self.deposit_amount
    }

    pub async fn cleanup_old_deposits(&self, older_than_days: i64) {
        let mut deposits = self.deposits.write().await;
        let now = chrono::Utc::now().timestamp();
        let cutoff = now - (older_than_days * 24 * 3600);

        let to_remove: Vec<MessageId> = deposits
            .iter()
            .filter(|(_, d)| d.status != DepositStatus::Escrowed && d.timestamp < cutoff)
            .map(|(id, _)| *id)
            .collect();

        let num_removed = to_remove.len();

        for id in to_remove {
            deposits.remove(&id);
        }

        if num_removed > 0 {
            info!("Cleaned up {} old deposits", num_removed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_economics::storage::MemoryStorage;

    #[tokio::test]
    async fn test_deposit_lifecycle() {
        let manager = DepositManager::new(10.0);
        let msg_id = MessageId::new(b"test");
        let proposer = PublicKey::from_bytes([1; 32]);

        // Escrow
        assert!(manager.escrow(msg_id, proposer).await.is_ok());
        assert_eq!(
            manager.get_state(&msg_id).await,
            Some(DepositState::Escrowed)
        );

        // Refund
        assert!(manager.refund(&msg_id).await.is_ok());
        assert_eq!(
            manager.get_state(&msg_id).await,
            Some(DepositState::Refunded)
        );
    }

    #[tokio::test]
    async fn test_deposit_with_balance_check() {
        let storage = Arc::new(MemoryStorage::new());
        let balance_manager = Arc::new(BalanceManager::new(storage));

        let deposit_amount = AdicAmount::from_adic(10.0);
        let manager = DepositManager::with_balance_manager(deposit_amount, balance_manager.clone());

        let msg_id = MessageId::new(b"test");
        let proposer = PublicKey::from_bytes([1; 32]);
        let proposer_addr = AccountAddress::from_public_key(&proposer);

        // Should fail without balance
        assert!(manager.escrow(msg_id, proposer).await.is_err());

        // Credit account
        balance_manager
            .credit(proposer_addr, AdicAmount::from_adic(100.0))
            .await
            .unwrap();

        // Now escrow should work
        assert!(manager.escrow(msg_id, proposer).await.is_ok());
        assert_eq!(
            manager.get_state(&msg_id).await,
            Some(DepositState::Escrowed)
        );

        // Check locked balance
        let locked = balance_manager
            .get_locked_balance(proposer_addr)
            .await
            .unwrap();
        assert_eq!(locked, deposit_amount);

        // Refund should unlock
        manager.refund(&msg_id).await.unwrap();
        let locked_after = balance_manager
            .get_locked_balance(proposer_addr)
            .await
            .unwrap();
        assert_eq!(locked_after, AdicAmount::ZERO);
    }

    #[tokio::test]
    async fn test_slash_to_treasury() {
        let storage = Arc::new(MemoryStorage::new());
        let balance_manager = Arc::new(BalanceManager::new(storage));

        let deposit_amount = AdicAmount::from_adic(10.0);
        let manager = DepositManager::with_balance_manager(deposit_amount, balance_manager.clone());

        let msg_id = MessageId::new(b"test");
        let proposer = PublicKey::from_bytes([2; 32]);
        let proposer_addr = AccountAddress::from_public_key(&proposer);

        // Fund account and escrow
        balance_manager
            .credit(proposer_addr, AdicAmount::from_adic(100.0))
            .await
            .unwrap();
        manager.escrow(msg_id, proposer).await.unwrap();

        // Slash the deposit
        manager.slash(&msg_id, "Invalid signature").await.unwrap();

        // Check treasury received the funds
        let treasury_balance = balance_manager
            .get_balance(AccountAddress::treasury())
            .await
            .unwrap();
        assert_eq!(treasury_balance, deposit_amount);

        // Check proposer lost the funds
        let proposer_balance = balance_manager.get_balance(proposer_addr).await.unwrap();
        assert_eq!(proposer_balance, AdicAmount::from_adic(90.0));
    }

    #[tokio::test]
    async fn test_deposit_stats() {
        let manager = DepositManager::new(10.0);

        // Create multiple deposits
        for i in 0..5 {
            let msg_id = MessageId::new(&[i; 32]);
            let proposer = PublicKey::from_bytes([i; 32]);
            manager.escrow(msg_id, proposer).await.unwrap();
        }

        // Refund one
        let msg_id_0 = MessageId::new(&[0; 32]);
        manager.refund(&msg_id_0).await.unwrap();

        // Slash one
        let msg_id_1 = MessageId::new(&[1; 32]);
        manager.slash(&msg_id_1, "test").await.unwrap();

        let stats = manager.get_stats().await;
        assert_eq!(stats.escrowed_count, 3);
        assert_eq!(stats.refunded_count, 1);
        assert_eq!(stats.slashed_count, 1);
    }
}
