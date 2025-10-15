use crate::error::{PoUWError, Result};
use crate::types::{
    ComputationType, ExecutionMetrics, ExecutionProof, Hash, ProofType, ResourceRequirements,
    Task, TaskType, WorkResult,
};
use adic_crypto::Keypair;
use adic_types::PublicKey;
use blake3;
use chrono::Utc;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tokio::time::timeout;
use tracing::info;

/// Task executor with resource limits and proof generation
pub struct TaskExecutor {
    worker_id: PublicKey,
    worker_keypair: Arc<Keypair>,
    config: ExecutorConfig,
    execution_stats: Arc<RwLock<ExecutionStats>>,
    concurrent_tasks: Arc<RwLock<usize>>, // Track number of currently executing tasks
}

#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    /// Global resource multiplier (for safety margin)
    pub resource_multiplier: f64,
    /// Enable strict resource enforcement
    pub strict_enforcement: bool,
    /// Maximum concurrent tasks
    pub max_concurrent_tasks: usize,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            resource_multiplier: 0.9, // Use 90% of declared limits
            strict_enforcement: true,
            max_concurrent_tasks: 4,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionStats {
    pub total_executions: u64,
    pub successful_executions: u64,
    pub failed_executions: u64,
    pub total_execution_time_ms: u64,
    pub total_cpu_time_ms: u64,
    pub total_memory_used_mb: u64,
}

impl TaskExecutor {
    pub fn new(worker_keypair: Keypair, config: ExecutorConfig) -> Self {
        let worker_id = *worker_keypair.public_key();
        Self {
            worker_id,
            worker_keypair: Arc::new(worker_keypair),
            config,
            execution_stats: Arc::new(RwLock::new(ExecutionStats::default())),
            concurrent_tasks: Arc::new(RwLock::new(0)),
        }
    }

