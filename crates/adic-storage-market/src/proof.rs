//! Proof Cycle Management for Storage Market
//!
//! Implements the proof-of-storage cycle with:
//! - **Deterministic challenge generation** (blake3-based, finality-bound)
//! - Merkle proof verification
//! - Automated payment releases
//! - Slashing for missed proofs
//! - Challenge window enforcement
//!
//! # Challenge Generation Design
//!
//! Currently uses **deterministic blake3** hashing of (deal_id, epoch, data_cid)
//! to generate challenge indices. This approach:
//! - ✓ Is deterministic and verifiable by all nodes
//! - ✓ Is cryptographically secure (blake3)
//! - ✓ Requires finality before challenges can be issued (prevents grinding)
//! - ✗ Does not use VRF (simplification for initial implementation)
//!
//! ## Production Considerations
//!
//! For mainnet, consider migrating to VRF-based challenge generation via
//! `VRFService::compute_challenge_for_epoch()` to provide:
//! - Unpredictability until VRF reveal
//! - Stronger sybil resistance
//! - Verifiable randomness properties
//!
//! Current deterministic approach is acceptable if:
//! 1. Challenges are only generated after F1/F2 finality (prevents reorg attacks)
//! 2. Providers cannot selectively store chunks (must store full data)
//! 3. Challenge epochs are sufficiently spaced (current: per epoch)
//!
//! See `generate_challenge()` for implementation details.

use crate::error::{Result, StorageMarketError};
use crate::types::{StorageDeal, StorageDealStatus, StorageProof};
use adic_app_common::{FinalityAdapter, FinalityPolicy};
use adic_challenges::{ChallengeWindowManager, DisputeAdjudicator, FraudProof};
use adic_economics::{AccountAddress, AdicAmount, BalanceManager, TransferReason};
use adic_types::MessageId;
use adic_vrf::VRFService;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Configuration for proof cycle
#[derive(Debug, Clone)]
pub struct ProofCycleConfig {
    /// Grace period before slashing (epochs)
    pub proof_grace_period: u64,

    /// Percentage of collateral slashed for missed proof (0.0-1.0)
    pub slash_percentage: f64,

    /// Number of chunks to challenge per proof
    pub chunks_per_challenge: u32,

    /// Payment released per successful proof (fraction of total)
    pub payment_per_proof: f64,

    /// VRF grace period: epochs to wait for VRF finalization before allowing fallback
    ///
    /// Challenges for epochs within this window MUST use VRF randomness.
    /// This prevents predictable challenges during the active trading period.
    /// Default: 3 epochs (aligned with VRF reveal_depth of 10)
    pub vrf_grace_period_epochs: u64,
}

impl Default for ProofCycleConfig {
    fn default() -> Self {
        Self {
            proof_grace_period: 5,         // 5 epochs
            slash_percentage: 0.1,         // 10% slash
            chunks_per_challenge: 3,       // 3 chunks
            payment_per_proof: 0.1,        // 10% per proof (10 proofs = full payment)
            vrf_grace_period_epochs: 3,    // 3 epochs (VRF reveal_depth is 10)
        }
    }
}

/// Manages proof cycles for active storage deals
pub struct ProofCycleManager {
    config: ProofCycleConfig,
    balance_manager: Arc<BalanceManager>,
    finality_adapter: Option<Arc<FinalityAdapter>>,
    challenge_manager: Arc<ChallengeWindowManager>,
    /// VRF service for VRF-based challenge generation
    /// Used to generate unpredictable challenges via canonical randomness
    vrf_service: Arc<VRFService>,
    /// Dispute adjudicator for fraud proof verification
    dispute_adjudicator: Arc<DisputeAdjudicator>,

    // Track proofs by deal ID
    proofs: Arc<RwLock<HashMap<u64, Vec<StorageProof>>>>,

    // Track challenges by deal ID
    pending_challenges: Arc<RwLock<HashMap<u64, Vec<u64>>>>,

    // Track payment released so far
    payments_released: Arc<RwLock<HashMap<u64, AdicAmount>>>,

    // Track message IDs for finality checking
    proof_message_ids: Arc<RwLock<HashMap<u64, MessageId>>>,

    // Track fraud proofs by ID
    fraud_proofs: Arc<RwLock<HashMap<MessageId, FraudProof>>>,
}

