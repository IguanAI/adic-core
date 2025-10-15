use crate::error::Result;
use crate::types::{ReputationChangeReason, ReputationUpdate, TaskOutcome, ValidationResult};
use adic_consensus::ReputationTracker;
use adic_types::PublicKey;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Reputation manager for PoUW with multiplicative-weight updates
///
/// Implements PoUW III §4.3:
/// - Accepted work: R' = R * (1 + α * quality_score)
/// - Rejected work: R' = R * (1 - β * severity)
/// - Per-epoch cap: ΔR_max per epoch
pub struct PoUWReputationManager {
    reputation_tracker: Arc<ReputationTracker>,
    config: ReputationConfig,
    epoch_updates: Arc<RwLock<HashMap<PublicKey, EpochReputationTracking>>>,
}

#[derive(Debug, Clone)]
pub struct ReputationConfig {
    /// Multiplicative bonus factor for accepted work (α)
    pub acceptance_multiplier: f64,
    /// Multiplicative penalty factor for rejected work (β)
    pub rejection_multiplier: f64,
    /// Maximum reputation change per epoch (ΔR_max)
    pub max_delta_per_epoch: f64,
    /// Minimum reputation floor (cannot go below this)
    pub min_reputation: f64,
    /// Speed bonus weight (faster execution)
    pub speed_weight: f64,
    /// Correctness bonus weight (validation acceptance)
    pub correctness_weight: f64,
    /// Challenge resistance weight (no successful disputes)
    pub challenge_resistance_weight: f64,
}

impl Default for ReputationConfig {
    fn default() -> Self {
        Self {
            acceptance_multiplier: 0.1,    // 10% max increase
            rejection_multiplier: 0.2,     // 20% max decrease
            max_delta_per_epoch: 100.0,    // Cap at ±100 per epoch
            min_reputation: 10.0,          // Cannot fall below 10
            speed_weight: 0.2,             // 20% weight
            correctness_weight: 0.5,       // 50% weight
            challenge_resistance_weight: 0.3, // 30% weight
        }
    }
}

/// Track reputation changes within an epoch
#[derive(Debug, Clone, Default)]
struct EpochReputationTracking {
    total_delta: f64,
    num_updates: u64,
    last_epoch: u64,
}

