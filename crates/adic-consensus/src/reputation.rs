use adic_types::PublicKey;
use chrono;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Reputation change event data
#[derive(Debug, Clone)]
pub struct ReputationChangeEvent {
    pub validator_pubkey: PublicKey,
    pub old_reputation: f64,
    pub new_reputation: f64,
    pub reason: String,
}

/// Callback for reputation change events
pub type ReputationEventCallback = Arc<dyn Fn(ReputationChangeEvent) + Send + Sync>;

/// ADIC-Rep: Non-transferable (Soul-Bound) reputation score
/// Per ADIC-DAG White Paper §5.3 and Yellow Paper §6.2
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationScore {
    pub value: f64,
    pub messages_finalized: u64,
    pub messages_pending: u64,
    pub last_update: i64,

    // SBT (Soul-Bound Token) properties - reputation is NOT transferable
    pub is_transferable: bool,       // Always false
    pub issue_timestamp: i64,        // When reputation was first issued
    pub issuer: Option<PublicKey>,   // Genesis or network authority
    pub bound_to_keypair: PublicKey, // Permanently bound to this key
}

impl Default for ReputationScore {
    fn default() -> Self {
        Self::new()
    }
}

impl ReputationScore {
    pub fn new() -> Self {
        // Default constructor for testing - creates unbound reputation
        Self {
            value: 1.0,
            messages_finalized: 0,
            messages_pending: 0,
            last_update: chrono::Utc::now().timestamp(),
            is_transferable: false,
            issue_timestamp: chrono::Utc::now().timestamp(),
            issuer: None,
            bound_to_keypair: PublicKey::from_bytes([0; 32]), // Placeholder for testing
        }
    }

    /// Create a new soul-bound reputation for a specific keypair
    pub fn new_sbt(bound_to: PublicKey, issuer: Option<PublicKey>) -> Self {
        Self {
            value: 1.0,
            messages_finalized: 0,
            messages_pending: 0,
            last_update: chrono::Utc::now().timestamp(),
            is_transferable: false, // Soul-bound: never transferable
            issue_timestamp: chrono::Utc::now().timestamp(),
            issuer,
            bound_to_keypair: bound_to,
        }
    }

    pub fn trust_score(&self) -> f64 {
        (1.0 + self.value).ln()
    }

    /// Check if reputation can be transferred (always returns error per SBT spec)
    /// ADIC-Rep is soul-bound and non-transferable per White Paper §5.3
    pub fn can_transfer(&self) -> Result<(), String> {
        Err(format!(
            "ADIC-Rep is non-transferable (soul-bound). Reputation is bound to keypair {} and cannot be transferred.",
            hex::encode(&self.bound_to_keypair.as_bytes()[..8])
        ))
    }
}

pub struct ReputationTracker {
    scores: Arc<RwLock<HashMap<PublicKey, ReputationScore>>>,
    decay_factor: f64,
    gamma: f64,
    /// Track message approvals for overlap detection
    approval_history: Arc<RwLock<HashMap<PublicKey, Vec<adic_types::MessageId>>>>,
    event_callback: Arc<RwLock<Option<ReputationEventCallback>>>,
}

