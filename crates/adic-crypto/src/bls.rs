//! BLS Threshold Signatures for ADIC
//!
//! Implements threshold BLS signatures as specified in PoUW III §10.3
//! for GovernanceReceipts and PoUWReceipts.
//!
//! # Domain Separation (PoUW III §10.3)
//!
//! All signatures MUST use domain-separated tags (DSTs):
//! - ADIC-GOV-PROP-v1: Governance proposals
//! - ADIC-GOV-VOTE-v1: Governance votes
//! - ADIC-GOV-R-v1: Governance receipts
//! - ADIC-PoUW-RECEIPT-v1: PoUW task receipts
//! - ADIC-PoUW-HOOK-v1: PoUW task hooks
//!
//! # Threshold Scheme
//!
//! Uses (t, n) threshold signatures where:
//! - n = total committee members
//! - t = threshold (minimum signatures needed)
//! - Per PoUW III Table 1: t_BLS ∈ [⌈m/2⌉, m]
//! - Default: t = ⌈2m/3⌉ (Byzantine fault tolerance)

use serde::{Deserialize, Serialize};
use thiserror::Error;
use threshold_crypto::{self as tc, SecretKeyShare, PublicKeySet, SignatureShare, Signature};

/// Domain Separation Tags per PoUW III §10.3
pub mod dst {
    pub const GOV_PROPOSAL: &[u8] = b"ADIC-GOV-PROP-v1";
    pub const GOV_VOTE: &[u8] = b"ADIC-GOV-VOTE-v1";
    pub const GOV_RECEIPT: &[u8] = b"ADIC-GOV-R-v1";
    pub const POUW_RECEIPT: &[u8] = b"ADIC-PoUW-RECEIPT-v1";
    pub const POUW_HOOK: &[u8] = b"ADIC-PoUW-HOOK-v1";
    pub const POUW_CLAIM: &[u8] = b"ADIC-PoUW-CLAIM-v1";
    pub const POUW_RESULT: &[u8] = b"ADIC-PoUW-RESULT-v1";
    pub const POUW_VERDICT: &[u8] = b"ADIC-PoUW-VERDICT-v1";
    pub const POUW_COMMITTEE: &[u8] = b"ADIC-PoUW-COMMITTEE-v1";
}

/// BLS threshold cryptography errors
#[derive(Debug, Error)]
pub enum BLSError {
    #[error("Invalid threshold: t={t} must be <= n={n}")]
    InvalidThreshold { t: usize, n: usize },

    #[error("Insufficient signature shares: got {got}, need {need}")]
    InsufficientShares { got: usize, need: usize },

    #[error("Signature verification failed")]
    VerificationFailed,

    #[error("Invalid signature share from participant {participant}")]
    InvalidShare { participant: usize },

    #[error("Duplicate share from participant {participant}")]
    DuplicateShare { participant: usize },

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Threshold crypto error: {0}")]
    ThresholdCryptoError(String),

    #[error("Hex decode error: {0}")]
    HexError(#[from] hex::FromHexError),
}

pub type Result<T> = std::result::Result<T, BLSError>;

/// BLS Signature (aggregated or individual)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BLSSignature(String); // Hex-encoded

impl BLSSignature {
    pub fn from_hex(hex: String) -> Self {
        Self(hex)
    }

    pub fn as_hex(&self) -> &str {
        &self.0
    }

    /// Create from threshold_crypto Signature
    pub fn from_tc_signature(sig: &Signature) -> Self {
        let bytes = bincode::serialize(sig).expect("Signature serialization failed");
        Self(hex::encode(bytes))
    }

    /// Convert to threshold_crypto Signature
    pub fn to_tc_signature(&self) -> Result<Signature> {
        let bytes = hex::decode(&self.0)?;
        bincode::deserialize(&bytes).map_err(|e| {
            BLSError::SerializationError(format!("Failed to deserialize signature: {}", e))
        })
    }
}

/// BLS Signature Share (from single participant)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BLSSignatureShare {
    pub participant_id: usize,
    signature_hex: String,
}

impl BLSSignatureShare {
    pub fn new(participant_id: usize, sig_share: &SignatureShare) -> Result<Self> {
        let bytes = bincode::serialize(sig_share).expect("SignatureShare serialization failed");
        Ok(Self {
            participant_id,
            signature_hex: hex::encode(bytes),
        })
    }

