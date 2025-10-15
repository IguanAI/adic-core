use crate::error::{PoUWError, Result};
use crate::types::{
    Dispute, DisputeEvidence, DisputeStatus, Hash, TaskId,
    WorkResult,
};
use adic_challenges::{ChallengeConfig, ChallengeWindow};
use adic_consensus::ReputationTracker;
use adic_economics::types::AdicAmount;
use adic_types::PublicKey;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Manages disputes and challenges for PoUW tasks
pub struct DisputeManager {
    reputation_tracker: Arc<ReputationTracker>,
    config: DisputeConfig,
    disputes: Arc<RwLock<HashMap<Hash, Dispute>>>,
    arbitration_committees: Arc<RwLock<HashMap<Hash, Vec<PublicKey>>>>,
    challenge_windows: Arc<RwLock<HashMap<String, ChallengeWindow>>>,
}

#[derive(Debug, Clone)]
pub struct DisputeConfig {
    /// Minimum stake required to submit a challenge
    pub min_challenger_stake: AdicAmount,
    /// Challenge period duration in epochs
    pub challenge_period_epochs: u64,
    /// Minimum reputation for arbitrators
    pub min_arbitrator_reputation: f64,
    /// Number of arbitrators
    pub arbitration_committee_size: usize,
    /// Threshold for arbitration decision (e.g., 2/3 majority)
    pub arbitration_threshold: f64,
    /// Reward for successful challenge (as fraction of slashed amount)
    pub challenger_reward_fraction: f64,
    /// Appeal period in epochs
    pub appeal_period_epochs: u64,
}

impl Default for DisputeConfig {
    fn default() -> Self {
        Self {
            min_challenger_stake: AdicAmount::from_adic(10.0),
            challenge_period_epochs: 24,     // ~6 hours if epochs are ~15min
            min_arbitrator_reputation: 500.0,
            arbitration_committee_size: 7,
            arbitration_threshold: 0.67,     // 2/3 majority
            challenger_reward_fraction: 0.3, // 30% of slashed amount
            appeal_period_epochs: 48,        // ~12 hours
        }
    }
}

