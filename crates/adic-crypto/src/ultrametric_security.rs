//! Ultrametric security enforcement for ADIC-DAG
//!
//! Implements p-adic ultrametric structure for consensus security as specified in the whitepaper.
//! Enforces multi-axis diversity via p-adic ball membership and distance calculations.

use adic_math::{ball_id, count_distinct_balls, vp_diff};
use adic_types::{AdicError, AdicFeatures, AdicMessage, AdicParams, AxisId, QpDigits, Result};
use blake3::Hasher;
use std::collections::HashSet;

/// Ultrametric security validator for enforcing p-adic constraints
pub struct UltrametricValidator {
    params: AdicParams,
}

impl UltrametricValidator {
    /// Create a new ultrametric validator with system parameters
    pub fn new(params: AdicParams) -> Self {
        Self { params }
    }

    /// Verify that a message satisfies ultrametric security constraints (C1)
    /// Each parent must be in the correct p-adic ball for each axis
    pub fn verify_c1_ultrametric(
        &self,
        message: &AdicMessage,
        parent_features: &[Vec<QpDigits>],
    ) -> Result<bool> {
        // For each axis, verify C1: vp(φ_j(x) - φ_j(a_j)) ≥ ρ_j
        for (axis_idx, &radius) in self.params.rho.iter().enumerate() {
            let axis_id = AxisId(axis_idx as u32);

            let msg_phi = message
                .features
                .get_axis(axis_id)
                .ok_or(AdicError::InvalidParameter(format!(
                    "Missing axis {} in message",
                    axis_idx
                )))?;

            // Check parent at index axis_idx (diagonal parent for this axis)
            if axis_idx >= parent_features.len() {
                return Err(AdicError::InvalidParameter(
                    "Not enough parents".to_string(),
                ));
            }

            let parent_phi =
                parent_features[axis_idx]
                    .get(axis_idx)
                    .ok_or(AdicError::InvalidParameter(format!(
                        "Parent {} missing axis {}",
                        axis_idx, axis_idx
                    )))?;

            let valuation = vp_diff(&msg_phi.qp_digits, parent_phi);

            if valuation < radius {
                return Ok(false); // C1 violation
            }
        }

        Ok(true)
    }

    /// Verify C2: Multi-axis diversity constraint
    /// Each axis must have parents from q distinct p-adic balls
    pub fn verify_c2_diversity(
        &self,
        parent_features: &[Vec<QpDigits>],
    ) -> Result<(bool, Vec<usize>)> {
        let mut distinct_counts = Vec::new();

        for (axis_idx, &radius) in self.params.rho.iter().enumerate() {
            let axis_features: Vec<QpDigits> = parent_features
                .iter()
                .filter_map(|pf| pf.get(axis_idx).cloned())
                .collect();

            if axis_features.len() != parent_features.len() {
                return Err(AdicError::InvalidParameter(format!(
                    "Not all parents have axis {}",
                    axis_idx
                )));
            }

            let distinct_count = count_distinct_balls(&axis_features, radius as usize);
            distinct_counts.push(distinct_count);

            if distinct_count < self.params.q as usize {
                return Ok((false, distinct_counts)); // C2 violation
            }
        }

        Ok((true, distinct_counts))
    }

    /// Calculate p-adic ball identifier for a feature vector
    pub fn compute_ball_id(&self, features: &AdicFeatures, axis: AxisId, radius: usize) -> Vec<u8> {
        features
            .get_axis(axis)
            .map(|phi| ball_id(&phi.qp_digits, radius))
            .unwrap_or_default()
    }

    /// Verify that two features are in distinct p-adic balls
    pub fn are_in_distinct_balls(
        &self,
        features1: &AdicFeatures,
        features2: &AdicFeatures,
        axis: AxisId,
        radius: usize,
    ) -> bool {
        let ball1 = self.compute_ball_id(features1, axis, radius);
        let ball2 = self.compute_ball_id(features2, axis, radius);
        ball1 != ball2
    }

    /// Compute ultrametric proximity score between message and parent
    /// Higher score means closer in p-adic metric
    pub fn proximity_score(&self, message: &AdicMessage, parent_features: &[QpDigits]) -> f64 {
        let mut score = 1.0;

        for (axis_idx, &radius) in self.params.rho.iter().enumerate() {
            let axis_id = AxisId(axis_idx as u32);

            if let Some(msg_phi) = message.features.get_axis(axis_id) {
                if let Some(parent_phi) = parent_features.get(axis_idx) {
                    let valuation = vp_diff(&msg_phi.qp_digits, parent_phi) as i32;
                    let radius_i32 = radius as i32;

                    // Exponential decay based on ultrametric distance
                    let factor = (self.params.p as f64).powi(radius_i32 - valuation - 1);
                    score *= 1.0 + factor;
                }
            }
        }

        score
    }

