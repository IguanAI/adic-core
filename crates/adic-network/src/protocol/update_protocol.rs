use anyhow::{anyhow, Result};
use libp2p::PeerId;
use sha2::{Sha256, Digest};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock, Semaphore};
use tokio::time;
use tracing::{debug, info, warn};
use crate::protocol::update::{UpdateMessage, VersionInfo, BinaryChunkData};
use super::binary_store::{BinaryStore, BinaryMetadata};
use super::swarm_tracker::SwarmSpeedTracker;

/// Type alias for version-indexed chunk storage
type VersionChunks = HashMap<String, HashMap<u32, Vec<u8>>>;

/// Type alias for version-indexed chunk hashes
type VersionChunkHashes = HashMap<String, HashMap<u32, String>>;

/// Configuration for the update protocol
#[derive(Debug, Clone)]
pub struct UpdateProtocolConfig {
    /// Maximum concurrent chunk transfers
    pub max_concurrent_transfers: usize,
    /// Chunk request timeout
    pub chunk_timeout: Duration,
    /// Maximum retries per chunk
    pub max_retries: u32,
    /// Upload rate limit (bytes/sec), 0 = unlimited
    pub upload_rate_limit: u64,
    /// Download rate limit (bytes/sec), 0 = unlimited
    pub download_rate_limit: u64,
}

impl Default for UpdateProtocolConfig {
    fn default() -> Self {
        Self {
            max_concurrent_transfers: 5,
            chunk_timeout: Duration::from_secs(30),
            max_retries: 3,
            upload_rate_limit: 0,  // Unlimited by default
            download_rate_limit: 0, // Unlimited by default
        }
    }
}

/// Tracks a chunk transfer
#[derive(Debug, Clone)]
struct ChunkTransfer {
    pub peer: PeerId,
    pub attempts: u32,
    pub started_at: Instant,
    pub data: Option<Vec<u8>>,
    pub hash: Option<String>,
    pub verified: bool,
}

/// Manages P2P update protocol operations
pub struct UpdateProtocol {
    /// Protocol configuration
    config: UpdateProtocolConfig,

    /// Active chunk transfers
    active_transfers: Arc<RwLock<HashMap<(String, u32), ChunkTransfer>>>,

    /// Available versions from peers
    peer_versions: Arc<RwLock<HashMap<PeerId, Vec<VersionInfo>>>>,

    /// Peers that have specific versions
    version_peers: Arc<RwLock<HashMap<String, HashSet<PeerId>>>>,

    /// Downloaded chunks for each version
    downloaded_chunks: Arc<RwLock<VersionChunks>>,

    /// Chunk verification hashes
    chunk_hashes: Arc<RwLock<VersionChunkHashes>>,

    /// Transfer semaphore for rate limiting
    transfer_semaphore: Arc<Semaphore>,

    /// Event channel for protocol events
    event_sender: mpsc::UnboundedSender<UpdateProtocolEvent>,

    /// Binary store for chunk management
    binary_store: Arc<BinaryStore>,

    /// Swarm speed tracker for collective metrics
    swarm_tracker: Arc<SwarmSpeedTracker>,
}

/// Events emitted by the update protocol
#[derive(Debug, Clone)]
pub enum UpdateProtocolEvent {
    /// New version discovered
    VersionDiscovered(PeerId, VersionInfo),
    /// Chunk download started
    ChunkDownloadStarted(String, u32, PeerId),
    /// Chunk download completed
    ChunkDownloadCompleted(String, u32, usize),
    /// Chunk download failed
    ChunkDownloadFailed(String, u32, String),
    /// Binary download completed
    BinaryDownloadCompleted(String, Vec<u8>),
    /// Binary verification failed
    BinaryVerificationFailed(String, String),
}

