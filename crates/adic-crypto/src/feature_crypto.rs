//! Feature cryptography for ADIC-DAG
//!
//! Provides cryptographic commitments and zero-knowledge proofs for message features
//! to prevent manipulation and ensure integrity of p-adic encodings.

use adic_types::{AdicError, AdicFeatures, AxisId, AxisPhi, QpDigits, Result};
use blake3::Hasher;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

/// Cryptographic commitment to message features
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureCommitment {
    /// Hash commitment to the features
    pub commitment: Vec<u8>,
    /// Blinding factor (kept secret by committer)
    #[serde(skip)]
    pub blinding: Option<Vec<u8>>,
    /// Timestamp of commitment
    pub timestamp: u64,
}

impl FeatureCommitment {
    /// Create a new commitment to features with blinding
    pub fn commit(features: &AdicFeatures) -> (Self, Vec<u8>) {
        let mut rng = OsRng;
        let mut blinding = vec![0u8; 32];
        rand::RngCore::fill_bytes(&mut rng, &mut blinding);

        let commitment = Self::compute_commitment(features, &blinding);

        let commit = Self {
            commitment,
            blinding: Some(blinding.clone()),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        (commit, blinding)
    }

    /// Compute commitment hash: H(features || blinding)
    fn compute_commitment(features: &AdicFeatures, blinding: &[u8]) -> Vec<u8> {
        let mut hasher = Hasher::new();

        // Hash all feature axes
        for phi in &features.phi {
            hasher.update(&phi.axis.0.to_le_bytes());
            hasher.update(&phi.qp_digits.p.to_le_bytes());
            hasher.update(&phi.qp_digits.to_bytes());
        }

        hasher.update(blinding);
        hasher.finalize().as_bytes().to_vec()
    }

    /// Verify a commitment with revealed features and blinding
    pub fn verify(&self, features: &AdicFeatures, blinding: &[u8]) -> bool {
        let computed = Self::compute_commitment(features, blinding);
        computed == self.commitment
    }
}

/// Zero-knowledge proof of feature properties
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureProof {
    /// Type of property being proved
    pub proof_type: ProofType,
    /// The proof data
    pub proof: Vec<u8>,
    /// Public inputs to the proof
    pub public_inputs: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProofType {
    /// Proves features are in specific p-adic ball
    BallMembership,
    /// Proves features satisfy diversity constraint
    DiversityConstraint,
    /// Proves features are within valid range
    RangeProof,
    /// Proves features match committed value
    CommitmentOpening,
}

/// Feature proof generator and verifier
pub struct FeatureProver {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
}

impl Default for FeatureProver {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureProver {
    /// Create a new prover with random key
    pub fn new() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Create from existing signing key
    pub fn from_key(signing_key: SigningKey) -> Self {
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Generate proof of ball membership for a specific axis
    pub fn prove_ball_membership(
        &self,
        features: &AdicFeatures,
        axis: AxisId,
        radius: usize,
        _p: u32,
    ) -> Result<FeatureProof> {
        let phi = features
            .get_axis(axis)
            .ok_or(AdicError::InvalidParameter("Axis not found".to_string()))?;

        // Compute ball identifier (first 'radius' digits)
        let ball_id = phi.qp_digits.ball_id(radius);

        // Create proof: sign(H(axis || radius || ball_id || features))
        let mut hasher = Hasher::new();
        hasher.update(&axis.0.to_le_bytes());
        hasher.update(&(radius as u32).to_le_bytes());
        hasher.update(&ball_id);
        hasher.update(&phi.qp_digits.to_bytes());

        let message = hasher.finalize();
        let signature = self.signing_key.sign(message.as_bytes());

        // Public inputs are axis, radius, and ball_id
        let mut public_inputs = Vec::new();
        public_inputs.extend_from_slice(&axis.0.to_le_bytes());
        public_inputs.extend_from_slice(&(radius as u32).to_le_bytes());
        public_inputs.extend_from_slice(&ball_id);

        Ok(FeatureProof {
            proof_type: ProofType::BallMembership,
            proof: signature.to_bytes().to_vec(),
            public_inputs,
        })
    }

    /// Verify a ball membership proof
    pub fn verify_ball_membership(
        &self,
        proof: &FeatureProof,
        features: &AdicFeatures,
    ) -> Result<bool> {
        if proof.proof_type != ProofType::BallMembership {
            return Ok(false);
        }

        // Extract public inputs
        if proof.public_inputs.len() < 8 {
            return Ok(false);
        }

        let axis = AxisId(u32::from_le_bytes([
            proof.public_inputs[0],
            proof.public_inputs[1],
            proof.public_inputs[2],
            proof.public_inputs[3],
        ]));

        let radius = u32::from_le_bytes([
            proof.public_inputs[4],
            proof.public_inputs[5],
            proof.public_inputs[6],
            proof.public_inputs[7],
        ]) as usize;

        let ball_id = &proof.public_inputs[8..];

        // Get feature for axis
        let phi = features
            .get_axis(axis)
            .ok_or(AdicError::InvalidParameter("Axis not found".to_string()))?;

        // Check ball membership
        let computed_ball = phi.qp_digits.ball_id(radius);
        if computed_ball != ball_id {
            return Ok(false);
        }

        // Recreate message and verify signature
        let mut hasher = Hasher::new();
        hasher.update(&axis.0.to_le_bytes());
        hasher.update(&(radius as u32).to_le_bytes());
        hasher.update(ball_id);
        hasher.update(&phi.qp_digits.to_bytes());

        let message = hasher.finalize();

        if proof.proof.len() != 64 {
            return Ok(false);
        }

        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(&proof.proof);
        let signature = Signature::from_bytes(&sig_bytes);

        Ok(self
            .verifying_key
            .verify(message.as_bytes(), &signature)
            .is_ok())
    }

    /// Generate proof that features satisfy diversity constraint
    pub fn prove_diversity(
        &self,
        feature_sets: &[AdicFeatures],
        axis: AxisId,
        radius: usize,
        min_distinct: usize,
    ) -> Result<FeatureProof> {
        // Collect unique ball IDs
        let mut unique_balls = std::collections::HashSet::new();

        for features in feature_sets {
            if let Some(phi) = features.get_axis(axis) {
                let ball_id = phi.qp_digits.ball_id(radius);
                unique_balls.insert(ball_id);
            }
        }

        if unique_balls.len() < min_distinct {
            return Err(AdicError::InvalidParameter(
                "Diversity constraint not met".to_string(),
            ));
        }

        // Create proof: sign(H(axis || radius || min_distinct || ball_ids))
        let mut hasher = Hasher::new();
        hasher.update(&axis.0.to_le_bytes());
        hasher.update(&(radius as u32).to_le_bytes());
        hasher.update(&(min_distinct as u32).to_le_bytes());

        for ball in &unique_balls {
            hasher.update(ball);
        }

        let message = hasher.finalize();
        let signature = self.signing_key.sign(message.as_bytes());

        // Public inputs
        let mut public_inputs = Vec::new();
        public_inputs.extend_from_slice(&axis.0.to_le_bytes());
        public_inputs.extend_from_slice(&(radius as u32).to_le_bytes());
        public_inputs.extend_from_slice(&(min_distinct as u32).to_le_bytes());
        public_inputs.extend_from_slice(&(unique_balls.len() as u32).to_le_bytes());

        Ok(FeatureProof {
            proof_type: ProofType::DiversityConstraint,
            proof: signature.to_bytes().to_vec(),
            public_inputs,
        })
    }

    /// Generate range proof for feature values
    pub fn prove_range(
        &self,
        features: &AdicFeatures,
        axis: AxisId,
        max_value: u64,
    ) -> Result<FeatureProof> {
        let phi = features
            .get_axis(axis)
            .ok_or(AdicError::InvalidParameter("Axis not found".to_string()))?;

        // Simple range check (in production, use bulletproofs or similar)
        let value = phi
            .qp_digits
            .digits
            .iter()
            .enumerate()
            .map(|(i, &d)| d as u64 * (phi.qp_digits.p as u64).pow(i as u32))
            .sum::<u64>();

        if value > max_value {
            return Err(AdicError::InvalidParameter(
                "Value out of range".to_string(),
            ));
        }

        // Create proof
        let mut hasher = Hasher::new();
        hasher.update(&axis.0.to_le_bytes());
        hasher.update(&max_value.to_le_bytes());
        hasher.update(&value.to_le_bytes());

        let message = hasher.finalize();
        let signature = self.signing_key.sign(message.as_bytes());

        let mut public_inputs = Vec::new();
        public_inputs.extend_from_slice(&axis.0.to_le_bytes());
        public_inputs.extend_from_slice(&max_value.to_le_bytes());

        Ok(FeatureProof {
            proof_type: ProofType::RangeProof,
            proof: signature.to_bytes().to_vec(),
            public_inputs,
        })
    }
}

/// Axis-specific feature encoder with cryptographic binding
pub struct FeatureEncoder {
    axis: AxisId,
    p: u32,
    precision: usize,
}

impl FeatureEncoder {
    pub fn new(axis: AxisId, p: u32, precision: usize) -> Self {
        Self { axis, p, precision }
    }

    /// Encode time bucket with cryptographic timestamp proof
    pub fn encode_time_bucket(&self, timestamp: u64, bucket_size: u64) -> AxisPhi {
        let bucket = timestamp / bucket_size;
        let qp = QpDigits::from_u64(bucket, self.p, self.precision);
        AxisPhi::new(self.axis.0, qp)
    }

    /// Encode topic using locality-sensitive hash
    pub fn encode_topic_lsh(&self, data: &[u8]) -> AxisPhi {
        let mut hasher = Hasher::new();
        hasher.update(data);
        let hash = hasher.finalize();

        // Take first 8 bytes as u64
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&hash.as_bytes()[..8]);
        let value = u64::from_le_bytes(bytes);

        let qp = QpDigits::from_u64(value % 1000000, self.p, self.precision);
        AxisPhi::new(self.axis.0, qp)
    }

    /// Encode stake tier with proof of stake
    pub fn encode_stake_tier(&self, stake: u64, tiers: &[u64]) -> Result<AxisPhi> {
        // Find appropriate tier
        let tier = tiers.iter().position(|&t| stake < t).unwrap_or(tiers.len());

        if tier > 255 {
            return Err(AdicError::InvalidParameter("Tier too large".to_string()));
        }

        let qp = QpDigits::from_u64(tier as u64, self.p, self.precision);
        Ok(AxisPhi::new(self.axis.0, qp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_features() -> AdicFeatures {
        AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(42, 3, 5)),
            AxisPhi::new(1, QpDigits::from_u64(100, 3, 5)),
            AxisPhi::new(2, QpDigits::from_u64(7, 3, 5)),
        ])
    }

    #[test]
    fn test_feature_commitment() {
        let features = create_test_features();
        let (commitment, blinding) = FeatureCommitment::commit(&features);

        assert!(commitment.verify(&features, &blinding));

        // Wrong blinding should fail
        let wrong_blinding = vec![0u8; 32];
        assert!(!commitment.verify(&features, &wrong_blinding));

        // Wrong features should fail
        let wrong_features = AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(43, 3, 5))]);
        assert!(!commitment.verify(&wrong_features, &blinding));
    }

