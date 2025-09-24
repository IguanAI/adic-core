use adic_consensus::DepositManager;
use adic_types::PublicKey;
use async_trait::async_trait;
use libp2p::PeerId;
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[async_trait]
pub trait DepositVerifier: Send + Sync {
    /// Verify that a peer has made a valid deposit
    async fn verify_deposit(&self, peer_id: &PeerId, proof: &[u8]) -> bool;

    /// Generate a proof-of-work challenge for the peer
    async fn generate_pow_challenge(&self) -> Vec<u8>;

    /// Verify the peer's response to the PoW challenge
    async fn verify_pow_response(&self, challenge: &[u8], response: &[u8]) -> bool;
}

/// Real implementation of deposit verification for peer connections
pub struct RealDepositVerifier {
    deposit_manager: Arc<DepositManager>,
    pow_difficulty: u8, // Number of leading zeros required in PoW
}

impl RealDepositVerifier {
    pub fn new(deposit_manager: Arc<DepositManager>) -> Self {
        Self {
            deposit_manager,
            pow_difficulty: 3, // Require 3 leading zero bits in hash
        }
    }

    pub fn with_difficulty(deposit_manager: Arc<DepositManager>, difficulty: u8) -> Self {
        Self {
            deposit_manager,
            pow_difficulty: difficulty,
        }
    }

    fn verify_pow_hash(&self, hash: &[u8]) -> bool {
        if hash.is_empty() {
            return false;
        }

        // Check if hash has required number of leading zero bits
        let mut zero_bits = 0;
        for byte in hash {
            if *byte == 0 {
                zero_bits += 8;
            } else {
                // Count leading zeros in this byte
                zero_bits += byte.leading_zeros() as u8;
                break;
            }
        }

        zero_bits >= self.pow_difficulty
    }
}

#[async_trait]
impl DepositVerifier for RealDepositVerifier {
    async fn verify_deposit(&self, _peer_id: &PeerId, proof: &[u8]) -> bool {
        // Proof should contain:
        // - PublicKey (32 bytes)
        // - MessageId (32 bytes)
        // - Signature (64 bytes)

        if proof.len() < 128 {
            return false;
        }

        // Extract components from proof
        let mut pubkey_bytes = [0u8; 32];
        pubkey_bytes.copy_from_slice(&proof[0..32]);
        let pubkey = PublicKey::from_bytes(pubkey_bytes);

        let mut message_id_bytes = [0u8; 32];
        message_id_bytes.copy_from_slice(&proof[32..64]);
        let message_id = adic_types::MessageId::from_bytes(message_id_bytes);

        // Verify that the peer has an escrowed deposit
        if let Some(proposer) = self.deposit_manager.get_proposer(&message_id).await {
            // Verify the public key matches
            if proposer.as_bytes() != pubkey.as_bytes() {
                return false;
            }

            // Verify deposit is in escrowed state
            if let Some(state) = self.deposit_manager.get_state(&message_id).await {
                return state == adic_consensus::DepositState::Escrowed;
            }
        }

        false
    }

    async fn generate_pow_challenge(&self) -> Vec<u8> {
        // Generate a random challenge
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let mut challenge = vec![0u8; 32];
        rng.fill(&mut challenge[..]);
        challenge
    }

    async fn verify_pow_response(&self, challenge: &[u8], response: &[u8]) -> bool {
        // Response should be a nonce that when hashed with challenge produces required difficulty
        if response.len() < 8 {
            return false;
        }

        // Compute hash of challenge + response
        let mut hasher = Sha256::new();
        hasher.update(challenge);
        hasher.update(response);
        let hash = hasher.finalize();

        // Verify the hash meets difficulty requirement
        self.verify_pow_hash(&hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_consensus::DEFAULT_DEPOSIT_AMOUNT;

    #[test]
    fn test_pow_hash_verification() {
        let deposit_manager = Arc::new(DepositManager::new(DEFAULT_DEPOSIT_AMOUNT));
        let verifier = RealDepositVerifier::with_difficulty(deposit_manager, 8);

        // Hash with 1 leading zero byte (8 bits)
        let hash1 = vec![0x00, 0xFF, 0xFF, 0xFF];
        assert!(verifier.verify_pow_hash(&hash1));

        // Hash with no leading zeros
        let hash2 = vec![0xFF, 0xFF, 0xFF, 0xFF];
        assert!(!verifier.verify_pow_hash(&hash2));

        // Hash with 2 leading zero bytes (16 bits)
        let verifier2 = RealDepositVerifier::with_difficulty(
            Arc::new(DepositManager::new(DEFAULT_DEPOSIT_AMOUNT)),
            16,
        );
        let hash3 = vec![0x00, 0x00, 0xFF, 0xFF];
        assert!(verifier2.verify_pow_hash(&hash3));
        assert!(!verifier2.verify_pow_hash(&hash1)); // Only 8 bits, need 16
    }

    #[tokio::test]
    async fn test_pow_challenge_and_response() {
        let deposit_manager = Arc::new(DepositManager::new(DEFAULT_DEPOSIT_AMOUNT));
        let verifier = RealDepositVerifier::with_difficulty(deposit_manager, 8);

        let challenge = verifier.generate_pow_challenge().await;
        assert_eq!(challenge.len(), 32);

        // Find a valid nonce (simplified for testing)
        let mut nonce = 0u64;
        loop {
            let response = nonce.to_be_bytes().to_vec();
            let mut hasher = Sha256::new();
            hasher.update(&challenge);
            hasher.update(&response);
            let hash = hasher.finalize();

            if hash[0] == 0 {
                // Found valid nonce
                assert!(verifier.verify_pow_response(&challenge, &response).await);
                break;
            }

            nonce += 1;
            if nonce > 100000 {
                panic!("Could not find valid nonce in reasonable time");
            }
        }
    }

    #[tokio::test]
    async fn test_deposit_verification() {
        let deposit_manager = Arc::new(DepositManager::new(DEFAULT_DEPOSIT_AMOUNT));
        let verifier = RealDepositVerifier::new(deposit_manager.clone());

        // Create a test deposit
        let message_id = adic_types::MessageId::new(b"test_message");
        let proposer = PublicKey::from_bytes([1; 32]);

        // Escrow the deposit
        deposit_manager.escrow(message_id, proposer).await.unwrap();

        // Create valid proof
        let mut proof = Vec::new();
        proof.extend_from_slice(proposer.as_bytes());
        proof.extend_from_slice(message_id.as_bytes());
        proof.extend_from_slice(&[0u8; 64]); // Dummy signature

        let peer_id = PeerId::random();
        assert!(verifier.verify_deposit(&peer_id, &proof).await);

        // Invalid proof (wrong length)
        assert!(!verifier.verify_deposit(&peer_id, &[0u8; 10]).await);

        // Invalid proof (wrong proposer)
        let mut wrong_proof = Vec::new();
        wrong_proof.extend_from_slice(&[2u8; 32]); // Wrong pubkey
        wrong_proof.extend_from_slice(message_id.as_bytes());
        wrong_proof.extend_from_slice(&[0u8; 64]);
        assert!(!verifier.verify_deposit(&peer_id, &wrong_proof).await);
    }
}