impl UpdateProtocol {
    /// Create a new update protocol instance
    pub fn new(config: UpdateProtocolConfig, storage_dir: PathBuf, local_peer_id: PeerId) -> Result<(Self, mpsc::UnboundedReceiver<UpdateProtocolEvent>)> {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();

        // Create binary store for chunk management
        let binary_store = BinaryStore::new(storage_dir.join("binaries"))?;

        // Create swarm speed tracker
        let swarm_tracker = SwarmSpeedTracker::new(local_peer_id);

        let protocol = Self {
            config: config.clone(),
            active_transfers: Arc::new(RwLock::new(HashMap::new())),
            peer_versions: Arc::new(RwLock::new(HashMap::new())),
            version_peers: Arc::new(RwLock::new(HashMap::new())),
            downloaded_chunks: Arc::new(RwLock::new(HashMap::new())),
            chunk_hashes: Arc::new(RwLock::new(HashMap::new())),
            transfer_semaphore: Arc::new(Semaphore::new(config.max_concurrent_transfers)),
            event_sender,
            binary_store: Arc::new(binary_store),
            swarm_tracker: Arc::new(swarm_tracker),
        };

        Ok((protocol, event_receiver))
    }

    /// Handle incoming update message
    pub async fn handle_message(&self, message: UpdateMessage, from_peer: PeerId) -> Result<Option<UpdateMessage>> {
        match message {
            UpdateMessage::VersionAnnounce { version, binary_hash, signature, total_chunks } => {
                self.handle_version_announce(from_peer, version, binary_hash, signature, total_chunks).await?;
                Ok(None)
            }
            UpdateMessage::VersionQuery => {
                // Return our available versions
                let versions = self.get_local_versions().await;
                Ok(Some(UpdateMessage::VersionResponse {
                    current_version: self.get_current_version().await,
                    available_versions: versions,
                }))
            }
            UpdateMessage::VersionResponse { current_version, available_versions } => {
                self.handle_version_response(from_peer, current_version, available_versions).await?;
                Ok(None)
            }
            UpdateMessage::BinaryRequest { version, chunk_index } => {
                // Serve the requested chunk if we have it
                let chunk_data = self.get_chunk_for_upload(&version, chunk_index).await?;
                Ok(Some(UpdateMessage::BinaryChunk {
                    version: version.clone(),
                    chunk_index,
                    total_chunks: chunk_data.total_chunks,
                    data: chunk_data.data,
                    chunk_hash: chunk_data.chunk_hash,
                }))
            }
            UpdateMessage::BinaryChunk { version, chunk_index, total_chunks, data, chunk_hash } => {
                self.handle_binary_chunk(from_peer, version, chunk_index, total_chunks, data, chunk_hash).await?;
                Ok(None)
            }
            UpdateMessage::UpdateComplete { version, peer_id, success, error_message } => {
                self.handle_update_complete(from_peer, version, peer_id, success, error_message).await?;
                Ok(None)
            }
            UpdateMessage::SwarmMetrics {
                download_speed,
                upload_speed,
                active_transfers,
                seeding_versions,
                downloading_version,
                download_progress,
                peer_count,
                ..
            } => {
                // Update swarm tracker with peer metrics
                self.swarm_tracker.update_peer_metrics(
                    from_peer,
                    download_speed,
                    upload_speed,
                    active_transfers,
                    seeding_versions,
                    downloading_version,
                    download_progress,
                    peer_count,
                ).await?;
                Ok(None)
            }
        }
    }

    /// Handle version announcement from peer
    async fn handle_version_announce(
        &self,
        from_peer: PeerId,
        version: String,
        binary_hash: String,
        signature: Vec<u8>,
        total_chunks: u32,
    ) -> Result<()> {
        info!(
            peer_id = %from_peer,
            version = %version,
            total_chunks = total_chunks,
            binary_hash = %binary_hash,
            "📦 Version announced"
        );

        let version_info = VersionInfo {
            version: version.clone(),
            binary_hash,
            signature,
            total_chunks,
            timestamp: chrono::Utc::now().timestamp() as u64,
            chunk_count: total_chunks,
            release_timestamp: chrono::Utc::now().timestamp(),
            size_bytes: 0, // Will be updated when full binary is known
        };

        // Store peer's version
        {
            let mut peer_versions = self.peer_versions.write().await;
            peer_versions.entry(from_peer)
                .or_insert_with(Vec::new)
                .push(version_info.clone());
        }

        // Track which peers have this version
        {
            let mut version_peers = self.version_peers.write().await;
            version_peers.entry(version.clone())
                .or_insert_with(HashSet::new)
                .insert(from_peer);
        }

        // Emit event
        self.event_sender.send(UpdateProtocolEvent::VersionDiscovered(from_peer, version_info)).ok();

        Ok(())
    }