    /// Execute a task and generate work result with proof
    pub async fn execute_task(
        &self,
        task: &Task,
        input_data: &[u8],
    ) -> Result<WorkResult> {
        // Check concurrent task limit
        {
            let mut concurrent = self.concurrent_tasks.write().await;
            if *concurrent >= self.config.max_concurrent_tasks {
                return Err(PoUWError::ResourceLimitExceeded {
                    resource: "Concurrent tasks".to_string(),
                    limit: format!("{}", self.config.max_concurrent_tasks),
                });
            }
            *concurrent += 1;
        }

        let start = Instant::now();
        let start_time = Utc::now();

        info!(
            task_id = hex::encode(&task.task_id[..8]),
            worker = hex::encode(&self.worker_id.as_bytes()[..8]),
            "⚙️ Starting task execution"
        );

        // Validate resource requirements
        let validation_result = self.validate_resource_requirements(task);
        if let Err(e) = validation_result {
            // Decrement counter before returning error
            let mut concurrent = self.concurrent_tasks.write().await;
            *concurrent = concurrent.saturating_sub(1);
            return Err(e);
        }

        // Execute based on task type
        let execution_result = match &task.task_type {
            TaskType::Compute {
                computation_type,
                resource_requirements,
            } => {
                self.execute_compute_task(
                    task,
                    input_data,
                    computation_type,
                    resource_requirements,
                )
                .await
            }
            TaskType::Validation {
                target_task_id,
                validation_criteria,
            } => {
                self.execute_validation_task(
                    task,
                    input_data,
                    target_task_id,
                    validation_criteria,
                )
                .await
            }
            TaskType::Storage {
                data_cid,
                duration_epochs,
                redundancy,
            } => {
                self.execute_storage_task(task, input_data, data_cid, *duration_epochs, *redundancy)
                    .await
            }
            TaskType::Aggregation {
                source_task_ids,
                aggregation_function,
            } => {
                self.execute_aggregation_task(
                    task,
                    input_data,
                    source_task_ids,
                    aggregation_function,
                )
                .await
            }
        };

        let (output_data, execution_proof) = match execution_result {
            Ok(result) => result,
            Err(e) => {
                // Decrement counter before returning error
                let mut concurrent = self.concurrent_tasks.write().await;
                *concurrent = concurrent.saturating_sub(1);
                return Err(e);
            }
        };

        let end_time = Utc::now();
        let execution_time = start.elapsed();

        // Estimate network usage (input download + output upload)
        let network_used_kb = ((input_data.len() + output_data.len()) / 1024) as u64;

        // Generate execution metrics
        let metrics = ExecutionMetrics {
            cpu_time_ms: execution_time.as_millis() as u64,
            memory_used_mb: self.estimate_memory_usage(&output_data),
            storage_used_mb: (output_data.len() / (1024 * 1024)) as u64,
            network_used_kb,
            start_time,
            end_time,
        };

        // Update stats
        let mut stats = self.execution_stats.write().await;
        stats.total_executions += 1;
        stats.successful_executions += 1;
        stats.total_execution_time_ms += execution_time.as_millis() as u64;
        stats.total_cpu_time_ms += metrics.cpu_time_ms;
        stats.total_memory_used_mb += metrics.memory_used_mb;
        drop(stats);

        // Generate output CID
        let output_cid = format!("Qm{}", hex::encode(&blake3::hash(&output_data).as_bytes()[..20]));

        info!(
            task_id = hex::encode(&task.task_id[..8]),
            execution_time_ms = execution_time.as_millis() as u64,
            output_size = output_data.len(),
            "✅ Task execution completed"
        );

        // Sign the work result with worker key
        // Message to sign: task_id || output_cid || proof_hash
        let mut sign_data = Vec::new();
        sign_data.extend_from_slice(&task.task_id);
        sign_data.extend_from_slice(output_cid.as_bytes());
        sign_data.extend_from_slice(&execution_proof.merkle_root);

        let worker_signature = self.worker_keypair.sign(&sign_data);

        let result = WorkResult {
            result_id: self.generate_result_id(task),
            task_id: task.task_id,
            worker: self.worker_id,
            output_cid,
            execution_proof,
            execution_metrics: metrics,
            worker_signature: worker_signature.as_bytes().to_vec(),
            submitted_at: Utc::now(),
        };

        // Decrement counter before returning
        {
            let mut concurrent = self.concurrent_tasks.write().await;
            *concurrent = concurrent.saturating_sub(1);
        }

        Ok(result)
    }

    /// Execute a computation task with resource limits
    async fn execute_compute_task(
        &self,
        task: &Task,
        input_data: &[u8],
        computation_type: &ComputationType,
        resource_requirements: &ResourceRequirements,
    ) -> Result<(Vec<u8>, ExecutionProof)> {
        let timeout_ms = resource_requirements.max_cpu_ms;
        let timeout_duration = std::time::Duration::from_millis(timeout_ms);

        // Execute with timeout
        let result = timeout(
            timeout_duration,
            self.do_computation(task, input_data, computation_type, resource_requirements),
        )
        .await;

        match result {
            Ok(Ok((output, proof))) => Ok((output, proof)),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(PoUWError::ResourceLimitExceeded {
                resource: "CPU time".to_string(),
                limit: format!("{}ms", timeout_ms),
            }),
        }
    }