impl PoUWReputationManager {
    pub fn new(reputation_tracker: Arc<ReputationTracker>, config: ReputationConfig) -> Self {
        Self {
            reputation_tracker,
            config,
            epoch_updates: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Update reputation after task completion
    pub async fn update_reputation(
        &self,
        account: &PublicKey,
        outcome: TaskOutcome,
        quality_score: f64,
        severity: f64,
        current_epoch: u64,
    ) -> Result<ReputationUpdate> {
        let old_reputation = self.reputation_tracker.get_reputation(account).await;

        // Calculate new reputation based on outcome
        let (new_reputation, change_reason) = match outcome {
            TaskOutcome::Success => {
                let quality = quality_score.clamp(0.0, 1.0);
                let delta = old_reputation * self.config.acceptance_multiplier * quality;
                let capped_delta = self.apply_epoch_cap(account, delta, current_epoch).await?;
                (
                    old_reputation + capped_delta,
                    ReputationChangeReason::WorkAccepted { quality },
                )
            }
            TaskOutcome::Failure | TaskOutcome::PartialSuccess => {
                let sev = severity.clamp(0.0, 1.0);
                let delta = old_reputation * self.config.rejection_multiplier * sev;
                let capped_delta = self.apply_epoch_cap(account, -delta, current_epoch).await?;
                (
                    old_reputation + capped_delta,
                    ReputationChangeReason::WorkRejected { severity: sev },
                )
            }
            TaskOutcome::Slashed => {
                // Heavy penalty for slashing
                let delta = old_reputation * self.config.rejection_multiplier * 2.0;
                let capped_delta = self.apply_epoch_cap(account, -delta, current_epoch).await?;
                (
                    old_reputation + capped_delta,
                    ReputationChangeReason::FraudDetected,
                )
            }
            TaskOutcome::Expired => {
                // Moderate penalty for missed deadline
                let delta = old_reputation * self.config.rejection_multiplier * 0.5;
                let capped_delta = self.apply_epoch_cap(account, -delta, current_epoch).await?;
                (
                    old_reputation + capped_delta,
                    ReputationChangeReason::DeadlineMissed,
                )
            }
        };

        // Apply reputation floor
        let final_reputation = new_reputation.max(self.config.min_reputation);

        // Update in tracker
        self.reputation_tracker
            .set_reputation(account, final_reputation)
            .await;

        info!(
            account = hex::encode(&account.as_bytes()[..8]),
            old_rep = old_reputation,
            new_rep = final_reputation,
            delta = final_reputation - old_reputation,
            outcome = ?outcome,
            "📊 Reputation updated"
        );

        Ok(ReputationUpdate {
            account: *account,
            old_reputation,
            new_reputation: final_reputation,
            change_reason,
            quality_score,
        })
    }

    /// Update reputation after validation participation
    pub async fn update_validator_reputation(
        &self,
        validator: &PublicKey,
        validation_correct: bool,
        current_epoch: u64,
    ) -> Result<ReputationUpdate> {
        let old_reputation = self.reputation_tracker.get_reputation(validator).await;

        let (new_reputation, quality_score) = if validation_correct {
            // Small bonus for correct validation
            let delta = old_reputation * 0.02; // 2% bonus
            let capped_delta = self.apply_epoch_cap(validator, delta, current_epoch).await?;
            (old_reputation + capped_delta, 1.0)
        } else {
            // Penalty for incorrect validation
            let delta = old_reputation * 0.05; // 5% penalty
            let capped_delta = self.apply_epoch_cap(validator, -delta, current_epoch).await?;
            (old_reputation + capped_delta, 0.0)
        };

        let final_reputation = new_reputation.max(self.config.min_reputation);

        self.reputation_tracker
            .set_reputation(validator, final_reputation)
            .await;

        Ok(ReputationUpdate {
            account: *validator,
            old_reputation,
            new_reputation: final_reputation,
            change_reason: ReputationChangeReason::ValidationPerformed {
                correctness: validation_correct,
            },
            quality_score,
        })
    }

    /// Apply per-epoch cap to reputation delta
    async fn apply_epoch_cap(
        &self,
        account: &PublicKey,
        delta: f64,
        current_epoch: u64,
    ) -> Result<f64> {
        let mut epoch_tracking = self.epoch_updates.write().await;

        let tracking = epoch_tracking.entry(*account).or_default();

        // Reset if new epoch
        if tracking.last_epoch < current_epoch {
            tracking.total_delta = 0.0;
            tracking.num_updates = 0;
            tracking.last_epoch = current_epoch;
        }

        // Calculate remaining budget for this epoch
        let used = tracking.total_delta.abs();
        let remaining = self.config.max_delta_per_epoch - used;

        if remaining <= 0.0 {
            // Cap reached, no more changes this epoch
            return Ok(0.0);
        }

        // Apply cap
        let capped_delta = if delta.abs() > remaining {
            remaining * delta.signum()
        } else {
            delta
        };

        tracking.total_delta += capped_delta;
        tracking.num_updates += 1;

        Ok(capped_delta)
    }

    /// Calculate quality score from execution metrics
    pub fn calculate_quality_score(
        &self,
        execution_time_ms: u64,
        expected_time_ms: u64,
        validation_result: ValidationResult,
        disputed: bool,
    ) -> f64 {
        // Speed component: faster is better (up to a point)
        let speed_ratio = expected_time_ms as f64 / execution_time_ms.max(1) as f64;
        let speed_score = speed_ratio.min(2.0) / 2.0; // Cap at 2x speed = 1.0 score

        // Correctness component: based on validation
        let correctness_score = match validation_result {
            ValidationResult::Accepted => 1.0,
            ValidationResult::Rejected => 0.0,
            ValidationResult::Inconclusive => 0.5,
        };

        // Challenge resistance: no disputes is good
        let challenge_score = if disputed { 0.0 } else { 1.0 };

        // Weighted combination
        let quality = self.config.speed_weight * speed_score
            + self.config.correctness_weight * correctness_score
            + self.config.challenge_resistance_weight * challenge_score;

        quality.clamp(0.0, 1.0)
    }

    /// Calculate severity of failure
    pub fn calculate_severity(
        &self,
        failure_type: &str,
        impact_level: u8,
    ) -> f64 {
        let base_severity = match failure_type {
            "incorrect_result" => 0.8,
            "missed_deadline" => 0.4,
            "resource_abuse" => 0.6,
            "proof_invalid" => 0.7,
            _ => 0.5,
        };

        // Scale by impact level (1-10)
        let impact_multiplier = (impact_level.min(10) as f64) / 10.0;

        (base_severity * impact_multiplier).clamp(0.0, 1.0)
    }

    /// Get epoch statistics for an account
    pub async fn get_epoch_stats(&self, account: &PublicKey) -> Option<EpochReputationStats> {
        let tracking = self.epoch_updates.read().await;
        tracking.get(account).map(|t| EpochReputationStats {
            total_delta: t.total_delta,
            num_updates: t.num_updates,
            remaining_budget: self.config.max_delta_per_epoch - t.total_delta.abs(),
            epoch: t.last_epoch,
        })
    }

    /// Reset epoch tracking (called at epoch boundary)
    pub async fn reset_epoch_tracking(&self) {
        self.epoch_updates.write().await.clear();
        info!("🔄 Epoch reputation tracking reset");
    }
}

#[derive(Debug, Clone)]
pub struct EpochReputationStats {
    pub total_delta: f64,
    pub num_updates: u64,
    pub remaining_budget: f64,
    pub epoch: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_reputation_increase_on_success() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let manager = PoUWReputationManager::new(rep_tracker.clone(), ReputationConfig::default());

        let worker = PublicKey::from_bytes([1u8; 32]);

        // Set initial reputation
        rep_tracker.set_reputation(&worker, 100.0).await;

        // Update after successful task with high quality
        let update = manager
            .update_reputation(&worker, TaskOutcome::Success, 1.0, 0.0, 1)
            .await
            .unwrap();

        assert_eq!(update.old_reputation, 100.0);
        assert!(update.new_reputation > 100.0);
        assert!(update.new_reputation <= 110.0); // Max 10% increase
        matches!(update.change_reason, ReputationChangeReason::WorkAccepted { .. });
    }

    #[tokio::test]
    async fn test_reputation_decrease_on_failure() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let manager = PoUWReputationManager::new(rep_tracker.clone(), ReputationConfig::default());

        let worker = PublicKey::from_bytes([2u8; 32]);

        rep_tracker.set_reputation(&worker, 100.0).await;

        // Update after failed task with high severity
        let update = manager
            .update_reputation(&worker, TaskOutcome::Failure, 0.0, 1.0, 1)
            .await
            .unwrap();

        assert_eq!(update.old_reputation, 100.0);
        assert!(update.new_reputation < 100.0);
        assert!(update.new_reputation >= 80.0); // Max 20% decrease
        matches!(update.change_reason, ReputationChangeReason::WorkRejected { .. });
    }

