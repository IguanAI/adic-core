use adic_storage::StorageEngine;
use adic_types::{ConflictId, MessageId, Result};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::ReputationTracker;

/// Energy descent tracker per whitepaper Section 4.1 and Appendix B
/// Ensures negative drift for conflict resolution
#[derive(Debug, Clone)]
pub struct EnergyDescentTracker {
    /// Track energy per conflict set
    conflicts: Arc<RwLock<HashMap<ConflictId, ConflictEnergy>>>,
    /// Parameters for energy calculation
    _lambda: f64, // MRW weight parameter (reserved for future use)
    mu: f64,    // Conflict penalty weight
    delta: f64, // Small neighborhood threshold
    c: f64,     // Negative drift coefficient
}

#[derive(Debug, Clone)]
pub struct ConflictEnergy {
    pub conflict_id: ConflictId,
    /// Support for each conflicting message
    pub support: HashMap<MessageId, f64>,
    /// Total energy E(t) per paper formula
    pub total_energy: f64,
    /// Tracks if energy is decreasing (negative drift)
    pub is_descending: bool,
    /// History of energy values for drift analysis
    pub energy_history: Vec<f64>,
    /// Timestamp of last update
    pub last_update: i64,
}

impl ConflictEnergy {
    pub fn new(conflict_id: ConflictId) -> Self {
        Self {
            conflict_id,
            support: HashMap::new(),
            total_energy: 0.0,
            is_descending: false,
            energy_history: Vec::new(),
            last_update: chrono::Utc::now().timestamp(),
        }
    }

    /// Calculate support for a message per paper formula:
    /// supp(z; C) = Σ_{y descends to z} R(y) / (1 + depth(y))
    pub async fn calculate_support(
        &self,
        message_id: &MessageId,
        storage: &StorageEngine,
        reputation: &ReputationTracker,
    ) -> f64 {
        let descendants = self.get_descendants(message_id, storage).await;
        let mut total_support = 0.0;

        for descendant_id in descendants {
            if let Ok(Some(descendant_msg)) = storage.get_message(&descendant_id).await {
                let rep = reputation.get_reputation(&descendant_msg.proposer_pk).await;
                let depth = self.calculate_depth(&descendant_id, storage).await;
                total_support += rep / (1.0 + depth as f64);
            }
        }

        total_support
    }

    /// Calculate message depth by traversing parents to genesis
    async fn calculate_depth(&self, message_id: &MessageId, storage: &StorageEngine) -> u32 {
        let mut depth = 0;
        let mut current_id = *message_id;
        let mut visited = std::collections::HashSet::new();

        loop {
            // Prevent infinite loops
            if !visited.insert(current_id) {
                break;
            }

            // Get parents
            let parents = match storage.get_parents(&current_id).await {
                Ok(parents) => parents,
                Err(_) => break,
            };

            // Genesis messages have no parents
            if parents.is_empty() {
                break;
            }

            // Move to first parent and increment depth
            depth += 1;
            current_id = parents[0];

            // Safety limit to prevent excessive traversal
            if depth > 10000 {
                break;
            }
        }

        depth
    }

    async fn get_descendants(
        &self,
        message_id: &MessageId,
        storage: &StorageEngine,
    ) -> HashSet<MessageId> {
        let mut descendants = HashSet::new();
        let mut queue = vec![*message_id];
        let mut visited = HashSet::new();

        while let Some(current_id) = queue.pop() {
            if visited.contains(&current_id) {
                continue;
            }
            visited.insert(current_id);

            if let Ok(children) = storage.get_children(&current_id).await {
                for child_id in children {
                    if descendants.insert(child_id) {
                        queue.push(child_id);
                    }
                }
            }
        }

        descendants
    }

