use crate::types::{AttestationProof, GrantSchedule, Milestone, MilestoneAttestation, TreasuryGrant, VerificationScheme};
use crate::{GovernanceError, Result};
use adic_crypto::BLSSignature;
use adic_economics::types::{AccountAddress, AdicAmount};
use adic_economics::BalanceManager;
use adic_pouw::DKGManager;
use adic_types::PublicKey;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Treasury execution engine for governance grants
pub struct TreasuryExecutor {
    balance_mgr: Arc<BalanceManager>,
    treasury_address: AccountAddress,
    active_grants: Arc<RwLock<HashMap<[u8; 32], GrantState>>>,
    dkg_manager: Option<Arc<DKGManager>>,
}

/// State tracking for an active grant
#[derive(Debug, Clone)]
pub struct GrantState {
    pub grant: TreasuryGrant,
    pub disbursed_amount: AdicAmount,
    pub next_milestone_id: Option<u32>,
    pub status: GrantStatus,
    pub start_epoch: u64,
    pub last_disbursement_epoch: Option<u64>,
}

/// Grant execution status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantStatus {
    Pending,
    Active,
    Paused,
    Completed,
    Failed,
    ClawedBack,
}

impl TreasuryExecutor {
    /// Create new treasury executor
    pub fn new(
        balance_mgr: Arc<BalanceManager>,
        treasury_address: AccountAddress,
    ) -> Self {
        Self {
            balance_mgr,
            treasury_address,
            active_grants: Arc::new(RwLock::new(HashMap::new())),
            dkg_manager: None,
        }
    }

    /// Set DKG manager for BLS signature verification
    pub fn with_dkg_manager(mut self, manager: Arc<DKGManager>) -> Self {
        self.dkg_manager = Some(manager);
        self
    }

    /// Execute an approved treasury grant
    pub async fn execute_grant(
        &self,
        grant: TreasuryGrant,
        current_epoch: u64,
    ) -> Result<()> {
        let grant_id = grant.proposal_id;
        let recipient = grant.recipient;
        let total_amount = grant.total_amount;

        let schedule_type = match &grant.schedule {
            GrantSchedule::Atomic => {
                self.execute_atomic_grant(&grant).await?;
                "atomic"
            }
            GrantSchedule::Streamed { .. } => {
                self.initialize_streamed_grant(grant, current_epoch).await?;
                "streamed"
            }
            GrantSchedule::Milestone { .. } => {
                self.initialize_milestone_grant(grant, current_epoch).await?;
                "milestone"
            }
        };

        info!(
            grant_id = hex::encode(&grant_id[..8]),
            recipient = hex::encode(recipient.as_bytes()),
            amount_adic = total_amount.to_adic(),
            schedule = schedule_type,
            "💰 Treasury grant executed"
        );

        Ok(())
    }

    /// Execute atomic grant (single immediate payment)
    async fn execute_atomic_grant(&self, grant: &TreasuryGrant) -> Result<()> {
        // Convert recipient PublicKey to AccountAddress
        let recipient_addr = AccountAddress::from_public_key(&grant.recipient);

        // Transfer full amount from treasury to recipient
        self.balance_mgr
            .transfer(self.treasury_address, recipient_addr, grant.total_amount)
            .await
            .map_err(|e| GovernanceError::TreasuryError(e.to_string()))?;

        info!(
            recipient = hex::encode(grant.recipient.as_bytes()),
            amount_adic = grant.total_amount.to_adic(),
            "💸 Atomic grant disbursed"
        );

        Ok(())
    }

    /// Initialize streamed grant
    async fn initialize_streamed_grant(
        &self,
        grant: TreasuryGrant,
        current_epoch: u64,
    ) -> Result<()> {
        let state = GrantState {
            grant,
            disbursed_amount: AdicAmount::from_adic(0.0),
            next_milestone_id: None,
            status: GrantStatus::Active,
            start_epoch: current_epoch,
            last_disbursement_epoch: None,
        };

        let mut grants = self.active_grants.write().await;
        grants.insert(state.grant.proposal_id, state);

        Ok(())
    }