    pub fn to_tc_signature_share(&self) -> Result<SignatureShare> {
        let bytes = hex::decode(&self.signature_hex)?;
        bincode::deserialize(&bytes).map_err(|e| {
            BLSError::SerializationError(format!("Failed to deserialize signature share: {}", e))
        })
    }
}

/// Threshold configuration (t, n)
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ThresholdConfig {
    pub total_participants: usize,
    pub threshold: usize,
}

impl ThresholdConfig {
    /// Create new threshold config
    ///
    /// Validates per PoUW III Table 1: t ∈ [⌈m/2⌉, m]
    pub fn new(total: usize, threshold: usize) -> Result<Self> {
        let min_threshold = (total + 1) / 2; // ⌈m/2⌉

        if threshold > total {
            return Err(BLSError::InvalidThreshold { t: threshold, n: total });
        }

        if threshold < min_threshold {
            return Err(BLSError::InvalidThreshold { t: threshold, n: total });
        }

        Ok(Self {
            total_participants: total,
            threshold,
        })
    }

    /// Create config with Byzantine fault tolerance
    ///
    /// Sets t = ⌈2m/3⌉ for optimal BFT
    pub fn with_bft_threshold(total: usize) -> Result<Self> {
        let threshold = (2 * total + 2) / 3; // ⌈2m/3⌉
        Self::new(total, threshold)
    }
}

/// BLS Threshold Signer
///
/// Handles signing, aggregation, and verification of threshold BLS signatures
pub struct BLSThresholdSigner {
    config: ThresholdConfig,
}

impl BLSThresholdSigner {
    pub fn new(config: ThresholdConfig) -> Self {
        Self { config }
    }

    /// Sign a message share with a secret key share
    ///
    /// # Domain Separation
    /// Message format: DST || message
    pub fn sign_share(
        &self,
        secret_key_share: &SecretKeyShare,
        participant_id: usize,
        message: &[u8],
        dst: &[u8],
    ) -> Result<BLSSignatureShare> {
        // Domain-separated message: DST || message
        let mut prefixed_message = Vec::with_capacity(dst.len() + message.len());
        prefixed_message.extend_from_slice(dst);
        prefixed_message.extend_from_slice(message);

        let sig_share = secret_key_share.sign(&prefixed_message);
        BLSSignatureShare::new(participant_id, &sig_share)
    }

    /// Verify a single signature share
    pub fn verify_share(
        &self,
        public_key_set: &PublicKeySet,
        message: &[u8],
        dst: &[u8],
        share: &BLSSignatureShare,
    ) -> Result<bool> {
        // Domain-separated message
        let mut prefixed_message = Vec::with_capacity(dst.len() + message.len());
        prefixed_message.extend_from_slice(dst);
        prefixed_message.extend_from_slice(message);

        // Get public key share for this participant
        let pk_share = public_key_set.public_key_share(share.participant_id);

        // Deserialize signature share
        let sig_share = share.to_tc_signature_share()?;

        // Verify share using participant's public key share
        let is_valid = pk_share.verify(&sig_share, &prefixed_message);

        Ok(is_valid)
    }

    /// Aggregate signature shares into threshold signature
    ///
    /// Performs Lagrange interpolation in the exponent to combine shares.
    /// Returns error if insufficient shares or duplicate indices.
    pub fn aggregate_shares(
        &self,
        public_key_set: &PublicKeySet,
        shares: &[BLSSignatureShare],
    ) -> Result<BLSSignature> {
        if shares.len() < self.config.threshold {
            return Err(BLSError::InsufficientShares {
                got: shares.len(),
                need: self.config.threshold,
            });
        }

        // Deserialize shares and create BTreeMap for combine_signatures
        use std::collections::BTreeMap;
        let mut shares_map: BTreeMap<usize, SignatureShare> = BTreeMap::new();

        for share_wrapper in shares {
            // Check for duplicates
            if shares_map.contains_key(&share_wrapper.participant_id) {
                return Err(BLSError::DuplicateShare {
                    participant: share_wrapper.participant_id,
                });
            }

            // Deserialize the signature share
            let sig_share = share_wrapper.to_tc_signature_share()?;
            shares_map.insert(share_wrapper.participant_id, sig_share);
        }

        // Combine signatures using Lagrange interpolation
        let combined_sig = public_key_set
            .combine_signatures(&shares_map)
            .map_err(|e| {
                BLSError::ThresholdCryptoError(format!("Failed to combine signatures: {:?}", e))
            })?;

        Ok(BLSSignature::from_tc_signature(&combined_sig))
    }