    /// Handle version response from peer
    async fn handle_version_response(
        &self,
        from_peer: PeerId,
        current_version: String,
        available_versions: Vec<VersionInfo>,
    ) -> Result<()> {
        debug!(
            peer_id = %from_peer,
            current_version = %current_version,
            available_count = available_versions.len(),
            "Version response received"
        );

        // Store peer's versions
        {
            let mut peer_versions = self.peer_versions.write().await;
            peer_versions.insert(from_peer, available_versions.clone());
        }

        // Update version-to-peer mapping
        {
            let mut version_peers = self.version_peers.write().await;
            for version_info in available_versions {
                version_peers.entry(version_info.version.clone())
                    .or_insert_with(HashSet::new)
                    .insert(from_peer);

                // Emit discovery event
                self.event_sender.send(UpdateProtocolEvent::VersionDiscovered(from_peer, version_info)).ok();
            }
        }

        Ok(())
    }

    /// Handle received binary chunk
    async fn handle_binary_chunk(
        &self,
        from_peer: PeerId,
        version: String,
        chunk_index: u32,
        total_chunks: u32,
        data: Vec<u8>,
        chunk_hash: String,
    ) -> Result<()> {
        debug!(
            peer_id = %from_peer,
            version = %version,
            chunk_index = chunk_index + 1,
            total_chunks = total_chunks,
            size_bytes = data.len(),
            "Chunk received"
        );

        // Verify chunk hash
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let computed_hash = hex::encode(hasher.finalize());

        if computed_hash != chunk_hash {
            warn!(
                version = %version,
                chunk_index = chunk_index,
                expected_hash = %chunk_hash,
                computed_hash = %computed_hash,
                "⚠️ Chunk hash mismatch"
            );
            self.event_sender.send(UpdateProtocolEvent::ChunkDownloadFailed(
                version.clone(),
                chunk_index,
                "Hash mismatch".to_string()
            )).ok();
            return Err(anyhow!("Chunk hash verification failed"));
        }

        // Store the chunk using BinaryStore
        self.binary_store.store_chunk(&version, chunk_index, data.clone()).await?;

        // Also keep in memory for backward compatibility
        {
            let mut chunks = self.downloaded_chunks.write().await;
            chunks.entry(version.clone())
                .or_insert_with(HashMap::new)
                .insert(chunk_index, data.clone());
        }

        // Store chunk hash for future verification
        {
            let mut hashes = self.chunk_hashes.write().await;
            hashes.entry(version.clone())
                .or_insert_with(HashMap::new)
                .insert(chunk_index, chunk_hash.clone());
        }

        // Update transfer tracking
        {
            let mut transfers = self.active_transfers.write().await;
            if let Some(transfer) = transfers.get_mut(&(version.clone(), chunk_index)) {
                transfer.data = Some(data.clone());
                transfer.hash = Some(chunk_hash);
                transfer.verified = true;
            }
        }

        // Emit success event
        self.event_sender.send(UpdateProtocolEvent::ChunkDownloadCompleted(
            version.clone(),
            chunk_index,
            data.len()
        )).ok();

        // Check if we have all chunks
        self.check_download_complete(&version, total_chunks).await?;

        Ok(())
    }

