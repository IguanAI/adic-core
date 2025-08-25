use adic_types::PublicKey;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono;

#[derive(Debug, Clone)]
pub struct ReputationScore {
    pub value: f64,
    pub messages_finalized: u64,
    pub messages_pending: u64,
    pub last_update: i64,
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
    /// Uses paper formula: R_{t+1} = γR_t + (1−γ) * good, where good = diversity/(1+depth)
    pub async fn good_update(&self, pubkey: &PublicKey, diversity: f64, depth: u32) {
        let mut scores = self.scores.write().await;
        let score = scores.entry(*pubkey).or_insert_with(ReputationScore::new);
        
        // Calculate good score: diversity / (1 + depth)
        let good_score = diversity / (1.0 + depth as f64);
        
        // Apply paper formula: R_{t+1} = γR_t + (1−γ) * good
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
        tracker.good_update(&pubkey, 5.0, 10).await;  // diversity=5.0, depth=10
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
}