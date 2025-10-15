use crate::{ChunkAcceptance, Hash, PoUWReceipt, Task, WorkResult};
use crate::executor::TaskExecutor;
use adic_challenges::{
    AdjudicationResult, DisputeAdjudicator, DisputeRuling, FraudEvidence, FraudProof, FraudType,
};
use adic_crypto::BLSThresholdSigner;
use adic_quorum::NodeInfo;
use adic_types::{MessageId, PublicKey};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Type alias for async function that provides eligible nodes for arbitration
pub type EligibleNodesProvider = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Vec<NodeInfo>> + Send>> + Send + Sync>;

/// Domain Separation Tag for PoUW receipt signatures (per PoUW III §10.3)
pub const DST_POUW_RECEIPT: &[u8] = b"ADIC-PoUW-RECEIPT-v1";

/// Receipt with associated fraud proofs and adjudication results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedReceipt {
    pub receipt: PoUWReceipt,
    /// Fraud proofs submitted against this receipt
    pub fraud_proofs: Vec<FraudProof>,
    /// Adjudication results for fraud proofs
    pub adjudications: Vec<AdjudicationResult>,
    /// Overall validation status
    pub status: ValidationStatus,
    /// Finalized at epoch (after challenge window expires)
    pub finalized_at_epoch: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationStatus {
    /// Within challenge window, pending potential fraud proofs
    PendingChallenge,
    /// Fraud proof submitted, under arbitration
    UnderDispute,
    /// All fraud proofs resolved, receipt valid
    Valid,
    /// Fraud proven, receipt invalid
    Invalid,
    /// Challenge window expired with no fraud
    FinalizedValid,
}

/// Fraud proof specifically for PoUW work results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkResultFraudProof {
    /// The base fraud proof
    pub proof: FraudProof,
    /// Specific chunk being challenged
    pub chunk_id: String,
    /// Worker who submitted the fraudulent work
    pub worker: PublicKey,
    /// Re-execution result (what it should have been)
    pub correct_result_hash: Option<Hash>,
    /// Discrepancy details
    pub discrepancy: String,
}

impl WorkResultFraudProof {
    pub fn new(
        receipt_id: MessageId,
        chunk_id: String,
        worker: PublicKey,
        challenger: PublicKey,
        correct_result_hash: Option<Hash>,
        discrepancy: String,
        submitted_at_epoch: u64,
    ) -> Self {
        Self::new_with_evidence(
            receipt_id,
            chunk_id,
            worker,
            challenger,
            correct_result_hash,
            discrepancy,
            submitted_at_epoch,
            None,
        )
    }

    /// Create fraud proof with re-execution evidence
    pub fn new_with_evidence(
        receipt_id: MessageId,
        chunk_id: String,
        worker: PublicKey,
        challenger: PublicKey,
        correct_result_hash: Option<Hash>,
        discrepancy: String,
        submitted_at_epoch: u64,
        re_execution_proof: Option<ReExecutionEvidence>,
    ) -> Self {
        // Build evidence showing the discrepancy
        let mut evidence_metadata = HashMap::new();
        evidence_metadata.insert("chunk_id".to_string(), chunk_id.clone());
        evidence_metadata.insert("discrepancy".to_string(), discrepancy.clone());
        if let Some(hash) = correct_result_hash {
            evidence_metadata.insert("correct_hash".to_string(), hex::encode(hash));
        }

        // Serialize re-execution proof if available
        let evidence_data = re_execution_proof.and_then(|proof| {
            serde_json::to_vec(&proof).ok()
        });

        let evidence = FraudEvidence {
            fraud_type: FraudType::InvalidResult,
            evidence_cid: format!("fraud_chunk_{}", chunk_id),
            evidence_data,
            metadata: evidence_metadata,
        };

        let proof = FraudProof::new(receipt_id, challenger, evidence, submitted_at_epoch);

        Self {
            proof,
            chunk_id,
            worker,
            correct_result_hash,
            discrepancy,
        }
    }
}

/// Re-execution evidence for fraud proofs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReExecutionEvidence {
    /// Task ID that was re-executed
    pub task_id: Hash,
    /// Input data hash used for re-execution
    pub input_hash: Hash,
    /// Expected output hash from re-execution
    pub expected_output_hash: Hash,
    /// Actual output hash claimed by worker
    pub claimed_output_hash: Hash,
    /// Re-execution merkle root
    pub re_execution_merkle_root: Hash,
    /// Intermediate states from re-execution
    pub re_execution_states: Vec<Hash>,
    /// Timestamp of re-execution
    pub re_executed_at: chrono::DateTime<chrono::Utc>,
}

