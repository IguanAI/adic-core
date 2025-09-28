use crate::storage::{EconomicsStorage, TransactionRecord};
use crate::types::{AccountAddress, AdicAmount};
use anyhow::{bail, Result};
use blake3;
use chrono::Utc;
use hex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub address: AccountAddress,
    pub balance: AdicAmount,
    pub nonce: u64,
    pub locked_balance: AdicAmount,
    pub last_activity: i64,
}

pub struct BalanceManager {
    storage: Arc<dyn EconomicsStorage>,
    cache: Arc<RwLock<HashMap<AccountAddress, AccountInfo>>>,
}

impl BalanceManager {
    pub fn new(storage: Arc<dyn EconomicsStorage>) -> Self {
        Self {
            storage,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_balance(&self, address: AccountAddress) -> Result<AdicAmount> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(info) = cache.get(&address) {
                return Ok(info.balance);
            }
        }

        // Load from storage
        let balance = self.storage.get_balance(address).await?;

        // Update cache
        let mut cache = self.cache.write().await;
        cache.insert(
            address,
            AccountInfo {
                address,
                balance,
                nonce: 0,
                locked_balance: AdicAmount::ZERO,
                last_activity: chrono::Utc::now().timestamp(),
            },
        );

        Ok(balance)
    }

    pub async fn credit(&self, address: AccountAddress, amount: AdicAmount) -> Result<()> {
        if amount == AdicAmount::ZERO {
            return Ok(());
        }

        let current = self.get_balance(address).await?;
        let new_balance = current
            .checked_add(amount)
            .ok_or_else(|| anyhow::anyhow!("Balance overflow for {}", address))?;

        // Ensure doesn't exceed max supply
        if new_balance > AdicAmount::MAX_SUPPLY {
            bail!("Balance would exceed max supply");
        }

        // Update storage
        self.storage.set_balance(address, new_balance).await?;

        // Update cache
        let mut cache = self.cache.write().await;
        if let Some(info) = cache.get_mut(&address) {
            info.balance = new_balance;
            info.last_activity = chrono::Utc::now().timestamp();
        } else {
            cache.insert(
                address,
                AccountInfo {
                    address,
                    balance: new_balance,
                    nonce: 0,
                    locked_balance: AdicAmount::ZERO,
                    last_activity: chrono::Utc::now().timestamp(),
                },
            );
        }

        info!(
            address = %address,
            amount = amount.to_adic(),
            balance_before = current.to_adic(),
            balance_after = new_balance.to_adic(),
            "💰 Balance credited"
        );
        Ok(())
    }

    pub async fn debit(&self, address: AccountAddress, amount: AdicAmount) -> Result<()> {
        if amount == AdicAmount::ZERO {
            return Ok(());
        }

        let current = self.get_balance(address).await?;
        let new_balance = current.checked_sub(amount).ok_or_else(|| {
            anyhow::anyhow!(
                "Insufficient balance for {}: has {}, needs {}",
                address,
                current,
                amount
            )
        })?;

        // Update storage
        self.storage.set_balance(address, new_balance).await?;

        // Update cache
        let mut cache = self.cache.write().await;
        if let Some(info) = cache.get_mut(&address) {
            info.balance = new_balance;
            info.last_activity = chrono::Utc::now().timestamp();
        }

        info!(
            address = %address,
            amount = amount.to_adic(),
            balance_before = current.to_adic(),
            balance_after = new_balance.to_adic(),
            "💸 Balance debited"
        );
        Ok(())
    }

    pub async fn transfer(
        &self,
        from: AccountAddress,
        to: AccountAddress,
        amount: AdicAmount,
    ) -> Result<()> {
        if amount == AdicAmount::ZERO {
            return Ok(());
        }

        if from == to {
            bail!("Cannot transfer to same address");
        }

        // Atomic transfer using storage transaction
        info!(
            from = %from,
            to = %to,
            amount = amount.to_adic(),
            "📝 Beginning transfer transaction"
        );
        self.storage.begin_transaction().await?;

        match self.transfer_internal(from, to, amount).await {
            Ok(tx_hash) => {
                self.storage.commit_transaction().await?;

                // Record the successful transaction
                let tx_record = TransactionRecord {
                    from,
                    to,
                    amount,
                    timestamp: Utc::now(),
                    tx_hash: tx_hash.clone(),
                    status: "confirmed".to_string(),
                };

                // Record transaction (ignore errors to not fail the transfer)
                if let Err(e) = self.storage.record_transaction(tx_record).await {
                    debug!(
                        tx_hash = %tx_hash,
                        error = %e,
                        "Failed to record transaction"
                    );
                }

                info!(
                    from = %from,
                    to = %to,
                    amount = amount.to_adic(),
                    tx_hash = %tx_hash,
                    status = "confirmed",
                    "✅ Transfer committed"
                );
                Ok(())
            }
            Err(e) => {
                info!(
                    from = %from,
                    to = %to,
                    amount = amount.to_adic(),
                    error = %e,
                    "❌ Transfer rolled back"
                );
                self.storage.rollback_transaction().await?;
                Err(e)
            }
        }
    }