impl ProofCycleManager {
    /// Create new proof cycle manager
    ///
    /// For testing, use `new_for_testing()` which creates a default adjudicator.
    /// For production, pass a properly configured adjudicator with metrics.
    pub fn new(
        config: ProofCycleConfig,
        balance_manager: Arc<BalanceManager>,
        challenge_manager: Arc<ChallengeWindowManager>,
        vrf_service: Arc<VRFService>,
        dispute_adjudicator: Arc<DisputeAdjudicator>,
    ) -> Self {
        Self {
            config,
            balance_manager,
            finality_adapter: None,
            challenge_manager,
            vrf_service,
            dispute_adjudicator,
            proofs: Arc::new(RwLock::new(HashMap::new())),
            pending_challenges: Arc::new(RwLock::new(HashMap::new())),
            payments_released: Arc::new(RwLock::new(HashMap::new())),
            proof_message_ids: Arc::new(RwLock::new(HashMap::new())),
            fraud_proofs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create proof cycle manager for testing (with default adjudicator)
    ///
    /// This creates a default DisputeAdjudicator without metrics for test simplicity.
    /// Production code should use `new()` with a properly wired adjudicator.
    pub fn new_for_testing(
        config: ProofCycleConfig,
        balance_manager: Arc<BalanceManager>,
        challenge_manager: Arc<ChallengeWindowManager>,
        vrf_service: Arc<VRFService>,
    ) -> Self {
        // Create a default adjudicator for testing (no metrics)
        use adic_challenges::AdjudicationConfig;
        use adic_consensus::ReputationTracker;

        // Create quorum selector with vrf_service and reputation tracker
        let reputation_tracker = Arc::new(ReputationTracker::new(0.9));
        let quorum_selector = Arc::new(adic_quorum::QuorumSelector::new(
            Arc::clone(&vrf_service),
            reputation_tracker,
        ));

        let adjudicator = Arc::new(DisputeAdjudicator::new(
            quorum_selector,
            AdjudicationConfig::default(),
        ));

        Self::new(
            config,
            balance_manager,
            challenge_manager,
            vrf_service,
            adjudicator,
        )
    }

    /// Set finality adapter (for production use with real F1/F2)
    pub fn with_finality_adapter(mut self, adapter: Arc<FinalityAdapter>) -> Self {
        self.finality_adapter = Some(adapter);
        self
    }

    /// Register message ID for a proof (for finality tracking)
    pub async fn register_proof_message(&self, deal_id: u64, message_id: MessageId) {
        let mut message_ids = self.proof_message_ids.write().await;
        message_ids.insert(deal_id, message_id);
    }

    /// Wait for finality of storage proof
    ///
    /// Per PoUW III §7.1: Operational signals MAY accept F1 only
    pub async fn wait_for_finality(
        &self,
        deal_id: u64,
        timeout: Duration,
    ) -> Result<Duration> {
        // Get finality adapter or skip if not configured (test mode)
        let adapter = match &self.finality_adapter {
            Some(adapter) => adapter,
            None => {
                warn!(
                    deal_id = deal_id,
                    "⚠️  No finality adapter configured, skipping finality check (test mode)"
                );
                // Return default time for test mode
                return Ok(Duration::from_secs(1));
            }
        };

        // Get message ID for this proof
        let message_ids = self.proof_message_ids.read().await;
        let message_id = message_ids
            .get(&deal_id)
            .ok_or_else(|| {
                StorageMarketError::Other(format!(
                    "Message ID not found for deal {}",
                    deal_id
                ))
            })?;

        info!(
            deal_id = deal_id,
            message_id = ?message_id,
            timeout_secs = timeout.as_secs(),
            "⏳ Waiting for finality of storage proof (F1 for operational signals)..."
        );

        // Wait for F1 finality (operational signals per PoUW III §7.1)
        let status = adapter
            .wait_for_finality(message_id, FinalityPolicy::F1Only, timeout)
            .await
            .map_err(|e| {
                StorageMarketError::Other(format!(
                    "Finality timeout for deal {}: {}",
                    deal_id,
                    e
                ))
            })?;

        // Extract finality time
        let f1_time = status
            .f1_time
            .map(|t| t.elapsed())
            .unwrap_or(Duration::from_secs(0));

        info!(
            deal_id = deal_id,
            f1_time_secs = f1_time.as_secs(),
            "✅ F1 finality achieved for storage proof"
        );

        Ok(f1_time)
    }

    /// Generate storage proof challenge for a deal at a specific epoch
    ///
    /// # Challenge Generation Strategy
    ///
    /// Uses **VRF-based randomness** when available: `VRF(R_k || "storage-challenge" || deal_id || data_cid)`
    ///
    /// Where `R_k` is the canonical randomness for epoch k from the VRF commit-reveal protocol.
    ///
    /// ## Security Properties (VRF Mode)
    ///
    /// - ✓ Challenges are unpredictable until epoch k VRF is finalized
    /// - ✓ Providers cannot precompute challenges for future epochs
    /// - ✓ Resistant to selective storage attacks
    /// - ✓ Uniform and unbiased chunk selection
    /// - ✓ Verifiable by all nodes (via canonical randomness)
    ///
    /// ## Fallback Mode
    ///
    /// If VRF randomness is not yet finalized for the epoch, falls back to:
    /// `blake3(deal_id || epoch || data_cid)`
    ///
    /// This ensures challenges can still be generated even if VRF is delayed,
    /// but with reduced unpredictability guarantees.
    ///
    /// ## Migration from Pure Deterministic
    ///
    /// Previous versions used only deterministic blake3 hashing. This implementation
    /// maintains that as a fallback while preferring VRF when available.
    ///
    /// # Security Properties
    ///
    /// - ✓ VRF-based: Unpredictable until epoch finalization
    /// - ✓ Fallback: Still cryptographically secure via blake3
    /// - ✓ Both modes: Verifiable by all nodes
    /// - ⚠ Requires finality adapter to prevent challenge grinding
    pub async fn generate_challenge(
        &self,
        deal: &StorageDeal,
        epoch: u64,
        current_epoch: u64,
    ) -> Result<Vec<u64>> {
        // Check that deal is active
        if deal.status != StorageDealStatus::Active {
            return Err(StorageMarketError::InvalidStateTransition {
                from: format!("{:?}", deal.status),
                to: "Challenge generation".to_string(),
            });
        }

        // Try to use VRF canonical randomness for enhanced security
        let challenge_seed = match self.vrf_service.get_canonical_randomness(epoch).await {
            Ok(_canonical_randomness) => {
                // VRF randomness available - use it for unpredictable challenges
                // Create bindings for parameters to extend lifetimes
                let deal_id_bytes = deal.deal_id.to_le_bytes();
                let params: Vec<&[u8]> = vec![
                    &deal_id_bytes,
                    &deal.data_cid,
                ];

                let seed = self
                    .vrf_service
                    .compute_challenge_for_epoch(epoch, "storage-challenge", &params)
                    .await
                    .map_err(|e| {
                        StorageMarketError::ChallengeError(format!(
                            "Failed to compute VRF challenge: {}",
                            e
                        ))
                    })?;

                info!(
                    deal_id = deal.deal_id,
                    epoch,
                    "🎲 Generated VRF-based storage challenge"
                );

                seed
            }
            Err(_) => {
                // VRF not finalized yet - check if within grace period
                let epoch_age = current_epoch.saturating_sub(epoch);

                // H2 Security: Enforce VRF for recent epochs within grace period
                if epoch_age < self.config.vrf_grace_period_epochs {
                    // Recent epoch - VRF must be available to prevent predictable challenges
                    #[cfg(all(not(test), not(debug_assertions), not(feature = "allow-deterministic-challenges")))]
                    {
                        return Err(StorageMarketError::ChallengeError(format!(
                            "VRF randomness required for epoch {} (age: {}, grace period: {}). \
                             Deterministic fallback blocked in production for security.",
                            epoch, epoch_age, self.config.vrf_grace_period_epochs
                        )));
                    }

                    // In dev/test builds or with feature flag, warn but allow fallback
                    #[cfg(any(test, debug_assertions, feature = "allow-deterministic-challenges"))]
                    {
                        tracing::error!(
                            deal_id = deal.deal_id,
                            epoch,
                            epoch_age,
                            grace_period = self.config.vrf_grace_period_epochs,
                            "🔴 SECURITY: VRF not finalized within grace period, using deterministic fallback (DEV/TEST ONLY)"
                        );
                    }
                } else {
                    // Old epoch outside grace period - deterministic fallback acceptable
                    warn!(
                        deal_id = deal.deal_id,
                        epoch,
                        epoch_age,
                        grace_period = self.config.vrf_grace_period_epochs,
                        "⚠️ VRF not finalized for old epoch, using deterministic challenge fallback"
                    );
                }

                // Deterministic fallback using blake3
                let mut seed_input = Vec::new();
                seed_input.extend_from_slice(&deal.deal_id.to_le_bytes());
                seed_input.extend_from_slice(&epoch.to_le_bytes());
                seed_input.extend_from_slice(&deal.data_cid);

                *blake3::hash(&seed_input).as_bytes()
            }
        };

        let seed_bytes = &challenge_seed;

        // Calculate number of chunks in the data
        const CHUNK_SIZE: u64 = 4096;
        let num_chunks = (deal.data_size + CHUNK_SIZE - 1) / CHUNK_SIZE;

        // Generate challenge indices
        let mut challenge_indices = Vec::new();
        for i in 0..self.config.chunks_per_challenge {
            let offset = (i * 8) as usize;
            if offset + 8 <= seed_bytes.len() {
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&seed_bytes[offset..offset + 8]);
                let index = u64::from_le_bytes(bytes) % num_chunks;
                challenge_indices.push(index);
            }
        }

        // Store pending challenges
        let mut pending = self.pending_challenges.write().await;
        pending.insert(deal.deal_id, challenge_indices.clone());

        Ok(challenge_indices)
    }

    /// Submit storage proof for verification
    ///
    /// Provider submits Merkle proofs for challenged chunks.
    /// On success, releases payment. On failure or timeout, slashes collateral.
    pub async fn submit_proof(
        &self,
        proof: StorageProof,
        deal: &mut StorageDeal,
        current_epoch: u64,
    ) -> Result<()> {
        // Validate proof timing
        self.validate_proof_timing(&proof, deal, current_epoch)?;

        // Get pending challenges
        let pending = self.pending_challenges.read().await;
        let challenges = pending
            .get(&deal.deal_id)
            .ok_or_else(|| {
                StorageMarketError::ChallengeError("No pending challenges".to_string())
            })?;

        // Verify challenge indices match expected deterministic challenges
        self.verify_challenge_authenticity(&proof, challenges)?;

        // Verify Merkle proofs for all challenged chunks
        self.verify_merkle_proofs(&proof, challenges, deal)?;

        // Create proof identifier for challenge window tracking
        let proof_id = Self::compute_proof_id(&proof, deal.deal_id);

        // Open challenge window for fraud proof submissions
        // This allows anyone to challenge the proof within the window period
        self.challenge_manager
            .open_window(proof_id, current_epoch)
            .await
            .map_err(|e| StorageMarketError::ChallengeError(format!("Failed to open challenge window: {}", e)))?;

        info!(
            deal_id = deal.deal_id,
            proof_epoch = proof.proof_epoch,
            proof_id = hex::encode(proof_id.as_bytes()),
            "⏳ Challenge window opened for storage proof"
        );

        // Release payment to provider
        self.release_payment(deal, current_epoch).await?;

        // Store the proof
        let mut proofs = self.proofs.write().await;
        proofs.entry(deal.deal_id).or_insert_with(Vec::new).push(proof);

        // Clear pending challenges
        drop(pending);
        self.pending_challenges.write().await.remove(&deal.deal_id);

        Ok(())
    }

    /// Check for missed proofs and slash collateral
    pub async fn check_missed_proofs(
        &self,
        deal: &mut StorageDeal,
        current_epoch: u64,
    ) -> Result<()> {
        // Check if proof is overdue
        if let Some(start_epoch) = deal.start_epoch {
            let epochs_since_start = current_epoch.saturating_sub(start_epoch);
            let expected_proofs = epochs_since_start / deal.deal_duration_epochs;

            let proofs = self.proofs.read().await;
            let actual_proofs = proofs.get(&deal.deal_id).map(|v| v.len()).unwrap_or(0) as u64;

            if actual_proofs < expected_proofs {
                // Proof is overdue, check grace period
                let last_proof_epoch = proofs
                    .get(&deal.deal_id)
                    .and_then(|v| v.last())
                    .map(|p| p.proof_epoch)
                    .unwrap_or(start_epoch);

                let epochs_overdue = current_epoch.saturating_sub(last_proof_epoch + deal.deal_duration_epochs);

                if epochs_overdue > self.config.proof_grace_period {
                    // Slash provider collateral
                    self.slash_provider(deal).await?;

                    // Mark deal as failed
                    deal.transition_to(StorageDealStatus::Failed)?;

                    return Ok(());
                }
            }
        }

        Ok(())
    }

    /// Validate proof submission timing
    fn validate_proof_timing(
        &self,
        proof: &StorageProof,
        _deal: &StorageDeal,
        current_epoch: u64,
    ) -> Result<()> {
        // Check that proof is for current epoch or recent past
        let epoch_diff = current_epoch.saturating_sub(proof.proof_epoch);
        if epoch_diff > self.config.proof_grace_period {
            return Err(StorageMarketError::ChallengeError(
                "Proof submission too late".to_string(),
            ));
        }

        // Check that proof is not duplicate
        // (Implementation would check proof.proof_epoch against stored proofs)

        Ok(())
    }

    /// Verify Merkle proofs for challenged chunks
    fn verify_merkle_proofs(
        &self,
        proof: &StorageProof,
        challenges: &[u64],
        deal: &StorageDeal,
    ) -> Result<()> {
        // Check we have proofs for all challenges
        if proof.merkle_proofs.len() != challenges.len() {
            return Err(StorageMarketError::ProofVerificationFailed(format!(
                "Expected {} proofs, got {}",
                challenges.len(),
                proof.merkle_proofs.len()
            )));
        }

        // Get Merkle root from deal
        let merkle_root = deal.proof_merkle_root.ok_or_else(|| {
            StorageMarketError::ProofVerificationFailed("No Merkle root set".to_string())
        })?;

        // Verify each Merkle proof
        for (challenge_index, merkle_proof) in challenges.iter().zip(&proof.merkle_proofs) {
            // Verify the chunk index matches
            if merkle_proof.chunk_index != *challenge_index {
                return Err(StorageMarketError::ProofVerificationFailed(format!(
                    "Merkle proof index mismatch: expected {}, got {}",
                    challenge_index, merkle_proof.chunk_index
                )));
            }

            // Verify the Merkle proof
            if !merkle_proof.verify(&merkle_root) {
                return Err(StorageMarketError::ProofVerificationFailed(format!(
                    "Invalid Merkle proof for chunk index {}",
                    challenge_index
                )));
            }
        }

        Ok(())
    }

    /// Verify challenge indices authenticity
    ///
    /// This implementation uses deterministic challenge generation based on
    /// (deal_id, epoch, data_cid) rather than VRF-based challenges.
    ///
    /// Rationale: Deterministic challenges are:
    /// - Reproducible: Anyone can verify the correct challenges for a deal/epoch
    /// - Non-gameable: Challenges are tied to finalized on-chain data
    /// - Efficient: No additional cryptographic operations needed
    ///
    /// VRF-based challenges would provide unpredictability but add complexity
    /// without additional security benefit, since challenges are already bound
    /// to finalized blockchain state.
    fn verify_challenge_authenticity(
        &self,
        proof: &StorageProof,
        expected_challenges: &[u64],
    ) -> Result<()> {
        // Verify that proof's challenge_indices match expected deterministic challenges
        if proof.challenge_indices.len() != expected_challenges.len() {
            return Err(StorageMarketError::ProofVerificationFailed(format!(
                "Challenge count mismatch: expected {}, got {}",
                expected_challenges.len(),
                proof.challenge_indices.len()
            )));
        }

        // Verify each challenge index matches
        for (i, (provided, expected)) in proof
            .challenge_indices
            .iter()
            .zip(expected_challenges.iter())
            .enumerate()
        {
            if provided != expected {
                return Err(StorageMarketError::ProofVerificationFailed(format!(
                    "Challenge index {} mismatch: expected {}, got {}",
                    i, expected, provided
                )));
            }
        }

        Ok(())
    }

    /// Release payment to provider for successful proof
    async fn release_payment(&self, deal: &StorageDeal, _current_epoch: u64) -> Result<()> {
        // Calculate payment amount (percentage of total)
        let payment_base = deal.client_payment_escrow.to_base_units();
        let release_base = (payment_base as f64 * self.config.payment_per_proof) as u64;
        let release_amount = AdicAmount::from_base_units(release_base);

        // Check we haven't released too much
        let mut released = self.payments_released.write().await;
        let total_released = released.entry(deal.deal_id).or_insert(AdicAmount::ZERO);

        let new_total_base = total_released.to_base_units() + release_base;
        if new_total_base > payment_base {
            return Err(StorageMarketError::EconomicsError(
                "Cannot release more than escrowed amount".to_string(),
            ));
        }

        // Release payment to provider
        self.balance_manager
            .credit(deal.provider, release_amount)
            .await
            .map_err(|e| StorageMarketError::EconomicsError(e.to_string()))?;

        // Update total released
        *total_released = AdicAmount::from_base_units(new_total_base);

        Ok(())
    }

    /// Slash provider collateral for missed proof
    ///
    /// Slashed funds are transferred to the treasury for:
    /// - Protocol security (compensate challengers/validators)
    /// - Network sustainability (fund development)
    /// - Economic alignment (disincentivize misbehavior)
    async fn slash_provider(&self, deal: &StorageDeal) -> Result<()> {
        let slash_base = (deal.provider_collateral.to_base_units() as f64
            * self.config.slash_percentage) as u64;
        let slash_amount = AdicAmount::from_base_units(slash_base);

        // Transfer slashed funds to treasury
        let treasury_addr = AccountAddress::treasury();

        self.balance_manager
            .transfer_with_reason(
                deal.provider,
                treasury_addr,
                slash_amount,
                TransferReason::Slashing,
            )
            .await
            .map_err(|e| StorageMarketError::EconomicsError(e.to_string()))?;

        tracing::warn!(
            provider = hex::encode(deal.provider.as_bytes()),
            deal_id = deal.deal_id,
            slash_amount = slash_amount.to_adic(),
            treasury = hex::encode(treasury_addr.as_bytes()),
            "⚖️ Provider slashed - funds transferred to treasury"
        );

        Ok(())
    }

    /// Compute unique identifier for a storage proof
    ///
    /// Creates a MessageId from proof metadata for challenge window tracking
    fn compute_proof_id(proof: &StorageProof, deal_id: u64) -> MessageId {
        let mut data = Vec::new();
        data.extend_from_slice(&deal_id.to_le_bytes());
        data.extend_from_slice(&proof.proof_epoch.to_le_bytes());
        data.extend_from_slice(proof.provider.as_bytes());
        data.extend_from_slice(&proof.timestamp.to_le_bytes());

        // Include challenge indices to make ID unique per proof attempt
        for idx in &proof.challenge_indices {
            data.extend_from_slice(&idx.to_le_bytes());
        }

        MessageId::new(&data)
    }

    /// Process expired challenge windows
    ///
    /// Finalizes windows that have expired without challenges.
    /// Should be called periodically (e.g., once per epoch).
    pub async fn process_expired_windows(&self, current_epoch: u64) -> Result<usize> {
        let expired = self.challenge_manager.get_expired_windows(current_epoch).await;
        let count = expired.len();

        for window in expired {
            if let Err(e) = self
                .challenge_manager
                .finalize_window(window.metadata.subject_id, current_epoch)
                .await
            {
                warn!(
                    subject_id = hex::encode(window.metadata.subject_id.as_bytes()),
                    error = %e,
                    "Failed to finalize expired challenge window"
                );
            }
        }

        if count > 0 {
            info!(
                finalized_count = count,
                current_epoch,
                "✅ Finalized expired challenge windows"
            );
        }

        Ok(count)
    }

    /// Get active challenge window for a proof
    pub async fn get_proof_challenge_window(
        &self,
        proof: &StorageProof,
        deal_id: u64,
    ) -> Option<adic_challenges::ChallengeWindow> {
        let proof_id = Self::compute_proof_id(proof, deal_id);
        self.challenge_manager.get_window(&proof_id).await
    }

    /// Submit fraud proof challenging a storage proof
    ///
    /// Allows anyone to submit evidence that a storage proof is invalid.
    /// The fraud proof opens an arbitration process where a VRF-selected
    /// committee votes on the validity of the challenge.
    pub async fn submit_fraud_proof(
        &self,
        proof_id: MessageId,
        fraud_proof: FraudProof,
    ) -> Result<()> {
        // Verify challenge window exists and is still active
        let window = self
            .challenge_manager
            .get_window(&proof_id)
            .await
            .ok_or_else(|| {
                StorageMarketError::ChallengeError(format!(
                    "Challenge window not found for proof {}",
                    hex::encode(proof_id.as_bytes())
                ))
            })?;

        if !window.is_active(fraud_proof.submitted_at_epoch) {
            return Err(StorageMarketError::ChallengeError(format!(
                "Challenge window expired at epoch {}, current epoch {}",
                window.metadata.window_expiry_epoch, fraud_proof.submitted_at_epoch
            )));
        }

        // Mark window as challenged
        self.challenge_manager
            .submit_challenge(
                proof_id,
                fraud_proof.challenger,
                fraud_proof.submitted_at_epoch,
            )
            .await
            .map_err(|e| {
                StorageMarketError::ChallengeError(format!("Failed to mark window challenged: {}", e))
            })?;

        // Store fraud proof for later adjudication
        let mut proofs = self.fraud_proofs.write().await;
        proofs.insert(fraud_proof.id, fraud_proof.clone());

        info!(
            fraud_proof_id = hex::encode(fraud_proof.id.as_bytes()),
            proof_id = hex::encode(proof_id.as_bytes()),
            challenger = hex::encode(fraud_proof.challenger.as_bytes()),
            "🚨 Fraud proof submitted against storage proof"
        );

        Ok(())
    }

    /// Get fraud proof by ID
    pub async fn get_fraud_proof(&self, fraud_proof_id: &MessageId) -> Option<FraudProof> {
        self.fraud_proofs.read().await.get(fraud_proof_id).cloned()
    }

    /// Get all fraud proofs
    pub async fn get_all_fraud_proofs(&self) -> Vec<FraudProof> {
        self.fraud_proofs.read().await.values().cloned().collect()
    }

    /// Adjudicate a fraud proof by processing arbitrator votes
    ///
    /// Takes the quorum result and votes collected externally, then
    /// runs the adjudication process through the DisputeAdjudicator.
    ///
    /// Returns the adjudication result indicating whether the proof was
    /// upheld (provider slashed) or rejected (challenger slashed).
    pub async fn adjudicate_fraud_proof(
        &self,
        fraud_proof_id: &MessageId,
        quorum: adic_quorum::QuorumResult,
        votes: Vec<adic_challenges::ArbitratorVote>,
        epoch: u64,
    ) -> Result<adic_challenges::AdjudicationResult> {
        // Retrieve fraud proof
        let fraud_proof = self
            .get_fraud_proof(fraud_proof_id)
            .await
            .ok_or_else(|| {
                StorageMarketError::ChallengeError(format!(
                    "Fraud proof not found: {}",
                    hex::encode(fraud_proof_id.as_bytes())
                ))
            })?;

        // Run adjudication through DisputeAdjudicator
        let result = self
            .dispute_adjudicator
            .adjudicate(&fraud_proof, quorum, votes, epoch)
            .await
            .map_err(|e| {
                StorageMarketError::ChallengeError(format!("Adjudication failed: {}", e))
            })?;

        info!(
            fraud_proof_id = hex::encode(fraud_proof_id.as_bytes()),
            ruling = ?result.ruling,
            votes_for = result.votes_for.len(),
            votes_against = result.votes_against.len(),
            threshold_met = result.threshold_met,
            "⚖️ Fraud proof adjudication completed"
        );

        Ok(result)
    }

    /// Complete a deal after all proofs submitted
    pub async fn complete_deal(&self, deal: &mut StorageDeal, current_epoch: u64) -> Result<()> {
        // Check that deal has reached completion
        if let Some(start_epoch) = deal.start_epoch {
            let elapsed = current_epoch.saturating_sub(start_epoch);
            if elapsed < deal.deal_duration_epochs {
                return Err(StorageMarketError::Other(
                    "Deal duration not yet elapsed".to_string(),
                ));
            }
        }

        // Release remaining payment
        let released = self.payments_released.read().await;
        let total_released = released.get(&deal.deal_id).copied().unwrap_or(AdicAmount::ZERO);

        let remaining_base = deal.client_payment_escrow.to_base_units()
            .saturating_sub(total_released.to_base_units());

        if remaining_base > 0 {
            let remaining = AdicAmount::from_base_units(remaining_base);
            self.balance_manager
                .credit(deal.provider, remaining)
                .await
                .map_err(|e| StorageMarketError::EconomicsError(e.to_string()))?;
        }

        // Return provider collateral
        self.balance_manager
            .credit(deal.provider, deal.provider_collateral)
            .await
            .map_err(|e| StorageMarketError::EconomicsError(e.to_string()))?;

        // Mark deal as completed
        deal.transition_to(StorageDealStatus::Completed)?;

        Ok(())
    }

    /// Get proof statistics for a deal
    pub async fn get_proof_stats(&self, deal_id: u64) -> ProofStats {
        let proofs = self.proofs.read().await;
        let deal_proofs = proofs.get(&deal_id);

        let total_proofs = deal_proofs.map(|v| v.len()).unwrap_or(0);
        let released = self.payments_released.read().await;
        let total_released = released.get(&deal_id).copied().unwrap_or(AdicAmount::ZERO);

        ProofStats {
            total_proofs,
            total_payment_released: total_released,
        }
    }
}

/// Proof statistics for a deal
#[derive(Debug, Clone)]
pub struct ProofStats {
    pub total_proofs: usize,
    pub total_payment_released: AdicAmount,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FinalityStatus, StorageDeal};
    use adic_economics::AccountAddress;