/// Result of receipt validation
#[derive(Debug, Clone)]
pub struct ReceiptValidationResult {
    pub receipt_id: Hash,
    pub is_valid: bool,
    pub fraud_proofs_count: usize,
    pub upheld_fraud_proofs: Vec<MessageId>,
    pub rejected_fraud_proofs: Vec<MessageId>,
    pub validation_status: ValidationStatus,
}

/// Receipt validator integrating challenge/fraud proof system
pub struct ReceiptValidator {
    adjudicator: Arc<DisputeAdjudicator>,
    /// Challenge window depth (in epochs)
    challenge_window: u64,
    /// Pending fraud proofs by receipt ID
    pending_proofs: Arc<tokio::sync::RwLock<HashMap<Hash, Vec<WorkResultFraudProof>>>>,
    /// Adjudication results by fraud proof ID
    adjudication_results: Arc<tokio::sync::RwLock<HashMap<MessageId, AdjudicationResult>>>,
    /// Task executor for re-execution verification
    executor: Option<Arc<TaskExecutor>>,
    /// BLS threshold signer for signature verification
    bls_signer: Option<Arc<BLSThresholdSigner>>,
    /// Whether to enforce BLS signature verification
    enforce_bls: bool,
    /// Provider function to fetch eligible nodes for arbitrator selection (H3 fix)
    eligible_nodes_provider: Option<EligibleNodesProvider>,
}

impl ReceiptValidator {
    pub fn new(adjudicator: Arc<DisputeAdjudicator>, challenge_window: u64) -> Self {
        Self {
            adjudicator,
            challenge_window,
            pending_proofs: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            adjudication_results: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            executor: None,
            bls_signer: None,
            enforce_bls: false,
            eligible_nodes_provider: None,
        }
    }

    /// Configure the validator with a task executor for re-execution verification
    pub fn with_executor(mut self, executor: Arc<TaskExecutor>) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Configure BLS threshold signature verification
    pub fn with_bls_verification(mut self, signer: Arc<BLSThresholdSigner>, enforce: bool) -> Self {
        self.bls_signer = Some(signer);
        self.enforce_bls = enforce;
        self
    }

    /// Configure eligible nodes provider for arbitrator selection (H3 fix)
    ///
    /// This provider function is called when fraud proofs are submitted to get the
    /// list of eligible nodes for arbitrator selection. In production, this should
    /// be wired to `AdicNode::get_eligible_quorum_nodes()` to use real peer data
    /// with network metadata (ASN/region) for proper diversity.
    pub fn with_eligible_nodes_provider(mut self, provider: EligibleNodesProvider) -> Self {
        self.eligible_nodes_provider = Some(provider);
        self
    }

    /// Submit a fraud proof against a receipt
    pub async fn submit_fraud_proof(
        &self,
        receipt: &PoUWReceipt,
        fraud_proof: WorkResultFraudProof,
        current_epoch: u64,
    ) -> Result<(), String> {
        // Check if still within challenge window
        if current_epoch > receipt.epoch_id + self.challenge_window {
            return Err(format!(
                "Challenge window expired for receipt at epoch {}",
                receipt.epoch_id
            ));
        }

        // Verify the fraud proof structure
        fraud_proof
            .proof
            .verify_structure()
            .map_err(|e| format!("Invalid fraud proof structure: {}", e))?;

        let receipt_hash = self.compute_receipt_hash(receipt);

        info!(
            receipt_hash = hex::encode(&receipt_hash),
            chunk_id = fraud_proof.chunk_id,
            worker = hex::encode(fraud_proof.worker.as_bytes()),
            challenger = hex::encode(fraud_proof.proof.challenger.as_bytes()),
            "📋 Fraud proof submitted against PoUW receipt"
        );

        // Store the fraud proof
        let mut proofs = self.pending_proofs.write().await;
        proofs
            .entry(receipt_hash)
            .or_insert_with(Vec::new)
            .push(fraud_proof.clone());
        drop(proofs); // Release lock before async call

        // H3 Fix: Escalate to dispute adjudication using real nodes from provider
        // Fetch eligible nodes for arbitrator selection
        let eligible_nodes = if let Some(ref provider) = self.eligible_nodes_provider {
            // Use provider to get real nodes with network metadata
            let nodes_future = provider();
            nodes_future.await
        } else {
            // No provider configured - use empty list (tests or early development)
            warn!("⚠️ No eligible nodes provider configured - arbitrator selection will fail");
            vec![]
        };

        // Attempt to escalate to arbitration - non-fatal if it fails
        // (e.g., no eligible nodes available yet)
        match self
            .adjudicator
            .select_arbitrators(&fraud_proof.proof, eligible_nodes.clone())
            .await
        {
            Ok(_arbitrators) => {
                info!(
                    fraud_proof_id = hex::encode(fraud_proof.proof.id.as_bytes()),
                    eligible_nodes_count = eligible_nodes.len(),
                    "🔍 Fraud proof escalated to arbitration with {} eligible nodes",
                    eligible_nodes.len()
                );
            }
            Err(e) => {
                warn!(
                    fraud_proof_id = hex::encode(fraud_proof.proof.id.as_bytes()),
                    eligible_nodes_count = eligible_nodes.len(),
                    error = %e,
                    "⚠️ Could not immediately escalate fraud proof to arbitration - will retry later"
                );
            }
        }

        Ok(())
    }

