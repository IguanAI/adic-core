use anyhow::Result;
use libp2p::PeerId;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Parameters for updating peer metrics
#[derive(Debug, Clone)]
pub struct PeerMetricsUpdate {
    pub download_speed: u64,
    pub upload_speed: u64,
    pub active_transfers: u32,
    pub seeding_versions: Vec<String>,
    pub downloading_version: Option<String>,
    pub download_progress: Option<f32>,
    pub peer_count: usize,
}

/// Tracks metrics from a single peer
#[derive(Debug, Clone)]
pub struct PeerMetrics {
    pub peer_id: PeerId,
    pub download_speed: u64,
    pub upload_speed: u64,
    pub active_transfers: u32,
    pub seeding_versions: Vec<String>,
    pub downloading_version: Option<String>,
    pub download_progress: Option<f32>,
    pub peer_count: usize,
    pub last_updated: Instant,
}

/// Aggregated swarm statistics
#[derive(Debug, Clone)]
pub struct SwarmStatistics {
    /// Total download speed across all peers (bytes/sec)
    pub total_download_speed: u64,
    /// Total upload speed across all peers (bytes/sec)
    pub total_upload_speed: u64,
    /// Number of peers currently downloading
    pub downloading_peers: usize,
    /// Number of peers currently seeding
    pub seeding_peers: usize,
    /// Number of idle peers
    pub idle_peers: usize,
    /// Total number of active transfers
    pub total_active_transfers: u32,
    /// Average download progress percentage
    pub average_download_progress: f32,
    /// Version distribution (version -> peer count)
    pub version_distribution: HashMap<String, usize>,
    /// Peers by state
    pub peers_by_state: HashMap<String, Vec<PeerId>>,
    /// Last calculation time
    pub last_updated: Instant,
}

impl Default for SwarmStatistics {
    fn default() -> Self {
        Self {
            total_download_speed: 0,
            total_upload_speed: 0,
            downloading_peers: 0,
            seeding_peers: 0,
            idle_peers: 0,
            total_active_transfers: 0,
            average_download_progress: 0.0,
            version_distribution: HashMap::new(),
            peers_by_state: HashMap::new(),
            last_updated: Instant::now(),
        }
    }
}

/// Manages swarm-wide speed and metrics tracking
pub struct SwarmSpeedTracker {
    /// Metrics from each peer
    peer_metrics: Arc<RwLock<HashMap<PeerId, PeerMetrics>>>,
    /// Cached aggregated statistics
    cached_stats: Arc<RwLock<SwarmStatistics>>,
    /// Metric expiry duration (remove stale metrics)
    metric_expiry: Duration,
    /// Our own peer ID
    local_peer_id: PeerId,
    /// Local download speed (bytes/sec)
    local_download_speed: Arc<RwLock<u64>>,
    /// Local upload speed (bytes/sec)
    local_upload_speed: Arc<RwLock<u64>>,
}

impl SwarmSpeedTracker {
    /// Create a new swarm speed tracker
    pub fn new(local_peer_id: PeerId) -> Self {
        Self {
            peer_metrics: Arc::new(RwLock::new(HashMap::new())),
            cached_stats: Arc::new(RwLock::new(SwarmStatistics::default())),
            metric_expiry: Duration::from_secs(30), // Remove metrics older than 30 seconds
            local_peer_id,
            local_download_speed: Arc::new(RwLock::new(0)),
            local_upload_speed: Arc::new(RwLock::new(0)),
        }
    }

    /// Update metrics for a peer
    pub async fn update_peer_metrics(
        &self,
        peer_id: PeerId,
        update: PeerMetricsUpdate,
    ) -> Result<()> {
        let metrics = PeerMetrics {
            peer_id,
            download_speed: update.download_speed,
            upload_speed: update.upload_speed,
            active_transfers: update.active_transfers,
            seeding_versions: update.seeding_versions,
            downloading_version: update.downloading_version,
            download_progress: update.download_progress,
            peer_count: update.peer_count,
            last_updated: Instant::now(),
        };

        // Update peer metrics
        let mut peer_metrics = self.peer_metrics.write().await;
        peer_metrics.insert(peer_id, metrics.clone());

        debug!(
            peer_id = %peer_id,
            download_speed = metrics.download_speed,
            upload_speed = metrics.upload_speed,
            active_transfers = metrics.active_transfers,
            "Peer metrics updated"
        );

        // Remove stale metrics
        let now = Instant::now();
        peer_metrics.retain(|_, m| now.duration_since(m.last_updated) < self.metric_expiry);

        drop(peer_metrics);

        // Recalculate aggregated stats
        self.calculate_statistics().await?;

        Ok(())
    }

    /// Update local speed metrics
    pub async fn update_local_speed(&self, download_speed: u64, upload_speed: u64) {
        *self.local_download_speed.write().await = download_speed;
        *self.local_upload_speed.write().await = upload_speed;
    }

    /// Get local metrics for broadcasting
    pub async fn get_local_metrics(
        &self,
        active_transfers: u32,
        seeding_versions: Vec<String>,
        downloading_version: Option<String>,
        download_progress: Option<f32>,
        peer_count: usize,
    ) -> PeerMetrics {
        PeerMetrics {
            peer_id: self.local_peer_id,
            download_speed: *self.local_download_speed.read().await,
            upload_speed: *self.local_upload_speed.read().await,
            active_transfers,
            seeding_versions,
            downloading_version,
            download_progress,
            peer_count,
            last_updated: Instant::now(),
        }
    }

