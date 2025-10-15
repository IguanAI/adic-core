use crate::error::{PoUWError, Result};
use crate::executor::TaskExecutor;
use crate::input_storage::InputStore;
use crate::task_storage::TaskStore;
use crate::types::{
    Hash, ProofType, TaskId, ValidationEvidence, ValidationReport, ValidationResult,
    WorkResult,
};
use adic_app_common::{FinalityAdapter, FinalityPolicy};
use adic_quorum::QuorumSelector;
use adic_types::{MessageId, PublicKey};
use blake3;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Trait for ZK proof verification
///
/// This trait allows plugging in different ZK proof systems (e.g., Halo2, Plonk, Groth16)
/// without coupling the validator to a specific implementation.
pub trait ZKProofVerifier: Send + Sync {
    /// Verify a ZK proof
    ///
    /// # Arguments
    /// * `proof_data` - Serialized ZK proof
    /// * `public_inputs` - Public inputs to the circuit
    /// * `verification_key` - Verification key for the circuit
    ///
    /// # Returns
    /// * `Ok(true)` if proof is valid
    /// * `Ok(false)` if proof is invalid
    /// * `Err(_)` if verification fails due to error
    fn verify(
        &self,
        proof_data: &[u8],
        public_inputs: &[u8],
        verification_key: &[u8],
    ) -> Result<bool>;
}

/// Placeholder ZK verifier for development and testing ONLY
///
/// ⚠️ **SECURITY WARNING** ⚠️
/// This verifier accepts ANY proof larger than 64 bytes without actual verification.
/// It is designed for development/testing when real ZK proofs are not available.
///
/// **DO NOT USE IN PRODUCTION**
///
/// To use this placeholder:
/// 1. Enable the `dev-placeholders` Cargo feature
/// 2. Or compile with `--features dev-placeholders`
///
/// In production, inject a real ZK verifier implementation (e.g., Halo2Verifier, PlonkVerifier)
/// using `ResultValidator::with_zk_verifier()`.
#[cfg(any(test, feature = "dev-placeholders"))]
pub struct PlaceholderZKVerifier;

#[cfg(any(test, feature = "dev-placeholders"))]
impl ZKProofVerifier for PlaceholderZKVerifier {
    fn verify(
        &self,
        proof_data: &[u8],
        _public_inputs: &[u8],
        _verification_key: &[u8],
    ) -> Result<bool> {
        // Basic sanity checks until real ZK verifier is integrated
        if proof_data.is_empty() {
            warn!("⚠️ ZK proof data is empty");
            return Ok(false);
        }

        // Check minimum proof size (e.g., Groth16 proofs are ~192 bytes)
        if proof_data.len() < 64 {
            warn!("⚠️ ZK proof data too small (< 64 bytes)");
            return Ok(false);
        }

        // TODO: Integrate real ZK verifier (Halo2, Arkworks, etc.)
        warn!("⚠️ Using placeholder ZK verifier - integrate real verifier for production");

        // For now, accept well-formed proofs
        Ok(true)
    }
}

/// Quorum-based result validator
pub struct ResultValidator {
    quorum_selector: Arc<QuorumSelector>,
    config: ValidatorConfig,
    finality_adapter: Option<Arc<FinalityAdapter>>,
    validation_cache: Arc<RwLock<HashMap<Hash, ValidationReport>>>,
    // Track message IDs for finality checking
    result_message_ids: Arc<RwLock<HashMap<Hash, MessageId>>>,
    /// Task executor for re-execution verification
    executor: Option<Arc<TaskExecutor>>,
    /// ZK proof verifier
    zk_verifier: Option<Arc<dyn ZKProofVerifier>>,
    /// BLS coordinator for threshold signature collection
    bls_coordinator: Option<Arc<crate::BLSCoordinator>>,
    /// Task storage for re-execution (H5)
    task_store: Option<Arc<dyn TaskStore>>,
    /// Input data storage for re-execution (H5)
    input_store: Option<Arc<dyn InputStore>>,
}

#[derive(Debug, Clone)]
pub struct ValidatorConfig {
    /// Minimum number of validators required
    pub min_validators: usize,
    /// Acceptance threshold (fraction of votes needed)
    pub acceptance_threshold: f64,
    /// Maximum time to collect votes (in seconds)
    pub vote_collection_timeout: u64,
    /// Require re-execution for validation
    pub require_re_execution: bool,
}

impl Default for ValidatorConfig {
    fn default() -> Self {
        Self {
            min_validators: 5,
            acceptance_threshold: 0.67, // 2/3 majority
            vote_collection_timeout: 300, // 5 minutes
            require_re_execution: true,
        }
    }
}

/// Validation vote from a single validator
#[derive(Debug, Clone)]
pub struct ValidationVote {
    pub validator: PublicKey,
    pub vote: bool, // true = accept, false = reject
    pub re_execution_hash: Option<Hash>,
    pub discrepancy_details: Option<String>,
    pub signature: Vec<u8>,
}

impl ResultValidator {
    pub fn new(quorum_selector: Arc<QuorumSelector>, config: ValidatorConfig) -> Self {
        Self {
            quorum_selector,
            config,
            finality_adapter: None,
            validation_cache: Arc::new(RwLock::new(HashMap::new())),
            result_message_ids: Arc::new(RwLock::new(HashMap::new())),
            executor: None,
            zk_verifier: None,
            bls_coordinator: None,
            task_store: None,
            input_store: None,
        }
    }

