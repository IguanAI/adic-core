use crate::backend::{Result, StorageBackend, StorageError, StorageStats};
use adic_types::{AdicMessage, MessageId, PublicKey};
use std::sync::Arc;

/// Configuration for storage engine
#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub backend_type: BackendType,
    pub cache_size: usize,
    pub flush_interval_ms: u64,
    pub max_batch_size: usize,
}

impl Default for StorageConfig {
    fn default() -> Self {
        // Default to RocksDB for persistence if available
        #[cfg(feature = "rocksdb")]
        let backend_type = BackendType::RocksDB {
            path: std::env::var("ADIC_DATA_DIR").unwrap_or_else(|_| "./data/storage".to_string()),
        };

        #[cfg(not(feature = "rocksdb"))]
        let backend_type = BackendType::Memory;

        Self {
            backend_type,
            cache_size: 10000,
            flush_interval_ms: 5000,
            max_batch_size: 100,
        }
    }
}

#[derive(Debug, Clone)]
pub enum BackendType {
    Memory,
    #[cfg(feature = "rocksdb")]
    RocksDB {
        path: String,
    },
}

/// High-level storage engine that wraps backend implementations
pub struct StorageEngine {
    backend: Arc<dyn StorageBackend>,
    _config: StorageConfig,
}

impl StorageEngine {
    /// Create a new storage engine with the given configuration
    pub fn new(config: StorageConfig) -> Result<Self> {
        let backend: Arc<dyn StorageBackend> = match &config.backend_type {
            BackendType::Memory => Arc::new(crate::memory::MemoryBackend::new()),
            #[cfg(feature = "rocksdb")]
            BackendType::RocksDB { path } => Arc::new(crate::rocks::RocksBackend::new(path)?),
        };

        Ok(Self {
            backend,
            _config: config,
        })
    }

    /// Store a message and update related indices
    pub async fn store_message(&self, message: &AdicMessage) -> Result<()> {
        // Begin transaction
        self.backend.begin_transaction().await?;

        // Store the message
        if let Err(e) = self.backend.put_message(message).await {
            self.backend.rollback_transaction().await?;
            return Err(e);
        }

        // Update tips (remove parents from tips, add this message as tip)
        for parent_id in &message.parents {
            if let Err(e) = self.backend.remove_tip(parent_id).await {
                // Ignore if parent wasn't a tip
                if !matches!(e, StorageError::NotFound(_)) {
                    self.backend.rollback_transaction().await?;
                    return Err(e);
                }
            }
        }

        if let Err(e) = self.backend.add_tip(&message.id).await {
            self.backend.rollback_transaction().await?;
            return Err(e);
        }

        // Commit transaction
        self.backend.commit_transaction().await?;

        Ok(())
    }

    /// Retrieve a message by ID
    pub async fn get_message(&self, id: &MessageId) -> Result<Option<AdicMessage>> {
        self.backend.get_message(id).await
    }

    /// Get all current tips
    pub async fn get_tips(&self) -> Result<Vec<MessageId>> {
        self.backend.get_tips().await
    }

    /// Get messages that reference a given message
    pub async fn get_children(&self, parent: &MessageId) -> Result<Vec<MessageId>> {
        self.backend.get_children(parent).await
    }

    /// Get messages referenced by a given message
    pub async fn get_parents(&self, child: &MessageId) -> Result<Vec<MessageId>> {
        self.backend.get_parents(child).await
    }

    /// Store reputation score for a public key
    pub async fn update_reputation(&self, pubkey: &PublicKey, score: f64) -> Result<()> {
        self.backend.put_reputation(pubkey, score).await
    }

    /// Get reputation score for a public key
    pub async fn get_reputation(&self, pubkey: &PublicKey) -> Result<Option<f64>> {
        self.backend.get_reputation(pubkey).await
    }

    /// Mark a message as finalized with an artifact
    pub async fn finalize_message(&self, id: &MessageId, artifact: &[u8]) -> Result<()> {
        self.backend.begin_transaction().await?;

        if let Err(e) = self.backend.mark_finalized(id, artifact).await {
            self.backend.rollback_transaction().await?;
            return Err(e);
        }

        // Remove from tips if it was a tip
        let _ = self.backend.remove_tip(id).await;

        self.backend.commit_transaction().await?;
        Ok(())
    }

