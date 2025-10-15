//! DKG Orchestrator - Network-aware DKG ceremony management
//!
//! Coordinates DKG ceremonies between DKGManager (local state) and NetworkEngine (message passing).
//! This bridges the business logic in DKGManager with network protocol in adic-network.
//!
//! # Ceremony Flow
//! 1. Committee is formed for epoch
//! 2. Orchestrator starts DKG ceremony if we're a committee member
//! 3. Generates our commitment and broadcasts to committee
//! 4. Receives commitments from other members
//! 5. Generates shares and sends to specific recipients
//! 6. Receives shares from other members
//! 7. Finalizes ceremony and provides PublicKeySet to BLSCoordinator

use crate::{BLSCoordinator, DKGManager, PoUWError, Result};
use adic_crypto::{DKGCommitment, DKGShare, DKGState, ThresholdConfig};
use adic_network::protocol::dkg::DKGMessage;
use adic_types::PublicKey;
use libp2p::PeerId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Maps PublicKey to PeerId for committee member communication
type PeerMapping = Arc<RwLock<HashMap<PublicKey, PeerId>>>;

/// DKG Orchestrator configuration
#[derive(Debug, Clone)]
pub struct DKGOrchestratorConfig {
    /// Our local node's public key
    pub our_public_key: PublicKey,
}

/// Tracks active DKG ceremonies
#[derive(Debug)]
struct ActiveCeremony {
    our_participant_id: usize,
    committee_members: Vec<PublicKey>,
    commitments_received: usize,
    shares_sent: bool,
}

/// DKG Orchestrator
///
/// Manages the full DKG ceremony lifecycle with network communication.
pub struct DKGOrchestrator {
    config: DKGOrchestratorConfig,
    dkg_manager: Arc<DKGManager>,
    bls_coordinator: Arc<BLSCoordinator>,

    /// Maps PublicKey -> PeerId for sending messages to committee members
    peer_mapping: PeerMapping,

    /// Active ceremonies we're participating in
    active_ceremonies: Arc<RwLock<HashMap<u64, ActiveCeremony>>>,
}

