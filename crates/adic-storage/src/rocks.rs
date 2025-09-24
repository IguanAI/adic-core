use async_trait::async_trait;
use rocksdb::{IteratorMode, Options, WriteBatch, DB};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, info, warn};

use crate::backend::StorageStats;
use crate::{StorageBackend, StorageError};
use adic_types::{AdicMessage, MessageId, PublicKey};

type Result<T> = std::result::Result<T, StorageError>;

pub struct RocksBackend {
    db: Arc<DB>,
}

impl RocksBackend {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);

        // Performance optimizations for range queries
        opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB
        opts.set_max_write_buffer_number(3);
        opts.set_target_file_size_base(64 * 1024 * 1024); // 64MB
        opts.set_max_background_jobs(4);
        opts.set_level_compaction_dynamic_level_bytes(true);

        // Optimize for time-series data and range scans
        opts.set_prefix_extractor(rocksdb::SliceTransform::create_fixed_prefix(10));
        opts.set_memtable_prefix_bloom_ratio(0.2);

        // Block cache for frequently accessed data
        let cache = rocksdb::Cache::new_lru_cache(256 * 1024 * 1024); // 256MB block cache
        let mut block_opts = rocksdb::BlockBasedOptions::default();
        block_opts.set_block_cache(&cache);
        block_opts.set_cache_index_and_filter_blocks(true);
        block_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);

        // Bloom filters for faster negative lookups
        block_opts.set_bloom_filter(10.0, false);

        opts.set_block_based_table_factory(&block_opts);

        // Enable statistics for monitoring
        opts.enable_statistics();
        opts.set_stats_dump_period_sec(600); // Dump stats every 10 minutes

        // Optimize compaction for time-series workload
        opts.set_compaction_style(rocksdb::DBCompactionStyle::Level);
        opts.optimize_level_style_compaction(512 * 1024 * 1024); // 512MB memtable

        let db = DB::open(&opts, path)
            .map_err(|e| StorageError::BackendError(format!("Failed to open RocksDB: {}", e)))?;

        Ok(Self { db: Arc::new(db) })
    }

    pub fn with_options<P: AsRef<Path>>(path: P, opts: Options) -> Result<Self> {
        let db = DB::open(&opts, path)
            .map_err(|e| StorageError::BackendError(format!("Failed to open RocksDB: {}", e)))?;

        Ok(Self { db: Arc::new(db) })
    }

    fn message_key(id: &MessageId) -> Vec<u8> {
        format!("msg:{}", id).into_bytes()
    }

    fn parent_child_key(parent: &MessageId, child: &MessageId) -> Vec<u8> {
        format!("pc:{}:{}", parent, child).into_bytes()
    }

    fn child_parent_key(child: &MessageId, parent: &MessageId) -> Vec<u8> {
        format!("cp:{}:{}", child, parent).into_bytes()
    }

    fn tip_key(id: &MessageId) -> Vec<u8> {
        format!("tip:{}", id).into_bytes()
    }

    fn metadata_key(id: &MessageId, key: &str) -> Vec<u8> {
        format!("meta:{}:{}", id, key).into_bytes()
    }

    fn reputation_key(pubkey: &PublicKey) -> Vec<u8> {
        format!("rep:{:?}", pubkey.as_bytes()).into_bytes()
    }

    fn finality_key(id: &MessageId) -> Vec<u8> {
        format!("fin:{}", id).into_bytes()
    }

    fn finality_artifact_key(id: &MessageId) -> Vec<u8> {
        format!("fin_art:{}", id).into_bytes()
    }

    fn conflict_key(conflict_id: &str, message_id: &MessageId) -> Vec<u8> {
        format!("conflict:{}:{}", conflict_id, message_id).into_bytes()
    }

    fn ball_index_key(axis: u32, ball_id: &[u8], message_id: &MessageId) -> Vec<u8> {
        let ball_id_hex = hex::encode(ball_id);
        format!("ball:{}:{}:{}", axis, ball_id_hex, message_id).into_bytes()
    }

    fn message_by_time_key(timestamp_millis: i64, message_id: &MessageId) -> Vec<u8> {
        // Use zero-padded timestamp for proper lexicographic ordering
        format!("msg_by_time:{:020}:{}", timestamp_millis, message_id).into_bytes()
    }

    // Helper function for transaction indexing - kept for future transaction support in storage module
    // Currently, transaction storage is handled by adic-economics module which uses the same format
    #[allow(dead_code)]
    fn tx_by_addr_time_key(address: &[u8], timestamp_millis: i64, tx_hash: &str) -> Vec<u8> {
        // Use zero-padded timestamp for proper ordering
        format!(
            "tx_by_addr:{}:{:020}:{}",
            hex::encode(address),
            timestamp_millis,
            tx_hash
        )
        .into_bytes()
    }
}