    fn create_test_deal() -> StorageDeal {
        StorageDeal {
            approvals: [[0u8; 32]; 4],
            features: [0; 3],
            signature: adic_types::Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: chrono::Utc::now().timestamp(),
            deal_id: 1,
            ref_intent: [0u8; 32],
            ref_acceptance: [1u8; 32],
            client: AccountAddress::from_bytes([1u8; 32]),
            provider: AccountAddress::from_bytes([2u8; 32]),
            data_cid: [5u8; 32],
            data_size: 1024 * 1024,
            deal_duration_epochs: 100,
            price_per_epoch: AdicAmount::from_adic(0.5),
            provider_collateral: AdicAmount::from_adic(50.0),
            client_payment_escrow: AdicAmount::from_adic(50.0),
            proof_merkle_root: Some([7u8; 32]),
            start_epoch: Some(10),
            activation_deadline: 60,
            status: StorageDealStatus::Active,
            finality_status: FinalityStatus::F1Complete {
                k: 3,
                depth: 10,
                rep_weight: 100.0,
            },
            finalized_at_epoch: Some(50),
        }
    }

    #[tokio::test]
    async fn test_generate_challenge() {
        use adic_challenges::ChallengeConfig;
        use adic_consensus::ReputationTracker;

        let storage = Arc::new(adic_economics::storage::MemoryStorage::new());
        let balance_manager = Arc::new(BalanceManager::new(storage));
        let challenge_manager = Arc::new(ChallengeWindowManager::new(
            ChallengeConfig::default()
        ));
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let vrf_service = Arc::new(VRFService::new(
            Default::default(),
            rep_tracker
        ));

        let manager = ProofCycleManager::new_for_testing(
            ProofCycleConfig::default(),
            balance_manager,
            challenge_manager,
            vrf_service,
        );

        let deal = create_test_deal();
        let challenges = manager.generate_challenge(&deal, 15, 15).await.unwrap();

        assert_eq!(challenges.len(), 3); // chunks_per_challenge
    }

