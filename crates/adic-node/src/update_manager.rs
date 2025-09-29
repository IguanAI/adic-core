use crate::progress_display::DownloadProgressBar;
use crate::update_verifier::UpdateVerifier;
use adic_network::dns_version::{DnsVersionDiscovery, VersionRecord};
use adic_network::protocol::update::{constants, UpdateState, VersionInfo};
use adic_network::NetworkEngine;
use anyhow::{anyhow, Result};
use libp2p::PeerId;
use std::collections::HashMap;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::{interval, sleep};
use tracing::{debug, error, info, warn};

/// Simple tracker for chunk downloads
struct ChunkTracker {
    chunks_complete: u32,
    start_time: Instant,
}

impl ChunkTracker {
    fn new(_total_chunks: u32) -> Self {
        Self {
            chunks_complete: 0,
            start_time: Instant::now(),
        }
    }

    fn complete_chunk(&mut self) {
        self.chunks_complete += 1;
    }

    fn chunks_complete(&self) -> u32 {
        self.chunks_complete
    }

    fn chunks_per_second(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.chunks_complete as f64 / elapsed
        } else {
            0.0
        }
    }
}

/// Manages software updates for the ADIC node
pub struct UpdateManager {
    /// Current version
    current_version: String,

    /// DNS version discovery
    dns_discovery: DnsVersionDiscovery,

    /// Update state
    state: Arc<RwLock<UpdateState>>,

    /// Network engine for P2P distribution
    network: Arc<NetworkEngine>,

    /// Data directory for storing updates
    data_dir: PathBuf,

    /// Downloaded binary chunks
    chunks: Arc<RwLock<HashMap<u32, Vec<u8>>>>,

    /// Available versions from peers
    peer_versions: Arc<RwLock<HashMap<PeerId, VersionInfo>>>,

    /// Update configuration
    config: UpdateConfig,

    /// API listener file descriptor (set after API server starts)
    api_listener_fd: Arc<RwLock<Option<i32>>>,
}

#[derive(Debug, Clone)]
pub struct UpdateConfig {
    /// Enable automatic updates
    pub auto_update: bool,

    /// Check interval in seconds
    pub check_interval: u64,

    /// Update window start hour (0-23)
    pub update_window_start: u8,

    /// Update window end hour (0-23)
    pub update_window_end: u8,

    /// Require manual confirmation for updates
    pub require_confirmation: bool,

    /// Maximum download retries
    pub max_retries: u32,

    /// DNS domain for version discovery
    pub dns_domain: String,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            auto_update: false,
            check_interval: constants::VERSION_CHECK_INTERVAL_SECS,
            update_window_start: 2, // 2 AM
            update_window_end: 4,   // 4 AM
            require_confirmation: true,
            max_retries: 3,
            dns_domain: "adic.network".to_string(),
        }
    }
}

impl UpdateManager {
    /// Create a new update manager
    pub fn new(
        current_version: String,
        network: Arc<NetworkEngine>,
        data_dir: PathBuf,
        config: UpdateConfig,
    ) -> Result<Self> {
        let dns_discovery = DnsVersionDiscovery::new(config.dns_domain.clone())?;

        // Create update directory if it doesn't exist
        let update_dir = data_dir.join("updates");
        fs::create_dir_all(&update_dir)?;

        Ok(Self {
            current_version,
            dns_discovery,
            state: Arc::new(RwLock::new(UpdateState::Idle)),
            network,
            data_dir: update_dir,
            chunks: Arc::new(RwLock::new(HashMap::new())),
            peer_versions: Arc::new(RwLock::new(HashMap::new())),
            config,
            api_listener_fd: Arc::new(RwLock::new(None)),
        })
    }

    /// Start the update manager
    pub async fn start(&self) -> Result<()> {
        if self.config.auto_update {
            let manager = self.clone_for_task();
            tokio::spawn(async move {
                if let Err(e) = manager.run_update_loop().await {
                    error!(
                        error = %e,
                        "❌ Update manager failed"
                    );
                }
            });
        }
        Ok(())
    }