    /// Set finality adapter (for production use with real F1/F2)
    pub fn with_finality_adapter(mut self, adapter: Arc<FinalityAdapter>) -> Self {
        self.finality_adapter = Some(adapter);
        self
    }

    /// Set task executor for re-execution verification
    pub fn with_executor(mut self, executor: Arc<TaskExecutor>) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Set ZK proof verifier
    pub fn with_zk_verifier(mut self, verifier: Arc<dyn ZKProofVerifier>) -> Self {
        self.zk_verifier = Some(verifier);
        self
    }

    /// Set BLS coordinator for threshold signature collection
    pub fn with_bls_coordinator(mut self, coordinator: Arc<crate::BLSCoordinator>) -> Self {
        self.bls_coordinator = Some(coordinator);
        self
    }

    /// Set task storage for re-execution validation (H5)
    ///
    /// Task storage is required for full deterministic re-execution. Without it,
    /// the validator falls back to proof-based verification which is weaker.
    pub fn with_task_store(mut self, store: Arc<dyn TaskStore>) -> Self {
        self.task_store = Some(store);
        self
    }

    /// Set input data storage for re-execution validation (H5)
    ///
    /// Input storage is required for full deterministic re-execution. Without it,
    /// the validator falls back to proof-based verification which is weaker.
    pub fn with_input_store(mut self, store: Arc<dyn InputStore>) -> Self {
        self.input_store = Some(store);
        self
    }

    /// Register message ID for a validation result (for finality tracking)
    pub async fn register_result_message(&self, result_id: Hash, message_id: MessageId) {
        let mut message_ids = self.result_message_ids.write().await;
        message_ids.insert(result_id, message_id);
    }

    /// Validate a work result using quorum consensus
    pub async fn validate_result(
        &self,
        task_id: &TaskId,
        result: &WorkResult,
        current_epoch: u64,
    ) -> Result<ValidationReport> {
        info!(
            task_id = hex::encode(&task_id[..8]),
            result_id = hex::encode(&result.result_id[..8]),
            worker = hex::encode(&result.worker.as_bytes()[..8]),
            "🔍 Starting quorum validation"
        );

        // Check cache first
        if let Some(cached) = self.validation_cache.read().await.get(&result.result_id) {
            return Ok(cached.clone());
        }

        // Select validation quorum
        // In production, this would call quorum_selector.select_committee()
        // with proper configuration and eligible nodes
        let validators = self.select_validators_for_task(task_id, current_epoch).await?;

        if validators.len() < self.config.min_validators {
            return Err(PoUWError::QuorumNotReached {
                required: self.config.min_validators,
                actual: validators.len(),
            });
        }

        info!(
            task_id = hex::encode(&task_id[..8]),
            validators = validators.len(),
            "📋 Validation quorum selected"
        );

        // Collect validation votes
        let votes = self.collect_validation_votes(result, &validators).await?;

        // Aggregate votes
        let (validation_result, votes_for, votes_against) =
            self.aggregate_votes(&votes, validators.len());

        // Generate threshold BLS signature
        let threshold_bls_sig = self
            .generate_threshold_signature(task_id, &result.result_id, &votes)
            .await?;

        // Create validation evidence
        let validation_evidence: Vec<ValidationEvidence> = votes
            .into_iter()
            .map(|vote| ValidationEvidence {
                validator: vote.validator,
                vote: vote.vote,
                re_execution_hash: vote.re_execution_hash,
                discrepancy_details: vote.discrepancy_details,
                signature: vote.signature,
            })
            .collect();

        let report = ValidationReport {
            report_id: self.generate_report_id(task_id, &result.result_id),
            task_id: *task_id,
            result_id: result.result_id,
            validators: validators.clone(),
            votes_for,
            votes_against,
            threshold_bls_sig,
            validation_result,
            validation_evidence,
            finalized_at_epoch: current_epoch,
            timestamp: Utc::now(),
        };

        info!(
            task_id = hex::encode(&task_id[..8]),
            result = ?validation_result,
            votes_for = votes_for,
            votes_against = votes_against,
            "✅ Validation complete"
        );

        // Cache the result
        self.validation_cache
            .write()
            .await
            .insert(result.result_id, report.clone());

        Ok(report)
    }

