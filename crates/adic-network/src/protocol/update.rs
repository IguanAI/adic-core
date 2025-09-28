use serde::{Deserialize, Serialize};
use std::fmt;

/// Protocol messages for P2P binary update distribution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UpdateMessage {
    /// Announce availability of a new version
    VersionAnnounce {
        version: String,
        binary_hash: String,
        signature: Vec<u8>,
        total_chunks: u32,
    },

    /// Request a specific chunk of the binary
    BinaryRequest {
        version: String,
        chunk_index: u32,
    },

    /// Transfer a chunk of binary data
    BinaryChunk {
        version: String,
        chunk_index: u32,
        total_chunks: u32,
        data: Vec<u8>,
        chunk_hash: String,
    },

    /// Confirm successful update completion
    UpdateComplete {
        version: String,
        peer_id: String,  // Use String instead of PeerId for serialization
        success: bool,
        error_message: Option<String>,
    },

    /// Query for available versions from peers
    VersionQuery,

    /// Response to version query
    VersionResponse {
        current_version: String,
        available_versions: Vec<VersionInfo>,
    },

    /// Share swarm metrics for collective speed tracking
    SwarmMetrics {
        /// Peer's current download speed (bytes/sec)
        download_speed: u64,
        /// Peer's current upload speed (bytes/sec)
        upload_speed: u64,
        /// Number of active chunk transfers
        active_transfers: u32,
        /// Versions this peer is seeding
        seeding_versions: Vec<String>,
        /// Version currently downloading (if any)
        downloading_version: Option<String>,
        /// Download progress percentage (0-100)
        download_progress: Option<f32>,
        /// Number of connected peers
        peer_count: usize,
        /// Timestamp of this metric report
        timestamp: u64,
    },
}

/// Information about an available version
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    pub version: String,
    pub binary_hash: String,
    pub signature: Vec<u8>,
    pub size_bytes: u64,
    pub chunk_count: u32,
    pub release_timestamp: i64,
    pub total_chunks: u32,
    pub timestamp: u64,
}

/// Data structure for binary chunk transfers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryChunkData {
    pub data: Vec<u8>,
    pub chunk_hash: String,
    pub total_chunks: u32,
}

impl fmt::Display for UpdateMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateMessage::VersionAnnounce { version, .. } => {
                write!(f, "VersionAnnounce(v{})", version)
            }
            UpdateMessage::BinaryRequest { version, chunk_index } => {
                write!(f, "BinaryRequest(v{}, chunk {})", version, chunk_index)
            }
            UpdateMessage::BinaryChunk { version, chunk_index, total_chunks, .. } => {
                write!(f, "BinaryChunk(v{}, {}/{})", version, chunk_index, total_chunks)
            }
            UpdateMessage::UpdateComplete { version, success, .. } => {
                write!(f, "UpdateComplete(v{}, success={})", version, success)
            }
            UpdateMessage::VersionQuery => write!(f, "VersionQuery"),
            UpdateMessage::VersionResponse { current_version, .. } => {
                write!(f, "VersionResponse(current: v{})", current_version)
            }
            UpdateMessage::SwarmMetrics { download_speed, upload_speed, peer_count, .. } => {
                write!(f, "SwarmMetrics(↓ {:.1} MB/s, ↑ {:.1} MB/s, {} peers)",
                    *download_speed as f64 / 1_048_576.0,
                    *upload_speed as f64 / 1_048_576.0,
                    peer_count)
            }
        }
    }
}

/// Constants for update protocol
pub mod constants {
    /// Maximum chunk size (1MB)
    pub const MAX_CHUNK_SIZE: usize = 1024 * 1024;

    /// Update protocol version
    pub const PROTOCOL_VERSION: &str = "1.0.0";

    /// Timeout for chunk requests
    pub const CHUNK_REQUEST_TIMEOUT_SECS: u64 = 30;

    /// Maximum concurrent chunk downloads
    pub const MAX_CONCURRENT_CHUNKS: usize = 5;

    /// Version check interval (1 hour)
    pub const VERSION_CHECK_INTERVAL_SECS: u64 = 3600;
}

/// Update state for tracking ongoing updates
#[derive(Debug, Clone)]
pub enum UpdateState {
    Idle,
    CheckingVersion,
    Downloading {
        version: String,
        progress: f32,
        chunks_received: u32,
        total_chunks: u32,
    },
    Verifying {
        version: String,
    },
    Applying {
        version: String,
    },
    Complete {
        version: String,
        success: bool,
    },
    Failed {
        version: String,
        error: String,
    },
}

impl fmt::Display for UpdateState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateState::Idle => write!(f, "Idle"),
            UpdateState::CheckingVersion => write!(f, "Checking for updates"),
            UpdateState::Downloading { version, progress, .. } => {
                write!(f, "Downloading v{} ({:.1}%)", version, progress * 100.0)
            }
            UpdateState::Verifying { version } => write!(f, "Verifying v{}", version),
            UpdateState::Applying { version } => write!(f, "Applying v{}", version),
            UpdateState::Complete { version, success } => {
                if *success {
                    write!(f, "Successfully updated to v{}", version)
                } else {
                    write!(f, "Update to v{} completed with warnings", version)
                }
            }
            UpdateState::Failed { version, error } => {
                write!(f, "Failed to update to v{}: {}", version, error)
            }
        }
    }
}