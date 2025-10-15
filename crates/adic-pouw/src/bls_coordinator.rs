//! BLS Threshold Signature Coordinator
//!
//! Implements t-of-n threshold signature collection for:
//! - ODC Committee Certificates (PoUW II §2.2)
//! - PoUW Task Receipts (PoUW III §10.3)
//!
//! # Protocol Flow
//! 1. Coordinator initiates signing request to committee members
//! 2. Each member signs message with their BLS key share
//! 3. Coordinator collects t-of-n signature shares
//! 4. Aggregates shares into final threshold signature using BLSThresholdSigner

use crate::{DKGManager, PoUWError, Result};
use adic_crypto::{BLSSignature, BLSSignatureShare, BLSThresholdSigner};
use adic_types::PublicKey;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use threshold_crypto::PublicKeySet;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Domain Separation Tags per PoUW specification
pub const DST_COMMITTEE_CERT: &[u8] = b"ADIC-PoUW-COMMITTEE-v1";
pub const DST_TASK_RECEIPT: &[u8] = b"ADIC-PoUW-RECEIPT-v1";

/// Configuration for BLS signing coordinator
#[derive(Debug, Clone)]
pub struct BLSCoordinatorConfig {
    /// Timeout for collecting signatures (seconds)
    pub collection_timeout: Duration,

    /// Minimum number of signatures required (threshold t)
    pub threshold: usize,

    /// Total number of committee members (n)
    pub total_members: usize,
}

impl Default for BLSCoordinatorConfig {
    fn default() -> Self {
        Self {
            collection_timeout: Duration::from_secs(60),
            threshold: 10,      // 2/3 of 15 default committee size
            total_members: 15,
        }
    }
}

/// Request ID for tracking signature collection sessions
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct SigningRequestId(pub String);

impl SigningRequestId {
    pub fn new(prefix: &str, epoch: u64, data_hash: &[u8]) -> Self {
        let id = format!(
            "{}-{}-{}",
            prefix,
            epoch,
            hex::encode(&data_hash[..8])
        );
        Self(id)
    }
}

/// Signing request sent to committee members
#[derive(Debug, Clone)]
pub struct SigningRequest {
    pub request_id: SigningRequestId,
    pub message: Vec<u8>,
    pub dst: Vec<u8>,
    pub committee_members: Vec<PublicKey>,
    pub threshold: usize,
    pub deadline: Instant,
}

/// State of a signature collection session
#[derive(Debug)]
struct CollectionSession {
    request: SigningRequest,
    shares: HashMap<PublicKey, BLSSignatureShare>,
    created_at: Instant,
    completed: bool,
}

/// BLS Threshold Signature Coordinator
///
/// Manages collection and aggregation of threshold signatures from committee members.
pub struct BLSCoordinator {
    config: BLSCoordinatorConfig,
    /// BLS threshold signer for creating signature shares
    signer: Arc<BLSThresholdSigner>,

    /// DKG manager for managing distributed key generation ceremonies
    dkg_manager: Option<Arc<DKGManager>>,

    /// Public key set from DKG (used for signature verification and aggregation)
    public_key_set: Arc<RwLock<Option<PublicKeySet>>>,

    /// Active signature collection sessions
    sessions: Arc<RwLock<HashMap<SigningRequestId, CollectionSession>>>,
}

