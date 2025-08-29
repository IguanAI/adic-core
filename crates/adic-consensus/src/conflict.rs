use adic_types::{ConflictId, MessageId, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct ConflictEnergy {
    pub conflict_id: ConflictId,
    pub energy: f64,
    pub support: HashMap<MessageId, f64>,
    pub last_update: i64,
}

impl ConflictEnergy {
    pub fn new(conflict_id: ConflictId) -> Self {
        Self {
            conflict_id,
            energy: 0.0,
            support: HashMap::new(),
            last_update: chrono::Utc::now().timestamp(),
        }
    }

    pub fn update_support(&mut self, message_id: MessageId, support_value: f64) {
        self.support.insert(message_id, support_value);
        self.recalculate_energy();
    }

    fn recalculate_energy(&mut self) {
        let total: f64 = self.support.values().sum();
        let max = self
            .support
            .values()
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .copied()
            .unwrap_or(0.0);

        self.energy = if total > 0.0 {
            (max / total) - 0.5
        } else {
            0.0
        };

        self.last_update = chrono::Utc::now().timestamp();
    }

    pub fn get_winner(&self) -> Option<MessageId> {
        self.support
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(id, _)| *id)
    }
}

pub struct ConflictResolver {
    conflicts: Arc<RwLock<HashMap<ConflictId, ConflictEnergy>>>,
    energy_cap: f64,
}

impl Default for ConflictResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ConflictResolver {
    pub fn new() -> Self {
        Self {
            conflicts: Arc::new(RwLock::new(HashMap::new())),
            energy_cap: 10.0,
        }
    }

    pub async fn register_conflict(&self, conflict_id: ConflictId) {
        let mut conflicts = self.conflicts.write().await;
        conflicts
            .entry(conflict_id.clone())
            .or_insert_with(|| ConflictEnergy::new(conflict_id));
    }

    pub async fn update_support(
        &self,
        conflict_id: &ConflictId,
        message_id: MessageId,
        reputation: f64,
        depth: u32,
    ) -> Result<()> {
        let mut conflicts = self.conflicts.write().await;

        if let Some(conflict) = conflicts.get_mut(conflict_id) {
            let support_value = reputation / (1.0 + depth as f64);
            conflict.update_support(message_id, support_value);
        } else {
            let mut conflict = ConflictEnergy::new(conflict_id.clone());
            let support_value = reputation / (1.0 + depth as f64);
            conflict.update_support(message_id, support_value);
            conflicts.insert(conflict_id.clone(), conflict);
        }

        Ok(())
    }

    pub async fn get_energy(&self, conflict_id: &ConflictId) -> f64 {
        let conflicts = self.conflicts.read().await;
        conflicts.get(conflict_id).map(|c| c.energy).unwrap_or(0.0)
    }

    pub async fn get_penalty(&self, message_id: &MessageId, conflict_id: &ConflictId) -> f64 {
        let conflicts = self.conflicts.read().await;

        if let Some(conflict) = conflicts.get(conflict_id) {
            if let Some(winner) = conflict.get_winner() {
                if winner != *message_id {
                    return (conflict.energy.abs() * 2.0).min(self.energy_cap);
                }
            }
        }

        0.0
    }

    pub async fn is_resolved(&self, conflict_id: &ConflictId, threshold: f64) -> bool {
        let conflicts = self.conflicts.read().await;
        conflicts
            .get(conflict_id)
            .map(|c| c.energy.abs() > threshold)
            .unwrap_or(false)
    }

    pub async fn get_winner(&self, conflict_id: &ConflictId) -> Option<MessageId> {
        let conflicts = self.conflicts.read().await;
        conflicts.get(conflict_id).and_then(|c| c.get_winner())
    }

    pub async fn cleanup_resolved(&self, threshold: f64, max_age: i64) {
        let now = chrono::Utc::now().timestamp();
        let mut conflicts = self.conflicts.write().await;

        conflicts.retain(|_, conflict| {
            let age = now - conflict.last_update;
            age < max_age || conflict.energy.abs() <= threshold
        });
    }