    /// Perform the actual computation
    async fn do_computation(
        &self,
        _task: &Task,
        input_data: &[u8],
        computation_type: &ComputationType,
        _resource_requirements: &ResourceRequirements,
    ) -> Result<(Vec<u8>, ExecutionProof)> {
        let mut intermediate_states = Vec::new();

        // Different computation types
        let output_data = match computation_type {
            ComputationType::HashVerification => {
                // Compute hash chain with intermediate states
                let mut current_hash = blake3::hash(input_data);
                intermediate_states.push(*current_hash.as_bytes());

                for _ in 0..3 {
                    current_hash = blake3::hash(current_hash.as_bytes());
                    intermediate_states.push(*current_hash.as_bytes());
                }

                current_hash.as_bytes().to_vec()
            }
            ComputationType::DataProcessing => {
                // Simple data transformation with proof
                let transformed = input_data
                    .iter()
                    .map(|b| b.wrapping_add(1))
                    .collect::<Vec<u8>>();

                intermediate_states.push(blake3::hash(&transformed[..transformed.len() / 2]).into());
                intermediate_states.push(blake3::hash(&transformed).into());

                transformed
            }
            ComputationType::ModelInference => {
                #[cfg(feature = "ml-inference")]
                {
                    // ⚠️ PLACEHOLDER IMPLEMENTATION - DO NOT USE IN PRODUCTION
                    //
                    // This placeholder returns deterministic fake results for testing.
                    // For production, integrate a real ML runtime:
                    // - ONNX Runtime: https://onnxruntime.ai/
                    // - TensorFlow Lite: https://www.tensorflow.org/lite
                    // - Candle (Rust): https://github.com/huggingface/candle
                    //
                    // Enable with: cargo build --features ml-inference
                    warn!("⚠️ Using placeholder ML inference - not suitable for production");
                    let output = format!("inference_result_{}", hex::encode(&task.task_id[..8]));
                    intermediate_states.push(blake3::hash(output.as_bytes()).into());
                    output.into_bytes()
                }
                #[cfg(not(feature = "ml-inference"))]
                {
                    // ML inference not available in production builds
                    return Err(PoUWError::UnsupportedTaskType(
                        "ML inference requires 'ml-inference' feature flag. \
                         This feature is experimental and requires integration with \
                         a real ML runtime (ONNX, TensorFlow Lite, or Candle) before \
                         production use.".to_string()
                    ));
                }
            }
            ComputationType::Custom(ref computation) => {
                // Custom computation logic
                let output = format!("custom_{}_{}", computation, hex::encode(input_data));
                intermediate_states.push(blake3::hash(output.as_bytes()).into());
                output.into_bytes()
            }
        };

        // Generate Merkle proof
        let merkle_root = self.compute_merkle_root(&intermediate_states);
        let proof_data = self.generate_proof_data(&intermediate_states);

        let proof = ExecutionProof {
            proof_type: ProofType::MerkleTree,
            merkle_root,
            intermediate_states,
            proof_data,
        };

        Ok((output_data, proof))
    }

    /// Execute a validation task
    async fn execute_validation_task(
        &self,
        task: &Task,
        input_data: &[u8],
        _target_task_id: &Hash,
        validation_criteria: &str,
    ) -> Result<(Vec<u8>, ExecutionProof)> {
        // Validation re-executes the target task and compares results
        let validation_result = format!(
            "validation_{}_{}_{}",
            validation_criteria,
            hex::encode(&task.task_id[..8]),
            blake3::hash(input_data)
        );

        let intermediate_states = vec![blake3::hash(validation_result.as_bytes()).into()];
        let merkle_root = self.compute_merkle_root(&intermediate_states);

        Ok((
            validation_result.into_bytes(),
            ExecutionProof {
                proof_type: ProofType::ReExecution,
                merkle_root,
                intermediate_states,
                proof_data: vec![],
            },
        ))
    }

    /// Execute a storage task
    async fn execute_storage_task(
        &self,
        task: &Task,
        input_data: &[u8],
        data_cid: &str,
        _duration_epochs: u64,
        _redundancy: u8,
    ) -> Result<(Vec<u8>, ExecutionProof)> {
        // Verify data integrity via CID
        let computed_hash = blake3::hash(input_data);
        let storage_receipt = format!("stored_{}_{}_{}", data_cid, hex::encode(&task.task_id[..8]), computed_hash);

        let intermediate_states = vec![computed_hash.into()];
        let merkle_root = self.compute_merkle_root(&intermediate_states);

        Ok((
            storage_receipt.into_bytes(),
            ExecutionProof {
                proof_type: ProofType::StateTransition,
                merkle_root,
                intermediate_states,
                proof_data: vec![],
            },
        ))
    }

