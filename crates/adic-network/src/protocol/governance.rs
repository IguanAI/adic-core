//! Governance Protocol Handler
//!
//! Network layer for governance messages (PoUW III §3).
//! Integrates with the adic-governance crate for proposal, voting, and receipt processing.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

use adic_types::{MessageId, PublicKey, Result as AdicResult, AdicError};
use adic_crypto::bls::{BLSSignatureShare, BLSSignature, BLSThresholdSigner, ThresholdConfig};
use threshold_crypto::PublicKeySet;

/// Governance protocol configuration
#[derive(Debug, Clone)]
pub struct GovernanceConfig {
    /// Timeout for voting period
    pub voting_period: Duration,
    /// Minimum reputation to submit proposals
    pub min_proposal_reputation: f64,
    /// Minimum reputation to vote
    pub min_vote_reputation: f64,
    /// Maximum concurrent proposals
    pub max_concurrent_proposals: usize,
}

impl Default for GovernanceConfig {
    fn default() -> Self {
        Self {
            voting_period: Duration::from_secs(7 * 24 * 3600), // 7 days
            min_proposal_reputation: 100.0,
            min_vote_reputation: 10.0,
            max_concurrent_proposals: 10,
        }
    }
}

/// Governance message types for network propagation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GovernanceMessage {
    /// Governance proposal submission
    Proposal {
        message_id: MessageId,
        proposal_id: [u8; 32],
        submitter: PublicKey,
        class: String, // "constitutional" | "operational"
        param_keys: Vec<String>,
        timestamp: u64,
    },

    /// Vote on a proposal
    Vote {
        message_id: MessageId,
        proposal_id: [u8; 32],
        voter: PublicKey,
        ballot: String, // "yes" | "no" | "abstain"
        credits: f64,
        timestamp: u64,
    },

    /// Receipt with BLS threshold signature
    Receipt {
        message_id: MessageId,
        proposal_id: [u8; 32],
        result: String, // "pass" | "fail" | "inconclusive"
        credits_yes: f64,
        credits_no: f64,
        threshold_sig: Vec<u8>,
        timestamp: u64,
    },

    /// BLS signature share for receipt generation
    /// Committee members broadcast their signature shares for aggregation
    SignatureShare {
        proposal_id: [u8; 32],
        signer: PublicKey,
        share: BLSSignatureShare,
        timestamp: u64,
    },

    /// Request signature shares from committee
    /// Sent by receipt coordinator to gather shares
    SignatureShareRequest {
        proposal_id: [u8; 32],
        requester: PublicKey,
        message_hash: [u8; 32],
        timestamp: u64,
    },
}

/// Governance protocol state tracking
///
/// Note: Fields are currently unused but retained for future protocol extensions
/// (e.g., proposal verification, receipt correlation, metrics)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ProposalTracking {
    proposal_id: [u8; 32],
    message_id: MessageId,
    submitter: PublicKey,
    received_at: Instant,
    vote_count: usize,
    receipt_received: bool,
}

/// BLS signature share collection state
#[derive(Debug, Clone)]
struct SignatureCollection {
    proposal_id: [u8; 32],
    message_hash: [u8; 32],
    shares: HashMap<PublicKey, BLSSignatureShare>,
    started_at: Instant,
}

/// Governance protocol handler
pub struct GovernanceProtocol {
    config: GovernanceConfig,
    /// Track active proposals
    active_proposals: Arc<RwLock<HashMap<[u8; 32], ProposalTracking>>>,
    /// Track votes per proposal
    votes: Arc<RwLock<HashMap<[u8; 32], HashMap<PublicKey, String>>>>,
    /// Track BLS signature share collections
    signature_collections: Arc<RwLock<HashMap<[u8; 32], SignatureCollection>>>,
    /// BLS threshold signer for signature verification
    bls_signer: Option<BLSThresholdSigner>,
    /// Public key set for BLS signature verification (from DKG)
    public_key_set: Option<Arc<PublicKeySet>>,
    /// Event channel for governance events
    event_sender: mpsc::UnboundedSender<GovernanceEvent>,
    event_receiver: Arc<RwLock<mpsc::UnboundedReceiver<GovernanceEvent>>>,
}

/// Governance events emitted by the protocol
#[derive(Debug, Clone)]
pub enum GovernanceEvent {
    /// New proposal received
    ProposalReceived {
        proposal_id: [u8; 32],
        message_id: MessageId,
        submitter: PublicKey,
    },

