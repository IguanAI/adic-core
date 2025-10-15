//! Proof Aggregation
//!
//! This module implements Sprint 10 from storage-market-design.md:
//! - Batched storage proofs for multiple deals/epochs
//! - Checkpoint proofs for long-term deals
//! - Proof compression and optimization
//!
//! ## Benefits
//!
//! - **Reduced overhead**: 99% reduction in proof messages for long-term deals
//! - **Lower costs**: Single message for multiple proofs
//! - **Efficient verification**: Aggregate Merkle root validation
//!
//! ## Proof Types
//!
//! 1. **BatchedStorageProof**: Multiple deals + epochs in one message
//! 2. **CheckpointProof**: Periodic checkpoint (every 1000 epochs)

use crate::error::{Result, StorageMarketError};
use crate::types::{FinalityStatus, Hash, MessageHash, MerkleProof};
use adic_economics::types::{AccountAddress, AdicAmount};
use adic_types::Signature;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Individual proof within a batch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndividualProof {
    pub deal_id: u64,
    pub epoch: u64,
    pub challenge_indices: Vec<u64>,
    pub merkle_proofs: Vec<MerkleProof>,
}

/// Batched storage proof (multiple deals and/or epochs)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchedStorageProof {
    // ========== Protocol Fields ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Batch Fields ==========
    /// Provider submitting batch
    pub provider: AccountAddress,
    /// Deal IDs included in batch
    pub deal_ids: Vec<u64>,
    /// Epoch range (start, end) inclusive
    pub epoch_range: (u64, u64),
    /// Aggregated Merkle root over all individual proofs
    pub aggregated_merkle_root: Hash,
    /// Individual proofs for each deal/epoch combination
    pub individual_proofs: Vec<IndividualProof>,
    /// Batch ID for tracking
    pub batch_id: Hash,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
}

/// Checkpoint proof (periodic verification for long-term deals)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointProof {
    // ========== Protocol Fields ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Checkpoint Fields ==========
    /// Deal being checkpointed
    pub deal_id: u64,
    /// Provider submitting checkpoint
    pub provider: AccountAddress,
    /// Checkpoint epoch (e.g., every 1000 epochs)
    pub checkpoint_epoch: u64,
    /// Merkle roots since last checkpoint
    pub merkle_roots_since_last: Vec<Hash>,
    /// Number of proofs covered by this checkpoint
    pub proofs_covered: u64,
    /// Combined proof data (aggregated)
    pub combined_merkle_root: Hash,
    /// Checkpoint ID
    pub checkpoint_id: Hash,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
}

