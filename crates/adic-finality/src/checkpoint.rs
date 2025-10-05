use adic_types::{MessageId, PublicKey, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Checkpoint for anti-entropy synchronization
/// Per ADIC-DAG Yellow Paper §5 - Anti-Entropy Protocol
///
/// Checkpoints capture the finalized state of the DAG at regular intervals,
/// enabling efficient synchronization and divergence detection between nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Checkpoint height (monotonically increasing)
    pub height: u64,

    /// Timestamp when checkpoint was created
    pub timestamp: i64,

    /// Merkle root of all finalized messages up to this checkpoint
    pub messages_merkle_root: [u8; 32],

    /// Merkle root of the finality witness set
    pub witnesses_merkle_root: [u8; 32],

    /// F1 (k-core) finality metadata
    pub f1_metadata: F1Metadata,

    /// F2 (homology) finality metadata
    pub f2_metadata: F2Metadata,

    /// Hash of the previous checkpoint (forms a checkpoint chain)
    pub prev_checkpoint_hash: Option<[u8; 32]>,

    /// Signature over the checkpoint data
    /// Signed by the node creating the checkpoint
    pub signature: Option<Vec<u8>>,

    /// Public key of the signer
    pub signer: Option<PublicKey>,
}

/// F1 (k-core) finality metadata in checkpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct F1Metadata {
    /// Highest k-core value observed
    pub max_k_core: u64,

    /// Number of messages with F1 finality
    pub finalized_count: usize,

    /// Set of message IDs that achieved F1 finality
    pub finalized_messages: Vec<MessageId>,
}

/// F2 (homology) finality metadata in checkpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct F2Metadata {
    /// Number of messages with F2 (homology) finality
    pub stabilized_count: usize,

    /// Set of message IDs that achieved F2 finality
    pub stabilized_messages: Vec<MessageId>,

    /// Current homology rank (if available)
    pub homology_rank: Option<usize>,
}

impl Checkpoint {
    /// Create a new checkpoint
    pub fn new(
        height: u64,
        timestamp: i64,
        messages_merkle_root: [u8; 32],
        witnesses_merkle_root: [u8; 32],
        f1_metadata: F1Metadata,
        f2_metadata: F2Metadata,
        prev_checkpoint_hash: Option<[u8; 32]>,
    ) -> Self {
        Self {
            height,
            timestamp,
            messages_merkle_root,
            witnesses_merkle_root,
            f1_metadata,
            f2_metadata,
            prev_checkpoint_hash,
            signature: None,
            signer: None,
        }
    }

    /// Compute the hash of this checkpoint
    pub fn compute_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();

        hasher.update(self.height.to_le_bytes());
        hasher.update(self.timestamp.to_le_bytes());
        hasher.update(self.messages_merkle_root);
        hasher.update(self.witnesses_merkle_root);

        // Hash F1 metadata
        hasher.update(self.f1_metadata.max_k_core.to_le_bytes());
        hasher.update(self.f1_metadata.finalized_count.to_le_bytes());
        for msg_id in &self.f1_metadata.finalized_messages {
            hasher.update(msg_id.as_bytes());
        }

        // Hash F2 metadata
        hasher.update(self.f2_metadata.stabilized_count.to_le_bytes());
        for msg_id in &self.f2_metadata.stabilized_messages {
            hasher.update(msg_id.as_bytes());
        }

        if let Some(prev_hash) = &self.prev_checkpoint_hash {
            hasher.update(prev_hash);
        }