    pub async fn get_all_conflicts(&self) -> HashMap<ConflictId, ConflictEnergy> {
        let conflicts = self.conflicts.read().await;
        conflicts.clone()
    }

    pub async fn get_resolved_conflicts(&self, threshold: f64) -> Vec<ConflictId> {
        let conflicts = self.conflicts.read().await;
        conflicts
            .iter()
            .filter(|(_, conflict)| conflict.energy.abs() > threshold)
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub async fn get_conflict_details(&self, conflict_id: &ConflictId) -> Option<ConflictEnergy> {
        let conflicts = self.conflicts.read().await;
        conflicts.get(conflict_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_conflict_resolution() {
        let resolver = ConflictResolver::new();
        let conflict_id = ConflictId::new("test-conflict".to_string());

        resolver.register_conflict(conflict_id.clone()).await;

        let msg1 = MessageId::new(b"msg1");
        let msg2 = MessageId::new(b"msg2");

        resolver
            .update_support(&conflict_id, msg1, 2.0, 1)
            .await
            .unwrap();
        resolver
            .update_support(&conflict_id, msg2, 1.0, 1)
            .await
            .unwrap();

        let winner = resolver.get_winner(&conflict_id).await;
        assert_eq!(winner, Some(msg1));

        let energy = resolver.get_energy(&conflict_id).await;
        assert!(energy != 0.0);
    }

    #[tokio::test]
    async fn test_conflict_penalty() {
        let resolver = ConflictResolver::new();
        let conflict_id = ConflictId::new("test-conflict".to_string());

        let msg1 = MessageId::new(b"msg1");
        let msg2 = MessageId::new(b"msg2");

        resolver
            .update_support(&conflict_id, msg1, 3.0, 1)
            .await
            .unwrap();
        resolver
            .update_support(&conflict_id, msg2, 1.0, 1)
            .await
            .unwrap();

        let penalty1 = resolver.get_penalty(&msg1, &conflict_id).await;
        let penalty2 = resolver.get_penalty(&msg2, &conflict_id).await;

        assert_eq!(penalty1, 0.0);
        assert!(penalty2 > 0.0);
    }

    #[test]
    fn test_conflict_energy_new() {
        let conflict_id = ConflictId::new("test".to_string());
        let energy = ConflictEnergy::new(conflict_id.clone());

        assert_eq!(energy.conflict_id, conflict_id);
        assert_eq!(energy.energy, 0.0);
        assert!(energy.support.is_empty());
        assert!(energy.last_update > 0);
    }

    #[test]
    fn test_conflict_energy_update_support() {
        let conflict_id = ConflictId::new("test".to_string());
        let mut energy = ConflictEnergy::new(conflict_id);

        let msg1 = MessageId::new(b"msg1");
        let msg2 = MessageId::new(b"msg2");

        energy.update_support(msg1, 3.0);
        assert_eq!(energy.support.get(&msg1), Some(&3.0));
        assert!(energy.energy != 0.0);

        energy.update_support(msg2, 1.0);
        assert_eq!(energy.support.get(&msg2), Some(&1.0));

        let winner = energy.get_winner();
        assert_eq!(winner, Some(msg1));
    }

    #[test]
    fn test_conflict_energy_recalculate() {
        let conflict_id = ConflictId::new("test".to_string());
        let mut energy = ConflictEnergy::new(conflict_id);

        let msg1 = MessageId::new(b"msg1");
        let msg2 = MessageId::new(b"msg2");
        let msg3 = MessageId::new(b"msg3");

        energy.update_support(msg1, 5.0);
        energy.update_support(msg2, 3.0);
        energy.update_support(msg3, 2.0);

        // Energy should be (max/total) - 0.5 = (5/10) - 0.5 = 0
        assert_eq!(energy.energy, 0.0);

        energy.update_support(msg1, 8.0);
        // Now energy should be (8/13) - 0.5 ≈ 0.115
        assert!(energy.energy > 0.0);
        assert!(energy.energy < 0.2);
    }

    #[tokio::test]
    async fn test_conflict_resolver_is_resolved() {
        let resolver = ConflictResolver::new();
        let conflict_id = ConflictId::new("test-conflict".to_string());

        let msg1 = MessageId::new(b"msg1");
        let msg2 = MessageId::new(b"msg2");

        // Initially not resolved
        assert!(!resolver.is_resolved(&conflict_id, 0.1).await);

        // Add support heavily favoring msg1
        resolver
            .update_support(&conflict_id, msg1, 10.0, 1)
            .await
            .unwrap();
        resolver
            .update_support(&conflict_id, msg2, 1.0, 1)
            .await
            .unwrap();

        // Should be resolved with high energy difference
        let energy = resolver.get_energy(&conflict_id).await;
        assert!(energy.abs() > 0.3);
        assert!(resolver.is_resolved(&conflict_id, 0.3).await);
    }

    #[tokio::test]
    async fn test_conflict_resolver_cleanup() {
        let resolver = ConflictResolver::new();

        // Add multiple conflicts
        for i in 0..5 {
            let conflict_id = ConflictId::new(format!("conflict-{}", i));
            let msg = MessageId::new(&[i; 32]);
            resolver
                .update_support(&conflict_id, msg, 1.0, 1)
                .await
                .unwrap();
        }

        // All should exist initially
        for i in 0..5 {
            let conflict_id = ConflictId::new(format!("conflict-{}", i));
            let energy = resolver.get_energy(&conflict_id).await;
            // Single message gets normalized to energy = (1.0/1.0) - 0.5 = 0.5
            assert_eq!(energy, 0.5);
        }

        // Cleanup with low threshold should remove conflicts with high energy
        // threshold=0.4 means conflicts with energy > 0.4 are resolved and can be removed
        resolver.cleanup_resolved(0.4, i64::MAX).await;

        // All should be retained because energy (0.5) > threshold (0.4)
        // but the function keeps those with energy <= threshold
        // So they should actually be retained
        for i in 0..5 {
            let conflict_id = ConflictId::new(format!("conflict-{}", i));
            let energy = resolver.get_energy(&conflict_id).await;
            assert_eq!(energy, 0.5); // Still exists
        }

        // Now cleanup with higher threshold
        resolver.cleanup_resolved(0.6, i64::MAX).await;

        // All should still be retained because energy (0.5) <= threshold (0.6)
        for i in 0..5 {
            let conflict_id = ConflictId::new(format!("conflict-{}", i));
            let energy = resolver.get_energy(&conflict_id).await;
            assert_eq!(energy, 0.5);
        }
    }

    #[tokio::test]
    async fn test_conflict_penalty_cap() {
        let resolver = ConflictResolver::new();
        let conflict_id = ConflictId::new("test-conflict".to_string());

        let msg1 = MessageId::new(b"msg1");
        let msg2 = MessageId::new(b"msg2");

        // Create extreme support difference
        resolver
            .update_support(&conflict_id, msg1, 1000.0, 1)
            .await
            .unwrap();
        resolver
            .update_support(&conflict_id, msg2, 1.0, 1)
            .await
            .unwrap();

        let penalty = resolver.get_penalty(&msg2, &conflict_id).await;

        // Penalty should be capped at energy_cap (10.0)
        assert!(penalty <= 10.0);
        assert!(penalty > 0.0);
    }

    #[tokio::test]
    async fn test_conflict_with_depth_factor() {
        let resolver = ConflictResolver::new();
        let conflict_id = ConflictId::new("test-conflict".to_string());

        let msg1 = MessageId::new(b"msg1");
        let msg2 = MessageId::new(b"msg2");

        // Same reputation but different depths
        resolver
            .update_support(&conflict_id, msg1, 10.0, 1)
            .await
            .unwrap();
        resolver
            .update_support(&conflict_id, msg2, 10.0, 5)
            .await
            .unwrap();

        // msg1 should win due to lower depth (higher support value)
        let winner = resolver.get_winner(&conflict_id).await;
        assert_eq!(winner, Some(msg1));
    }
}
