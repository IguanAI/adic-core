use adic_types::PublicKey;
use chrono;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationScore {
    pub value: f64,
    pub messages_finalized: u64,
    pub messages_pending: u64,
    pub last_update: i64,
}

impl Default for ReputationScore {
    fn default() -> Self {
        Self::new()
    }
}

impl ReputationScore {
    pub fn new() -> Self {
        Self {
            value: 1.0,
            messages_finalized: 0,
            messages_pending: 0,
            last_update: chrono::Utc::now().timestamp(),
        }
    }

    pub fn trust_score(&self) -> f64 {
        (1.0 + self.value).ln()
    }
}

pub struct ReputationTracker {
    scores: Arc<RwLock<HashMap<PublicKey, ReputationScore>>>,
    decay_factor: f64,
    gamma: f64,
}

impl ReputationTracker {
    pub fn new(gamma: f64) -> Self {
        Self {
            scores: Arc::new(RwLock::new(HashMap::new())),
            decay_factor: 0.99,
            gamma,
        }
    }

    pub async fn get_reputation(&self, pubkey: &PublicKey) -> f64 {
        let scores = self.scores.read().await;
        scores.get(pubkey).map(|s| s.value).unwrap_or(1.0)
    }

    pub async fn get_trust_score(&self, pubkey: &PublicKey) -> f64 {
        let scores = self.scores.read().await;
        scores.get(pubkey).map(|s| s.trust_score()).unwrap_or(0.0)
    }

    /// Update reputation positively when message is finalized
    /// Uses adjusted formula: R_{t+1} = γR_t + (1−γ) * (1 + diversity/(1+depth))
    pub async fn good_update(&self, pubkey: &PublicKey, diversity: f64, depth: u32) {
        let mut scores = self.scores.write().await;
        let score = scores.entry(*pubkey).or_insert_with(ReputationScore::new);

        // Calculate good score with baseline 1.0 to reward finalization
        let good_score = 1.0 + diversity / (1.0 + depth as f64);

        // Apply update: R_{t+1} = γR_t + (1−γ) * good
        let old_rep = score.value;
        score.value = (self.gamma * old_rep + (1.0 - self.gamma) * good_score).min(10.0); // Cap at 10.0
        score.messages_finalized += 1;
        score.last_update = chrono::Utc::now().timestamp();
    }

    /// Update reputation negatively when message is invalid/slashed
    /// Uses paper formula: R_{t+1} = γR_t + (1−γ) * (-penalty)
    pub async fn bad_update(&self, pubkey: &PublicKey, penalty: f64) {
        let mut scores = self.scores.write().await;
        let score = scores.entry(*pubkey).or_insert_with(ReputationScore::new);

        // Apply paper formula with negative penalty: R_{t+1} = γR_t + (1−γ) * (-penalty)
        let old_rep = score.value;
        score.value = (self.gamma * old_rep + (1.0 - self.gamma) * (-penalty)).max(0.1); // Floor at 0.1
        score.last_update = chrono::Utc::now().timestamp();
    }

    /// Apply time-based decay to all reputations
    pub async fn apply_decay(&self) {
        let mut scores = self.scores.write().await;
        let now = chrono::Utc::now().timestamp();

        for score in scores.values_mut() {
            let time_diff = (now - score.last_update) as f64 / 86400.0; // days
            if time_diff > 0.0 {
                score.value = (score.value * self.decay_factor.powf(time_diff)).max(0.1);
                score.last_update = now;
            }
        }
    }

    /// Get all reputation scores
    pub async fn get_all_scores(&self) -> HashMap<PublicKey, ReputationScore> {
        let scores = self.scores.read().await;
        scores.clone()
    }

    /// Set reputation for testing
    pub async fn set_reputation(&self, pubkey: &PublicKey, value: f64) {
        let mut scores = self.scores.write().await;
        let score = scores.entry(*pubkey).or_insert_with(ReputationScore::new);
        score.value = value;
        score.last_update = chrono::Utc::now().timestamp();
    }
}