    /// Select validators for a task using VRF-based quorum selection
    async fn select_validators_for_task(
        &self,
        task_id: &TaskId,
        current_epoch: u64,
    ) -> Result<Vec<PublicKey>> {
        // Build quorum config from validator config
        let quorum_config = adic_quorum::QuorumConfig {
            min_reputation: 50.0,  // Minimum reputation for validators
            members_per_axis: self.config.min_validators / 3,  // Distribute across 3 axes
            total_size: self.config.min_validators,
            max_per_asn: 2,  // Max 2 validators per ASN (Sybil resistance)
            max_per_region: 3,  // Max 3 validators per region
            domain_separator: format!("ADIC-PoUW-VALIDATOR-TASK-{}", hex::encode(task_id)),
            num_axes: 3,
        };

        // Generate eligible node info from available validators
        // TODO: Integrate with node registry once available to get real ASN/region metadata
        let eligible_nodes: Vec<adic_quorum::NodeInfo> = (0..self.config.min_validators * 2)
            .map(|i| {
                let mut seed = Vec::new();
                seed.extend_from_slice(task_id);
                seed.extend_from_slice(&i.to_le_bytes());
                let hash = blake3::hash(&seed);

                adic_quorum::NodeInfo {
                    public_key: PublicKey::from_bytes(*hash.as_bytes()),
                    reputation: 100.0 + (i as f64 * 10.0),  // Mock reputation scores
                    asn: Some((i % 5) as u32),  // Distribute across 5 ASNs
                    region: Some(format!("region-{}", i % 3)),  // 3 regions
                    axis_balls: vec![
                        vec![(i % 256) as u8],  // Time axis ball
                        vec![((i * 2) % 256) as u8],  // Topic axis ball
                        vec![((i * 3) % 256) as u8],  // Region axis ball
                    ],
                }
            })
            .collect();

        // Select committee using VRF-based quorum selector
        let quorum_result = self.quorum_selector
            .select_committee(current_epoch, &quorum_config, eligible_nodes)
            .await?;  // QuorumError automatically converts to PoUWError

        // Extract public keys from selected committee members
        let validators: Vec<PublicKey> = quorum_result
            .members
            .iter()
            .map(|member| member.public_key.clone())
            .collect();

        Ok(validators)
    }

    /// Collect votes from all validators
    async fn collect_validation_votes(
        &self,
        result: &WorkResult,
        validators: &[PublicKey],
    ) -> Result<Vec<ValidationVote>> {
        let mut votes = Vec::new();

        // In production, this would:
        // 1. Send validation requests to validators
        // 2. Wait for responses (with timeout)
        // 3. Verify validator signatures
        // For now, simulate validation

        for validator in validators {
            let vote = self.verify_work_result(result, validator).await?;
            votes.push(vote);
        }

        Ok(votes)
    }

    /// Verify work result with real verification logic
    ///
    /// Implements deterministic replication and challenge-based verification
    /// per PoUW specification.
    async fn verify_work_result(
        &self,
        result: &WorkResult,
        validator: &PublicKey,
    ) -> Result<ValidationVote> {
        debug!(
            validator = hex::encode(&validator.as_bytes()[..8]),
            result_id = hex::encode(&result.result_id[..8]),
            "🔍 Starting result verification"
        );

        // Step 1: Verify execution proof structure
        let proof_valid = self.verify_execution_proof(result)?;

        // Step 2: Perform deterministic re-execution (if required)
        let (re_execution_hash, output_matches) = if self.config.require_re_execution {
            let re_exec_result = self.deterministic_re_execution(result).await?;
            (Some(re_exec_result.output_hash), re_exec_result.matches)
        } else {
            (None, true) // Skip re-execution check
        };

        // Step 3: Verify resource claims are reasonable
        let resources_valid = self.verify_resource_claims(result)?;

        // Aggregate verification results
        let vote_accepts = proof_valid && output_matches && resources_valid;

        // Build discrepancy details if validation fails
        let discrepancy_details = if !vote_accepts {
            let mut details = Vec::new();
            if !proof_valid {
                details.push("Invalid execution proof");
            }
            if !output_matches {
                details.push("Re-execution output mismatch");
            }
            if !resources_valid {
                details.push("Resource usage claims invalid");
            }
            Some(details.join("; "))
        } else {
            None
        };

        // Sign the vote
        let signature = self.sign_vote(validator, vote_accepts, &result.result_id);

        debug!(
            validator = hex::encode(&validator.as_bytes()[..8]),
            vote_accepts = vote_accepts,
            "✅ Verification complete"
        );

        Ok(ValidationVote {
            validator: *validator,
            vote: vote_accepts,
            re_execution_hash,
            discrepancy_details,
            signature,
        })
    }

    /// Verify execution proof structure and cryptographic validity
    fn verify_execution_proof(&self, result: &WorkResult) -> Result<bool> {
        let proof = &result.execution_proof;

        // Verify proof based on type
        match proof.proof_type {
            ProofType::MerkleTree => self.verify_merkle_proof(proof),
            ProofType::StateTransition => self.verify_state_transition_proof(proof),
            ProofType::ZKProof => self.verify_zk_proof(proof),
            ProofType::ReExecution => self.verify_reexecution_proof(proof),
        }
    }

    /// Verify Merkle tree proof
    fn verify_merkle_proof(&self, proof: &crate::types::ExecutionProof) -> Result<bool> {
        // Verify merkle root is not empty
        if proof.merkle_root.iter().all(|&b| b == 0) {
            debug!("❌ Merkle root is empty");
            return Ok(false);
        }

        // Verify intermediate states exist
        if proof.intermediate_states.is_empty() {
            debug!("❌ No intermediate states provided");
            return Ok(false);
        }

        // Compute merkle root from intermediate states
        let computed_root = self.compute_merkle_root(&proof.intermediate_states);

        // Compare with provided root
        if computed_root != proof.merkle_root.to_vec() {
            debug!("❌ Merkle root mismatch");
            return Ok(false);
        }

        debug!("✅ Merkle proof valid");
        Ok(true)
    }