#[async_trait]
impl StorageBackend for RocksBackend {
    async fn put_message(&self, message: &AdicMessage) -> Result<()> {
        let key = Self::message_key(&message.id);
        let key_str = String::from_utf8_lossy(&key);
        let value = bincode::serialize(message)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        // Use WriteBatch for atomic updates
        let mut batch = WriteBatch::default();

        // Store the message itself
        batch.put(&key, &value);

        // Store timestamp index for efficient time-based queries
        let timestamp_millis = message.meta.timestamp.timestamp_millis();
        let time_key = Self::message_by_time_key(timestamp_millis, &message.id);
        batch.put(&time_key, b""); // Only need the key for indexing

        tracing::info!(
            "Storing message {} with key: {} (size: {} bytes, timestamp: {})",
            message.id,
            key_str,
            value.len(),
            timestamp_millis
        );

        let result = self
            .db
            .write(batch)
            .map_err(|e| StorageError::BackendError(format!("RocksDB put error: {}", e)));

        if result.is_ok() {
            tracing::info!(
                "Successfully stored message {} in RocksDB with timestamp index",
                message.id
            );
        } else {
            tracing::error!("Failed to store message {}: {:?}", message.id, result);
        }

        result
    }

    async fn get_message(&self, id: &MessageId) -> Result<Option<AdicMessage>> {
        let key = Self::message_key(id);
        let key_str = String::from_utf8_lossy(&key);

        tracing::debug!("Getting message {} with key: {}", id, key_str);

        match self.db.get(&key) {
            Ok(Some(data)) => {
                tracing::debug!(
                    "Found message {} in storage (size: {} bytes)",
                    id,
                    data.len()
                );
                let message = bincode::deserialize(&data)
                    .map_err(|e| StorageError::SerializationError(e.to_string()))?;
                Ok(Some(message))
            }
            Ok(None) => {
                tracing::debug!("Message {} not found in storage", id);
                Ok(None)
            }
            Err(e) => {
                tracing::error!("RocksDB get error for message {}: {}", id, e);
                Err(StorageError::BackendError(format!(
                    "RocksDB get error: {}",
                    e
                )))
            }
        }
    }

    async fn has_message(&self, id: &MessageId) -> Result<bool> {
        let key = Self::message_key(id);
        self.db
            .get(key)
            .map(|v| v.is_some())
            .map_err(|e| StorageError::BackendError(format!("RocksDB has error: {}", e)))
    }

    async fn delete_message(&self, id: &MessageId) -> Result<()> {
        let key = Self::message_key(id);
        self.db
            .delete(key)
            .map_err(|e| StorageError::BackendError(format!("RocksDB delete error: {}", e)))
    }

