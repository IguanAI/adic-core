use crate::treasury::TreasuryExecutor;
use crate::types::{
    GovernanceProposal, GovernanceReceipt, GovernanceVote, ProposalStatus,
    VoteResult,
};
use crate::voting::VotingEngine;
use crate::{GovernanceError, Result};
use adic_app_common::{FinalityAdapter, FinalityPolicy, NetworkMetadataRegistry, ParameterStore};
use adic_consensus::ReputationTracker;
use adic_crypto::{BLSSignature, BLSThresholdSigner};
use adic_pouw::{BLSCoordinator, DKGManager};
use adic_quorum::QuorumSelector;
use adic_types::{MessageId, PublicKey};
use blake3;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Domain Separation Tag for governance receipt signatures
pub const DST_GOVERNANCE_RECEIPT: &[u8] = b"ADIC-GOV-R-v1";

/// Callback function type for receipt emission to DAG
pub type ReceiptCallback = Arc<
    dyn Fn(GovernanceReceipt, u64) -> Pin<Box<dyn Future<Output = std::result::Result<MessageId, String>> + Send>>
        + Send
        + Sync,
>;

/// Configuration for proposal lifecycle management
#[derive(Debug, Clone)]
pub struct LifecycleConfig {
    /// Minimum reputation to submit proposal
    pub min_proposer_reputation: f64,
    /// Voting period duration in seconds
    pub voting_duration_secs: i64,
    /// Minimum quorum participation (percentage of total eligible credits)
    pub min_quorum: f64,
    /// Maximum reputation cap for voting credits (Rmax)
    pub rmax: f64,
    /// Timelock multiplier for F1 finality (γ₁)
    pub gamma_f1: f64,
    /// Timelock multiplier for F2 finality (γ₂)
    pub gamma_f2: f64,
    /// Minimum timelock duration in seconds (Tmin)
    pub min_timelock_secs: f64,
    /// Governance committee size
    pub committee_size: usize,
    /// BLS threshold (t-of-n)
    pub bls_threshold: usize,
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            min_proposer_reputation: 100.0,
            voting_duration_secs: 7 * 24 * 3600, // 7 days
            min_quorum: 0.1,                      // 10% participation
            rmax: 100_000.0,
            gamma_f1: 2.0,
            gamma_f2: 3.0,
            min_timelock_secs: 1000.0,
            committee_size: 15,                   // Governance committee size
            bls_threshold: 10,                    // 2/3 threshold (10 of 15)
        }
    }
}

/// Proposal lifecycle manager
///
/// # Event Emission
/// State changes should emit events when used in a node context:
/// - submit_proposal() → ProposalSubmitted
/// - cast_vote() → VoteCast
/// - finalize_voting() → ProposalTallied + ProposalStatusChanged + GovernanceReceiptEmitted
/// - enact_proposal() → ProposalEnacted + ParameterChanged (per parameter)
pub struct ProposalLifecycleManager {
    config: LifecycleConfig,
    voting_engine: VotingEngine,
    reputation_tracker: Arc<ReputationTracker>,
    finality_adapter: Option<Arc<FinalityAdapter>>,
    parameter_store: Option<Arc<ParameterStore>>,
    quorum_selector: Option<Arc<QuorumSelector>>,
    bls_signer: Option<Arc<BLSThresholdSigner>>,
    dkg_manager: Option<Arc<DKGManager>>,
    bls_coordinator: Option<Arc<BLSCoordinator>>,
    network_metadata: Option<Arc<NetworkMetadataRegistry>>,
    treasury_executor: Option<Arc<TreasuryExecutor>>,
    receipt_callback: Option<ReceiptCallback>,
    proposals: Arc<RwLock<HashMap<[u8; 32], GovernanceProposal>>>,
    votes: Arc<RwLock<HashMap<[u8; 32], Vec<GovernanceVote>>>>,
    receipts: Arc<RwLock<HashMap<[u8; 32], Vec<GovernanceReceipt>>>>,
    // Track message IDs for finality checking
    proposal_message_ids: Arc<RwLock<HashMap<[u8; 32], MessageId>>>,
}