    async fn transfer_internal(
        &self,
        from: AccountAddress,
        to: AccountAddress,
        amount: AdicAmount,
    ) -> Result<String> {
        // Lock cache for the entire transfer to ensure atomicity
        let mut cache = self.cache.write().await;

        // Get current balances from storage (not cache) for consistency
        let from_balance = self.storage.get_balance(from).await?;
        if from_balance < amount {
            bail!(
                "Insufficient balance: {} has {}, needs {}",
                from,
                from_balance,
                amount
            );
        }

        let to_balance = self.storage.get_balance(to).await?;

        // Calculate new balances
        let new_from_balance = from_balance.saturating_sub(amount);
        let new_to_balance = to_balance
            .checked_add(amount)
            .ok_or_else(|| anyhow::anyhow!("Balance overflow for recipient"))?;

        info!(
            from = %from,
            to = %to,
            amount = amount.to_adic(),
            from_balance_before = from_balance.to_adic(),
            from_balance_after = new_from_balance.to_adic(),
            to_balance_before = to_balance.to_adic(),
            to_balance_after = new_to_balance.to_adic(),
            "💸 Executing transfer"
        );

        // Update storage atomically
        self.storage.set_balance(from, new_from_balance).await?;
        self.storage.set_balance(to, new_to_balance).await?;

        // Update cache (already locked above)
        let now = chrono::Utc::now().timestamp();

        cache
            .entry(from)
            .and_modify(|info| {
                info.balance = new_from_balance;
                info.last_activity = now;
            })
            .or_insert(AccountInfo {
                address: from,
                balance: new_from_balance,
                nonce: 0,
                locked_balance: AdicAmount::ZERO,
                last_activity: now,
            });

        cache
            .entry(to)
            .and_modify(|info| {
                info.balance = new_to_balance;
                info.last_activity = now;
            })
            .or_insert(AccountInfo {
                address: to,
                balance: new_to_balance,
                nonce: 0,
                locked_balance: AdicAmount::ZERO,
                last_activity: now,
            });

        // Generate transaction hash using Blake3 (cryptographically secure)
        let mut hasher = blake3::Hasher::new();
        hasher.update(from.as_bytes());
        hasher.update(to.as_bytes());
        hasher.update(&amount.to_base_units().to_le_bytes());
        hasher.update(&now.to_le_bytes());
        let tx_hash = hex::encode(hasher.finalize().as_bytes());

        Ok(tx_hash)
    }

    pub async fn lock(&self, address: AccountAddress, amount: AdicAmount) -> Result<()> {
        let mut cache = self.cache.write().await;

        let info = cache.entry(address).or_insert_with(|| AccountInfo {
            address,
            balance: AdicAmount::ZERO,
            nonce: 0,
            locked_balance: AdicAmount::ZERO,
            last_activity: chrono::Utc::now().timestamp(),
        });

        let old_locked = info.locked_balance;

        // Ensure sufficient unlocked balance
        let unlocked = info.balance.saturating_sub(info.locked_balance);
        if unlocked < amount {
            bail!(
                "Insufficient unlocked balance: has {}, needs {}",
                unlocked,
                amount
            );
        }

        info.locked_balance = info.locked_balance.saturating_add(amount);

        // Persist to storage
        self.storage
            .set_locked_balance(address, info.locked_balance)
            .await?;

        info!(
            address = %address,
            amount = amount.to_adic(),
            locked_before = old_locked.to_adic(),
            locked_after = info.locked_balance.to_adic(),
            total_balance = info.balance.to_adic(),
            "🔒 Balance locked"
        );
        Ok(())
    }

    pub async fn unlock(&self, address: AccountAddress, amount: AdicAmount) -> Result<()> {
        let mut cache = self.cache.write().await;

        let info = cache
            .get_mut(&address)
            .ok_or_else(|| anyhow::anyhow!("Account not found: {}", address))?;

        let old_locked = info.locked_balance;

        if info.locked_balance < amount {
            bail!(
                "Insufficient locked balance: has {}, trying to unlock {}",
                info.locked_balance,
                amount
            );
        }

        info.locked_balance = info.locked_balance.saturating_sub(amount);

        // Persist to storage
        self.storage
            .set_locked_balance(address, info.locked_balance)
            .await?;

        info!(
            address = %address,
            amount = amount.to_adic(),
            locked_before = old_locked.to_adic(),
            locked_after = info.locked_balance.to_adic(),
            total_balance = info.balance.to_adic(),
            "🔓 Balance unlocked"
        );
        Ok(())
    }

