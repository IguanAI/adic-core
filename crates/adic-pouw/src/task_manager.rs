use crate::types::*;
use crate::{PoUWError, Result};
use adic_app_common::escrow::{EscrowManager, EscrowType};
use adic_consensus::ReputationTracker;
use adic_economics::types::AdicAmount;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Task manager handles task lifecycle and escrow integration
pub struct TaskManager {
    escrow_manager: Arc<EscrowManager>,
    reputation_tracker: Arc<ReputationTracker>,
    tasks: Arc<RwLock<HashMap<TaskId, Task>>>,
    work_assignments: Arc<RwLock<HashMap<TaskId, Vec<WorkAssignment>>>>,
    work_results: Arc<RwLock<HashMap<TaskId, Vec<WorkResult>>>>,
    validation_reports: Arc<RwLock<HashMap<TaskId, ValidationReport>>>,
    task_receipts: Arc<RwLock<HashMap<TaskId, TaskReceipt>>>,
    config: TaskManagerConfig,
}

#[derive(Debug, Clone)]
pub struct TaskManagerConfig {
    /// Minimum deposit for anti-spam (in addition to reward)
    pub min_deposit: AdicAmount,
    /// Maximum task duration in epochs
    pub max_task_duration_epochs: u64,
    /// Minimum reputation to submit tasks
    pub min_sponsor_reputation: f64,
    /// Challenge window duration in epochs
    pub challenge_window_epochs: u64,
    /// Maximum workers per task
    pub max_workers_per_task: u8,
}

impl Default for TaskManagerConfig {
    fn default() -> Self {
        Self {
            min_deposit: AdicAmount::from_adic(1.0),
            max_task_duration_epochs: 100,
            min_sponsor_reputation: 100.0,
            challenge_window_epochs: 5,
            max_workers_per_task: 10,
        }
    }
}

impl TaskManager {
    /// Create new task manager
    pub fn new(
        escrow_manager: Arc<EscrowManager>,
        reputation_tracker: Arc<ReputationTracker>,
        config: TaskManagerConfig,
    ) -> Self {
        Self {
            escrow_manager,
            reputation_tracker,
            tasks: Arc::new(RwLock::new(HashMap::new())),
            work_assignments: Arc::new(RwLock::new(HashMap::new())),
            work_results: Arc::new(RwLock::new(HashMap::new())),
            validation_reports: Arc::new(RwLock::new(HashMap::new())),
            task_receipts: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    /// Submit a new task with escrow
    pub async fn submit_task(&self, mut task: Task, current_epoch: u64) -> Result<TaskId> {
        let start = std::time::Instant::now();

        // Validate sponsor reputation
        let sponsor_rep = self.reputation_tracker.get_reputation(&task.sponsor).await;
        if sponsor_rep < self.config.min_sponsor_reputation {
            return Err(PoUWError::InsufficientReputation {
                required: self.config.min_sponsor_reputation,
                actual: sponsor_rep,
            });
        }

        // Validate task parameters
        self.validate_task(&task, current_epoch)?;

        // Calculate total escrow amount (reward + deposit)
        let total_escrow = task
            .reward
            .checked_add(self.config.min_deposit)
            .ok_or_else(|| PoUWError::Other("Escrow amount overflow".to_string()))?;

        // Create escrow lock
        let escrow_type = EscrowType::Custom {
            lock_id: format!("pouw_task_{}", hex::encode(&task.task_id[..8])),
            owner: adic_economics::types::AccountAddress::from_public_key(&task.sponsor),
        };

        let _lock_id = self
            .escrow_manager
            .lock(escrow_type, total_escrow, current_epoch)
            .await
            .map_err(|e| PoUWError::EscrowError(e.to_string()))?;

        // Update task status
        task.status = TaskStatus::Submitted;
        task.finality_status = FinalityStatus::Pending;

        // Store task
        let task_id = task.task_id;
        let mut tasks = self.tasks.write().await;
        tasks.insert(task_id, task.clone());

        info!(
            task_id = hex::encode(&task_id[..8]),
            sponsor = hex::encode(task.sponsor.as_bytes()),
            reward_adic = task.reward.to_adic(),
            deadline_epoch = task.deadline_epoch,
            worker_count = task.worker_count,
            duration_ms = start.elapsed().as_millis() as u64,
            "📋 Task submitted"
        );

        Ok(task_id)
    }

    /// Update task finality status (called by node after message finalization)
    pub async fn mark_task_finalized(&self, task_id: &TaskId, current_epoch: u64) -> Result<()> {
        let mut tasks = self.tasks.write().await;
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| PoUWError::TaskNotFound(hex::encode(task_id)))?;

        if task.status != TaskStatus::Submitted {
            return Err(PoUWError::InvalidTaskState {
                expected: "Submitted".to_string(),
                actual: format!("{:?}", task.status),
            });
        }

        task.status = TaskStatus::Finalized;
        task.finality_status = FinalityStatus::F1Complete;

        info!(
            task_id = hex::encode(&task_id[..8]),
            epoch = current_epoch,
            "✅ Task finalized, ready for worker selection"
        );

        Ok(())
    }

    /// Record worker assignments (called after VRF selection)
    pub async fn record_worker_assignments(
        &self,
        task_id: &TaskId,
        assignments: Vec<WorkAssignment>,
        current_epoch: u64,
    ) -> Result<()> {
        let mut tasks = self.tasks.write().await;
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| PoUWError::TaskNotFound(hex::encode(task_id)))?;

        if task.status != TaskStatus::Finalized {
            return Err(PoUWError::InvalidTaskState {
                expected: "Finalized".to_string(),
                actual: format!("{:?}", task.status),
            });
        }

        // Verify worker count matches task requirements
        if assignments.len() != task.worker_count as usize {
            return Err(PoUWError::Other(format!(
                "Worker count mismatch: expected {}, got {}",
                task.worker_count,
                assignments.len()
            )));
        }

        task.status = TaskStatus::Assigned;

        // Store assignments
        let mut work_assignments = self.work_assignments.write().await;
        work_assignments.insert(*task_id, assignments.clone());

        info!(
            task_id = hex::encode(&task_id[..8]),
            workers_assigned = assignments.len(),
            epoch = current_epoch,
            "👷 Workers assigned to task"
        );

        Ok(())
    }

