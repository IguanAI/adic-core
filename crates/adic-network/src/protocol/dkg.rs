//! DKG Protocol for Distributed Key Generation
//!
//! Enables committee members to exchange DKG messages (commitments and shares)
//! over the network to collectively generate threshold BLS keys.
//!
//! # Protocol Flow
//! 1. Committee is formed for a new epoch
//! 2. Each member generates commitment (Feldman VSS)
//! 3. Commitments are broadcast via DKGCommitment messages
//! 4. Once all commitments received, members generate shares
//! 5. Shares are sent to specific participants via DKGShare messages
//! 6. After verification, DKG completes and PublicKeySet is available
//!
//! # Message Types
//! - `DKGCommitment`: Public key commitments for Feldman VSS
//! - `DKGShare`: Secret shares sent to specific participants
//! - `DKGComplete`: Notification that DKG ceremony completed

use adic_crypto::{DKGCommitment, DKGShare};
use adic_types::PublicKey;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// DKG protocol configuration
#[derive(Debug, Clone)]
pub struct DKGConfig {
    /// Maximum number of concurrent DKG ceremonies
    pub max_concurrent_ceremonies: usize,

    /// Timeout for DKG ceremony completion
    pub ceremony_timeout_secs: u64,
}

impl Default for DKGConfig {
    fn default() -> Self {
        Self {
            max_concurrent_ceremonies: 10,
            ceremony_timeout_secs: 300, // 5 minutes
        }
    }
}

/// DKG network messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DKGMessage {
    /// Broadcast commitment to all committee members
    Commitment {
        epoch_id: u64,
        commitment: DKGCommitment,
    },

    /// Send share to specific participant
    Share {
        epoch_id: u64,
        share: DKGShare,
    },

    /// Notify that DKG ceremony completed successfully
    Complete {
        epoch_id: u64,
        participant_id: usize,
    },
}

/// Tracks state of DKG ceremony over the network
#[derive(Debug)]
struct CeremonyState {
    epoch_id: u64,
    committee_size: usize,
    received_commitments: HashSet<usize>,
    received_shares: HashSet<usize>,
    completed_participants: HashSet<usize>,
    started_at: std::time::Instant,
}

impl CeremonyState {
    fn new(epoch_id: u64, committee_size: usize) -> Self {
        Self {
            epoch_id,
            committee_size,
            received_commitments: HashSet::new(),
            received_shares: HashSet::new(),
            completed_participants: HashSet::new(),
            started_at: std::time::Instant::now(),
        }
    }

    fn is_commitments_complete(&self) -> bool {
        self.received_commitments.len() >= self.committee_size
    }

    fn is_shares_complete(&self) -> bool {
        self.received_shares.len() >= self.committee_size
    }

    fn is_ceremony_complete(&self) -> bool {
        self.completed_participants.len() >= self.committee_size
    }
}

/// DKG Protocol handler
pub struct DKGProtocol {
    config: DKGConfig,

    /// Active ceremony states (epoch_id -> state)
    ceremonies: Arc<RwLock<HashMap<u64, CeremonyState>>>,

    /// Peer mappings (peer_id -> adic public key)
    peer_keys: Arc<RwLock<HashMap<PeerId, PublicKey>>>,
}

