use crate::types::*;
use crate::{PoUWError, Result};
use adic_app_common::escrow::{EscrowManager, EscrowType};
use adic_consensus::ReputationTracker;
use adic_economics::types::AdicAmount;
use adic_types::encoders::{AsnEncoder, FeatureEncoder, RegionCodeEncoder, StakeTierEncoder};
use adic_types::PublicKey;
use blake3;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Worker selection engine using deterministic hash-based lottery
pub struct WorkerSelector {
    reputation_tracker: Arc<ReputationTracker>,
    escrow_manager: Arc<EscrowManager>,
    worker_performance: Arc<RwLock<HashMap<PublicKey, WorkerPerformance>>>,
    config: WorkerSelectionConfig,

    // P-adic feature encoders for diversity checking
    region_encoder: Arc<RegionCodeEncoder>,
    asn_encoder: Arc<AsnEncoder>,
    stake_encoder: Arc<StakeTierEncoder>,

    /// Worker feature metadata (region, ASN, stake tier) - derived from worker identity
    worker_features: Arc<RwLock<HashMap<PublicKey, WorkerFeatures>>>,
}

#[derive(Debug, Clone)]
pub struct WorkerSelectionConfig {
    /// Minimum reputation to be eligible as worker
    pub min_worker_reputation: f64,
    /// Collateral multiplier (worker_collateral = task_reward * multiplier)
    pub collateral_multiplier: f64,
    /// Axis diversity requirement (workers should be in different p-adic balls)
    pub require_axis_diversity: bool,
    /// Maximum workers from same axis ball
    pub max_workers_per_axis_ball: usize,
    /// Performance weight in selection (0.0 = ignore, 1.0 = heavy weight)
    pub performance_weight: f64,
    /// VRF selection rounds (more rounds = better randomness)
    pub vrf_rounds: usize,
}

impl Default for WorkerSelectionConfig {
    fn default() -> Self {
        Self {
            min_worker_reputation: 500.0,
            collateral_multiplier: 0.5, // Worker posts 50% of task reward
            require_axis_diversity: true,
            max_workers_per_axis_ball: 2,
            performance_weight: 0.3,
            vrf_rounds: 3,
        }
    }
}

/// Worker performance tracking
#[derive(Debug, Clone)]
pub struct WorkerPerformance {
    pub total_tasks_completed: u64,
    pub tasks_failed: u64,
    pub tasks_slashed: u64,
    pub average_execution_time_ms: f64,
    pub average_quality_score: f64,
    pub last_task_epoch: u64,
}

impl Default for WorkerPerformance {
    fn default() -> Self {
        Self {
            total_tasks_completed: 0,
            tasks_failed: 0,
            tasks_slashed: 0,
            average_execution_time_ms: 0.0,
            average_quality_score: 1.0, // Start with perfect score
            last_task_epoch: 0,
        }
    }
}

impl WorkerPerformance {
    /// Calculate performance score [0.0, 1.0]
    pub fn calculate_score(&self) -> f64 {
        let total_attempted = self.total_tasks_completed + self.tasks_failed + self.tasks_slashed;

        if total_attempted == 0 {
            return 1.0; // New workers get benefit of doubt
        }

        let success_rate = self.total_tasks_completed as f64 / total_attempted as f64;

        let slash_penalty = if self.tasks_slashed > 0 {
            0.5 // Heavy penalty for slashing
        } else {
            1.0
        };

        (success_rate * self.average_quality_score * slash_penalty).max(0.0).min(1.0)
    }
}

/// Worker eligibility result
#[derive(Debug, Clone)]
pub struct EligibleWorker {
    pub worker: PublicKey,
    pub reputation: f64,
    pub performance_score: f64,
    pub vrf_value: [u8; 32],
    pub selection_score: f64, // Combined score for ranking
}

/// Worker feature metadata for p-adic diversity
#[derive(Debug, Clone)]
pub struct WorkerFeatures {
    pub region: String,
    pub asn: String,
    pub stake_tier: u64,
}