    /// Calculate aggregated swarm statistics
    async fn calculate_statistics(&self) -> Result<()> {
        let peer_metrics = self.peer_metrics.read().await;

        let mut stats = SwarmStatistics {
            total_download_speed: 0,
            total_upload_speed: 0,
            downloading_peers: 0,
            seeding_peers: 0,
            idle_peers: 0,
            total_active_transfers: 0,
            average_download_progress: 0.0,
            version_distribution: HashMap::new(),
            peers_by_state: HashMap::new(),
            last_updated: Instant::now(),
        };

        let mut download_progress_sum = 0.0;
        let mut download_progress_count = 0;

        // Aggregate metrics from all peers
        for (peer_id, metrics) in peer_metrics.iter() {
            stats.total_download_speed += metrics.download_speed;
            stats.total_upload_speed += metrics.upload_speed;
            stats.total_active_transfers += metrics.active_transfers;

            // Categorize peer state
            if let Some(ref version) = metrics.downloading_version {
                stats.downloading_peers += 1;
                stats
                    .peers_by_state
                    .entry("downloading".to_string())
                    .or_default()
                    .push(*peer_id);

                // Track version being downloaded
                *stats
                    .version_distribution
                    .entry(version.clone())
                    .or_insert(0) += 1;

                // Aggregate download progress
                if let Some(progress) = metrics.download_progress {
                    download_progress_sum += progress;
                    download_progress_count += 1;
                }
            } else if !metrics.seeding_versions.is_empty() {
                stats.seeding_peers += 1;
                stats
                    .peers_by_state
                    .entry("seeding".to_string())
                    .or_default()
                    .push(*peer_id);

                // Track versions being seeded
                for version in &metrics.seeding_versions {
                    *stats
                        .version_distribution
                        .entry(version.clone())
                        .or_insert(0) += 1;
                }
            } else {
                stats.idle_peers += 1;
                stats
                    .peers_by_state
                    .entry("idle".to_string())
                    .or_default()
                    .push(*peer_id);
            }
        }

        // Calculate average download progress
        if download_progress_count > 0 {
            stats.average_download_progress =
                download_progress_sum / download_progress_count as f32;
        }

        // Include local metrics
        let local_download = *self.local_download_speed.read().await;
        let local_upload = *self.local_upload_speed.read().await;
        stats.total_download_speed += local_download;
        stats.total_upload_speed += local_upload;

        // Update cached statistics
        *self.cached_stats.write().await = stats.clone();

        info!(
            total_peers = peer_metrics.len() + 1, // +1 for local peer
            download_speed_mbps = stats.total_download_speed as f64 / 1_048_576.0,
            upload_speed_mbps = stats.total_upload_speed as f64 / 1_048_576.0,
            downloading_peers = stats.downloading_peers,
            seeding_peers = stats.seeding_peers,
            idle_peers = stats.idle_peers,
            "🌊 Swarm statistics updated"
        );

        Ok(())
    }

    /// Get current swarm statistics
    pub async fn get_statistics(&self) -> SwarmStatistics {
        self.cached_stats.read().await.clone()
    }

    /// Get swarm speed summary string
    pub async fn get_speed_summary(&self) -> String {
        let stats = self.cached_stats.read().await;

        format!(
            "Swarm: ↓ {:.1} MB/s / ↑ {:.1} MB/s ({} peers, {} downloading, {} seeding)",
            stats.total_download_speed as f64 / 1_048_576.0,
            stats.total_upload_speed as f64 / 1_048_576.0,
            stats.downloading_peers + stats.seeding_peers + stats.idle_peers,
            stats.downloading_peers,
            stats.seeding_peers
        )
    }

    /// Get current timestamp for metrics
    pub fn current_timestamp() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Clear metrics for a specific peer
    pub async fn clear_peer_metrics(&self, peer_id: &PeerId) {
        let mut peer_metrics = self.peer_metrics.write().await;
        if peer_metrics.remove(peer_id).is_some() {
            debug!(peer_id = %peer_id, "Peer metrics cleared");
        }
        drop(peer_metrics);

        // Recalculate statistics
        let _ = self.calculate_statistics().await;
    }

    /// Get detailed peer breakdown
    pub async fn get_peer_breakdown(&self) -> HashMap<String, Vec<(PeerId, f64, f64)>> {
        let peer_metrics = self.peer_metrics.read().await;
        let mut breakdown = HashMap::new();

        for (peer_id, metrics) in peer_metrics.iter() {
            let state = if metrics.downloading_version.is_some() {
                "downloading"
            } else if !metrics.seeding_versions.is_empty() {
                "seeding"
            } else {
                "idle"
            };

            breakdown
                .entry(state.to_string())
                .or_insert_with(Vec::new)
                .push((
                    *peer_id,
                    metrics.download_speed as f64 / 1_048_576.0,
                    metrics.upload_speed as f64 / 1_048_576.0,
                ));
        }

        breakdown
    }
}
