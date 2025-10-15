use crate::error::{PoUWError, Result};
use crate::types::{
    Hash, RewardDistribution, RewardType, SlashingEvent, SlashingReason, TaskId,
    VestingSchedule,
};
use adic_app_common::escrow::EscrowManager;
use adic_app_common::LockId;
use adic_economics::types::{AccountAddress, AdicAmount};
use adic_types::PublicKey;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Manages reward distribution and collateral slashing for PoUW
pub struct RewardManager {
    escrow_manager: Arc<EscrowManager>,
    config: RewardConfig,
    reward_history: Arc<RwLock<HashMap<TaskId, Vec<RewardDistribution>>>>,
    slashing_history: Arc<RwLock<HashMap<TaskId, Vec<SlashingEvent>>>>,
}

#[derive(Debug, Clone)]
pub struct RewardConfig {
    /// Base reward percentage (rest goes to quality/speed bonuses)
    pub base_reward_percentage: f64,
    /// Maximum quality bonus (as percentage of base reward)
    pub max_quality_bonus: f64,
    /// Maximum speed bonus (as percentage of base reward)
    pub max_speed_bonus: f64,
    /// Validator reward (as percentage of task reward)
    pub validator_reward_percentage: f64,
    /// Whether to use vesting for large rewards
    pub enable_vesting: bool,
    /// Minimum reward amount for vesting
    pub vesting_threshold: AdicAmount,
    /// Vesting duration in epochs
    pub vesting_duration_epochs: u64,
}

impl Default for RewardConfig {
    fn default() -> Self {
        Self {
            base_reward_percentage: 0.7,         // 70% base
            max_quality_bonus: 0.2,              // 20% quality bonus
            max_speed_bonus: 0.1,                // 10% speed bonus
            validator_reward_percentage: 0.05,   // 5% to validators
            enable_vesting: true,
            vesting_threshold: AdicAmount::from_adic(1000.0),
            vesting_duration_epochs: 100,        // ~1 day if epochs are ~15min
        }
    }
}

