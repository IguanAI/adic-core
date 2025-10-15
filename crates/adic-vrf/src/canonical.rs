use crate::VRFOpen;

/// Canonical randomness for an epoch
#[derive(Debug, Clone)]
pub struct CanonicalRandomness {
    pub epoch: u64,
    pub randomness: [u8; 32],
    pub num_contributors: usize,
}

/// Compute canonical randomness R_k for epoch k
/// Formula: R_k = H(Root_{k-1} || ⊕_{π∈S_k} π)
/// where:
/// - Root_{k-1} is the root hash from previous epoch
/// - S_k is the set of revealed VRF proofs for epoch k
/// - ⊕ is XOR operation
pub fn compute_canonical_randomness(
    epoch: u64,
    root_prev: &[u8; 32],
    reveals: &[VRFOpen],
) -> CanonicalRandomness {
    // XOR all VRF proofs together
    let mut combined_proof = vec![0u8; 32];
    let mut num_contributors = 0;

    for reveal in reveals {
        if reveal.target_epoch == epoch {
            // XOR the VRF proof into combined_proof
            let proof_hash = blake3::hash(&reveal.vrf_proof);
            for (i, byte) in proof_hash.as_bytes().iter().enumerate() {
                if i < combined_proof.len() {
                    combined_proof[i] ^= byte;
                }
            }
            num_contributors += 1;
        }
    }

    // Combine with previous root
    let mut mixing_input = Vec::new();
    mixing_input.extend_from_slice(root_prev);
    mixing_input.extend_from_slice(&combined_proof);

    // Final hash to get canonical randomness
    let randomness = *blake3::hash(&mixing_input).as_bytes();

    CanonicalRandomness {
        epoch,
        randomness,
        num_contributors,
    }
}

/// Compute a challenge from canonical randomness
/// Used for storage proofs, PoUW tasks, etc.
pub fn compute_challenge(
    canonical_randomness: &[u8; 32],
    domain: &str,
    params: &[&[u8]],
) -> [u8; 32] {
    let mut input = Vec::new();

    // Domain separation
    input.extend_from_slice(domain.as_bytes());
    input.push(0); // Separator

    // Canonical randomness
    input.extend_from_slice(canonical_randomness);

    // Additional parameters
    for param in params {
        input.extend_from_slice(param);
    }

    *blake3::hash(&input).as_bytes()
}

/// Compute VRF score for committee selection
/// y_{u,j} := VRF(R_k || "committee" || axis_id=j || axis_ball(u,j))
pub fn compute_vrf_score(
    canonical_randomness: &[u8; 32],
    domain: &str,
    axis_id: usize,
    node_id: &[u8],
    axis_ball: &[u8],
) -> [u8; 32] {
    let mut input = Vec::new();

    // Canonical randomness
    input.extend_from_slice(canonical_randomness);

    // Domain separator
    input.extend_from_slice(domain.as_bytes());
    input.push(0);

    // Axis ID
    input.extend_from_slice(&axis_id.to_le_bytes());

    // Node identifier
    input.extend_from_slice(node_id);

    // Axis ball (p-adic neighborhood)
    input.extend_from_slice(axis_ball);

    *blake3::hash(&input).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VRFOpen;
    use adic_types::{AdicMessage, AdicFeatures, AdicMeta, PublicKey, MessageId};
    use chrono::Utc;

    #[test]
    fn test_canonical_randomness_deterministic() {
        let root = [1u8; 32];
        let msg1 = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![],
        );

        let reveals = vec![
            VRFOpen::new(
                msg1.clone(),
                100,
                MessageId::new(b"commit1"),
                b"proof1".to_vec(),
                PublicKey::from_bytes([1; 32]),
            ),
            VRFOpen::new(
                msg1.clone(),
                100,
                MessageId::new(b"commit2"),
                b"proof2".to_vec(),
                PublicKey::from_bytes([2; 32]),
            ),
        ];

        let r1 = compute_canonical_randomness(100, &root, &reveals);
        let r2 = compute_canonical_randomness(100, &root, &reveals);

        // Should be deterministic
        assert_eq!(r1.randomness, r2.randomness);
        assert_eq!(r1.num_contributors, 2);
    }

    #[test]
    fn test_challenge_computation() {
        let randomness = [42u8; 32];
        let domain = "storage_challenge";
        let deal_id = 123u64;
        let epoch = 456u64;

        let challenge = compute_challenge(
            &randomness,
            domain,
            &[&deal_id.to_le_bytes(), &epoch.to_le_bytes()],
        );

        // Should be deterministic
        let challenge2 = compute_challenge(
            &randomness,
            domain,
            &[&deal_id.to_le_bytes(), &epoch.to_le_bytes()],
        );

        assert_eq!(challenge, challenge2);

        // Different domain should give different result
        let challenge3 = compute_challenge(
            &randomness,
            "different_domain",
            &[&deal_id.to_le_bytes(), &epoch.to_le_bytes()],
        );

        assert_ne!(challenge, challenge3);
    }

    #[test]
    fn test_vrf_score_computation() {
        let randomness = [42u8; 32];
        let node_id = b"node_1";
        let axis_ball = b"ball_123";

        let score1 = compute_vrf_score(&randomness, "committee", 0, node_id, axis_ball);
        let score2 = compute_vrf_score(&randomness, "committee", 0, node_id, axis_ball);

        // Deterministic
        assert_eq!(score1, score2);

        // Different axis should give different score
        let score3 = compute_vrf_score(&randomness, "committee", 1, node_id, axis_ball);
        assert_ne!(score1, score3);
    }
}