    /// Submit work result from a worker
    pub async fn submit_work_result(
        &self,
        result: WorkResult,
        current_epoch: u64,
    ) -> Result<Hash> {
        let start = std::time::Instant::now();
        let task_id = result.task_id;

        // Verify task exists and is in correct state
        let mut tasks = self.tasks.write().await;
        let task = tasks
            .get_mut(&task_id)
            .ok_or_else(|| PoUWError::TaskNotFound(hex::encode(&task_id)))?;

        // Check task hasn't expired
        if current_epoch > task.deadline_epoch {
            return Err(PoUWError::DeadlineExceeded {
                deadline: task.deadline_epoch,
                current: current_epoch,
            });
        }

        // Verify worker was assigned to this task
        let work_assignments = self.work_assignments.read().await;
        let assignments = work_assignments
            .get(&task_id)
            .ok_or_else(|| PoUWError::Other("Task not assigned".to_string()))?;

        let assigned = assignments.iter().any(|a| a.worker == result.worker);
        if !assigned {
            return Err(PoUWError::WorkerNotEligible {
                reason: "Worker not assigned to this task".to_string(),
            });
        }

        // Check if worker already submitted result
        let work_results = self.work_results.read().await;
        if let Some(results) = work_results.get(&task_id) {
            if results.iter().any(|r| r.worker == result.worker) {
                return Err(PoUWError::ResultAlreadySubmitted {
                    task_id: hex::encode(&task_id),
                });
            }
        }
        drop(work_results);

        // Update task status
        if task.status == TaskStatus::Assigned {
            task.status = TaskStatus::InProgress;
        }

        // Store result
        let result_id = result.result_id;
        let mut work_results = self.work_results.write().await;
        work_results
            .entry(task_id)
            .or_insert_with(Vec::new)
            .push(result.clone());

        // Check if all workers submitted
        let results_count = work_results.get(&task_id).map(|r| r.len()).unwrap_or(0);
        drop(work_results);
        drop(work_assignments);

        if results_count == task.worker_count as usize {
            task.status = TaskStatus::ResultSubmitted;
            info!(
                task_id = hex::encode(&task_id[..8]),
                "✅ All workers submitted results, ready for validation"
            );
        }

        drop(tasks);

        info!(
            task_id = hex::encode(&task_id[..8]),
            worker = hex::encode(&result.worker.as_bytes()[..8]),
            output_cid = &result.output_cid[..16],
            execution_time_ms = result.execution_metrics.cpu_time_ms,
            duration_ms = start.elapsed().as_millis() as u64,
            "💼 Work result submitted"
        );

        Ok(result_id)
    }

