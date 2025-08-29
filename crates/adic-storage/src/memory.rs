use crate::backend::{Result, StorageBackend, StorageError, StorageStats};
use adic_types::{AdicMessage, MessageId, PublicKey};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

// Type aliases for complex types
type MetadataMap = Arc<RwLock<HashMap<(MessageId, String), Vec<u8>>>>;
type BallIndexMap = Arc<RwLock<HashMap<(u32, Vec<u8>), HashSet<MessageId>>>>;

/// In-memory storage backend for testing and development
pub struct MemoryBackend {
    messages: Arc<RwLock<HashMap<MessageId, AdicMessage>>>,
    parents: Arc<RwLock<HashMap<MessageId, Vec<MessageId>>>>,
    children: Arc<RwLock<HashMap<MessageId, Vec<MessageId>>>>,
    tips: Arc<RwLock<HashSet<MessageId>>>,
    metadata: MetadataMap,
    reputation: Arc<RwLock<HashMap<PublicKey, f64>>>,
    finalized: Arc<RwLock<HashMap<MessageId, Vec<u8>>>>,
    conflicts: Arc<RwLock<HashMap<String, HashSet<MessageId>>>>,
    ball_index: BallIndexMap,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self {
            messages: Arc::new(RwLock::new(HashMap::new())),
            parents: Arc::new(RwLock::new(HashMap::new())),
            children: Arc::new(RwLock::new(HashMap::new())),
            tips: Arc::new(RwLock::new(HashSet::new())),
            metadata: Arc::new(RwLock::new(HashMap::new())),
            reputation: Arc::new(RwLock::new(HashMap::new())),
            finalized: Arc::new(RwLock::new(HashMap::new())),
            conflicts: Arc::new(RwLock::new(HashMap::new())),
            ball_index: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StorageBackend for MemoryBackend {
    async fn put_message(&self, message: &AdicMessage) -> Result<()> {
        let mut messages = self.messages.write().await;

        if messages.contains_key(&message.id) {
            return Err(StorageError::AlreadyExists(message.id.to_string()));
        }

        messages.insert(message.id, message.clone());

        // Update parent-child relationships
        let mut parents_map = self.parents.write().await;
        let mut children_map = self.children.write().await;

        parents_map.insert(message.id, message.parents.clone());

        for parent_id in &message.parents {
            children_map
                .entry(*parent_id)
                .or_insert_with(Vec::new)
                .push(message.id);
        }

        Ok(())
    }

    async fn get_message(&self, id: &MessageId) -> Result<Option<AdicMessage>> {
        let messages = self.messages.read().await;
        Ok(messages.get(id).cloned())
    }

    async fn has_message(&self, id: &MessageId) -> Result<bool> {
        let messages = self.messages.read().await;
        Ok(messages.contains_key(id))
    }

    async fn delete_message(&self, id: &MessageId) -> Result<()> {
        let mut messages = self.messages.write().await;
        messages.remove(id);

        let mut parents_map = self.parents.write().await;
        let mut children_map = self.children.write().await;

        // Remove from parent-child mappings
        if let Some(parents) = parents_map.remove(id) {
            for parent_id in parents {
                if let Some(children) = children_map.get_mut(&parent_id) {
                    children.retain(|child| child != id);
                }
            }
        }

        children_map.remove(id);

        Ok(())
    }

    async fn list_messages(&self) -> Result<Vec<MessageId>> {
        let messages = self.messages.read().await;
        Ok(messages.keys().copied().collect())
    }

    async fn add_parent_child(&self, parent: &MessageId, child: &MessageId) -> Result<()> {
        let mut children_map = self.children.write().await;
        children_map
            .entry(*parent)
            .or_insert_with(Vec::new)
            .push(*child);

        let mut parents_map = self.parents.write().await;
        parents_map
            .entry(*child)
            .or_insert_with(Vec::new)
            .push(*parent);

        Ok(())
    }

    async fn get_children(&self, parent: &MessageId) -> Result<Vec<MessageId>> {
        let children = self.children.read().await;
        Ok(children.get(parent).cloned().unwrap_or_default())
    }

    async fn get_parents(&self, child: &MessageId) -> Result<Vec<MessageId>> {
        let parents = self.parents.read().await;
        Ok(parents.get(child).cloned().unwrap_or_default())
    }

    async fn add_tip(&self, id: &MessageId) -> Result<()> {
        let mut tips = self.tips.write().await;
        tips.insert(*id);
        Ok(())
    }

    async fn remove_tip(&self, id: &MessageId) -> Result<()> {
        let mut tips = self.tips.write().await;
        tips.remove(id);
        Ok(())
    }

    async fn get_tips(&self) -> Result<Vec<MessageId>> {
        let tips = self.tips.read().await;
        Ok(tips.iter().copied().collect())
    }

    async fn put_metadata(&self, id: &MessageId, key: &str, value: &[u8]) -> Result<()> {
        let mut metadata = self.metadata.write().await;
        metadata.insert((*id, key.to_string()), value.to_vec());
        Ok(())
    }

    async fn get_metadata(&self, id: &MessageId, key: &str) -> Result<Option<Vec<u8>>> {
        let metadata = self.metadata.read().await;
        Ok(metadata.get(&(*id, key.to_string())).cloned())
    }

    async fn put_reputation(&self, pubkey: &PublicKey, score: f64) -> Result<()> {
        let mut reputation = self.reputation.write().await;
        reputation.insert(*pubkey, score);
        Ok(())
    }

    async fn get_reputation(&self, pubkey: &PublicKey) -> Result<Option<f64>> {
        let reputation = self.reputation.read().await;
        Ok(reputation.get(pubkey).copied())
    }

    async fn mark_finalized(&self, id: &MessageId, artifact: &[u8]) -> Result<()> {
        let mut finalized = self.finalized.write().await;
        finalized.insert(*id, artifact.to_vec());
        Ok(())
    }

    async fn is_finalized(&self, id: &MessageId) -> Result<bool> {
        let finalized = self.finalized.read().await;
        Ok(finalized.contains_key(id))
    }

    async fn get_finality_artifact(&self, id: &MessageId) -> Result<Option<Vec<u8>>> {
        let finalized = self.finalized.read().await;
        Ok(finalized.get(id).cloned())
    }

    async fn add_to_conflict(&self, conflict_id: &str, message_id: &MessageId) -> Result<()> {
        let mut conflicts = self.conflicts.write().await;
        conflicts
            .entry(conflict_id.to_string())
            .or_insert_with(HashSet::new)
            .insert(*message_id);
        Ok(())
    }

    async fn get_conflict_set(&self, conflict_id: &str) -> Result<Vec<MessageId>> {
        let conflicts = self.conflicts.read().await;
        Ok(conflicts
            .get(conflict_id)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default())
    }

    async fn add_to_ball_index(
        &self,
        axis: u32,
        ball_id: &[u8],
        message_id: &MessageId,
    ) -> Result<()> {
        let mut ball_index = self.ball_index.write().await;
        ball_index
            .entry((axis, ball_id.to_vec()))
            .or_insert_with(HashSet::new)
            .insert(*message_id);
        Ok(())
    }

    async fn get_ball_members(&self, axis: u32, ball_id: &[u8]) -> Result<Vec<MessageId>> {
        let ball_index = self.ball_index.read().await;
        Ok(ball_index
            .get(&(axis, ball_id.to_vec()))
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default())
    }

    async fn begin_transaction(&self) -> Result<()> {
        // No-op for memory backend
        Ok(())
    }

    async fn commit_transaction(&self) -> Result<()> {
        // No-op for memory backend
        Ok(())
    }

    async fn rollback_transaction(&self) -> Result<()> {
        // No-op for memory backend
        Ok(())
    }

    async fn flush(&self) -> Result<()> {
        // No-op for memory backend
        Ok(())
    }

    async fn get_stats(&self) -> Result<StorageStats> {
        let messages = self.messages.read().await;
        let tips = self.tips.read().await;
        let finalized = self.finalized.read().await;
        let reputation = self.reputation.read().await;
        let conflicts = self.conflicts.read().await;

        Ok(StorageStats {
            message_count: messages.len(),
            tip_count: tips.len(),
            finalized_count: finalized.len(),
            reputation_entries: reputation.len(),
            conflict_sets: conflicts.len(),
            total_size_bytes: None,
        })
    }
}

impl Clone for MemoryBackend {
    fn clone(&self) -> Self {
        Self {
            messages: Arc::clone(&self.messages),
            parents: Arc::clone(&self.parents),
            children: Arc::clone(&self.children),
            tips: Arc::clone(&self.tips),
            metadata: Arc::clone(&self.metadata),
            reputation: Arc::clone(&self.reputation),
            finalized: Arc::clone(&self.finalized),
            conflicts: Arc::clone(&self.conflicts),
            ball_index: Arc::clone(&self.ball_index),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::{AdicFeatures, AdicMeta, AxisPhi, QpDigits};
    use chrono::Utc;

    #[tokio::test]
    async fn test_memory_backend_basic() {
        let backend = MemoryBackend::new();

        let message = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(10, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![1, 2, 3],
        );

        backend.put_message(&message).await.unwrap();

        let retrieved = backend.get_message(&message.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, message.id);

        assert!(backend.has_message(&message.id).await.unwrap());
    }

    #[tokio::test]
    async fn test_duplicate_message_error() {
        let backend = MemoryBackend::new();

        let message = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(10, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![],
        );

        backend.put_message(&message).await.unwrap();

        // Trying to insert the same message again should fail
        let result = backend.put_message(&message).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            StorageError::AlreadyExists(_)
        ));
    }

    #[tokio::test]
    async fn test_delete_message() {
        let backend = MemoryBackend::new();

        let parent = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(1, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![],
        );

        let child = AdicMessage::new(
            vec![parent.id],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(2, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([1; 32]),
            vec![],
        );

        backend.put_message(&parent).await.unwrap();
        backend.put_message(&child).await.unwrap();

        // Delete child message
        backend.delete_message(&child.id).await.unwrap();

        assert!(!backend.has_message(&child.id).await.unwrap());
        assert!(backend.has_message(&parent.id).await.unwrap());

        // Parent should no longer have child in its children list
        let children = backend.get_children(&parent.id).await.unwrap();
        assert!(!children.contains(&child.id));
    }

    #[tokio::test]
    async fn test_list_messages() {
        let backend = MemoryBackend::new();

        let mut message_ids = vec![];

        for i in 0..5 {
            let message = AdicMessage::new(
                vec![],
                AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(i, 3, 5))]),
                AdicMeta::new(Utc::now()),
                PublicKey::from_bytes([i as u8; 32]),
                vec![],
            );
            message_ids.push(message.id);
            backend.put_message(&message).await.unwrap();
        }

        let listed = backend.list_messages().await.unwrap();
        assert_eq!(listed.len(), 5);

        for id in message_ids {
            assert!(listed.contains(&id));
        }
    }