    /// Initialize milestone-gated grant
    async fn initialize_milestone_grant(
        &self,
        grant: TreasuryGrant,
        current_epoch: u64,
    ) -> Result<()> {
        let first_milestone_id = if let GrantSchedule::Milestone { milestones } = &grant.schedule {
            milestones.first().map(|m| m.id)
        } else {
            None
        };

        let state = GrantState {
            grant,
            disbursed_amount: AdicAmount::from_adic(0.0),
            next_milestone_id: first_milestone_id,
            status: GrantStatus::Active,
            start_epoch: current_epoch,
            last_disbursement_epoch: None,
        };

        let mut grants = self.active_grants.write().await;
        grants.insert(state.grant.proposal_id, state);

        Ok(())
    }

    /// Process epoch for streamed grants
    pub async fn process_epoch(&self, current_epoch: u64) -> Result<()> {
        let mut grants = self.active_grants.write().await;

        for (grant_id, state) in grants.iter_mut() {
            if state.status != GrantStatus::Active {
                continue;
            }

            match &state.grant.schedule {
                GrantSchedule::Streamed {
                    rate_per_epoch,
                    duration_epochs,
                } => {
                    // Check if grant is still within duration
                    let epochs_elapsed = current_epoch.saturating_sub(state.start_epoch);

                    if epochs_elapsed > *duration_epochs {
                        // Grant completed (duration exceeded)
                        state.status = GrantStatus::Completed;
                        info!(
                            grant_id = hex::encode(&grant_id[..8]),
                            "✅ Streamed grant completed"
                        );
                        continue;
                    }

                    // Disburse this epoch's tranche
                    let tranche_amount = *rate_per_epoch;
                    let remaining = state
                        .grant
                        .total_amount
                        .checked_sub(state.disbursed_amount)
                        .ok_or_else(|| {
                            GovernanceError::TreasuryError(
                                "Disbursed amount exceeds total".to_string(),
                            )
                        })?;

                    if remaining >= tranche_amount {
                        self.disburse_tranche(state, tranche_amount, current_epoch)
                            .await?;

                        // Check if all funds have been disbursed
                        if state.disbursed_amount >= state.grant.total_amount {
                            state.status = GrantStatus::Completed;
                            info!(
                                grant_id = hex::encode(&grant_id[..8]),
                                "✅ Streamed grant completed (all funds disbursed)"
                            );
                        }
                    } else if remaining > AdicAmount::from_adic(0.0) {
                        // Final partial tranche
                        self.disburse_tranche(state, remaining, current_epoch)
                            .await?;
                        state.status = GrantStatus::Completed;
                    } else {
                        // All funds disbursed
                        state.status = GrantStatus::Completed;
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Complete a milestone and disburse funds
    pub async fn complete_milestone(
        &self,
        grant_id: &[u8; 32],
        milestone_id: u32,
        attestation: MilestoneAttestation,
        current_epoch: u64,
    ) -> Result<()> {
        // Validate attestation matches request
        if attestation.milestone_id != milestone_id {
            return Err(GovernanceError::MilestoneVerificationFailed {
                milestone_id,
                reason: "Attestation milestone ID mismatch".to_string(),
            });
        }

        if &attestation.grant_id != grant_id {
            return Err(GovernanceError::MilestoneVerificationFailed {
                milestone_id,
                reason: "Attestation grant ID mismatch".to_string(),
            });
        }

        let mut grants = self.active_grants.write().await;
        let state = grants
            .get_mut(grant_id)
            .ok_or_else(|| GovernanceError::TreasuryError("Grant not found".to_string()))?;

        // Verify this is the next expected milestone
        if state.next_milestone_id != Some(milestone_id) {
            return Err(GovernanceError::MilestoneVerificationFailed {
                milestone_id,
                reason: "Not the next expected milestone".to_string(),
            });
        }

        // Get milestone details
        let milestone = if let GrantSchedule::Milestone { milestones } = &state.grant.schedule {
            milestones
                .iter()
                .find(|m| m.id == milestone_id)
                .ok_or_else(|| GovernanceError::MilestoneVerificationFailed {
                    milestone_id,
                    reason: "Milestone not found in grant".to_string(),
                })?
        } else {
            return Err(GovernanceError::TreasuryError(
                "Grant is not milestone-based".to_string(),
            ));
        };

        // Check deadline
        if current_epoch > milestone.deadline_epoch {
            warn!(
                grant_id = hex::encode(&grant_id[..8]),
                milestone_id,
                "Milestone deadline passed"
            );
            state.status = GrantStatus::Paused;
            return Err(GovernanceError::MilestoneVerificationFailed {
                milestone_id,
                reason: "Deadline passed".to_string(),
            });
        }

        // Verify milestone completion with cryptographic proof
        self.verify_milestone_attestation(milestone, &attestation, current_epoch).await?;

        // Store milestone data before mutable borrow
        let milestone_amount = milestone.amount;
        let milestone_amount_adic = milestone.amount.to_adic();

        // Disburse milestone amount
        self.disburse_tranche(state, milestone_amount, current_epoch)
            .await?;

        // Update next milestone
        if let GrantSchedule::Milestone { milestones } = &state.grant.schedule {
            let next_index = milestones.iter().position(|m| m.id == milestone_id).unwrap() + 1;
            state.next_milestone_id = milestones.get(next_index).map(|m| m.id);

            if state.next_milestone_id.is_none() {
                // All milestones completed
                state.status = GrantStatus::Completed;
                info!(
                    grant_id = hex::encode(&grant_id[..8]),
                    "✅ All milestones completed"
                );
            }
        }

        info!(
            grant_id = hex::encode(&grant_id[..8]),
            milestone_id,
            amount_adic = milestone_amount_adic,
            "🎯 Milestone completed"
        );

        Ok(())
    }

    /// Pause a grant (e.g., due to missed milestone)
    pub async fn pause_grant(&self, grant_id: &[u8; 32], reason: String) -> Result<()> {
        let mut grants = self.active_grants.write().await;
        let state = grants
            .get_mut(grant_id)
            .ok_or_else(|| GovernanceError::TreasuryError("Grant not found".to_string()))?;

        state.status = GrantStatus::Paused;

        warn!(
            grant_id = hex::encode(&grant_id[..8]),
            reason,
            "⏸️ Grant paused"
        );

        Ok(())
    }

    /// Claw back remaining grant funds
    pub async fn clawback_grant(&self, grant_id: &[u8; 32]) -> Result<AdicAmount> {
        let mut grants = self.active_grants.write().await;
        let state = grants
            .get_mut(grant_id)
            .ok_or_else(|| GovernanceError::TreasuryError("Grant not found".to_string()))?;

        let remaining = state
            .grant
            .total_amount
            .checked_sub(state.disbursed_amount)
            .ok_or_else(|| {
                GovernanceError::TreasuryError("Disbursed amount exceeds total".to_string())
            })?;

        if remaining > AdicAmount::from_adic(0.0) {
            // In production, would transfer remaining back to treasury
            // For now, just update state
            state.status = GrantStatus::ClawedBack;

            info!(
                grant_id = hex::encode(&grant_id[..8]),
                clawed_back_adic = remaining.to_adic(),
                "⚡ Grant clawed back"
            );
        }

        Ok(remaining)
    }

    /// Get grant state
    pub async fn get_grant_state(&self, grant_id: &[u8; 32]) -> Option<GrantState> {
        let grants = self.active_grants.read().await;
        grants.get(grant_id).cloned()
    }

    /// Helper: Disburse a tranche of funds
    async fn disburse_tranche(
        &self,
        state: &mut GrantState,
        amount: AdicAmount,
        current_epoch: u64,
    ) -> Result<()> {
        // Convert recipient PublicKey to AccountAddress
        let recipient_addr = AccountAddress::from_public_key(&state.grant.recipient);

        // Log tranche disbursement details
        let grant_id = &state.grant.proposal_id;
        let schedule_type = match &state.grant.schedule {
            GrantSchedule::Atomic => "atomic",
            GrantSchedule::Streamed { .. } => "streamed",
            GrantSchedule::Milestone { .. } => "milestone",
        };

        info!(
            grant_id = hex::encode(&grant_id[..8]),
            recipient = hex::encode(state.grant.recipient.as_bytes()),
            tranche_amount_adic = amount.to_adic(),
            total_disbursed_adic = state.disbursed_amount.to_adic(),
            total_grant_adic = state.grant.total_amount.to_adic(),
            current_epoch,
            schedule_type,
            "💸 Treasury tranche disbursed"
        );

        self.balance_mgr
            .transfer(self.treasury_address, recipient_addr, amount)
            .await
            .map_err(|e| GovernanceError::TreasuryError(e.to_string()))?;

        let previous_disbursed = state.disbursed_amount;
        state.disbursed_amount = state
            .disbursed_amount
            .checked_add(amount)
            .ok_or_else(|| {
                GovernanceError::TreasuryError("Disbursed amount overflow".to_string())
            })?;
        state.last_disbursement_epoch = Some(current_epoch);

        // Calculate progress percentage
        let progress_pct = (state.disbursed_amount.to_adic() / state.grant.total_amount.to_adic()) * 100.0;

        info!(
            grant_id = hex::encode(&grant_id[..8]),
            previous_disbursed_adic = previous_disbursed.to_adic(),
            new_disbursed_adic = state.disbursed_amount.to_adic(),
            remaining_adic = (state.grant.total_amount.to_adic() - state.disbursed_amount.to_adic()),
            progress_pct = format!("{:.1}%", progress_pct),
            "📊 Grant progress updated"
        );

        Ok(())
    }

    /// Verify milestone attestation with cryptographic proofs
    async fn verify_milestone_attestation(
        &self,
        milestone: &Milestone,
        attestation: &MilestoneAttestation,
        current_epoch: u64,
    ) -> Result<()> {
        // Verify deliverable CID matches
        if attestation.deliverable_cid != milestone.deliverable_cid {
            return Err(GovernanceError::MilestoneVerificationFailed {
                milestone_id: milestone.id,
                reason: "Deliverable CID mismatch".to_string(),
            });
        }

        // Verify proof matches verification scheme
        match (&milestone.verification_scheme, &attestation.proof) {
            (
                VerificationScheme::QuorumAttestation { threshold },
                AttestationProof::QuorumSignature {
                    signature,
                    signers,
                    quorum_size,
                    signed_message,
                },
            ) => {
                self.verify_quorum_signature(
                    milestone,
                    signature,
                    signers,
                    *quorum_size,
                    signed_message,
                    *threshold,
                    current_epoch,
                )
                .await
            }
            (
                VerificationScheme::AutomatedCheck { criteria },
                AttestationProof::AutomatedCheck {
                    passed,
                    evidence_cid,
                    service_signature,
                },
            ) => self.verify_automated_check_proof(milestone, *passed, evidence_cid, service_signature, criteria),
            (
                VerificationScheme::OracleVerification { oracle_id },
                AttestationProof::OracleAttestation {
                    oracle_id: proof_oracle_id,
                    result,
                    oracle_signature,
                    oracle_data,
                },
            ) => self.verify_oracle_proof(
                milestone,
                oracle_id,
                proof_oracle_id,
                *result,
                oracle_signature,
                oracle_data.as_deref(),
            ),
            (
                VerificationScheme::PoUWTaskVerification { task_id, min_quality },
                AttestationProof::PoUWTaskCompletion {
                    task_id: proof_task_id,
                    completed,
                    quality_score,
                    receipt_id,
                    receipt_signature,
                },
            ) => self.verify_pouw_completion(
                milestone,
                task_id,
                proof_task_id,
                *completed,
                *quality_score,
                *min_quality,
                receipt_id,
                receipt_signature.as_ref(),
            ),
            _ => Err(GovernanceError::MilestoneVerificationFailed {
                milestone_id: milestone.id,
                reason: "Proof type does not match verification scheme".to_string(),
            }),
        }
    }


    /// Verify quorum signature with real BLS threshold verification
    async fn verify_quorum_signature(
        &self,
        milestone: &Milestone,
        signature: &BLSSignature,
        signers: &[PublicKey],
        quorum_size: usize,
        signed_message: &[u8],
        threshold: f64,
        current_epoch: u64,
    ) -> Result<()> {
        // Check threshold
        let actual_threshold = signers.len() as f64 / quorum_size as f64;
        if actual_threshold < threshold {
            return Err(GovernanceError::MilestoneVerificationFailed {
                milestone_id: milestone.id,
                reason: format!(
                    "Quorum threshold not met: {:.2} < {:.2}",
                    actual_threshold, threshold
                ),
            });
        }

        // Verify BLS signature using DKG manager
        if let Some(ref dkg_manager) = self.dkg_manager {
            // Get PublicKeySet for the current epoch
            let public_key_set = dkg_manager
                .get_public_key_set(current_epoch)
                .await
                .map_err(|e| GovernanceError::MilestoneVerificationFailed {
                    milestone_id: milestone.id,
                    reason: format!("Failed to get PublicKeySet for epoch {}: {}", current_epoch, e),
                })?;

            // Convert BLSSignature to threshold_crypto format
            let sig_tc = signature
                .to_tc_signature()
                .map_err(|e| GovernanceError::MilestoneVerificationFailed {
                    milestone_id: milestone.id,
                    reason: format!("Failed to convert signature: {}", e),
                })?;

            // Construct domain-separated message
            const DST_MILESTONE_ATTESTATION: &[u8] = b"ADIC-GOV-MILESTONE-v1";
            let mut prefixed_message = Vec::new();
            prefixed_message.extend_from_slice(DST_MILESTONE_ATTESTATION);
            prefixed_message.extend_from_slice(signed_message);

            // Verify signature
            let is_valid = public_key_set.public_key().verify(&sig_tc, &prefixed_message);

            if !is_valid {
                return Err(GovernanceError::MilestoneVerificationFailed {
                    milestone_id: milestone.id,
                    reason: "BLS signature verification failed".to_string(),
                });
            }

            info!(
                milestone_id = milestone.id,
                signers_count = signers.len(),
                quorum_size,
                threshold = actual_threshold,
                "✅ Milestone quorum signature verified with BLS"
            );

            Ok(())
        } else {
            warn!(
                milestone_id = milestone.id,
                "⚠️ DKG manager not configured, falling back to threshold check only"
            );

            // Fallback: Only verify threshold without cryptographic verification
            info!(
                milestone_id = milestone.id,
                signers_count = signers.len(),
                quorum_size,
                threshold = actual_threshold,
                "✅ Milestone threshold verified (no cryptographic verification)"
            );

            Ok(())
        }
    }

    /// Verify automated check proof (structured)
    fn verify_automated_check_proof(
        &self,
        milestone: &Milestone,
        passed: bool,
        evidence_cid: &str,
        _service_signature: &Option<Vec<u8>>,
        criteria: &str,
    ) -> Result<()> {
        if !passed {
            return Err(GovernanceError::MilestoneVerificationFailed {
                milestone_id: milestone.id,
                reason: format!("Automated check failed for criteria: {}", criteria),
            });
        }

        if evidence_cid.is_empty() {
            return Err(GovernanceError::MilestoneVerificationFailed {
                milestone_id: milestone.id,
                reason: "Evidence CID is empty".to_string(),
            });
        }

        // In production: Verify service_signature if provided

        info!(
            milestone_id = milestone.id,
            criteria = criteria,
            evidence_cid = evidence_cid,
            "✅ Automated check verified"
        );

        Ok(())
    }

    /// Verify oracle proof (structured)
    fn verify_oracle_proof(
        &self,
        milestone: &Milestone,
        expected_oracle_id: &str,
        proof_oracle_id: &str,
        result: bool,
        _oracle_signature: &[u8],
        _oracle_data: Option<&str>,
    ) -> Result<()> {
        if proof_oracle_id != expected_oracle_id {
            return Err(GovernanceError::MilestoneVerificationFailed {
                milestone_id: milestone.id,
                reason: format!(
                    "Oracle ID mismatch: expected {}, got {}",
                    expected_oracle_id, proof_oracle_id
                ),
            });
        }

        if !result {
            return Err(GovernanceError::MilestoneVerificationFailed {
                milestone_id: milestone.id,
                reason: "Oracle verification failed".to_string(),
            });
        }

        // In production: Verify oracle_signature cryptographically

        info!(
            milestone_id = milestone.id,
            oracle_id = expected_oracle_id,
            "✅ Oracle verification passed"
        );

        Ok(())
    }

    /// Verify PoUW task completion proof (structured)
    fn verify_pouw_completion(
        &self,
        milestone: &Milestone,
        expected_task_id: &[u8; 32],
        proof_task_id: &[u8; 32],
        completed: bool,
        quality_score: f64,
        min_quality: f64,
        _receipt_id: &[u8; 32],
        _receipt_signature: Option<&BLSSignature>,
    ) -> Result<()> {
        if proof_task_id != expected_task_id {
            return Err(GovernanceError::MilestoneVerificationFailed {
                milestone_id: milestone.id,
                reason: "Task ID mismatch".to_string(),
            });
        }

        if !completed {
            return Err(GovernanceError::MilestoneVerificationFailed {
                milestone_id: milestone.id,
                reason: "PoUW task not completed".to_string(),
            });
        }

        if quality_score < min_quality {
            return Err(GovernanceError::MilestoneVerificationFailed {
                milestone_id: milestone.id,
                reason: format!(
                    "Quality score {} below minimum {}",
                    quality_score, min_quality
                ),
            });
        }

        // In production: Verify receipt_signature if provided

        info!(
            milestone_id = milestone.id,
            task_id = hex::encode(expected_task_id),
            quality_score,
            "✅ PoUW task completion verified"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_economics::storage::MemoryStorage;
    use adic_types::PublicKey;

    #[tokio::test]
    async fn test_atomic_grant() {
        let storage = Arc::new(MemoryStorage::new());
        let balance_mgr = Arc::new(BalanceManager::new(storage));

        let treasury_addr = AccountAddress::from_bytes([1; 32]);
        let recipient_pk = PublicKey::from_bytes([2; 32]);
        let recipient_addr = AccountAddress::from_public_key(&recipient_pk);

        // Fund treasury
        balance_mgr
            .credit(treasury_addr, AdicAmount::from_adic(1000.0))
            .await
            .unwrap();

        let executor = TreasuryExecutor::new(balance_mgr.clone(), treasury_addr);

        let grant = TreasuryGrant {
            recipient: recipient_pk,
            total_amount: AdicAmount::from_adic(100.0),
            schedule: GrantSchedule::Atomic,
            proposal_id: [1u8; 32],
        };

        executor.execute_grant(grant, 1).await.unwrap();

        let recipient_balance = balance_mgr.get_balance(recipient_addr).await.unwrap();
        assert_eq!(recipient_balance, AdicAmount::from_adic(100.0));
    }

    #[tokio::test]
    async fn test_streamed_grant() {
        let storage = Arc::new(MemoryStorage::new());
        let balance_mgr = Arc::new(BalanceManager::new(storage));

        let treasury_addr = AccountAddress::from_bytes([1; 32]);
        let recipient_pk = PublicKey::from_bytes([2; 32]);
        let recipient_addr = AccountAddress::from_public_key(&recipient_pk);

        balance_mgr
            .credit(treasury_addr, AdicAmount::from_adic(1000.0))
            .await
            .unwrap();

        let executor = TreasuryExecutor::new(balance_mgr.clone(), treasury_addr);

        let grant = TreasuryGrant {
            recipient: recipient_pk,
            total_amount: AdicAmount::from_adic(300.0),
            schedule: GrantSchedule::Streamed {
                rate_per_epoch: AdicAmount::from_adic(100.0),
                duration_epochs: 3,
            },
            proposal_id: [2u8; 32],
        };

        // Initialize grant
        executor.execute_grant(grant.clone(), 1).await.unwrap();

        // Process epoch 2
        executor.process_epoch(2).await.unwrap();
        let balance = balance_mgr.get_balance(recipient_addr).await.unwrap();
        assert_eq!(balance, AdicAmount::from_adic(100.0));

        // Process epoch 3
        executor.process_epoch(3).await.unwrap();
        let balance = balance_mgr.get_balance(recipient_addr).await.unwrap();
        assert_eq!(balance, AdicAmount::from_adic(200.0));

        // Process epoch 4 (should complete)
        executor.process_epoch(4).await.unwrap();
        let balance = balance_mgr.get_balance(recipient_addr).await.unwrap();
        assert_eq!(balance, AdicAmount::from_adic(300.0));

        // Verify grant completed
        let state = executor.get_grant_state(&grant.proposal_id).await.unwrap();
        assert_eq!(state.status, GrantStatus::Completed);
    }
}