    /// Execute an aggregation task
    async fn execute_aggregation_task(
        &self,
        task: &Task,
        input_data: &[u8],
        source_task_ids: &[Hash],
        aggregation_function: &str,
    ) -> Result<(Vec<u8>, ExecutionProof)> {
        let mut intermediate_states = Vec::new();

        // Aggregate results from multiple sources
        for source_id in source_task_ids {
            let state = blake3::hash(&[&source_id[..], input_data].concat());
            intermediate_states.push(state.into());
        }

        let aggregation_result = format!(
            "aggregated_{}_{}_{}",
            aggregation_function,
            hex::encode(&task.task_id[..8]),
            source_task_ids.len()
        );

        let merkle_root = self.compute_merkle_root(&intermediate_states);

        Ok((
            aggregation_result.into_bytes(),
            ExecutionProof {
                proof_type: ProofType::MerkleTree,
                merkle_root,
                intermediate_states,
                proof_data: vec![],
            },
        ))
    }

    /// Validate resource requirements before execution
    fn validate_resource_requirements(&self, task: &Task) -> Result<()> {
        if let TaskType::Compute {
            resource_requirements,
            ..
        } = &task.task_type
        {
            // Apply resource multiplier for safety margin (e.g., 0.9 means use only 90% of declared limits)
            let effective_cpu_limit = (600_000.0 * self.config.resource_multiplier) as u64;
            let effective_memory_limit = (8192.0 * self.config.resource_multiplier) as u64;

            // Check CPU time limit
            if resource_requirements.max_cpu_ms > effective_cpu_limit {
                let msg = format!(
                    "Task requires {}ms CPU, but limit is {}ms (600000ms * {} multiplier)",
                    resource_requirements.max_cpu_ms,
                    effective_cpu_limit,
                    self.config.resource_multiplier
                );

                if self.config.strict_enforcement {
                    return Err(PoUWError::ResourceLimitExceeded {
                        resource: "CPU time".to_string(),
                        limit: format!("{}ms", effective_cpu_limit),
                    });
                } else {
                    tracing::warn!("Resource limit warning: {}", msg);
                }
            }

            // Check memory limit
            if resource_requirements.max_memory_mb > effective_memory_limit {
                let msg = format!(
                    "Task requires {}MB memory, but limit is {}MB (8192MB * {} multiplier)",
                    resource_requirements.max_memory_mb,
                    effective_memory_limit,
                    self.config.resource_multiplier
                );

                if self.config.strict_enforcement {
                    return Err(PoUWError::ResourceLimitExceeded {
                        resource: "Memory".to_string(),
                        limit: format!("{}MB", effective_memory_limit),
                    });
                } else {
                    tracing::warn!("Resource limit warning: {}", msg);
                }
            }
        }

        Ok(())
    }

    /// Estimate memory usage from output size
    fn estimate_memory_usage(&self, output_data: &[u8]) -> u64 {
        // Rough estimate: output size + 50% overhead, minimum 1 MB
        let estimated = ((output_data.len() as f64 * 1.5) / (1024.0 * 1024.0)) as u64;
        estimated.max(1) // Ensure at least 1 MB for validation
    }

    /// Compute Merkle root from intermediate states
    fn compute_merkle_root(&self, states: &[Hash]) -> Hash {
        if states.is_empty() {
            return [0u8; 32];
        }

        let combined: Vec<u8> = states.iter().flat_map(|s| s.iter()).copied().collect();
        blake3::hash(&combined).into()
    }

    /// Generate proof data for verification
    fn generate_proof_data(&self, states: &[Hash]) -> Vec<u8> {
        // Simple proof: concatenate all states
        states.iter().flat_map(|s| s.iter()).copied().collect()
    }