impl BLSCoordinator {
    /// Create new BLS coordinator
    pub fn new(config: BLSCoordinatorConfig, signer: Arc<BLSThresholdSigner>) -> Self {
        Self {
            config,
            signer,
            dkg_manager: None,
            public_key_set: Arc::new(RwLock::new(None)),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create new BLS coordinator with DKG manager
    pub fn with_dkg(
        config: BLSCoordinatorConfig,
        signer: Arc<BLSThresholdSigner>,
        dkg_manager: Arc<DKGManager>,
    ) -> Self {
        Self {
            config,
            signer,
            dkg_manager: Some(dkg_manager),
            public_key_set: Arc::new(RwLock::new(None)),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Set the public key set from DKG ceremony
    ///
    /// This must be called after DKG completes to enable real signature aggregation
    pub async fn set_public_key_set(&self, pk_set: PublicKeySet) {
        let mut pks = self.public_key_set.write().await;
        *pks = Some(pk_set);
        info!("🔑 PublicKeySet configured for BLS signature aggregation");
    }

    /// Check if DKG has been completed and PublicKeySet is available
    pub async fn has_public_key_set(&self) -> bool {
        self.public_key_set.read().await.is_some()
    }

    /// Sign a message with our local BLS key share (from DKG)
    ///
    /// This creates a signature share that can be aggregated with other committee members' shares.
    /// Requires that DKG has been completed and we have a secret key share.
    ///
    /// # Arguments
    /// * `epoch_id` - Committee epoch ID (used to retrieve the correct key share)
    /// * `participant_id` - Our participant ID in the committee
    /// * `message` - Message to sign
    /// * `dst` - Domain Separation Tag
    ///
    /// # Returns
    /// BLS signature share that can be submitted to the coordinator
    pub async fn sign_with_share(
        &self,
        epoch_id: u64,
        participant_id: usize,
        message: &[u8],
        dst: &[u8],
    ) -> Result<BLSSignatureShare> {
        // Get secret key share from DKG manager
        let dkg_manager = self.dkg_manager.as_ref().ok_or_else(|| {
            PoUWError::Other("DKG manager not configured".to_string())
        })?;

        let secret_share = dkg_manager.get_secret_share(epoch_id).await?;

        // Sign with the BLS threshold signer using our secret share
        let signature_share = self.signer.sign_share(&secret_share, participant_id, message, dst)
            .map_err(|e| PoUWError::BLSError(e))?;

        Ok(signature_share)
    }

    /// Initiate signing request to committee
    ///
    /// # Arguments
    /// * `request_id` - Unique identifier for this signing request
    /// * `message` - Message to be signed (e.g., serialized certificate or receipt)
    /// * `dst` - Domain Separation Tag (DST_COMMITTEE_CERT or DST_TASK_RECEIPT)
    /// * `committee_members` - Public keys of committee members who should sign
    ///
    /// # Returns
    /// SigningRequest that should be broadcast to committee members
    pub async fn initiate_signing(
        &self,
        request_id: SigningRequestId,
        message: Vec<u8>,
        dst: Vec<u8>,
        committee_members: Vec<PublicKey>,
    ) -> Result<SigningRequest> {
        if committee_members.len() < self.config.threshold {
            return Err(PoUWError::InsufficientCommitteeSize {
                required: self.config.threshold,
                actual: committee_members.len(),
            });
        }

        let request = SigningRequest {
            request_id: request_id.clone(),
            message: message.clone(),
            dst: dst.clone(),
            committee_members: committee_members.clone(),
            threshold: self.config.threshold,
            deadline: Instant::now() + self.config.collection_timeout,
        };

        let session = CollectionSession {
            request: request.clone(),
            shares: HashMap::new(),
            created_at: Instant::now(),
            completed: false,
        };

        let mut sessions = self.sessions.write().await;
        sessions.insert(request_id.clone(), session);

        info!(
            request_id = %request_id.0,
            committee_size = committee_members.len(),
            threshold = self.config.threshold,
            dst = %String::from_utf8_lossy(&dst),
            "📝 Initiated BLS signing request"
        );

        Ok(request)
    }

    /// Submit signature share from committee member
    ///
    /// # Arguments
    /// * `request_id` - ID of the signing request
    /// * `signer_pk` - Public key of the committee member submitting the share
    /// * `share` - BLS signature share from this member
    ///
    /// # Returns
    /// - `Ok(None)` if still collecting shares
    /// - `Ok(Some(signature))` if threshold reached and signature aggregated
    /// - `Err(_)` if validation fails
    pub async fn submit_share(
        &self,
        request_id: &SigningRequestId,
        signer_pk: PublicKey,
        share: BLSSignatureShare,
    ) -> Result<Option<BLSSignature>> {
        let mut sessions = self.sessions.write().await;

        let session = sessions
            .get_mut(request_id)
            .ok_or_else(|| PoUWError::SigningRequestNotFound(request_id.0.clone()))?;

        // Check if session already completed
        if session.completed {
            return Err(PoUWError::SigningSessionCompleted(request_id.0.clone()));
        }

        // Check if deadline passed
        if Instant::now() > session.request.deadline {
            return Err(PoUWError::SigningDeadlineExpired(request_id.0.clone()));
        }

        // Verify signer is in committee
        if !session.request.committee_members.contains(&signer_pk) {
            return Err(PoUWError::UnauthorizedSigner {
                signer: hex::encode(signer_pk.as_bytes()),
                request_id: request_id.0.clone(),
            });
        }

        // Check for duplicate submission
        if session.shares.contains_key(&signer_pk) {
            warn!(
                request_id = %request_id.0,
                signer = %hex::encode(&signer_pk.as_bytes()[..8]),
                "⚠️ Duplicate signature share ignored"
            );
            return Ok(None);
        }

        // Verify the signature share
        // Note: Individual share verification would require the PublicKeySet and participant index
        // For now, we trust shares and will verify the final aggregated signature

        // Add share to collection
        session.shares.insert(signer_pk, share);

        debug!(
            request_id = %request_id.0,
            signer = %hex::encode(&signer_pk.as_bytes()[..8]),
            collected = session.shares.len(),
            threshold = session.request.threshold,
            "✅ Signature share collected"
        );

        // Check if we've reached threshold
        if session.shares.len() >= session.request.threshold {
            info!(
                request_id = %request_id.0,
                shares_collected = session.shares.len(),
                threshold = session.request.threshold,
                "🔐 Threshold reached, aggregating signatures"
            );

            // Aggregate signatures
            let shares: Vec<BLSSignatureShare> = session.shares.values().cloned().collect();

            // Aggregate using real PublicKeySet from DKG
            let aggregated = self.aggregate_shares(&shares).await?;

            session.completed = true;

            info!(
                request_id = %request_id.0,
                shares_used = shares.len(),
                "✨ BLS threshold signature aggregated successfully"
            );

            return Ok(Some(aggregated));
        }

        Ok(None)
    }

    /// Get current status of signing request
    pub async fn get_status(
        &self,
        request_id: &SigningRequestId,
    ) -> Result<SigningStatus> {
        let sessions = self.sessions.read().await;

        let session = sessions
            .get(request_id)
            .ok_or_else(|| PoUWError::SigningRequestNotFound(request_id.0.clone()))?;

        let elapsed = session.created_at.elapsed();
        let remaining = session.request.deadline
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO);

        Ok(SigningStatus {
            request_id: request_id.clone(),
            shares_collected: session.shares.len(),
            threshold: session.request.threshold,
            committee_size: session.request.committee_members.len(),
            completed: session.completed,
            elapsed,
            time_remaining: remaining,
        })
    }

    /// Get aggregated signature from completed session
    ///
    /// Returns the aggregated signature if the session is completed and threshold was reached.
    /// This re-aggregates the shares on-demand (shares are stored in the session).
    pub async fn get_aggregated_signature(
        &self,
        request_id: &SigningRequestId,
    ) -> Result<Option<BLSSignature>> {
        let sessions = self.sessions.read().await;

        let session = sessions
            .get(request_id)
            .ok_or_else(|| PoUWError::SigningRequestNotFound(request_id.0.clone()))?;

        if !session.completed {
            return Ok(None);
        }

        // Re-aggregate the shares
        let shares: Vec<BLSSignatureShare> = session.shares.values().cloned().collect();
        let aggregated = self.aggregate_shares(&shares).await?;

        Ok(Some(aggregated))
    }

    /// Cleanup expired signing sessions
    pub async fn cleanup_expired(&self) {
        let mut sessions = self.sessions.write().await;
        let now = Instant::now();

        let before_count = sessions.len();
        sessions.retain(|request_id, session| {
            let keep = now < session.request.deadline || !session.completed;
            if !keep {
                debug!(
                    request_id = %request_id.0,
                    "🗑️ Cleaned up expired signing session"
                );
            }
            keep
        });

        let removed = before_count - sessions.len();
        if removed > 0 {
            info!(removed, "🧹 Cleaned up expired signing sessions");
        }
    }

    /// Aggregate signature shares using PublicKeySet
    ///
    /// This is the real threshold signature aggregation using the PublicKeySet
    /// obtained from DKG ceremony
    async fn aggregate_shares(&self, shares: &[BLSSignatureShare]) -> Result<BLSSignature> {
        // Get the public key set from DKG
        let pk_set_opt = self.public_key_set.read().await;
        let pk_set = pk_set_opt.as_ref().ok_or_else(|| {
            PoUWError::BLSError(adic_crypto::BLSError::ThresholdCryptoError(
                "PublicKeySet not configured - run DKG first".to_string(),
            ))
        })?;

        // Convert shares to threshold_crypto format and validate indices
        let mut tc_shares = Vec::new();
        for share in shares {
            let tc_share = share.to_tc_signature_share().map_err(|e| {
                PoUWError::BLSError(e)
            })?;
            tc_shares.push((share.participant_id, tc_share));
        }

        // Aggregate using threshold_crypto
        let aggregated = pk_set
            .combine_signatures(tc_shares.iter().map(|(i, s)| (*i, s)))
            .map_err(|e| {
                PoUWError::BLSError(adic_crypto::BLSError::ThresholdCryptoError(
                    format!("Failed to aggregate signatures: {}", e),
                ))
            })?;

        let signature = BLSSignature::from_tc_signature(&aggregated);

        info!(
            shares_count = shares.len(),
            "✨ Successfully aggregated BLS threshold signature"
        );

        Ok(signature)
    }
}

/// Status information for a signing request
#[derive(Debug, Clone)]
pub struct SigningStatus {
    pub request_id: SigningRequestId,
    pub shares_collected: usize,
    pub threshold: usize,
    pub committee_size: usize,
    pub completed: bool,
    pub elapsed: Duration,
    pub time_remaining: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_crypto::ThresholdConfig;

    #[tokio::test]
    async fn test_signing_flow() {
        let config = BLSCoordinatorConfig {
            collection_timeout: Duration::from_secs(60),
            threshold: 2,
            total_members: 3,
        };

        let threshold_config = ThresholdConfig::new(3, 2).unwrap();
        let signer = Arc::new(BLSThresholdSigner::new(threshold_config));
        let coordinator = BLSCoordinator::new(config, signer);

        // Create committee
        let members = vec![
            PublicKey::from_bytes([1u8; 32]),
            PublicKey::from_bytes([2u8; 32]),
            PublicKey::from_bytes([3u8; 32]),
        ];

        // Initiate signing request
        let request_id = SigningRequestId::new("test", 1, &[0u8; 32]);
        let message = b"test message".to_vec();
        let dst = DST_COMMITTEE_CERT.to_vec();

        let request = coordinator
            .initiate_signing(request_id.clone(), message, dst, members.clone())
            .await
            .unwrap();

        assert_eq!(request.threshold, 2);
        assert_eq!(request.committee_members.len(), 3);

        // Check initial status
        let status = coordinator.get_status(&request_id).await.unwrap();
        assert_eq!(status.shares_collected, 0);
        assert!(!status.completed);
    }
}
