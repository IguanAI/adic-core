//! Hybrid Security Manager for ADIC-DAG
//!
//! This module implements a dual-purpose security system:
//! - Standard cryptography (AES, X25519, Ed25519) for transport/networking security
//! - Ultrametric cryptography (p-adic) for consensus validation and admissibility
//!
//! This separation ensures production-grade security for networking while
//! maintaining the innovative p-adic consensus mechanisms from the whitepaper.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
// use libp2p::identity::Keypair; // Not used
use blake3::Hasher;
use libp2p::PeerId;
use tracing::{debug, info, warn};

use adic_crypto::{
    BallMembershipProof,
    FeatureCommitment,
    SecurityParams,
    // Standard crypto for transport security
    StandardCrypto,
    StandardKeyExchange,
    // Ultrametric crypto for consensus validation
    UltrametricValidator,
};
use adic_types::{AdicError, AdicMessage, AdicParams, Result};

/// Hybrid security manager combining standard and ultrametric cryptography
pub struct SecurityManager {
    // Transport Security (Standard Crypto)
    blacklist: Arc<RwLock<HashSet<PeerId>>>,
    rate_limits: Arc<RwLock<HashMap<PeerId, RateLimit>>>,
    puzzle_challenges: Arc<RwLock<HashMap<PeerId, PuzzleChallenge>>>,
    standard_crypto: StandardCrypto,
    key_exchanges: Arc<RwLock<HashMap<PeerId, StandardKeyExchange>>>,
    session_keys: Arc<RwLock<HashMap<PeerId, [u8; 32]>>>,

    // Consensus Security (Ultrametric Crypto)
    ultrametric_validator: Arc<UltrametricValidator>,
    security_params: SecurityParams,
    feature_commitments: Arc<RwLock<HashMap<Vec<u8>, FeatureCommitment>>>,
    ball_proofs: Arc<RwLock<HashMap<Vec<u8>, Vec<BallMembershipProof>>>>,
}

#[derive(Debug, Clone)]
struct RateLimit {
    requests: Vec<Instant>,
    limit: usize,
    window: Duration,
}

#[derive(Debug, Clone)]
struct PuzzleChallenge {
    challenge: Vec<u8>,
    difficulty: u32,
    created_at: Instant,
    solved: bool,
}

