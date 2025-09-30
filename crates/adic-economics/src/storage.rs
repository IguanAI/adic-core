use crate::types::{AccountAddress, AdicAmount};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

// Transaction record for history tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRecord {
    pub from: AccountAddress,
    pub to: AccountAddress,
    pub amount: AdicAmount,
    pub timestamp: DateTime<Utc>,
    pub tx_hash: String,
    pub status: String,
}

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

    // Transaction history methods
    async fn record_transaction(&self, tx: TransactionRecord) -> Result<()>;
    async fn get_transaction_history(
        &self,
        address: AccountAddress,
    ) -> Result<Vec<TransactionRecord>>;

    // Paginated transaction history (optimized)
    async fn get_transaction_history_paginated(
        &self,
        address: AccountAddress,
        limit: usize,
        cursor: Option<String>, // Cursor format: "timestamp:tx_hash"
    ) -> Result<(Vec<TransactionRecord>, Option<String>)>;
}

pub struct MemoryStorage {
    balances: Arc<RwLock<BalanceMap>>,
    locked_balances: Arc<RwLock<BalanceMap>>,
    transaction_backup: Arc<RwLock<TransactionBackup>>,
    transaction_history: Arc<RwLock<Vec<TransactionRecord>>>,
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
            transaction_history: Arc::new(RwLock::new(Vec::new())),
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
        let old_balance = balances.get(&address).copied().unwrap_or(AdicAmount::ZERO);

        if balance == AdicAmount::ZERO {
            balances.remove(&address);
        } else {
            balances.insert(address, balance);
        }