    pub async fn get_locked_balance(&self, address: AccountAddress) -> Result<AdicAmount> {
        let cache = self.cache.read().await;
        if let Some(info) = cache.get(&address) {
            Ok(info.locked_balance)
        } else {
            self.storage.get_locked_balance(address).await
        }
    }

    pub async fn get_unlocked_balance(&self, address: AccountAddress) -> Result<AdicAmount> {
        let balance = self.get_balance(address).await?;
        let locked = self.get_locked_balance(address).await?;
        Ok(balance.saturating_sub(locked))
    }

    pub async fn get_all_accounts(&self) -> Result<Vec<AccountInfo>> {
        let accounts = self.storage.get_all_accounts().await?;
        let mut result = Vec::new();

        for address in accounts {
            let balance = self.get_balance(address).await?;
            let locked = self.get_locked_balance(address).await?;

            result.push(AccountInfo {
                address,
                balance,
                nonce: 0,
                locked_balance: locked,
                last_activity: chrono::Utc::now().timestamp(),
            });
        }

        Ok(result)
    }

    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        let cache_size = cache.len();
        cache.clear();
        info!(entries_cleared = cache_size, "🧹 Balance cache cleared");
    }

    pub async fn get_transaction_history(
        &self,
        address: AccountAddress,
    ) -> Result<Vec<TransactionRecord>> {
        self.storage.get_transaction_history(address).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;

    #[tokio::test]
    async fn test_basic_operations() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = BalanceManager::new(storage);

        let addr1 = AccountAddress::from_bytes([1; 32]);
        let addr2 = AccountAddress::from_bytes([2; 32]);

        // Credit
        let amount = AdicAmount::from_adic(100.0);
        manager.credit(addr1, amount).await.unwrap();
        assert_eq!(manager.get_balance(addr1).await.unwrap(), amount);

        // Transfer
        let transfer_amount = AdicAmount::from_adic(30.0);
        manager
            .transfer(addr1, addr2, transfer_amount)
            .await
            .unwrap();

        assert_eq!(
            manager.get_balance(addr1).await.unwrap(),
            AdicAmount::from_adic(70.0)
        );
        assert_eq!(
            manager.get_balance(addr2).await.unwrap(),
            AdicAmount::from_adic(30.0)
        );

        // Debit
        manager
            .debit(addr1, AdicAmount::from_adic(20.0))
            .await
            .unwrap();
        assert_eq!(
            manager.get_balance(addr1).await.unwrap(),
            AdicAmount::from_adic(50.0)
        );
    }

    #[tokio::test]
    async fn test_locking() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = BalanceManager::new(storage);

        let addr = AccountAddress::from_bytes([3; 32]);
        let total = AdicAmount::from_adic(100.0);

        manager.credit(addr, total).await.unwrap();

        // Lock some balance
        let lock_amount = AdicAmount::from_adic(40.0);
        manager.lock(addr, lock_amount).await.unwrap();

        assert_eq!(manager.get_locked_balance(addr).await.unwrap(), lock_amount);
        assert_eq!(
            manager.get_unlocked_balance(addr).await.unwrap(),
            AdicAmount::from_adic(60.0)
        );

        // Cannot lock more than available
        assert!(manager
            .lock(addr, AdicAmount::from_adic(70.0))
            .await
            .is_err());

        // Unlock
        manager
            .unlock(addr, AdicAmount::from_adic(20.0))
            .await
            .unwrap();
        assert_eq!(
            manager.get_locked_balance(addr).await.unwrap(),
            AdicAmount::from_adic(20.0)
        );
    }

    #[tokio::test]
    async fn test_insufficient_balance() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = BalanceManager::new(storage);

        let addr1 = AccountAddress::from_bytes([4; 32]);
        let addr2 = AccountAddress::from_bytes([5; 32]);

        manager
            .credit(addr1, AdicAmount::from_adic(50.0))
            .await
            .unwrap();

        // Try to transfer more than balance
        assert!(manager
            .transfer(addr1, addr2, AdicAmount::from_adic(100.0))
            .await
            .is_err());

        // Balance should remain unchanged
        assert_eq!(
            manager.get_balance(addr1).await.unwrap(),
            AdicAmount::from_adic(50.0)
        );
        assert_eq!(manager.get_balance(addr2).await.unwrap(), AdicAmount::ZERO);
    }
}