    /// Verify state transition proof
    fn verify_state_transition_proof(&self, proof: &crate::types::ExecutionProof) -> Result<bool> {
        // State transition proofs should have non-empty states
        if proof.intermediate_states.is_empty() {
            debug!("❌ No state transitions provided");
            return Ok(false);
        }

        // Verify each state transition is valid (non-zero)
        for (i, state) in proof.intermediate_states.iter().enumerate() {
            if state.iter().all(|&b| b == 0) {
                debug!("❌ Invalid state at index {}", i);
                return Ok(false);
            }
        }

        debug!("✅ State transition proof valid");
        Ok(true)
    }

    /// Verify ZK proof using configured verifier
    fn verify_zk_proof(&self, proof: &crate::types::ExecutionProof) -> Result<bool> {
        debug!("🔐 Verifying ZK proof");

        if let Some(ref verifier) = self.zk_verifier {
            // Extract public inputs from intermediate states
            let public_inputs: Vec<u8> = proof
                .intermediate_states
                .iter()
                .flat_map(|state| state.iter().copied())
                .collect();

            // Use merkle root as verification key (simplified)
            let verification_key = &proof.merkle_root;

            // Verify the ZK proof
            let is_valid = verifier.verify(
                &proof.proof_data,
                &public_inputs,
                verification_key,
            )?;

            if !is_valid {
                debug!("❌ ZK proof verification failed");
                return Ok(false);
            }

            debug!("✅ ZK proof valid");
            Ok(true)
        } else {
            // SECURITY: No ZK verifier configured
            // Block this in production release builds

            #[cfg(all(not(debug_assertions), not(test)))]
            {
                return Err(PoUWError::InvalidExecutionProof {
                    reason: "PRODUCTION SECURITY: ZK proof verification requires a cryptographic verifier. \
                    Configure a Halo2/Arkworks verifier or disable ZK proof type. \
                    Structural checks without cryptographic verification are NOT acceptable for production."
                        .to_string(),
                });
            }

            // Development/test mode: perform basic structural checks
            #[cfg(any(debug_assertions, test))]
            {
                warn!("⚠️ No ZK verifier configured, performing basic structural checks only");
                warn!("⚠️ THIS IS DEVELOPMENT MODE - NOT ACCEPTABLE FOR PRODUCTION");

                if proof.proof_data.is_empty() {
                    debug!("❌ ZK proof data is empty");
                    return Ok(false);
                }

                if proof.proof_data.len() < 64 {
                    debug!("❌ ZK proof data too small");
                    return Ok(false);
                }

                // Accept structurally valid proofs (insecure - for development only)
                warn!("⚠️ ZK proof accepted without cryptographic verification (DEV MODE)");
                Ok(true)
            }
        }
    }

    /// Verify re-execution proof
    ///
    /// For re-execution proofs, we expect the proof_data to contain the original
    /// input data hash, allowing validators to re-run the computation and verify.
    fn verify_reexecution_proof(&self, proof: &crate::types::ExecutionProof) -> Result<bool> {
        debug!("🔄 Verifying re-execution proof");

        // Re-execution proofs must have proof_data containing input hash
        if proof.proof_data.is_empty() {
            debug!("❌ Re-execution proof missing input data hash");
            return Ok(false);
        }

        // Verify intermediate states were recorded
        if proof.intermediate_states.is_empty() {
            debug!("❌ Re-execution proof missing execution states");
            return Ok(false);
        }

        // Verify merkle root matches computed root from states
        let computed_root = self.compute_merkle_root(&proof.intermediate_states);
        if computed_root != proof.merkle_root.to_vec() {
            debug!("❌ Re-execution merkle root mismatch");
            return Ok(false);
        }

        debug!("✅ Re-execution proof structure valid");
        Ok(true)
    }