    #[tokio::test]
    async fn test_release_payment() {
        use adic_challenges::ChallengeConfig;
        use adic_consensus::ReputationTracker;

        let storage = Arc::new(adic_economics::storage::MemoryStorage::new());
        let balance_manager = Arc::new(BalanceManager::new(storage));
        let challenge_manager = Arc::new(ChallengeWindowManager::new(
            ChallengeConfig::default()
        ));
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let vrf_service = Arc::new(VRFService::new(
            Default::default(),
            rep_tracker
        ));

        let manager = ProofCycleManager::new_for_testing(
            ProofCycleConfig::default(),
            balance_manager.clone(),
            challenge_manager,
            vrf_service,
        );

        let deal = create_test_deal();

        // Release payment
        manager.release_payment(&deal, 15).await.unwrap();

        // Check payment was released
        let stats = manager.get_proof_stats(deal.deal_id).await;
        assert!(stats.total_payment_released > AdicAmount::ZERO);
    }

    #[tokio::test]
    async fn test_complete_deal() {
        use adic_challenges::ChallengeConfig;
        use adic_consensus::ReputationTracker;

        let storage = Arc::new(adic_economics::storage::MemoryStorage::new());
        let balance_manager = Arc::new(BalanceManager::new(storage.clone()));
        let challenge_manager = Arc::new(ChallengeWindowManager::new(
            ChallengeConfig::default()
        ));
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let vrf_service = Arc::new(VRFService::new(
            Default::default(),
            rep_tracker
        ));

        let manager = ProofCycleManager::new_for_testing(
            ProofCycleConfig::default(),
            balance_manager.clone(),
            challenge_manager,
            vrf_service,
        );

        let mut deal = create_test_deal();

        // Fund the manager to release payments
        balance_manager
            .credit(deal.provider, AdicAmount::from_adic(100.0))
            .await
            .unwrap();

        // Complete the deal (after duration elapsed)
        manager.complete_deal(&mut deal, 150).await.unwrap();

        assert_eq!(deal.status, StorageDealStatus::Completed);
    }