impl ReputationTracker {
    pub fn new(gamma: f64) -> Self {
        Self {
            scores: Arc::new(RwLock::new(HashMap::new())),
            decay_factor: 0.99,
            gamma,
            approval_history: Arc::new(RwLock::new(HashMap::new())),
            event_callback: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the event callback for reputation changes
    pub async fn set_event_callback(&self, callback: ReputationEventCallback) {
        let mut cb = self.event_callback.write().await;
        *cb = Some(callback);
    }

    /// Emit a reputation change event if callback is set
    fn emit_reputation_event(&self, event: ReputationChangeEvent) {
        let callback_clone = self.event_callback.clone();
        tokio::spawn(async move {
            let callback_guard = callback_clone.read().await;
            if let Some(callback) = callback_guard.as_ref() {
                callback(event);
            }
        });
    }

    pub async fn get_reputation(&self, pubkey: &PublicKey) -> f64 {
        let scores = self.scores.read().await;
        let reputation = scores.get(pubkey).map(|s| s.value).unwrap_or(1.0);

        // Debug logging to track reputation lookups
        if let Some(score) = scores.get(pubkey) {
            tracing::debug!(
                pubkey_prefix = format!(
                    "{:x}",
                    pubkey.as_bytes()[..8]
                        .iter()
                        .fold(0u64, |acc, &b| acc << 8 | b as u64)
                ),
                reputation_value = score.value,
                "Reputation lookup - found"
            );
        } else {
            tracing::debug!(
                pubkey_prefix = format!(
                    "{:x}",
                    pubkey.as_bytes()[..8]
                        .iter()
                        .fold(0u64, |acc, &b| acc << 8 | b as u64)
                ),
                default_value = 1.0,
                "Reputation lookup - not found"
            );
        }

        reputation
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
        let old_finalized = score.messages_finalized;
        score.value = (self.gamma * old_rep + (1.0 - self.gamma) * good_score).min(10.0); // Cap at 10.0
        score.messages_finalized += 1;
        score.last_update = chrono::Utc::now().timestamp();

        tracing::info!(
            pubkey = %pubkey.to_hex(),
            reputation_before = old_rep,
            reputation_after = score.value,
            messages_finalized_before = old_finalized,
            messages_finalized_after = score.messages_finalized,
            diversity = diversity,
            depth = depth,
            good_score = good_score,
            "✅ Reputation increased (good behavior)"
        );

        // Emit reputation change event
        self.emit_reputation_event(ReputationChangeEvent {
            validator_pubkey: *pubkey,
            old_reputation: old_rep,
            new_reputation: score.value,
            reason: format!(
                "Good behavior: message finalized (diversity={:.2}, depth={})",
                diversity, depth
            ),
        });
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

        tracing::info!(
            pubkey = %pubkey.to_hex(),
            reputation_before = old_rep,
            reputation_after = score.value,
            penalty = penalty,
            "⚠️ Reputation decreased (bad behavior)"
        );

        // Emit reputation change event
        self.emit_reputation_event(ReputationChangeEvent {
            validator_pubkey: *pubkey,
            old_reputation: old_rep,
            new_reputation: score.value,
            reason: format!(
                "Bad behavior: message invalid/slashed (penalty={:.2})",
                penalty
            ),
        });
    }

    /// Comprehensive reputation update per whitepaper Appendix E
    /// Combines good score from finalized approvals and bad score from overlap
    pub async fn update_reputation(
        &self,
        pubkey: &PublicKey,
        finalized_count: usize,
        diversity: f64,
        depth: u32,
        active_peers: &[PublicKey],
    ) {
        // Calculate good score from finalized approvals
        let good = if finalized_count > 0 {
            finalized_count as f64 * (1.0 + diversity / (1.0 + depth as f64))
        } else {
            0.0
        };

        // Calculate overlap penalty (η parameter from paper)
        let eta = 0.5; // Overlap penalty weight
        let overlap_score = self.calculate_overlap_score(pubkey, active_peers).await;
        let bad = eta * overlap_score;

        // Update reputation: R_{t+1} = γR_t + (1−γ)(good − bad)
        let mut scores = self.scores.write().await;
        let score = scores.entry(*pubkey).or_insert_with(ReputationScore::new);

        let old_rep = score.value;
        let old_finalized = score.messages_finalized;
        let new_rep = self.gamma * old_rep + (1.0 - self.gamma) * (good - bad);

        // Apply bounds [0.1, 10.0]
        score.value = new_rep.clamp(0.1, 10.0);
        score.messages_finalized += finalized_count as u64;
        score.last_update = chrono::Utc::now().timestamp();

        tracing::info!(
            pubkey = %pubkey.to_hex(),
            reputation_before = old_rep,
            reputation_after = score.value,
            messages_finalized_before = old_finalized,
            messages_finalized_after = score.messages_finalized,
            finalized_count = finalized_count,
            diversity = diversity,
            depth = depth,
            overlap_score = overlap_score,
            good_score = good,
            bad_score = bad,
            "🔄 Reputation updated (comprehensive)"
        );

        // Emit reputation change event
        self.emit_reputation_event(ReputationChangeEvent {
            validator_pubkey: *pubkey,
            old_reputation: old_rep,
            new_reputation: score.value,
            reason: format!(
                "Comprehensive update: finalized={}, diversity={:.2}, depth={}, overlap={:.2}",
                finalized_count, diversity, depth, overlap_score
            ),
        });
    }

    /// Calculate overlap score for Sybil detection
    /// Per whitepaper: detect when multiple keys approve similar message sets
    pub async fn calculate_overlap_score(
        &self,
        pubkey: &PublicKey,
        other_keys: &[PublicKey],
    ) -> f64 {
        let history = self.approval_history.read().await;

        let my_approvals = match history.get(pubkey) {
            Some(approvals) => approvals,
            None => return 0.0,
        };

        if my_approvals.is_empty() {
            return 0.0;
        }

        let mut max_overlap: f64 = 0.0;

        for other_key in other_keys {
            if other_key == pubkey {
                continue;
            }

            if let Some(other_approvals) = history.get(other_key) {
                // Calculate Jaccard similarity
                let intersection: usize = my_approvals
                    .iter()
                    .filter(|msg| other_approvals.contains(msg))
                    .count();

                let union = my_approvals.len() + other_approvals.len() - intersection;

                if union > 0 {
                    let overlap = intersection as f64 / union as f64;
                    max_overlap = max_overlap.max(overlap);
                }
            }
        }

        // Return penalty factor based on overlap (0 = no overlap, 1 = complete overlap)
        max_overlap
    }

    /// Record approval for overlap detection
    pub async fn record_approval(&self, pubkey: &PublicKey, message_id: adic_types::MessageId) {
        let mut history = self.approval_history.write().await;
        let approvals = history.entry(*pubkey).or_insert_with(Vec::new);

        // Keep only recent approvals (last 100)
        let removed = if approvals.len() >= 100 {
            Some(approvals.remove(0))
        } else {
            None
        };

        let old_count = approvals.len();
        approvals.push(message_id);

        tracing::info!(
            pubkey = %pubkey.to_hex(),
            message_id = %message_id,
            approvals_before = old_count,
            approvals_after = approvals.len(),
            removed_message_id = ?removed,
            "📝 Approval recorded for overlap detection"
        );
    }

    /// Apply time-based decay to all reputations
    pub async fn apply_decay(&self) {
        let mut scores = self.scores.write().await;
        let now = chrono::Utc::now().timestamp();
        let mut decayed_count = 0;
        let mut total_decay = 0.0;

        for (_pubkey, score) in scores.iter_mut() {
            let time_diff = (now - score.last_update) as f64 / 86400.0; // days
            if time_diff > 0.0 {
                let old_value = score.value;
                score.value = (score.value * self.decay_factor.powf(time_diff)).max(0.1);
                score.last_update = now;

                let decay_amount = old_value - score.value;
                if decay_amount > 0.001 {
                    // Only log significant decays
                    decayed_count += 1;
                    total_decay += decay_amount;
                }
            }
        }

        if decayed_count > 0 {
            tracing::info!(
                accounts_decayed = decayed_count,
                total_decay_amount = total_decay,
                decay_factor = self.decay_factor,
                "⏰ Applied time-based reputation decay"
            );
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
        let old_value = score.value;
        score.value = value;
        score.last_update = chrono::Utc::now().timestamp();

        tracing::info!(
            pubkey = %pubkey.to_hex(),
            reputation_before = old_value,
            reputation_after = value,
            "🔧 Reputation manually set (testing)"
        );
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
            approval_history: Arc::clone(&self.approval_history),
            event_callback: Arc::clone(&self.event_callback),
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

    #[test]
    fn test_reputation_non_transferable() {
        let keypair = PublicKey::from_bytes([1; 32]);
        let issuer = Some(PublicKey::from_bytes([2; 32]));
        let score = ReputationScore::new_sbt(keypair, issuer);

        // Verify is_transferable is false
        assert!(!score.is_transferable);

        // Verify bound_to_keypair is set correctly
        assert_eq!(score.bound_to_keypair, keypair);

        // Verify issuer is set correctly
        assert_eq!(score.issuer, issuer);

        // Verify can_transfer always returns error
        let result = score.can_transfer();
        assert!(result.is_err());

        // Verify error message contains expected text
        let error_msg = result.unwrap_err();
        assert!(error_msg.contains("non-transferable"));
        assert!(error_msg.contains("soul-bound"));
    }
}