    async fn list_messages(&self) -> Result<Vec<MessageId>> {
        // Get both stored messages and tips
        let mut message_ids = Vec::new();

        // First get all messages with "msg:" prefix
        let msg_prefix = b"msg:";
        let msg_iter = self
            .db
            .iterator(IteratorMode::From(msg_prefix, rocksdb::Direction::Forward));

        for item in msg_iter {
            let (key, _) =
                item.map_err(|e| StorageError::BackendError(format!("Iterator error: {}", e)))?;

            if !key.starts_with(msg_prefix) {
                break;
            }

            // Extract message ID from key
            if let Ok(key_str) = std::str::from_utf8(&key) {
                if let Some(id_str) = key_str.strip_prefix("msg:") {
                    // Parse MessageId from hex string representation
                    if let Ok(id) = MessageId::from_hex(id_str) {
                        message_ids.push(id);
                    }
                }
            }
        }

        // Also get all tips since they represent current messages
        let tips = self.get_tips().await?;
        for tip in tips {
            if !message_ids.contains(&tip) {
                message_ids.push(tip);
            }
        }

        tracing::debug!(
            "list_messages: found {} total messages (including tips)",
            message_ids.len()
        );

        Ok(message_ids)
    }

    async fn add_parent_child(&self, parent: &MessageId, child: &MessageId) -> Result<()> {
        let mut batch = WriteBatch::default();

        // Add parent->child mapping
        let pc_key = Self::parent_child_key(parent, child);
        batch.put(pc_key, b"1");

        // Add child->parent mapping
        let cp_key = Self::child_parent_key(child, parent);
        batch.put(cp_key, b"1");

        self.db
            .write(batch)
            .map_err(|e| StorageError::BackendError(format!("RocksDB batch write error: {}", e)))
    }

    async fn get_children(&self, parent: &MessageId) -> Result<Vec<MessageId>> {
        let prefix = format!("pc:{}:", parent).into_bytes();
        let iter = self
            .db
            .iterator(IteratorMode::From(&prefix, rocksdb::Direction::Forward));
        let mut children = Vec::new();

        for item in iter {
            let (key, _) =
                item.map_err(|e| StorageError::BackendError(format!("Iterator error: {}", e)))?;

            if !key.starts_with(&prefix) {
                break;
            }

            // Extract child ID from key
            if let Ok(key_str) = std::str::from_utf8(&key) {
                if let Some(suffix) = key_str.strip_prefix(&format!("pc:{}:", parent)) {
                    if let Ok(child) = MessageId::from_hex(suffix) {
                        children.push(child);
                    }
                }
            }
        }

        Ok(children)
    }

    async fn get_parents(&self, child: &MessageId) -> Result<Vec<MessageId>> {
        let prefix = format!("cp:{}:", child).into_bytes();
        let iter = self
            .db
            .iterator(IteratorMode::From(&prefix, rocksdb::Direction::Forward));
        let mut parents = Vec::new();

        for item in iter {
            let (key, _) =
                item.map_err(|e| StorageError::BackendError(format!("Iterator error: {}", e)))?;

            if !key.starts_with(&prefix) {
                break;
            }

            // Extract parent ID from key
            if let Ok(key_str) = std::str::from_utf8(&key) {
                if let Some(suffix) = key_str.strip_prefix(&format!("cp:{}:", child)) {
                    if let Ok(parent) = MessageId::from_hex(suffix) {
                        parents.push(parent);
                    }
                }
            }
        }

        Ok(parents)
    }

    async fn add_tip(&self, id: &MessageId) -> Result<()> {
        let key = Self::tip_key(id);
        self.db
            .put(key, b"1")
            .map_err(|e| StorageError::BackendError(format!("RocksDB put tip error: {}", e)))
    }

    async fn remove_tip(&self, id: &MessageId) -> Result<()> {
        let key = Self::tip_key(id);
        self.db
            .delete(key)
            .map_err(|e| StorageError::BackendError(format!("RocksDB delete tip error: {}", e)))
    }

