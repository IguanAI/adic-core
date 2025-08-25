use adic_types::{MessageId, ConflictId, Result};
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
        let max = self.support.values().max_by(|a, b| a.partial_cmp(b).unwrap()).copied().unwrap_or(0.0);
        
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

impl ConflictResolver {
    pub fn new() -> Self {
        Self {
            conflicts: Arc::new(RwLock::new(HashMap::new())),
            energy_cap: 10.0,
        }
    }

    pub async fn register_conflict(&self, conflict_id: ConflictId) {
        let mut conflicts = self.conflicts.write().await;
        conflicts.entry(conflict_id.clone()).or_insert_with(|| ConflictEnergy::new(conflict_id));
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
        conflicts.get(conflict_id).map(|c| c.energy.abs() > threshold).unwrap_or(false)
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
        
        resolver.update_support(&conflict_id, msg1, 2.0, 1).await.unwrap();
        resolver.update_support(&conflict_id, msg2, 1.0, 1).await.unwrap();
        
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
        
        resolver.update_support(&conflict_id, msg1, 3.0, 1).await.unwrap();
        resolver.update_support(&conflict_id, msg2, 1.0, 1).await.unwrap();
        
        let penalty1 = resolver.get_penalty(&msg1, &conflict_id).await;
        let penalty2 = resolver.get_penalty(&msg2, &conflict_id).await;
        
        assert_eq!(penalty1, 0.0);
        assert!(penalty2 > 0.0);
    }
}