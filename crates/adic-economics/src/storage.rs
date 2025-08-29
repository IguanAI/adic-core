use crate::types::{AccountAddress, AdicAmount};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// Type aliases for complex types
type BalanceMap = HashMap<AccountAddress, AdicAmount>;
type TransactionBackup = Option<(BalanceMap, BalanceMap)>;

#[async_trait]
pub trait EconomicsStorage: Send + Sync {
    async fn get_balance(&self, address: AccountAddress) -> Result<AdicAmount>;
    async fn set_balance(&self, address: AccountAddress, balance: AdicAmount) -> Result<()>;
    async fn get_locked_balance(&self, address: AccountAddress) -> Result<AdicAmount>;
    async fn set_locked_balance(&self, address: AccountAddress, locked: AdicAmount) -> Result<()>;
    async fn get_all_accounts(&self) -> Result<Vec<AccountAddress>>;

    async fn begin_transaction(&self) -> Result<()>;
    async fn commit_transaction(&self) -> Result<()>;
    async fn rollback_transaction(&self) -> Result<()>;
}

pub struct MemoryStorage {
    balances: Arc<RwLock<BalanceMap>>,
    locked_balances: Arc<RwLock<BalanceMap>>,
    transaction_backup: Arc<RwLock<TransactionBackup>>,
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            balances: Arc::new(RwLock::new(HashMap::new())),
            locked_balances: Arc::new(RwLock::new(HashMap::new())),
            transaction_backup: Arc::new(RwLock::new(None)),
        }
    }
}

#[async_trait]
impl EconomicsStorage for MemoryStorage {
    async fn get_balance(&self, address: AccountAddress) -> Result<AdicAmount> {
        let balances = self.balances.read().await;
        Ok(balances.get(&address).copied().unwrap_or(AdicAmount::ZERO))
    }

    async fn set_balance(&self, address: AccountAddress, balance: AdicAmount) -> Result<()> {
        let mut balances = self.balances.write().await;
        if balance == AdicAmount::ZERO {
            balances.remove(&address);
        } else {
            balances.insert(address, balance);
        }
        Ok(())
    }

    async fn get_locked_balance(&self, address: AccountAddress) -> Result<AdicAmount> {
        let locked = self.locked_balances.read().await;
        Ok(locked.get(&address).copied().unwrap_or(AdicAmount::ZERO))
    }

    async fn set_locked_balance(&self, address: AccountAddress, locked: AdicAmount) -> Result<()> {
        let mut locked_balances = self.locked_balances.write().await;
        if locked == AdicAmount::ZERO {
            locked_balances.remove(&address);
        } else {
            locked_balances.insert(address, locked);
        }
        Ok(())
    }

    async fn get_all_accounts(&self) -> Result<Vec<AccountAddress>> {
        let balances = self.balances.read().await;
        let locked = self.locked_balances.read().await;

        let mut accounts: Vec<AccountAddress> = balances.keys().copied().collect();
        for addr in locked.keys() {
            if !balances.contains_key(addr) {
                accounts.push(*addr);
            }
        }

        Ok(accounts)
    }

    async fn begin_transaction(&self) -> Result<()> {
        let balances = self.balances.read().await;
        let locked = self.locked_balances.read().await;

        let mut backup = self.transaction_backup.write().await;
        *backup = Some((balances.clone(), locked.clone()));

        Ok(())
    }

    async fn commit_transaction(&self) -> Result<()> {
        let mut backup = self.transaction_backup.write().await;
        *backup = None;
        Ok(())
    }

    async fn rollback_transaction(&self) -> Result<()> {
        let mut backup = self.transaction_backup.write().await;

        if let Some((balance_backup, locked_backup)) = backup.take() {
            let mut balances = self.balances.write().await;
            let mut locked = self.locked_balances.write().await;

            *balances = balance_backup;
            *locked = locked_backup;
        }

        Ok(())
    }
}

#[cfg(feature = "rocksdb")]
pub struct RocksDbStorage {
    db: Arc<rocksdb::DB>,
    cf_balances: String,
    cf_locked: String,
    _cf_supply: String,
    _cf_emissions: String,
    _cf_treasury: String,
}

#[cfg(feature = "rocksdb")]
impl RocksDbStorage {
    pub fn new(path: &str) -> Result<Self> {
        use rocksdb::{Options, DB};

        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cf_names = vec![
            "balances",
            "locked_balances",
            "supply_metrics",
            "emission_schedule",
            "treasury_proposals",
        ];

        let db = DB::open_cf(&opts, path, &cf_names)?;

        Ok(Self {
            db: Arc::new(db),
            cf_balances: "balances".to_string(),
            cf_locked: "locked_balances".to_string(),
            _cf_supply: "supply_metrics".to_string(),
            _cf_emissions: "emission_schedule".to_string(),
            _cf_treasury: "treasury_proposals".to_string(),
        })
    }
}

#[cfg(feature = "rocksdb")]
#[async_trait]
impl EconomicsStorage for RocksDbStorage {
    async fn get_balance(&self, address: AccountAddress) -> Result<AdicAmount> {
        let cf = self
            .db
            .cf_handle(&self.cf_balances)
            .ok_or_else(|| anyhow::anyhow!("Column family not found"))?;

        match self.db.get_cf(cf, address.as_bytes())? {
            Some(bytes) => {
                let value = u64::from_le_bytes(bytes.as_slice().try_into()?);
                Ok(AdicAmount::from_base_units(value))
            }
            None => Ok(AdicAmount::ZERO),
        }
    }