    /// Vote received
    VoteReceived {
        proposal_id: [u8; 32],
        voter: PublicKey,
        ballot: String,
    },

    /// Receipt received (proposal decided)
    ReceiptReceived {
        proposal_id: [u8; 32],
        result: String,
    },

    /// Voting period expired
    VotingPeriodExpired {
        proposal_id: [u8; 32],
    },

    /// BLS signature share received
    SignatureShareReceived {
        proposal_id: [u8; 32],
        signer: PublicKey,
        share_count: usize,
    },

    /// BLS signature shares aggregated into receipt
    SignatureAggregated {
        proposal_id: [u8; 32],
        signature: BLSSignature,
    },
}

impl GovernanceProtocol {
    /// Create new governance protocol handler
    pub fn new(config: GovernanceConfig) -> Self {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();

        Self {
            config,
            active_proposals: Arc::new(RwLock::new(HashMap::new())),
            votes: Arc::new(RwLock::new(HashMap::new())),
            signature_collections: Arc::new(RwLock::new(HashMap::new())),
            bls_signer: None,
            public_key_set: None,
            event_sender,
            event_receiver: Arc::new(RwLock::new(event_receiver)),
        }
    }

    /// Configure BLS threshold signature verification
    ///
    /// Should be called after DKG completes to enable signature verification.
    /// Per PoUW III §10.3: Committee uses BLS threshold signatures for receipts.
    pub fn configure_bls(&mut self, threshold_config: ThresholdConfig, public_key_set: PublicKeySet) {
        self.bls_signer = Some(BLSThresholdSigner::new(threshold_config));
        self.public_key_set = Some(Arc::new(public_key_set));
        info!(
            threshold = threshold_config.threshold,
            total = threshold_config.total_participants,
            "🔐 BLS signature verification configured for governance"
        );
    }

    /// Handle incoming governance message
    pub async fn handle_message(&self, msg: GovernanceMessage) -> AdicResult<()> {
        match msg {
            GovernanceMessage::Proposal {
                message_id,
                proposal_id,
                submitter,
                class: _,
                param_keys,
                timestamp: _,
            } => {
                self.handle_proposal(message_id, proposal_id, submitter, param_keys)
                    .await
            }
            GovernanceMessage::Vote {
                message_id: _,
                proposal_id,
                voter,
                ballot,
                credits,
                timestamp: _,
            } => {
                self.handle_vote(proposal_id, voter, ballot, credits)
                    .await
            }
            GovernanceMessage::Receipt {
                message_id: _,
                proposal_id,
                result,
                credits_yes: _,
                credits_no: _,
                threshold_sig: _,
                timestamp: _,
            } => {
                self.handle_receipt(proposal_id, result).await
            }
            GovernanceMessage::SignatureShare {
                proposal_id,
                signer,
                share,
                timestamp: _,
            } => {
                self.handle_signature_share(proposal_id, signer, share).await
            }
            GovernanceMessage::SignatureShareRequest {
                proposal_id,
                requester: _,
                message_hash,
                timestamp: _,
            } => {
                self.handle_signature_share_request(proposal_id, message_hash).await
            }
        }
    }

    /// Handle proposal submission
    async fn handle_proposal(
        &self,
        message_id: MessageId,
        proposal_id: [u8; 32],
        submitter: PublicKey,
        param_keys: Vec<String>,
    ) -> AdicResult<()> {
        let mut proposals = self.active_proposals.write().await;

        // Check if proposal already exists
        if proposals.contains_key(&proposal_id) {
            debug!(
                proposal_id = hex::encode(&proposal_id[..8]),
                "Proposal already tracked"
            );
            return Ok(());
        }

        // Check max concurrent proposals
        if proposals.len() >= self.config.max_concurrent_proposals {
            warn!("Max concurrent proposals reached, dropping oldest");
            // Remove oldest proposal
            if let Some((oldest_id, _)) = proposals
                .iter()
                .min_by_key(|(_, tracking)| tracking.received_at)
            {
                let oldest_id = *oldest_id;
                proposals.remove(&oldest_id);
            }
        }

        // Add proposal tracking
        proposals.insert(
            proposal_id,
            ProposalTracking {
                proposal_id,
                message_id,
                submitter,
                received_at: Instant::now(),
                vote_count: 0,
                receipt_received: false,
            },
        );

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            submitter = hex::encode(&submitter.as_bytes()[..8]),
            params = ?param_keys,
            "📋 Governance proposal received"
        );