    /// Record validation report from quorum
    pub async fn record_validation_report(
        &self,
        report: ValidationReport,
        current_epoch: u64,
    ) -> Result<()> {
        let task_id = report.task_id;

        let mut tasks = self.tasks.write().await;
        let task = tasks
            .get_mut(&task_id)
            .ok_or_else(|| PoUWError::TaskNotFound(hex::encode(&task_id)))?;

        if task.status != TaskStatus::ResultSubmitted {
            return Err(PoUWError::InvalidTaskState {
                expected: "ResultSubmitted".to_string(),
                actual: format!("{:?}", task.status),
            });
        }

        task.status = TaskStatus::Validating;

        // Store validation report
        let mut validation_reports = self.validation_reports.write().await;
        validation_reports.insert(task_id, report.clone());

        info!(
            task_id = hex::encode(&task_id[..8]),
            result = ?report.validation_result,
            votes_for = report.votes_for,
            votes_against = report.votes_against,
            validators = report.validators.len(),
            epoch = current_epoch,
            "🔍 Validation report recorded"
        );

        Ok(())
    }

    /// Begin challenge period for a validated task
    pub async fn begin_challenge_period(
        &self,
        task_id: &TaskId,
        current_epoch: u64,
    ) -> Result<u64> {
        let mut tasks = self.tasks.write().await;
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| PoUWError::TaskNotFound(hex::encode(task_id)))?;

        if task.status != TaskStatus::Validating {
            return Err(PoUWError::InvalidTaskState {
                expected: "Validating".to_string(),
                actual: format!("{:?}", task.status),
            });
        }

        task.status = TaskStatus::Challenging;
        let challenge_end_epoch = current_epoch + self.config.challenge_window_epochs;

        info!(
            task_id = hex::encode(&task_id[..8]),
            challenge_end_epoch,
            window_epochs = self.config.challenge_window_epochs,
            "⏰ Challenge period started"
        );