impl ProposalLifecycleManager {
    /// Create new lifecycle manager
    pub fn new(
        config: LifecycleConfig,
        reputation_tracker: Arc<ReputationTracker>,
    ) -> Self {
        let voting_engine = VotingEngine::new(config.rmax, config.min_quorum);

        Self {
            config,
            voting_engine,
            reputation_tracker,
            finality_adapter: None,
            parameter_store: None,
            quorum_selector: None,
            bls_signer: None,
            dkg_manager: None,
            bls_coordinator: None,
            network_metadata: None,
            treasury_executor: None,
            receipt_callback: None,
            proposals: Arc::new(RwLock::new(HashMap::new())),
            votes: Arc::new(RwLock::new(HashMap::new())),
            receipts: Arc::new(RwLock::new(HashMap::new())),
            proposal_message_ids: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Set finality adapter (for production use with real F1/F2)
    pub fn with_finality_adapter(mut self, adapter: Arc<FinalityAdapter>) -> Self {
        self.finality_adapter = Some(adapter);
        self
    }

    /// Set parameter store (for live parameter updates)
    pub fn with_parameter_store(mut self, store: Arc<ParameterStore>) -> Self {
        self.parameter_store = Some(store);
        self
    }

    /// Set quorum selector for committee selection
    pub fn with_quorum_selector(mut self, selector: Arc<QuorumSelector>) -> Self {
        self.quorum_selector = Some(selector);
        self
    }

    /// Set BLS threshold signer for receipt signatures
    pub fn with_bls_signer(mut self, signer: Arc<BLSThresholdSigner>) -> Self {
        self.bls_signer = Some(signer);
        self
    }

    /// Set DKG manager for committee key generation
    pub fn with_dkg_manager(mut self, manager: Arc<DKGManager>) -> Self {
        self.dkg_manager = Some(manager);
        self
    }

    /// Set BLS coordinator for threshold signature collection
    pub fn with_bls_coordinator(mut self, coordinator: Arc<BLSCoordinator>) -> Self {
        self.bls_coordinator = Some(coordinator);
        self
    }

    /// Set network metadata registry for ASN/region lookups
    pub fn with_network_metadata(mut self, registry: Arc<NetworkMetadataRegistry>) -> Self {
        self.network_metadata = Some(registry);
        self
    }

    /// Set treasury executor for executing treasury grants
    pub fn with_treasury_executor(mut self, executor: Arc<TreasuryExecutor>) -> Self {
        self.treasury_executor = Some(executor);
        self
    }

    /// Set receipt callback for emitting receipts to DAG
    pub fn with_receipt_callback(mut self, callback: ReceiptCallback) -> Self {
        self.receipt_callback = Some(callback);
        self
    }

    /// Submit a new proposal
    pub async fn submit_proposal(&self, mut proposal: GovernanceProposal) -> Result<[u8; 32]> {
        // Check proposer reputation
        let proposer_rep = self
            .reputation_tracker
            .get_reputation(&proposal.proposer_pk)
            .await;

        if proposer_rep < self.config.min_proposer_reputation {
            return Err(GovernanceError::InsufficientReputation {
                required: self.config.min_proposer_reputation,
                actual: proposer_rep,
            });
        }

        // Set voting period
        proposal.voting_end_timestamp =
            proposal.creation_timestamp + chrono::Duration::seconds(self.config.voting_duration_secs);

        // Set initial status
        proposal.status = ProposalStatus::Voting;

        let proposal_id = proposal.proposal_id;

        // Store proposal
        let mut proposals = self.proposals.write().await;
        proposals.insert(proposal_id, proposal.clone());

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            proposer = hex::encode(&proposal.proposer_pk.as_bytes()[..8]),
            class = ?proposal.class,
            voting_ends = %proposal.voting_end_timestamp,
            "📜 Proposal submitted"
        );

        Ok(proposal_id)
    }

    /// Cast a vote on a proposal
    pub async fn cast_vote(&self, vote: GovernanceVote) -> Result<()> {
        // Get proposal
        let proposals = self.proposals.read().await;
        let proposal = proposals
            .get(&vote.proposal_id)
            .ok_or_else(|| GovernanceError::ProposalNotFound(hex::encode(&vote.proposal_id[..8])))?;

        // Check proposal is in voting status
        if proposal.status != ProposalStatus::Voting {
            return Err(GovernanceError::InvalidStatus {
                expected: "Voting".to_string(),
                found: format!("{:?}", proposal.status),
            });
        }

        // Check voting period hasn't ended
        if proposal.voting_ended(Utc::now()) {
            return Err(GovernanceError::VotingEnded);
        }

        // Get voter reputation
        let voter_rep = self
            .reputation_tracker
            .get_reputation(&vote.voter_pk)
            .await;

        // Validate vote credits match reputation
        self.voting_engine.validate_credits(vote.credits, voter_rep)?;

        // Check for duplicate vote
        let votes = self.votes.read().await;
        if let Some(existing_votes) = votes.get(&vote.proposal_id) {
            if existing_votes.iter().any(|v| v.voter_pk == vote.voter_pk) {
                return Err(GovernanceError::DuplicateVote(hex::encode(
                    vote.voter_pk.as_bytes(),
                )));
            }
        }
        drop(votes);

        // Store vote
        let mut votes = self.votes.write().await;
        votes
            .entry(vote.proposal_id)
            .or_insert_with(Vec::new)
            .push(vote.clone());

        info!(
            proposal_id = hex::encode(&vote.proposal_id[..8]),
            voter = hex::encode(&vote.voter_pk.as_bytes()[..8]),
            ballot = ?vote.ballot,
            credits = vote.credits,
            "🗳️ Vote cast"
        );

        Ok(())
    }

    /// Finalize voting and tally results
    pub async fn finalize_voting(&self, proposal_id: &[u8; 32]) -> Result<VoteResult> {
        // Get proposal
        let mut proposals = self.proposals.write().await;
        let proposal = proposals
            .get_mut(proposal_id)
            .ok_or_else(|| GovernanceError::ProposalNotFound(hex::encode(&proposal_id[..8])))?;

        // Check status
        if proposal.status != ProposalStatus::Voting {
            return Err(GovernanceError::InvalidStatus {
                expected: "Voting".to_string(),
                found: format!("{:?}", proposal.status),
            });
        }

        // Check voting period has ended
        if !proposal.voting_ended(Utc::now()) {
            return Err(GovernanceError::VotingNotEnded);
        }

        // Tally votes
        let votes = self.votes.read().await;
        let proposal_votes = votes.get(proposal_id).cloned().unwrap_or_default();

        let stats = self.voting_engine.tally_votes(&proposal_votes, proposal_id).await?;

        // Update proposal tallies
        proposal.tally_yes = stats.yes;
        proposal.tally_no = stats.no;
        proposal.tally_abstain = stats.abstain;
        proposal.status = ProposalStatus::Tallying;

        // Calculate total eligible credits
        let total_eligible = self.calculate_total_eligible_credits().await;

        // Check quorum
        let quorum_check = self
            .voting_engine
            .check_quorum(&stats, total_eligible);

        // Determine result
        let result = if quorum_check.is_ok() {
            let threshold = proposal.required_threshold();
            self.voting_engine.determine_result(&stats, threshold)?
        } else {
            warn!(
                proposal_id = hex::encode(&proposal_id[..8]),
                participation = stats.total_participation,
                required = total_eligible * self.config.min_quorum,
                "Quorum not met"
            );
            VoteResult::Fail
        };

        // Update proposal status based on result
        proposal.status = match result {
            VoteResult::Pass => ProposalStatus::Succeeded,
            VoteResult::Fail => ProposalStatus::Rejected,
        };

        // Create receipt with BLS signature
        let (quorum_signature, committee_members) = self
            .generate_receipt_signature(proposal_id, &stats, result)
            .await?;

        let receipt = GovernanceReceipt {
            proposal_id: *proposal_id,
            quorum_stats: stats.clone(),
            result,
            receipt_seq: 0,
            prev_receipt_hash: [0u8; 32],
            timestamp: Utc::now(),
            quorum_signature,
            committee_members,
        };

        // Store receipt
        let mut receipts = self.receipts.write().await;
        receipts
            .entry(*proposal_id)
            .or_insert_with(Vec::new)
            .push(receipt.clone());
        drop(receipts);

        // Emit receipt to DAG if callback configured
        if let Some(ref callback) = self.receipt_callback {
            // Use epoch 0 for now; in production would use finality engine epoch
            let current_epoch = 0u64;
            match callback(receipt, current_epoch).await {
                Ok(message_id) => {
                    info!(
                        proposal_id = hex::encode(&proposal_id[..8]),
                        message_id = ?message_id,
                        "📤 Governance receipt emitted to DAG"
                    );
                }
                Err(e) => {
                    warn!(
                        proposal_id = hex::encode(&proposal_id[..8]),
                        error = %e,
                        "⚠️ Failed to emit governance receipt to DAG"
                    );
                }
            }
        }

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            result = ?result,
            yes = proposal.tally_yes,
            no = proposal.tally_no,
            abstain = proposal.tally_abstain,
            "📊 Voting finalized"
        );

        Ok(result)
    }

