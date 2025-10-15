use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock, Semaphore};
use tracing::{info, warn};

use adic_finality::FinalityEngine;
use adic_storage::StorageEngine;
use adic_types::{AdicError, AdicMessage, MessageId, Result};
use libp2p::PeerId;

#[derive(Debug, Clone)]
pub struct StateSyncConfig {
    pub batch_size: usize,
    pub parallel_downloads: usize,
    pub checkpoint_interval: u64,
    pub sync_timeout: Duration,
    pub retry_attempts: usize,
}

impl Default for StateSyncConfig {
    fn default() -> Self {
        Self {
            batch_size: 1000,
            parallel_downloads: 4,
            checkpoint_interval: 1000,
            sync_timeout: Duration::from_secs(60),
            retry_attempts: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub height: u64,
    pub root_hash: [u8; 32],
    pub finalized_messages: Vec<MessageId>,
    pub k_core_value: u64,
    pub timestamp: u64,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SyncState {
    pub local_height: u64,
    pub target_height: u64,
    pub synced_messages: usize,
    pub pending_messages: HashSet<MessageId>,
    pub verified_checkpoints: HashMap<u64, Checkpoint>,
    pub sync_start_time: Option<Instant>,
    pub is_syncing: bool,
    pub sync_peer: Option<PeerId>,
}

impl SyncState {
    fn new() -> Self {
        Self {
            local_height: 0,
            target_height: 0,
            synced_messages: 0,
            pending_messages: HashSet::new(),
            verified_checkpoints: HashMap::new(),
            sync_start_time: None,
            is_syncing: false,
            sync_peer: None,
        }
    }

    pub fn progress(&self) -> f64 {
        if self.target_height == 0 {
            return 0.0;
        }
        (self.local_height as f64 / self.target_height as f64).min(1.0)
    }
}

pub struct StateSync {
    config: StateSyncConfig,
    state: Arc<RwLock<SyncState>>,
    storage: Arc<StorageEngine>,
    finality_engine: Arc<FinalityEngine>,
    download_semaphore: Arc<Semaphore>,
    event_sender: mpsc::UnboundedSender<SyncEvent>,
    event_receiver: Arc<RwLock<mpsc::UnboundedReceiver<SyncEvent>>>,
}

#[derive(Debug, Clone)]
pub enum SyncEvent {
    SyncStarted(PeerId, u64),          // peer, target height
    SyncProgress(PeerId, usize, f64),  // peer, synced_messages, progress (0.0 to 1.0)
    CheckpointVerified(u64),           // height
    MessagesDownloaded(usize),         // count
    SyncCompleted(PeerId, usize, u64), // peer, synced_messages, duration_ms
    SyncFailed(String),
}

impl StateSync {
    pub fn new(
        config: StateSyncConfig,
        storage: Arc<StorageEngine>,
        finality_engine: Arc<FinalityEngine>,
    ) -> Self {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();

        Self {
            config: config.clone(),
            state: Arc::new(RwLock::new(SyncState::new())),
            storage,
            finality_engine,
            download_semaphore: Arc::new(Semaphore::new(config.parallel_downloads)),
            event_sender,
            event_receiver: Arc::new(RwLock::new(event_receiver)),
        }
    }

    pub async fn start_fast_sync(&self, peer: PeerId, target_height: u64) -> Result<()> {
        let mut state = self.state.write().await;
        if state.is_syncing {
            return Err(AdicError::Network("Sync already in progress".to_string()));
        }

        let local_height_before = state.local_height;
        state.is_syncing = true;
        state.target_height = target_height;
        state.sync_start_time = Some(Instant::now());
        state.sync_peer = Some(peer);

        self.event_sender
            .send(SyncEvent::SyncStarted(peer, target_height))
            .ok();

        info!(
            peer_id = %peer,
            local_height_before = local_height_before,
            target_height = target_height,
            height_deficit = target_height.saturating_sub(local_height_before),
            batch_size = self.config.batch_size,
            parallel_downloads = self.config.parallel_downloads,
            "🚀 Fast sync started"
        );

        // Start sync process
        let state_clone = self.state.clone();
        let _storage_clone = self.storage.clone();
        let event_sender = self.event_sender.clone();
        let peer_clone = peer;

        tokio::spawn(async move {
            // In real implementation, download checkpoints and verify

            // Simulate sync with progress updates
            let steps = 10;
            for i in 1..=steps {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let progress = i as f64 / steps as f64;

                // Update state progress
                let synced_messages = {
                    let mut state = state_clone.write().await;
                    state.local_height = local_height_before
                        + ((target_height - local_height_before) as f64 * progress) as u64;
                    state.synced_messages = (progress * 1000.0) as usize; // Simulate message count
                    state.synced_messages
                };

                event_sender
                    .send(SyncEvent::SyncProgress(
                        peer_clone,
                        synced_messages,
                        progress,
                    ))
                    .ok();
            }

            let duration_ms = {
                let mut state = state_clone.write().await;
                state.is_syncing = false;
                state.local_height = target_height;
                state
                    .sync_start_time
                    .map(|start| start.elapsed().as_millis() as u64)
                    .unwrap_or(0)
            };

            let synced_messages = state_clone.read().await.synced_messages;
            event_sender
                .send(SyncEvent::SyncCompleted(
                    peer_clone,
                    synced_messages,
                    duration_ms,
                ))
                .ok();
        });

        Ok(())
    }

    pub async fn start_incremental_sync(&self, frontier: Vec<MessageId>) -> Result<()> {
        let mut state = self.state.write().await;
        if state.is_syncing {
            return Err(AdicError::Network("Sync already in progress".to_string()));
        }

        let frontier_count = frontier.len();
        let synced_before = state.synced_messages;
        state.is_syncing = true;
        state.pending_messages = frontier.into_iter().collect();
        state.sync_start_time = Some(Instant::now());

        info!(
            frontier_messages = frontier_count,
            synced_before = synced_before,
            local_height = state.local_height,
            checkpoint_count = state.verified_checkpoints.len(),
            "🔄 Incremental sync started"
        );

        Ok(())
    }

    pub async fn download_messages(&self, message_ids: Vec<MessageId>) -> Vec<AdicMessage> {
        let start_time = Instant::now();
        let total_messages = message_ids.len();
        let batch_count = (total_messages + self.config.batch_size - 1) / self.config.batch_size;

        let permits = self.download_semaphore.clone();
        let storage = self.storage.clone();
        let mut handles = Vec::new();

        for batch in message_ids.chunks(self.config.batch_size) {
            let batch = batch.to_vec();
            let permit = permits.clone().acquire_owned().await.unwrap();
            let storage_clone = storage.clone();

            handles.push(tokio::spawn(async move {
                let mut messages = Vec::new();

                // Fetch messages from local storage
                for message_id in batch {
                    match storage_clone.get_message(&message_id).await {
                        Ok(Some(msg)) => messages.push(msg),
                        Ok(None) => {
                            // Message not in local storage - would need network download
                            // This should be handled by the network layer's sync protocol
                        }
                        Err(e) => {
                            warn!(
                                message_id = %message_id,
                                error = %e,
                                "Failed to retrieve message from storage"
                            );
                        }
                    }
                }

                drop(permit);
                messages
            }));
        }

        let mut all_messages = Vec::new();
        let mut failed_batches = 0;
        for handle in handles {
            match handle.await {
                Ok(messages) => all_messages.extend(messages),
                Err(_) => failed_batches += 1,
            }
        }

        let elapsed = start_time.elapsed();
        let missing_count = total_messages.saturating_sub(all_messages.len());

        info!(
            requested_messages = total_messages,
            retrieved_messages = all_messages.len(),
            missing_messages = missing_count,
            batch_count = batch_count,
            failed_batches = failed_batches,
            duration_ms = elapsed.as_millis(),
            throughput_msg_per_sec = if elapsed.as_secs_f64() > 0.0 {
                all_messages.len() as f64 / elapsed.as_secs_f64()
            } else {
                0.0
            },
            "📦 Messages retrieved from local storage"
        );

        self.event_sender
            .send(SyncEvent::MessagesDownloaded(all_messages.len()))
            .ok();

        all_messages
    }

    pub async fn verify_checkpoint(&self, checkpoint: Checkpoint) -> Result<bool> {
        // Verify checkpoint signature
        // In real implementation, verify against known validators
        let start_time = Instant::now();

        // Verify merkle root
        let calculated_root = self
            .calculate_merkle_root(&checkpoint.finalized_messages)
            .await?;
        let root_matches = calculated_root == checkpoint.root_hash;

        if !root_matches {
            warn!(
                checkpoint_height = checkpoint.height,
                expected_root = hex::encode(checkpoint.root_hash),
                calculated_root = hex::encode(calculated_root),
                "❌ Checkpoint verification failed (root mismatch)"
            );
            return Ok(false);
        }

        // Store verified checkpoint
        let mut state = self.state.write().await;
        let checkpoints_before = state.verified_checkpoints.len();
        state
            .verified_checkpoints
            .insert(checkpoint.height, checkpoint.clone());

        let elapsed = start_time.elapsed();
        info!(
            checkpoint_height = checkpoint.height,
            message_count = checkpoint.finalized_messages.len(),
            k_core = checkpoint.k_core_value,
            checkpoints_before = checkpoints_before,
            checkpoints_after = state.verified_checkpoints.len(),
            verification_time_ms = elapsed.as_millis(),
            "✅ Checkpoint verified"
        );

        self.event_sender
            .send(SyncEvent::CheckpointVerified(checkpoint.height))
            .ok();

        Ok(true)
    }

    async fn calculate_merkle_root(&self, message_ids: &[MessageId]) -> Result<[u8; 32]> {
        // Simple merkle root calculation
        use blake3::Hasher;

        let mut hasher = Hasher::new();
        for id in message_ids {
            hasher.update(id.as_bytes());
        }

        Ok(*hasher.finalize().as_bytes())
    }

    pub async fn apply_checkpoint(&self, checkpoint: &Checkpoint) -> Result<()> {
        let start_time = Instant::now();
        let total_messages = checkpoint.finalized_messages.len();
        let mut applied = 0;
        let mut missing = 0;

        // Apply checkpoint to storage and finality engine
        for message_id in &checkpoint.finalized_messages {
            // Verify message exists in storage
            let message_exists = self
                .storage
                .get_message(message_id)
                .await
                .map_err(|e| AdicError::Storage(format!("Failed to get message: {}", e)))?;
            if message_exists.is_none() {
                missing += 1;
                warn!(
                    message_id = %message_id,
                    checkpoint_height = checkpoint.height,
                    "Message missing for checkpoint"
                );
                continue;
            }

            // Add to finality engine with proper parameters
            let parents = self
                .storage
                .get_parents(message_id)
                .await
                .map_err(|e| AdicError::Storage(format!("Failed to get parents: {}", e)))?;
            let ball_ids = HashMap::new(); // Empty ball IDs for checkpoint messages

            self.finality_engine
                .add_message(
                    *message_id,
                    parents,
                    100.0, // High reputation for checkpoint messages
                    ball_ids,
                )
                .await?;

            applied += 1;
        }

        // Run finality check to process the checkpoint messages
        let finalized = self.finality_engine.check_finality().await?;
        let elapsed = start_time.elapsed();

        info!(
            checkpoint_height = checkpoint.height,
            total_messages = total_messages,
            applied_messages = applied,
            missing_messages = missing,
            finalized_count = finalized.len(),
            duration_ms = elapsed.as_millis(),
            success_rate = if total_messages > 0 {
                applied as f64 / total_messages as f64
            } else {
                1.0
            },
            "✅ Checkpoint applied"
        );

        // Update sync state
        let mut state = self.state.write().await;
        state.local_height = checkpoint.height;
        state.synced_messages += checkpoint.finalized_messages.len();

        let progress = state.progress();
        let synced_messages = state.synced_messages;

        // Emit progress event with tracked peer
        if let Some(peer) = state.sync_peer {
            self.event_sender
                .send(SyncEvent::SyncProgress(
                    peer,
                    synced_messages,
                    progress,
                ))
                .ok();
        }

        info!(
            "Applied checkpoint at height {} ({:.1}% complete)",
            checkpoint.height,
            progress * 100.0
        );

        Ok(())
    }

    pub async fn create_checkpoint(&self, height: u64) -> Result<Checkpoint> {
        // Get all messages up to this height from storage
        let all_messages = self
            .storage
            .list_all_messages()
            .await
            .map_err(|e| AdicError::Storage(format!("Failed to list messages: {}", e)))?;

        // Filter for finalized messages
        let mut finalized = Vec::new();
        for msg_id in all_messages {
            if self.finality_engine.is_finalized(&msg_id).await {
                // Get message to check its height/timestamp
                let msg_result = self
                    .storage
                    .get_message(&msg_id)
                    .await
                    .map_err(|e| AdicError::Storage(format!("Failed to get message: {}", e)))?;
                if let Some(_msg) = msg_result {
                    // Check if message height is at or below checkpoint height
                    if let Ok(Some(msg_height)) = self.storage.get_message_height(&msg_id).await {
                        if msg_height <= height {
                            finalized.push(msg_id);
                        }
                    }
                }
            }
        }

        // Calculate merkle root
        let root_hash = self.calculate_merkle_root(&finalized).await?;

        // Use a default k-core value based on finalized count
        let stats = self.finality_engine.get_stats().await;
        // Use finalized_count as a proxy for k-core value, or default to 3
        let k_core_value = if stats.finalized_count > 0 {
            (stats.finalized_count / 10).max(3) as u64
        } else {
            3
        };

        let checkpoint = Checkpoint {
            height,
            root_hash,
            finalized_messages: finalized,
            k_core_value,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            signature: vec![], // Would be signed by validator key
        };

        Ok(checkpoint)
    }

    pub async fn get_sync_status(&self) -> (bool, f64, Option<Duration>) {
        let state = self.state.read().await;

        let elapsed = state.sync_start_time.map(|start| start.elapsed());

        (state.is_syncing, state.progress(), elapsed)
    }

    pub async fn get_verified_checkpoints(&self) -> Vec<(u64, Checkpoint)> {
        let state = self.state.read().await;
        let mut checkpoints: Vec<_> = state
            .verified_checkpoints
            .iter()
            .map(|(h, c)| (*h, c.clone()))
            .collect();
        checkpoints.sort_by_key(|(h, _)| *h);
        checkpoints
    }

    pub async fn estimate_sync_time(&self) -> Option<Duration> {
        let state = self.state.read().await;

        if !state.is_syncing || state.sync_start_time.is_none() {
            return None;
        }

        let elapsed = state.sync_start_time.unwrap().elapsed();
        let progress = state.progress();

        if progress > 0.0 {
            let total_estimated = elapsed.as_secs_f64() / progress;
            let remaining = total_estimated - elapsed.as_secs_f64();
            Some(Duration::from_secs_f64(remaining))
        } else {
            None
        }
    }

    pub fn event_stream(&self) -> Arc<RwLock<mpsc::UnboundedReceiver<SyncEvent>>> {
        self.event_receiver.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sync_state() {
        let state = SyncState::new();
        assert_eq!(state.progress(), 0.0);
        assert!(!state.is_syncing);
    }

    #[tokio::test]
    async fn test_checkpoint_creation() {
        let _config = StateSyncConfig::default();
        // Would need mock storage and finality engine for full test

        let checkpoint = Checkpoint {
            height: 100,
            root_hash: [0; 32],
            finalized_messages: vec![],
            k_core_value: 20,
            timestamp: 0,
            signature: vec![],
        };

        assert_eq!(checkpoint.height, 100);
    }
}