    async fn get_tips(&self) -> Result<Vec<MessageId>> {
        let prefix = b"tip:";
        let iter = self
            .db
            .iterator(IteratorMode::From(prefix, rocksdb::Direction::Forward));
        let mut tips = Vec::new();

        for item in iter {
            let (key, _) =
                item.map_err(|e| StorageError::BackendError(format!("Iterator error: {}", e)))?;

            if !key.starts_with(prefix) {
                break;
            }

            // Extract tip ID from key
            if let Ok(key_str) = std::str::from_utf8(&key) {
                if let Some(id_str) = key_str.strip_prefix("tip:") {
                    if let Ok(id) = MessageId::from_hex(id_str) {
                        tips.push(id);
                    }
                }
            }
        }

        Ok(tips)
    }

    async fn put_metadata(&self, id: &MessageId, key: &str, value: &[u8]) -> Result<()> {
        let storage_key = Self::metadata_key(id, key);
        self.db
            .put(storage_key, value)
            .map_err(|e| StorageError::BackendError(format!("RocksDB put metadata error: {}", e)))
    }

    async fn get_metadata(&self, id: &MessageId, key: &str) -> Result<Option<Vec<u8>>> {
        let storage_key = Self::metadata_key(id, key);
        self.db
            .get(storage_key)
            .map_err(|e| StorageError::BackendError(format!("RocksDB get metadata error: {}", e)))
    }

    async fn put_reputation(&self, pubkey: &PublicKey, score: f64) -> Result<()> {
        let key = Self::reputation_key(pubkey);
        let value = score.to_le_bytes();
        self.db
            .put(key, value)
            .map_err(|e| StorageError::BackendError(format!("RocksDB put reputation error: {}", e)))
    }