    /// Register message ID for a proposal (for finality tracking)
    pub async fn register_proposal_message(&self, proposal_id: [u8; 32], message_id: MessageId) {
        let mut message_ids = self.proposal_message_ids.write().await;
        message_ids.insert(proposal_id, message_id);
    }

    /// Wait for governance-grade finality (F1 ∧ F2) per PoUW III §7.1
    /// Returns finality times for timelock calculation
    pub async fn wait_for_finality(
        &self,
        proposal_id: &[u8; 32],
        timeout: Duration,
    ) -> Result<(Duration, Duration)> {
        // Get finality adapter or skip if not configured (test mode)
        let adapter = match &self.finality_adapter {
            Some(adapter) => adapter,
            None => {
                warn!(
                    proposal_id = hex::encode(&proposal_id[..8]),
                    "⚠️  No finality adapter configured, skipping finality check (test mode)"
                );
                // Return default times for test mode
                return Ok((Duration::from_secs(1), Duration::from_secs(1)));
            }
        };

        // Get message ID for this proposal
        let message_ids = self.proposal_message_ids.read().await;
        let message_id = message_ids
            .get(proposal_id)
            .ok_or_else(|| {
                GovernanceError::InvalidOperation(format!(
                    "Message ID not found for proposal {}",
                    hex::encode(&proposal_id[..8])
                ))
            })?;

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            message_id = ?message_id,
            timeout_secs = timeout.as_secs(),
            "⏳ Waiting for governance-grade finality (F1 ∧ F2)..."
        );

        // Wait for STRICT finality (F1 AND F2) per PoUW III §7.1
        let status = adapter
            .wait_for_finality(message_id, FinalityPolicy::Strict, timeout)
            .await
            .map_err(|e| {
                GovernanceError::FinalityTimeout(format!(
                    "Finality timeout for proposal {}: {}",
                    hex::encode(&proposal_id[..8]),
                    e
                ))
            })?;