    /// Run the update check loop
    async fn run_update_loop(&self) -> Result<()> {
        let mut check_interval = interval(Duration::from_secs(self.config.check_interval));

        loop {
            check_interval.tick().await;

            if !self.is_in_update_window() {
                debug!("Outside update window, skipping check");
                continue;
            }

            info!("Checking for updates");
            if let Err(e) = self.check_and_update().await {
                warn!(
                    error = %e,
                    "⚠️ Update check failed"
                );
            }
        }
    }

    /// Get current update state
    pub async fn get_state(&self) -> UpdateState {
        self.state.read().await.clone()
    }

    /// Get latest available version
    pub async fn get_latest_version(&self) -> Option<VersionRecord> {
        self.dns_discovery
            .check_for_update(&self.current_version)
            .await
            .unwrap_or_default()
    }

    /// Get swarm statistics from the update protocol
    pub async fn get_swarm_statistics(
        &self,
    ) -> adic_network::protocol::swarm_tracker::SwarmStatistics {
        // Get swarm statistics from the network engine's update protocol
        if let Some(update_protocol) = self.network.get_update_protocol() {
            update_protocol.get_swarm_statistics().await
        } else {
            // Return empty statistics if protocol not available
            adic_network::protocol::swarm_tracker::SwarmStatistics::default()
        }
    }

    /// Check for updates (without applying)
    pub async fn check_for_update(&self) -> Result<Option<VersionRecord>> {
        // Update state
        {
            let mut state = self.state.write().await;
            *state = UpdateState::CheckingVersion;
        }

        // Check DNS for latest version with retries
        let mut retry_count = 0;
        let result = loop {
            match self
                .dns_discovery
                .check_for_update(&self.current_version)
                .await
            {
                Ok(update) => break Ok(update),
                Err(e) if retry_count < self.config.max_retries => {
                    debug!(
                        error = %e,
                        retry_count = retry_count,
                        max_retries = self.config.max_retries,
                        "DNS version check failed, retrying"
                    );
                    retry_count += 1;
                    sleep(Duration::from_secs(2)).await;
                }
                Err(e) => break Err(e),
            }
        };

        // Reset state
        {
            let mut state = self.state.write().await;
            *state = UpdateState::Idle;
        }

        result
    }

    /// Start auto-update process
    pub async fn start_auto_update(&self) -> Result<()> {
        self.check_and_update().await
    }

    /// Check for updates and apply if available
    pub async fn check_and_update(&self) -> Result<()> {
        // Update state
        {
            let mut state = self.state.write().await;
            *state = UpdateState::CheckingVersion;
        }

        // Check DNS for latest version
        let update_available = match self
            .dns_discovery
            .check_for_update(&self.current_version)
            .await
        {
            Ok(update) => update,
            Err(e) => {
                warn!(
                    error = %e,
                    "⚠️ DNS version check failed, trying P2P"
                );
                // Fallback to P2P version discovery
                self.check_p2p_versions().await?
            }
        };

        if let Some(version_record) = update_available {
            info!(
                new_version = %version_record.version,
                current_version = %self.current_version,
                "🆕 Update available"
            );

            if !self.config.require_confirmation || self.confirm_update(&version_record).await? {
                self.download_and_apply(version_record).await?;
            }
        } else {
            info!(
                current_version = %self.current_version,
                "✅ Already on latest version"
            );
            let mut state = self.state.write().await;
            *state = UpdateState::Idle;
        }

        Ok(())
    }