    #[tokio::test]
    async fn test_slash_provider_transfers_to_treasury() {
        use adic_challenges::ChallengeConfig;
        use adic_consensus::ReputationTracker;

        let storage = Arc::new(adic_economics::storage::MemoryStorage::new());
        let balance_manager = Arc::new(BalanceManager::new(storage));
        let challenge_manager = Arc::new(ChallengeWindowManager::new(
            ChallengeConfig::default()
        ));
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let vrf_service = Arc::new(VRFService::new(
            Default::default(),
            rep_tracker
        ));

        let config = ProofCycleConfig {
            slash_percentage: 0.1, // 10% slash
            ..Default::default()
        };

        let manager = ProofCycleManager::new_for_testing(
            config,
            balance_manager.clone(),
            challenge_manager,
            vrf_service,
        );

        let deal = create_test_deal();

        // Fund provider with collateral
        balance_manager
            .credit(deal.provider, deal.provider_collateral)
            .await
            .unwrap();

        // Get initial balances
        let provider_balance_before = balance_manager.get_balance(deal.provider).await.unwrap();
        let treasury_addr = AccountAddress::treasury();
        let treasury_balance_before = balance_manager
            .get_balance(treasury_addr)
            .await
            .unwrap_or(AdicAmount::ZERO);

        // Slash the provider
        manager.slash_provider(&deal).await.unwrap();

        // Calculate expected slash amount (10% of 50.0 ADIC collateral = 5.0 ADIC)
        let expected_slash = AdicAmount::from_adic(5.0);

        // Verify provider balance decreased
        let provider_balance_after = balance_manager.get_balance(deal.provider).await.unwrap();
        assert_eq!(
            provider_balance_after,
            provider_balance_before
                .checked_sub(expected_slash)
                .unwrap()
        );

        // Verify treasury balance increased
        let treasury_balance_after = balance_manager.get_balance(treasury_addr).await.unwrap();
        assert_eq!(
            treasury_balance_after,
            treasury_balance_before
                .checked_add(expected_slash)
                .unwrap()
        );

        // Verify exact slash amount
        assert_eq!(expected_slash, AdicAmount::from_adic(5.0));
    }
}