        // Extract finality times
        let f1_time = status
            .f1_time
            .map(|t| t.elapsed())
            .unwrap_or(Duration::from_secs(0));
        let f2_time = status
            .f2_time
            .map(|t| t.elapsed())
            .unwrap_or(Duration::from_secs(0));

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            f1_time_secs = f1_time.as_secs(),
            f2_time_secs = f2_time.as_secs(),
            f2_confidence = ?status.f2_confidence,
            "✅ Governance-grade finality achieved (F1 ∧ F2)"
        );

        Ok((f1_time, f2_time))
    }

    /// Calculate timelock duration based on finality times
    ///
    /// Formula (PoUW III §7.2): Tenact = max{γ₁·TF1, γ₂·TF2, Tmin}
    pub fn calculate_timelock(
        &self,
        f1_finality_time: Duration,
        f2_finality_time: Duration,
    ) -> Duration {
        let t_f1 = Duration::from_secs_f64(
            f1_finality_time.as_secs_f64() * self.config.gamma_f1
        );
        let t_f2 = Duration::from_secs_f64(
            f2_finality_time.as_secs_f64() * self.config.gamma_f2
        );
        let t_min = Duration::from_secs_f64(self.config.min_timelock_secs);

        let t_enact = t_f1.max(t_f2).max(t_min);

        info!(
            t_f1_secs = t_f1.as_secs(),
            t_f2_secs = t_f2.as_secs(),
            t_min_secs = t_min.as_secs(),
            t_enact_secs = t_enact.as_secs(),
            "⏱️  Calculated governance timelock"
        );

        t_enact
    }

    /// Begin enactment period (after finality achieved)
    pub async fn begin_enactment(
        &self,
        proposal_id: &[u8; 32],
        timelock_duration_secs: f64,
    ) -> Result<()> {
        let mut proposals = self.proposals.write().await;
        let proposal = proposals
            .get_mut(proposal_id)
            .ok_or_else(|| GovernanceError::ProposalNotFound(hex::encode(&proposal_id[..8])))?;

        // Check status
        if proposal.status != ProposalStatus::Succeeded {
            return Err(GovernanceError::InvalidStatus {
                expected: "Succeeded".to_string(),
                found: format!("{:?}", proposal.status),
            });
        }

        proposal.status = ProposalStatus::Enacting;

        // Calculate enactment epoch
        // Note: In production, this would be set to current_epoch + (timelock_secs / epoch_duration_secs)
        // For now, we store the original enact_epoch from proposal creation
        // The timelock is enforced by checking timestamp instead
        let current_time = Utc::now();
        let enact_time = current_time + chrono::Duration::seconds(timelock_duration_secs as i64);

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            timelock_secs = timelock_duration_secs,
            enact_epoch = proposal.enact_epoch,
            enact_time = %enact_time,
            "⏳ Proposal entered enactment period with dynamic timelock"
        );

        Ok(())
    }

    /// Complete workflow: Wait for finality, calculate dynamic timelock, begin enactment
    ///
    /// This integrates the timelock formula T_enact = max{γ₁T_F1, γ₂T_F2, T_min}
    /// per PoUW III §7.2 with actual finality metrics.
    pub async fn finalize_and_prepare_enactment(
        &self,
        proposal_id: &[u8; 32],
        finality_timeout: Duration,
    ) -> Result<Duration> {
        // Step 1: Wait for governance-grade finality (F1 ∧ F2)
        let (f1_time, f2_time) = self
            .wait_for_finality(proposal_id, finality_timeout)
            .await?;

        // Step 2: Calculate dynamic timelock based on observed finality times
        let timelock = self.calculate_timelock(f1_time, f2_time);

        // Step 3: Begin enactment period with calculated timelock
        self.begin_enactment(proposal_id, timelock.as_secs_f64())
            .await?;

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            f1_time_secs = f1_time.as_secs(),
            f2_time_secs = f2_time.as_secs(),
            timelock_secs = timelock.as_secs(),
            "🔗 Integrated finality → timelock → enactment"
        );

        Ok(timelock)
    }

    /// Enact a proposal (apply parameter changes)
    pub async fn enact_proposal(&self, proposal_id: &[u8; 32]) -> Result<()> {
        let mut proposals = self.proposals.write().await;
        let proposal = proposals
            .get_mut(proposal_id)
            .ok_or_else(|| GovernanceError::ProposalNotFound(hex::encode(&proposal_id[..8])))?;

        // Check status
        if proposal.status != ProposalStatus::Enacting {
            return Err(GovernanceError::InvalidStatus {
                expected: "Enacting".to_string(),
                found: format!("{:?}", proposal.status),
            });
        }

        // Check timelock has expired (simplified - would check against current epoch)
        // In production: verify current_epoch >= proposal.enact_epoch

        // Apply parameter changes if parameter store is configured
        if let Some(param_store) = &self.parameter_store {
            // Parse parameter changes from proposal
            let param_keys = &proposal.param_keys;
            let new_values = &proposal.new_values;

            // Apply each parameter change
            if let Some(obj) = new_values.as_object() {
                for key in param_keys {
                    if let Some(new_value) = obj.get(key) {
                        match param_store
                            .apply_governance_change(key, new_value.clone(), Some(*proposal_id))
                            .await
                        {
                            Ok(_) => {
                                info!(
                                    proposal_id = hex::encode(&proposal_id[..8]),
                                    param = %key,
                                    new_value = %new_value,
                                    "✅ Parameter updated"
                                );
                            }
                            Err(e) => {
                                warn!(
                                    proposal_id = hex::encode(&proposal_id[..8]),
                                    param = %key,
                                    error = %e,
                                    "❌ Failed to apply parameter change"
                                );
                                proposal.status = ProposalStatus::Failed;
                                return Err(GovernanceError::ParameterUpdateFailed {
                                    param: key.clone(),
                                    reason: e.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        } else {
            info!(
                proposal_id = hex::encode(&proposal_id[..8]),
                "⚠️  Parameter store not configured, skipping parameter updates"
            );
        }

        // Execute treasury grant if present
        if let Some(ref treasury_grant) = proposal.treasury_grant {
            if let Some(ref executor) = self.treasury_executor {
                // Execute the grant using the current enact_epoch
                match executor.execute_grant(treasury_grant.clone(), proposal.enact_epoch).await {
                    Ok(_) => {
                        info!(
                            proposal_id = hex::encode(&proposal_id[..8]),
                            recipient = hex::encode(treasury_grant.recipient.as_bytes()),
                            total_amount_adic = treasury_grant.total_amount.to_adic(),
                            schedule = ?treasury_grant.schedule,
                            "💸 Treasury grant executed"
                        );
                    }
                    Err(e) => {
                        warn!(
                            proposal_id = hex::encode(&proposal_id[..8]),
                            error = %e,
                            "❌ Failed to execute treasury grant"
                        );
                        proposal.status = ProposalStatus::Failed;
                        return Err(GovernanceError::TreasuryError(e.to_string()));
                    }
                }
            } else {
                warn!(
                    proposal_id = hex::encode(&proposal_id[..8]),
                    "⚠️  Treasury executor not configured, skipping treasury grant execution"
                );
            }
        }

        proposal.status = ProposalStatus::Enacted;

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            param_count = proposal.param_keys.len(),
            has_treasury_grant = proposal.treasury_grant.is_some(),
            "✅ Proposal enacted"
        );

        Ok(())
    }

    /// Get proposal by ID
    pub async fn get_proposal(&self, proposal_id: &[u8; 32]) -> Option<GovernanceProposal> {
        let proposals = self.proposals.read().await;
        proposals.get(proposal_id).cloned()
    }

    /// Get votes for a proposal
    pub async fn get_votes(&self, proposal_id: &[u8; 32]) -> Vec<GovernanceVote> {
        let votes = self.votes.read().await;
        votes.get(proposal_id).cloned().unwrap_or_default()
    }

    /// Get receipts for a proposal
    pub async fn get_receipts(&self, proposal_id: &[u8; 32]) -> Vec<GovernanceReceipt> {
        let receipts = self.receipts.read().await;
        receipts.get(proposal_id).cloned().unwrap_or_default()
    }

    /// Get all proposals
    pub async fn get_all_proposals(&self) -> Vec<GovernanceProposal> {
        let proposals = self.proposals.read().await;
        proposals.values().cloned().collect()
    }

    /// Get proposals by status
    pub async fn get_proposals_by_status(&self, status: ProposalStatus) -> Vec<GovernanceProposal> {
        let proposals = self.proposals.read().await;
        proposals
            .values()
            .filter(|p| p.status == status)
            .cloned()
            .collect()
    }

    /// Calculate total eligible voting credits
    ///
    /// Formula per PoUW III §7.1: credits(P) = √min{R(P), Rmax}
    ///
    /// Sums √min{R(P), Rmax} for all participants with R > 0
    async fn calculate_total_eligible_credits(&self) -> f64 {
        let all_scores = self.reputation_tracker.get_all_scores().await;
        let rmax = self.config.rmax;

        let mut total_credits = 0.0;

        for (pubkey, score) in all_scores.iter() {
            let reputation = score.value;

            // Only count participants with positive reputation
            if reputation > 0.0 {
                // Calculate voting credits: √min{R(P), Rmax}
                let capped_reputation = reputation.min(rmax);
                let credits = capped_reputation.sqrt();
                total_credits += credits;

                tracing::trace!(
                    pubkey_prefix = format!("{:x}", &pubkey.as_bytes()[..4].iter().fold(0u32, |acc, &b| (acc << 8) | b as u32)),
                    reputation = reputation,
                    capped = capped_reputation,
                    credits = credits,
                    "Calculated voting credits for participant"
                );
            }
        }

        tracing::info!(
            participant_count = all_scores.len(),
            eligible_count = all_scores.values().filter(|s| s.value > 0.0).count(),
            total_credits = total_credits,
            rmax = rmax,
            "📊 Calculated total eligible voting credits"
        );

        // Return at least 1.0 to avoid division by zero in quorum calculations
        total_credits.max(1.0)
    }

    /// Generate BLS threshold signature for governance receipt
    ///
    /// # Process
    /// 1. Select governance committee using VRF/quorum mechanism
    /// 2. Generate message to sign (proposal_id || quorum_stats || result)
    /// 3. Simulate t-of-n signature collection (in production, distributed across committee)
    /// 4. Aggregate signatures using BLS threshold scheme
    ///
    /// # Returns
    /// Result containing tuple of (optional BLS signature, committee member public keys)
    async fn generate_receipt_signature(
        &self,
        proposal_id: &[u8; 32],
        stats: &crate::types::QuorumStats,
        result: VoteResult,
    ) -> Result<(Option<BLSSignature>, Vec<PublicKey>)> {
        // Check if quorum selector and BLS signer are configured
        let (selector, _signer) = match (&self.quorum_selector, &self.bls_signer) {
            (Some(sel), Some(sig)) => (sel, sig),
            _ => {
                debug!(
                    proposal_id = hex::encode(&proposal_id[..8]),
                    "⚠️ Quorum selector or BLS signer not configured, skipping signature"
                );
                return Ok((None, Vec::new()));
            }
        };

        // Step 1: Select governance committee using VRF
        // Build a QuorumConfig for committee selection
        let quorum_config = adic_quorum::QuorumConfig {
            min_reputation: 100.0,
            members_per_axis: (self.config.committee_size + 2) / 3, // Divide across 3 axes
            total_size: self.config.committee_size,
            max_per_asn: 2,
            max_per_region: 3,
            domain_separator: "ADIC-GOV-Q".to_string(),
            num_axes: 3,
        };

        // Get eligible nodes from reputation tracker with network metadata
        let all_scores = self.reputation_tracker.get_all_scores().await;
        let mut eligible_nodes = Vec::new();

        for (pk, score) in all_scores.iter() {
            if score.value < quorum_config.min_reputation {
                continue;
            }

            // Get network metadata if available
            let (asn, region) = if let Some(ref metadata_registry) = self.network_metadata {
                let net_info = metadata_registry.get_or_default(pk).await;
                (Some(net_info.asn), Some(net_info.region))
            } else {
                // Fallback: derive from public key hash for deterministic diversity
                let pk_hash = blake3::hash(pk.as_bytes());
                let asn = u32::from_be_bytes([pk_hash.as_bytes()[0], pk_hash.as_bytes()[1], pk_hash.as_bytes()[2], pk_hash.as_bytes()[3]]) % 65536;
                let region_code = (pk_hash.as_bytes()[4] % 26) as char;
                (Some(asn), Some(format!("r{}", region_code as u8)))
            };

            eligible_nodes.push(adic_quorum::NodeInfo {
                public_key: *pk,
                reputation: score.value,
                asn,
                region,
                axis_balls: vec![], // p-adic balls computed by QuorumSelector
            });
        }

        let committee = match selector
            .select_committee(
                proposal_id.iter().fold(0u64, |acc, &b| acc.wrapping_add(b as u64)), // Use proposal_id for epoch
                &quorum_config,
                eligible_nodes,
            )
            .await
        {
            Ok(committee) => committee,
            Err(e) => {
                warn!(
                    proposal_id = hex::encode(&proposal_id[..8]),
                    error = %e,
                    "⚠️ Failed to select governance committee"
                );
                return Ok((None, Vec::new()));
            }
        };

        let committee_members: Vec<PublicKey> = committee.members.iter().map(|m| m.public_key).collect();

        debug!(
            proposal_id = hex::encode(&proposal_id[..8]),
            committee_size = committee_members.len(),
            bls_threshold = self.config.bls_threshold,
            "📝 Selected governance committee"
        );

        // Step 2: Build message to sign
        let mut message = Vec::new();
        message.extend_from_slice(proposal_id);
        message.extend_from_slice(&stats.yes.to_le_bytes());
        message.extend_from_slice(&stats.no.to_le_bytes());
        message.extend_from_slice(&stats.abstain.to_le_bytes());
        message.push(result as u8);

        // Step 3 & 4: Sign and aggregate with BLSCoordinator
        // Require BLS coordinator for production governance receipts
        let coordinator = self.bls_coordinator.as_ref().ok_or_else(|| {
            GovernanceError::BLSCoordinatorRequired(
                "BLS coordinator must be configured for governance receipt signatures. \
                 Use .with_bls_coordinator() when constructing GovernanceManager."
            )
        })?;

        // Create signing request ID
        let current_epoch = proposal_id.iter().fold(0u64, |acc, &b| acc.wrapping_add(b as u64));
        let request_id = adic_pouw::SigningRequestId::new(
            "gov-receipt",
            current_epoch,
            proposal_id,
        );

        // Initiate signing request to committee
        match coordinator
            .initiate_signing(
                request_id.clone(),
                message.clone(),
                DST_GOVERNANCE_RECEIPT.to_vec(),
                committee_members.clone(),
            )
            .await
        {
            Ok(_request) => {
                info!(
                    proposal_id = hex::encode(&proposal_id[..8]),
                    request_id = %request_id.0,
                    committee_size = committee_members.len(),
                    threshold = self.config.bls_threshold,
                    "📝 BLS signing request initiated for governance receipt"
                );

                // The signing request will be broadcast to committee members via network layer.
                // Committee member flow:
                // 1. Receive SigningRequest via network protocol
                // 2. Sign message with BLS key share using BLSThresholdSigner
                // 3. Submit signature share via coordinator.submit_share()
                // 4. When threshold (t) shares collected, coordinator aggregates them
                //
                // Signature collection is asynchronous (network protocol handles broadcast).
                // Signature can be retrieved later via coordinator once threshold is reached.

                debug!(
                    proposal_id = hex::encode(&proposal_id[..8]),
                    "⏳ BLS signature collection pending network integration"
                );

                Ok((None, committee_members))
            }
            Err(e) => {
                warn!(
                    proposal_id = hex::encode(&proposal_id[..8]),
                    error = %e,
                    "⚠️ Failed to initiate BLS signing request"
                );
                Ok((None, committee_members))
            }
        }
    }

    /// Get governance configuration parameters
    pub fn config(&self) -> &LifecycleConfig {
        &self.config
    }

    /// Test helper: Set voting_end_timestamp for a proposal
    /// This is needed for tests that want to simulate expired voting periods
    #[doc(hidden)]
    pub async fn test_set_voting_end_timestamp(
        &self,
        proposal_id: &[u8; 32],
        timestamp: DateTime<Utc>,
    ) -> Result<()> {
        let mut proposals = self.proposals.write().await;
        if let Some(proposal) = proposals.get_mut(proposal_id) {
            proposal.voting_end_timestamp = timestamp;
            Ok(())
        } else {
            Err(GovernanceError::ProposalNotFound(hex::encode(&proposal_id[..8])))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ProposalClass;
    use adic_types::PublicKey;

    fn create_test_config() -> LifecycleConfig {
        LifecycleConfig {
            min_proposer_reputation: 100.0,
            voting_duration_secs: 60, // 1 minute for testing
            min_quorum: 0.1,
            rmax: 100_000.0,
            gamma_f1: 2.0,
            gamma_f2: 3.0,
            min_timelock_secs: 10.0,
            committee_size: 7,
            bls_threshold: 5,  // 5 of 7
        }
    }

    #[test]
    fn test_timelock_calculation() {
        let config = create_test_config();
        let manager = ProposalLifecycleManager::new(
            config.clone(),
            Arc::new(ReputationTracker::new(0.9)), // gamma for reputation decay
        );

        // TF1 = 5s, TF2 = 3s
        // Result = max{2*5, 3*3, 10} = max{10, 9, 10} = 10
        let timelock = manager.calculate_timelock(Duration::from_secs(5), Duration::from_secs(3));
        assert_eq!(timelock, Duration::from_secs(10));

        // TF1 = 10s, TF2 = 5s
        // Result = max{2*10, 3*5, 10} = max{20, 15, 10} = 20
        let timelock = manager.calculate_timelock(Duration::from_secs(10), Duration::from_secs(5));
        assert_eq!(timelock, Duration::from_secs(20));

        // TF1 = 2s, TF2 = 5s
        // Result = max{2*2, 3*5, 10} = max{4, 15, 10} = 15
        let timelock = manager.calculate_timelock(Duration::from_secs(2), Duration::from_secs(5));
        assert_eq!(timelock, Duration::from_secs(15));
    }

    #[tokio::test]
    async fn test_proposal_lifecycle() {
        let config = create_test_config();
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));

        // Set proposer reputation
        let proposer_pk = PublicKey::from_bytes([1; 32]);
        rep_tracker.set_reputation(&proposer_pk, 150.0).await;

        let manager = ProposalLifecycleManager::new(config, rep_tracker);

        // Create proposal
        let proposal = GovernanceProposal {
            proposal_id: [1u8; 32],
            class: ProposalClass::Operational,
            proposer_pk,
            param_keys: vec!["k".to_string()],
            new_values: serde_json::json!({"k": 30}),
            axis_changes: None,
            treasury_grant: None,
            enact_epoch: 100,
            rationale_cid: "QmTest".to_string(),
            creation_timestamp: Utc::now(),
            voting_end_timestamp: Utc::now(),
            status: ProposalStatus::Voting,
            tally_yes: 0.0,
            tally_no: 0.0,
            tally_abstain: 0.0,
        };

        // Submit proposal
        let proposal_id = manager.submit_proposal(proposal).await.unwrap();

        // Verify proposal stored
        let stored = manager.get_proposal(&proposal_id).await;
        assert!(stored.is_some());
        assert_eq!(stored.unwrap().status, ProposalStatus::Voting);
    }

    #[tokio::test]
    async fn test_insufficient_reputation() {
        let config = create_test_config();
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));

        // Set proposer reputation BELOW threshold
        let proposer_pk = PublicKey::from_bytes([1; 32]);
        rep_tracker.set_reputation(&proposer_pk, 50.0).await;

        let manager = ProposalLifecycleManager::new(config, rep_tracker);

        let proposal = GovernanceProposal {
            proposal_id: [1u8; 32],
            class: ProposalClass::Operational,
            proposer_pk,
            param_keys: vec!["k".to_string()],
            new_values: serde_json::json!({"k": 30}),
            axis_changes: None,
            treasury_grant: None,
            enact_epoch: 100,
            rationale_cid: "QmTest".to_string(),
            creation_timestamp: Utc::now(),
            voting_end_timestamp: Utc::now(),
            status: ProposalStatus::Voting,
            tally_yes: 0.0,
            tally_no: 0.0,
            tally_abstain: 0.0,
        };

        // Should fail with insufficient reputation
        let result = manager.submit_proposal(proposal).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GovernanceError::InsufficientReputation { .. }
        ));
    }

    #[tokio::test]
    async fn test_parameter_store_integration() {
        use adic_app_common::ParameterStore;

        let config = create_test_config();
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let param_store = Arc::new(ParameterStore::new());

        let proposer_pk = PublicKey::from_bytes([2; 32]);
        rep_tracker.set_reputation(&proposer_pk, 150.0).await;

        let manager = ProposalLifecycleManager::new(config, rep_tracker)
            .with_parameter_store(param_store.clone());

        // Verify initial parameter value
        assert_eq!(param_store.get_u32("k").await.unwrap(), 20);

        // Create proposal to change k from 20 to 35
        let proposal = GovernanceProposal {
            proposal_id: [2u8; 32],
            class: ProposalClass::Operational,
            proposer_pk,
            param_keys: vec!["k".to_string()],
            new_values: serde_json::json!({"k": 35}),
            axis_changes: None,
            treasury_grant: None,
            enact_epoch: 100,
            rationale_cid: "QmTest123".to_string(),
            creation_timestamp: Utc::now(),
            voting_end_timestamp: Utc::now(),
            status: ProposalStatus::Enacting, // Set to Enacting for test
            tally_yes: 0.0,
            tally_no: 0.0,
            tally_abstain: 0.0,
        };

        // Store proposal directly in Enacting state (skipping voting for test)
        {
            let mut proposals = manager.proposals.write().await;
            proposals.insert(proposal.proposal_id, proposal.clone());
        }

        // Enact proposal
        manager.enact_proposal(&proposal.proposal_id).await.unwrap();

        // Verify parameter was updated
        assert_eq!(param_store.get_u32("k").await.unwrap(), 35);

        // Verify proposal status changed to Enacted
        let enacted = manager.get_proposal(&proposal.proposal_id).await.unwrap();
        assert_eq!(enacted.status, ProposalStatus::Enacted);
    }

    #[tokio::test]
    async fn test_parameter_validation_failure() {
        use adic_app_common::ParameterStore;

        let config = create_test_config();
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let param_store = Arc::new(ParameterStore::new());

        let proposer_pk = PublicKey::from_bytes([3; 32]);
        rep_tracker.set_reputation(&proposer_pk, 150.0).await;

        let manager = ProposalLifecycleManager::new(config, rep_tracker)
            .with_parameter_store(param_store.clone());

        // Create proposal with invalid parameter value (p=11 not in allowed set)
        let proposal = GovernanceProposal {
            proposal_id: [3u8; 32],
            class: ProposalClass::Constitutional,
            proposer_pk,
            param_keys: vec!["p".to_string()],
            new_values: serde_json::json!({"p": 11}), // Invalid: p must be 3, 5, or 7
            axis_changes: None,
            treasury_grant: None,
            enact_epoch: 100,
            rationale_cid: "QmTest456".to_string(),
            creation_timestamp: Utc::now(),
            voting_end_timestamp: Utc::now(),
            status: ProposalStatus::Enacting,
            tally_yes: 0.0,
            tally_no: 0.0,
            tally_abstain: 0.0,
        };

        // Store proposal
        {
            let mut proposals = manager.proposals.write().await;
            proposals.insert(proposal.proposal_id, proposal.clone());
        }

        // Attempt to enact - should fail validation
        let result = manager.enact_proposal(&proposal.proposal_id).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GovernanceError::ParameterUpdateFailed { .. }
        ));

        // Verify parameter was NOT changed
        assert_eq!(param_store.get_u8("p").await.unwrap(), 3);
    }

    #[tokio::test]
    async fn test_integrated_finality_timelock_enactment() {
        let config = create_test_config();
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));

        let proposer_pk = PublicKey::from_bytes([3; 32]);
        rep_tracker.set_reputation(&proposer_pk, 200.0).await;

        // Create manager WITHOUT finality adapter (test mode)
        let manager = ProposalLifecycleManager::new(config, rep_tracker);

        // Create and submit proposal
        let proposal = GovernanceProposal {
            proposal_id: [3u8; 32],
            class: ProposalClass::Operational,
            proposer_pk,
            param_keys: vec!["k".to_string()],
            new_values: serde_json::json!({"k": 25}),
            axis_changes: None,
            treasury_grant: None,
            enact_epoch: 120,
            rationale_cid: "QmTest3".to_string(),
            creation_timestamp: Utc::now(),
            voting_end_timestamp: Utc::now(),
            status: ProposalStatus::Succeeded, // Start at Succeeded for this test
            tally_yes: 100.0,
            tally_no: 0.0,
            tally_abstain: 0.0,
        };

        let proposal_id = manager.submit_proposal(proposal.clone()).await.unwrap();

        // Manually set status to Succeeded since we're skipping voting
        {
            let mut proposals = manager.proposals.write().await;
            proposals.get_mut(&proposal_id).unwrap().status = ProposalStatus::Succeeded;
        }

        // Test integrated finality → timelock → enactment workflow
        // In test mode (no finality adapter), this will use default finality times (1s, 1s)
        let timelock = manager
            .finalize_and_prepare_enactment(&proposal_id, Duration::from_secs(10))
            .await
            .unwrap();

        // Verify timelock was calculated
        // With γ₁=2.0, γ₂=3.0, T_min=10.0, and test finality times (1s, 1s):
        // T_enact = max{2*1, 3*1, 10} = max{2, 3, 10} = 10 seconds
        assert_eq!(timelock, Duration::from_secs(10));

        // Verify proposal status transitioned to Enacting
        let stored = manager.get_proposal(&proposal_id).await.unwrap();
        assert_eq!(stored.status, ProposalStatus::Enacting);
    }

    #[tokio::test]
    async fn test_reputation_based_voting_credits() {
        let config = create_test_config();
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));

        // Set up test participants with various reputation levels
        let participant1 = PublicKey::from_bytes([1; 32]);
        let participant2 = PublicKey::from_bytes([2; 32]);
        let participant3 = PublicKey::from_bytes([3; 32]);
        let participant4 = PublicKey::from_bytes([4; 32]);
        let participant5 = PublicKey::from_bytes([5; 32]);

        // Participant 1: R=100 (below Rmax)
        rep_tracker.set_reputation(&participant1, 100.0).await;
        // Participant 2: R=10000 (below Rmax)
        rep_tracker.set_reputation(&participant2, 10_000.0).await;
        // Participant 3: R=100_000 (equals Rmax, should be capped)
        rep_tracker.set_reputation(&participant3, 100_000.0).await;
        // Participant 4: R=200_000 (exceeds Rmax, should be capped)
        rep_tracker.set_reputation(&participant4, 200_000.0).await;
        // Participant 5: R=0 (should not contribute)
        rep_tracker.set_reputation(&participant5, 0.0).await;

        let manager = ProposalLifecycleManager::new(config, rep_tracker);

        // Calculate total eligible credits
        let total_credits = manager.calculate_total_eligible_credits().await;

        // Expected credits:
        // Participant 1: √min{100, 100_000} = √100 = 10.0
        // Participant 2: √min{10_000, 100_000} = √10_000 = 100.0
        // Participant 3: √min{100_000, 100_000} = √100_000 = 316.227766...
        // Participant 4: √min{200_000, 100_000} = √100_000 = 316.227766... (capped)
        // Participant 5: Not counted (R=0)
        //
        // Total = 10.0 + 100.0 + 316.227766 + 316.227766 = 742.455532

        let expected = 100.0_f64.sqrt()
            + 10_000.0_f64.sqrt()
            + 100_000.0_f64.sqrt()
            + 100_000.0_f64.sqrt(); // Participant 4 capped at Rmax

        assert!((total_credits - expected).abs() < 0.01,
            "Expected {} credits, got {}", expected, total_credits);
    }

    #[tokio::test]
    async fn test_quorum_with_reputation_integration() {
        let config = create_test_config();
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));

        // Set up 10 participants with R=10,000 each
        // Total credits = 10 × √10,000 = 10 × 100 = 1,000
        for i in 0..10 {
            let pk = PublicKey::from_bytes([i; 32]);
            rep_tracker.set_reputation(&pk, 10_000.0).await;
        }

        let manager = ProposalLifecycleManager::new(config.clone(), rep_tracker.clone());

        // Create a proposal
        let proposer_pk = PublicKey::from_bytes([100; 32]);
        rep_tracker.set_reputation(&proposer_pk, 200.0).await;

        let proposal = GovernanceProposal {
            proposal_id: [1u8; 32],
            class: ProposalClass::Operational,
            proposer_pk,
            param_keys: vec!["k".to_string()],
            new_values: serde_json::json!({"k": 25}),
            axis_changes: None,
            treasury_grant: None,
            enact_epoch: 100,
            rationale_cid: "QmTest".to_string(),
            creation_timestamp: Utc::now(),
            voting_end_timestamp: Utc::now() + chrono::Duration::seconds(60),
            status: ProposalStatus::Voting,
            tally_yes: 0.0,
            tally_no: 0.0,
            tally_abstain: 0.0,
        };

        let proposal_id = manager.submit_proposal(proposal).await.unwrap();

        // Cast votes from 3 participants (300 credits = 30% participation)
        // With min_quorum = 0.1 (10%), this should pass quorum
        for i in 0..3 {
            let voter_pk = PublicKey::from_bytes([i; 32]);
            let vote = GovernanceVote::new(
                proposal_id,
                voter_pk,
                100.0, // √10,000
                crate::types::Ballot::Yes,
            );
            manager.cast_vote(vote).await.unwrap();
        }

        // Manually end the voting period by updating the timestamp to the past
        {
            let mut proposals = manager.proposals.write().await;
            if let Some(proposal) = proposals.get_mut(&proposal_id) {
                proposal.voting_end_timestamp = Utc::now() - chrono::Duration::seconds(1);
            }
        }

        // End voting and tally
        manager.finalize_voting(&proposal_id).await.unwrap();

        // Check that proposal succeeded (passed quorum and threshold)
        let stored = manager.get_proposal(&proposal_id).await.unwrap();
        assert_eq!(stored.status, ProposalStatus::Succeeded);

        println!("✅ Quorum passed with reputation-based credits");
        println!("   - Total eligible credits: ~1,014 (10 participants + proposer)");
        println!("   - Votes cast: 300 credits (3 participants)");
        println!("   - Participation rate: ~29.6%");
        println!("   - Required quorum: 10%");
    }
}