impl WorkerSelector {
    /// Create new worker selector
    pub fn new(
        reputation_tracker: Arc<ReputationTracker>,
        escrow_manager: Arc<EscrowManager>,
        config: WorkerSelectionConfig,
    ) -> Self {
        Self {
            reputation_tracker,
            escrow_manager,
            worker_performance: Arc::new(RwLock::new(HashMap::new())),
            config,

            // Initialize p-adic encoders (matching ADIC-DAG paper Appendix A)
            region_encoder: Arc::new(RegionCodeEncoder::new(1)),
            asn_encoder: Arc::new(AsnEncoder::new(2)),
            stake_encoder: Arc::new(StakeTierEncoder::new(3)),

            worker_features: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Select workers for a task using VRF lottery
    pub async fn select_workers(
        &self,
        task: &Task,
        candidate_workers: Vec<PublicKey>,
        current_epoch: u64,
    ) -> Result<Vec<WorkAssignment>> {
        let start = std::time::Instant::now();

        // Filter eligible workers
        let eligible = self
            .filter_eligible_workers(task, candidate_workers, current_epoch)
            .await?;

        if eligible.len() < task.worker_count as usize {
            return Err(PoUWError::Other(format!(
                "Insufficient eligible workers: need {}, found {}",
                task.worker_count,
                eligible.len()
            )));
        }

        // Use VRF to deterministically select workers
        let selected = self.vrf_select_workers(task, eligible, current_epoch).await?;

        // Escrow collateral from selected workers
        let assignments = self
            .escrow_worker_collateral(task, selected, current_epoch)
            .await?;

        info!(
            task_id = hex::encode(&task.task_id[..8]),
            workers_selected = assignments.len(),
            duration_ms = start.elapsed().as_millis() as u64,
            "🎲 Workers selected via VRF"
        );

        Ok(assignments)
    }

    /// Filter workers by eligibility criteria
    async fn filter_eligible_workers(
        &self,
        task: &Task,
        candidates: Vec<PublicKey>,
        current_epoch: u64,
    ) -> Result<Vec<EligibleWorker>> {
        let mut eligible = Vec::new();
        let performance_map = self.worker_performance.read().await;

        for worker in candidates {
            // Check reputation
            let reputation = self.reputation_tracker.get_reputation(&worker).await;
            if reputation < self.config.min_worker_reputation {
                continue;
            }

            if reputation < task.min_reputation {
                continue;
            }

            // Get performance score
            let performance = performance_map
                .get(&worker)
                .cloned()
                .unwrap_or_default();
            let performance_score = performance.calculate_score();

            // Generate VRF value for this worker
            let vrf_seed = self.generate_worker_seed(task, &worker, current_epoch);
            let vrf_value = blake3::hash(&vrf_seed).into();

            // Calculate combined selection score
            let selection_score = self.calculate_selection_score(
                reputation,
                performance_score,
                &vrf_value,
            );

            eligible.push(EligibleWorker {
                worker,
                reputation,
                performance_score,
                vrf_value,
                selection_score,
            });
        }

        Ok(eligible)
    }

    /// VRF-based worker selection with diversity
    async fn vrf_select_workers(
        &self,
        task: &Task,
        mut eligible: Vec<EligibleWorker>,
        _current_epoch: u64,
    ) -> Result<Vec<EligibleWorker>> {
        // Sort by selection score (descending)
        eligible.sort_by(|a, b| {
            b.selection_score
                .partial_cmp(&a.selection_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut selected = Vec::new();
        // Track usage per axis: axis_id -> (ball_id -> count)
        let mut used_axis_balls: HashMap<u32, HashMap<Vec<u8>, usize>> = HashMap::new();

        for candidate in eligible {
            if selected.len() >= task.worker_count as usize {
                break;
            }

            // Check axis diversity if required
            if self.config.require_axis_diversity {
                let worker_balls = self.get_worker_axis_balls(&candidate.worker).await;

                // Check if worker violates diversity constraints on any axis
                let mut violates_diversity = false;
                for (axis_id, ball_id) in &worker_balls {
                    let axis_usage = used_axis_balls.entry(*axis_id).or_insert_with(HashMap::new);
                    let count = axis_usage.get(ball_id).copied().unwrap_or(0);

                    if count >= self.config.max_workers_per_axis_ball {
                        violates_diversity = true;
                        break;
                    }
                }

                if violates_diversity {
                    continue; // Skip, too many workers from this ball on some axis
                }

                // Update usage counts for all axes
                for (axis_id, ball_id) in worker_balls {
                    let axis_usage = used_axis_balls.entry(axis_id).or_insert_with(HashMap::new);
                    *axis_usage.entry(ball_id).or_insert(0) += 1;
                }
            }

            selected.push(candidate);
        }

        if selected.len() < task.worker_count as usize {
            return Err(PoUWError::Other(format!(
                "Could not select enough diverse workers: need {}, selected {}",
                task.worker_count,
                selected.len()
            )));
        }

        Ok(selected)
    }

    /// Escrow collateral from selected workers
    async fn escrow_worker_collateral(
        &self,
        task: &Task,
        selected: Vec<EligibleWorker>,
        current_epoch: u64,
    ) -> Result<Vec<WorkAssignment>> {
        let mut assignments = Vec::new();

        // Calculate per-worker collateral (collateral_multiplier * task_reward / worker_count)
        let reward_base = task.reward.to_base_units() as f64;
        let total_collateral_base = (reward_base * self.config.collateral_multiplier) as u64;
        let per_worker_collateral_base = total_collateral_base / task.worker_count as u64;
        let per_worker_collateral = AdicAmount::from_base_units(per_worker_collateral_base);

        for (index, worker_info) in selected.into_iter().enumerate() {
            // Create escrow lock for worker collateral
            let escrow_type = EscrowType::PoUWDeposit {
                hook_id: adic_types::MessageId::from_bytes(task.task_id),
                worker: adic_economics::types::AccountAddress::from_public_key(&worker_info.worker),
            };

            let lock_id = self
                .escrow_manager
                .lock(escrow_type, per_worker_collateral, current_epoch)
                .await
                .map_err(|e| PoUWError::EscrowError(e.to_string()))?;

            // Generate assignment
            let assignment_id = self.generate_assignment_id(task, &worker_info.worker, index);

            let assignment = WorkAssignment {
                assignment_id,
                task_id: task.task_id,
                worker: worker_info.worker,
                assigned_epoch: current_epoch,
                vrf_proof: worker_info.vrf_value.to_vec(),
                worker_index: index as u8,
                collateral_lock_id: Some(lock_id.to_string()),
                assignment_timestamp: Utc::now(),
            };

            assignments.push(assignment);
        }

        Ok(assignments)
    }

    /// Generate deterministic seed for worker VRF
    fn generate_worker_seed(&self, task: &Task, worker: &PublicKey, epoch: u64) -> Vec<u8> {
        let mut seed = Vec::new();
        seed.extend_from_slice(&task.task_id);
        seed.extend_from_slice(worker.as_bytes());
        seed.extend_from_slice(&epoch.to_le_bytes());
        seed.extend_from_slice(b"ADIC-POUW-WORKER-SELECTION-v1");
        seed
    }

    /// Calculate combined selection score
    fn calculate_selection_score(
        &self,
        reputation: f64,
        performance_score: f64,
        vrf_value: &[u8; 32],
    ) -> f64 {
        // Convert VRF value to normalized score [0.0, 1.0]
        let vrf_score = u64::from_le_bytes([
            vrf_value[0], vrf_value[1], vrf_value[2], vrf_value[3],
            vrf_value[4], vrf_value[5], vrf_value[6], vrf_value[7],
        ]) as f64
            / u64::MAX as f64;

        // Normalize reputation (assume max 10000)
        let rep_score = (reputation / 10000.0).min(1.0);

        // Weighted combination
        let rep_weight = 0.4;
        let perf_weight = self.config.performance_weight;
        let vrf_weight = 1.0 - rep_weight - perf_weight;

        rep_score * rep_weight + performance_score * perf_weight + vrf_score * vrf_weight
    }

    /// Get worker's p-adic axis ball identifiers
    ///
    /// Returns a map of axis_id -> ball_id for this worker based on their features.
    /// Uses real p-adic encoders to compute ball membership per ADIC-DAG ultrametric model.
    async fn get_worker_axis_balls(&self, worker: &PublicKey) -> HashMap<u32, Vec<u8>> {
        const P: u32 = 3; // Base prime for p-adic space
        const PRECISION: usize = 5; // Precision for p-adic encoding
        const BALL_RADIUS: usize = 3; // Depth for ball ID (first 3 digits)

        // Get or derive worker features
        let features = self.get_worker_features(worker).await;

        let mut axis_balls = HashMap::new();

        // Axis 1: Region/ASN diversity
        let region_encoded = self.region_encoder.encode(
            features.region.as_bytes(),
            P,
            PRECISION,
        );
        axis_balls.insert(1, region_encoded.ball_id(BALL_RADIUS));

        // Axis 2: ASN (network topology)
        let asn_encoded = self.asn_encoder.encode(
            features.asn.as_bytes(),
            P,
            PRECISION,
        );
        axis_balls.insert(2, asn_encoded.ball_id(BALL_RADIUS));

        // Axis 3: Stake tier
        let stake_encoded = self.stake_encoder.encode(
            features.stake_tier.to_string().as_bytes(),
            P,
            PRECISION,
        );
        axis_balls.insert(3, stake_encoded.ball_id(BALL_RADIUS));

        axis_balls
    }

    /// Get or derive worker features from their public key
    ///
    /// In production, this would query a worker registry or metadata service.
    /// For now, we derive features deterministically from the worker's public key.
    async fn get_worker_features(&self, worker: &PublicKey) -> WorkerFeatures {
        let features_map = self.worker_features.read().await;

        if let Some(features) = features_map.get(worker) {
            return features.clone();
        }

        drop(features_map);

        // Derive features deterministically from worker public key
        let hash = blake3::hash(worker.as_bytes());
        let hash_bytes = hash.as_bytes();

        // Region: hash first byte maps to regions
        let region = match hash_bytes[0] % 6 {
            0 => "US",
            1 => "EU",
            2 => "ASIA",
            3 => "AF",
            4 => "SA",
            5 => "OC",
            _ => "UNKNOWN",
        }.to_string();

        // ASN: derive from hash bytes 1-4
        let asn_num = u32::from_be_bytes([
            hash_bytes[1],
            hash_bytes[2],
            hash_bytes[3],
            hash_bytes[4],
        ]) % 100_000; // ASNs typically < 100000
        let asn = format!("AS{}", asn_num);

        // Stake tier: derive from hash bytes 5-8 (simulating balance lookup)
        let stake_tier = u64::from_be_bytes([
            hash_bytes[5],
            hash_bytes[6],
            hash_bytes[7],
            hash_bytes[8],
            0, 0, 0, 0,
        ]) % 1_000_000; // Up to 1M ADIC tokens

        let features = WorkerFeatures {
            region,
            asn,
            stake_tier,
        };

        // Cache for future use
        let mut features_map = self.worker_features.write().await;
        features_map.insert(*worker, features.clone());

        features
    }

    /// Generate deterministic assignment ID
    fn generate_assignment_id(&self, task: &Task, worker: &PublicKey, index: usize) -> Hash {
        let mut data = Vec::new();
        data.extend_from_slice(&task.task_id);
        data.extend_from_slice(worker.as_bytes());
        data.extend_from_slice(&index.to_le_bytes());
        blake3::hash(&data).into()
    }

    /// Update worker performance after task completion
    pub async fn update_worker_performance(
        &self,
        worker: &PublicKey,
        task_outcome: TaskOutcome,
        execution_time_ms: u64,
        quality_score: f64,
        current_epoch: u64,
    ) -> Result<()> {
        let mut performance_map = self.worker_performance.write().await;
        let mut perf = performance_map
            .get(worker)
            .cloned()
            .unwrap_or_default();

        match task_outcome {
            TaskOutcome::Success => {
                // Update average execution time (exponential moving average)
                if perf.total_tasks_completed == 0 {
                    perf.average_execution_time_ms = execution_time_ms as f64;
                    perf.average_quality_score = quality_score;
                } else {
                    perf.average_execution_time_ms =
                        0.7 * perf.average_execution_time_ms + 0.3 * execution_time_ms as f64;
                    perf.average_quality_score =
                        0.7 * perf.average_quality_score + 0.3 * quality_score;
                }

                perf.total_tasks_completed += 1;
            }
            TaskOutcome::Failure => {
                perf.tasks_failed += 1;
            }
            TaskOutcome::Slashed => {
                perf.tasks_slashed += 1;
            }
            _ => {}
        }

        perf.last_task_epoch = current_epoch;
        performance_map.insert(*worker, perf);

        Ok(())
    }

    /// Get worker performance
    pub async fn get_worker_performance(&self, worker: &PublicKey) -> WorkerPerformance {
        let performance_map = self.worker_performance.read().await;
        performance_map
            .get(worker)
            .cloned()
            .unwrap_or_default()
    }

    /// Get all workers with performance above threshold
    pub async fn get_high_performance_workers(&self, min_score: f64) -> Vec<PublicKey> {
        let performance_map = self.worker_performance.read().await;
        performance_map
            .iter()
            .filter(|(_, perf)| perf.calculate_score() >= min_score)
            .map(|(worker, _)| *worker)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_economics::storage::MemoryStorage;
    use adic_economics::BalanceManager;

    fn create_test_task(task_id: TaskId, worker_count: u8) -> Task {
        Task {
            task_id,
            sponsor: PublicKey::from_bytes([99; 32]),
            task_type: TaskType::Compute {
                computation_type: ComputationType::HashVerification,
                resource_requirements: ResourceRequirements::default(),
            },
            input_cid: "QmTest".to_string(),
            expected_output_schema: None,
            reward: AdicAmount::from_adic(100.0),
            collateral_requirement: AdicAmount::from_adic(50.0),
            deadline_epoch: 100,
            min_reputation: 500.0,
            worker_count,
            created_at: Utc::now(),
            status: TaskStatus::Finalized,
            finality_status: FinalityStatus::F1Complete,
        }
    }

    #[tokio::test]
    async fn test_worker_eligibility_filtering() {
        let storage = Arc::new(MemoryStorage::new());
        let balance_mgr = Arc::new(BalanceManager::new(storage));
        let escrow_mgr = Arc::new(EscrowManager::new(balance_mgr.clone()));
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));

        let selector = WorkerSelector::new(
            rep_tracker.clone(),
            escrow_mgr,
            WorkerSelectionConfig::default(),
        );

        // Create candidate workers with different reputations
        let worker1 = PublicKey::from_bytes([1; 32]);
        let worker2 = PublicKey::from_bytes([2; 32]);
        let worker3 = PublicKey::from_bytes([3; 32]);

        rep_tracker.set_reputation(&worker1, 1000.0).await; // Eligible
        rep_tracker.set_reputation(&worker2, 300.0).await;  // Below minimum
        rep_tracker.set_reputation(&worker3, 800.0).await;  // Eligible

        let task = create_test_task([1u8; 32], 2);
        let candidates = vec![worker1, worker2, worker3];

        let eligible = selector
            .filter_eligible_workers(&task, candidates, 10)
            .await
            .unwrap();

        // Only worker1 and worker3 should be eligible (worker2 below min reputation)
        assert_eq!(eligible.len(), 2);
        assert!(eligible.iter().any(|e| e.worker == worker1));
        assert!(eligible.iter().any(|e| e.worker == worker3));
        assert!(!eligible.iter().any(|e| e.worker == worker2));
    }

    #[tokio::test]
    async fn test_worker_performance_tracking() {
        let storage = Arc::new(MemoryStorage::new());
        let balance_mgr = Arc::new(BalanceManager::new(storage));
        let escrow_mgr = Arc::new(EscrowManager::new(balance_mgr.clone()));
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));

        let selector = WorkerSelector::new(
            rep_tracker,
            escrow_mgr,
            WorkerSelectionConfig::default(),
        );

        let worker = PublicKey::from_bytes([10; 32]);

        // Record successful task
        selector
            .update_worker_performance(&worker, TaskOutcome::Success, 1000, 0.95, 10)
            .await
            .unwrap();

        let perf = selector.get_worker_performance(&worker).await;
        assert_eq!(perf.total_tasks_completed, 1);
        assert_eq!(perf.tasks_failed, 0);
        assert_eq!(perf.average_execution_time_ms, 1000.0);
        assert!((perf.average_quality_score - 0.95).abs() < 0.01);

        // Record failed task
        selector
            .update_worker_performance(&worker, TaskOutcome::Failure, 0, 0.0, 11)
            .await
            .unwrap();

        let perf = selector.get_worker_performance(&worker).await;
        assert_eq!(perf.total_tasks_completed, 1);
        assert_eq!(perf.tasks_failed, 1);

        // Performance score should be reduced due to failure
        let score = perf.calculate_score();
        assert!(score < 1.0);
        assert!(score > 0.0);
    }

    #[tokio::test]
    async fn test_performance_score_calculation() {
        let mut perf = WorkerPerformance::default();

        // New worker gets benefit of doubt
        assert_eq!(perf.calculate_score(), 1.0);

        // Perfect track record
        perf.total_tasks_completed = 10;
        perf.average_quality_score = 1.0;
        assert_eq!(perf.calculate_score(), 1.0);

        // Some failures
        perf.tasks_failed = 2;
        let score = perf.calculate_score();
        assert!(score < 1.0);
        assert!(score > 0.5);

        // Slashing heavily penalizes
        perf.tasks_slashed = 1;
        let score_with_slash = perf.calculate_score();
        assert!(score_with_slash < score);
        assert!(score_with_slash < 0.5);
    }
}