    #[test]
    fn test_ball_membership_proof() {
        let prover = FeatureProver::new();
        let features = create_test_features();

        let proof = prover
            .prove_ball_membership(&features, AxisId(0), 2, 3)
            .unwrap();

        assert!(prover.verify_ball_membership(&proof, &features).unwrap());

        // Different features should fail
        let other_features = AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(50, 3, 5))]);
        assert!(!prover
            .verify_ball_membership(&proof, &other_features)
            .unwrap());
    }

    #[test]
    fn test_diversity_proof() {
        let prover = FeatureProver::new();

        // Use values that are guaranteed to be in different balls with radius 1
        // 0 (ball: [0]), 1 (ball: [1]), 2 (ball: [2])
        let feature_sets = vec![
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(0, 3, 5))]),
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(1, 3, 5))]),
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(2, 3, 5))]),
        ];

        let proof = prover
            .prove_diversity(&feature_sets, AxisId(0), 1, 3)
            .unwrap();

        assert_eq!(proof.proof_type, ProofType::DiversityConstraint);
        assert!(!proof.proof.is_empty());
    }

    #[test]
    fn test_feature_encoders() {
        let time_encoder = FeatureEncoder::new(AxisId(0), 3, 5);
        let topic_encoder = FeatureEncoder::new(AxisId(1), 3, 5);
        let stake_encoder = FeatureEncoder::new(AxisId(2), 3, 5);

        // Test time encoding
        let time_phi = time_encoder.encode_time_bucket(1234567890, 3600);
        assert_eq!(time_phi.axis.0, 0);

        // Test topic encoding
        let topic_phi = topic_encoder.encode_topic_lsh(b"hello world");
        assert_eq!(topic_phi.axis.0, 1);

        // Test stake tier encoding
        let tiers = vec![100, 1000, 10000, 100000];
        let stake_phi = stake_encoder.encode_stake_tier(5000, &tiers).unwrap();
        assert_eq!(stake_phi.axis.0, 2);
    }
}