    #[tokio::test]
    async fn test_epoch_cap_enforcement() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let config = ReputationConfig {
            max_delta_per_epoch: 20.0, // Low cap for testing
            ..Default::default()
        };
        let manager = PoUWReputationManager::new(rep_tracker.clone(), config);

        let worker = PublicKey::from_bytes([3u8; 32]);
        rep_tracker.set_reputation(&worker, 100.0).await;

        let epoch = 1;

        // First update: should succeed
        let update1 = manager
            .update_reputation(&worker, TaskOutcome::Success, 1.0, 0.0, epoch)
            .await
            .unwrap();

        assert!(update1.new_reputation > 100.0);

        // Second update in same epoch: should be capped
        let update2 = manager
            .update_reputation(&worker, TaskOutcome::Success, 1.0, 0.0, epoch)
            .await
            .unwrap();

        // Total change should not exceed cap
        let total_delta = update2.new_reputation - 100.0;
        assert!(total_delta <= 20.0);
    }

    #[tokio::test]
    async fn test_reputation_floor() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let manager = PoUWReputationManager::new(rep_tracker.clone(), ReputationConfig::default());

        let worker = PublicKey::from_bytes([4u8; 32]);

        // Start with low reputation
        rep_tracker.set_reputation(&worker, 15.0).await;

        // Apply heavy penalty (slashing)
        let update = manager
            .update_reputation(&worker, TaskOutcome::Slashed, 0.0, 1.0, 1)
            .await
            .unwrap();

        // Should not fall below floor
        assert!(update.new_reputation >= 10.0);
    }

    #[tokio::test]
    async fn test_quality_score_calculation() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let manager = PoUWReputationManager::new(rep_tracker, ReputationConfig::default());

        // Perfect execution: fast, accepted, no disputes
        let quality1 = manager.calculate_quality_score(
            1000,
            2000,
            ValidationResult::Accepted,
            false,
        );
        assert!(quality1 > 0.9);

        // Poor execution: slow, rejected, disputed
        let quality2 = manager.calculate_quality_score(
            5000,
            2000,
            ValidationResult::Rejected,
            true,
        );
        assert!(quality2 < 0.3);
    }

    #[tokio::test]
    async fn test_severity_calculation() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let manager = PoUWReputationManager::new(rep_tracker, ReputationConfig::default());

        // High severity: incorrect result with high impact
        let sev1 = manager.calculate_severity("incorrect_result", 10);
        assert!(sev1 > 0.7);

        // Low severity: missed deadline with low impact
        let sev2 = manager.calculate_severity("missed_deadline", 3);
        assert!(sev2 < 0.5);
    }

    #[tokio::test]
    async fn test_validator_reputation_update() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let manager = PoUWReputationManager::new(rep_tracker.clone(), ReputationConfig::default());

        let validator = PublicKey::from_bytes([5u8; 32]);
        rep_tracker.set_reputation(&validator, 100.0).await;

        // Correct validation
        let update1 = manager
            .update_validator_reputation(&validator, true, 1)
            .await
            .unwrap();

        assert!(update1.new_reputation > 100.0);

        // Incorrect validation
        let update2 = manager
            .update_validator_reputation(&validator, false, 2)
            .await
            .unwrap();

        assert!(update2.new_reputation < update1.new_reputation);
    }

    #[tokio::test]
    async fn test_epoch_stats() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let manager = PoUWReputationManager::new(rep_tracker.clone(), ReputationConfig::default());

        let worker = PublicKey::from_bytes([6u8; 32]);
        rep_tracker.set_reputation(&worker, 100.0).await;

        let epoch = 1;

        // Perform updates
        manager
            .update_reputation(&worker, TaskOutcome::Success, 0.8, 0.0, epoch)
            .await
            .unwrap();

        manager
            .update_reputation(&worker, TaskOutcome::Success, 0.6, 0.0, epoch)
            .await
            .unwrap();

        // Check stats
        let stats = manager.get_epoch_stats(&worker).await.unwrap();
        assert_eq!(stats.epoch, epoch);
        assert_eq!(stats.num_updates, 2);
        assert!(stats.total_delta > 0.0);
        assert!(stats.remaining_budget < 100.0);
    }
}