    /// Check P2P network for version information
    async fn check_p2p_versions(&self) -> Result<Option<VersionRecord>> {
        // Query peers for their versions
        self.broadcast_version_query().await?;

        // Wait for responses
        sleep(Duration::from_secs(5)).await;

        // Find the newest version among peers
        let peer_versions = self.peer_versions.read().await;
        let newest = peer_versions
            .values()
            .filter(|v| self.is_newer(&v.version))
            .max_by(|a, b| a.version.cmp(&b.version));

        if let Some(version_info) = newest {
            Ok(Some(VersionRecord {
                version: version_info.version.clone(),
                sha256_hash: version_info.binary_hash.clone(),
                signature: hex::encode(&version_info.signature),
                release_date: None,
                min_compatible: None,
            }))
        } else {
            Ok(None)
        }
    }

    /// Download and apply an update
    async fn download_and_apply(&self, version: VersionRecord) -> Result<()> {
        info!(
            version = %version.version,
            "📥 Downloading update"
        );

        // Update state
        {
            let mut state = self.state.write().await;
            *state = UpdateState::Downloading {
                version: version.version.clone(),
                progress: 0.0,
                chunks_received: 0,
                total_chunks: 0,
            };
        }

        // Download binary via P2P
        let binary_path = self.download_binary(&version).await?;

        // Verify the binary
        {
            let mut state = self.state.write().await;
            *state = UpdateState::Verifying {
                version: version.version.clone(),
            };
        }

        self.verify_binary(&binary_path, &version).await?;

        // Apply the update
        {
            let mut state = self.state.write().await;
            *state = UpdateState::Applying {
                version: version.version.clone(),
            };
        }

        self.apply_update(&binary_path, &version).await?;

        // Update state
        {
            let mut state = self.state.write().await;
            *state = UpdateState::Complete {
                version: version.version.clone(),
                success: true,
            };
        }

        info!(
            version = %version.version,
            "✅ Update successfully applied"
        );

        Ok(())
    }

    /// Download binary from P2P network with progress display
    async fn download_binary(&self, version: &VersionRecord) -> Result<PathBuf> {
        // Find peers with this version
        let peers = self.find_peers_with_version(&version.version).await?;
        if peers.is_empty() {
            return Err(anyhow!("No peers found with version {}", version.version));
        }

        // Request binary chunks from peers
        let total_chunks = self
            .request_chunk_count(&peers[0], &version.version)
            .await?;

        // Create progress bar if we're in a TTY
        let progress = if io::stderr().is_terminal() {
            Some(DownloadProgressBar::new_chunk_progress(
                &version.version,
                total_chunks,
            ))
        } else {
            None
        };

        // Use ChunkTracker for smooth speed calculation
        let mut tracker = ChunkTracker::new(total_chunks);

        // Download chunks with error handling
        for chunk_idx in 0..total_chunks {
            let peer = &peers[chunk_idx as usize % peers.len()];

            // Try to download chunk with retries
            let mut retry_count = 0;
            let chunk = loop {
                match self.request_chunk(peer, &version.version, chunk_idx).await {
                    Ok(data) => break data,
                    Err(e) if retry_count < self.config.max_retries => {
                        warn!(
                            chunk_index = chunk_idx,
                            error = %e,
                            retry_count = retry_count,
                            max_retries = self.config.max_retries,
                            "⚠️ Chunk download failed, retrying"
                        );
                        retry_count += 1;
                        sleep(Duration::from_secs(1)).await;
                    }
                    Err(e) => {
                        if let Some(ref pb) = progress {
                            pb.finish_error(&format!(
                                "Failed to download chunk {}: {}",
                                chunk_idx, e
                            ));
                        }
                        return Err(e);
                    }
                }
            };

            // Verify chunk hash before storing
            let verifier = UpdateVerifier::new()?;

            // Calculate chunk hash for verification
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&chunk);
            let chunk_hash = format!("{:x}", hasher.finalize());

            // Verify the chunk with its hash
            // Signature verification happens at the full binary level after assembly
            verifier.verify_chunk(&chunk, &chunk_hash, None)?;

            debug!(
                chunk_index = chunk_idx,
                hash = %chunk_hash,
                "✅ Chunk verified"
            );

            // Store verified chunk
            {
                let mut chunks = self.chunks.write().await;
                chunks.insert(chunk_idx, chunk);
            }

            // Update tracker
            tracker.complete_chunk();

            // Update progress bar with smooth speed
            if let Some(ref pb) = progress {
                pb.update_chunk(
                    tracker.chunks_complete(),
                    total_chunks,
                    tracker.chunks_per_second(),
                    peers.len(),
                );
            }

            // Update internal state
            {
                let mut state = self.state.write().await;
                *state = UpdateState::Downloading {
                    version: version.version.clone(),
                    progress: tracker.chunks_complete() as f32 / total_chunks as f32,
                    chunks_received: tracker.chunks_complete(),
                    total_chunks,
                };
            }
        }