impl BatchedStorageProof {
    pub fn new(
        provider: AccountAddress,
        deal_ids: Vec<u64>,
        epoch_range: (u64, u64),
        individual_proofs: Vec<IndividualProof>,
    ) -> Self {
        let now = chrono::Utc::now().timestamp();

        // Compute aggregated Merkle root over all individual proofs
        let aggregated_root = Self::compute_aggregate_root(&individual_proofs);

        // Generate batch ID
        let mut batch_data = Vec::new();
        batch_data.extend_from_slice(provider.as_bytes());
        batch_data.extend_from_slice(&epoch_range.0.to_le_bytes());
        batch_data.extend_from_slice(&epoch_range.1.to_le_bytes());
        batch_data.extend_from_slice(&now.to_le_bytes());
        let batch_id = blake3::hash(&batch_data).into();

        Self {
            approvals: [MessageHash::default(); 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO, // Reduced deposit for batches
            timestamp: now,
            provider,
            deal_ids,
            epoch_range,
            aggregated_merkle_root: aggregated_root,
            individual_proofs,
            batch_id,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
        }
    }

    /// Compute aggregate Merkle root from individual proofs
    fn compute_aggregate_root(proofs: &[IndividualProof]) -> Hash {
        let mut leaves = Vec::new();
        for proof in proofs {
            let mut proof_data = Vec::new();
            proof_data.extend_from_slice(&proof.deal_id.to_le_bytes());
            proof_data.extend_from_slice(&proof.epoch.to_le_bytes());
            for merkle_proof in &proof.merkle_proofs {
                proof_data.extend_from_slice(&merkle_proof.chunk_data);
            }
            leaves.push(*blake3::hash(&proof_data).as_bytes());
        }

        // Build Merkle tree from leaves
        if leaves.is_empty() {
            return [0u8; 32];
        }

        let mut current_level = leaves;
        while current_level.len() > 1 {
            let mut next_level = Vec::new();
            for i in (0..current_level.len()).step_by(2) {
                if i + 1 < current_level.len() {
                    let mut combined = Vec::new();
                    combined.extend_from_slice(&current_level[i]);
                    combined.extend_from_slice(&current_level[i + 1]);
                    next_level.push(*blake3::hash(&combined).as_bytes());
                } else {
                    next_level.push(current_level[i]);
                }
            }
            current_level = next_level;
        }

        current_level[0]
    }

    pub fn is_finalized(&self) -> bool {
        self.finality_status.is_finalized()
    }

    /// Get number of proofs in batch
    pub fn proof_count(&self) -> usize {
        self.individual_proofs.len()
    }

    /// Get savings compared to individual proofs
    pub fn get_savings_ratio(&self) -> f64 {
        let individual_count = self.individual_proofs.len();
        if individual_count == 0 {
            return 0.0;
        }
        // 1 batched message vs N individual messages
        1.0 - (1.0 / individual_count as f64)
    }
}

impl CheckpointProof {
    pub fn new(
        deal_id: u64,
        provider: AccountAddress,
        checkpoint_epoch: u64,
        merkle_roots_since_last: Vec<Hash>,
    ) -> Self {
        let now = chrono::Utc::now().timestamp();

        // Compute combined Merkle root
        let combined_root = Self::compute_combined_root(&merkle_roots_since_last);

        // Generate checkpoint ID
        let mut checkpoint_data = Vec::new();
        checkpoint_data.extend_from_slice(&deal_id.to_le_bytes());
        checkpoint_data.extend_from_slice(provider.as_bytes());
        checkpoint_data.extend_from_slice(&checkpoint_epoch.to_le_bytes());
        let checkpoint_id = blake3::hash(&checkpoint_data).into();

        Self {
            approvals: [MessageHash::default(); 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: now,
            deal_id,
            provider,
            checkpoint_epoch,
            merkle_roots_since_last: merkle_roots_since_last.clone(),
            proofs_covered: merkle_roots_since_last.len() as u64,
            combined_merkle_root: combined_root,
            checkpoint_id,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
        }
    }

    /// Compute combined Merkle root from individual roots
    fn compute_combined_root(roots: &[Hash]) -> Hash {
        if roots.is_empty() {
            return [0u8; 32];
        }

        let mut current_level: Vec<Hash> = roots.to_vec();
        while current_level.len() > 1 {
            let mut next_level = Vec::new();
            for i in (0..current_level.len()).step_by(2) {
                if i + 1 < current_level.len() {
                    let mut combined = Vec::new();
                    combined.extend_from_slice(&current_level[i]);
                    combined.extend_from_slice(&current_level[i + 1]);
                    next_level.push(*blake3::hash(&combined).as_bytes());
                } else {
                    next_level.push(current_level[i]);
                }
            }
            current_level = next_level;
        }

        current_level[0]
    }

    pub fn is_finalized(&self) -> bool {
        self.finality_status.is_finalized()
    }

    /// Calculate savings compared to individual proofs
    pub fn get_savings_ratio(&self) -> f64 {
        if self.proofs_covered == 0 {
            return 0.0;
        }
        // 1 checkpoint vs N individual proofs
        1.0 - (1.0 / self.proofs_covered as f64)
    }
}

/// Aggregation configuration
#[derive(Debug, Clone)]
pub struct AggregationConfig {
    /// Maximum proofs per batch
    pub max_proofs_per_batch: usize,
    /// Checkpoint interval (epochs)
    pub checkpoint_interval: u64,
    /// Minimum proofs for batching
    pub min_batch_size: usize,
    /// Enable automatic batching
    pub auto_batch: bool,
}

impl Default for AggregationConfig {
    fn default() -> Self {
        Self {
            max_proofs_per_batch: 100,
            checkpoint_interval: 1000, // Every 1000 epochs
            min_batch_size: 5,
            auto_batch: true,
        }
    }
}

/// Aggregation manager
pub struct AggregationManager {
    config: AggregationConfig,
    batched_proofs: Arc<RwLock<HashMap<Hash, BatchedStorageProof>>>,
    checkpoints: Arc<RwLock<HashMap<Hash, CheckpointProof>>>,
    // Track pending proofs for batching
    pending_proofs: Arc<RwLock<HashMap<AccountAddress, Vec<IndividualProof>>>>,
}

impl AggregationManager {
    pub fn new(config: AggregationConfig) -> Self {
        Self {
            config,
            batched_proofs: Arc::new(RwLock::new(HashMap::new())),
            checkpoints: Arc::new(RwLock::new(HashMap::new())),
            pending_proofs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add proof to pending queue for batching
    pub async fn add_pending_proof(
        &self,
        provider: AccountAddress,
        proof: IndividualProof,
    ) -> Result<()> {
        let mut pending = self.pending_proofs.write().await;
        pending.entry(provider).or_insert_with(Vec::new).push(proof);
        Ok(())
    }

    /// Create batched proof from pending proofs
    pub async fn create_batch(
        &self,
        provider: AccountAddress,
        epoch_range: (u64, u64),
    ) -> Result<BatchedStorageProof> {
        let mut pending = self.pending_proofs.write().await;
        let proofs = pending.remove(&provider).unwrap_or_default();

        if proofs.is_empty() {
            return Err(StorageMarketError::InvalidMessage(
                "No pending proofs to batch".into(),
            ));
        }

        if proofs.len() < self.config.min_batch_size {
            return Err(StorageMarketError::InvalidMessage(
                format!(
                    "Not enough proofs for batch: {} < {}",
                    proofs.len(),
                    self.config.min_batch_size
                ),
            ));
        }

        // Extract deal IDs
        let deal_ids: Vec<u64> = proofs.iter().map(|p| p.deal_id).collect();

        let batch = BatchedStorageProof::new(provider, deal_ids, epoch_range, proofs);

        // Store batch
        self.batched_proofs.write().await.insert(batch.batch_id, batch.clone());

        info!(
            "Created batched proof {} with {} proofs ({:.1}% savings)",
            hex::encode(&batch.batch_id[..8]),
            batch.proof_count(),
            batch.get_savings_ratio() * 100.0
        );

        Ok(batch)
    }

    /// Submit batched proof
    pub async fn submit_batch(&self, batch: BatchedStorageProof) -> Result<Hash> {
        // Validate batch
        if batch.individual_proofs.is_empty() {
            return Err(StorageMarketError::InvalidMessage(
                "Batch must contain at least one proof".into(),
            ));
        }

        if batch.individual_proofs.len() > self.config.max_proofs_per_batch {
            return Err(StorageMarketError::InvalidMessage(
                format!(
                    "Batch size {} exceeds maximum {}",
                    batch.individual_proofs.len(),
                    self.config.max_proofs_per_batch
                ),
            ));
        }

        // Verify aggregate root
        let computed_root = BatchedStorageProof::compute_aggregate_root(&batch.individual_proofs);
        if computed_root != batch.aggregated_merkle_root {
            return Err(StorageMarketError::ProofVerificationFailed(
                "Aggregate Merkle root mismatch".into(),
            ));
        }

        let batch_id = batch.batch_id;
        self.batched_proofs.write().await.insert(batch_id, batch);

        info!(
            "Submitted batched proof {}",
            hex::encode(&batch_id[..8])
        );

        Ok(batch_id)
    }

    /// Create checkpoint proof
    pub async fn create_checkpoint(
        &self,
        deal_id: u64,
        provider: AccountAddress,
        checkpoint_epoch: u64,
        merkle_roots: Vec<Hash>,
    ) -> Result<CheckpointProof> {
        if merkle_roots.is_empty() {
            return Err(StorageMarketError::InvalidMessage(
                "Checkpoint must contain at least one Merkle root".into(),
            ));
        }

        let checkpoint = CheckpointProof::new(deal_id, provider, checkpoint_epoch, merkle_roots);

        self.checkpoints.write().await.insert(checkpoint.checkpoint_id, checkpoint.clone());

        info!(
            "Created checkpoint {} for deal {} ({:.1}% savings)",
            hex::encode(&checkpoint.checkpoint_id[..8]),
            deal_id,
            checkpoint.get_savings_ratio() * 100.0
        );

        Ok(checkpoint)
    }

    /// Submit checkpoint proof
    pub async fn submit_checkpoint(&self, checkpoint: CheckpointProof) -> Result<Hash> {
        // Validate checkpoint
        if checkpoint.merkle_roots_since_last.is_empty() {
            return Err(StorageMarketError::InvalidMessage(
                "Checkpoint must contain Merkle roots".into(),
            ));
        }

        // Verify combined root
        let computed_root =
            CheckpointProof::compute_combined_root(&checkpoint.merkle_roots_since_last);
        if computed_root != checkpoint.combined_merkle_root {
            return Err(StorageMarketError::ProofVerificationFailed(
                "Combined Merkle root mismatch".into(),
            ));
        }

        let checkpoint_id = checkpoint.checkpoint_id;
        self.checkpoints.write().await.insert(checkpoint_id, checkpoint);

        info!(
            "Submitted checkpoint proof {}",
            hex::encode(&checkpoint_id[..8])
        );

        Ok(checkpoint_id)
    }

    /// Get batched proof by ID
    pub async fn get_batch(&self, batch_id: &Hash) -> Option<BatchedStorageProof> {
        self.batched_proofs.read().await.get(batch_id).cloned()
    }

    /// Get checkpoint by ID
    pub async fn get_checkpoint(&self, checkpoint_id: &Hash) -> Option<CheckpointProof> {
        self.checkpoints.read().await.get(checkpoint_id).cloned()
    }

    /// Get statistics
    pub async fn get_stats(&self) -> AggregationStats {
        let batches = self.batched_proofs.read().await;
        let checkpoints = self.checkpoints.read().await;
        let pending = self.pending_proofs.read().await;

        let total_proofs_batched: usize = batches.values().map(|b| b.proof_count()).sum();
        let total_proofs_checkpointed: u64 = checkpoints.values().map(|c| c.proofs_covered).sum();

        let avg_batch_size = if !batches.is_empty() {
            total_proofs_batched as f64 / batches.len() as f64
        } else {
            0.0
        };

        let total_pending: usize = pending.values().map(|v| v.len()).sum();

        AggregationStats {
            total_batches: batches.len(),
            total_checkpoints: checkpoints.len(),
            total_proofs_batched,
            total_proofs_checkpointed: total_proofs_checkpointed as usize,
            avg_batch_size,
            pending_proofs: total_pending,
        }
    }
}

/// Aggregation statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregationStats {
    pub total_batches: usize,
    pub total_checkpoints: usize,
    pub total_proofs_batched: usize,
    pub total_proofs_checkpointed: usize,
    pub avg_batch_size: f64,
    pub pending_proofs: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_individual_proof(deal_id: u64, epoch: u64) -> IndividualProof {
        IndividualProof {
            deal_id,
            epoch,
            challenge_indices: vec![0, 1, 2],
            merkle_proofs: vec![
                MerkleProof {
                    chunk_index: 0,
                    chunk_data: vec![1, 2, 3],
                    sibling_hashes: vec![[4u8; 32], [5u8; 32]],
                },
            ],
        }
    }

    #[tokio::test]
    async fn test_create_batched_proof() {
        let provider = AccountAddress::from_bytes([1u8; 32]);
        let proofs = vec![
            create_individual_proof(1, 100),
            create_individual_proof(1, 101),
            create_individual_proof(2, 100),
        ];

        let batch = BatchedStorageProof::new(provider, vec![1, 2], (100, 101), proofs);

        assert_eq!(batch.deal_ids, vec![1, 2]);
        assert_eq!(batch.epoch_range, (100, 101));
        assert_eq!(batch.proof_count(), 3);
        assert!(batch.get_savings_ratio() > 0.6); // 1 message vs 3 = 66% savings
    }

    #[tokio::test]
    async fn test_create_checkpoint() {
        let deal_id = 1;
        let provider = AccountAddress::from_bytes([2u8; 32]);
        let roots = vec![[1u8; 32], [2u8; 32], [3u8; 32]];

        let checkpoint = CheckpointProof::new(deal_id, provider, 1000, roots);

        assert_eq!(checkpoint.deal_id, deal_id);
        assert_eq!(checkpoint.checkpoint_epoch, 1000);
        assert_eq!(checkpoint.proofs_covered, 3);
        assert!(checkpoint.get_savings_ratio() > 0.6);
    }

    #[tokio::test]
    async fn test_aggregation_manager() {
        let manager = AggregationManager::new(AggregationConfig {
            min_batch_size: 2,
            ..Default::default()
        });
        let provider = AccountAddress::from_bytes([1u8; 32]);

        // Add pending proofs
        manager
            .add_pending_proof(provider, create_individual_proof(1, 100))
            .await
            .unwrap();
        manager
            .add_pending_proof(provider, create_individual_proof(1, 101))
            .await
            .unwrap();

        // Create batch
        let batch = manager.create_batch(provider, (100, 101)).await.unwrap();
        assert_eq!(batch.proof_count(), 2);

        // Get stats
        let stats = manager.get_stats().await;
        assert_eq!(stats.total_batches, 1);
        assert_eq!(stats.total_proofs_batched, 2);
    }

    #[tokio::test]
    async fn test_batch_validation() {
        let manager = AggregationManager::new(AggregationConfig::default());
        let provider = AccountAddress::from_bytes([1u8; 32]);

        // Empty batch should fail
        let empty_batch = BatchedStorageProof::new(provider, vec![], (100, 101), vec![]);
        assert!(manager.submit_batch(empty_batch).await.is_err());
    }

    #[tokio::test]
    async fn test_checkpoint_validation() {
        let manager = AggregationManager::new(AggregationConfig::default());

        // Empty checkpoint should fail
        let checkpoint = CheckpointProof::new(
            1,
            AccountAddress::from_bytes([1u8; 32]),
            1000,
            vec![],
        );
        assert!(manager.submit_checkpoint(checkpoint).await.is_err());
    }
}