    /// Generate cryptographic proof of ball membership
    pub fn generate_ball_proof(
        &self,
        features: &AdicFeatures,
        axis: AxisId,
        radius: usize,
    ) -> BallMembershipProof {
        let ball_id = self.compute_ball_id(features, axis, radius);

        // Create proof hash: H(features || axis || radius || ball_id)
        let mut hasher = Hasher::new();

        // Hash feature data
        if let Some(phi) = features.get_axis(axis) {
            hasher.update(&phi.qp_digits.to_bytes());
        }

        hasher.update(&axis.0.to_le_bytes());
        hasher.update(&(radius as u32).to_le_bytes());
        hasher.update(&ball_id);

        let proof_hash = hasher.finalize();

        BallMembershipProof {
            axis,
            radius,
            ball_id,
            proof: proof_hash.as_bytes().to_vec(),
        }
    }

    /// Verify a ball membership proof
    pub fn verify_ball_proof(&self, features: &AdicFeatures, proof: &BallMembershipProof) -> bool {
        // Recompute the proof and compare
        let computed_proof = self.generate_ball_proof(features, proof.axis, proof.radius);
        computed_proof.proof == proof.proof && computed_proof.ball_id == proof.ball_id
    }

    /// Check if a set of features covers diverse p-adic neighborhoods
    pub fn verify_neighborhood_coverage(
        &self,
        feature_sets: &[AdicFeatures],
        min_distinct_per_axis: usize,
    ) -> bool {
        for (axis_idx, &radius) in self.params.rho.iter().enumerate() {
            let axis_id = AxisId(axis_idx as u32);

            let mut unique_balls = HashSet::new();
            for features in feature_sets {
                let ball_id = self.compute_ball_id(features, axis_id, radius as usize);
                unique_balls.insert(ball_id);
            }

            if unique_balls.len() < min_distinct_per_axis {
                return false;
            }
        }

        true
    }

    /// Calculate the ultrametric security score for admissibility
    /// This implements S(x;A) from Section 7.1 of the whitepaper
    pub fn compute_security_score(
        &self,
        message: &AdicMessage,
        parent_features: &[Vec<QpDigits>],
    ) -> f64 {
        let mut total_score = 0.0;

        for (axis_idx, &radius) in self.params.rho.iter().enumerate() {
            let axis_id = AxisId(axis_idx as u32);

            if let Some(msg_phi) = message.features.get_axis(axis_id) {
                let mut min_term = f64::INFINITY;

                // Find minimum ultrametric term for this axis
                for parent_axis_features in parent_features {
                    if let Some(parent_phi) = parent_axis_features.get(axis_idx) {
                        let valuation = vp_diff(&msg_phi.qp_digits, parent_phi) as i32;
                        let max_diff = (radius as i32 - valuation).max(0);
                        let term = (self.params.p as f64).powi(-max_diff);

                        if term < min_term {
                            min_term = term;
                        }
                    }
                }

                if min_term != f64::INFINITY {
                    total_score += min_term;
                }
            }
        }

        total_score
    }
}

/// Proof of membership in a p-adic ball
#[derive(Debug, Clone, PartialEq)]
pub struct BallMembershipProof {
    pub axis: AxisId,
    pub radius: usize,
    pub ball_id: Vec<u8>,
    pub proof: Vec<u8>,
}

/// Ultrametric security attestation for finality
pub struct UltrametricAttestation {
    /// Message being attested
    pub message_id: Vec<u8>,
    /// Ball proofs for each axis
    pub ball_proofs: Vec<BallMembershipProof>,
    /// Security score
    pub security_score: f64,
    /// Signature over the attestation
    pub signature: Vec<u8>,
}

impl UltrametricAttestation {
    /// Create a new attestation
    pub fn new(
        message_id: Vec<u8>,
        ball_proofs: Vec<BallMembershipProof>,
        security_score: f64,
    ) -> Self {
        Self {
            message_id,
            ball_proofs,
            security_score,
            signature: vec![], // To be filled by signing
        }
    }