    #[tokio::test]
    async fn test_tips_management() {
        let backend = MemoryBackend::new();

        let id1 = MessageId::new(b"tip1");
        let id2 = MessageId::new(b"tip2");

        backend.add_tip(&id1).await.unwrap();
        backend.add_tip(&id2).await.unwrap();

        let tips = backend.get_tips().await.unwrap();
        assert_eq!(tips.len(), 2);

        backend.remove_tip(&id1).await.unwrap();

        let tips = backend.get_tips().await.unwrap();
        assert_eq!(tips.len(), 1);
        assert!(tips.contains(&id2));
    }

    #[tokio::test]
    async fn test_parent_child_relationships() {
        let backend = MemoryBackend::new();

        let parent = MessageId::new(b"parent");
        let child1 = MessageId::new(b"child1");
        let child2 = MessageId::new(b"child2");

        backend.add_parent_child(&parent, &child1).await.unwrap();
        backend.add_parent_child(&parent, &child2).await.unwrap();

        let children = backend.get_children(&parent).await.unwrap();
        assert_eq!(children.len(), 2);
        assert!(children.contains(&child1));
        assert!(children.contains(&child2));

        let parents = backend.get_parents(&child1).await.unwrap();
        assert_eq!(parents.len(), 1);
        assert!(parents.contains(&parent));
    }