    /// Handle update completion notification
    async fn handle_update_complete(
        &self,
        from_peer: PeerId,
        version: String,
        _peer_id: String,
        success: bool,
        error_message: Option<String>,
    ) -> Result<()> {
        if success {
            info!(
                peer_id = %from_peer,
                version = %version,
                "✅ Peer updated successfully"
            );
        } else {
            warn!(
                peer_id = %from_peer,
                version = %version,
                error = %error_message.unwrap_or_else(|| "Unknown error".to_string()),
                "⚠️ Peer update failed"
            );
        }
        Ok(())
    }

    /// Check if all chunks have been downloaded
    async fn check_download_complete(&self, version: &str, total_chunks: u32) -> Result<()> {
        // Check if we have all chunks using BinaryStore
        if self.binary_store.has_complete_binary(version, total_chunks).await {
            // Assemble the binary from chunks
            let binary_path = self.binary_store.assemble_binary(version, total_chunks).await?;

            // Read the assembled binary
            let binary = std::fs::read(&binary_path)?;

            info!(
                version = %version,
                size_bytes = binary.len(),
                chunks_count = total_chunks,
                "🎯 Binary assembled"
            );

            // Emit completion event
            self.event_sender.send(UpdateProtocolEvent::BinaryDownloadCompleted(
                version.to_string(),
                binary
            )).ok();
        }
        Ok(())
    }

    /// Request a specific chunk from a peer with retry logic
    pub async fn request_chunk(&self, peer: PeerId, version: String, chunk_index: u32) -> Result<()> {
        // Use semaphore for rate limiting
        let _permit = self.transfer_semaphore.acquire().await?;

        // Check if we should retry an existing transfer
        let should_retry = {
            let transfers = self.active_transfers.read().await;
            if let Some(transfer) = transfers.get(&(version.clone(), chunk_index)) {
                // Check if transfer has timed out (using config timeout)
                let elapsed = transfer.started_at.elapsed();
                let timeout = self.config.chunk_timeout;

                if elapsed > timeout && transfer.attempts < self.config.max_retries {
                    true  // Should retry
                } else if elapsed > timeout {
                    // Max retries exceeded
                    return Err(anyhow::anyhow!("Chunk transfer failed after {} attempts", transfer.attempts));
                } else {
                    false  // Transfer still in progress
                }
            } else {
                true  // New transfer
            }
        };

        if !should_retry {
            return Ok(());  // Transfer already in progress
        }

        // Update or create the transfer record
        {
            let mut transfers = self.active_transfers.write().await;
            let key = (version.clone(), chunk_index);

            let attempts = if let Some(existing) = transfers.get(&key) {
                existing.attempts + 1
            } else {
                1
            };

            transfers.insert(
                key,
                ChunkTransfer {
                    peer,
                    attempts,
                    started_at: Instant::now(),
                    data: None,
                    hash: None,
                    verified: false,
                }
            );
        }

        // Emit start event
        self.event_sender.send(UpdateProtocolEvent::ChunkDownloadStarted(
            version.clone(),
            chunk_index,
            peer
        )).ok();

        Ok(())
    }