    /// Perform deterministic re-execution to verify output
    ///
    /// This method will actually re-execute the task if an executor is configured,
    /// or fall back to proof-based verification otherwise.
    async fn deterministic_re_execution(
        &self,
        result: &WorkResult,
    ) -> Result<ReExecutionResult> {
        debug!(
            output_cid = %result.output_cid,
            "🔄 Performing deterministic re-execution"
        );

        // If executor is available, perform real re-execution
        if let Some(ref executor) = self.executor {
            return self.re_execute_with_executor(result, executor).await;
        }

        // H5 Production Safety: Block weak fallback in production builds
        #[cfg(all(
            not(test),
            not(debug_assertions),
            not(feature = "allow-weak-validation")
        ))]
        {
            return Err(PoUWError::ReExecutionRequired(
                "Re-execution required in production. Configure TaskExecutor, TaskStore, and InputStore for full validation security.".into()
            ));
        }

        // Fallback: Proof-based verification without re-execution (DEV/TEST ONLY)
        #[cfg(any(test, debug_assertions, feature = "allow-weak-validation"))]
        {
            warn!("⚠️ No executor configured, using proof-based verification (WEAK SECURITY)");

            // Compute expected output hash from proof data
            let mut output_data = Vec::new();
            output_data.extend_from_slice(&result.execution_proof.proof_data);
            output_data.extend_from_slice(&result.execution_proof.merkle_root);
            let expected_hash = blake3::hash(&output_data);

            // Verify output CID matches the hash
            let output_cid_bytes = result.output_cid.as_bytes();
            let matches = if output_cid_bytes.starts_with(b"Qm") {
                // Content-addressed format check (simplified)
                !result.output_cid.is_empty() && result.output_cid.len() >= 10
            } else {
                // Direct hash comparison
                let output_hash = blake3::hash(output_cid_bytes);
                output_hash.as_bytes()[..16] == expected_hash.as_bytes()[..16]
            };

            debug!(
                matches = matches,
                "🔄 Proof-based verification complete (weak)"
            );

            Ok(ReExecutionResult {
                output_hash: *expected_hash.as_bytes(),
                matches,
            })
        }
    }

    /// Re-execute task using TaskExecutor and compare results
    ///
    /// # Arguments
    /// * `result` - Original work result to verify
    /// * `executor` - Task executor to use for re-execution
    ///
    /// # Returns
    /// Re-execution result with output hash and match status
    ///
    /// # Implementation (H5)
    /// This method fetches the original task and input data from storage,
    /// re-executes the task deterministically, and compares outputs.
    async fn re_execute_with_executor(
        &self,
        result: &WorkResult,
        executor: &TaskExecutor,
    ) -> Result<ReExecutionResult> {
        debug!(
            task_id = hex::encode(&result.task_id),
            "🔄 Starting deterministic re-execution"
        );

        // Step 1: Fetch task from storage
        let task = match &self.task_store {
            Some(store) => {
                match store.get_task(&result.task_id).await? {
                    Some(task) => task,
                    None => {
                        return Err(PoUWError::TaskNotFoundInStorage(
                            hex::encode(&result.task_id)
                        ));
                    }
                }
            }
            None => {
                return Err(PoUWError::StorageNotConfigured(
                    "TaskStore required for re-execution".into()
                ));
            }
        };

        debug!(
            task_id = hex::encode(&result.task_id),
            input_cid = %task.input_cid,
            "✅ Retrieved task from storage"
        );

        // Step 2: Extract input hash from proof or parse from CID
        let input_hash = if result.execution_proof.proof_data.len() >= 32 {
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&result.execution_proof.proof_data[..32]);
            hash
        } else {
            // Parse input_cid from task (content-addressed)
            self.parse_cid_to_hash(&task.input_cid)?
        };

        // Step 3: Fetch input data from storage
        let input_data = match &self.input_store {
            Some(store) => {
                match store.get_input(&input_hash).await? {
                    Some(data) => data,
                    None => {
                        return Err(PoUWError::InputNotFound(hex::encode(&input_hash)));
                    }
                }
            }
            None => {
                return Err(PoUWError::StorageNotConfigured(
                    "InputStore required for re-execution".into()
                ));
            }
        };

        debug!(
            input_hash = hex::encode(&input_hash),
            input_size = input_data.len(),
            "✅ Retrieved input data from storage"
        );

        // Step 4: Deterministically re-execute the task
        let re_exec_result = executor.execute_task(&task, &input_data).await.map_err(|e| {
            PoUWError::ValidationFailed {
                reason: format!("Re-execution failed: {}", e),
            }
        })?;

        debug!(
            re_exec_output_cid = %re_exec_result.output_cid,
            "✅ Re-execution complete"
        );

        // Step 5: Compare output hashes
        let re_exec_output_hash = blake3::hash(re_exec_result.output_cid.as_bytes());
        let claimed_output_hash = blake3::hash(result.output_cid.as_bytes());
        let matches = re_exec_output_hash.as_bytes() == claimed_output_hash.as_bytes();

        if matches {
            info!(
                task_id = hex::encode(&result.task_id),
                "✅ Re-execution verification PASSED - outputs match"
            );
        } else {
            warn!(
                task_id = hex::encode(&result.task_id),
                expected = %hex::encode(re_exec_output_hash.as_bytes()),
                claimed = %hex::encode(claimed_output_hash.as_bytes()),
                "❌ Re-execution verification FAILED - output mismatch detected"
            );
        }

        Ok(ReExecutionResult {
            output_hash: *re_exec_output_hash.as_bytes(),
            matches,
        })
    }

    /// Parse CID string to content hash
    ///
    /// For now, uses blake3 hash of CID string. In production, this should
    /// properly decode the CID format (multihash) to extract the actual hash.
    fn parse_cid_to_hash(&self, cid: &str) -> Result<Hash> {
        // Simplified CID parsing - hash the CID string itself
        // TODO: Proper CID decoding with multihash support
        Ok(*blake3::hash(cid.as_bytes()).as_bytes())
    }

    /// Verify resource usage claims are reasonable
    fn verify_resource_claims(&self, result: &WorkResult) -> Result<bool> {
        let metrics = &result.execution_metrics;

        // Check that execution duration is positive
        if metrics.end_time <= metrics.start_time {
            debug!("❌ Invalid execution time: end <= start");
            return Ok(false);
        }

        let duration_secs = (metrics.end_time - metrics.start_time).num_seconds();

        // Check CPU time is reasonable (not exceeding wall-clock time significantly)
        let cpu_time_secs = metrics.cpu_time_ms / 1000;
        if cpu_time_secs > (duration_secs * 10) as u64 {
            // Allow 10x for parallel execution
            debug!(
                cpu_time_secs = cpu_time_secs,
                duration_secs = duration_secs,
                "❌ CPU time exceeds reasonable bounds"
            );
            return Ok(false);
        }

        // Check memory usage is non-zero and reasonable
        if metrics.memory_used_mb == 0 || metrics.memory_used_mb > 1_000_000 {
            // Max 1TB
            debug!(memory_mb = metrics.memory_used_mb, "❌ Invalid memory usage");
            return Ok(false);
        }

        debug!("✅ Resource claims verified");
        Ok(true)
    }

    /// Compute merkle root from intermediate states
    fn compute_merkle_root(&self, intermediate_states: &[Hash]) -> Vec<u8> {
        if intermediate_states.is_empty() {
            return vec![0u8; 32];
        }

        // Simple merkle tree: hash all states together
        let mut data = Vec::new();
        for state in intermediate_states {
            data.extend_from_slice(state);
        }

        blake3::hash(&data).as_bytes().to_vec()
    }

    /// Aggregate votes to determine validation result
    fn aggregate_votes(
        &self,
        votes: &[ValidationVote],
        total_validators: usize,
    ) -> (ValidationResult, usize, usize) {
        let votes_for = votes.iter().filter(|v| v.vote).count();
        let votes_against = votes.iter().filter(|v| !v.vote).count();

        let acceptance_rate = votes_for as f64 / total_validators as f64;

        let result = if acceptance_rate >= self.config.acceptance_threshold {
            ValidationResult::Accepted
        } else if votes_against as f64 / total_validators as f64 > (1.0 - self.config.acceptance_threshold) {
            ValidationResult::Rejected
        } else {
            ValidationResult::Inconclusive
        };

        (result, votes_for, votes_against)
    }

    /// Generate threshold BLS signature
    async fn generate_threshold_signature(
        &self,
        task_id: &TaskId,
        result_id: &Hash,
        votes: &[ValidationVote],
    ) -> Result<Vec<u8>> {
        if let Some(ref coordinator) = self.bls_coordinator {
            // Use real BLS threshold signature aggregation
            use crate::{SigningRequestId, DST_TASK_RECEIPT};

            // Create signing request ID
            let request_id = SigningRequestId::new(
                "validation",
                0, // epoch would be tracked in real implementation
                result_id
            );

            // Prepare message to sign (result validation data)
            let mut message = Vec::new();
            message.extend_from_slice(task_id);
            message.extend_from_slice(result_id);

            // Extract committee members from votes
            let committee_members: Vec<_> = votes.iter()
                .map(|v| v.validator.clone())
                .collect();

            // Initiate signing request
            let _request = coordinator.initiate_signing(
                request_id.clone(),
                message,
                DST_TASK_RECEIPT.to_vec(),
                committee_members.clone(),
            ).await?;

            // Check if signature already available (from completed async collection)
            if let Some(signature) = coordinator.get_aggregated_signature(&request_id).await? {
                info!(
                    request_id = %request_id.0,
                    "✅ Retrieved existing BLS threshold signature"
                );
                return Ok(signature.as_hex().as_bytes().to_vec());
            }

            // Simulate signature share collection from committee validators
            // In production network, each validator signs with their BLS secret share
            // (obtained from DKG) and submits via network protocol.
            // Here we generate shares to simulate that process.

            let threshold = committee_members.len();
            let (pub_key_set, secret_shares) = adic_crypto::generate_threshold_keys(
                committee_members.len(),
                (committee_members.len() * 2 / 3) + 1, // 2/3 threshold
            ).map_err(|e| PoUWError::BLSError(e))?;

            // Set public key set in coordinator if not already set
            if !coordinator.has_public_key_set().await {
                coordinator.set_public_key_set(pub_key_set).await;
            }

            // Create BLS signer for generating shares
            let threshold_config = adic_crypto::ThresholdConfig::new(
                committee_members.len(),
                (committee_members.len() * 2 / 3) + 1,
            ).map_err(|e| PoUWError::BLSError(e))?;

            let signer = adic_crypto::BLSThresholdSigner::new(threshold_config);

            // Simulate each validator signing with their secret share
            let message_bytes = {
                let mut msg = Vec::new();
                msg.extend_from_slice(task_id);
                msg.extend_from_slice(result_id);
                msg
            };

            for (i, validator_pk) in committee_members.iter().enumerate().take(threshold) {
                // Validator signs with their BLS secret share
                let share = signer.sign_share(
                    &secret_shares[i],
                    i,
                    &message_bytes,
                    DST_TASK_RECEIPT,
                ).map_err(|e| PoUWError::BLSError(e))?;

                // Submit share to coordinator (simulates network protocol delivery)
                if let Some(final_sig) = coordinator.submit_share(
                    &request_id,
                    validator_pk.clone(),
                    share,
                ).await? {
                    // Threshold reached! Coordinator aggregated signature
                    info!(
                        request_id = %request_id.0,
                        shares_collected = i + 1,
                        "✅ BLS threshold signature aggregated"
                    );
                    return Ok(final_sig.as_hex().as_bytes().to_vec());
                }
            }

            // All shares submitted but threshold not reached - shouldn't happen
            Err(PoUWError::Other(format!(
                "Failed to reach BLS threshold after submitting {} shares",
                threshold
            )))
        } else {
            // Fallback for tests without BLS coordinator
            warn!("⚠️ No BLS coordinator configured, using blake3 fallback for testing");
            let mut sig_data = Vec::new();
            sig_data.extend_from_slice(task_id);
            sig_data.extend_from_slice(result_id);
            for vote in votes {
                sig_data.push(if vote.vote { 1 } else { 0 });
            }
            Ok(blake3::hash(&sig_data).as_bytes().to_vec())
        }
    }

    /// Sign a validation vote
    fn sign_vote(&self, _validator: &PublicKey, vote: bool, result_id: &Hash) -> Vec<u8> {
        // In production, use validator's private key
        let mut sig_data = Vec::new();
        sig_data.extend_from_slice(result_id);
        sig_data.push(if vote { 1 } else { 0 });
        let hash = blake3::hash(&sig_data);
        // BLS signatures are 64 bytes, double the hash for simulation
        let mut signature = hash.as_bytes().to_vec();
        signature.extend_from_slice(hash.as_bytes());
        signature
    }

    /// Generate unique report ID
    fn generate_report_id(&self, task_id: &TaskId, result_id: &Hash) -> Hash {
        let mut data = Vec::new();
        data.extend_from_slice(task_id);
        data.extend_from_slice(result_id);
        data.extend_from_slice(&Utc::now().timestamp().to_le_bytes());
        blake3::hash(&data).into()
    }

    /// Get cached validation report
    pub async fn get_validation_report(&self, result_id: &Hash) -> Option<ValidationReport> {
        self.validation_cache.read().await.get(result_id).cloned()
    }

    /// Check if result has been validated
    pub async fn is_validated(&self, result_id: &Hash) -> bool {
        self.validation_cache.read().await.contains_key(result_id)
    }

    /// Wait for finality of validation result
    ///
    /// Per PoUW III §7.1: Operational signals MAY accept F1 only
    /// but remain subject to timelocks
    pub async fn wait_for_finality(
        &self,
        result_id: &Hash,
        timeout: Duration,
    ) -> Result<Duration> {
        // Get finality adapter or skip if not configured (test mode)
        let adapter = match &self.finality_adapter {
            Some(adapter) => adapter,
            None => {
                warn!(
                    result_id = hex::encode(&result_id[..8]),
                    "⚠️  No finality adapter configured, skipping finality check (test mode)"
                );
                // Return default time for test mode
                return Ok(Duration::from_secs(1));
            }
        };

        // Get message ID for this result
        let message_ids = self.result_message_ids.read().await;
        let message_id = message_ids
            .get(result_id)
            .ok_or_else(|| {
                PoUWError::ValidationFailed {
                    reason: format!(
                        "Message ID not found for result {}",
                        hex::encode(&result_id[..8])
                    )
                }
            })?;

        info!(
            result_id = hex::encode(&result_id[..8]),
            message_id = ?message_id,
            timeout_secs = timeout.as_secs(),
            "⏳ Waiting for finality (F1 for operational signals)..."
        );

        // Wait for F1 finality (operational signals per PoUW III §7.1)
        // Governance would require FinalityPolicy::Strict (F1 ∧ F2)
        let status = adapter
            .wait_for_finality(message_id, FinalityPolicy::F1Only, timeout)
            .await
            .map_err(|e| {
                PoUWError::ValidationFailed {
                    reason: format!(
                        "Finality timeout for result {}: {}",
                        hex::encode(&result_id[..8]),
                        e
                    )
                }
            })?;

        // Extract finality time
        let f1_time = status
            .f1_time
            .map(|t| t.elapsed())
            .unwrap_or(Duration::from_secs(0));

        info!(
            result_id = hex::encode(&result_id[..8]),
            f1_time_secs = f1_time.as_secs(),
            "✅ F1 finality achieved for validation result"
        );

        Ok(f1_time)
    }

    /// Get validation statistics
    pub async fn get_stats(&self) -> ValidationStats {
        let cache = self.validation_cache.read().await;
        let total = cache.len() as u64;
        let accepted = cache
            .values()
            .filter(|r| r.validation_result == ValidationResult::Accepted)
            .count() as u64;
        let rejected = cache
            .values()
            .filter(|r| r.validation_result == ValidationResult::Rejected)
            .count() as u64;
        let inconclusive = cache
            .values()
            .filter(|r| r.validation_result == ValidationResult::Inconclusive)
            .count() as u64;

        ValidationStats {
            total_validations: total,
            accepted,
            rejected,
            inconclusive,
        }
    }
}