    #[tokio::test]
    async fn test_metadata() {
        let backend = MemoryBackend::new();

        let msg_id = MessageId::new(b"msg1");
        let key = "test_key";
        let value = b"test_value";

        backend.put_metadata(&msg_id, key, value).await.unwrap();

        let retrieved = backend.get_metadata(&msg_id, key).await.unwrap();
        assert_eq!(retrieved, Some(value.to_vec()));

        let missing = backend.get_metadata(&msg_id, "missing_key").await.unwrap();
        assert_eq!(missing, None);
    }

    #[tokio::test]
    async fn test_reputation() {
        let backend = MemoryBackend::new();

        let pubkey = PublicKey::from_bytes([42; 32]);
        let score = 0.75;

        backend.put_reputation(&pubkey, score).await.unwrap();

        let retrieved = backend.get_reputation(&pubkey).await.unwrap();
        assert_eq!(retrieved, Some(score));

        let missing_key = PublicKey::from_bytes([99; 32]);
        let missing = backend.get_reputation(&missing_key).await.unwrap();
        assert_eq!(missing, None);
    }

    #[tokio::test]
    async fn test_finality() {
        let backend = MemoryBackend::new();

        let msg_id = MessageId::new(b"final_msg");
        let artifact = b"finality_artifact_data";

        assert!(!backend.is_finalized(&msg_id).await.unwrap());

        backend.mark_finalized(&msg_id, artifact).await.unwrap();

        assert!(backend.is_finalized(&msg_id).await.unwrap());

        let retrieved = backend.get_finality_artifact(&msg_id).await.unwrap();
        assert_eq!(retrieved, Some(artifact.to_vec()));
    }