impl RewardManager {
    pub fn new(escrow_manager: Arc<EscrowManager>, config: RewardConfig) -> Self {
        Self {
            escrow_manager,
            config,
            reward_history: Arc::new(RwLock::new(HashMap::new())),
            slashing_history: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Distribute rewards for completed task
    pub async fn distribute_rewards(
        &self,
        task_id: &TaskId,
        total_reward: AdicAmount,
        workers: &[(PublicKey, f64, f64)], // (worker, quality_score, speed_ratio)
        validators: &[PublicKey],
        current_epoch: u64,
    ) -> Result<Vec<RewardDistribution>> {
        let mut distributions = Vec::new();

        // Calculate validator pool
        let validator_pool_base = total_reward.to_base_units() as f64;
        let validator_total = (validator_pool_base * self.config.validator_reward_percentage) as u64;
        let worker_pool = (validator_pool_base * (1.0 - self.config.validator_reward_percentage)) as u64;

        // Distribute to workers
        let per_worker_base = worker_pool / workers.len().max(1) as u64;

        for (worker, quality_score, speed_ratio) in workers {
            let base_amount = (per_worker_base as f64 * self.config.base_reward_percentage) as u64;

            // Calculate bonuses
            let quality_bonus = (per_worker_base as f64
                * self.config.max_quality_bonus
                * quality_score.clamp(0.0, 1.0)) as u64;

            let speed_bonus = (per_worker_base as f64
                * self.config.max_speed_bonus
                * (speed_ratio.min(2.0) - 1.0).max(0.0)) as u64;

            // Total worker reward
            let total_worker_reward = AdicAmount::from_base_units(base_amount);
            let total_quality_bonus = AdicAmount::from_base_units(quality_bonus);
            let total_speed_bonus = AdicAmount::from_base_units(speed_bonus);

            // Base reward distribution
            distributions.push(RewardDistribution {
                recipient: *worker,
                amount: total_worker_reward,
                reward_type: RewardType::BaseReward,
                vesting_schedule: None,
            });

            // Quality bonus (with potential vesting)
            if quality_bonus > 0 {
                let vesting = self.create_vesting_schedule(
                    total_quality_bonus,
                    current_epoch,
                );

                distributions.push(RewardDistribution {
                    recipient: *worker,
                    amount: total_quality_bonus,
                    reward_type: RewardType::QualityBonus,
                    vesting_schedule: vesting,
                });
            }

            // Speed bonus
            if speed_bonus > 0 {
                distributions.push(RewardDistribution {
                    recipient: *worker,
                    amount: total_speed_bonus,
                    reward_type: RewardType::SpeedBonus,
                    vesting_schedule: None,
                });
            }

            info!(
                worker = hex::encode(&worker.as_bytes()[..8]),
                base = base_amount,
                quality_bonus = quality_bonus,
                speed_bonus = speed_bonus,
                "💰 Worker reward distributed"
            );
        }

        // Distribute to validators
        if !validators.is_empty() {
            let per_validator = validator_total / validators.len() as u64;

            for validator in validators {
                let validator_reward = AdicAmount::from_base_units(per_validator);

                distributions.push(RewardDistribution {
                    recipient: *validator,
                    amount: validator_reward,
                    reward_type: RewardType::ValidationReward,
                    vesting_schedule: None,
                });

                info!(
                    validator = hex::encode(&validator.as_bytes()[..8]),
                    amount = per_validator,
                    "🔍 Validator reward distributed"
                );
            }
        }

        // Record in history
        self.reward_history
            .write()
            .await
            .insert(*task_id, distributions.clone());

        Ok(distributions)
    }

    /// Slash collateral for failed or fraudulent work
    pub async fn slash_collateral(
        &self,
        task_id: &TaskId,
        slashed_account: &PublicKey,
        collateral_amount: AdicAmount,
        reason: SlashingReason,
        evidence_hash: Hash,
        beneficiary: Option<PublicKey>,
    ) -> Result<SlashingEvent> {
        // In production, this would:
        // 1. Release collateral from escrow
        // 2. Burn portion or send to treasury
        // 3. Optionally reward challenger (beneficiary)

        let event = SlashingEvent {
            slashed_account: *slashed_account,
            amount: collateral_amount,
            reason: reason.clone(),
            evidence_hash,
            beneficiary,
        };

        info!(
            account = hex::encode(&slashed_account.as_bytes()[..8]),
            amount = collateral_amount.to_base_units(),
            reason = ?reason,
            "⚠️ Collateral slashed"
        );

        // Record in history
        self.slashing_history
            .write()
            .await
            .entry(*task_id)
            .or_insert_with(Vec::new)
            .push(event.clone());

        Ok(event)
    }

    /// Release collateral back to worker after successful completion
    pub async fn release_collateral(
        &self,
        worker: &PublicKey,
        lock_id: &LockId,
        to: AccountAddress,
    ) -> Result<()> {
        self.escrow_manager
            .release(lock_id, to)
            .await
            .map_err(|e| PoUWError::EscrowError(e.to_string()))?;

        info!(
            worker = hex::encode(&worker.as_bytes()[..8]),
            lock_id = lock_id.as_str(),
            "✅ Collateral released"
        );

        Ok(())
    }

    /// Create vesting schedule if amount exceeds threshold
    fn create_vesting_schedule(
        &self,
        amount: AdicAmount,
        start_epoch: u64,
    ) -> Option<VestingSchedule> {
        if !self.config.enable_vesting {
            return None;
        }

        if amount.to_base_units() < self.config.vesting_threshold.to_base_units() {
            return None;
        }

        let duration = self.config.vesting_duration_epochs;
        let vested_per_epoch_base = amount.to_base_units() / duration;

        Some(VestingSchedule {
            total_amount: amount,
            vested_per_epoch: AdicAmount::from_base_units(vested_per_epoch_base),
            start_epoch,
            duration_epochs: duration,
        })
    }

    /// Calculate total rewards distributed for a task
    pub async fn get_task_rewards(&self, task_id: &TaskId) -> Vec<RewardDistribution> {
        self.reward_history
            .read()
            .await
            .get(task_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Get slashing events for a task
    pub async fn get_task_slashing(&self, task_id: &TaskId) -> Vec<SlashingEvent> {
        self.slashing_history
            .read()
            .await
            .get(task_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Calculate statistics
    pub async fn get_stats(&self) -> RewardStats {
        let rewards = self.reward_history.read().await;
        let slashing = self.slashing_history.read().await;

        let total_tasks = rewards.len() as u64;
        let total_slashed_tasks = slashing.len() as u64;

        let mut total_distributed_base = 0u64;
        let mut total_slashed_base = 0u64;

        for distributions in rewards.values() {
            for dist in distributions {
                total_distributed_base += dist.amount.to_base_units();
            }
        }

        for events in slashing.values() {
            for event in events {
                total_slashed_base += event.amount.to_base_units();
            }
        }

        RewardStats {
            total_tasks_rewarded: total_tasks,
            total_tasks_slashed: total_slashed_tasks,
            total_rewards_distributed: AdicAmount::from_base_units(total_distributed_base),
            total_collateral_slashed: AdicAmount::from_base_units(total_slashed_base),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RewardStats {
    pub total_tasks_rewarded: u64,
    pub total_tasks_slashed: u64,
    pub total_rewards_distributed: AdicAmount,
    pub total_collateral_slashed: AdicAmount,
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_economics::storage::MemoryStorage;
    use adic_economics::BalanceManager;

    #[tokio::test]
    async fn test_reward_distribution() {
        let storage = Arc::new(MemoryStorage::new());
        let balance_mgr = Arc::new(BalanceManager::new(storage));
        let escrow_mgr = Arc::new(EscrowManager::new(balance_mgr));

        let reward_mgr = RewardManager::new(escrow_mgr, RewardConfig::default());

        let task_id = [1u8; 32];
        let total_reward = AdicAmount::from_adic(100.0);

        let workers = vec![
            (PublicKey::from_bytes([1u8; 32]), 1.0, 1.5), // High quality, fast
            (PublicKey::from_bytes([2u8; 32]), 0.8, 1.0), // Good quality, normal speed
        ];

        let validators = vec![
            PublicKey::from_bytes([10u8; 32]),
            PublicKey::from_bytes([11u8; 32]),
        ];

        let distributions = reward_mgr
            .distribute_rewards(&task_id, total_reward, &workers, &validators, 10)
            .await
            .unwrap();

        // Should have rewards for: 2 workers (base + bonuses) + 2 validators
        assert!(distributions.len() >= 4);

        // Check validator rewards
        let validator_rewards: Vec<_> = distributions
            .iter()
            .filter(|d| matches!(d.reward_type, RewardType::ValidationReward))
            .collect();
        assert_eq!(validator_rewards.len(), 2);

        // Check worker rewards
        let worker_rewards: Vec<_> = distributions
            .iter()
            .filter(|d| matches!(d.reward_type, RewardType::BaseReward))
            .collect();
        assert_eq!(worker_rewards.len(), 2);
    }

    #[tokio::test]
    async fn test_quality_and_speed_bonuses() {
        let storage = Arc::new(MemoryStorage::new());
        let balance_mgr = Arc::new(BalanceManager::new(storage));
        let escrow_mgr = Arc::new(EscrowManager::new(balance_mgr));

        let reward_mgr = RewardManager::new(escrow_mgr, RewardConfig::default());

        let task_id = [2u8; 32];
        let total_reward = AdicAmount::from_adic(100.0);

        // Worker with perfect quality and 2x speed
        let workers = vec![(PublicKey::from_bytes([3u8; 32]), 1.0, 2.0)];

        let distributions = reward_mgr
            .distribute_rewards(&task_id, total_reward, &workers, &[], 10)
            .await
            .unwrap();

        // Should have base + quality bonus + speed bonus
        let worker_dists: Vec<_> = distributions
            .iter()
            .filter(|d| d.recipient == workers[0].0)
            .collect();

        assert!(worker_dists.len() >= 2); // At least base + quality/speed
    }

    #[tokio::test]
    async fn test_vesting_schedule_creation() {
        let storage = Arc::new(MemoryStorage::new());
        let balance_mgr = Arc::new(BalanceManager::new(storage));
        let escrow_mgr = Arc::new(EscrowManager::new(balance_mgr));

        let config = RewardConfig {
            vesting_threshold: AdicAmount::from_adic(50.0),
            ..Default::default()
        };

        let reward_mgr = RewardManager::new(escrow_mgr, config);

        // Large reward should have vesting
        let vesting1 = reward_mgr.create_vesting_schedule(AdicAmount::from_adic(100.0), 10);
        assert!(vesting1.is_some());

        // Small reward should not have vesting
        let vesting2 = reward_mgr.create_vesting_schedule(AdicAmount::from_adic(10.0), 10);
        assert!(vesting2.is_none());
    }

    #[tokio::test]
    async fn test_collateral_slashing() {
        let storage = Arc::new(MemoryStorage::new());
        let balance_mgr = Arc::new(BalanceManager::new(storage));
        let escrow_mgr = Arc::new(EscrowManager::new(balance_mgr));

        let reward_mgr = RewardManager::new(escrow_mgr, RewardConfig::default());

        let task_id = [3u8; 32];
        let worker = PublicKey::from_bytes([4u8; 32]);
        let collateral = AdicAmount::from_adic(50.0);
        let evidence = [5u8; 32];

        let event = reward_mgr
            .slash_collateral(
                &task_id,
                &worker,
                collateral,
                SlashingReason::IncorrectResult,
                evidence,
                None,
            )
            .await
            .unwrap();

        assert_eq!(event.slashed_account, worker);
        assert_eq!(event.amount, collateral);
        matches!(event.reason, SlashingReason::IncorrectResult);

        // Check history
        let slashing_events = reward_mgr.get_task_slashing(&task_id).await;
        assert_eq!(slashing_events.len(), 1);
    }

    #[tokio::test]
    async fn test_reward_stats() {
        let storage = Arc::new(MemoryStorage::new());
        let balance_mgr = Arc::new(BalanceManager::new(storage));
        let escrow_mgr = Arc::new(EscrowManager::new(balance_mgr));

        let reward_mgr = RewardManager::new(escrow_mgr, RewardConfig::default());

        // Distribute rewards for multiple tasks
        for i in 0..3 {
            let task_id = [i; 32];
            let workers = vec![(PublicKey::from_bytes([i; 32]), 0.9, 1.0)];

            reward_mgr
                .distribute_rewards(&task_id, AdicAmount::from_adic(100.0), &workers, &[], 10)
                .await
                .unwrap();
        }

        // Add slashing event
        reward_mgr
            .slash_collateral(
                &[99u8; 32],
                &PublicKey::from_bytes([99u8; 32]),
                AdicAmount::from_adic(50.0),
                SlashingReason::ProvenFraud,
                [0u8; 32],
                None,
            )
            .await
            .unwrap();

        let stats = reward_mgr.get_stats().await;
        assert_eq!(stats.total_tasks_rewarded, 3);
        assert_eq!(stats.total_tasks_slashed, 1);
        assert!(stats.total_rewards_distributed.to_base_units() > 0);
        assert!(stats.total_collateral_slashed.to_base_units() > 0);
    }
}