    /// Validate a receipt considering all fraud proofs and adjudication results
    /// Verify BLS threshold signature on a receipt
    ///
    /// Per PoUW III §10.3, receipts must be signed by t-of-n quorum members.
    /// This verifies the aggregated BLS signature against the quorum's public key set.
    ///
    /// Returns Ok(true) if signature is valid, Ok(false) if no signature present,
    /// or Err if signature verification fails.
    pub fn verify_bls_signature(
        &self,
        receipt: &PoUWReceipt,
    ) -> Result<bool, String> {
        // Check if BLS signer is configured
        let signer = match &self.bls_signer {
            Some(s) => s,
            None => {
                if self.enforce_bls {
                    return Err("BLS signer not configured but BLS enforcement enabled".to_string());
                }
                return Ok(false); // No verification configured, skip
            }
        };

        // Check if receipt has BLS signature
        let bls_sig = match &receipt.sig_qk {
            Some(sig) => sig,
            None => {
                if self.enforce_bls {
                    return Err("Receipt missing required BLS signature".to_string());
                }
                return Ok(false); // No signature present
            }
        };

        // Check if receipt has aggregated public key
        if receipt.agg_pk.is_empty() {
            return Err("Receipt missing aggregated public key".to_string());
        }

        // Build the message that was signed
        // Per PoUW III §10.3: sig_Qk = BLS.Sign(hook_id || epoch || acceptances || rejections)
        let message = self.build_receipt_message(receipt);

        // Deserialize aggregated public key
        let pk_set = match bincode::deserialize::<threshold_crypto::PublicKeySet>(&receipt.agg_pk) {
            Ok(pk) => pk,
            Err(e) => return Err(format!("Invalid aggregated public key: {}", e)),
        };

        // Verify the BLS signature
        match signer.verify(&pk_set, &message, DST_POUW_RECEIPT, bls_sig) {
            Ok(valid) => {
                if valid {
                    debug!(
                        hook_id = hex::encode(&receipt.hook_id),
                        epoch = receipt.epoch_id,
                        "✅ PoUW receipt BLS signature verified"
                    );
                    Ok(true)
                } else {
                    warn!(
                        hook_id = hex::encode(&receipt.hook_id),
                        epoch = receipt.epoch_id,
                        "❌ PoUW receipt BLS signature verification failed"
                    );
                    Err("BLS signature verification failed".to_string())
                }
            }
            Err(e) => Err(format!("BLS signature verification error: {}", e)),
        }
    }