    #[tokio::test]
    async fn test_conflict_sets() {
        let backend = MemoryBackend::new();

        let conflict_id = "conflict_1";
        let msg1 = MessageId::new(b"msg1");
        let msg2 = MessageId::new(b"msg2");
        let msg3 = MessageId::new(b"msg3");

        backend.add_to_conflict(conflict_id, &msg1).await.unwrap();
        backend.add_to_conflict(conflict_id, &msg2).await.unwrap();
        backend.add_to_conflict(conflict_id, &msg3).await.unwrap();

        let conflict_set = backend.get_conflict_set(conflict_id).await.unwrap();
        assert_eq!(conflict_set.len(), 3);
        assert!(conflict_set.contains(&msg1));
        assert!(conflict_set.contains(&msg2));
        assert!(conflict_set.contains(&msg3));

        let empty_set = backend.get_conflict_set("nonexistent").await.unwrap();
        assert!(empty_set.is_empty());
    }

    #[tokio::test]
    async fn test_ball_index() {
        let backend = MemoryBackend::new();

        let axis = 0;
        let ball_id = b"ball_123";
        let msg1 = MessageId::new(b"msg1");
        let msg2 = MessageId::new(b"msg2");

        backend
            .add_to_ball_index(axis, ball_id, &msg1)
            .await
            .unwrap();
        backend
            .add_to_ball_index(axis, ball_id, &msg2)
            .await
            .unwrap();

        let members = backend.get_ball_members(axis, ball_id).await.unwrap();
        assert_eq!(members.len(), 2);
        assert!(members.contains(&msg1));
        assert!(members.contains(&msg2));

        let empty = backend
            .get_ball_members(axis, b"nonexistent")
            .await
            .unwrap();
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn test_transactions() {
        let backend = MemoryBackend::new();

        // Transactions are no-ops for memory backend, but should not error
        backend.begin_transaction().await.unwrap();
        backend.commit_transaction().await.unwrap();
        backend.rollback_transaction().await.unwrap();
        backend.flush().await.unwrap();
    }

    #[tokio::test]
    async fn test_storage_stats() {
        let backend = MemoryBackend::new();

        // Add some data
        let msg1 = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(1, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([1; 32]),
            vec![],
        );

        let msg2 = AdicMessage::new(
            vec![msg1.id],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(2, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([2; 32]),
            vec![],
        );

        backend.put_message(&msg1).await.unwrap();
        backend.put_message(&msg2).await.unwrap();
        backend.add_tip(&msg2.id).await.unwrap();
        backend.mark_finalized(&msg1.id, b"artifact").await.unwrap();
        backend
            .put_reputation(&PublicKey::from_bytes([1; 32]), 1.0)
            .await
            .unwrap();
        backend
            .add_to_conflict("conflict1", &msg1.id)
            .await
            .unwrap();

        let stats = backend.get_stats().await.unwrap();

        assert_eq!(stats.message_count, 2);
        assert_eq!(stats.tip_count, 1);
        assert_eq!(stats.finalized_count, 1);
        assert_eq!(stats.reputation_entries, 1);
        assert_eq!(stats.conflict_sets, 1);
        assert_eq!(stats.total_size_bytes, None);
    }

    #[tokio::test]
    async fn test_concurrent_operations() {
        let backend = MemoryBackend::new();
        let mut handles = vec![];

        // Concurrent message insertions
        for i in 0..10 {
            let backend_clone = backend.clone();
            let handle = tokio::spawn(async move {
                let message = AdicMessage::new(
                    vec![],
                    AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(i, 3, 5))]),
                    AdicMeta::new(Utc::now()),
                    PublicKey::from_bytes([i as u8; 32]),
                    vec![],
                );
                backend_clone.put_message(&message).await
            });
            handles.push(handle);
        }

        for handle in handles {
            assert!(handle.await.unwrap().is_ok());
        }

        let messages = backend.list_messages().await.unwrap();
        assert_eq!(messages.len(), 10);
    }

    #[tokio::test]
    async fn test_default_impl() {
        let backend = MemoryBackend::default();
        let stats = backend.get_stats().await.unwrap();
        assert_eq!(stats.message_count, 0);
    }
}