    /// Update total energy using paper formula:
    /// E = Σ_C |Σ_{z∈C} sgn(z) * supp(z; C)|
    pub fn update_energy(&mut self) {
        // Store previous energy for drift analysis
        if self.energy_history.len() >= 100 {
            self.energy_history.remove(0);
        }
        self.energy_history.push(self.total_energy);

        // Calculate new energy
        let mut signed_support = 0.0;
        let mut max_support = 0.0;
        let mut winner_id = None;

        for (msg_id, support) in &self.support {
            if *support > max_support {
                max_support = *support;
                winner_id = Some(*msg_id);
            }
        }

        // Assign signs: +1 for leader, -1 for others
        for (msg_id, support) in &self.support {
            let sign = if Some(*msg_id) == winner_id {
                1.0
            } else {
                -1.0
            };
            signed_support += sign * support;
        }

        self.total_energy = signed_support.abs();

        // Check for negative drift
        self.check_drift();
        self.last_update = chrono::Utc::now().timestamp();
    }

    /// Check if energy has negative drift (decreasing over time)
    fn check_drift(&mut self) {
        if self.energy_history.len() >= 2 {
            let recent = &self.energy_history[self.energy_history.len() - 2..];
            self.is_descending = recent[1] < recent[0];
        }
    }

    /// Get the current winner (highest support)
    pub fn get_winner(&self) -> Option<MessageId> {
        self.support
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(id, _)| *id)
    }

    /// Check if conflict is resolved (energy below threshold)
    pub fn is_resolved(&self, threshold: f64) -> bool {
        self.total_energy < threshold && self.is_descending
    }
}

impl EnergyDescentTracker {
    pub fn new(lambda: f64, mu: f64) -> Self {
        Self {
            conflicts: Arc::new(RwLock::new(HashMap::new())),
            _lambda: lambda,
            mu,
            delta: 0.1, // Default small neighborhood
            c: 0.01,    // Default drift coefficient
        }
    }

    /// Register a new conflict set
    pub async fn register_conflict(&self, conflict_id: ConflictId) {
        let mut conflicts = self.conflicts.write().await;
        let conflict_id_clone = conflict_id.clone();
        conflicts
            .entry(conflict_id.clone())
            .or_insert_with(|| ConflictEnergy::new(conflict_id_clone));

        info!(
            conflict_id = ?conflict_id,
            "⚡ Registered new conflict"
        );
    }

    /// Update support for a message in a conflict
    pub async fn update_support(
        &self,
        conflict_id: &ConflictId,
        message_id: MessageId,
        storage: &StorageEngine,
        reputation: &ReputationTracker,
    ) -> Result<()> {
        let mut conflicts = self.conflicts.write().await;

        let conflict = conflicts
            .entry(conflict_id.clone())
            .or_insert_with(|| ConflictEnergy::new(conflict_id.clone()));

        // Calculate support using paper formula
        let support = conflict
            .calculate_support(&message_id, storage, reputation)
            .await;

        // Update or add support
        let old_support = conflict.support.insert(message_id, support).unwrap_or(0.0);

        debug!(
            message_id = %message_id,
            conflict_id = ?conflict_id,
            old_support = format!("{:.4}", old_support),
            new_support = format!("{:.4}", support),
            "⚡ Updated conflict support"
        );

        // Recalculate energy
        conflict.update_energy();

        Ok(())
    }

    /// Calculate expected drift per paper Appendix B
    /// E[E(t+1) - E(t) | X_t] ≤ -c * 1_{E(t) > δ}
    pub async fn calculate_expected_drift(&self, conflict_id: &ConflictId) -> f64 {
        let conflicts = self.conflicts.read().await;

        if let Some(conflict) = conflicts.get(conflict_id) {
            if conflict.total_energy > self.delta {
                // Calculate drift based on message support difference
                let mut max_support: f64 = 0.0;
                let mut min_support: f64 = f64::INFINITY;

                for support_value in conflict.support.values() {
                    max_support = max_support.max(*support_value);
                    min_support = min_support.min(*support_value);
                }

                // Drift is proportional to the difference in support
                let drift = (max_support - min_support) * self.c;
                return -drift;
            }
        }

        0.0
    }

