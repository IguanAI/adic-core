//! Consensus cryptography for ADIC-DAG
//!
//! Implements reputation-weighted validation, conflict resolution verification,
//! and finality certificate generation as specified in the whitepaper.

use adic_types::{AdicError, AdicMessage, MessageId, PublicKey, Result, Signature};
use blake3::Hasher;
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Reputation-weighted consensus validator
pub struct ConsensusValidator {
    /// Reputation scores for public keys
    reputations: HashMap<PublicKey, f64>,
    /// Minimum reputation thresholds
    r_min: f64,
    r_sum_min: f64,
}

impl ConsensusValidator {
    pub fn new(r_min: f64, r_sum_min: f64) -> Self {
        Self {
            reputations: HashMap::new(),
            r_min,
            r_sum_min,
        }
    }

    /// Update reputation for a public key
    pub fn update_reputation(&mut self, pubkey: PublicKey, reputation: f64) {
        self.reputations.insert(pubkey, reputation);
    }

    /// Get reputation for a public key
    pub fn get_reputation(&self, pubkey: &PublicKey) -> f64 {
        self.reputations.get(pubkey).copied().unwrap_or(0.0)
    }

    /// Verify reputation-weighted signature with C3 constraint
    pub fn verify_weighted_signature(
        &self,
        message: &AdicMessage,
        signatures: &[(PublicKey, Signature)],
    ) -> Result<bool> {
        // Check C3: minimum individual reputation
        let min_rep = signatures
            .iter()
            .map(|(pk, _)| self.get_reputation(pk))
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(0.0);

        if min_rep < self.r_min {
            return Ok(false);
        }

        // Check C3: sum of reputations
        let sum_rep: f64 = signatures
            .iter()
            .map(|(pk, _)| self.get_reputation(pk))
            .sum();

        if sum_rep < self.r_sum_min {
            return Ok(false);
        }

        // Verify all signatures
        let message_bytes = message.to_bytes();
        for (pubkey, signature) in signatures {
            if !self.verify_single_signature(&message_bytes, pubkey, signature)? {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Verify a single signature
    fn verify_single_signature(
        &self,
        message: &[u8],
        pubkey: &PublicKey,
        signature: &Signature,
    ) -> Result<bool> {
        let verifying_key = VerifyingKey::from_bytes(pubkey.as_bytes())
            .map_err(|_| AdicError::SignatureVerification)?;

        let sig_bytes = signature.as_bytes();
        if sig_bytes.len() != 64 {
            return Ok(false);
        }

        let mut sig_array = [0u8; 64];
        sig_array.copy_from_slice(sig_bytes);
        let dalek_sig = ed25519_dalek::Signature::from_bytes(&sig_array);

        Ok(verifying_key.verify(message, &dalek_sig).is_ok())
    }

    /// Calculate reputation-weighted support for a message
    pub fn calculate_support(&self, supporters: &[PublicKey], depth: u32) -> f64 {
        supporters
            .iter()
            .map(|pk| self.get_reputation(pk) / (1.0 + depth as f64))
            .sum()
    }
}

/// Conflict set resolver with energy verification
#[derive(Debug, Clone, Default)]
pub struct ConflictResolver {
    /// Active conflict sets
    conflict_sets: HashMap<String, ConflictSet>,
}

#[derive(Debug, Clone)]
pub struct ConflictSet {
    pub id: String,
    pub members: HashSet<MessageId>,
    pub supports: HashMap<MessageId, f64>,
}

impl ConflictResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a new conflict set
    pub fn add_conflict_set(&mut self, id: String, members: HashSet<MessageId>) {
        let mut supports = HashMap::new();
        for member in &members {
            supports.insert(*member, 0.0);
        }

        self.conflict_sets.insert(
            id.clone(),
            ConflictSet {
                id,
                members,
                supports,
            },
        );
    }

    /// Update support for a conflict set member
    pub fn update_support(&mut self, conflict_id: &str, message_id: &MessageId, support: f64) {
        if let Some(conflict_set) = self.conflict_sets.get_mut(conflict_id) {
            if conflict_set.members.contains(message_id) {
                conflict_set.supports.insert(*message_id, support);
            }
        }
    }

    /// Calculate energy for conflict resolution (Section 4.1)
    pub fn calculate_energy(&self, conflict_id: &str) -> f64 {
        self.conflict_sets
            .get(conflict_id)
            .map(|cs| {
                // Energy = |Σ sgn(z) * supp(z; C)|
                let weighted_sum: f64 = cs
                    .supports
                    .values()
                    .enumerate()
                    .map(|(i, &supp)| {
                        let sign = if i == 0 { 1.0 } else { -1.0 };
                        sign * supp
                    })
                    .sum();

                weighted_sum.abs()
            })
            .unwrap_or(0.0)
    }

    /// Verify energy descent for conflict resolution
    pub fn verify_energy_descent(&self, conflict_id: &str, old_energy: f64) -> bool {
        let new_energy = self.calculate_energy(conflict_id);
        new_energy < old_energy
    }

    /// Get winner of a conflict set (highest support)
    pub fn get_winner(&self, conflict_id: &str) -> Option<MessageId> {
        self.conflict_sets.get(conflict_id).and_then(|cs| {
            cs.supports
                .iter()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
                .map(|(id, _)| *id)
        })
    }

    /// Generate proof of conflict resolution
    pub fn generate_resolution_proof(
        &self,
        conflict_id: &str,
        signing_key: &SigningKey,
    ) -> Result<ConflictResolutionProof> {
        let conflict_set =
            self.conflict_sets
                .get(conflict_id)
                .ok_or(AdicError::InvalidParameter(
                    "Conflict set not found".to_string(),
                ))?;

        let winner = self
            .get_winner(conflict_id)
            .ok_or(AdicError::InvalidParameter("No winner found".to_string()))?;

        let energy = self.calculate_energy(conflict_id);

        // Create proof hash
        let mut hasher = Hasher::new();
        hasher.update(conflict_id.as_bytes());
        hasher.update(winner.as_bytes());
        hasher.update(&energy.to_le_bytes());

        for (msg_id, support) in &conflict_set.supports {
            hasher.update(msg_id.as_bytes());
            hasher.update(&support.to_le_bytes());
        }

        let proof_hash = hasher.finalize();
        let signature = signing_key.sign(proof_hash.as_bytes());

        Ok(ConflictResolutionProof {
            conflict_id: conflict_id.to_string(),
            winner,
            energy,
            supports: conflict_set.supports.clone(),
            signature: signature.to_bytes().to_vec(),
        })
    }
}

/// Proof of conflict resolution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictResolutionProof {
    pub conflict_id: String,
    pub winner: MessageId,
    pub energy: f64,
    pub supports: HashMap<MessageId, f64>,
    pub signature: Vec<u8>,
}

/// Finality certificate generator
pub struct FinalityCertificate {
    /// Message being certified as final
    pub message_id: MessageId,
    /// k-core attestation
    pub k_core_proof: KCoreProof,
    /// Homology attestation (optional)
    pub homology_proof: Option<HomologyProof>,
    /// Signatures from validators
    pub validator_signatures: Vec<(PublicKey, Signature)>,
    /// Certificate timestamp
    pub timestamp: u64,
}

/// Proof of k-core coverage (Section 4.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KCoreProof {
    /// Size of k-core
    pub k: u32,
    /// Number of distinct p-adic balls per axis
    pub distinct_balls_per_axis: Vec<usize>,
    /// Total reputation in k-core
    pub total_reputation: f64,
    /// Depth of coverage
    pub depth: u32,
    /// Hash of k-core members
    pub members_hash: Vec<u8>,
}