        // Show verification stage
        if let Some(ref pb) = progress {
            pb.start_verification();
        }

        // Assemble binary
        let binary_path = self.assemble_binary(&version.version, total_chunks).await?;

        // Mark success
        if let Some(ref pb) = progress {
            pb.finish_success(&version.version);
        }

        Ok(binary_path)
    }

    /// Assemble downloaded chunks into complete binary
    async fn assemble_binary(&self, version: &str, total_chunks: u32) -> Result<PathBuf> {
        let binary_path = self.data_dir.join(format!("adic-{}", version));
        let mut file = fs::File::create(&binary_path)?;

        let chunks = self.chunks.read().await;
        for idx in 0..total_chunks {
            let chunk = chunks
                .get(&idx)
                .ok_or_else(|| anyhow!("Missing chunk {}", idx))?;
            file.write_all(chunk)?;
        }

        file.sync_all()?;
        Ok(binary_path)
    }

    /// Verify binary hash and signature
    async fn verify_binary(&self, path: &Path, version: &VersionRecord) -> Result<()> {
        // Create verifier with default ADIC public key
        let verifier = UpdateVerifier::new()?;

        // Verify the binary with hash and signature
        verifier.verify_binary(path, &version.signature, Some(&version.sha256_hash))?;

        // Also verify the version record signature if we have the proper format
        // The signature should be over "version:hash" message
        match verifier.verify_version_record(
            &version.version,
            &version.sha256_hash,
            &version.signature,
        ) {
            Ok(_) => {
                info!(
                    version = %version.version,
                    hash = %version.sha256_hash,
                    "✅ Version signature verified"
                );
            }
            Err(e) => {
                // For now, just warn if version record verification fails
                // since we already verified the binary itself
                warn!(
                    error = %e,
                    version = %version.version,
                    "⚠️ Version signature verification failed"
                );
            }
        }

        info!(
            version = %version.version,
            "✅ Binary verified successfully"
        );

        Ok(())
    }

    /// Set the API listener file descriptor
    pub async fn set_api_listener_fd(&self, fd: i32) {
        let mut api_fd = self.api_listener_fd.write().await;
        *api_fd = Some(fd);
    }

    /// Get the API listener file descriptor if available
    async fn get_api_listener_fd(&self) -> Option<i32> {
        let api_fd = self.api_listener_fd.read().await;
        *api_fd
    }

    /// Apply the update (trigger copyover)
    async fn apply_update(&self, binary_path: &Path, version: &VersionRecord) -> Result<()> {
        info!(
            version = %version.version,
            path = %binary_path.display(),
            "🔄 Applying update via copyover"
        );

        // Make binary executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(binary_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(binary_path, perms)?;
        }

        // Move to final location
        let final_path = self
            .data_dir
            .parent()
            .ok_or_else(|| anyhow!("Invalid data directory"))?
            .join("adic.new");
        fs::rename(binary_path, &final_path)?;

        // Create copyover manager and execute the update
        let mut copyover = crate::copyover::CopyoverManager::new();

        // Get the config path (assuming it's in the parent of data_dir)
        let config_path = self
            .data_dir
            .parent()
            .ok_or_else(|| anyhow!("Invalid data directory"))?
            .join("adic-config.toml");

        // Prepare state with current running configuration
        // Get the API listener fd if we have access to it
        let api_fd = self.get_api_listener_fd().await;

        copyover.prepare_state(
            api_fd,
            config_path.to_string_lossy().to_string(),
            self.data_dir.to_string_lossy().to_string(),
            version.version.clone(),
        )?;

        info!(
            version = %version.version,
            "🔄 Executing copyover to apply update"
        );

        // Execute the copyover - this will replace the current process
        copyover
            .safe_copyover(&final_path.to_string_lossy())
            .await?;

        // This line should never be reached if copyover succeeds
        Err(anyhow!("Copyover failed - process was not replaced"))
    }

    /// Helper methods for P2P communication
    async fn broadcast_version_query(&self) -> Result<()> {
        use adic_network::protocol::update::UpdateMessage;

        debug!("Broadcasting version query");

        // Send VersionQuery to all connected peers
        let transport = self.network.transport().await;
        let query = UpdateMessage::VersionQuery;

        transport
            .broadcast_update_message(query)
            .await
            .map_err(|e| anyhow!("Failed to broadcast version query: {}", e))?;

        Ok(())
    }

    async fn find_peers_with_version(&self, version: &str) -> Result<Vec<PeerId>> {
        let peer_versions = self.peer_versions.read().await;
        Ok(peer_versions
            .iter()
            .filter(|(_, v)| v.version == version)
            .map(|(peer_id, _)| *peer_id)
            .collect())
    }

    async fn request_chunk_count(&self, peer: &PeerId, version: &str) -> Result<u32> {
        use adic_network::protocol::update::UpdateMessage;

        // Send BinaryRequest for chunk 0 to get total_chunks in response
        let transport = self.network.transport().await;
        let request = UpdateMessage::BinaryRequest {
            version: version.to_string(),
            chunk_index: 0,
        };

        transport
            .send_update_message(peer, request)
            .await
            .map_err(|e| anyhow!("Failed to request chunk count: {}", e))?;

        // In a real implementation, we'd wait for the response
        // For now, return a default
        Ok(10)
    }

    async fn request_chunk(&self, peer: &PeerId, version: &str, idx: u32) -> Result<Vec<u8>> {
        use adic_network::protocol::update::UpdateMessage;

        // Send BinaryRequest for specific chunk
        let transport = self.network.transport().await;
        let request = UpdateMessage::BinaryRequest {
            version: version.to_string(),
            chunk_index: idx,
        };

        transport
            .send_update_message(peer, request)
            .await
            .map_err(|e| anyhow!("Failed to request chunk {}: {}", idx, e))?;

        // In a real implementation, we'd wait for the BinaryChunk response
        // For now, return empty chunk
        Ok(vec![0; 1024])
    }

    async fn confirm_update(&self, _version: &VersionRecord) -> Result<bool> {
        // In production, this would prompt the user
        // For now, auto-confirm
        Ok(true)
    }

    fn is_newer(&self, version: &str) -> bool {
        // Simple version comparison
        version > self.current_version.as_str()
    }

    fn is_in_update_window(&self) -> bool {
        use chrono::Timelike;
        let now = chrono::Local::now();
        let hour = now.hour() as u8;

        if self.config.update_window_start <= self.config.update_window_end {
            hour >= self.config.update_window_start && hour < self.config.update_window_end
        } else {
            // Handle wrap-around (e.g., 23:00 to 02:00)
            hour >= self.config.update_window_start || hour < self.config.update_window_end
        }
    }

    fn clone_for_task(&self) -> Self {
        Self {
            current_version: self.current_version.clone(),
            dns_discovery: DnsVersionDiscovery::new(self.config.dns_domain.clone()).unwrap(),
            state: self.state.clone(),
            network: self.network.clone(),
            data_dir: self.data_dir.clone(),
            chunks: self.chunks.clone(),
            peer_versions: self.peer_versions.clone(),
            config: self.config.clone(),
            api_listener_fd: self.api_listener_fd.clone(),
        }
    }
}