    /// Get peers that have a specific version
    pub async fn get_peers_with_version(&self, version: &str) -> Vec<PeerId> {
        let version_peers = self.version_peers.read().await;
        version_peers.get(version)
            .map(|peers| peers.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Get all known versions from peers
    pub async fn get_all_peer_versions(&self) -> HashMap<String, Vec<PeerId>> {
        let version_peers = self.version_peers.read().await;
        version_peers.iter()
            .map(|(v, peers)| (v.clone(), peers.iter().copied().collect()))
            .collect()
    }

    // Placeholder methods - would be implemented with actual storage
    async fn get_local_versions(&self) -> Vec<VersionInfo> {
        Vec::new()
    }

    async fn get_current_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    /// Get swarm speed statistics
    pub async fn get_swarm_statistics(&self) -> super::swarm_tracker::SwarmStatistics {
        self.swarm_tracker.get_statistics().await
    }

    /// Get swarm speed summary string
    pub async fn get_swarm_speed_summary(&self) -> String {
        self.swarm_tracker.get_speed_summary().await
    }

    /// Update local speed metrics
    pub async fn update_local_speed(&self, download_speed: u64, upload_speed: u64) {
        self.swarm_tracker.update_local_speed(download_speed, upload_speed).await;
    }

    /// Create SwarmMetrics message for broadcasting
    pub async fn create_swarm_metrics_message(&self) -> UpdateMessage {
        // Get active transfers count
        let active_transfers = self.active_transfers.read().await.len() as u32;

        // Get seeding versions from binary store
        let seeding_versions = self.binary_store.list_versions().await;

        // Determine if we're downloading anything
        let downloading_version = {
            let transfers = self.active_transfers.read().await;
            transfers.iter()
                .map(|((version, _), _)| version.clone())
                .next()
        };

        // Calculate download progress if downloading
        let download_progress = if let Some(ref version) = downloading_version {
            let chunks = self.downloaded_chunks.read().await;
            if let Some(version_chunks) = chunks.get(version) {
                let downloaded = version_chunks.len();
                // Try to get total chunks from version info
                let total = self.peer_versions.read().await
                    .values()
                    .flatten()
                    .find(|v| v.version == *version)
                    .map(|v| v.total_chunks)
                    .unwrap_or(100); // Default to 100 if unknown

                Some((downloaded as f32 / total as f32) * 100.0)
            } else {
                None
            }
        } else {
            None
        };

        // Get peer count
        let peer_count = self.peer_versions.read().await.len();

        // Get current speeds from swarm tracker
        let local_metrics = self.swarm_tracker.get_local_metrics(
            active_transfers,
            seeding_versions.clone(),
            downloading_version.clone(),
            download_progress,
            peer_count,
        ).await;

        UpdateMessage::SwarmMetrics {
            download_speed: local_metrics.download_speed,
            upload_speed: local_metrics.upload_speed,
            active_transfers,
            seeding_versions,
            downloading_version,
            download_progress,
            peer_count,
            timestamp: SwarmSpeedTracker::current_timestamp(),
        }
    }

    /// Process chunk response from a peer
    pub async fn handle_chunk_response(
        &self,
        peer: PeerId,
        version: String,
        chunk_index: u32,
        data: Vec<u8>,
        hash: String,
    ) -> Result<()> {
        // Update the transfer record with received data
        {
            let mut transfers = self.active_transfers.write().await;
            if let Some(transfer) = transfers.get_mut(&(version.clone(), chunk_index)) {
                transfer.data = Some(data.clone());
                transfer.hash = Some(hash.clone());
                transfer.verified = true;

                // Check if this peer matches the one we requested from
                if transfer.peer != peer {
                    tracing::warn!(
                        expected_peer = %transfer.peer,
                        actual_peer = %peer,
                        chunk_index,
                        "Received chunk from unexpected peer"
                    );
                }
            }
        }

        // Store the chunk
        self.binary_store.store_chunk(&version, chunk_index, data.clone()).await?;

        let data_len = data.len();

        // Store in downloaded chunks
        {
            let mut chunks = self.downloaded_chunks.write().await;
            chunks.entry(version.clone())
                .or_insert_with(HashMap::new)
                .insert(chunk_index, data);
        }

        // Store the hash
        {
            let mut hashes = self.chunk_hashes.write().await;
            hashes.entry(version.clone())
                .or_insert_with(HashMap::new)
                .insert(chunk_index, hash);
        }

        // Remove from active transfers
        {
            let mut transfers = self.active_transfers.write().await;
            transfers.remove(&(version.clone(), chunk_index));
        }

        // Emit completion event
        self.event_sender.send(UpdateProtocolEvent::ChunkDownloadCompleted(
            version,
            chunk_index,
            data_len
        )).ok();

        Ok(())
    }

    /// Check and retry timed-out transfers
    pub async fn check_timeouts(&self) -> Result<()> {
        let timeout = self.config.chunk_timeout;
        let mut timed_out = Vec::new();

        // Find timed-out transfers
        {
            let transfers = self.active_transfers.read().await;
            for ((version, chunk_index), transfer) in transfers.iter() {
                if transfer.started_at.elapsed() > timeout {
                    timed_out.push((
                        version.clone(),
                        *chunk_index,
                        transfer.peer,
                        transfer.attempts
                    ));
                }
            }
        }

        // Retry timed-out transfers
        for (version, chunk_index, peer, attempts) in timed_out {
            if attempts < self.config.max_retries {
                tracing::debug!(
                    version,
                    chunk_index,
                    attempts,
                    "Retrying timed-out chunk transfer"
                );

                // Find a different peer if possible
                let peers = self.get_peers_with_version(&version).await;
                let new_peer = peers.iter()
                    .find(|p| **p != peer)
                    .copied()
                    .unwrap_or(peer);

                // Retry the request
                self.request_chunk(new_peer, version, chunk_index).await?;
            } else {
                // Remove failed transfer
                let mut transfers = self.active_transfers.write().await;
                transfers.remove(&(version.clone(), chunk_index));

                self.event_sender.send(UpdateProtocolEvent::ChunkDownloadFailed(
                    version,
                    chunk_index,
                    format!("Failed after {} attempts", attempts)
                )).ok();
            }
        }

        Ok(())
    }

    /// Get transfer statistics
    pub async fn get_transfer_stats(&self) -> (usize, usize, f64) {
        let transfers = self.active_transfers.read().await;
        let active = transfers.len();
        let mut timed_out = 0;
        let mut total_attempts = 0;

        let timeout = self.config.chunk_timeout;
        for transfer in transfers.values() {
            total_attempts += transfer.attempts as usize;
            if transfer.started_at.elapsed() > timeout {
                timed_out += 1;
            }
        }

        let avg_attempts = if active > 0 {
            total_attempts as f64 / active as f64
        } else {
            0.0
        };

        (active, timed_out, avg_attempts)
    }

    /// Start background tasks for the update protocol
    pub fn start_background_tasks(self: Arc<Self>) {
        // Start metrics broadcasting
        let metrics_self = self.clone();
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(5)); // Broadcast every 5 seconds
            loop {
                interval.tick().await;

                // Create and broadcast metrics message
                let metrics_msg = metrics_self.create_swarm_metrics_message().await;

                // This would be sent to all connected peers via P2P handler
                debug!(metrics = ?metrics_msg, "Broadcasting swarm metrics");

                // Note: Actual broadcasting would be done via the P2P handler
                // which would send this message to all connected peers
            }
        });