impl KCoreProof {
    pub fn new(
        k: u32,
        distinct_balls: Vec<usize>,
        reputation: f64,
        depth: u32,
        members: &[MessageId],
    ) -> Self {
        let mut hasher = Hasher::new();
        for member in members {
            hasher.update(member.as_bytes());
        }

        Self {
            k,
            distinct_balls_per_axis: distinct_balls,
            total_reputation: reputation,
            depth,
            members_hash: hasher.finalize().as_bytes().to_vec(),
        }
    }

    /// Verify k-core meets finality requirements
    pub fn verify(&self, min_k: u32, min_distinct: usize, min_rep: f64, min_depth: u32) -> bool {
        self.k >= min_k
            && self
                .distinct_balls_per_axis
                .iter()
                .all(|&d| d >= min_distinct)
            && self.total_reputation >= min_rep
            && self.depth >= min_depth
    }
}

/// Proof of persistent homology stabilization (Section 4.2 & Appendix C)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomologyProof {
    /// Dimension of homology group
    pub dimension: usize,
    /// Stabilization window
    pub window: u32,
    /// Bottleneck distance
    pub bottleneck_distance: f64,
    /// Hash of simplicial complex
    pub complex_hash: Vec<u8>,
}

impl HomologyProof {
    pub fn new(dimension: usize, window: u32, distance: f64, complex: &[Vec<MessageId>]) -> Self {
        let mut hasher = Hasher::new();
        for simplex in complex {
            for vertex in simplex {
                hasher.update(vertex.as_bytes());
            }
        }

        Self {
            dimension,
            window,
            bottleneck_distance: distance,
            complex_hash: hasher.finalize().as_bytes().to_vec(),
        }
    }