impl Default for SecurityManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SecurityManager {
    /// Create a new hybrid security manager
    pub fn new() -> Self {
        let params = SecurityParams::v1_defaults();
        let adic_params = security_params_to_adic_params(&params);

        Self {
            // Transport security components
            blacklist: Arc::new(RwLock::new(HashSet::new())),
            rate_limits: Arc::new(RwLock::new(HashMap::new())),
            puzzle_challenges: Arc::new(RwLock::new(HashMap::new())),
            standard_crypto: StandardCrypto::new(),
            key_exchanges: Arc::new(RwLock::new(HashMap::new())),
            session_keys: Arc::new(RwLock::new(HashMap::new())),

            // Consensus security components
            ultrametric_validator: Arc::new(UltrametricValidator::new(adic_params)),
            security_params: params,
            feature_commitments: Arc::new(RwLock::new(HashMap::new())),
            ball_proofs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create with custom parameters
    pub fn with_params(security_params: SecurityParams) -> Self {
        let adic_params = security_params_to_adic_params(&security_params);

        Self {
            blacklist: Arc::new(RwLock::new(HashSet::new())),
            rate_limits: Arc::new(RwLock::new(HashMap::new())),
            puzzle_challenges: Arc::new(RwLock::new(HashMap::new())),
            standard_crypto: StandardCrypto::new(),
            key_exchanges: Arc::new(RwLock::new(HashMap::new())),
            session_keys: Arc::new(RwLock::new(HashMap::new())),
            ultrametric_validator: Arc::new(UltrametricValidator::new(adic_params)),
            security_params,
            feature_commitments: Arc::new(RwLock::new(HashMap::new())),
            ball_proofs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // ===== TRANSPORT SECURITY (Standard Crypto) =====

    pub async fn is_blacklisted(&self, peer: &PeerId) -> bool {
        let blacklist = self.blacklist.read().await;
        blacklist.contains(peer)
    }

    pub async fn blacklist_peer(&self, peer: PeerId, reason: &str) {
        warn!("Blacklisting peer {}: {}", peer, reason);
        let mut blacklist = self.blacklist.write().await;
        blacklist.insert(peer);
    }

    pub async fn check_rate_limit(&self, peer: &PeerId) -> bool {
        let mut rate_limits = self.rate_limits.write().await;
        let now = Instant::now();

        let rate_limit = rate_limits.entry(*peer).or_insert_with(|| RateLimit {
            requests: Vec::new(),
            limit: 100,                      // 100 requests
            window: Duration::from_secs(60), // per minute
        });

        // Remove old requests outside the window
        rate_limit
            .requests
            .retain(|req| now.duration_since(*req) < rate_limit.window);

        if rate_limit.requests.len() >= rate_limit.limit {
            warn!("Rate limit exceeded for peer {}", peer);
            return false;
        }

        rate_limit.requests.push(now);
        true
    }

    pub async fn create_puzzle_challenge(&self, peer: &PeerId, difficulty: u32) -> Vec<u8> {
        let mut hasher = Hasher::new();
        hasher.update(peer.to_bytes().as_slice());
        hasher.update(&Instant::now().elapsed().as_nanos().to_le_bytes());
        let challenge = hasher.finalize().as_bytes().to_vec();

        let mut challenges = self.puzzle_challenges.write().await;
        challenges.insert(
            *peer,
            PuzzleChallenge {
                challenge: challenge.clone(),
                difficulty,
                created_at: Instant::now(),
                solved: false,
            },
        );

        challenge
    }

    pub async fn verify_puzzle_solution(&self, peer: &PeerId, solution: &[u8]) -> bool {
        let mut challenges = self.puzzle_challenges.write().await;

        if let Some(challenge) = challenges.get_mut(peer) {
            // Check if challenge is not expired (5 minutes)
            if challenge.created_at.elapsed() > Duration::from_secs(300) {
                debug!("Challenge expired for peer {}", peer);
                challenges.remove(peer);
                return false;
            }

            // Verify solution (simplified: check if hash starts with enough zeros)
            let mut hasher = Hasher::new();
            hasher.update(&challenge.challenge);
            hasher.update(solution);
            let hash = hasher.finalize();

            let leading_zeros = hash.as_bytes().iter().take_while(|&&b| b == 0).count() as u32;

            if leading_zeros >= challenge.difficulty {
                challenge.solved = true;
                info!("Puzzle solved by peer {}", peer);
                return true;
            }
        }

        false
    }

    /// Initialize key exchange for secure communication (transport layer)
    pub async fn init_key_exchange(&self, peer: &PeerId) -> [u8; 32] {
        let key_exchange = StandardKeyExchange::new();
        let public_key = key_exchange.public_key();

        let mut exchanges = self.key_exchanges.write().await;
        exchanges.insert(*peer, key_exchange);

        public_key
    }

    /// Complete key exchange and derive session key (transport layer)
    pub async fn complete_key_exchange(
        &self,
        peer: &PeerId,
        peer_public_key: &[u8; 32],
    ) -> Result<()> {
        let mut exchanges = self.key_exchanges.write().await;

        if let Some(exchange) = exchanges.remove(peer) {
            let shared_secret = exchange
                .compute_shared_secret(peer_public_key)
                .map_err(|e| {
                    AdicError::InvalidParameter(format!("Key exchange failed: {:?}", e))
                })?;

            let mut session_keys = self.session_keys.write().await;
            session_keys.insert(*peer, shared_secret);

            Ok(())
        } else {
            Err(AdicError::InvalidParameter(
                "No pending key exchange".to_string(),
            ))
        }
    }

    /// Encrypt data for peer-to-peer communication (transport layer)
    pub async fn encrypt_for_peer(&self, peer: &PeerId, data: &[u8]) -> Result<Vec<u8>> {
        let session_keys = self.session_keys.read().await;

        if let Some(key) = session_keys.get(peer) {
            self.standard_crypto
                .encrypt(data, key)
                .map_err(|e| AdicError::InvalidParameter(format!("Encryption failed: {:?}", e)))
        } else {
            Err(AdicError::InvalidParameter(
                "No session key for peer".to_string(),
            ))
        }
    }

    /// Decrypt data from peer (transport layer)
    pub async fn decrypt_from_peer(&self, peer: &PeerId, encrypted: &[u8]) -> Result<Vec<u8>> {
        let session_keys = self.session_keys.read().await;

        if let Some(key) = session_keys.get(peer) {
            self.standard_crypto
                .decrypt(encrypted, key)
                .map_err(|e| AdicError::InvalidParameter(format!("Decryption failed: {:?}", e)))
        } else {
            Err(AdicError::InvalidParameter(
                "No session key for peer".to_string(),
            ))
        }
    }

    // ===== CONSENSUS SECURITY (Ultrametric Crypto) =====

    /// Verify C1-C3 admissibility constraints using ultrametric validation
    pub async fn verify_admissibility(
        &self,
        message: &AdicMessage,
        parent_features: &[Vec<adic_types::features::QpDigits>],
    ) -> Result<bool> {
        // C1: Ultrametric proximity constraint
        let c1_passed = self
            .ultrametric_validator
            .verify_c1_ultrametric(message, parent_features)?;
        if !c1_passed {
            debug!("Message failed C1 ultrametric constraint");
            return Ok(false);
        }

        // C2: Diversity constraint
        let (c2_passed, distinct_counts) = self
            .ultrametric_validator
            .verify_c2_diversity(parent_features)?;
        if !c2_passed {
            debug!(
                "Message failed C2 diversity constraint: {:?}",
                distinct_counts
            );
            return Ok(false);
        }

        info!("Message passed ultrametric admissibility checks (C1-C2)");
        Ok(true)
    }

    /// Calculate security score S(x;A) from the whitepaper
    pub fn calculate_security_score(
        &self,
        message: &AdicMessage,
        parent_features: &[Vec<adic_types::features::QpDigits>],
    ) -> f64 {
        self.ultrametric_validator
            .compute_security_score(message, parent_features)
    }

    /// Generate ball membership proof for consensus validation
    pub async fn generate_ball_proof(
        &self,
        features: &adic_types::AdicFeatures,
        axis: adic_types::AxisId,
        radius: usize,
    ) -> BallMembershipProof {
        let proof = self
            .ultrametric_validator
            .generate_ball_proof(features, axis, radius);

        // Store proof for later verification
        let mut proofs = self.ball_proofs.write().await;
        let key = features
            .get_axis(axis)
            .map(|phi| phi.qp_digits.to_bytes())
            .unwrap_or_default();

        proofs
            .entry(key)
            .or_insert_with(Vec::new)
            .push(proof.clone());

        proof
    }

    /// Verify ball membership proof
    pub fn verify_ball_proof(
        &self,
        features: &adic_types::AdicFeatures,
        proof: &BallMembershipProof,
    ) -> bool {
        self.ultrametric_validator
            .verify_ball_proof(features, proof)
    }

    /// Create feature commitment to prevent manipulation
    pub async fn commit_features(
        &self,
        features: &adic_types::AdicFeatures,
    ) -> (FeatureCommitment, Vec<u8>) {
        let (commitment, blinding) = FeatureCommitment::commit(features);

        // Store commitment
        let mut commitments = self.feature_commitments.write().await;
        commitments.insert(commitment.commitment.clone(), commitment.clone());

        (commitment, blinding)
    }

    /// Verify feature commitment
    pub async fn verify_feature_commitment(
        &self,
        features: &adic_types::AdicFeatures,
        commitment: &FeatureCommitment,
        blinding: &[u8],
    ) -> bool {
        commitment.verify(features, blinding)
    }

    /// Check if message features satisfy neighborhood coverage for finality
    pub fn verify_neighborhood_coverage(
        &self,
        feature_sets: &[adic_types::AdicFeatures],
        min_distinct_per_axis: usize,
    ) -> bool {
        self.ultrametric_validator
            .verify_neighborhood_coverage(feature_sets, min_distinct_per_axis)
    }

    /// Get security parameters
    pub fn security_params(&self) -> &SecurityParams {
        &self.security_params
    }

    /// Check security level
    pub fn security_level(&self) -> u32 {
        self.security_params.security_level()
    }

    /// Verify message signature (for network layer validation)
    pub fn verify_message_signature(&self, message: &AdicMessage) -> bool {
        // Messages without signatures are considered invalid
        if message.signature.is_empty() {
            return false;
        }

        // Use standard crypto to verify Ed25519 signature
        // This is for transport-layer message authenticity
        adic_crypto::standard_crypto::verify_signature(
            message.proposer_pk.as_bytes(),
            &message.to_bytes(),
            message.signature.as_bytes(),
        )
        .is_ok()
    }
}

// Helper function to convert SecurityParams to AdicParams
fn security_params_to_adic_params(sp: &SecurityParams) -> AdicParams {
    AdicParams {
        p: sp.p,
        d: sp.d,
        rho: sp.rho.clone(),
        q: sp.q,
        k: sp.k_core,
        depth_star: sp.depth_threshold,
        delta: sp.homology_window,
        deposit: sp.deposit,
        r_min: sp.r_min,
        r_sum_min: sp.r_sum_min,
        lambda: sp.mrw_params.0,
        beta: sp.reputation_exponents.1,
        mu: sp.mrw_params.1,
        gamma: sp.gamma,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::identity;

    #[tokio::test]
    async fn test_hybrid_security() {
        let security = SecurityManager::new();

        // Test transport security (standard crypto)
        let keypair = identity::Keypair::generate_ed25519();
        let peer_id = PeerId::from(keypair.public());

        assert!(!security.is_blacklisted(&peer_id).await);
        assert!(security.check_rate_limit(&peer_id).await);

        // Test consensus security (ultrametric)
        assert!(security.security_level() > 0);
        assert_eq!(security.security_params().p, 3); // V1 defaults
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        let security = SecurityManager::new();
        let keypair = identity::Keypair::generate_ed25519();
        let peer_id = PeerId::from(keypair.public());

        // Should allow initial requests
        for _ in 0..50 {
            assert!(security.check_rate_limit(&peer_id).await);
        }
    }

    #[tokio::test]
    async fn test_puzzle_challenge() {
        let security = SecurityManager::new();
        let keypair = identity::Keypair::generate_ed25519();
        let peer_id = PeerId::from(keypair.public());

        let challenge = security.create_puzzle_challenge(&peer_id, 2).await;
        assert!(!challenge.is_empty());

        // Solving puzzle would require finding correct nonce
        // This is a simplified test
        let solution = vec![0u8; 32];
        let _solved = security.verify_puzzle_solution(&peer_id, &solution).await;
    }
}
