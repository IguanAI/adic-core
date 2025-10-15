use adic_types::{AdicMessage, MessageId};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;

// Type aliases for complex types
type MessageIdList = Vec<MessageId>;
type AxisValueIndex = HashMap<Vec<u8>, MessageIdList>;
type AxisIndex = HashMap<u32, AxisValueIndex>;

/// Index for efficient message queries
pub struct MessageIndex {
    /// Index by timestamp (epoch seconds -> message IDs) - BTreeMap for O(log N + K) range queries
    by_timestamp: Arc<RwLock<BTreeMap<i64, MessageIdList>>>,

    /// Index by author (public key bytes -> message IDs)
    by_author: Arc<RwLock<HashMap<[u8; 32], MessageIdList>>>,

    /// Index by axis values (axis -> value -> message IDs)
    by_axis_value: Arc<RwLock<AxisIndex>>,

    /// Depth index (message ID -> depth from genesis)
    depths: Arc<RwLock<HashMap<MessageId, u32>>>,

    /// Descendant count cache
    descendant_counts: Arc<RwLock<HashMap<MessageId, usize>>>,
}

impl Default for MessageIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageIndex {
    pub fn new() -> Self {
        Self {
            by_timestamp: Arc::new(RwLock::new(BTreeMap::new())),
            by_author: Arc::new(RwLock::new(HashMap::new())),
            by_axis_value: Arc::new(RwLock::new(HashMap::new())),
            depths: Arc::new(RwLock::new(HashMap::new())),
            descendant_counts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a message to the index
    pub async fn add_message(&self, message: &AdicMessage, parent_depths: Vec<u32>) {
        let msg_id = message.id;

        // Index by timestamp
        let timestamp = message.meta.timestamp.timestamp();
        let mut by_time = self.by_timestamp.write().await;
        by_time
            .entry(timestamp)
            .or_insert_with(Vec::new)
            .push(msg_id);
        drop(by_time);

        // Index by author
        let mut by_auth = self.by_author.write().await;
        by_auth
            .entry(*message.proposer_pk.as_bytes())
            .or_insert_with(Vec::new)
            .push(msg_id);
        drop(by_auth);

        // Index by axis values
        let mut by_axis = self.by_axis_value.write().await;
        for axis_phi in &message.features.phi {
            by_axis
                .entry(axis_phi.axis.0)
                .or_insert_with(HashMap::new)
                .entry(axis_phi.qp_digits.to_bytes())
                .or_insert_with(Vec::new)
                .push(msg_id);
        }
        drop(by_axis);

        // Calculate and store depth
        let depth = if parent_depths.is_empty() {
            0
        } else {
            parent_depths.iter().max().unwrap() + 1
        };

        let mut depths = self.depths.write().await;
        depths.insert(msg_id, depth);

        // Update descendant counts for all ancestors
        self.update_descendant_counts(msg_id, &message.parents)
            .await;
    }

    /// Remove a message from the index
    pub async fn remove_message(&self, message: &AdicMessage) {
        let msg_id = message.id;

        // Remove from timestamp index
        let timestamp = message.meta.timestamp.timestamp();
        let mut by_time = self.by_timestamp.write().await;
        if let Some(messages) = by_time.get_mut(&timestamp) {
            messages.retain(|&id| id != msg_id);
            if messages.is_empty() {
                by_time.remove(&timestamp);
            }
        }
        drop(by_time);

        // Remove from author index
        let mut by_auth = self.by_author.write().await;
        if let Some(messages) = by_auth.get_mut(message.proposer_pk.as_bytes()) {
            messages.retain(|&id| id != msg_id);
            if messages.is_empty() {
                by_auth.remove(message.proposer_pk.as_bytes());
            }
        }
        drop(by_auth);

        // Remove from axis value index
        let mut by_axis = self.by_axis_value.write().await;
        for axis_phi in &message.features.phi {
            if let Some(axis_map) = by_axis.get_mut(&axis_phi.axis.0) {
                if let Some(messages) = axis_map.get_mut(&axis_phi.qp_digits.to_bytes()) {
                    messages.retain(|&id| id != msg_id);
                    if messages.is_empty() {
                        axis_map.remove(&axis_phi.qp_digits.to_bytes());
                    }
                }
            }
        }
        drop(by_axis);

        // Remove from depth index
        let mut depths = self.depths.write().await;
        depths.remove(&msg_id);

        // Update descendant counts
        self.decrement_descendant_counts(&message.parents).await;
    }

    /// Get messages by timestamp range
    pub async fn get_by_timestamp_range(&self, start: i64, end: i64) -> Vec<MessageId> {
        let by_time = self.by_timestamp.read().await;
        let mut results = Vec::new();

        // BTreeMap range query: O(log N + K) instead of O(N)
        for (_timestamp, messages) in by_time.range(start..=end) {
            results.extend(messages.iter().copied());
        }

        results
    }

    /// Get messages by author
    pub async fn get_by_author(&self, author: &[u8; 32]) -> Vec<MessageId> {
        let by_auth = self.by_author.read().await;
        by_auth.get(author).cloned().unwrap_or_default()
    }

    /// Get messages by axis value
    pub async fn get_by_axis_value(&self, axis: u32, value: &[u8]) -> Vec<MessageId> {
        let by_axis = self.by_axis_value.read().await;
        by_axis
            .get(&axis)
            .and_then(|axis_map| axis_map.get(value))
            .cloned()
            .unwrap_or_default()
    }

    /// Get depth of a message
    pub async fn get_depth(&self, id: &MessageId) -> Option<u32> {
        let depths = self.depths.read().await;
        depths.get(id).copied()
    }

    /// Get descendant count for a message
    pub async fn get_descendant_count(&self, id: &MessageId) -> usize {
        let counts = self.descendant_counts.read().await;
        counts.get(id).copied().unwrap_or(0)
    }

    /// Update descendant counts for ancestors
    async fn update_descendant_counts(&self, _msg_id: MessageId, parents: &[MessageId]) {
        let mut counts = self.descendant_counts.write().await;

        // BFS to update all ancestors
        let mut queue = VecDeque::from(parents.to_vec());
        let mut visited = HashSet::new();

        while let Some(ancestor) = queue.pop_front() {
            if !visited.insert(ancestor) {
                continue;
            }

            *counts.entry(ancestor).or_insert(0) += 1;

            // Note: In practice, we'd need access to parent relationships
            // This is simplified for the example
        }
    }

    /// Decrement descendant counts for ancestors
    async fn decrement_descendant_counts(&self, parents: &[MessageId]) {
        let mut counts = self.descendant_counts.write().await;

        for parent in parents {
            if let Some(count) = counts.get_mut(parent) {
                *count = count.saturating_sub(1);
            }
        }
    }
}

/// Manager for tracking DAG tips
pub struct TipManager {
    tips: Arc<RwLock<HashSet<MessageId>>>,
    tip_scores: Arc<RwLock<HashMap<MessageId, f64>>>,
}

impl Default for TipManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TipManager {
    pub fn new() -> Self {
        Self {
            tips: Arc::new(RwLock::new(HashSet::new())),
            tip_scores: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a new tip
    pub async fn add_tip(&self, id: MessageId, score: f64) {
        let mut tips = self.tips.write().await;
        tips.insert(id);

        let mut scores = self.tip_scores.write().await;
        scores.insert(id, score);
    }

    /// Remove a tip (when it becomes a parent)
    pub async fn remove_tip(&self, id: &MessageId) -> bool {
        let mut tips = self.tips.write().await;
        let removed = tips.remove(id);

        if removed {
            let mut scores = self.tip_scores.write().await;
            scores.remove(id);
        }

        removed
    }

    /// Get all current tips
    pub async fn get_tips(&self) -> Vec<MessageId> {
        let tips = self.tips.read().await;
        tips.iter().copied().collect()
    }

    /// Get tips sorted by score
    pub async fn get_tips_by_score(&self) -> Vec<(MessageId, f64)> {
        let scores = self.tip_scores.read().await;
        let mut tips: Vec<_> = scores.iter().map(|(&id, &score)| (id, score)).collect();
        tips.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        tips
    }

    /// Update score for a tip
    pub async fn update_score(&self, id: &MessageId, score: f64) -> bool {
        let tips = self.tips.read().await;
        if !tips.contains(id) {
            return false;
        }
        drop(tips);

        let mut scores = self.tip_scores.write().await;
        scores.insert(*id, score);
        true
    }

    /// Get the score of a tip
    pub async fn get_score(&self, id: &MessageId) -> Option<f64> {
        let scores = self.tip_scores.read().await;
        scores.get(id).copied()
    }

    /// Get top N tips by score
    pub async fn get_top_tips(&self, n: usize) -> Vec<(MessageId, f64)> {
        let mut tips = self.get_tips_by_score().await;
        tips.truncate(n);
        tips
    }

    /// Check if a message is a tip
    pub async fn is_tip(&self, id: &MessageId) -> bool {
        let tips = self.tips.read().await;
        tips.contains(id)
    }

    /// Get tip count
    pub async fn tip_count(&self) -> usize {
        let tips = self.tips.read().await;
        tips.len()
    }

    /// Clear all tips (used for reset)
    pub async fn clear(&self) {
        let mut tips = self.tips.write().await;
        tips.clear();

        let mut scores = self.tip_scores.write().await;
        scores.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::{AdicFeatures, AdicMeta, AxisPhi, PublicKey, QpDigits};
    use chrono::Utc;

    #[tokio::test]
    async fn test_message_index() {
        let index = MessageIndex::new();

        let message = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![
                AxisPhi::new(0, QpDigits::from_u64(10, 3, 5)),
                AxisPhi::new(1, QpDigits::from_u64(20, 3, 5)),
            ]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([1; 32]),
            vec![],
        );

        index.add_message(&message, vec![]).await;

        // Test by author lookup
        let by_author = index.get_by_author(&[1; 32]).await;
        assert_eq!(by_author.len(), 1);
        assert_eq!(by_author[0], message.id);

        // Test depth
        let depth = index.get_depth(&message.id).await;
        assert_eq!(depth, Some(0));

        // Test removal
        index.remove_message(&message).await;
        let by_author = index.get_by_author(&[1; 32]).await;
        assert_eq!(by_author.len(), 0);
    }

    #[tokio::test]
    async fn test_tip_manager() {
        let manager = TipManager::new();

        let id1 = MessageId::new(b"tip1");
        let id2 = MessageId::new(b"tip2");
        let id3 = MessageId::new(b"tip3");

        manager.add_tip(id1, 0.5).await;
        manager.add_tip(id2, 0.8).await;
        manager.add_tip(id3, 0.3).await;

        assert_eq!(manager.tip_count().await, 3);
        assert!(manager.is_tip(&id1).await);

        // Test getting by score
        let by_score = manager.get_tips_by_score().await;
        assert_eq!(by_score[0].0, id2); // Highest score
        assert_eq!(by_score[1].0, id1);
        assert_eq!(by_score[2].0, id3); // Lowest score

        // Test top N
        let top_2 = manager.get_top_tips(2).await;
        assert_eq!(top_2.len(), 2);
        assert_eq!(top_2[0].0, id2);

        // Test removal
        assert!(manager.remove_tip(&id1).await);
        assert!(!manager.is_tip(&id1).await);
        assert_eq!(manager.tip_count().await, 2);
    }
}