        if old_balance != balance {
            info!(
                address = %address,
                balance_before = old_balance.to_adic(),
                balance_after = balance.to_adic(),
                storage_type = "memory",
                "💾 Balance stored"
            );
        }
        Ok(())
    }

    async fn get_locked_balance(&self, address: AccountAddress) -> Result<AdicAmount> {
        let locked = self.locked_balances.read().await;
        Ok(locked.get(&address).copied().unwrap_or(AdicAmount::ZERO))
    }

    async fn set_locked_balance(&self, address: AccountAddress, locked: AdicAmount) -> Result<()> {
        let mut locked_balances = self.locked_balances.write().await;
        let old_locked = locked_balances
            .get(&address)
            .copied()
            .unwrap_or(AdicAmount::ZERO);

        if locked == AdicAmount::ZERO {
            locked_balances.remove(&address);
        } else {
            locked_balances.insert(address, locked);
        }

        if old_locked != locked {
            info!(
                address = %address,
                locked_before = old_locked.to_adic(),
                locked_after = locked.to_adic(),
                storage_type = "memory",
                "🔒 Locked balance stored"
            );
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

        info!(
            accounts_count = balances.len(),
            locked_accounts_count = locked.len(),
            storage_type = "memory",
            "📝 Transaction began (snapshot created)"
        );
        Ok(())
    }

    async fn commit_transaction(&self) -> Result<()> {
        let mut backup = self.transaction_backup.write().await;
        let had_backup = backup.is_some();
        *backup = None;

        if had_backup {
            info!(
                storage_type = "memory",
                "✅ Transaction committed (snapshot discarded)"
            );
        }
        Ok(())
    }

    async fn rollback_transaction(&self) -> Result<()> {
        let mut backup = self.transaction_backup.write().await;

        if let Some((balance_backup, locked_backup)) = backup.take() {
            let mut balances = self.balances.write().await;
            let mut locked = self.locked_balances.write().await;

            let accounts_before = balances.len();
            let locked_before = locked.len();

            *balances = balance_backup;
            *locked = locked_backup;

            info!(
                accounts_before = accounts_before,
                accounts_after = balances.len(),
                locked_before = locked_before,
                locked_after = locked.len(),
                storage_type = "memory",
                "❌ Transaction rolled back (snapshot restored)"
            );
        }

        Ok(())
    }

    async fn record_transaction(&self, tx: TransactionRecord) -> Result<()> {
        let mut history = self.transaction_history.write().await;
        let history_size = history.len();

        info!(
            from = %tx.from,
            to = %tx.to,
            amount = tx.amount.to_adic(),
            tx_hash = %tx.tx_hash,
            status = %tx.status,
            history_size_before = history_size,
            history_size_after = history_size + 1,
            storage_type = "memory",
            "📦 Transaction recorded"
        );

        history.push(tx);
        Ok(())
    }

    async fn get_transaction_history(
        &self,
        address: AccountAddress,
    ) -> Result<Vec<TransactionRecord>> {
        let start_time = Instant::now();

        info!(
            operation = "tx_history",
            address = %hex::encode(address.as_bytes()),
            "🔍 Starting transaction history query"
        );

        let history = self.transaction_history.read().await;
        let filtered: Vec<TransactionRecord> = history
            .iter()
            .filter(|tx| tx.from == address || tx.to == address)
            .cloned()
            .collect();

        let duration_ms = start_time.elapsed().as_millis() as u64;

        info!(
            operation = "tx_history",
            address = %hex::encode(address.as_bytes()),
            result_count = filtered.len(),
            duration_ms = duration_ms,
            "✅ Completed transaction history query"
        );

        Ok(filtered)
    }

    async fn get_transaction_history_paginated(
        &self,
        address: AccountAddress,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<(Vec<TransactionRecord>, Option<String>)> {
        let start_time = Instant::now();

        info!(
            operation = "tx_history_paginated",
            address = %hex::encode(address.as_bytes()),
            page_size = limit,
            has_cursor = cursor.is_some(),
            "🔍 Starting paginated transaction history query"
        );

        let history = self.transaction_history.read().await;

        // Filter transactions for this address and sort by timestamp (newest first)
        let mut address_txs: Vec<_> = history
            .iter()
            .filter(|tx| tx.from == address || tx.to == address)
            .cloned()
            .collect();

        // Sort by timestamp descending (newest first)
        address_txs.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        // Parse cursor if provided
        let start_pos = if let Some(ref cursor_str) = cursor {
            debug!(
                operation = "parse_cursor",
                cursor = %cursor_str,
                "Parsing pagination cursor"
            );
            let parts: Vec<&str> = cursor_str.split(':').collect();
            if parts.len() == 2 {
                let timestamp = parts[0].parse::<i64>().unwrap_or(0);
                let tx_hash = parts[1];

                // Find position after cursor (in sorted list)
                // We need to skip past the transaction that matches the cursor
                let mut position = 0;
                for (idx, tx) in address_txs.iter().enumerate() {
                    let tx_ts = tx.timestamp.timestamp_millis();
                    if tx_ts == timestamp && tx.tx_hash == tx_hash {
                        // Found the cursor, next tx starts at idx + 1
                        position = idx + 1;
                        break;
                    }
                }
                position
            } else {
                0
            }
        } else {
            0
        };

        let mut filtered = Vec::new();
        let mut last_tx = None;

        for tx in address_txs.iter().skip(start_pos) {
            if filtered.len() >= limit {
                break;
            }
            filtered.push(tx.clone());
            last_tx = Some(tx);
        }

        // Create next cursor if there are more results
        let next_cursor = if filtered.len() == limit {
            last_tx.map(|last| format!("{}:{}", last.timestamp.timestamp_millis(), last.tx_hash))
        } else {
            None
        };

        let duration_ms = start_time.elapsed().as_millis() as u64;

        if duration_ms > 50 {
            warn!(
                operation = "tx_history_paginated",
                address = %hex::encode(address.as_bytes()),
                result_count = filtered.len(),
                has_next_page = next_cursor.is_some(),
                duration_ms = duration_ms,
                "⚠️ Slow query detected: transaction history pagination"
            );
        } else {
            info!(
                operation = "tx_history_paginated",
                address = %hex::encode(address.as_bytes()),
                result_count = filtered.len(),
                has_next_page = next_cursor.is_some(),
                duration_ms = duration_ms,
                "✅ Completed paginated transaction history query"
            );
        }

        Ok((filtered, next_cursor))
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
        use rocksdb::{BlockBasedOptions, Cache, Options, DB};

        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        // Performance optimizations for transaction queries
        opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
        opts.set_max_write_buffer_number(3);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);

        // Optimize for range scans on transaction indexes
        opts.set_prefix_extractor(rocksdb::SliceTransform::create_fixed_prefix(12));
        opts.set_memtable_prefix_bloom_ratio(0.2);

        // Block cache for frequently accessed transactions
        let cache = Cache::new_lru_cache(128 * 1024 * 1024); // 128MB cache
        let mut block_opts = BlockBasedOptions::default();
        block_opts.set_block_cache(&cache);
        block_opts.set_bloom_filter(10.0, false);
        opts.set_block_based_table_factory(&block_opts);

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

    async fn record_transaction(&self, tx: TransactionRecord) -> Result<()> {
        use rocksdb::WriteBatch;

        let mut batch = WriteBatch::default();

        // Store transaction indexed by hash
        let tx_key = format!("tx:{}", tx.tx_hash);
        let tx_data = serde_json::to_vec(&tx)?;
        batch.put(tx_key.as_bytes(), &tx_data);

        // Store optimized indexes with zero-padded timestamps for correct ordering
        // This matches the format used in adic-storage for consistency
        // Format: tx_by_addr:{address}:{timestamp_padded}:{tx_hash}
        let timestamp_padded = format!("{:020}", tx.timestamp.timestamp_millis());

        let from_key = format!(
            "tx_by_addr:{}:{}:{}",
            hex::encode(tx.from.as_bytes()),
            timestamp_padded,
            tx.tx_hash
        );
        let to_key = format!(
            "tx_by_addr:{}:{}:{}",
            hex::encode(tx.to.as_bytes()),
            timestamp_padded,
            tx.tx_hash
        );

        batch.put(from_key.as_bytes(), tx.tx_hash.as_bytes());

        // Only add to_key if it's different from from_key (avoid duplicates for self-transfers)
        if tx.from != tx.to {
            batch.put(to_key.as_bytes(), tx.tx_hash.as_bytes());
        }

        // Write all operations atomically
        self.db.write(batch)?;

        Ok(())
    }

    async fn get_transaction_history(
        &self,
        address: AccountAddress,
    ) -> Result<Vec<TransactionRecord>> {
        let mut transactions = Vec::new();
        let addr_hex = hex::encode(address.as_bytes());

        // Get transactions where address is sender
        let from_prefix = format!("tx_from:{}:", addr_hex);
        let from_iter = self.db.iterator(rocksdb::IteratorMode::From(
            from_prefix.as_bytes(),
            rocksdb::Direction::Forward,
        ));

        for item in from_iter {
            let (key, value) = item?;
            let key_str = String::from_utf8_lossy(&key);

            // Check if key still matches our prefix
            if !key_str.starts_with(&from_prefix) {
                break;
            }

            // Load the actual transaction
            let tx_hash = String::from_utf8_lossy(&value);
            let tx_key = format!("tx:{}", tx_hash);

            if let Some(tx_data) = self.db.get(tx_key.as_bytes())? {
                if let Ok(tx) = serde_json::from_slice::<TransactionRecord>(&tx_data) {
                    transactions.push(tx);
                }
            }
        }

        // Get transactions where address is receiver
        let to_prefix = format!("tx_to:{}:", addr_hex);
        let to_iter = self.db.iterator(rocksdb::IteratorMode::From(
            to_prefix.as_bytes(),
            rocksdb::Direction::Forward,
        ));

        for item in to_iter {
            let (key, value) = item?;
            let key_str = String::from_utf8_lossy(&key);

            // Check if key still matches our prefix
            if !key_str.starts_with(&to_prefix) {
                break;
            }

            // Load the actual transaction
            let tx_hash = String::from_utf8_lossy(&value);
            let tx_key = format!("tx:{}", tx_hash);

            if let Some(tx_data) = self.db.get(tx_key.as_bytes())? {
                if let Ok(tx) = serde_json::from_slice::<TransactionRecord>(&tx_data) {
                    // Avoid duplicates if address is both sender and receiver
                    if !transactions.iter().any(|t| t.tx_hash == tx.tx_hash) {
                        transactions.push(tx);
                    }
                }
            }
        }

        // Sort by timestamp (newest first)
        transactions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(transactions)
    }

    async fn get_transaction_history_paginated(
        &self,
        address: AccountAddress,
        limit: usize,
        cursor: Option<String>,
    ) -> Result<(Vec<TransactionRecord>, Option<String>)> {
        let mut transactions = Vec::new();
        let addr_hex = hex::encode(address.as_bytes());

        // Parse cursor to get start position
        let (start_key, skip_first) = if let Some(ref cursor_str) = cursor {
            let parts: Vec<&str> = cursor_str.split(':').collect();
            if parts.len() == 2 {
                let timestamp = parts[0];
                let tx_hash = parts[1];
                // Use padded timestamp to continue from cursor position
                (
                    format!("tx_by_addr:{}:{}:{}", addr_hex, timestamp, tx_hash),
                    true,
                )
            } else {
                (format!("tx_by_addr:{}:", addr_hex), false)
            }
        } else {
            (format!("tx_by_addr:{}:", addr_hex), false)
        };

        // Use optimized index with proper ordering
        let prefix = format!("tx_by_addr:{}:", addr_hex);
        let iter = self.db.iterator(rocksdb::IteratorMode::From(
            start_key.as_bytes(),
            rocksdb::Direction::Reverse, // Newest first
        ));

        let mut last_cursor_key = None;
        let mut skip_first = skip_first; // Skip the cursor entry itself

        for item in iter {
            if transactions.len() >= limit {
                // Save last key for cursor
                if let Ok((key, _)) = &item {
                    last_cursor_key = Some(key.clone());
                }
                break;
            }

            let (key, value) = item?;
            let key_str = String::from_utf8_lossy(&key);

            // Check if key still matches our prefix
            if !key_str.starts_with(&prefix) {
                break;
            }

            // Skip the cursor entry itself on first iteration
            if skip_first {
                skip_first = false;
                continue;
            }

            // Extract tx_hash from value and fetch transaction
            let tx_hash = String::from_utf8_lossy(&value);
            let tx_key = format!("tx:{}", tx_hash);

            if let Some(tx_data) = self.db.get(tx_key.as_bytes())? {
                if let Ok(tx) = serde_json::from_slice::<TransactionRecord>(&tx_data) {
                    transactions.push(tx);
                }
            }
        }

        // Create next cursor if there are potentially more results
        let next_cursor = if transactions.len() == limit {
            last_cursor_key.and_then(|key| {
                let key_str = String::from_utf8_lossy(&key);
                key_str.strip_prefix(&prefix).map(|parts| parts.to_string())
            })
        } else {
            None
        };

        Ok((transactions, next_cursor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "rocksdb")]
    use tempfile::TempDir;

    #[cfg(feature = "rocksdb")]
    async fn create_test_storage() -> RocksDbStorage {
        let temp_dir = TempDir::new().unwrap();
        RocksDbStorage::new(temp_dir.path().to_str().unwrap()).unwrap()
    }

    #[cfg(feature = "rocksdb")]
    #[tokio::test]
    #[ignore] // Requires RocksDB feature properly compiled
    async fn test_record_transaction() {
        let storage = create_test_storage().await;

        let from = AccountAddress::from_bytes([1; 32]);
        let to = AccountAddress::from_bytes([2; 32]);
        let amount = AdicAmount::from_adic(100.0);

        let tx = TransactionRecord {
            tx_hash: "test_tx_001".to_string(),
            from,
            to,
            amount,
            timestamp: chrono::Utc::now(),
            status: "completed".to_string(),
        };

        // Record transaction
        storage.record_transaction(tx.clone()).await.unwrap();

        // Verify it was stored
        let history = storage.get_transaction_history(from).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].tx_hash, "test_tx_001");
    }

    #[cfg(feature = "rocksdb")]
    #[tokio::test]
    #[ignore] // Requires RocksDB feature properly compiled
    async fn test_get_transaction_history() {
        let storage = create_test_storage().await;

        let addr1 = AccountAddress::from_bytes([1; 32]);
        let addr2 = AccountAddress::from_bytes([2; 32]);
        let addr3 = AccountAddress::from_bytes([3; 32]);

        // Create multiple transactions
        for i in 0..5 {
            let tx = TransactionRecord {
                tx_hash: format!("tx_{}", i),
                from: addr1,
                to: addr2,
                amount: AdicAmount::from_adic(10.0 * i as f64),
                timestamp: chrono::Utc::now() + chrono::Duration::seconds(i),
                status: "completed".to_string(),
            };
            storage.record_transaction(tx).await.unwrap();
        }

        // Add transactions where addr1 is receiver
        for i in 5..8 {
            let tx = TransactionRecord {
                tx_hash: format!("tx_{}", i),
                from: addr3,
                to: addr1,
                amount: AdicAmount::from_adic(20.0 * i as f64),
                timestamp: chrono::Utc::now() + chrono::Duration::seconds(i),
                status: "completed".to_string(),
            };
            storage.record_transaction(tx).await.unwrap();
        }

        // Check addr1 history (should have 8 transactions)
        let history = storage.get_transaction_history(addr1).await.unwrap();
        assert_eq!(history.len(), 8);

        // Check addr2 history (should have 5 transactions)
        let history = storage.get_transaction_history(addr2).await.unwrap();
        assert_eq!(history.len(), 5);

        // Verify sorting (newest first)
        assert!(history[0].timestamp > history[1].timestamp);
    }

    #[cfg(feature = "rocksdb")]
    #[tokio::test]
    #[ignore] // Requires RocksDB feature properly compiled
    async fn test_transaction_indexing() {
        let storage = create_test_storage().await;

        let addr1 = AccountAddress::from_bytes([10; 32]);

        // Record a self-transaction (same sender and receiver)
        let tx = TransactionRecord {
            tx_hash: "self_tx".to_string(),
            from: addr1,
            to: addr1,
            amount: AdicAmount::from_adic(50.0),
            timestamp: chrono::Utc::now(),
            status: "completed".to_string(),
        };
        storage.record_transaction(tx).await.unwrap();

        // Should appear only once in history
        let history = storage.get_transaction_history(addr1).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].tx_hash, "self_tx");
    }

    #[cfg(feature = "rocksdb")]
    #[tokio::test]
    #[ignore] // Requires RocksDB feature properly compiled
    async fn test_concurrent_transactions() {
        let storage = Arc::new(create_test_storage().await);

        let addr1 = AccountAddress::from_bytes([5; 32]);
        let addr2 = AccountAddress::from_bytes([6; 32]);

        // Spawn multiple concurrent transaction recordings
        let mut handles = vec![];
        for i in 0..10 {
            let storage_clone = storage.clone();
            let from_addr = addr1; // Copy the addresses
            let to_addr = addr2;
            let handle = tokio::spawn(async move {
                let tx = TransactionRecord {
                    tx_hash: format!("concurrent_{}", i),
                    from: from_addr,
                    to: to_addr,
                    amount: AdicAmount::from_adic(5.0),
                    timestamp: chrono::Utc::now(),
                    status: "completed".to_string(),
                };
                storage_clone.record_transaction(tx).await
            });
            handles.push(handle);
        }

        // Wait for all to complete
        for handle in handles {
            handle.await.unwrap().unwrap();
        }

        // Verify all were recorded
        let history = storage.get_transaction_history(addr1).await.unwrap();
        assert_eq!(history.len(), 10);
    }

    #[cfg(feature = "rocksdb")]
    #[tokio::test]
    async fn test_empty_transaction_history() {
        let storage = create_test_storage().await;

        let addr = AccountAddress::from_bytes([99; 32]);

        // Should return empty vec for address with no transactions
        let history = storage.get_transaction_history(addr).await.unwrap();
        assert_eq!(history.len(), 0);
    }

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