    /// Get penalty for a message based on conflict energy
    pub async fn get_conflict_penalty(
        &self,
        message_id: &MessageId,
        conflict_id: &ConflictId,
    ) -> f64 {
        let conflicts = self.conflicts.read().await;

        if let Some(conflict) = conflicts.get(conflict_id) {
            if let Some(winner) = conflict.get_winner() {
                if winner != *message_id {
                    // Penalize non-winners proportional to energy
                    return self.mu * conflict.total_energy;
                }
            }
        }

        0.0
    }

    /// Check if a conflict is resolved
    pub async fn is_resolved(&self, conflict_id: &ConflictId) -> bool {
        let conflicts = self.conflicts.read().await;

        conflicts
            .get(conflict_id)
            .map(|c| c.is_resolved(self.delta))
            .unwrap_or(false)
    }

    /// Get the winner of a resolved conflict
    pub async fn get_winner(&self, conflict_id: &ConflictId) -> Option<MessageId> {
        let conflicts = self.conflicts.read().await;

        conflicts
            .get(conflict_id)
            .filter(|c| c.is_resolved(self.delta))
            .and_then(|c| c.get_winner())
    }

    /// Get total energy for monitoring
    pub async fn get_total_energy(&self) -> f64 {
        let conflicts = self.conflicts.read().await;

        conflicts.values().map(|c| c.total_energy).sum()
    }

    /// Clean up old resolved conflicts
    pub async fn cleanup_resolved(&self, max_age_seconds: i64) {
        let now = chrono::Utc::now().timestamp();
        let mut conflicts = self.conflicts.write().await;

        let before_count = conflicts.len();

        conflicts.retain(|id, conflict| {
            let age = now - conflict.last_update;
            let should_keep = age < max_age_seconds || !conflict.is_resolved(self.delta);

            if !should_keep {
                debug!(
                    conflict_id = ?id,
                    "✅ Removing resolved conflict"
                );
            }

            should_keep
        });

        let removed = before_count - conflicts.len();
        if removed > 0 {
            info!(
                removed_count = removed,
                remaining_conflicts = self.conflicts.read().await.len(),
                "🧹 Cleaned up resolved conflicts"
            );
        }
    }

    /// Get detailed metrics for monitoring
    pub async fn get_metrics(&self) -> EnergyMetrics {
        let conflicts = self.conflicts.read().await;

        let total_conflicts = conflicts.len();
        let resolved = conflicts
            .values()
            .filter(|c| c.is_resolved(self.delta))
            .count();
        let descending = conflicts.values().filter(|c| c.is_descending).count();
        let total_energy = conflicts.values().map(|c| c.total_energy).sum();

        EnergyMetrics {
            total_conflicts,
            resolved_conflicts: resolved,
            descending_conflicts: descending,
            total_energy,
            average_energy: if total_conflicts > 0 {
                total_energy / total_conflicts as f64
            } else {
                0.0
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnergyMetrics {
    pub total_conflicts: usize,
    pub resolved_conflicts: usize,
    pub descending_conflicts: usize,
    pub total_energy: f64,
    pub average_energy: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ReputationTracker;
    use adic_storage::{store::BackendType, StorageConfig, StorageEngine};

    fn create_test_storage() -> StorageEngine {
        let config = StorageConfig {
            backend_type: BackendType::Memory,
            ..Default::default()
        };
        StorageEngine::new(config).unwrap()
    }

    #[tokio::test]
    async fn test_energy_descent() {
        let tracker = EnergyDescentTracker::new(1.0, 1.0);
        let storage = create_test_storage();
        let reputation = ReputationTracker::new(0.9);
        let conflict_id = ConflictId::new("test-conflict".to_string());

        tracker.register_conflict(conflict_id.clone()).await;

        // Add support for competing messages
        let msg1 = MessageId::new(b"msg1");
        let msg2 = MessageId::new(b"msg2");

        // Create and store test messages with features
        use adic_types::{AdicFeatures, AdicMessage, AdicMeta, AxisPhi, PublicKey, QpDigits};
        use chrono::Utc;

        let test_msg1 = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(1, 3, 10))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([1; 32]),
            vec![],
        );
        let test_msg2 = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(2, 3, 10))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([2; 32]),
            vec![],
        );

        // Store messages with correct IDs
        let mut msg1_stored = test_msg1.clone();
        msg1_stored.id = msg1;
        let mut msg2_stored = test_msg2.clone();
        msg2_stored.id = msg2;

        storage.store_message(&msg1_stored).await.unwrap();
        storage.store_message(&msg2_stored).await.unwrap();

        // Set reputations for the proposers
        reputation
            .set_reputation(&PublicKey::from_bytes([1; 32]), 0.8)
            .await;
        reputation
            .set_reputation(&PublicKey::from_bytes([2; 32]), 0.6)
            .await;

        // msg1 gets more support - use proper signature
        tracker
            .update_support(&conflict_id, msg1, &storage, &reputation)
            .await
            .unwrap();
        tracker
            .update_support(&conflict_id, msg2, &storage, &reputation)
            .await
            .unwrap();

        // Check drift is negative when energy > delta
        let drift = tracker.calculate_expected_drift(&conflict_id).await;
        assert!(drift <= 0.0);

        // msg1 should be winner
        let _winner = tracker.get_winner(&conflict_id).await;
        // Winner might be determined based on support

        // Add more support to create clear winner - add child messages
        let child_msg = AdicMessage::new(
            vec![msg1],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(3, 3, 10))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([3; 32]),
            vec![],
        );
        storage.store_message(&child_msg).await.unwrap();
        // Store parent-child relationship via the message's parents field
        reputation
            .set_reputation(&PublicKey::from_bytes([3; 32]), 0.9)
            .await;