    /// Verify threshold signature
    ///
    /// Uses real BLS pairing-based verification:
    /// e(sig, G2) == e(H(m), pk_set.public_key())
    pub fn verify(
        &self,
        public_key_set: &PublicKeySet,
        message: &[u8],
        dst: &[u8],
        signature: &BLSSignature,
    ) -> Result<bool> {
        // Domain-separated message: DST || message
        let mut prefixed_message = Vec::with_capacity(dst.len() + message.len());
        prefixed_message.extend_from_slice(dst);
        prefixed_message.extend_from_slice(message);

        // Deserialize signature
        let sig = signature.to_tc_signature()?;

        // Verify using public key set's master public key
        let is_valid = public_key_set.public_key().verify(&sig, &prefixed_message);

        Ok(is_valid)
    }
}

/// Generate threshold keys for testing/setup
///
/// Returns (PublicKeySet, Vec<SecretKeyShare>)
pub fn generate_threshold_keys(
    total: usize,
    threshold: usize,
) -> Result<(PublicKeySet, Vec<SecretKeyShare>)> {
    let config = ThresholdConfig::new(total, threshold)?;

    // Use threshold_crypto's random generation
    // Note: threshold_crypto uses threshold-1 internally
    // Use thread_rng from rand 0.7 (compatible with threshold_crypto)
    let mut rng = rand_07::thread_rng();
    let sk_set = tc::SecretKeySet::random(config.threshold - 1, &mut rng);
    let pk_set = sk_set.public_keys();

    let shares: Vec<SecretKeyShare> = (0..total)
        .map(|i| sk_set.secret_key_share(i))
        .collect();

    Ok((pk_set, shares))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_threshold_config() {
        // Valid: 2-of-3
        let config = ThresholdConfig::new(3, 2).unwrap();
        assert_eq!(config.total_participants, 3);
        assert_eq!(config.threshold, 2);

        // Invalid: threshold > total
        assert!(ThresholdConfig::new(3, 4).is_err());

        // Invalid: threshold < ⌈m/2⌉
        assert!(ThresholdConfig::new(5, 2).is_err());
    }

    #[test]
    fn test_bft_threshold() {
        // 64 participants -> threshold = ⌈2*64/3⌉ = 43
        let config = ThresholdConfig::with_bft_threshold(64).unwrap();
        assert_eq!(config.threshold, 43);

        // 10 participants -> threshold = ⌈20/3⌉ = 7
        let config = ThresholdConfig::with_bft_threshold(10).unwrap();
        assert_eq!(config.threshold, 7);
    }

    #[test]
    fn test_threshold_key_generation() {
        let (pk_set, shares) = generate_threshold_keys(5, 3).unwrap();

        // Should generate correct number of shares
        assert_eq!(shares.len(), 5);

        // Public key set should be valid
        assert!(pk_set.public_key_share(0).to_bytes().len() > 0);
    }

    #[test]
    fn test_sign_and_aggregate() {
        let (pk_set, shares) = generate_threshold_keys(5, 3).unwrap();
        let signer = BLSThresholdSigner::new(ThresholdConfig::new(5, 3).unwrap());

        let message = b"test message";
        let dst = dst::GOV_RECEIPT;

        // Sign with first 3 participants
        let sig_shares: Vec<BLSSignatureShare> = shares[0..3]
            .iter()
            .enumerate()
            .map(|(i, share)| signer.sign_share(share, i, message, dst).unwrap())
            .collect();

        // Aggregate
        let agg_sig = signer.aggregate_shares(&pk_set, &sig_shares).unwrap();

        // Verify
        assert!(signer.verify(&pk_set, message, dst, &agg_sig).unwrap());
    }

    #[test]
    fn test_domain_separation() {
        let (_pk_set, shares) = generate_threshold_keys(3, 2).unwrap();
        let signer = BLSThresholdSigner::new(ThresholdConfig::new(3, 2).unwrap());

        let message = b"test";

        // Same message, different DSTs should produce different signatures
        let sig1 = signer.sign_share(&shares[0], 0, message, dst::GOV_RECEIPT).unwrap();
        let sig2 = signer.sign_share(&shares[0], 0, message, dst::POUW_RECEIPT).unwrap();

        // Signatures should differ due to domain separation
        assert_ne!(sig1.signature_hex, sig2.signature_hex);
    }

    #[test]
    fn test_insufficient_shares() {
        let (pk_set, shares) = generate_threshold_keys(5, 3).unwrap();
        let signer = BLSThresholdSigner::new(ThresholdConfig::new(5, 3).unwrap());

        let message = b"test";

        // Only 2 shares when threshold is 3
        let sig_shares: Vec<BLSSignatureShare> = shares[0..2]
            .iter()
            .enumerate()
            .map(|(i, share)| signer.sign_share(share, i, message, dst::GOV_RECEIPT).unwrap())
            .collect();

        // Should fail with insufficient shares
        assert!(signer.aggregate_shares(&pk_set, &sig_shares).is_err());
    }

    #[test]
    fn test_signature_verification_rejects_wrong_signature() {
        let (pk_set, shares) = generate_threshold_keys(5, 3).unwrap();
        let signer = BLSThresholdSigner::new(ThresholdConfig::new(5, 3).unwrap());

        let message = b"correct message";
        let wrong_message = b"wrong message";
        let dst = dst::GOV_RECEIPT;

        // Sign correct message
        let sig_shares: Vec<BLSSignatureShare> = shares[0..3]
            .iter()
            .enumerate()
            .map(|(i, share)| signer.sign_share(share, i, message, dst).unwrap())
            .collect();

        let agg_sig = signer.aggregate_shares(&pk_set, &sig_shares).unwrap();

        // Verify with correct message should succeed
        assert!(signer.verify(&pk_set, message, dst, &agg_sig).unwrap());

        // Verify with wrong message should fail
        assert!(!signer.verify(&pk_set, wrong_message, dst, &agg_sig).unwrap());
    }

    #[test]
    fn test_signature_verification_rejects_wrong_dst() {
        let (pk_set, shares) = generate_threshold_keys(5, 3).unwrap();
        let signer = BLSThresholdSigner::new(ThresholdConfig::new(5, 3).unwrap());

        let message = b"test message";
        let correct_dst = dst::GOV_RECEIPT;
        let wrong_dst = dst::POUW_RECEIPT;

        // Sign with correct DST
        let sig_shares: Vec<BLSSignatureShare> = shares[0..3]
            .iter()
            .enumerate()
            .map(|(i, share)| signer.sign_share(share, i, message, correct_dst).unwrap())
            .collect();

        let agg_sig = signer.aggregate_shares(&pk_set, &sig_shares).unwrap();

        // Verify with correct DST should succeed
        assert!(signer.verify(&pk_set, message, correct_dst, &agg_sig).unwrap());

        // Verify with wrong DST should fail
        assert!(!signer.verify(&pk_set, message, wrong_dst, &agg_sig).unwrap());
    }

    #[test]
    fn test_duplicate_shares_rejected() {
        let (pk_set, shares) = generate_threshold_keys(5, 3).unwrap();
        let signer = BLSThresholdSigner::new(ThresholdConfig::new(5, 3).unwrap());

        let message = b"test";
        let dst = dst::GOV_RECEIPT;

        // Create duplicate share from participant 0
        let share1 = signer.sign_share(&shares[0], 0, message, dst).unwrap();
        let share2 = signer.sign_share(&shares[1], 1, message, dst).unwrap();
        let share3_dup = signer.sign_share(&shares[0], 0, message, dst).unwrap();

        let sig_shares = vec![share1, share2, share3_dup];

        // Should fail with duplicate share error
        let result = signer.aggregate_shares(&pk_set, &sig_shares);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BLSError::DuplicateShare { participant: 0 }));
    }

    #[test]
    fn test_verify_individual_share() {
        let (pk_set, shares) = generate_threshold_keys(5, 3).unwrap();
        let signer = BLSThresholdSigner::new(ThresholdConfig::new(5, 3).unwrap());

        let message = b"test message";
        let wrong_message = b"wrong message";
        let dst = dst::GOV_RECEIPT;

        // Create a valid share
        let share = signer.sign_share(&shares[0], 0, message, dst).unwrap();

        // Verify with correct message should succeed
        assert!(signer.verify_share(&pk_set, message, dst, &share).unwrap());

        // Verify with wrong message should fail
        assert!(!signer.verify_share(&pk_set, wrong_message, dst, &share).unwrap());
    }
}
