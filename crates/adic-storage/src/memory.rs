use crate::backend::{StorageBackend, StorageError, StorageStats, Result};
use adic_types::{MessageId, AdicMessage, PublicKey};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-memory storage backend for testing and development
pub struct MemoryBackend {
    messages: Arc<RwLock<HashMap<MessageId, AdicMessage>>>,
    parents: Arc<RwLock<HashMap<MessageId, Vec<MessageId>>>>,
    children: Arc<RwLock<HashMap<MessageId, Vec<MessageId>>>>,
    tips: Arc<RwLock<HashSet<MessageId>>>,
    metadata: Arc<RwLock<HashMap<(MessageId, String), Vec<u8>>>>,
    reputation: Arc<RwLock<HashMap<PublicKey, f64>>>,
    finalized: Arc<RwLock<HashMap<MessageId, Vec<u8>>>>,
    conflicts: Arc<RwLock<HashMap<String, HashSet<MessageId>>>>,
    ball_index: Arc<RwLock<HashMap<(u32, Vec<u8>), HashSet<MessageId>>>>,
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
    
    async fn get_ball_members(
        &self,
        axis: u32,
        ball_id: &[u8],
    ) -> Result<Vec<MessageId>> {
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
}