impl DKGOrchestrator {
    /// Create new DKG orchestrator
    pub fn new(
        config: DKGOrchestratorConfig,
        dkg_manager: Arc<DKGManager>,
        bls_coordinator: Arc<BLSCoordinator>,
    ) -> Self {
        Self {
            config,
            dkg_manager,
            bls_coordinator,
            peer_mapping: Arc::new(RwLock::new(HashMap::new())),
            active_ceremonies: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a peer's public key mapping
    pub async fn register_peer(&self, public_key: PublicKey, peer_id: PeerId) {
        let mut mapping = self.peer_mapping.write().await;
        mapping.insert(public_key, peer_id);
        debug!(
            pubkey = hex::encode(public_key.as_bytes()),
            peer_id = %peer_id,
            "Registered peer mapping"
        );
    }

    /// Start a DKG ceremony for a committee epoch
    ///
    /// Returns messages to broadcast to committee members
    pub async fn start_ceremony(
        &self,
        epoch_id: u64,
        committee_members: Vec<PublicKey>,
    ) -> Result<Vec<(PeerId, DKGMessage)>> {
        // Check if we're in the committee
        let our_participant_id = committee_members
            .iter()
            .position(|pk| pk == &self.config.our_public_key);

        let our_participant_id = match our_participant_id {
            Some(id) => id,
            None => {
                info!(
                    epoch_id,
                    "Not a committee member, skipping DKG ceremony"
                );
                return Ok(vec![]);
            }
        };

        // Calculate threshold (typically 2/3 + 1)
        let threshold = (committee_members.len() * 2 / 3) + 1;
        let threshold_config = ThresholdConfig::new(committee_members.len(), threshold)
            .map_err(|e| PoUWError::BLSError(e))?;

        info!(
            epoch_id,
            participant_id = our_participant_id,
            committee_size = committee_members.len(),
            threshold,
            "🔐 Starting DKG ceremony"
        );

        // Start ceremony in DKGManager
        self.dkg_manager
            .start_ceremony(epoch_id, our_participant_id, threshold_config)
            .await?;

        // Generate our commitment
        let commitment = self.dkg_manager.generate_commitment(epoch_id).await?;

        // Add our own commitment to our DKG manager
        self.dkg_manager.add_commitment(epoch_id, commitment.clone()).await?;

        // Track this ceremony
        let ceremony = ActiveCeremony {
            our_participant_id,
            committee_members: committee_members.clone(),
            commitments_received: 1, // We have our own
            shares_sent: false,
        };

        let mut ceremonies = self.active_ceremonies.write().await;
        ceremonies.insert(epoch_id, ceremony);

        // Build commitment message to broadcast
        let dkg_msg = DKGMessage::Commitment {
            epoch_id,
            commitment,
        };

        // Get PeerIds for all committee members (except us)
        let peer_mapping = self.peer_mapping.read().await;
        let mut messages = Vec::new();

        for member_pk in &committee_members {
            if member_pk == &self.config.our_public_key {
                continue; // Don't send to ourselves
            }

            if let Some(peer_id) = peer_mapping.get(member_pk) {
                messages.push((*peer_id, dkg_msg.clone()));
            } else {
                warn!(
                    member = hex::encode(member_pk.as_bytes()),
                    "No peer mapping for committee member"
                );
            }
        }

        info!(
            epoch_id,
            broadcast_count = messages.len(),
            "📤 Broadcasting DKG commitment"
        );

        Ok(messages)
    }

    /// Handle received DKG commitment
    ///
    /// Returns messages to send if we're ready to exchange shares
    pub async fn handle_commitment(
        &self,
        epoch_id: u64,
        commitment: DKGCommitment,
        from_peer: PeerId,
    ) -> Result<Vec<(PeerId, DKGMessage)>> {
        // Add commitment to DKGManager
        self.dkg_manager.add_commitment(epoch_id, commitment.clone()).await?;

        // Update ceremony state
        let mut ceremonies = self.active_ceremonies.write().await;
        let ceremony = match ceremonies.get_mut(&epoch_id) {
            Some(c) => c,
            None => {
                debug!(epoch_id, "Received commitment for unknown ceremony");
                return Ok(vec![]);
            }
        };

        ceremony.commitments_received += 1;

        info!(
            epoch_id,
            from_participant = commitment.participant_id,
            from_peer = %from_peer,
            received = ceremony.commitments_received,
            required = ceremony.committee_members.len(),
            "✅ Received DKG commitment"
        );

        // Check if we have all commitments and haven't sent shares yet
        if ceremony.commitments_received >= ceremony.committee_members.len() && !ceremony.shares_sent {
            info!(epoch_id, "🔐 All commitments received, generating shares");

            // Generate shares for all participants
            let shares = self.dkg_manager.generate_shares(epoch_id).await?;
            ceremony.shares_sent = true;

            // Build messages to send shares to specific recipients
            let peer_mapping = self.peer_mapping.read().await;
            let mut messages = Vec::new();

            for share in shares {
                // Find the recipient's PeerId
                if let Some(recipient_pk) = ceremony.committee_members.get(share.to) {
                    if recipient_pk == &self.config.our_public_key {
                        // This is our own share, add it locally
                        self.dkg_manager.add_share(epoch_id, share.clone()).await?;
                        continue;
                    }

                    if let Some(peer_id) = peer_mapping.get(recipient_pk) {
                        let dkg_msg = DKGMessage::Share {
                            epoch_id,
                            share: share.clone(),
                        };
                        messages.push((*peer_id, dkg_msg));
                    } else {
                        warn!(
                            recipient = hex::encode(recipient_pk.as_bytes()),
                            "No peer mapping for share recipient"
                        );
                    }
                }
            }

            info!(
                epoch_id,
                shares_sent = messages.len(),
                "📤 Sending DKG shares to committee members"
            );

            // Check if ceremony is ready to finalize after adding our own share
            // This handles the case where our own share completes the ceremony
            if let Some(completion_msgs) = self.try_finalize_ceremony(epoch_id).await? {
                messages.extend(completion_msgs);
            }

            return Ok(messages);
        }

        Ok(vec![])
    }

    /// Try to finalize ceremony if ready
    ///
    /// Returns completion messages to broadcast if ceremony was finalized
    async fn try_finalize_ceremony(&self, epoch_id: u64) -> Result<Option<Vec<(PeerId, DKGMessage)>>> {
        // Check if the ceremony is ready to finalize (not if it's already finalized)
        // The state transitions to Verifying when all shares are received
        let ceremony_ready = match self.dkg_manager.get_ceremony_state(epoch_id).await {
            Ok(state) => matches!(state, DKGState::Verifying),
            Err(_) => false,
        };

        if ceremony_ready && !self.dkg_manager.is_complete(epoch_id).await {
            info!(epoch_id, "🎉 DKG ceremony complete, finalizing");

            // Finalize ceremony
            let public_key_set = self.dkg_manager.finalize_ceremony(epoch_id).await?;

            // Provide PublicKeySet to BLSCoordinator
            self.bls_coordinator.set_public_key_set(public_key_set).await;

            // Get our participant ID
            let ceremonies = self.active_ceremonies.read().await;
            let participant_id = ceremonies
                .get(&epoch_id)
                .map(|c| c.our_participant_id)
                .unwrap_or(0);

            // Broadcast completion message
            let peer_mapping = self.peer_mapping.read().await;
            let ceremonies_read = self.active_ceremonies.read().await;
            let ceremony = ceremonies_read.get(&epoch_id);

            let mut messages = Vec::new();
            if let Some(ceremony) = ceremony {
                for member_pk in &ceremony.committee_members {
                    if member_pk == &self.config.our_public_key {
                        continue;
                    }

                    if let Some(peer_id) = peer_mapping.get(member_pk) {
                        let dkg_msg = DKGMessage::Complete {
                            epoch_id,
                            participant_id,
                        };
                        messages.push((*peer_id, dkg_msg));
                    }
                }
            }

            info!(
                epoch_id,
                "✨ DKG ceremony finalized, BLSCoordinator updated"
            );

            return Ok(Some(messages));
        }

        Ok(None)
    }

    /// Handle received DKG share
    ///
    /// Returns completion message if ceremony is done
    pub async fn handle_share(
        &self,
        epoch_id: u64,
        share: DKGShare,
        from_peer: PeerId,
    ) -> Result<Option<Vec<(PeerId, DKGMessage)>>> {
        // Add share to DKGManager (performs Feldman VSS verification)
        self.dkg_manager.add_share(epoch_id, share.clone()).await?;

        info!(
            epoch_id,
            from_participant = share.from,
            from_peer = %from_peer,
            "✅ Received and verified DKG share"
        );

        // Try to finalize ceremony if all shares received
        self.try_finalize_ceremony(epoch_id).await
    }

    /// Handle completion notification
    pub async fn handle_complete(
        &self,
        epoch_id: u64,
        participant_id: usize,
        from_peer: PeerId,
    ) {
        info!(
            epoch_id,
            participant_id,
            from_peer = %from_peer,
            "✅ Participant completed DKG"
        );

        // This is informational - we track completion but don't need to act on it
        // The important thing is that our own ceremony is complete
    }

    /// Check if a ceremony is complete
    pub async fn is_ceremony_complete(&self, epoch_id: u64) -> bool {
        self.dkg_manager.is_complete(epoch_id).await
    }

    /// Get our participant ID for an epoch (if we're in the committee)
    pub async fn get_our_participant_id(&self, epoch_id: u64) -> Option<usize> {
        let ceremonies = self.active_ceremonies.read().await;
        ceremonies.get(&epoch_id).map(|c| c.our_participant_id)
    }
}
