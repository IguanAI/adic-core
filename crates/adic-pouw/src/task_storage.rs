//! Task Storage for Re-execution Validation
//!
//! Provides storage and retrieval of Task definitions needed for deterministic re-execution.
//! Tasks are stored by their TaskId and can be retrieved during validation to re-execute work.

use crate::types::{Task, TaskId};
use adic_storage::{StorageBackend, StorageError};
use adic_types::MessageId;
use std::sync::Arc;
use tracing::debug;

pub type Result<T> = std::result::Result<T, StorageError>;

/// Trait for task storage operations
///
/// Note: Methods use `async_trait` for dyn compatibility
pub trait TaskStore: Send + Sync {
    /// Store a task for future re-execution
    fn store_task<'a, 'async_trait>(
        &'a self,
        task: &'a Task,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'async_trait>>
    where
        'a: 'async_trait,
        Self: 'async_trait;

    /// Retrieve a task by ID
    fn get_task<'a, 'async_trait>(
        &'a self,
        task_id: &'a TaskId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Task>>> + Send + 'async_trait>>
    where
        'a: 'async_trait,
        Self: 'async_trait;

    /// Check if a task exists in storage
    fn has_task<'a, 'async_trait>(
        &'a self,
        task_id: &'a TaskId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send + 'async_trait>>
    where
        'a: 'async_trait,
        Self: 'async_trait;
}

/// TaskStore implementation using StorageBackend
///
/// Stores tasks using the metadata API: `put_metadata(message_id, "task", serialized_task)`
pub struct StorageBackendTaskStore {
    backend: Arc<dyn StorageBackend>,
}

impl StorageBackendTaskStore {
    pub fn new(backend: Arc<dyn StorageBackend>) -> Self {
        Self { backend }
    }

    /// Convert TaskId to MessageId for storage key
    fn task_id_to_message_id(task_id: &TaskId) -> MessageId {
        MessageId::from_bytes(*task_id)
    }
}

impl TaskStore for StorageBackendTaskStore {
    fn store_task<'a, 'async_trait>(
        &'a self,
        task: &'a Task,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'async_trait>>
    where
        'a: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move { self.store_task_impl(task).await })
    }

    fn get_task<'a, 'async_trait>(
        &'a self,
        task_id: &'a TaskId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Task>>> + Send + 'async_trait>>
    where
        'a: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move { self.get_task_impl(task_id).await })
    }

    fn has_task<'a, 'async_trait>(
        &'a self,
        task_id: &'a TaskId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send + 'async_trait>>
    where
        'a: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move { self.has_task_impl(task_id).await })
    }
}

impl StorageBackendTaskStore {
    async fn store_task_impl(&self, task: &Task) -> Result<()> {
        let message_id = Self::task_id_to_message_id(&task.task_id);

        // Serialize task using bincode for efficiency
        let serialized = bincode::serialize(task).map_err(|e| {
            StorageError::SerializationError(format!("Failed to serialize task: {}", e))
        })?;

        debug!(
            task_id = hex::encode(&task.task_id),
            size = serialized.len(),
            "📦 Storing task for re-execution"
        );

        self.backend
            .put_metadata(&message_id, "task", &serialized)
            .await?;

        Ok(())
    }

    async fn get_task_impl(&self, task_id: &TaskId) -> Result<Option<Task>> {
        let message_id = Self::task_id_to_message_id(task_id);

        let data = match self.backend.get_metadata(&message_id, "task").await? {
            Some(data) => data,
            None => {
                debug!(task_id = hex::encode(task_id), "Task not found in storage");
                return Ok(None);
            }
        };

        // Deserialize task
        let task: Task = bincode::deserialize(&data).map_err(|e| {
            StorageError::SerializationError(format!("Failed to deserialize task: {}", e))
        })?;

        debug!(
            task_id = hex::encode(task_id),
            "✅ Retrieved task from storage"
        );

        Ok(Some(task))
    }

    async fn has_task_impl(&self, task_id: &TaskId) -> Result<bool> {
        let message_id = Self::task_id_to_message_id(task_id);
        let exists = self.backend.get_metadata(&message_id, "task").await?.is_some();
        Ok(exists)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ComputationType, FinalityStatus, ResourceRequirements, TaskStatus, TaskType};
    use adic_economics::types::AdicAmount;
    use adic_storage::MemoryBackend;
    use adic_types::PublicKey;
    use chrono::Utc;

    fn create_test_task(task_id: TaskId) -> Task {
        Task {
            task_id,
            sponsor: PublicKey::from_bytes([1u8; 32]),
            task_type: TaskType::Compute {
                computation_type: ComputationType::HashVerification,
                resource_requirements: ResourceRequirements::default(),
            },
            input_cid: "QmTestInput123".to_string(),
            expected_output_schema: None,
            reward: AdicAmount::from_adic(10.0),
            collateral_requirement: AdicAmount::from_adic(5.0),
            deadline_epoch: 100,
            min_reputation: 100.0,
            worker_count: 3,
            created_at: Utc::now(),
            status: TaskStatus::Submitted,
            finality_status: FinalityStatus::Pending,
        }
    }

    #[tokio::test]
    async fn test_store_and_retrieve_task() {
        let backend = Arc::new(MemoryBackend::new());
        let store = StorageBackendTaskStore::new(backend);

        let task_id = [1u8; 32];
        let task = create_test_task(task_id);

        // Store task
        store.store_task(&task).await.unwrap();

        // Retrieve task
        let retrieved = store.get_task(&task_id).await.unwrap();
        assert!(retrieved.is_some());

        let retrieved_task = retrieved.unwrap();
        assert_eq!(retrieved_task.task_id, task.task_id);
        assert_eq!(retrieved_task.sponsor, task.sponsor);
        assert_eq!(retrieved_task.input_cid, task.input_cid);
        assert_eq!(retrieved_task.reward, task.reward);
    }

    #[tokio::test]
    async fn test_get_nonexistent_task() {
        let backend = Arc::new(MemoryBackend::new());
        let store = StorageBackendTaskStore::new(backend);

        let task_id = [99u8; 32];
        let result = store.get_task(&task_id).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_has_task() {
        let backend = Arc::new(MemoryBackend::new());
        let store = StorageBackendTaskStore::new(backend);

        let task_id = [2u8; 32];
        let task = create_test_task(task_id);

        // Initially doesn't exist
        assert!(!store.has_task(&task_id).await.unwrap());

        // Store task
        store.store_task(&task).await.unwrap();

        // Now exists
        assert!(store.has_task(&task_id).await.unwrap());
    }

    #[tokio::test]
    async fn test_overwrite_task() {
        let backend = Arc::new(MemoryBackend::new());
        let store = StorageBackendTaskStore::new(backend);

        let task_id = [3u8; 32];
        let mut task1 = create_test_task(task_id);
        task1.reward = AdicAmount::from_adic(10.0);

        let mut task2 = create_test_task(task_id);
        task2.reward = AdicAmount::from_adic(20.0);

        // Store first version
        store.store_task(&task1).await.unwrap();

        // Store second version (overwrite)
        store.store_task(&task2).await.unwrap();

        // Retrieve should get second version
        let retrieved = store.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(retrieved.reward, AdicAmount::from_adic(20.0));
    }
}