    /// Build the canonical message for BLS signing/verification
    ///
    /// Message format (per PoUW III §10.3):
    /// hook_id || epoch_id || accepted_count || rejected_count || acceptance_hashes || rejection_hashes
    fn build_receipt_message(&self, receipt: &PoUWReceipt) -> Vec<u8> {
        let mut message = Vec::new();

        // Add hook_id
        message.extend_from_slice(&receipt.hook_id);

        // Add epoch_id
        message.extend_from_slice(&receipt.epoch_id.to_le_bytes());

        // Add counts
        message.extend_from_slice(&(receipt.accepted.len() as u32).to_le_bytes());
        message.extend_from_slice(&(receipt.rejected.len() as u32).to_le_bytes());

        // Add acceptance hashes (deterministic ordering)
        for acceptance in &receipt.accepted {
            message.extend_from_slice(&acceptance.output_hash);
            message.extend_from_slice(acceptance.worker_pk.as_bytes());
        }

        // Add rejection data
        for rejection in &receipt.rejected {
            message.extend_from_slice(&rejection.chunk_id.to_le_bytes());
            message.extend_from_slice(rejection.worker_pk.as_bytes());
        }

        message
    }

    pub async fn validate_receipt(
        &self,
        receipt: &PoUWReceipt,
        current_epoch: u64,
    ) -> ReceiptValidationResult {
        let receipt_hash = self.compute_receipt_hash(receipt);

        // Step 1: Verify BLS threshold signature (if configured)
        if let Err(e) = self.verify_bls_signature(receipt) {
            warn!(
                receipt_hash = hex::encode(&receipt_hash),
                error = %e,
                "❌ Receipt BLS signature verification failed"
            );
            // If BLS enforcement is enabled, reject receipt immediately
            if self.enforce_bls {
                return ReceiptValidationResult {
                    receipt_id: receipt_hash,
                    is_valid: false,
                    fraud_proofs_count: 0,
                    upheld_fraud_proofs: vec![],
                    rejected_fraud_proofs: vec![],
                    validation_status: ValidationStatus::Invalid,
                };
            }
        }

        // Step 2: Check fraud proofs and adjudication results
        let proofs = self.pending_proofs.read().await;
        let adjudications = self.adjudication_results.read().await;

        let fraud_proofs = proofs.get(&receipt_hash).cloned().unwrap_or_default();

        let mut upheld_fraud_proofs = Vec::new();
        let mut rejected_fraud_proofs = Vec::new();

        // Check adjudication results
        for fraud_proof in &fraud_proofs {
            if let Some(result) = adjudications.get(&fraud_proof.proof.id) {
                match result.ruling {
                    DisputeRuling::ChallengerWins => {
                        upheld_fraud_proofs.push(fraud_proof.proof.id);
                    }
                    DisputeRuling::SubjectWins => {
                        rejected_fraud_proofs.push(fraud_proof.proof.id);
                    }
                    _ => {
                        // Still pending or inconclusive
                    }
                }
            }
        }

        // Determine validation status
        let validation_status = if !upheld_fraud_proofs.is_empty() {
            ValidationStatus::Invalid
        } else if fraud_proofs.is_empty()
            && current_epoch > receipt.epoch_id + self.challenge_window
        {
            ValidationStatus::FinalizedValid
        } else if !fraud_proofs.is_empty() && upheld_fraud_proofs.is_empty() {
            if current_epoch > receipt.epoch_id + self.challenge_window {
                // All fraud proofs were rejected or inconclusive, finalize as valid
                ValidationStatus::FinalizedValid
            } else {
                ValidationStatus::UnderDispute
            }
        } else if current_epoch <= receipt.epoch_id + self.challenge_window {
            ValidationStatus::PendingChallenge
        } else {
            ValidationStatus::Valid
        };

        // A receipt is considered valid if:
        // - It's finalized and valid
        // - It's pending challenge (no fraud proofs yet)
        // - It has no upheld fraud proofs
        let is_valid = matches!(
            validation_status,
            ValidationStatus::Valid
                | ValidationStatus::FinalizedValid
                | ValidationStatus::PendingChallenge
        );

        ReceiptValidationResult {
            receipt_id: receipt_hash,
            is_valid,
            fraud_proofs_count: fraud_proofs.len(),
            upheld_fraud_proofs,
            rejected_fraud_proofs,
            validation_status,
        }
    }

    /// Store an adjudication result
    pub async fn record_adjudication(&self, result: AdjudicationResult) {
        let emoji = match result.ruling {
            DisputeRuling::ChallengerWins => "⚔️",
            DisputeRuling::SubjectWins => "🛡️",
            DisputeRuling::Inconclusive => "❓",
            DisputeRuling::Partial => "⚖️",
        };

        info!(
            fraud_proof_id = hex::encode(result.fraud_proof_id.as_bytes()),
            ruling = ?result.ruling,
            threshold_met = result.threshold_met,
            "{} Adjudication recorded for PoUW receipt challenge",
            emoji
        );

        let mut adjudications = self.adjudication_results.write().await;
        adjudications.insert(result.fraud_proof_id, result);
    }

