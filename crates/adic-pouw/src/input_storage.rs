//! Input Data Storage for Re-execution Validation
//!
//! Provides content-addressed storage and retrieval of task input data needed for
//! deterministic re-execution. Input data is stored by its content hash and verified
//! on retrieval to ensure integrity.

use crate::Hash;
use adic_storage::{StorageBackend, StorageError};
use adic_types::MessageId;
use std::sync::Arc;
use tracing::{debug, warn};

pub type Result<T> = std::result::Result<T, StorageError>;

/// Trait for content-addressed input data storage
pub trait InputStore: Send + Sync {
    /// Store input data by its content hash
    fn store_input<'a, 'async_trait>(
        &'a self,
        input_hash: &'a Hash,
        data: &'a [u8],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'async_trait>>
    where
        'a: 'async_trait,
        Self: 'async_trait;

    /// Retrieve input data by hash, verifying integrity
    fn get_input<'a, 'async_trait>(
        &'a self,
        input_hash: &'a Hash,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Vec<u8>>>> + Send + 'async_trait>>
    where
        'a: 'async_trait,
        Self: 'async_trait;

    /// Check if input data exists
    fn has_input<'a, 'async_trait>(
        &'a self,
        input_hash: &'a Hash,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send + 'async_trait>>
    where
        'a: 'async_trait,
        Self: 'async_trait;
}

/// InputStore implementation using StorageBackend with content verification
///
/// Stores input data using content-addressed keys and verifies hash on retrieval
/// to detect any storage corruption.
pub struct ContentAddressedInputStore {
    backend: Arc<dyn StorageBackend>,
}

impl ContentAddressedInputStore {
    pub fn new(backend: Arc<dyn StorageBackend>) -> Self {
        Self { backend }
    }

    /// Convert input hash to MessageId for storage key
    fn hash_to_message_id(hash: &Hash) -> MessageId {
        MessageId::from_bytes(*hash)
    }

    /// Verify data matches expected hash
    fn verify_hash(data: &[u8], expected_hash: &Hash) -> bool {
        let actual_hash = blake3::hash(data);
        actual_hash.as_bytes() == expected_hash
    }
}

impl InputStore for ContentAddressedInputStore {
    fn store_input<'a, 'async_trait>(
        &'a self,
        input_hash: &'a Hash,
        data: &'a [u8],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'async_trait>>
    where
        'a: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move { self.store_input_impl(input_hash, data).await })
    }

    fn get_input<'a, 'async_trait>(
        &'a self,
        input_hash: &'a Hash,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Vec<u8>>>> + Send + 'async_trait>>
    where
        'a: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move { self.get_input_impl(input_hash).await })
    }

    fn has_input<'a, 'async_trait>(
        &'a self,
        input_hash: &'a Hash,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send + 'async_trait>>
    where
        'a: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move { self.has_input_impl(input_hash).await })
    }
}

impl ContentAddressedInputStore {
    async fn store_input_impl(&self, input_hash: &Hash, data: &[u8]) -> Result<()> {
        // Verify the provided hash matches the data
        if !Self::verify_hash(data, input_hash) {
            return Err(StorageError::SerializationError(
                "Input hash does not match data content".to_string(),
            ));
        }

        let message_id = Self::hash_to_message_id(input_hash);

        debug!(
            hash = hex::encode(input_hash),
            size = data.len(),
            "📦 Storing input data"
        );

        self.backend
            .put_metadata(&message_id, "input", data)
            .await?;

        Ok(())
    }

    async fn get_input_impl(&self, input_hash: &Hash) -> Result<Option<Vec<u8>>> {
        let message_id = Self::hash_to_message_id(input_hash);

        let data = match self.backend.get_metadata(&message_id, "input").await? {
            Some(data) => data,
            None => {
                debug!(hash = hex::encode(input_hash), "Input data not found");
                return Ok(None);
            }
        };

        // Verify integrity on retrieval
        if !Self::verify_hash(&data, input_hash) {
            warn!(
                hash = hex::encode(input_hash),
                "⚠️ Input data hash mismatch - possible corruption"
            );
            return Err(StorageError::BackendError(
                "Input data integrity check failed".to_string(),
            ));
        }

        debug!(
            hash = hex::encode(input_hash),
            size = data.len(),
            "✅ Retrieved input data"
        );

        Ok(Some(data))
    }

    async fn has_input_impl(&self, input_hash: &Hash) -> Result<bool> {
        let message_id = Self::hash_to_message_id(input_hash);
        let exists = self.backend.get_metadata(&message_id, "input").await?.is_some();
        Ok(exists)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_storage::MemoryBackend;

    #[tokio::test]
    async fn test_store_and_retrieve_input() {
        let backend = Arc::new(MemoryBackend::new());
        let store = ContentAddressedInputStore::new(backend);

        let input_data = b"test input data for task execution";
        let input_hash = blake3::hash(input_data);
        let hash_bytes = *input_hash.as_bytes();

        // Store input
        store.store_input(&hash_bytes, input_data).await.unwrap();

        // Retrieve input
        let retrieved = store.get_input(&hash_bytes).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap(), input_data);
    }

    #[tokio::test]
    async fn test_get_nonexistent_input() {
        let backend = Arc::new(MemoryBackend::new());
        let store = ContentAddressedInputStore::new(backend);

        let fake_hash = [99u8; 32];
        let result = store.get_input(&fake_hash).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_has_input() {
        let backend = Arc::new(MemoryBackend::new());
        let store = ContentAddressedInputStore::new(backend);

        let input_data = b"another test input";
        let input_hash = *blake3::hash(input_data).as_bytes();

        // Initially doesn't exist
        assert!(!store.has_input(&input_hash).await.unwrap());

        // Store input
        store.store_input(&input_hash, input_data).await.unwrap();

        // Now exists
        assert!(store.has_input(&input_hash).await.unwrap());
    }

    #[tokio::test]
    async fn test_store_wrong_hash() {
        let backend = Arc::new(MemoryBackend::new());
        let store = ContentAddressedInputStore::new(backend);

        let input_data = b"test data";
        let wrong_hash = [1u8; 32]; // Not the actual hash

        // Should fail with hash mismatch
        let result = store.store_input(&wrong_hash, input_data).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("hash does not match"));
    }

    #[tokio::test]
    async fn test_integrity_check() {
        let backend = Arc::new(MemoryBackend::new());

        // Create store and store data
        let input_data = b"original data";
        let input_hash = *blake3::hash(input_data).as_bytes();
        let message_id = MessageId::from_bytes(input_hash);

        // Manually store corrupted data with correct key
        backend
            .put_metadata(&message_id, "input", b"corrupted data")
            .await
            .unwrap();

        // Create store and try to retrieve
        let store = ContentAddressedInputStore::new(backend);

        // Should fail integrity check
        let result = store.get_input(&input_hash).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("integrity check failed"));
    }

    #[tokio::test]
    async fn test_multiple_inputs() {
        let backend = Arc::new(MemoryBackend::new());
        let store = ContentAddressedInputStore::new(backend);

        // Store multiple inputs
        let inputs = vec![
            b"input 1".as_slice(),
            b"input 2 with more data".as_slice(),
            b"input 3 completely different".as_slice(),
        ];

        let mut hashes = Vec::new();
        for input in &inputs {
            let hash = *blake3::hash(input).as_bytes();
            store.store_input(&hash, input).await.unwrap();
            hashes.push(hash);
        }

        // Retrieve all inputs
        for (hash, expected_input) in hashes.iter().zip(&inputs) {
            let retrieved = store.get_input(hash).await.unwrap().unwrap();
            assert_eq!(&retrieved[..], *expected_input);
        }
    }
}