    /// Compute hash for signing
    pub fn compute_hash(&self) -> Vec<u8> {
        let mut hasher = Hasher::new();
        hasher.update(&self.message_id);

        for proof in &self.ball_proofs {
            hasher.update(&proof.axis.0.to_le_bytes());
            hasher.update(&proof.ball_id);
            hasher.update(&proof.proof);
        }

        hasher.update(&self.security_score.to_le_bytes());
        hasher.finalize().as_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::{AdicMeta, AxisPhi, MessageId, PublicKey};
    use chrono::Utc;

    fn create_test_params() -> AdicParams {
        AdicParams {
            p: 3,
            d: 3,
            rho: vec![2, 2, 1],
            q: 3,
            ..Default::default()
        }
    }

    fn create_test_message() -> AdicMessage {
        let features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(10, 3, 5)),
            AxisPhi::new(1, QpDigits::from_u64(20, 3, 5)),
            AxisPhi::new(2, QpDigits::from_u64(30, 3, 5)),
        ]);

        AdicMessage::new(
            vec![
                MessageId::new(b"p1"),
                MessageId::new(b"p2"),
                MessageId::new(b"p3"),
                MessageId::new(b"p4"),
            ],
            features,
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![],
        )
    }

    #[test]
    fn test_c1_ultrametric_verification() {
        let params = create_test_params();
        let validator = UltrametricValidator::new(params);
        let message = create_test_message();

        // Parent features that satisfy C1
        let parent_features = vec![
            vec![
                QpDigits::from_u64(19, 3, 5), // vp(10-19) = vp(18) = 2 >= ρ_0=2
                QpDigits::from_u64(21, 3, 5),
                QpDigits::from_u64(31, 3, 5),
            ],
            vec![
                QpDigits::from_u64(11, 3, 5),
                QpDigits::from_u64(29, 3, 5), // vp(20-29) = vp(18) = 2 >= ρ_1=2
                QpDigits::from_u64(32, 3, 5),
            ],
            vec![
                QpDigits::from_u64(12, 3, 5),
                QpDigits::from_u64(22, 3, 5),
                QpDigits::from_u64(33, 3, 5), // vp(30-33) = vp(3) = 1 >= ρ_2=1
            ],
        ];

        let result = validator
            .verify_c1_ultrametric(&message, &parent_features)
            .unwrap();
        assert!(result);
    }

    #[test]
    fn test_c2_diversity_verification() {
        let params = create_test_params();
        let validator = UltrametricValidator::new(params);

        // Parents with diverse balls
        let parent_features = vec![
            vec![
                QpDigits::from_u64(0, 3, 5),
                QpDigits::from_u64(0, 3, 5),
                QpDigits::from_u64(0, 3, 5),
            ],
            vec![
                QpDigits::from_u64(9, 3, 5),
                QpDigits::from_u64(3, 3, 5),
                QpDigits::from_u64(1, 3, 5),
            ],
            vec![
                QpDigits::from_u64(3, 3, 5),
                QpDigits::from_u64(9, 3, 5),
                QpDigits::from_u64(2, 3, 5),
            ],
            vec![
                QpDigits::from_u64(1, 3, 5),
                QpDigits::from_u64(1, 3, 5),
                QpDigits::from_u64(3, 3, 5),
            ],
        ];

        let (passed, counts) = validator.verify_c2_diversity(&parent_features).unwrap();
        assert!(passed);
        assert!(counts.iter().all(|&c| c >= 3));
    }

    #[test]
    fn test_ball_membership_proof() {
        let params = create_test_params();
        let validator = UltrametricValidator::new(params);

        let features = AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(42, 3, 5))]);

        let proof = validator.generate_ball_proof(&features, AxisId(0), 2);
        assert!(validator.verify_ball_proof(&features, &proof));

        // Different features should fail verification
        let other_features = AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(43, 3, 5))]);
        assert!(!validator.verify_ball_proof(&other_features, &proof));
    }

    #[test]
    fn test_security_score_computation() {
        let params = create_test_params();
        let d = params.d;
        let validator = UltrametricValidator::new(params);
        let message = create_test_message();

        let parent_features = vec![
            vec![
                QpDigits::from_u64(19, 3, 5),
                QpDigits::from_u64(29, 3, 5),
                QpDigits::from_u64(33, 3, 5),
            ],
            vec![
                QpDigits::from_u64(11, 3, 5),
                QpDigits::from_u64(21, 3, 5),
                QpDigits::from_u64(31, 3, 5),
            ],
        ];

        let score = validator.compute_security_score(&message, &parent_features);
        assert!(score > 0.0);
        assert!(score <= d as f64);
    }
}