    /// Check if a message is finalized
    pub async fn is_finalized(&self, id: &MessageId) -> Result<bool> {
        self.backend.is_finalized(id).await
    }

    /// Get finality artifact for a message
    pub async fn get_finality_artifact(&self, id: &MessageId) -> Result<Option<Vec<u8>>> {
        self.backend.get_finality_artifact(id).await
    }

    /// Add a message to a conflict set
    pub async fn add_to_conflict(&self, conflict_id: &str, message_id: &MessageId) -> Result<()> {
        self.backend.add_to_conflict(conflict_id, message_id).await
    }

    /// Get all messages in a conflict set
    pub async fn get_conflict_set(&self, conflict_id: &str) -> Result<Vec<MessageId>> {
        self.backend.get_conflict_set(conflict_id).await
    }

    /// Index a message by its ball membership
    pub async fn index_by_ball(
        &self,
        axis: u32,
        ball_id: &[u8],
        message_id: &MessageId,
    ) -> Result<()> {
        self.backend
            .add_to_ball_index(axis, ball_id, message_id)
            .await
    }

    /// Get all messages in a specific ball
    pub async fn get_ball_members(&self, axis: u32, ball_id: &[u8]) -> Result<Vec<MessageId>> {
        self.backend.get_ball_members(axis, ball_id).await
    }

    /// Store arbitrary metadata for a message
    pub async fn put_metadata(&self, id: &MessageId, key: &str, value: &[u8]) -> Result<()> {
        self.backend.put_metadata(id, key, value).await
    }

    /// Get metadata for a message
    pub async fn get_metadata(&self, id: &MessageId, key: &str) -> Result<Option<Vec<u8>>> {
        self.backend.get_metadata(id, key).await
    }

    /// Flush any pending writes
    pub async fn flush(&self) -> Result<()> {
        self.backend.flush().await
    }

    /// Get storage statistics
    pub async fn get_stats(&self) -> Result<StorageStats> {
        self.backend.get_stats().await
    }

    /// Delete a message (used for pruning)
    pub async fn delete_message(&self, id: &MessageId) -> Result<()> {
        self.backend.begin_transaction().await?;

        if let Err(e) = self.backend.delete_message(id).await {
            self.backend.rollback_transaction().await?;
            return Err(e);
        }

        // Clean up related indices
        let _ = self.backend.remove_tip(id).await;

        self.backend.commit_transaction().await?;
        Ok(())
    }

    /// Get all message IDs in storage
    pub async fn list_all_messages(&self) -> Result<Vec<MessageId>> {
        self.backend.list_messages().await
    }

    /// Get the underlying backend (for advanced operations)
    pub fn backend(&self) -> &Arc<dyn StorageBackend> {
        &self.backend
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::{AdicFeatures, AdicMeta, AxisPhi, QpDigits};
    use chrono::Utc;

    #[tokio::test]
    async fn test_storage_engine() {
        let config = StorageConfig::default();
        let engine = StorageEngine::new(config).unwrap();

        // Create test message
        let message = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(10, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![1, 2, 3],
        );

        // Store message
        engine.store_message(&message).await.unwrap();

        // Verify it's stored
        let retrieved = engine.get_message(&message.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, message.id);

        // Verify it's a tip
        let tips = engine.get_tips().await.unwrap();
        assert!(tips.contains(&message.id));

        // Test metadata
        engine
            .put_metadata(&message.id, "test_key", b"test_value")
            .await
            .unwrap();
        let metadata = engine.get_metadata(&message.id, "test_key").await.unwrap();
        assert_eq!(metadata, Some(b"test_value".to_vec()));

        // Test reputation
        let pubkey = PublicKey::from_bytes([1; 32]);
        engine.update_reputation(&pubkey, 0.5).await.unwrap();
        let rep = engine.get_reputation(&pubkey).await.unwrap();
        assert_eq!(rep, Some(0.5));

        // Test finalization
        engine
            .finalize_message(&message.id, b"artifact")
            .await
            .unwrap();
        assert!(engine.is_finalized(&message.id).await.unwrap());

        // Should no longer be a tip after finalization
        let tips = engine.get_tips().await.unwrap();
        assert!(!tips.contains(&message.id));
    }
}