    async fn set_balance(&self, address: AccountAddress, balance: AdicAmount) -> Result<()> {
        let cf = self
            .db
            .cf_handle(&self.cf_balances)
            .ok_or_else(|| anyhow::anyhow!("Column family not found"))?;

        if balance == AdicAmount::ZERO {
            self.db.delete_cf(cf, address.as_bytes())?;
        } else {
            let value = balance.to_base_units().to_le_bytes();
            self.db.put_cf(cf, address.as_bytes(), value)?;
        }

        Ok(())
    }

    async fn get_locked_balance(&self, address: AccountAddress) -> Result<AdicAmount> {
        let cf = self
            .db
            .cf_handle(&self.cf_locked)
            .ok_or_else(|| anyhow::anyhow!("Column family not found"))?;

        match self.db.get_cf(cf, address.as_bytes())? {
            Some(bytes) => {
                let value = u64::from_le_bytes(bytes.as_slice().try_into()?);
                Ok(AdicAmount::from_base_units(value))
            }
            None => Ok(AdicAmount::ZERO),
        }
    }

    async fn set_locked_balance(&self, address: AccountAddress, locked: AdicAmount) -> Result<()> {
        let cf = self
            .db
            .cf_handle(&self.cf_locked)
            .ok_or_else(|| anyhow::anyhow!("Column family not found"))?;

        if locked == AdicAmount::ZERO {
            self.db.delete_cf(cf, address.as_bytes())?;
        } else {
            let value = locked.to_base_units().to_le_bytes();
            self.db.put_cf(cf, address.as_bytes(), value)?;
        }

        Ok(())
    }

    async fn get_all_accounts(&self) -> Result<Vec<AccountAddress>> {
        use rocksdb::IteratorMode;

        let mut accounts = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Iterate through balances
        let cf_balances = self
            .db
            .cf_handle(&self.cf_balances)
            .ok_or_else(|| anyhow::anyhow!("Column family not found"))?;

        for item in self.db.iterator_cf(cf_balances, IteratorMode::Start) {
            let (key, _) = item?;
            if key.len() == 32 {
                let mut addr_bytes = [0u8; 32];
                addr_bytes.copy_from_slice(&key);
                let addr = AccountAddress::from_bytes(addr_bytes);
                if seen.insert(addr) {
                    accounts.push(addr);
                }
            }
        }

        // Iterate through locked balances
        let cf_locked = self
            .db
            .cf_handle(&self.cf_locked)
            .ok_or_else(|| anyhow::anyhow!("Column family not found"))?;

        for item in self.db.iterator_cf(cf_locked, IteratorMode::Start) {
            let (key, _) = item?;
            if key.len() == 32 {
                let mut addr_bytes = [0u8; 32];
                addr_bytes.copy_from_slice(&key);
                let addr = AccountAddress::from_bytes(addr_bytes);
                if seen.insert(addr) {
                    accounts.push(addr);
                }
            }
        }

        Ok(accounts)
    }

    async fn begin_transaction(&self) -> Result<()> {
        // RocksDB has built-in transaction support
        // For now, we'll use WriteBatch for atomic operations
        Ok(())
    }

    async fn commit_transaction(&self) -> Result<()> {
        // Flush any pending writes
        self.db.flush()?;
        Ok(())
    }

    async fn rollback_transaction(&self) -> Result<()> {
        // In a real implementation, we'd use RocksDB transactions
        // For now, this is a no-op
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_storage() {
        let storage = MemoryStorage::new();
        let addr = AccountAddress::from_bytes([1; 32]);

        // Initial balance should be zero
        assert_eq!(storage.get_balance(addr).await.unwrap(), AdicAmount::ZERO);

        // Set balance
        let amount = AdicAmount::from_adic(100.0);
        storage.set_balance(addr, amount).await.unwrap();
        assert_eq!(storage.get_balance(addr).await.unwrap(), amount);

        // Test locked balance
        let locked = AdicAmount::from_adic(30.0);
        storage.set_locked_balance(addr, locked).await.unwrap();
        assert_eq!(storage.get_locked_balance(addr).await.unwrap(), locked);

        // Test account listing
        let accounts = storage.get_all_accounts().await.unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0], addr);
    }

    #[tokio::test]
    async fn test_transaction_rollback() {
        let storage = MemoryStorage::new();
        let addr = AccountAddress::from_bytes([2; 32]);
        let initial = AdicAmount::from_adic(100.0);

        storage.set_balance(addr, initial).await.unwrap();

        // Begin transaction
        storage.begin_transaction().await.unwrap();

        // Modify balance
        storage
            .set_balance(addr, AdicAmount::from_adic(200.0))
            .await
            .unwrap();
        assert_eq!(
            storage.get_balance(addr).await.unwrap(),
            AdicAmount::from_adic(200.0)
        );

        // Rollback
        storage.rollback_transaction().await.unwrap();

        // Balance should be restored
        assert_eq!(storage.get_balance(addr).await.unwrap(), initial);
    }
}