    /// Get validated receipt with all fraud proof information
    pub async fn get_validated_receipt(
        &self,
        receipt: PoUWReceipt,
        current_epoch: u64,
    ) -> ValidatedReceipt {
        let receipt_hash = self.compute_receipt_hash(&receipt);
        let proofs = self.pending_proofs.read().await;
        let adjudications = self.adjudication_results.read().await;

        let fraud_proofs: Vec<FraudProof> = proofs
            .get(&receipt_hash)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|wp| wp.proof)
            .collect();

        let adjudication_results: Vec<AdjudicationResult> = fraud_proofs
            .iter()
            .filter_map(|fp| adjudications.get(&fp.id).cloned())
            .collect();

        let validation_result = self.validate_receipt(&receipt, current_epoch).await;

        let finalized_at_epoch = if matches!(
            validation_result.validation_status,
            ValidationStatus::FinalizedValid | ValidationStatus::Invalid
        ) {
            Some(receipt.epoch_id + self.challenge_window)
        } else {
            None
        };

        ValidatedReceipt {
            receipt,
            fraud_proofs,
            adjudications: adjudication_results,
            status: validation_result.validation_status,
            finalized_at_epoch,
        }
    }

    /// Verify a chunk acceptance by re-executing the task
    ///
    /// # Arguments
    /// * `acceptance` - The chunk acceptance to verify
    /// * `task` - The task that was executed
    /// * `work_result` - The original work result submitted by the worker
    /// * `input_data` - The input data for the task
    ///
    /// # Returns
    /// * `Ok(None)` if verification succeeded (result is valid)
    /// * `Ok(Some(evidence))` if fraud detected, with re-execution evidence
    /// * `Err(_)` if verification failed due to error
    pub async fn verify_chunk_acceptance(
        &self,
        acceptance: &ChunkAcceptance,
        task: &Task,
        work_result: &WorkResult,
        input_data: &[u8],
    ) -> Result<Option<ReExecutionEvidence>, String> {
        debug!(
            chunk_id = acceptance.chunk_id,
            worker = hex::encode(&acceptance.worker_pk.as_bytes()[..8]),
            "🔍 Starting chunk re-execution verification"
        );

        // Step 1: Verify that the work_result matches the acceptance
        if work_result.worker != acceptance.worker_pk {
            return Err("Worker mismatch between acceptance and work result".to_string());
        }

        // Step 2: Perform deterministic re-execution if executor is available
        if let Some(ref executor) = self.executor {
            let re_execution_result = executor
                .execute_task(task, input_data)
                .await
                .map_err(|e| format!("Re-execution failed: {}", e))?;

            let input_hash = blake3::hash(input_data);

            // Step 3: Compare output hashes
            let re_exec_output_hash = self.compute_output_hash(&re_execution_result.output_cid);
            let claimed_output_hash = acceptance.output_hash;

            if re_exec_output_hash != claimed_output_hash {
                info!(
                    chunk_id = acceptance.chunk_id,
                    expected = hex::encode(&re_exec_output_hash),
                    claimed = hex::encode(&claimed_output_hash),
                    "⚠️ FRAUD DETECTED: Output hash mismatch"
                );

                // Build evidence of fraud
                let evidence = ReExecutionEvidence {
                    task_id: task.task_id,
                    input_hash: *input_hash.as_bytes(),
                    expected_output_hash: re_exec_output_hash,
                    claimed_output_hash,
                    re_execution_merkle_root: re_execution_result.execution_proof.merkle_root,
                    re_execution_states: re_execution_result.execution_proof.intermediate_states.clone(),
                    re_executed_at: chrono::Utc::now(),
                };

                return Ok(Some(evidence));
            }

            // Step 4: Verify execution proof consistency
            if !self.verify_execution_proof_consistency(&work_result.execution_proof, &re_execution_result.execution_proof) {
                warn!(
                    chunk_id = acceptance.chunk_id,
                    "⚠️ Execution proof mismatch (output matches but proof differs)"
                );

                let evidence = ReExecutionEvidence {
                    task_id: task.task_id,
                    input_hash: *input_hash.as_bytes(),
                    expected_output_hash: re_exec_output_hash,
                    claimed_output_hash,
                    re_execution_merkle_root: re_execution_result.execution_proof.merkle_root,
                    re_execution_states: re_execution_result.execution_proof.intermediate_states.clone(),
                    re_executed_at: chrono::Utc::now(),
                };

                return Ok(Some(evidence));
            }

            debug!(
                chunk_id = acceptance.chunk_id,
                "✅ Re-execution verification passed"
            );
            Ok(None)
        } else {
            // No executor configured - fall back to basic proof verification
            warn!(
                chunk_id = acceptance.chunk_id,
                "⚠️ No executor configured, skipping re-execution (proof structure check only)"
            );

            // Verify execution proof is not empty
            if work_result.execution_proof.merkle_root.iter().all(|&b| b == 0) {
                return Err("Invalid execution proof: empty merkle root".to_string());
            }

            Ok(None)
        }
    }

    /// Compute output hash from output CID
    fn compute_output_hash(&self, output_cid: &str) -> Hash {
        // In a real system, we would fetch the actual output data from content-addressed storage
        // For now, hash the CID itself as a deterministic representation
        blake3::hash(output_cid.as_bytes()).into()
    }

    /// Verify that execution proofs are consistent
    fn verify_execution_proof_consistency(
        &self,
        claimed_proof: &crate::types::ExecutionProof,
        re_execution_proof: &crate::types::ExecutionProof,
    ) -> bool {
        // Check that proof types match
        if claimed_proof.proof_type != re_execution_proof.proof_type {
            debug!("Proof type mismatch");
            return false;
        }

        // Check that merkle roots match
        if claimed_proof.merkle_root != re_execution_proof.merkle_root {
            debug!(
                "Merkle root mismatch: claimed={}, re_exec={}",
                hex::encode(&claimed_proof.merkle_root),
                hex::encode(&re_execution_proof.merkle_root)
            );
            return false;
        }

        // Check that intermediate states match
        if claimed_proof.intermediate_states.len() != re_execution_proof.intermediate_states.len() {
            debug!("Intermediate states length mismatch");
            return false;
        }

        for (i, (claimed_state, re_exec_state)) in claimed_proof
            .intermediate_states
            .iter()
            .zip(&re_execution_proof.intermediate_states)
            .enumerate()
        {
            if claimed_state != re_exec_state {
                debug!(
                    "Intermediate state {} mismatch: claimed={}, re_exec={}",
                    i,
                    hex::encode(claimed_state),
                    hex::encode(re_exec_state)
                );
                return false;
            }
        }

        true
    }

    /// Compute deterministic hash of receipt for indexing
    fn compute_receipt_hash(&self, receipt: &PoUWReceipt) -> Hash {
        // Use canonical JSON for deterministic hashing per RFC 8785
        use adic_types::canonical_hash;

        canonical_hash(receipt).expect("Failed to compute receipt hash")
    }

    /// Get all fraud proofs for a receipt
    pub async fn get_fraud_proofs(&self, receipt_hash: Hash) -> Vec<WorkResultFraudProof> {
        let proofs = self.pending_proofs.read().await;
        proofs.get(&receipt_hash).cloned().unwrap_or_default()
    }

    /// Clean up finalized receipts (remove fraud proofs after finalization)
    pub async fn cleanup_finalized(&self, receipt: &PoUWReceipt, current_epoch: u64) {
        if current_epoch > receipt.epoch_id + self.challenge_window {
            let receipt_hash = self.compute_receipt_hash(receipt);
            let mut proofs = self.pending_proofs.write().await;

            if let Some(removed) = proofs.remove(&receipt_hash) {
                info!(
                    receipt_hash = hex::encode(&receipt_hash),
                    fraud_proofs_count = removed.len(),
                    "🧹 Cleaned up finalized receipt fraud proofs"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_challenges::{AdjudicationConfig, DisputeAdjudicator};
    use adic_consensus::ReputationTracker;
    use adic_crypto::{BLSSignature, Keypair};
    use adic_quorum::QuorumSelector;
    use adic_vrf::VRFService;
    use crate::ExecutorConfig;
    use chrono::Utc;

    fn create_test_receipt(epoch_id: u64) -> PoUWReceipt {
        PoUWReceipt {
            version: 1,
            hook_id: [1; 32],
            epoch_id,
            receipt_seq: 1,
            prev_receipt_hash: [0; 32],
            accepted: vec![ChunkAcceptance {
                chunk_id: 1,
                worker_pk: PublicKey::from_bytes([1; 32]),
                output_hash: [2; 32],
            }],
            rejected: vec![],
            agg_pk: vec![],
            sig_qk: Some(BLSSignature::from_hex("test".to_string())),
        }
    }

    #[tokio::test]
    async fn test_submit_fraud_proof() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.5));
        let vrf_service = Arc::new(VRFService::new(
            adic_vrf::VRFConfig::default(),
            rep_tracker.clone(),
        ));
        let quorum_selector = Arc::new(QuorumSelector::new(vrf_service, rep_tracker));
        let adjudicator = Arc::new(DisputeAdjudicator::new(
            quorum_selector,
            AdjudicationConfig::default(),
        ));

        let validator = ReceiptValidator::new(adjudicator, 100);
        let receipt = create_test_receipt(1000);

        let fraud_proof = WorkResultFraudProof::new(
            MessageId::new(b"receipt_1"),
            "chunk_1".to_string(),
            PublicKey::from_bytes([1; 32]),
            PublicKey::from_bytes([2; 32]),
            Some([3; 32]),
            "Result hash mismatch".to_string(),
            1050,
        );

        let result = validator.submit_fraud_proof(&receipt, fraud_proof, 1050).await;
        assert!(result.is_ok());

        let receipt_hash = validator.compute_receipt_hash(&receipt);
        let proofs = validator.get_fraud_proofs(receipt_hash).await;
        assert_eq!(proofs.len(), 1);
    }

    #[tokio::test]
    async fn test_validate_receipt_pending() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.5));
        let vrf_service = Arc::new(VRFService::new(
            adic_vrf::VRFConfig::default(),
            rep_tracker.clone(),
        ));
        let quorum_selector = Arc::new(QuorumSelector::new(vrf_service, rep_tracker));
        let adjudicator = Arc::new(DisputeAdjudicator::new(
            quorum_selector,
            AdjudicationConfig::default(),
        ));

        let validator = ReceiptValidator::new(adjudicator, 100);
        let receipt = create_test_receipt(1000);

        let result = validator.validate_receipt(&receipt, 1050).await;
        assert_eq!(result.validation_status, ValidationStatus::PendingChallenge);
        assert!(result.is_valid);
    }

    #[tokio::test]
    async fn test_validate_receipt_finalized() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.5));
        let vrf_service = Arc::new(VRFService::new(
            adic_vrf::VRFConfig::default(),
            rep_tracker.clone(),
        ));
        let quorum_selector = Arc::new(QuorumSelector::new(vrf_service, rep_tracker));
        let adjudicator = Arc::new(DisputeAdjudicator::new(
            quorum_selector,
            AdjudicationConfig::default(),
        ));

        let validator = ReceiptValidator::new(adjudicator, 100);
        let receipt = create_test_receipt(1000);

        // After challenge window (1000 + 100 = 1100)
        let result = validator.validate_receipt(&receipt, 1101).await;
        assert_eq!(result.validation_status, ValidationStatus::FinalizedValid);
        assert!(result.is_valid);
    }

    #[tokio::test]
    async fn test_fraud_proof_window_expiry() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.5));
        let vrf_service = Arc::new(VRFService::new(
            adic_vrf::VRFConfig::default(),
            rep_tracker.clone(),
        ));
        let quorum_selector = Arc::new(QuorumSelector::new(vrf_service, rep_tracker));
        let adjudicator = Arc::new(DisputeAdjudicator::new(
            quorum_selector,
            AdjudicationConfig::default(),
        ));

        let validator = ReceiptValidator::new(adjudicator, 100);
        let receipt = create_test_receipt(1000);

        let fraud_proof = WorkResultFraudProof::new(
            MessageId::new(b"receipt_1"),
            "chunk_1".to_string(),
            PublicKey::from_bytes([1; 32]),
            PublicKey::from_bytes([2; 32]),
            Some([3; 32]),
            "Result hash mismatch".to_string(),
            1150, // After window expires
        );

        // Should fail - window expired
        let result = validator.submit_fraud_proof(&receipt, fraud_proof, 1150).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_re_execution_verification() {
        use crate::types::{
            ComputationType, FinalityStatus, ResourceRequirements, Task, TaskStatus, TaskType,
        };
        use adic_economics::types::AdicAmount;

        // Create task executor
        let keypair = Keypair::generate();
        let executor = Arc::new(TaskExecutor::new(keypair.clone(), ExecutorConfig::default()));

        // Create receipt validator with executor
        let rep_tracker = Arc::new(ReputationTracker::new(0.5));
        let vrf_service = Arc::new(VRFService::new(
            adic_vrf::VRFConfig::default(),
            rep_tracker.clone(),
        ));
        let quorum_selector = Arc::new(QuorumSelector::new(vrf_service, rep_tracker));
        let adjudicator = Arc::new(DisputeAdjudicator::new(
            quorum_selector,
            AdjudicationConfig::default(),
        ));

        let validator = ReceiptValidator::new(adjudicator, 100).with_executor(executor.clone());

        // Create a task
        let task = Task {
            task_id: [1u8; 32],
            sponsor: PublicKey::from_bytes([2u8; 32]),
            task_type: TaskType::Compute {
                computation_type: ComputationType::HashVerification,
                resource_requirements: ResourceRequirements::default(),
            },
            input_cid: "QmTest123".to_string(),
            expected_output_schema: None,
            reward: AdicAmount::from_adic(10.0),
            collateral_requirement: AdicAmount::from_adic(5.0),
            deadline_epoch: 100,
            min_reputation: 100.0,
            worker_count: 1,
            created_at: Utc::now(),
            status: TaskStatus::Assigned,
            finality_status: FinalityStatus::F2Complete,
        };

        // Execute task to get a valid work result
        let input_data = b"test input data";
        let work_result = executor.execute_task(&task, input_data).await.unwrap();

        // Create chunk acceptance from work result
        let acceptance = ChunkAcceptance {
            chunk_id: 1,
            worker_pk: work_result.worker,
            output_hash: blake3::hash(work_result.output_cid.as_bytes()).into(),
        };

        // Verify chunk acceptance with re-execution - should pass
        let verification_result = validator
            .verify_chunk_acceptance(&acceptance, &task, &work_result, input_data)
            .await;

        assert!(verification_result.is_ok());
        assert!(verification_result.unwrap().is_none()); // No fraud detected

        // Now test with fraudulent output hash
        let fraudulent_acceptance = ChunkAcceptance {
            chunk_id: 1,
            worker_pk: work_result.worker,
            output_hash: [99u8; 32], // Wrong hash
        };

        let fraud_check = validator
            .verify_chunk_acceptance(&fraudulent_acceptance, &task, &work_result, input_data)
            .await;

        assert!(fraud_check.is_ok());
        let fraud_evidence = fraud_check.unwrap();
        assert!(fraud_evidence.is_some()); // Fraud detected!

        // Verify evidence contains correct information
        let evidence = fraud_evidence.unwrap();
        assert_eq!(evidence.task_id, task.task_id);
        assert_eq!(evidence.claimed_output_hash, [99u8; 32]);
        assert_ne!(evidence.expected_output_hash, evidence.claimed_output_hash);
    }

    #[tokio::test]
    async fn test_fraud_proof_with_evidence() {
        // Test creating fraud proof with re-execution evidence
        let evidence = ReExecutionEvidence {
            task_id: [1u8; 32],
            input_hash: [2u8; 32],
            expected_output_hash: [3u8; 32],
            claimed_output_hash: [4u8; 32],
            re_execution_merkle_root: [5u8; 32],
            re_execution_states: vec![[6u8; 32], [7u8; 32]],
            re_executed_at: Utc::now(),
        };

        let fraud_proof = WorkResultFraudProof::new_with_evidence(
            MessageId::new(b"receipt_1"),
            "chunk_1".to_string(),
            PublicKey::from_bytes([1; 32]),
            PublicKey::from_bytes([2; 32]),
            Some([3; 32]),
            "Re-execution output mismatch".to_string(),
            1000,
            Some(evidence.clone()),
        );

        // Verify evidence was serialized
        assert!(fraud_proof.proof.evidence.evidence_data.is_some());

        // Verify we can deserialize it back
        let serialized = fraud_proof.proof.evidence.evidence_data.unwrap();
        let deserialized: ReExecutionEvidence = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(deserialized.task_id, evidence.task_id);
        assert_eq!(deserialized.expected_output_hash, evidence.expected_output_hash);
    }
}