/// Arbitration decision from committee
#[derive(Debug, Clone)]
pub struct ArbitrationDecision {
    pub dispute_id: Hash,
    pub arbitrators: Vec<PublicKey>,
    pub votes_for_challenger: usize,
    pub votes_for_worker: usize,
    pub outcome: DisputeOutcome,
    pub reasoning: String,
    pub decided_at_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisputeOutcome {
    ChallengeAccepted,  // Challenger wins, worker slashed
    ChallengeRejected,  // Worker wins, challenger loses stake
    Inconclusive,       // No clear winner, both get stakes back
}

impl DisputeManager {
    pub fn new(
        reputation_tracker: Arc<ReputationTracker>,
        config: DisputeConfig,
    ) -> Self {
        Self {
            reputation_tracker,
            config,
            disputes: Arc::new(RwLock::new(HashMap::new())),
            arbitration_committees: Arc::new(RwLock::new(HashMap::new())),
            challenge_windows: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Open a challenge window for a task result
    pub async fn open_challenge_window(
        &self,
        task_id: &TaskId,
        result_id: &Hash,
        submitted_at_epoch: u64,
    ) -> Result<String> {
        let window_id = self.generate_window_id(task_id, result_id);

        // Create MessageId from task and result
        let mut msg_id_data = Vec::new();
        msg_id_data.extend_from_slice(task_id);
        msg_id_data.extend_from_slice(result_id);
        let msg_id = adic_types::MessageId::from_bytes(blake3::hash(&msg_id_data).into());

        let challenge_config = ChallengeConfig {
            window_depth: self.config.challenge_period_epochs,
            ..Default::default()
        };

        let window = ChallengeWindow::new(msg_id, submitted_at_epoch, challenge_config);

        self.challenge_windows
            .write()
            .await
            .insert(window_id.clone(), window);

        Ok(window_id)
    }

    /// Submit a challenge during the challenge period
    pub async fn submit_challenge(
        &self,
        task_id: &TaskId,
        result_id: &Hash,
        worker: &PublicKey,
        challenger: &PublicKey,
        challenger_stake: AdicAmount,
        evidence: DisputeEvidence,
        current_epoch: u64,
    ) -> Result<Hash> {
        // Verify minimum stake
        if challenger_stake.to_base_units() < self.config.min_challenger_stake.to_base_units() {
            return Err(PoUWError::Other(format!(
                "Insufficient challenger stake: required {}, provided {}",
                self.config.min_challenger_stake.to_base_units(),
                challenger_stake.to_base_units()
            )));
        }

        // Check if challenge period is active
        let window_id = self.generate_window_id(task_id, result_id);
        if !self.is_challenge_period_active(&window_id, current_epoch).await? {
            return Err(PoUWError::Other(
                "Challenge period has expired".to_string(),
            ));
        }

        // Generate dispute ID
        let dispute_id = self.generate_dispute_id(task_id, result_id, challenger);

        // Check for duplicate disputes
        if self.disputes.read().await.contains_key(&dispute_id) {
            return Err(PoUWError::Other("Dispute already exists".to_string()));
        }

        let resolution_deadline = current_epoch + self.config.challenge_period_epochs;

        let dispute = Dispute {
            dispute_id,
            task_id: *task_id,
            result_id: *result_id,
            worker: *worker,
            challenger: *challenger,
            challenger_stake,
            evidence,
            status: DisputeStatus::Submitted,
            created_at: Utc::now(),
            resolution_deadline,
        };

        info!(
            dispute_id = hex::encode(&dispute_id[..8]),
            task_id = hex::encode(&task_id[..8]),
            challenger = hex::encode(&challenger.as_bytes()[..8]),
            "⚖️ Challenge submitted"
        );

        self.disputes.write().await.insert(dispute_id, dispute);

        Ok(dispute_id)
    }

    /// Select arbitration committee for a dispute
    pub async fn select_arbitration_committee(
        &self,
        dispute_id: &Hash,
        current_epoch: u64,
    ) -> Result<Vec<PublicKey>> {
        // Check if dispute exists
        let mut disputes = self.disputes.write().await;
        let dispute = disputes
            .get_mut(dispute_id)
            .ok_or_else(|| PoUWError::Other("Dispute not found".to_string()))?;

        // Update status
        dispute.status = DisputeStatus::UnderReview;

        // In production, this would:
        // 1. Query reputation tracker for high-reputation nodes
        // 2. Use VRF to select committee deterministically
        // 3. Ensure no conflicts of interest

        // For now, generate deterministic committee from dispute_id
        let mut arbitrators = Vec::new();
        for i in 0..self.config.arbitration_committee_size {
            let mut seed = Vec::new();
            seed.extend_from_slice(dispute_id);
            seed.push(i as u8);
            seed.extend_from_slice(&current_epoch.to_le_bytes());
            let hash = blake3::hash(&seed);
            arbitrators.push(PublicKey::from_bytes(*hash.as_bytes()));
        }

        self.arbitration_committees
            .write()
            .await
            .insert(*dispute_id, arbitrators.clone());

        info!(
            dispute_id = hex::encode(&dispute_id[..8]),
            committee_size = arbitrators.len(),
            "👥 Arbitration committee selected"
        );

        Ok(arbitrators)
    }

    /// Process arbitration decision
    pub async fn process_arbitration(
        &self,
        dispute_id: &Hash,
        decision: ArbitrationDecision,
    ) -> Result<DisputeOutcome> {
        let mut disputes = self.disputes.write().await;
        let dispute = disputes
            .get_mut(dispute_id)
            .ok_or_else(|| PoUWError::Other("Dispute not found".to_string()))?;

        // Verify decision is from valid committee
        let committees = self.arbitration_committees.read().await;
        let expected_committee = committees
            .get(dispute_id)
            .ok_or_else(|| PoUWError::Other("No committee found".to_string()))?;

        // Verify all arbitrators are from the committee
        for arbitrator in &decision.arbitrators {
            if !expected_committee.contains(arbitrator) {
                return Err(PoUWError::Other(
                    "Invalid arbitrator in decision".to_string(),
                ));
            }
        }

        // Update dispute status based on outcome
        dispute.status = match decision.outcome {
            DisputeOutcome::ChallengeAccepted => DisputeStatus::Accepted,
            DisputeOutcome::ChallengeRejected => DisputeStatus::Rejected,
            DisputeOutcome::Inconclusive => DisputeStatus::Inconclusive,
        };

        // Apply reputation adjustments based on arbitration outcome
        let worker = dispute.worker;
        let challenger = dispute.challenger;
        drop(disputes); // Release write lock before async reputation updates

        match decision.outcome {
            DisputeOutcome::ChallengeAccepted => {
                // Challenger wins: slash worker reputation
                let current_rep = self.reputation_tracker.get_reputation(&worker).await;
                let slash_amount = (current_rep * 0.1).max(10.0); // Slash 10% or minimum 10 points
                let new_rep = (current_rep - slash_amount).max(0.1); // Floor at 0.1
                self.reputation_tracker
                    .set_reputation(&worker, new_rep)
                    .await;

                info!(
                    worker = hex::encode(worker.as_bytes()),
                    slash_amount,
                    new_reputation = new_rep,
                    "⚖️ Worker reputation slashed for failed challenge"
                );

                // Optionally reward challenger (small boost)
                let challenger_rep = self.reputation_tracker.get_reputation(&challenger).await;
                self.reputation_tracker
                    .set_reputation(&challenger, (challenger_rep + 5.0).min(10.0))
                    .await;
            }
            DisputeOutcome::ChallengeRejected => {
                // Worker wins: slash challenger reputation for frivolous challenge
                let current_rep = self.reputation_tracker.get_reputation(&challenger).await;
                let slash_amount = (current_rep * 0.05).max(5.0); // Slash 5% or minimum 5 points
                let new_rep = (current_rep - slash_amount).max(0.1); // Floor at 0.1
                self.reputation_tracker
                    .set_reputation(&challenger, new_rep)
                    .await;

                info!(
                    challenger = hex::encode(challenger.as_bytes()),
                    slash_amount,
                    new_reputation = new_rep,
                    "⚖️ Challenger reputation slashed for rejected challenge"
                );
            }
            DisputeOutcome::Inconclusive => {
                // No reputation changes for inconclusive disputes
                info!(
                    "⚖️ Inconclusive dispute - no reputation changes applied"
                );
            }
        }

        info!(
            dispute_id = hex::encode(&dispute_id[..8]),
            outcome = ?decision.outcome,
            votes_for = decision.votes_for_challenger,
            votes_against = decision.votes_for_worker,
            "⚖️ Arbitration decision processed"
        );

        Ok(decision.outcome)
    }

    /// Verify dispute evidence
    pub async fn verify_evidence(
        &self,
        evidence: &DisputeEvidence,
        original_result: &WorkResult,
    ) -> Result<bool> {
        match evidence {
            DisputeEvidence::ReExecutionProof {
                different_output_cid,
                execution_trace,
            } => {
                // Verify re-execution produced different result
                if different_output_cid == &original_result.output_cid {
                    return Ok(false); // Same output, no discrepancy
                }

                // Verify execution trace is valid
                if execution_trace.is_empty() {
                    return Ok(false);
                }

                Ok(true)
            }
            DisputeEvidence::InputOutputMismatch {
                expected_output,
                actual_output,
            } => {
                // Verify there's a mismatch
                Ok(expected_output != actual_output)
            }
            DisputeEvidence::ResourceUsageFraud {
                claimed_metrics,
                actual_metrics,
            } => {
                // Verify significant discrepancy in resource usage
                let cpu_diff = (claimed_metrics.cpu_time_ms as i64
                    - actual_metrics.cpu_time_ms as i64)
                    .abs();
                let memory_diff = (claimed_metrics.memory_used_mb as i64
                    - actual_metrics.memory_used_mb as i64)
                    .abs();

                // Threshold: >50% difference is fraud
                let cpu_fraud = cpu_diff > (claimed_metrics.cpu_time_ms as i64 / 2);
                let memory_fraud = memory_diff > (claimed_metrics.memory_used_mb as i64 / 2);

                Ok(cpu_fraud || memory_fraud)
            }
        }
    }

    /// Submit appeal to governance
    pub async fn submit_appeal(
        &self,
        dispute_id: &Hash,
        appellant: &PublicKey,
        _appeal_evidence: String,
        current_epoch: u64,
    ) -> Result<()> {
        let mut disputes = self.disputes.write().await;
        let dispute = disputes
            .get_mut(dispute_id)
            .ok_or_else(|| PoUWError::Other("Dispute not found".to_string()))?;

        // Verify appeal period is active
        if current_epoch > dispute.resolution_deadline + self.config.appeal_period_epochs {
            return Err(PoUWError::Other("Appeal period has expired".to_string()));
        }

        // Update status
        dispute.status = DisputeStatus::Appealed;

        info!(
            dispute_id = hex::encode(&dispute_id[..8]),
            appellant = hex::encode(&appellant.as_bytes()[..8]),
            "📢 Appeal submitted to governance"
        );

        Ok(())
    }

    /// Calculate challenger reward based on slashed amount
    pub fn calculate_challenger_reward(&self, slashed_amount: AdicAmount) -> AdicAmount {
        let reward_base =
            (slashed_amount.to_base_units() as f64 * self.config.challenger_reward_fraction) as u64;
        AdicAmount::from_base_units(reward_base)
    }

    /// Check if challenge period is active
    async fn is_challenge_period_active(
        &self,
        window_id: &str,
        current_epoch: u64,
    ) -> Result<bool> {
        let windows = self.challenge_windows.read().await;
        if let Some(window) = windows.get(window_id) {
            Ok(window.is_active(current_epoch))
        } else {
            Ok(false) // No window means not active
        }
    }

    /// Generate window ID for challenge period
    fn generate_window_id(&self, task_id: &TaskId, result_id: &Hash) -> String {
        format!(
            "pouw_challenge_{}_{}",
            hex::encode(&task_id[..8]),
            hex::encode(&result_id[..8])
        )
    }

    /// Generate deterministic dispute ID
    fn generate_dispute_id(&self, task_id: &TaskId, result_id: &Hash, challenger: &PublicKey) -> Hash {
        let mut data = Vec::new();
        data.extend_from_slice(task_id);
        data.extend_from_slice(result_id);
        data.extend_from_slice(challenger.as_bytes());
        data.extend_from_slice(&Utc::now().timestamp().to_le_bytes());
        blake3::hash(&data).into()
    }

    /// Get dispute by ID
    pub async fn get_dispute(&self, dispute_id: &Hash) -> Option<Dispute> {
        self.disputes.read().await.get(dispute_id).cloned()
    }

    /// Get all disputes for a task
    pub async fn get_task_disputes(&self, task_id: &TaskId) -> Vec<Dispute> {
        self.disputes
            .read()
            .await
            .values()
            .filter(|d| &d.task_id == task_id)
            .cloned()
            .collect()
    }

    /// Get statistics
    pub async fn get_stats(&self) -> DisputeStats {
        let disputes = self.disputes.read().await;

        let total = disputes.len() as u64;
        let submitted = disputes
            .values()
            .filter(|d| d.status == DisputeStatus::Submitted)
            .count() as u64;
        let under_review = disputes
            .values()
            .filter(|d| d.status == DisputeStatus::UnderReview)
            .count() as u64;
        let accepted = disputes
            .values()
            .filter(|d| d.status == DisputeStatus::Accepted)
            .count() as u64;
        let rejected = disputes
            .values()
            .filter(|d| d.status == DisputeStatus::Rejected)
            .count() as u64;
        let appealed = disputes
            .values()
            .filter(|d| d.status == DisputeStatus::Appealed)
            .count() as u64;

        DisputeStats {
            total_disputes: total,
            submitted,
            under_review,
            accepted,
            rejected,
            appealed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DisputeStats {
    pub total_disputes: u64,
    pub submitted: u64,
    pub under_review: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub appealed: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ExecutionProof;
    use crate::ExecutionMetrics;
    use chrono::Utc;

    fn create_test_work_result(output_cid: String) -> WorkResult {
        WorkResult {
            result_id: [1u8; 32],
            task_id: [2u8; 32],
            worker: PublicKey::from_bytes([3u8; 32]),
            output_cid,
            execution_proof: ExecutionProof {
                proof_type: crate::types::ProofType::MerkleTree,
                merkle_root: [0u8; 32],
                intermediate_states: vec![],
                proof_data: vec![],
            },
            execution_metrics: ExecutionMetrics {
                cpu_time_ms: 1000,
                memory_used_mb: 128,
                storage_used_mb: 64,
                network_used_kb: 512,
                start_time: Utc::now(),
                end_time: Utc::now(),
            },
            worker_signature: vec![0u8; 64],
            submitted_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_challenge_submission() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let dispute_mgr = DisputeManager::new(
            rep_tracker,
            DisputeConfig::default(),
        );

        let task_id = [1u8; 32];
        let result_id = [2u8; 32];
        let worker = PublicKey::from_bytes([3u8; 32]);
        let challenger = PublicKey::from_bytes([10u8; 32]);

        // Open challenge window
        dispute_mgr.open_challenge_window(&task_id, &result_id, 10).await.unwrap();

        // Submit challenge
        let evidence = DisputeEvidence::ReExecutionProof {
            different_output_cid: "QmDifferent123".to_string(),
            execution_trace: vec![[1u8; 32], [2u8; 32]],
        };

        let dispute_id = dispute_mgr
            .submit_challenge(
                &task_id,
                &result_id,
                &worker,
                &challenger,
                AdicAmount::from_adic(20.0),
                evidence,
                10,
            )
            .await
            .unwrap();

        // Verify dispute was created
        let dispute = dispute_mgr.get_dispute(&dispute_id).await.unwrap();
        assert_eq!(dispute.task_id, task_id);
        assert_eq!(dispute.challenger, challenger);
        assert_eq!(dispute.status, DisputeStatus::Submitted);
    }

    #[tokio::test]
    async fn test_insufficient_stake_rejection() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let dispute_mgr = DisputeManager::new(
            rep_tracker,
            DisputeConfig::default(),
        );

        let task_id = [3u8; 32];
        let result_id = [4u8; 32];
        let worker = PublicKey::from_bytes([4u8; 32]);
        let challenger = PublicKey::from_bytes([11u8; 32]);

        dispute_mgr.open_challenge_window(&task_id, &result_id, 10).await.unwrap();

        let evidence = DisputeEvidence::InputOutputMismatch {
            expected_output: "expected".to_string(),
            actual_output: "actual".to_string(),
        };

        // Insufficient stake
        let result = dispute_mgr
            .submit_challenge(
                &task_id,
                &result_id,
                &worker,
                &challenger,
                AdicAmount::from_adic(5.0), // Below minimum
                evidence,
                10,
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_arbitration_committee_selection() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let config = DisputeConfig {
            arbitration_committee_size: 5,
            ..Default::default()
        };
        let dispute_mgr = DisputeManager::new(rep_tracker, config);

        let task_id = [5u8; 32];
        let result_id = [6u8; 32];
        let worker = PublicKey::from_bytes([5u8; 32]);
        let challenger = PublicKey::from_bytes([12u8; 32]);

        dispute_mgr.open_challenge_window(&task_id, &result_id, 10).await.unwrap();

        let evidence = DisputeEvidence::ReExecutionProof {
            different_output_cid: "QmNew456".to_string(),
            execution_trace: vec![[3u8; 32]],
        };

        let dispute_id = dispute_mgr
            .submit_challenge(
                &task_id,
                &result_id,
                &worker,
                &challenger,
                AdicAmount::from_adic(15.0),
                evidence,
                10,
            )
            .await
            .unwrap();

        // Select committee
        let committee = dispute_mgr
            .select_arbitration_committee(&dispute_id, 11)
            .await
            .unwrap();

        assert_eq!(committee.len(), 5);

        // Verify dispute status updated
        let dispute = dispute_mgr.get_dispute(&dispute_id).await.unwrap();
        assert_eq!(dispute.status, DisputeStatus::UnderReview);
    }

    #[tokio::test]
    async fn test_evidence_verification() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let dispute_mgr = DisputeManager::new(rep_tracker, DisputeConfig::default());

        let original_result = create_test_work_result("QmOriginal123".to_string());

        // Test re-execution proof with different output
        let evidence1 = DisputeEvidence::ReExecutionProof {
            different_output_cid: "QmDifferent456".to_string(),
            execution_trace: vec![[1u8; 32], [2u8; 32]],
        };
        assert!(dispute_mgr.verify_evidence(&evidence1, &original_result).await.unwrap());

        // Test re-execution proof with same output (invalid)
        let evidence2 = DisputeEvidence::ReExecutionProof {
            different_output_cid: "QmOriginal123".to_string(),
            execution_trace: vec![[1u8; 32]],
        };
        assert!(!dispute_mgr.verify_evidence(&evidence2, &original_result).await.unwrap());

        // Test resource usage fraud
        let evidence3 = DisputeEvidence::ResourceUsageFraud {
            claimed_metrics: ExecutionMetrics {
                cpu_time_ms: 1000,
                memory_used_mb: 128,
                storage_used_mb: 64,
                network_used_kb: 512,
                start_time: Utc::now(),
                end_time: Utc::now(),
            },
            actual_metrics: ExecutionMetrics {
                cpu_time_ms: 100,  // 10x difference = fraud
                memory_used_mb: 128,
                storage_used_mb: 64,
                network_used_kb: 512,
                start_time: Utc::now(),
                end_time: Utc::now(),
            },
        };
        assert!(dispute_mgr.verify_evidence(&evidence3, &original_result).await.unwrap());
    }

    #[tokio::test]
    async fn test_challenger_reward_calculation() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let config = DisputeConfig {
            challenger_reward_fraction: 0.3,
            ..Default::default()
        };
        let dispute_mgr = DisputeManager::new(rep_tracker, config);

        let slashed = AdicAmount::from_adic(100.0);
        let reward = dispute_mgr.calculate_challenger_reward(slashed);

        // Should be 30% of slashed amount
        assert_eq!(reward.to_base_units(), (slashed.to_base_units() as f64 * 0.3) as u64);
    }

    #[tokio::test]
    async fn test_dispute_stats() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let dispute_mgr = DisputeManager::new(
            rep_tracker,
            DisputeConfig::default(),
        );

        // Submit multiple disputes
        for i in 0..3 {
            let task_id = [i; 32];
            let result_id = [i + 10; 32];
            let worker = PublicKey::from_bytes([i + 15; 32]);
            let challenger = PublicKey::from_bytes([i + 20; 32]);

            dispute_mgr.open_challenge_window(&task_id, &result_id, 10).await.unwrap();

            let evidence = DisputeEvidence::InputOutputMismatch {
                expected_output: format!("expected{}", i),
                actual_output: format!("actual{}", i),
            };

            dispute_mgr
                .submit_challenge(
                    &task_id,
                    &result_id,
                    &worker,
                    &challenger,
                    AdicAmount::from_adic(15.0),
                    evidence,
                    10,
                )
                .await
                .unwrap();
        }

        let stats = dispute_mgr.get_stats().await;
        assert_eq!(stats.total_disputes, 3);
        assert_eq!(stats.submitted, 3);
    }
}