impl DKGProtocol {
    /// Create new DKG protocol handler
    pub fn new(config: DKGConfig) -> Self {
        Self {
            config,
            ceremonies: Arc::new(RwLock::new(HashMap::new())),
            peer_keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new DKG ceremony
    pub async fn register_ceremony(&self, epoch_id: u64, committee_size: usize) {
        let mut ceremonies = self.ceremonies.write().await;

        if ceremonies.len() >= self.config.max_concurrent_ceremonies {
            warn!("Maximum concurrent DKG ceremonies reached, cleaning up old ones");
            self.cleanup_old_ceremonies(&mut ceremonies);
        }

        ceremonies.insert(epoch_id, CeremonyState::new(epoch_id, committee_size));

        info!(
            epoch_id,
            committee_size,
            "📝 Registered new DKG ceremony"
        );
    }

    /// Handle incoming DKG message
    pub async fn handle_message(
        &self,
        message: DKGMessage,
        from_peer: PeerId,
    ) -> Option<DKGMessage> {
        match message {
            DKGMessage::Commitment { epoch_id, commitment } => {
                self.handle_commitment(epoch_id, commitment, from_peer).await;
                None
            }
            DKGMessage::Share { epoch_id, share } => {
                self.handle_share(epoch_id, share, from_peer).await;
                None
            }
            DKGMessage::Complete { epoch_id, participant_id } => {
                self.handle_complete(epoch_id, participant_id, from_peer).await;
                None
            }
        }
    }

    /// Handle commitment message
    async fn handle_commitment(
        &self,
        epoch_id: u64,
        commitment: DKGCommitment,
        from_peer: PeerId,
    ) {
        let mut ceremonies = self.ceremonies.write().await;

        if let Some(ceremony) = ceremonies.get_mut(&epoch_id) {
            ceremony.received_commitments.insert(commitment.participant_id);

            debug!(
                epoch_id,
                participant_id = commitment.participant_id,
                from_peer = %from_peer,
                collected = ceremony.received_commitments.len(),
                required = ceremony.committee_size,
                "✅ Received DKG commitment"
            );

            if ceremony.is_commitments_complete() {
                info!(
                    epoch_id,
                    "🔐 All DKG commitments received, ready for share exchange"
                );
            }
        } else {
            warn!(
                epoch_id,
                from_peer = %from_peer,
                "⚠️ Received commitment for unknown ceremony"
            );
        }
    }

    /// Handle share message
    async fn handle_share(
        &self,
        epoch_id: u64,
        share: DKGShare,
        from_peer: PeerId,
    ) {
        let mut ceremonies = self.ceremonies.write().await;

        if let Some(ceremony) = ceremonies.get_mut(&epoch_id) {
            ceremony.received_shares.insert(share.from);

            debug!(
                epoch_id,
                from = share.from,
                to = share.to,
                from_peer = %from_peer,
                collected = ceremony.received_shares.len(),
                required = ceremony.committee_size,
                "✅ Received DKG share"
            );

            if ceremony.is_shares_complete() {
                info!(
                    epoch_id,
                    "🔐 All DKG shares received, ready for finalization"
                );
            }
        } else {
            warn!(
                epoch_id,
                from_peer = %from_peer,
                "⚠️ Received share for unknown ceremony"
            );
        }
    }

    /// Handle completion message
    async fn handle_complete(
        &self,
        epoch_id: u64,
        participant_id: usize,
        from_peer: PeerId,
    ) {
        let mut ceremonies = self.ceremonies.write().await;

        if let Some(ceremony) = ceremonies.get_mut(&epoch_id) {
            ceremony.completed_participants.insert(participant_id);

            info!(
                epoch_id,
                participant_id,
                from_peer = %from_peer,
                completed = ceremony.completed_participants.len(),
                required = ceremony.committee_size,
                "✨ Participant completed DKG"
            );

            if ceremony.is_ceremony_complete() {
                let duration = ceremony.started_at.elapsed();
                info!(
                    epoch_id,
                    duration_secs = duration.as_secs(),
                    "🎉 DKG ceremony completed successfully"
                );

                // Ceremony complete, can be cleaned up
            }
        } else {
            warn!(
                epoch_id,
                from_peer = %from_peer,
                "⚠️ Received completion for unknown ceremony"
            );
        }
    }

    /// Get ceremony status
    pub async fn get_ceremony_status(&self, epoch_id: u64) -> Option<CeremonyStatus> {
        let ceremonies = self.ceremonies.read().await;

        ceremonies.get(&epoch_id).map(|ceremony| CeremonyStatus {
            epoch_id: ceremony.epoch_id,
            committee_size: ceremony.committee_size,
            commitments_received: ceremony.received_commitments.len(),
            shares_received: ceremony.received_shares.len(),
            completed_participants: ceremony.completed_participants.len(),
            is_complete: ceremony.is_ceremony_complete(),
            duration_secs: ceremony.started_at.elapsed().as_secs(),
        })
    }

    /// Cleanup expired ceremonies
    pub async fn cleanup_expired(&self) {
        let mut ceremonies = self.ceremonies.write().await;
        self.cleanup_old_ceremonies(&mut ceremonies);
    }

    fn cleanup_old_ceremonies(&self, ceremonies: &mut HashMap<u64, CeremonyState>) {
        let timeout = std::time::Duration::from_secs(self.config.ceremony_timeout_secs);
        let now = std::time::Instant::now();

        let before_count = ceremonies.len();
        ceremonies.retain(|epoch_id, ceremony| {
            let keep = now.duration_since(ceremony.started_at) < timeout || !ceremony.is_ceremony_complete();
            if !keep {
                debug!(
                    epoch_id = epoch_id,
                    "🗑️ Cleaned up DKG ceremony"
                );
            }
            keep
        });

        let removed = before_count - ceremonies.len();
        if removed > 0 {
            info!(removed, "🧹 Cleaned up expired DKG ceremonies");
        }
    }

    /// Register peer's ADIC public key
    pub async fn register_peer(&self, peer_id: PeerId, public_key: PublicKey) {
        let mut peer_keys = self.peer_keys.write().await;
        peer_keys.insert(peer_id, public_key);
    }

    /// Get peer's ADIC public key
    pub async fn get_peer_key(&self, peer_id: &PeerId) -> Option<PublicKey> {
        let peer_keys = self.peer_keys.read().await;
        peer_keys.get(peer_id).copied()
    }
}

/// Status information for a DKG ceremony
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CeremonyStatus {
    pub epoch_id: u64,
    pub committee_size: usize,
    pub commitments_received: usize,
    pub shares_received: usize,
    pub completed_participants: usize,
    pub is_complete: bool,
    pub duration_secs: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dkg_protocol_flow() {
        let protocol = DKGProtocol::new(DKGConfig::default());

        // Register ceremony
        protocol.register_ceremony(1, 3).await;

        // Check status
        let status = protocol.get_ceremony_status(1).await.unwrap();
        assert_eq!(status.epoch_id, 1);
        assert_eq!(status.committee_size, 3);
        assert_eq!(status.commitments_received, 0);
        assert!(!status.is_complete);
    }
}