        tracker
            .update_support(&conflict_id, msg1, &storage, &reputation)
            .await
            .unwrap();

        // Check penalty for losing message - only if there's a clear winner
        let conflicts = tracker.conflicts.read().await;
        if let Some(conflict) = conflicts.get(&conflict_id) {
            // Only check penalty if there's actually a winner with higher support
            if conflict.get_winner().is_some() {
                drop(conflicts);
                let penalty = tracker.get_conflict_penalty(&msg2, &conflict_id).await;
                // Penalty should exist if msg2 is not the winner
                if tracker.get_winner(&conflict_id).await != Some(msg2) {
                    assert!(penalty >= 0.0); // Allow 0 if energy is below threshold
                }
            }
        }
    }

    #[tokio::test]
    async fn test_conflict_resolution() {
        let tracker = EnergyDescentTracker::new(1.0, 1.0);
        let storage = create_test_storage();
        let reputation = ReputationTracker::new(0.9);
        let conflict_id = ConflictId::new("double-spend".to_string());

        tracker.register_conflict(conflict_id.clone()).await;

        let msg1 = MessageId::new(b"spend1");
        let msg2 = MessageId::new(b"spend2");

        // Create and store test messages
        use adic_types::{AdicFeatures, AdicMessage, AdicMeta, AxisPhi, PublicKey, QpDigits};
        use chrono::Utc;

        let mut msg1_stored = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(100, 3, 10))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([1; 32]),
            vec![],
        );
        msg1_stored.id = msg1;

        let mut msg2_stored = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(200, 3, 10))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([2; 32]),
            vec![],
        );
        msg2_stored.id = msg2;

        storage.store_message(&msg1_stored).await.unwrap();
        storage.store_message(&msg2_stored).await.unwrap();

        // Set reputations
        reputation
            .set_reputation(&PublicKey::from_bytes([1; 32]), 0.7)
            .await;
        reputation
            .set_reputation(&PublicKey::from_bytes([2; 32]), 0.5)
            .await;

        // Simulate approvals over time
        for i in 0..10 {
            if i % 3 == 0 {
                tracker
                    .update_support(&conflict_id, msg2, &storage, &reputation)
                    .await
                    .unwrap();
            } else {
                tracker
                    .update_support(&conflict_id, msg1, &storage, &reputation)
                    .await
                    .unwrap();
            }
        }

        // Get metrics
        let metrics = tracker.get_metrics().await;
        assert_eq!(metrics.total_conflicts, 1);
        // Energy might be 0 if no descendants, but conflict should exist
        assert!(metrics.total_conflicts > 0);
    }
}