    /// Verify homology meets stabilization criteria
    pub fn verify(&self, min_window: u32, max_distance: f64) -> bool {
        self.window >= min_window && self.bottleneck_distance <= max_distance
    }
}

/// Certificate validator
pub struct CertificateValidator {
    consensus: ConsensusValidator,
    min_validators: usize,
}

impl CertificateValidator {
    pub fn new(consensus: ConsensusValidator, min_validators: usize) -> Self {
        Self {
            consensus,
            min_validators,
        }
    }

    /// Validate a finality certificate
    pub fn validate_certificate(&self, cert: &FinalityCertificate) -> Result<bool> {
        // Check enough validators signed
        if cert.validator_signatures.len() < self.min_validators {
            return Ok(false);
        }

        // Verify k-core proof
        if !cert.k_core_proof.verify(20, 3, 100.0, 12) {
            return Ok(false);
        }

        // Verify homology proof if present
        if let Some(ref homology) = cert.homology_proof {
            if !homology.verify(5, 0.01) {
                return Ok(false);
            }
        }

        // Verify validator signatures
        let cert_bytes = self.certificate_to_bytes(cert);
        for (pubkey, signature) in &cert.validator_signatures {
            if !self
                .consensus
                .verify_single_signature(&cert_bytes, pubkey, signature)?
            {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn certificate_to_bytes(&self, cert: &FinalityCertificate) -> Vec<u8> {
        let mut hasher = Hasher::new();
        hasher.update(cert.message_id.as_bytes());
        hasher.update(&cert.k_core_proof.members_hash);
        hasher.update(&cert.timestamp.to_le_bytes());

        if let Some(ref homology) = cert.homology_proof {
            hasher.update(&homology.complex_hash);
        }

        hasher.finalize().as_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reputation_weighted_validation() {
        let mut validator = ConsensusValidator::new(1.0, 5.0);

        // Add reputations
        let pk1 = PublicKey::from_bytes([1; 32]);
        let pk2 = PublicKey::from_bytes([2; 32]);
        let pk3 = PublicKey::from_bytes([3; 32]);

        validator.update_reputation(pk1, 2.0);
        validator.update_reputation(pk2, 2.0);
        validator.update_reputation(pk3, 2.0);

        // Calculate support
        let supporters = vec![pk1, pk2, pk3];
        let support = validator.calculate_support(&supporters, 1);
        assert!(support > 0.0);
    }

    #[test]
    fn test_conflict_resolution() {
        let mut resolver = ConflictResolver::new();

        let msg1 = MessageId::new(b"msg1");
        let msg2 = MessageId::new(b"msg2");

        let mut members = HashSet::new();
        members.insert(msg1);
        members.insert(msg2);

        resolver.add_conflict_set("conflict1".to_string(), members);

        // Update supports
        resolver.update_support("conflict1", &msg1, 10.0);
        resolver.update_support("conflict1", &msg2, 5.0);

        // Check energy
        let energy = resolver.calculate_energy("conflict1");
        assert!(energy > 0.0);

        // Check winner
        let winner = resolver.get_winner("conflict1");
        assert_eq!(winner, Some(msg1));
    }

    #[test]
    fn test_k_core_proof() {
        let members = vec![
            MessageId::new(b"m1"),
            MessageId::new(b"m2"),
            MessageId::new(b"m3"),
        ];

        let proof = KCoreProof::new(20, vec![3, 3, 3], 150.0, 15, &members);

        assert!(proof.verify(20, 3, 100.0, 12));
        assert!(!proof.verify(25, 3, 100.0, 12)); // k too low
    }

    #[test]
    fn test_homology_proof() {
        let complex = vec![
            vec![MessageId::new(b"v1"), MessageId::new(b"v2")],
            vec![MessageId::new(b"v2"), MessageId::new(b"v3")],
            vec![MessageId::new(b"v1"), MessageId::new(b"v3")],
        ];

        let proof = HomologyProof::new(1, 10, 0.005, &complex);

        assert!(proof.verify(5, 0.01));
        assert!(!proof.verify(5, 0.001)); // distance too large
    }
}