        hasher.finalize().into()
    }

    /// Sign the checkpoint with a private key
    pub fn sign(&mut self, signer: PublicKey, signature: Vec<u8>) {
        self.signer = Some(signer);
        self.signature = Some(signature);
    }

    /// Verify checkpoint signature
    pub fn verify_signature(&self) -> Result<bool> {
        // Check if signature and signer exist
        let signature = match &self.signature {
            Some(sig) => sig,
            None => return Ok(false),
        };

        let signer = match &self.signer {
            Some(pk) => pk,
            None => return Ok(false),
        };

        // Verify signature length
        if signature.len() != 64 {
            return Ok(false);
        }

        // Create verifying key from signer public key
        use ed25519_dalek::{Signature as DalekSignature, Verifier, VerifyingKey};

        let verifying_key = VerifyingKey::from_bytes(signer.as_bytes())
            .map_err(|_| adic_types::AdicError::SignatureVerification)?;

        // Convert signature bytes to ed25519 signature
        let mut sig_array = [0u8; 64];
        sig_array.copy_from_slice(signature);
        let dalek_sig = DalekSignature::from_bytes(&sig_array);

        // Compute checkpoint hash (this is what was signed)
        let checkpoint_hash = self.compute_hash();

        // Verify signature - checkpoint_hash is [u8; 32], convert to slice
        Ok(verifying_key.verify(&checkpoint_hash, &dalek_sig).is_ok())
    }

    /// Check if this checkpoint diverges from another checkpoint at the same height
    pub fn diverges_from(&self, other: &Checkpoint) -> bool {
        if self.height != other.height {
            return false; // Can't compare different heights
        }

        // Check if merkle roots differ
        self.messages_merkle_root != other.messages_merkle_root
            || self.witnesses_merkle_root != other.witnesses_merkle_root
    }

    /// Get checkpoint size in bytes (approximate)
    pub fn size(&self) -> usize {
        std::mem::size_of::<u64>() // height
            + std::mem::size_of::<i64>() // timestamp
            + 32 // messages_merkle_root
            + 32 // witnesses_merkle_root
            + std::mem::size_of_val(&self.f1_metadata)
            + std::mem::size_of_val(&self.f2_metadata)
            + 32 // prev_checkpoint_hash (Option adds minimal overhead)
            + self.signature.as_ref().map(|s| s.len()).unwrap_or(0)
            + 32 // signer (PublicKey is 32 bytes)
    }
}

impl F1Metadata {
    pub fn new(max_k_core: u64, finalized_messages: Vec<MessageId>) -> Self {
        let finalized_count = finalized_messages.len();
        Self {
            max_k_core,
            finalized_count,
            finalized_messages,
        }
    }

    pub fn empty() -> Self {
        Self {
            max_k_core: 0,
            finalized_count: 0,
            finalized_messages: Vec::new(),
        }
    }
}

impl F2Metadata {
    pub fn new(stabilized_messages: Vec<MessageId>, homology_rank: Option<usize>) -> Self {
        let stabilized_count = stabilized_messages.len();
        Self {
            stabilized_count,
            stabilized_messages,
            homology_rank,
        }
    }

    pub fn empty() -> Self {
        Self {
            stabilized_count: 0,
            stabilized_messages: Vec::new(),
            homology_rank: None,
        }
    }
}

/// Merkle tree builder for checkpoint creation
pub struct MerkleTreeBuilder {
    leaves: Vec<[u8; 32]>,
}

impl MerkleTreeBuilder {
    pub fn new() -> Self {
        Self { leaves: Vec::new() }
    }

    /// Add a message ID to the merkle tree
    pub fn add_message(&mut self, message_id: MessageId) {
        let mut hasher = Sha256::new();
        hasher.update(message_id.as_bytes());
        self.leaves.push(hasher.finalize().into());
    }

    /// Add a witness to the merkle tree
    pub fn add_witness(&mut self, witness_data: &[u8]) {
        let mut hasher = Sha256::new();
        hasher.update(witness_data);
        self.leaves.push(hasher.finalize().into());
    }

    /// Compute the merkle root
    pub fn compute_root(&self) -> [u8; 32] {
        if self.leaves.is_empty() {
            return [0u8; 32];
        }

        let mut current_level = self.leaves.clone();

        while current_level.len() > 1 {
            let mut next_level = Vec::new();

            for chunk in current_level.chunks(2) {
                let mut hasher = Sha256::new();
                hasher.update(chunk[0]);
                if chunk.len() == 2 {
                    hasher.update(chunk[1]);
                } else {
                    // Odd number - hash the single element with itself
                    hasher.update(chunk[0]);
                }
                next_level.push(hasher.finalize().into());
            }

            current_level = next_level;
        }

        current_level[0]
    }
}

impl Default for MerkleTreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_creation() {
        let f1_meta = F1Metadata::empty();
        let f2_meta = F2Metadata::empty();

        let checkpoint = Checkpoint::new(
            1,
            chrono::Utc::now().timestamp(),
            [0u8; 32],
            [1u8; 32],
            f1_meta,
            f2_meta,
            None,
        );