/// Result of deterministic re-execution
struct ReExecutionResult {
    output_hash: Hash,
    matches: bool,
}

#[derive(Debug, Clone)]
pub struct ValidationStats {
    pub total_validations: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub inconclusive: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_consensus::ReputationTracker;
    use adic_vrf::VRFService;

    fn create_test_work_result(worker: PublicKey, output_cid: String) -> WorkResult {
        let mut intermediate_states: Vec<[u8; 32]> = Vec::new();
        intermediate_states.push(blake3::hash(b"state1").into());
        intermediate_states.push(blake3::hash(b"state2").into());

        // Compute valid merkle root from intermediate states
        let mut merkle_data = Vec::new();
        for state in &intermediate_states {
            merkle_data.extend_from_slice(&state[..]);
        }
        let merkle_root: [u8; 32] = blake3::hash(&merkle_data).into();

        // Generate unique result_id using canonical JSON for deterministic hashing
        use adic_types::canonical_hash;
        use serde::Serialize;

        let task_id = [1u8; 32];

        #[derive(Serialize)]
        struct ResultCanonical<'a> {
            task_id: &'a [u8; 32],
            worker: &'a [u8; 32],
            output_cid: &'a str,
        }

        let canonical = ResultCanonical {
            task_id: &task_id,
            worker: worker.as_bytes(),
            output_cid: &output_cid,
        };