        // Emit event
        let _ = self.event_sender.send(GovernanceEvent::ProposalReceived {
            proposal_id,
            message_id,
            submitter,
        });

        Ok(())
    }

    /// Handle vote submission
    async fn handle_vote(
        &self,
        proposal_id: [u8; 32],
        voter: PublicKey,
        ballot: String,
        credits: f64,
    ) -> AdicResult<()> {
        // Check if proposal exists
        let proposals = self.active_proposals.read().await;
        if !proposals.contains_key(&proposal_id) {
            warn!(
                proposal_id = hex::encode(&proposal_id[..8]),
                "Vote for unknown proposal, ignoring"
            );
            return Ok(());
        }
        drop(proposals);

        // Record vote
        let mut votes = self.votes.write().await;
        let proposal_votes = votes.entry(proposal_id).or_insert_with(HashMap::new);

        // Check for duplicate vote
        if proposal_votes.contains_key(&voter) {
            debug!(
                proposal_id = hex::encode(&proposal_id[..8]),
                voter = hex::encode(&voter.as_bytes()[..8]),
                "Duplicate vote, ignoring"
            );
            return Ok(());
        }

        proposal_votes.insert(voter, ballot.clone());

        // Update vote count
        let mut proposals = self.active_proposals.write().await;
        if let Some(tracking) = proposals.get_mut(&proposal_id) {
            tracking.vote_count += 1;
        }

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            voter = hex::encode(&voter.as_bytes()[..8]),
            ballot = ballot,
            credits = credits,
            "🗳️ Vote received"
        );

        // Emit event
        let _ = self.event_sender.send(GovernanceEvent::VoteReceived {
            proposal_id,
            voter,
            ballot,
        });

        Ok(())
    }

    /// Handle receipt (proposal decision)
    async fn handle_receipt(
        &self,
        proposal_id: [u8; 32],
        result: String,
    ) -> AdicResult<()> {
        let mut proposals = self.active_proposals.write().await;

        // Check if proposal exists
        if !proposals.contains_key(&proposal_id) {
            warn!(
                proposal_id = hex::encode(&proposal_id[..8]),
                "Receipt for unknown proposal, ignoring"
            );
            return Ok(());
        }

        // Mark receipt as received
        if let Some(tracking) = proposals.get_mut(&proposal_id) {
            if tracking.receipt_received {
                debug!(
                    proposal_id = hex::encode(&proposal_id[..8]),
                    "Receipt already received, ignoring duplicate"
                );
                return Ok(());
            }
            tracking.receipt_received = true;
        }

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            result = result,
            "✅ Governance receipt received"
        );

        // Emit event
        let _ = self.event_sender.send(GovernanceEvent::ReceiptReceived {
            proposal_id,
            result,
        });

        // Clean up old votes for this proposal
        let mut votes = self.votes.write().await;
        votes.remove(&proposal_id);

        Ok(())
    }

    /// Handle BLS signature share
    ///
    /// Collects signature shares from committee members for aggregation into threshold signature.
    /// Per PoUW III §10.3: Committee members sign receipts with BLS threshold signatures.
    async fn handle_signature_share(
        &self,
        proposal_id: [u8; 32],
        signer: PublicKey,
        share: BLSSignatureShare,
    ) -> AdicResult<()> {
        let mut collections = self.signature_collections.write().await;

        // Check if collection exists for this proposal
        if let Some(collection) = collections.get_mut(&proposal_id) {
            // Check for duplicate share from this signer
            if collection.shares.contains_key(&signer) {
                debug!(
                    proposal_id = hex::encode(&proposal_id[..8]),
                    signer = hex::encode(&signer.as_bytes()[..8]),
                    "Duplicate signature share, ignoring"
                );
                return Ok(());
            }

            // Verify signature share against message hash before accepting
            // Per BLS threshold signature scheme (PoUW III §10.3): each share must be valid
            if let (Some(bls_signer), Some(pk_set)) = (&self.bls_signer, &self.public_key_set) {
                let is_valid = bls_signer
                    .verify_share(
                        pk_set,
                        &collection.message_hash,
                        adic_crypto::bls::dst::GOV_RECEIPT,
                        &share,
                    )
                    .unwrap_or(false);

                if !is_valid {
                    warn!(
                        proposal_id = hex::encode(&proposal_id[..8]),
                        signer = hex::encode(&signer.as_bytes()[..8]),
                        message_hash = hex::encode(&collection.message_hash[..8]),
                        "❌ Invalid BLS signature share, rejecting"
                    );
                    return Err(AdicError::SignatureVerification);
                }
            } else {
                debug!(
                    proposal_id = hex::encode(&proposal_id[..8]),
                    "⚠️ BLS verification not configured, accepting share without verification"
                );
            }

            // Add share to collection
            collection.shares.insert(signer, share);
            let share_count = collection.shares.len();

            info!(
                proposal_id = hex::encode(&proposal_id[..8]),
                signer = hex::encode(&signer.as_bytes()[..8]),
                total_shares = share_count,
                "🔑 BLS signature share received"
            );

            // Emit event
            let _ = self.event_sender.send(GovernanceEvent::SignatureShareReceived {
                proposal_id,
                signer,
                share_count,
            });
        } else {
            debug!(
                proposal_id = hex::encode(&proposal_id[..8]),
                "Signature share for unknown collection, ignoring"
            );
        }

        Ok(())
    }

    /// Handle signature share request
    ///
    /// Initiates BLS signature share collection for receipt generation.
    async fn handle_signature_share_request(
        &self,
        proposal_id: [u8; 32],
        message_hash: [u8; 32],
    ) -> AdicResult<()> {
        let mut collections = self.signature_collections.write().await;

        // Check if collection already exists
        if collections.contains_key(&proposal_id) {
            debug!(
                proposal_id = hex::encode(&proposal_id[..8]),
                "Signature collection already started"
            );
            return Ok(());
        }

        // Start new signature collection
        collections.insert(
            proposal_id,
            SignatureCollection {
                proposal_id,
                message_hash,
                shares: HashMap::new(),
                started_at: Instant::now(),
            },
        );

        info!(
            proposal_id = hex::encode(&proposal_id[..8]),
            message_hash = hex::encode(&message_hash[..8]),
            "🔐 BLS signature collection started"
        );

        Ok(())
    }

    /// Get collected signature shares for a proposal
    ///
    /// Returns all collected BLS signature shares for aggregation.
    pub async fn get_signature_shares(
        &self,
        proposal_id: &[u8; 32],
    ) -> Option<Vec<BLSSignatureShare>> {
        let collections = self.signature_collections.read().await;
        collections.get(proposal_id).map(|c| c.shares.values().cloned().collect())
    }

    /// Complete signature collection and return shares
    ///
    /// Removes collection from tracking and returns all shares for aggregation.
    pub async fn complete_signature_collection(
        &self,
        proposal_id: &[u8; 32],
    ) -> Option<Vec<BLSSignatureShare>> {
        let mut collections = self.signature_collections.write().await;
        collections.remove(proposal_id).map(|c| c.shares.into_values().collect())
    }

    /// Get proposal tracking info (internal use only)
    ///
    /// Provides access to proposal metadata for integration with higher-level governance logic.
    /// Used by node APIs and governance coordinators to query proposal status.
    #[allow(dead_code)]
    pub(crate) async fn get_proposal(&self, proposal_id: &[u8; 32]) -> Option<ProposalTracking> {
        let proposals = self.active_proposals.read().await;
        proposals.get(proposal_id).cloned()
    }

    /// Get vote count for a proposal
    pub async fn get_vote_count(&self, proposal_id: &[u8; 32]) -> usize {
        let votes = self.votes.read().await;
        votes.get(proposal_id).map(|v| v.len()).unwrap_or(0)
    }

    /// Get active proposal count
    pub async fn active_proposal_count(&self) -> usize {
        let proposals = self.active_proposals.read().await;
        proposals.len()
    }

    /// Get next governance event (non-blocking)
    pub async fn try_recv_event(&self) -> Option<GovernanceEvent> {
        let mut receiver = self.event_receiver.write().await;
        receiver.try_recv().ok()
    }

    /// Cleanup expired proposals and stale signature collections
    pub async fn cleanup_expired(&self) {
        let now = Instant::now();
        let mut proposals = self.active_proposals.write().await;
        let mut votes = self.votes.write().await;
        let mut signatures = self.signature_collections.write().await;

        let expired: Vec<[u8; 32]> = proposals
            .iter()
            .filter(|(_, tracking)| {
                !tracking.receipt_received
                    && now.duration_since(tracking.received_at) > self.config.voting_period
            })
            .map(|(id, _)| *id)
            .collect();

        for proposal_id in &expired {
            info!(
                proposal_id = hex::encode(&proposal_id[..8]),
                "⏰ Voting period expired"
            );

            // Emit expiry event
            let _ = self
                .event_sender
                .send(GovernanceEvent::VotingPeriodExpired {
                    proposal_id: *proposal_id,
                });

            proposals.remove(proposal_id);
            votes.remove(proposal_id);
        }

        if !expired.is_empty() {
            info!(count = expired.len(), "Cleaned up expired proposals");
        }

        // Cleanup stale signature collections (timeout after 5 minutes)
        let signature_timeout = Duration::from_secs(300);
        let stale_collections: Vec<[u8; 32]> = signatures
            .iter()
            .filter(|(_, collection)| {
                now.duration_since(collection.started_at) > signature_timeout
            })
            .map(|(_, collection)| collection.proposal_id)
            .collect();

        for proposal_id in &stale_collections {
            info!(
                proposal_id = hex::encode(&proposal_id[..8]),
                "⏰ Signature collection timed out"
            );
            signatures.remove(proposal_id);
        }

        if !stale_collections.is_empty() {
            info!(
                count = stale_collections.len(),
                "Cleaned up stale signature collections"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_proposal_tracking() {
        let protocol = GovernanceProtocol::new(GovernanceConfig::default());

        let message_id = MessageId::from_bytes([1u8; 32]);
        let proposal_id = [2u8; 32];
        let submitter = PublicKey::from_bytes([3u8; 32]);

        // Handle proposal
        protocol
            .handle_proposal(message_id, proposal_id, submitter, vec!["rho".to_string()])
            .await
            .unwrap();

        // Verify tracking
        let tracking = protocol.get_proposal(&proposal_id).await.unwrap();
        assert_eq!(tracking.proposal_id, proposal_id);
        assert_eq!(tracking.submitter, submitter);
        assert_eq!(tracking.vote_count, 0);

        // Verify event
        let event = protocol.try_recv_event().await.unwrap();
        match event {
            GovernanceEvent::ProposalReceived {
                proposal_id: p,
                submitter: s,
                ..
            } => {
                assert_eq!(p, proposal_id);
                assert_eq!(s, submitter);
            }
            _ => panic!("Wrong event type"),
        }
    }

    #[tokio::test]
    async fn test_vote_tracking() {
        let protocol = GovernanceProtocol::new(GovernanceConfig::default());

        let message_id = MessageId::from_bytes([1u8; 32]);
        let proposal_id = [2u8; 32];
        let submitter = PublicKey::from_bytes([3u8; 32]);
        let voter = PublicKey::from_bytes([4u8; 32]);

        // Submit proposal first
        protocol
            .handle_proposal(message_id, proposal_id, submitter, vec!["rho".to_string()])
            .await
            .unwrap();

        // Submit vote
        protocol
            .handle_vote(proposal_id, voter, "yes".to_string(), 10.0)
            .await
            .unwrap();

        // Verify vote count
        let tracking = protocol.get_proposal(&proposal_id).await.unwrap();
        assert_eq!(tracking.vote_count, 1);

        let vote_count = protocol.get_vote_count(&proposal_id).await;
        assert_eq!(vote_count, 1);

        // Try duplicate vote
        protocol
            .handle_vote(proposal_id, voter, "no".to_string(), 10.0)
            .await
            .unwrap();

        // Vote count should still be 1
        let vote_count = protocol.get_vote_count(&proposal_id).await;
        assert_eq!(vote_count, 1);
    }

    #[tokio::test]
    async fn test_receipt_handling() {
        let protocol = GovernanceProtocol::new(GovernanceConfig::default());

        let message_id = MessageId::from_bytes([1u8; 32]);
        let proposal_id = [2u8; 32];
        let submitter = PublicKey::from_bytes([3u8; 32]);

        // Submit proposal
        protocol
            .handle_proposal(message_id, proposal_id, submitter, vec!["rho".to_string()])
            .await
            .unwrap();

        // Submit receipt
        protocol
            .handle_receipt(proposal_id, "pass".to_string())
            .await
            .unwrap();

        // Verify receipt received flag
        let tracking = protocol.get_proposal(&proposal_id).await.unwrap();
        assert!(tracking.receipt_received);

        // Verify votes cleaned up
        let vote_count = protocol.get_vote_count(&proposal_id).await;
        assert_eq!(vote_count, 0);
    }
}