        // Start timeout checker
        let timeout_self = self.clone();
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(10)); // Check every 10 seconds
            loop {
                interval.tick().await;

                if let Err(e) = timeout_self.check_timeouts().await {
                    tracing::warn!(error = %e, "Failed to check transfer timeouts");
                }
            }
        });
    }

    async fn get_chunk_for_upload(&self, version: &str, chunk_index: u32) -> Result<BinaryChunkData> {
        // Fetch chunk from BinaryStore
        let chunk_data = self.binary_store.get_chunk(version, chunk_index).await?;

        // Get metadata to know total chunks
        let metadata = self.binary_store.get_metadata(version).await
            .ok_or_else(|| anyhow!("Version {} not found in store", version))?;

        // Calculate chunk hash
        let mut hasher = Sha256::new();
        hasher.update(&chunk_data);
        let chunk_hash = format!("{:x}", hasher.finalize());

        Ok(BinaryChunkData {
            data: chunk_data,
            total_chunks: metadata.total_chunks,
            chunk_hash,
        })
    }

    /// Add a binary to the store for distribution
    pub async fn add_binary_for_distribution(&self, version: String, binary_path: &std::path::Path) -> Result<BinaryMetadata> {
        self.binary_store.add_binary(version, binary_path).await
    }

    /// Get list of locally available versions
    pub async fn get_available_versions(&self) -> Vec<String> {
        self.binary_store.list_versions().await
    }
}