impl Default for ReputationTracker {
    fn default() -> Self {
        Self::new(0.9)
    }
}

impl Clone for ReputationTracker {
    fn clone(&self) -> Self {
        Self {
            scores: Arc::clone(&self.scores),
            decay_factor: self.decay_factor,
            gamma: self.gamma,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_reputation_updates() {
        let tracker = ReputationTracker::new(0.9);
        let pubkey = PublicKey::from_bytes([1; 32]);

        // Initial reputation should be 1.0
        assert_eq!(tracker.get_reputation(&pubkey).await, 1.0);

        // Good update should increase reputation
        tracker.good_update(&pubkey, 5.0, 10).await; // diversity=5.0, depth=10
        assert!(tracker.get_reputation(&pubkey).await > 1.0);

        // Bad update should decrease reputation
        tracker.bad_update(&pubkey, 2.0).await;
        let final_rep = tracker.get_reputation(&pubkey).await;
        assert!(final_rep < 1.1); // Should be decreased
    }

    #[tokio::test]
    async fn test_trust_score() {
        let tracker = ReputationTracker::new(0.1);
        let pubkey = PublicKey::from_bytes([2; 32]);

        tracker.set_reputation(&pubkey, 5.0).await;
        let trust = tracker.get_trust_score(&pubkey).await;
        assert_eq!(trust, (1.0f64 + 5.0f64).ln());
    }

    #[test]
    fn test_reputation_score_new() {
        let score = ReputationScore::new();
        assert_eq!(score.value, 1.0);
        assert_eq!(score.messages_finalized, 0);
        assert_eq!(score.messages_pending, 0);
        assert!(score.last_update > 0);
    }

    #[test]
    fn test_reputation_score_trust() {
        let mut score = ReputationScore::new();
        score.value = 0.0;
        assert_eq!(score.trust_score(), 1.0f64.ln());

        score.value = 1.0;
        assert_eq!(score.trust_score(), 2.0f64.ln());

        score.value = 5.0;
        assert_eq!(score.trust_score(), 6.0f64.ln());
    }

    #[tokio::test]
    async fn test_reputation_cap() {
        let tracker = ReputationTracker::new(0.5);
        let pubkey = PublicKey::from_bytes([3; 32]);

        // Multiple good updates
        for _ in 0..20 {
            tracker.good_update(&pubkey, 10.0, 1).await;
        }

        // Should be capped at 10.0
        let rep = tracker.get_reputation(&pubkey).await;
        assert!(rep <= 10.0);
    }

    #[tokio::test]
    async fn test_reputation_floor() {
        let tracker = ReputationTracker::new(0.5);
        let pubkey = PublicKey::from_bytes([4; 32]);

        // Multiple bad updates
        for _ in 0..20 {
            tracker.bad_update(&pubkey, 5.0).await;
        }

        // Should have floor at 0.1
        let rep = tracker.get_reputation(&pubkey).await;
        assert!(rep >= 0.1);
    }

    #[tokio::test]
    async fn test_gamma_factor() {
        let tracker_high_gamma = ReputationTracker::new(0.9);
        let tracker_low_gamma = ReputationTracker::new(0.1);

        let pubkey1 = PublicKey::from_bytes([5; 32]);
        let pubkey2 = PublicKey::from_bytes([6; 32]);

        // Set initial reputation
        tracker_high_gamma.set_reputation(&pubkey1, 5.0).await;
        tracker_low_gamma.set_reputation(&pubkey2, 5.0).await;

        // Apply same good update
        tracker_high_gamma.good_update(&pubkey1, 3.0, 1).await;
        tracker_low_gamma.good_update(&pubkey2, 3.0, 1).await;

        let rep1 = tracker_high_gamma.get_reputation(&pubkey1).await;
        let rep2 = tracker_low_gamma.get_reputation(&pubkey2).await;

        // High gamma should preserve more of the old value
        assert!(rep1 > rep2);
    }

    #[tokio::test]
    async fn test_apply_decay() {
        let tracker = ReputationTracker::new(0.9);
        let pubkey = PublicKey::from_bytes([7; 32]);

        tracker.set_reputation(&pubkey, 5.0).await;

        // Manually set last_update to past
        {
            let mut scores = tracker.scores.write().await;
            if let Some(score) = scores.get_mut(&pubkey) {
                score.last_update = chrono::Utc::now().timestamp() - 86400; // 1 day ago
            }
        }

        tracker.apply_decay().await;

        let rep = tracker.get_reputation(&pubkey).await;
        assert!(rep < 5.0); // Should have decayed
        assert!(rep > 4.0); // But not too much with decay_factor = 0.99
    }

    #[tokio::test]
    async fn test_get_all_scores() {
        let tracker = ReputationTracker::new(0.9);

        // Add multiple reputations
        for i in 0..5 {
            let pubkey = PublicKey::from_bytes([i; 32]);
            tracker.set_reputation(&pubkey, (i + 1) as f64).await;
        }

        let all_scores = tracker.get_all_scores().await;
        assert_eq!(all_scores.len(), 5);

        for i in 0..5 {
            let pubkey = PublicKey::from_bytes([i; 32]);
            assert_eq!(all_scores.get(&pubkey).unwrap().value, (i + 1) as f64);
        }
    }

    #[tokio::test]
    async fn test_message_finalized_count() {
        let tracker = ReputationTracker::new(0.9);
        let pubkey = PublicKey::from_bytes([8; 32]);

        // Multiple good updates should increment finalized count
        for i in 0..3 {
            tracker.good_update(&pubkey, 2.0, i).await;
        }

        let scores = tracker.get_all_scores().await;
        let score = scores.get(&pubkey).unwrap();
        assert_eq!(score.messages_finalized, 3);
    }

    #[tokio::test]
    async fn test_depth_impact_on_good_update() {
        let tracker = ReputationTracker::new(0.5);
        let pubkey1 = PublicKey::from_bytes([9; 32]);
        let pubkey2 = PublicKey::from_bytes([10; 32]);

        // Same diversity, different depths
        tracker.good_update(&pubkey1, 5.0, 1).await;
        tracker.good_update(&pubkey2, 5.0, 10).await;

        let rep1 = tracker.get_reputation(&pubkey1).await;
        let rep2 = tracker.get_reputation(&pubkey2).await;

        // Lower depth should result in higher reputation gain
        assert!(rep1 > rep2);
    }

    #[tokio::test]
    async fn test_default_tracker() {
        let tracker = ReputationTracker::default();
        let pubkey = PublicKey::from_bytes([11; 32]);

        // Default gamma should be 0.9
        assert_eq!(tracker.get_reputation(&pubkey).await, 1.0);

        tracker.good_update(&pubkey, 3.0, 2).await;
        let rep = tracker.get_reputation(&pubkey).await;
        assert!(rep > 1.0);
    }

    #[tokio::test]
    async fn test_concurrent_updates() {
        let tracker = ReputationTracker::new(0.5);
        let pubkey = PublicKey::from_bytes([12; 32]);

        let mut handles = vec![];

        // Spawn multiple concurrent good updates
        for i in 0..10 {
            let tracker_clone = tracker.clone();
            let pubkey_clone = pubkey;
            let handle = tokio::spawn(async move {
                tracker_clone.good_update(&pubkey_clone, 1.0, i).await;
            });
            handles.push(handle);
        }

        // Wait for all to complete
        for handle in handles {
            handle.await.unwrap();
        }

        // Check final state
        let scores = tracker.get_all_scores().await;
        let score = scores.get(&pubkey).unwrap();
        assert_eq!(score.messages_finalized, 10);
        assert!(score.value > 1.0);
    }
}
