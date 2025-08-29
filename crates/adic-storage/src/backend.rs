use adic_types::{AdicMessage, MessageId, PublicKey};
use async_trait::async_trait;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Message not found: {0}")]
    NotFound(String),

    #[error("Message already exists: {0}")]
    AlreadyExists(String),

    #[error("Storage backend error: {0}")]
    BackendError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// Trait for storage backend implementations
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// Store a message
    async fn put_message(&self, message: &AdicMessage) -> Result<()>;

    /// Retrieve a message by ID
    async fn get_message(&self, id: &MessageId) -> Result<Option<AdicMessage>>;

    /// Check if a message exists
    async fn has_message(&self, id: &MessageId) -> Result<bool>;

    /// Delete a message
    async fn delete_message(&self, id: &MessageId) -> Result<()>;

    /// Get all message IDs
    async fn list_messages(&self) -> Result<Vec<MessageId>>;

    /// Store parent-child relationship
    async fn add_parent_child(&self, parent: &MessageId, child: &MessageId) -> Result<()>;

    /// Get children of a message
    async fn get_children(&self, parent: &MessageId) -> Result<Vec<MessageId>>;

    /// Get parents of a message
    async fn get_parents(&self, child: &MessageId) -> Result<Vec<MessageId>>;

    /// Add a message to tips
    async fn add_tip(&self, id: &MessageId) -> Result<()>;

    /// Remove a message from tips
    async fn remove_tip(&self, id: &MessageId) -> Result<()>;

    /// Get current tips
    async fn get_tips(&self) -> Result<Vec<MessageId>>;

    /// Store message metadata
    async fn put_metadata(&self, id: &MessageId, key: &str, value: &[u8]) -> Result<()>;

    /// Get message metadata
    async fn get_metadata(&self, id: &MessageId, key: &str) -> Result<Option<Vec<u8>>>;

    /// Store reputation score
    async fn put_reputation(&self, pubkey: &PublicKey, score: f64) -> Result<()>;

    /// Get reputation score
    async fn get_reputation(&self, pubkey: &PublicKey) -> Result<Option<f64>>;

    /// Store finality status
    async fn mark_finalized(&self, id: &MessageId, artifact: &[u8]) -> Result<()>;

    /// Check if message is finalized
    async fn is_finalized(&self, id: &MessageId) -> Result<bool>;

    /// Get finality artifact
    async fn get_finality_artifact(&self, id: &MessageId) -> Result<Option<Vec<u8>>>;

    /// Store conflict information
    async fn add_to_conflict(&self, conflict_id: &str, message_id: &MessageId) -> Result<()>;

    /// Get messages in a conflict set
    async fn get_conflict_set(&self, conflict_id: &str) -> Result<Vec<MessageId>>;

    /// Store ball index entry
    async fn add_to_ball_index(
        &self,
        axis: u32,
        ball_id: &[u8],
        message_id: &MessageId,
    ) -> Result<()>;

    /// Get messages in a ball
    async fn get_ball_members(&self, axis: u32, ball_id: &[u8]) -> Result<Vec<MessageId>>;

    /// Begin a transaction (if supported)
    async fn begin_transaction(&self) -> Result<()>;

    /// Commit a transaction (if supported)
    async fn commit_transaction(&self) -> Result<()>;

    /// Rollback a transaction (if supported)
    async fn rollback_transaction(&self) -> Result<()>;

    /// Flush any pending writes
    async fn flush(&self) -> Result<()>;

    /// Get storage statistics
    async fn get_stats(&self) -> Result<StorageStats>;
}

#[derive(Debug, Clone, Default)]
pub struct StorageStats {
    pub message_count: usize,
    pub tip_count: usize,
    pub finalized_count: usize,
    pub reputation_entries: usize,
    pub conflict_sets: usize,
    pub total_size_bytes: Option<u64>,
}