    async fn get_reputation(&self, pubkey: &PublicKey) -> Result<Option<f64>> {
        let key = Self::reputation_key(pubkey);
        match self.db.get(key) {
            Ok(Some(data)) => {
                if data.len() == 8 {
                    let bytes: [u8; 8] = data.as_slice().try_into().map_err(|_| {
                        StorageError::SerializationError("Invalid score data".to_string())
                    })?;
                    Ok(Some(f64::from_le_bytes(bytes)))
                } else {
                    Err(StorageError::SerializationError(
                        "Invalid score data length".to_string(),
                    ))
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(StorageError::BackendError(format!(
                "RocksDB get reputation error: {}",
                e
            ))),
        }
    }

    async fn mark_finalized(&self, id: &MessageId, artifact: &[u8]) -> Result<()> {
        let mut batch = WriteBatch::default();

        // Mark as finalized
        let fin_key = Self::finality_key(id);
        batch.put(fin_key, b"1");

        // Store artifact
        let art_key = Self::finality_artifact_key(id);
        batch.put(art_key, artifact);

        self.db
            .write(batch)
            .map_err(|e| StorageError::BackendError(format!("RocksDB mark finalized error: {}", e)))
    }

    async fn is_finalized(&self, id: &MessageId) -> Result<bool> {
        let key = Self::finality_key(id);
        self.db
            .get(key)
            .map(|v| v.is_some())
            .map_err(|e| StorageError::BackendError(format!("RocksDB is finalized error: {}", e)))
    }

    async fn get_finality_artifact(&self, id: &MessageId) -> Result<Option<Vec<u8>>> {
        let key = Self::finality_artifact_key(id);
        self.db
            .get(key)
            .map_err(|e| StorageError::BackendError(format!("RocksDB get artifact error: {}", e)))
    }

    async fn add_to_conflict(&self, conflict_id: &str, message_id: &MessageId) -> Result<()> {
        let key = Self::conflict_key(conflict_id, message_id);
        self.db
            .put(key, b"1")
            .map_err(|e| StorageError::BackendError(format!("RocksDB add conflict error: {}", e)))
    }

    async fn get_conflict_set(&self, conflict_id: &str) -> Result<Vec<MessageId>> {
        let prefix = format!("conflict:{}:", conflict_id).into_bytes();
        let iter = self
            .db
            .iterator(IteratorMode::From(&prefix, rocksdb::Direction::Forward));
        let mut messages = Vec::new();

        for item in iter {
            let (key, _) =
                item.map_err(|e| StorageError::BackendError(format!("Iterator error: {}", e)))?;

            if !key.starts_with(&prefix) {
                break;
            }

            // Extract message ID from key
            if let Ok(key_str) = std::str::from_utf8(&key) {
                if let Some(suffix) = key_str.strip_prefix(&format!("conflict:{}:", conflict_id)) {
                    if let Ok(id) = MessageId::from_hex(suffix) {
                        messages.push(id);
                    }
                }
            }
        }

        Ok(messages)
    }

    async fn add_to_ball_index(
        &self,
        axis: u32,
        ball_id: &[u8],
        message_id: &MessageId,
    ) -> Result<()> {
        let key = Self::ball_index_key(axis, ball_id, message_id);
        self.db
            .put(key, b"1")
            .map_err(|e| StorageError::BackendError(format!("RocksDB add ball index error: {}", e)))
    }

    async fn get_ball_members(&self, axis: u32, ball_id: &[u8]) -> Result<Vec<MessageId>> {
        let ball_id_hex = hex::encode(ball_id);
        let prefix = format!("ball:{}:{}:", axis, ball_id_hex).into_bytes();
        let iter = self
            .db
            .iterator(IteratorMode::From(&prefix, rocksdb::Direction::Forward));
        let mut messages = Vec::new();

        for item in iter {
            let (key, _) =
                item.map_err(|e| StorageError::BackendError(format!("Iterator error: {}", e)))?;

            if !key.starts_with(&prefix) {
                break;
            }

            // Extract message ID from key
            if let Ok(key_str) = std::str::from_utf8(&key) {
                if let Some(suffix) =
                    key_str.strip_prefix(&format!("ball:{}:{}:", axis, ball_id_hex))
                {
                    if let Ok(id) = MessageId::from_hex(suffix) {
                        messages.push(id);
                    }
                }
            }
        }

        Ok(messages)
    }

    async fn begin_transaction(&self) -> Result<()> {
        // RocksDB doesn't have explicit transactions in this context
        // WriteBatch is used for atomic operations
        Ok(())
    }

    async fn commit_transaction(&self) -> Result<()> {
        // RocksDB doesn't have explicit transactions in this context
        Ok(())
    }

    async fn rollback_transaction(&self) -> Result<()> {
        // RocksDB doesn't have explicit transactions in this context
        Ok(())
    }

    async fn flush(&self) -> Result<()> {
        self.db
            .flush()
            .map_err(|e| StorageError::BackendError(format!("RocksDB flush error: {}", e)))
    }

    async fn get_stats(&self) -> Result<StorageStats> {
        let message_count = self.list_messages().await?.len();
        let tip_count = self.get_tips().await?.len();

        // Count finalized messages
        let prefix = b"fin:";
        let iter = self
            .db
            .iterator(IteratorMode::From(prefix, rocksdb::Direction::Forward));
        let mut finalized_count = 0;

        for item in iter {
            let (key, _) =
                item.map_err(|e| StorageError::BackendError(format!("Iterator error: {}", e)))?;
            if !key.starts_with(prefix) {
                break;
            }
            finalized_count += 1;
        }

        // Count reputation entries
        let prefix = b"rep:";
        let iter = self
            .db
            .iterator(IteratorMode::From(prefix, rocksdb::Direction::Forward));
        let mut reputation_entries = 0;

        for item in iter {
            let (key, _) =
                item.map_err(|e| StorageError::BackendError(format!("Iterator error: {}", e)))?;
            if !key.starts_with(prefix) {
                break;
            }
            reputation_entries += 1;
        }

        // Count conflict sets (unique conflict IDs)
        let prefix = b"conflict:";
        let iter = self
            .db
            .iterator(IteratorMode::From(prefix, rocksdb::Direction::Forward));
        let mut conflict_ids = std::collections::HashSet::new();

        for item in iter {
            let (key, _) =
                item.map_err(|e| StorageError::BackendError(format!("Iterator error: {}", e)))?;
            if !key.starts_with(prefix) {
                break;
            }

            if let Ok(key_str) = std::str::from_utf8(&key) {
                if let Some(suffix) = key_str.strip_prefix("conflict:") {
                    if let Some(conflict_id) = suffix.split(':').next() {
                        conflict_ids.insert(conflict_id.to_string());
                    }
                }
            }
        }

        Ok(StorageStats {
            message_count,
            tip_count,
            finalized_count,
            reputation_entries,
            conflict_sets: conflict_ids.len(),
            total_size_bytes: None, // RocksDB doesn't provide easy size estimation
        })
    }

    async fn get_recently_finalized(&self, limit: usize) -> Result<Vec<MessageId>> {
        let prefix = b"fin:";
        let iter = self
            .db
            .iterator(IteratorMode::From(prefix, rocksdb::Direction::Forward));
        let mut finalized_ids = Vec::new();

        for item in iter {
            if finalized_ids.len() >= limit {
                break;
            }

            let (key, _) =
                item.map_err(|e| StorageError::BackendError(format!("Iterator error: {}", e)))?;
            if !key.starts_with(prefix) {
                break;
            }

            // Extract MessageId from key (format: "fin:<message_id_hex>")
            if let Ok(key_str) = std::str::from_utf8(&key) {
                if let Some(id_hex) = key_str.strip_prefix("fin:") {
                    if let Ok(msg_id) = MessageId::from_hex(id_hex) {
                        finalized_ids.push(msg_id);
                    }
                }
            }
        }

        Ok(finalized_ids)
    }

    async fn get_messages_in_time_range(
        &self,
        start_timestamp_millis: i64,
        end_timestamp_millis: Option<i64>,
        limit: usize,
    ) -> Result<Vec<MessageId>> {
        let start_time = Instant::now();
        let mut message_ids = Vec::new();

        info!(
            operation = "query_time_range",
            start_ts = start_timestamp_millis,
            end_ts = ?end_timestamp_millis,
            limit = limit,
            "🔍 Starting timestamp range query"
        );

        // Use zero-padded timestamp for proper ordering
        let start_key = format!("msg_by_time:{:020}", start_timestamp_millis).into_bytes();
        let end_prefix = if let Some(end_ts) = end_timestamp_millis {
            format!("msg_by_time:{:020}", end_ts)
        } else {
            // If no end timestamp, use a far future timestamp
            format!("msg_by_time:{:020}", i64::MAX)
        };

        let iter = self
            .db
            .iterator(IteratorMode::From(&start_key, rocksdb::Direction::Forward));

        for item in iter {
            if message_ids.len() >= limit {
                break;
            }

            let (key, _) = item.map_err(|e| {
                error!(
                    operation = "query_time_range",
                    error = %e,
                    "❌ Iterator error during timestamp range query"
                );
                StorageError::BackendError(format!("Iterator error: {}", e))
            })?;
            let key_str = String::from_utf8_lossy(&key);

            // Check if we've passed the end timestamp
            if key_str.as_ref() >= end_prefix.as_str() {
                break;
            }

            // Check if this is still a timestamp index key
            if !key_str.starts_with("msg_by_time:") {
                break;
            }

            // Extract MessageId from key (format: "msg_by_time:{timestamp}:{message_id}")
            if let Some(parts) = key_str.strip_prefix("msg_by_time:") {
                if let Some((_, id_str)) = parts.split_once(':') {
                    if let Ok(msg_id) = MessageId::from_hex(id_str) {
                        message_ids.push(msg_id);
                    }
                }
            }
        }

        let duration = start_time.elapsed();
        let duration_ms = duration.as_millis();

        if duration_ms > 100 {
            warn!(
                operation = "query_time_range",
                start_ts = start_timestamp_millis,
                end_ts = ?end_timestamp_millis,
                result_count = message_ids.len(),
                duration_ms = duration_ms as u64,
                "⚠️ Slow query detected: timestamp range query"
            );
        } else {
            info!(
                operation = "query_time_range",
                start_ts = start_timestamp_millis,
                end_ts = ?end_timestamp_millis,
                result_count = message_ids.len(),
                duration_ms = duration_ms as u64,
                "✅ Completed timestamp range query"
            );
        }

        Ok(message_ids)
    }

    async fn get_messages_after_timestamp(
        &self,
        timestamp_millis: i64,
        limit: usize,
    ) -> Result<Vec<MessageId>> {
        let start_time = Instant::now();

        info!(
            operation = "query_after_timestamp",
            after_ts = timestamp_millis,
            limit = limit,
            "🔍 Starting query for messages after timestamp"
        );

        // Use get_messages_in_time_range with no end timestamp
        let result = self
            .get_messages_in_time_range(timestamp_millis, None, limit)
            .await;

        match &result {
            Ok(messages) => {
                let duration_ms = start_time.elapsed().as_millis() as u64;
                info!(
                    operation = "query_after_timestamp",
                    after_ts = timestamp_millis,
                    result_count = messages.len(),
                    duration_ms = duration_ms,
                    "✅ Completed query for messages after timestamp"
                );
            }
            Err(e) => {
                error!(
                    operation = "query_after_timestamp",
                    after_ts = timestamp_millis,
                    error = %e,
                    "❌ Failed to query messages after timestamp"
                );
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::{AdicFeatures, AdicMeta};
    use chrono::Utc;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_rocks_backend_message_operations() {
        let temp_dir = TempDir::new().unwrap();
        let backend = RocksBackend::new(temp_dir.path()).unwrap();

        // Create a test message
        let message = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![1, 2, 3],
        );

        // Test put and get
        backend.put_message(&message).await.unwrap();
        let retrieved = backend.get_message(&message.id).await.unwrap();
        assert!(retrieved.is_some());

        // Test has_message
        assert!(backend.has_message(&message.id).await.unwrap());

        // Test list_messages
        let messages = backend.list_messages().await.unwrap();
        assert!(messages.contains(&message.id));

        // Test delete
        backend.delete_message(&message.id).await.unwrap();
        assert!(!backend.has_message(&message.id).await.unwrap());
    }

    #[tokio::test]
    async fn test_rocks_backend_relationships() {
        let temp_dir = TempDir::new().unwrap();
        let backend = RocksBackend::new(temp_dir.path()).unwrap();

        let parent = MessageId::new(b"parent");
        let child1 = MessageId::new(b"child1");
        let child2 = MessageId::new(b"child2");

        // Add relationships
        backend.add_parent_child(&parent, &child1).await.unwrap();
        backend.add_parent_child(&parent, &child2).await.unwrap();

        // Test get_children
        let children = backend.get_children(&parent).await.unwrap();
        assert_eq!(children.len(), 2);

        // Test get_parents
        let parents = backend.get_parents(&child1).await.unwrap();
        assert_eq!(parents.len(), 1);
        assert!(parents.contains(&parent));
    }

    #[tokio::test]
    async fn test_rocks_backend_tips() {
        let temp_dir = TempDir::new().unwrap();
        let backend = RocksBackend::new(temp_dir.path()).unwrap();

        let tip1 = MessageId::new(b"tip1");
        let tip2 = MessageId::new(b"tip2");

        // Add tips
        backend.add_tip(&tip1).await.unwrap();
        backend.add_tip(&tip2).await.unwrap();

        // Get tips
        let tips = backend.get_tips().await.unwrap();
        assert_eq!(tips.len(), 2);

        // Remove a tip
        backend.remove_tip(&tip1).await.unwrap();
        let tips = backend.get_tips().await.unwrap();
        assert_eq!(tips.len(), 1);
        assert!(tips.contains(&tip2));
    }
}