        assert_eq!(checkpoint.height, 1);
        assert!(checkpoint.signature.is_none());
    }

    #[test]
    fn test_checkpoint_hash() {
        let f1_meta = F1Metadata::empty();
        let f2_meta = F2Metadata::empty();

        let checkpoint1 = Checkpoint::new(
            1,
            1234567890,
            [0u8; 32],
            [1u8; 32],
            f1_meta.clone(),
            f2_meta.clone(),
            None,
        );

        let checkpoint2 =
            Checkpoint::new(1, 1234567890, [0u8; 32], [1u8; 32], f1_meta, f2_meta, None);

        // Same data should produce same hash
        assert_eq!(checkpoint1.compute_hash(), checkpoint2.compute_hash());
    }

    #[test]
    fn test_checkpoint_divergence() {
        let f1_meta = F1Metadata::empty();
        let f2_meta = F2Metadata::empty();

        let checkpoint1 = Checkpoint::new(
            1,
            1234567890,
            [0u8; 32],
            [1u8; 32],
            f1_meta.clone(),
            f2_meta.clone(),
            None,
        );

        let checkpoint2 = Checkpoint::new(
            1, 1234567890, [2u8; 32], // Different merkle root
            [1u8; 32], f1_meta, f2_meta, None,
        );

        assert!(checkpoint1.diverges_from(&checkpoint2));
    }

    #[test]
    fn test_merkle_tree_builder() {
        let mut builder = MerkleTreeBuilder::new();

        let msg_id = MessageId::from_bytes([1u8; 32]);
        builder.add_message(msg_id);

        let root = builder.compute_root();
        assert_ne!(root, [0u8; 32]); // Should have a non-zero root
    }

    #[test]
    fn test_merkle_tree_empty() {
        let builder = MerkleTreeBuilder::new();
        let root = builder.compute_root();
        assert_eq!(root, [0u8; 32]); // Empty tree should have zero root
    }

    #[test]
    fn test_merkle_tree_multiple_messages() {
        let mut builder = MerkleTreeBuilder::new();

        for i in 0..5 {
            let mut msg_bytes = [0u8; 32];
            msg_bytes[0] = i;
            builder.add_message(MessageId::from_bytes(msg_bytes));
        }

        let root = builder.compute_root();
        assert_ne!(root, [0u8; 32]);
    }

    #[test]
    fn test_f1_metadata() {
        let msg_id = MessageId::from_bytes([1u8; 32]);
        let f1_meta = F1Metadata::new(5, vec![msg_id]);

        assert_eq!(f1_meta.max_k_core, 5);
        assert_eq!(f1_meta.finalized_count, 1);
        assert_eq!(f1_meta.finalized_messages.len(), 1);
    }

    #[test]
    fn test_f2_metadata() {
        let msg_id = MessageId::from_bytes([2u8; 32]);
        let f2_meta = F2Metadata::new(vec![msg_id], Some(3));

        assert_eq!(f2_meta.stabilized_count, 1);
        assert_eq!(f2_meta.homology_rank, Some(3));
    }

    #[test]
    fn test_checkpoint_signature_verification() {
        use adic_crypto::Keypair;

        // Create a checkpoint
        let f1_meta = F1Metadata::empty();
        let f2_meta = F2Metadata::empty();
        let mut checkpoint = Checkpoint::new(
            1,
            chrono::Utc::now().timestamp(),
            [0u8; 32],
            [1u8; 32],
            f1_meta,
            f2_meta,
            None,
        );

        // Verify unsigned checkpoint returns false
        assert!(!checkpoint.verify_signature().unwrap());

        // Generate keypair and sign the checkpoint
        let keypair = Keypair::generate();
        let checkpoint_hash = checkpoint.compute_hash();
        let signature = keypair.sign(&checkpoint_hash);

        checkpoint.sign(*keypair.public_key(), signature.as_bytes().to_vec());

        // Verify signed checkpoint returns true
        assert!(checkpoint.verify_signature().unwrap());
    }

    #[test]
    fn test_checkpoint_signature_invalid() {
        use adic_crypto::Keypair;

        // Create and sign a checkpoint
        let f1_meta = F1Metadata::empty();
        let f2_meta = F2Metadata::empty();
        let mut checkpoint = Checkpoint::new(
            1,
            chrono::Utc::now().timestamp(),
            [0u8; 32],
            [1u8; 32],
            f1_meta,
            f2_meta,
            None,
        );

        let keypair = Keypair::generate();
        let checkpoint_hash = checkpoint.compute_hash();
        let signature = keypair.sign(&checkpoint_hash);
        checkpoint.sign(*keypair.public_key(), signature.as_bytes().to_vec());

        // Modify checkpoint after signing (invalidates signature)
        checkpoint.height = 999;

        // Verification should fail because signature is now invalid
        assert!(!checkpoint.verify_signature().unwrap());
    }
}
