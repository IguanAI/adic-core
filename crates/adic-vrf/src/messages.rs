use adic_types::{AdicMessage, MessageId, PublicKey};
use serde::{Deserialize, Serialize};

/// VRF Commit message
/// Posted in epoch E_{k-1} for epoch k
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VRFCommit {
    /// Underlying ADIC message
    pub message: AdicMessage,

    /// Target epoch for which randomness is being committed
    pub target_epoch: u64,

    /// Commitment: H(π) where π is VRF proof
    pub commitment: [u8; 32],

    /// Committer's public key
    pub committer: PublicKey,

    /// Committer's reputation at time of commit
    pub committer_reputation: f64,
}

impl VRFCommit {
    pub fn new(
        message: AdicMessage,
        target_epoch: u64,
        commitment: [u8; 32],
        committer: PublicKey,
        committer_reputation: f64,
    ) -> Self {
        Self {
            message,
            target_epoch,
            commitment,
            committer,
            committer_reputation,
        }
    }

    pub fn id(&self) -> MessageId {
        self.message.id
    }

    pub fn verify_commitment(&self, vrf_proof: &[u8]) -> bool {
        let hash = blake3::hash(vrf_proof);
        hash.as_bytes() == &self.commitment
    }
}

/// VRF Open (Reveal) message
/// Posted in early epoch E_k to reveal VRF proof
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VRFOpen {
    /// Underlying ADIC message
    pub message: AdicMessage,

    /// Target epoch
    pub target_epoch: u64,

    /// Reference to the commit message
    pub ref_commit: MessageId,

    /// VRF proof π
    pub vrf_proof: Vec<u8>,

    /// VRF public key for verification
    pub public_key: PublicKey,
}

impl VRFOpen {
    pub fn new(
        message: AdicMessage,
        target_epoch: u64,
        ref_commit: MessageId,
        vrf_proof: Vec<u8>,
        public_key: PublicKey,
    ) -> Self {
        Self {
            message,
            target_epoch,
            ref_commit,
            vrf_proof,
            public_key,
        }
    }

    pub fn id(&self) -> MessageId {
        self.message.id
    }

    pub fn compute_commitment(&self) -> [u8; 32] {
        let hash = blake3::hash(&self.vrf_proof);
        *hash.as_bytes()
    }
}

/// VRF state for an epoch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VRFState {
    pub epoch: u64,
    pub commits: Vec<VRFCommit>,
    pub reveals: Vec<VRFOpen>,
    pub is_finalized: bool,
    pub canonical_randomness: Option<[u8; 32]>,
}

impl VRFState {
    pub fn new(epoch: u64) -> Self {
        Self {
            epoch,
            commits: Vec::new(),
            reveals: Vec::new(),
            is_finalized: false,
            canonical_randomness: None,
        }
    }

    pub fn add_commit(&mut self, commit: VRFCommit) {
        self.commits.push(commit);
    }

    pub fn add_reveal(&mut self, reveal: VRFOpen) {
        self.reveals.push(reveal);
    }

    pub fn finalize(&mut self, randomness: [u8; 32]) {
        self.canonical_randomness = Some(randomness);
        self.is_finalized = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::{AdicFeatures, AdicMeta};
    use chrono::Utc;

    #[test]
    fn test_vrf_commit_verification() {
        let vrf_proof = b"test_vrf_proof".to_vec();
        let commitment = *blake3::hash(&vrf_proof).as_bytes();

        let msg = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![],
        );

        let commit = VRFCommit::new(
            msg,
            100,
            commitment,
            PublicKey::from_bytes([1; 32]),
            50.0,
        );

        assert!(commit.verify_commitment(&vrf_proof));
        assert!(!commit.verify_commitment(b"wrong_proof"));
    }

    #[test]
    fn test_vrf_reveal_commitment() {
        let vrf_proof = b"test_vrf_proof".to_vec();
        let expected_commitment = *blake3::hash(&vrf_proof).as_bytes();

        let msg = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![],
        );

        let reveal = VRFOpen::new(
            msg,
            100,
            MessageId::new(b"commit"),
            vrf_proof,
            PublicKey::from_bytes([1; 32]),
        );

        assert_eq!(reveal.compute_commitment(), expected_commitment);
    }
}