        Ok(challenge_end_epoch)
    }

    /// Get task by ID
    pub async fn get_task(&self, task_id: &TaskId) -> Option<Task> {
        let tasks = self.tasks.read().await;
        tasks.get(task_id).cloned()
    }

    /// Get work assignments for a task
    pub async fn get_work_assignments(&self, task_id: &TaskId) -> Option<Vec<WorkAssignment>> {
        let assignments = self.work_assignments.read().await;
        assignments.get(task_id).cloned()
    }

    /// Get work results for a task
    pub async fn get_work_results(&self, task_id: &TaskId) -> Option<Vec<WorkResult>> {
        let results = self.work_results.read().await;
        results.get(task_id).cloned()
    }

    /// Get validation report for a task
    pub async fn get_validation_report(&self, task_id: &TaskId) -> Option<ValidationReport> {
        let reports = self.validation_reports.read().await;
        reports.get(task_id).cloned()
    }

    /// Get task receipt
    pub async fn get_task_receipt(&self, task_id: &TaskId) -> Option<TaskReceipt> {
        let receipts = self.task_receipts.read().await;
        receipts.get(task_id).cloned()
    }

    /// Get all tasks with a specific status
    pub async fn get_tasks_by_status(&self, status: TaskStatus) -> Vec<Task> {
        let tasks = self.tasks.read().await;
        tasks
            .values()
            .filter(|t| t.status == status)
            .cloned()
            .collect()
    }

    /// Get tasks that need worker selection (finalized but not assigned)
    pub async fn get_tasks_pending_assignment(&self) -> Vec<Task> {
        self.get_tasks_by_status(TaskStatus::Finalized).await
    }

    /// Get tasks ready for validation (all results submitted)
    pub async fn get_tasks_ready_for_validation(&self) -> Vec<Task> {
        self.get_tasks_by_status(TaskStatus::ResultSubmitted).await
    }

    /// Validate task parameters
    fn validate_task(&self, task: &Task, current_epoch: u64) -> Result<()> {
        // Check deadline is in the future
        if task.deadline_epoch <= current_epoch {
            return Err(PoUWError::DeadlineExceeded {
                deadline: task.deadline_epoch,
                current: current_epoch,
            });
        }

        // Check deadline is not too far in the future
        let duration = task.deadline_epoch - current_epoch;
        if duration > self.config.max_task_duration_epochs {
            return Err(PoUWError::InvalidTaskInput(format!(
                "Task duration {} epochs exceeds maximum {}",
                duration, self.config.max_task_duration_epochs
            )));
        }

        // Check worker count
        if task.worker_count == 0 {
            return Err(PoUWError::InvalidTaskInput(
                "Worker count must be at least 1".to_string(),
            ));
        }

        if task.worker_count > self.config.max_workers_per_task {
            return Err(PoUWError::InvalidTaskInput(format!(
                "Worker count {} exceeds maximum {}",
                task.worker_count, self.config.max_workers_per_task
            )));
        }

        // Check reward is positive
        if task.reward <= AdicAmount::from_adic(0.0) {
            return Err(PoUWError::InvalidTaskInput(
                "Task reward must be positive".to_string(),
            ));
        }

        // Check collateral requirement
        if task.collateral_requirement < AdicAmount::from_adic(0.0) {
            return Err(PoUWError::InvalidTaskInput(
                "Collateral requirement cannot be negative".to_string(),
            ));
        }

        // Check input CID is not empty
        if task.input_cid.is_empty() {
            return Err(PoUWError::InvalidTaskInput(
                "Input CID cannot be empty".to_string(),
            ));
        }

        Ok(())
    }

    /// Get task statistics
    pub async fn get_task_stats(&self) -> TaskStats {
        let tasks = self.tasks.read().await;
        let receipts = self.task_receipts.read().await;

        let mut stats = TaskStats::default();
        stats.total_tasks = tasks.len() as u64;

        for task in tasks.values() {
            match task.status {
                TaskStatus::Submitted
                | TaskStatus::Finalized
                | TaskStatus::Assigned
                | TaskStatus::InProgress
                | TaskStatus::ResultSubmitted
                | TaskStatus::Validating
                | TaskStatus::Challenging => {
                    stats.active_tasks += 1;
                }
                TaskStatus::Completed => {
                    stats.completed_tasks += 1;
                }
                TaskStatus::Failed | TaskStatus::Slashed | TaskStatus::Expired => {
                    stats.failed_tasks += 1;
                }
                _ => {}
            }
        }

        // Aggregate from receipts
        for receipt in receipts.values() {
            for reward in &receipt.rewards_distributed {
                stats.total_rewards_distributed = stats
                    .total_rewards_distributed
                    .checked_add(reward.amount)
                    .unwrap_or(stats.total_rewards_distributed);
            }

            for slash in &receipt.slashing_events {
                stats.total_slashed = stats
                    .total_slashed
                    .checked_add(slash.amount)
                    .unwrap_or(stats.total_slashed);
            }
        }

        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_economics::storage::MemoryStorage;
    use adic_economics::BalanceManager;
    use adic_types::PublicKey;
    use chrono::Utc;

    #[tokio::test]
    async fn test_task_submission() {
        let storage = Arc::new(MemoryStorage::new());
        let balance_mgr = Arc::new(BalanceManager::new(storage));
        let escrow_mgr = Arc::new(EscrowManager::new(balance_mgr.clone()));
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));

        let sponsor = PublicKey::from_bytes([1; 32]);
        let sponsor_addr = adic_economics::types::AccountAddress::from_public_key(&sponsor);

        // Fund sponsor
        balance_mgr
            .credit(sponsor_addr, AdicAmount::from_adic(1000.0))
            .await
            .unwrap();

        // Set sponsor reputation
        rep_tracker.set_reputation(&sponsor, 500.0).await;

        let task_manager = TaskManager::new(escrow_mgr, rep_tracker, TaskManagerConfig::default());

        let task = Task {
            task_id: [1u8; 32],
            sponsor,
            task_type: TaskType::Compute {
                computation_type: ComputationType::HashVerification,
                resource_requirements: ResourceRequirements::default(),
            },
            input_cid: "QmTest123".to_string(),
            expected_output_schema: Some("hash:sha256".to_string()),
            reward: AdicAmount::from_adic(100.0),
            collateral_requirement: AdicAmount::from_adic(50.0),
            deadline_epoch: 100,
            min_reputation: 100.0,
            worker_count: 3,
            created_at: Utc::now(),
            status: TaskStatus::Submitted,
            finality_status: FinalityStatus::Pending,
        };

        let task_id = task_manager.submit_task(task, 10).await.unwrap();
        assert_eq!(task_id, [1u8; 32]);

        let retrieved_task = task_manager.get_task(&task_id).await.unwrap();
        assert_eq!(retrieved_task.status, TaskStatus::Submitted);
    }

    #[tokio::test]
    async fn test_task_lifecycle_transitions() {
        let storage = Arc::new(MemoryStorage::new());
        let balance_mgr = Arc::new(BalanceManager::new(storage));
        let escrow_mgr = Arc::new(EscrowManager::new(balance_mgr.clone()));
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));

        let sponsor = PublicKey::from_bytes([2; 32]);
        let sponsor_addr = adic_economics::types::AccountAddress::from_public_key(&sponsor);

        balance_mgr
            .credit(sponsor_addr, AdicAmount::from_adic(1000.0))
            .await
            .unwrap();
        rep_tracker.set_reputation(&sponsor, 500.0).await;

        let task_manager = TaskManager::new(escrow_mgr, rep_tracker, TaskManagerConfig::default());

        let task = Task {
            task_id: [2u8; 32],
            sponsor,
            task_type: TaskType::Compute {
                computation_type: ComputationType::DataProcessing,
                resource_requirements: ResourceRequirements::default(),
            },
            input_cid: "QmTest456".to_string(),
            expected_output_schema: None,
            reward: AdicAmount::from_adic(50.0),
            collateral_requirement: AdicAmount::from_adic(25.0),
            deadline_epoch: 50,
            min_reputation: 100.0,
            worker_count: 1,
            created_at: Utc::now(),
            status: TaskStatus::Submitted,
            finality_status: FinalityStatus::Pending,
        };

        let task_id = task_manager.submit_task(task, 10).await.unwrap();

        // Test state transition: Submitted → Finalized
        task_manager.mark_task_finalized(&task_id, 11).await.unwrap();
        let task = task_manager.get_task(&task_id).await.unwrap();
        assert_eq!(task.status, TaskStatus::Finalized);

        // Test state transition: Finalized → Assigned
        let assignment = WorkAssignment {
            assignment_id: [3u8; 32],
            task_id,
            worker: PublicKey::from_bytes([10; 32]),
            assigned_epoch: 12,
            vrf_proof: vec![1, 2, 3],
            worker_index: 0,
            collateral_lock_id: None,
            assignment_timestamp: Utc::now(),
        };

        task_manager
            .record_worker_assignments(&task_id, vec![assignment], 12)
            .await
            .unwrap();

        let task = task_manager.get_task(&task_id).await.unwrap();
        assert_eq!(task.status, TaskStatus::Assigned);
    }

    #[tokio::test]
    async fn test_insufficient_reputation() {
        let storage = Arc::new(MemoryStorage::new());
        let balance_mgr = Arc::new(BalanceManager::new(storage));
        let escrow_mgr = Arc::new(EscrowManager::new(balance_mgr.clone()));
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));

        let sponsor = PublicKey::from_bytes([3; 32]);
        let sponsor_addr = adic_economics::types::AccountAddress::from_public_key(&sponsor);

        balance_mgr
            .credit(sponsor_addr, AdicAmount::from_adic(1000.0))
            .await
            .unwrap();

        // Set low reputation (below minimum)
        rep_tracker.set_reputation(&sponsor, 50.0).await;

        let task_manager = TaskManager::new(escrow_mgr, rep_tracker, TaskManagerConfig::default());

        let task = Task {
            task_id: [3u8; 32],
            sponsor,
            task_type: TaskType::Compute {
                computation_type: ComputationType::HashVerification,
                resource_requirements: ResourceRequirements::default(),
            },
            input_cid: "QmTest789".to_string(),
            expected_output_schema: None,
            reward: AdicAmount::from_adic(10.0),
            collateral_requirement: AdicAmount::from_adic(5.0),
            deadline_epoch: 30,
            min_reputation: 100.0,
            worker_count: 1,
            created_at: Utc::now(),
            status: TaskStatus::Submitted,
            finality_status: FinalityStatus::Pending,
        };

        let result = task_manager.submit_task(task, 10).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PoUWError::InsufficientReputation { .. }
        ));
    }
}