        let result_id = canonical_hash(&canonical).expect("Failed to compute result ID");

        WorkResult {
            result_id,
            task_id,
            worker,
            output_cid,
            execution_proof: crate::types::ExecutionProof {
                proof_type: crate::types::ProofType::MerkleTree,
                merkle_root,
                intermediate_states,
                proof_data: vec![1, 2, 3, 4],
            },
            execution_metrics: crate::types::ExecutionMetrics {
                cpu_time_ms: 1000,
                memory_used_mb: 128,
                storage_used_mb: 64,
                network_used_kb: 512,
                start_time: Utc::now(),
                end_time: Utc::now() + chrono::Duration::seconds(2),
            },
            worker_signature: vec![0u8; 64],
            submitted_at: Utc::now(),
        }
    }

    // Helper to set up VRF randomness for testing
    async fn setup_test_vrf_randomness(vrf_service: &Arc<VRFService>, epoch: u64) {
        // Use test helper to directly set finalized randomness
        vrf_service.set_test_randomness(epoch, [epoch as u8; 32]).await;
    }

    #[tokio::test]
    #[ignore] // Needs full infrastructure setup - see validator_bls_quorum_integration_test for wiring validation
    async fn test_result_validation_accepted() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let vrf_service = Arc::new(VRFService::new(Default::default(), rep_tracker.clone()));

        // Setup VRF randomness for test epoch
        setup_test_vrf_randomness(&vrf_service, 0).await;

        let quorum_selector = Arc::new(QuorumSelector::new(vrf_service, rep_tracker));

        let mut config = ValidatorConfig::default();
        config.require_re_execution = false; // Simplify test
        config.min_validators = 2; // Lower threshold for unit test
        let validator = ResultValidator::new(quorum_selector, config);

        let worker = PublicKey::from_bytes([10u8; 32]);
        let result = create_test_work_result(worker, "QmValidOutput123".to_string());
        let task_id = [1u8; 32];

        let report = validator
            .validate_result(&task_id, &result, 0)
            .await
            .unwrap();

        assert_eq!(report.task_id, task_id);
        assert_eq!(report.result_id, result.result_id);
        assert_eq!(report.validation_result, ValidationResult::Accepted);
        assert!(report.votes_for > 0);
        assert!(!report.threshold_bls_sig.is_empty());
    }

    #[tokio::test]
    #[ignore] // Needs full infrastructure setup - see validator_bls_quorum_integration_test for wiring validation
    async fn test_result_validation_cached() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let vrf_service = Arc::new(VRFService::new(Default::default(), rep_tracker.clone()));

        // Setup VRF randomness for test epoch
        setup_test_vrf_randomness(&vrf_service, 0).await;

        let quorum_selector = Arc::new(QuorumSelector::new(vrf_service, rep_tracker));

        let mut config = ValidatorConfig::default();
        config.require_re_execution = false;
        config.min_validators = 2; // Lower threshold for unit test
        let validator = ResultValidator::new(quorum_selector, config);

        let worker = PublicKey::from_bytes([20u8; 32]);
        let result = create_test_work_result(worker, "QmCachedOutput456".to_string());
        let task_id = [2u8; 32];

        // First validation
        let report1 = validator
            .validate_result(&task_id, &result, 0)
            .await
            .unwrap();

        // Second validation should use cache
        let report2 = validator
            .validate_result(&task_id, &result, 0)
            .await
            .unwrap();

        assert_eq!(report1.report_id, report2.report_id);
        assert!(validator.is_validated(&result.result_id).await);
    }

    #[tokio::test]
    #[ignore] // Needs full infrastructure setup - see validator_bls_quorum_integration_test for wiring validation
    async fn test_validation_stats() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let vrf_service = Arc::new(VRFService::new(Default::default(), rep_tracker.clone()));

        // Setup VRF randomness for test epoch
        setup_test_vrf_randomness(&vrf_service, 0).await;

        let quorum_selector = Arc::new(QuorumSelector::new(vrf_service, rep_tracker));

        let mut config = ValidatorConfig::default();
        config.require_re_execution = false;
        config.min_validators = 2; // Lower threshold for unit test
        let validator = ResultValidator::new(quorum_selector, config);

        // Validate multiple results
        for i in 0..3 {
            let worker = PublicKey::from_bytes([i; 32]);
            let result = create_test_work_result(
                worker,
                format!("QmOutput{}", i),
            );
            let task_id = [i; 32];

            validator
                .validate_result(&task_id, &result, 0)
                .await
                .unwrap();
        }

        let stats = validator.get_stats().await;
        assert_eq!(stats.total_validations, 3);
        assert_eq!(stats.accepted, 3);
        assert_eq!(stats.rejected, 0);
    }
}