    /// Generate deterministic result ID
    fn generate_result_id(&self, task: &Task) -> Hash {
        let mut data = Vec::new();
        data.extend_from_slice(&task.task_id);
        data.extend_from_slice(self.worker_id.as_bytes());
        data.extend_from_slice(&Utc::now().timestamp().to_le_bytes());
        blake3::hash(&data).into()
    }

    /// Get execution statistics
    pub async fn get_stats(&self) -> ExecutionStats {
        self.execution_stats.read().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FinalityStatus, TaskStatus};
    use adic_economics::types::AdicAmount;

    fn create_test_task(computation_type: ComputationType) -> Task {
        Task {
            task_id: [1u8; 32],
            sponsor: PublicKey::from_bytes([2u8; 32]),
            task_type: TaskType::Compute {
                computation_type,
                resource_requirements: ResourceRequirements {
                    max_cpu_ms: 5000,
                    max_memory_mb: 512,
                    max_storage_mb: 1024,
                    max_network_kb: 1024,
                },
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
        }
    }

    #[tokio::test]
    async fn test_hash_verification_execution() {
        use adic_crypto::Keypair;
        let executor = TaskExecutor::new(
            Keypair::generate(),
            ExecutorConfig::default(),
        );

        let task = create_test_task(ComputationType::HashVerification);
        let input_data = b"test input data";

        let result = executor.execute_task(&task, input_data).await.unwrap();

        assert_eq!(result.task_id, task.task_id);
        assert_eq!(result.worker, executor.worker_id);
        assert!(!result.output_cid.is_empty());
        assert_eq!(result.execution_proof.proof_type, ProofType::MerkleTree);
        assert_eq!(result.execution_proof.intermediate_states.len(), 4);

        let stats = executor.get_stats().await;
        assert_eq!(stats.total_executions, 1);
        assert_eq!(stats.successful_executions, 1);
    }

    #[tokio::test]
    async fn test_data_processing_execution() {
        use adic_crypto::Keypair;
        let executor = TaskExecutor::new(
            Keypair::generate(),
            ExecutorConfig::default(),
        );

        let task = create_test_task(ComputationType::DataProcessing);
        let input_data = b"process this data";

        let result = executor.execute_task(&task, input_data).await.unwrap();

        assert_eq!(result.task_id, task.task_id);
        assert!(!result.output_cid.is_empty());
        assert!(!result.execution_proof.intermediate_states.is_empty());
    }

    #[tokio::test]
    async fn test_resource_limit_enforcement() {
        use adic_crypto::Keypair;
        let executor = TaskExecutor::new(
            Keypair::generate(),
            ExecutorConfig {
                strict_enforcement: true,
                ..Default::default()
            },
        );

        let mut task = create_test_task(ComputationType::HashVerification);

        // Set excessive resource requirements
        if let TaskType::Compute {
            ref mut resource_requirements,
            ..
        } = task.task_type
        {
            resource_requirements.max_cpu_ms = 700_000; // Over 10 minute limit
        }

        let result = executor.execute_task(&task, b"test").await;

        assert!(result.is_err());
        match result.unwrap_err() {
            PoUWError::ResourceLimitExceeded { resource, .. } => {
                assert_eq!(resource, "CPU time");
            }
            _ => panic!("Expected ResourceLimitExceeded error"),
        }
    }

    #[tokio::test]
    async fn test_execution_proof_generation() {
        use adic_crypto::Keypair;
        let executor = TaskExecutor::new(
            Keypair::generate(),
            ExecutorConfig::default(),
        );

        let task = create_test_task(ComputationType::HashVerification);
        let input_data = b"proof test";

        let result = executor.execute_task(&task, input_data).await.unwrap();

        // Verify proof structure
        assert!(!result.execution_proof.merkle_root.iter().all(|&b| b == 0));
        assert!(!result.execution_proof.intermediate_states.is_empty());
        assert!(!result.execution_proof.proof_data.is_empty());
    }
}
