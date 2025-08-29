use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use blake3;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use thiserror::Error;
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey};

#[derive(Error, Debug)]
pub enum CryptoError {
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("Key exchange failed: {0}")]
    KeyExchangeFailed(String),

    #[error("Invalid key size")]
    InvalidKeySize,

    #[error("Signature verification failed")]
    SignatureVerificationFailed,
}

pub type Result<T> = std::result::Result<T, CryptoError>;

/// Standard AES-256-GCM encryption with X25519 key exchange
pub struct StandardCrypto {
    // Uses standard, vetted cryptographic primitives
}

impl Default for StandardCrypto {
    fn default() -> Self {
        Self::new()
    }
}

impl StandardCrypto {
    pub fn new() -> Self {
        Self {}
    }

    /// Generate a new random 256-bit key
    pub fn generate_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        key
    }

    /// Encrypt data using AES-256-GCM
    pub fn encrypt(&self, plaintext: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));

        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

        // Prepend nonce to ciphertext
        let mut result = nonce_bytes.to_vec();
        result.extend_from_slice(&ciphertext);
        Ok(result)
    }

    /// Decrypt data using AES-256-GCM
    pub fn decrypt(&self, ciphertext: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
        if ciphertext.len() < 12 {
            return Err(CryptoError::DecryptionFailed("Ciphertext too short".into()));
        }

        let (nonce_bytes, actual_ciphertext) = ciphertext.split_at(12);
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
        let nonce = Nonce::from_slice(nonce_bytes);

        cipher
            .decrypt(nonce, actual_ciphertext)
            .map_err(|e| CryptoError::DecryptionFailed(e.to_string()))
    }

    /// Hash data using Blake3
    pub fn hash(&self, data: &[u8]) -> [u8; 32] {
        blake3::hash(data).into()
    }
}

/// Standard X25519 Diffie-Hellman key exchange
pub struct StandardKeyExchange {
    private_key: EphemeralSecret,
    public_key: X25519PublicKey,
}

impl Default for StandardKeyExchange {
    fn default() -> Self {
        Self::new()
    }
}

impl StandardKeyExchange {
    /// Create a new key exchange instance
    pub fn new() -> Self {
        let private_key = EphemeralSecret::random_from_rng(OsRng);
        let public_key = X25519PublicKey::from(&private_key);

        Self {
            private_key,
            public_key,
        }
    }

    /// Get our public key
    pub fn public_key(&self) -> [u8; 32] {
        self.public_key.to_bytes()
    }

    /// Compute shared secret with peer's public key
    pub fn compute_shared_secret(self, peer_public_key: &[u8; 32]) -> Result<[u8; 32]> {
        let peer_key = X25519PublicKey::from(*peer_public_key);
        let shared_secret = self.private_key.diffie_hellman(&peer_key);
        Ok(shared_secret.to_bytes())
    }
}

/// Ed25519 signature operations
pub struct StandardSigner {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
}

impl StandardSigner {
    /// Create new signer from seed
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(seed);
        let verifying_key = signing_key.verifying_key();

        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Generate new random signer
    pub fn new() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Sign a message
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        let signature = self.signing_key.sign(message);
        signature.to_bytes().to_vec()
    }

    /// Get verifying key bytes
    pub fn verifying_key_bytes(&self) -> [u8; 32] {
        self.verifying_key.to_bytes()
    }

    /// Verify a signature
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> Result<()> {
        if signature.len() != 64 {
            return Err(CryptoError::SignatureVerificationFailed);
        }

        let sig_array: [u8; 64] = signature
            .try_into()
            .map_err(|_| CryptoError::SignatureVerificationFailed)?;
        let signature = Signature::from_bytes(&sig_array);

        self.verifying_key
            .verify(message, &signature)
            .map_err(|_| CryptoError::SignatureVerificationFailed)
    }
}

/// Verify a signature with a public key
pub fn verify_signature(public_key: &[u8; 32], message: &[u8], signature: &[u8]) -> Result<()> {
    if signature.len() != 64 {
        return Err(CryptoError::SignatureVerificationFailed);
    }

    let verifying_key = VerifyingKey::from_bytes(public_key)
        .map_err(|_| CryptoError::SignatureVerificationFailed)?;

    let sig_array: [u8; 64] = signature
        .try_into()
        .map_err(|_| CryptoError::SignatureVerificationFailed)?;
    let signature = Signature::from_bytes(&sig_array);

    verifying_key
        .verify(message, &signature)
        .map_err(|_| CryptoError::SignatureVerificationFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encryption_decryption() {
        let crypto = StandardCrypto::new();
        let key = StandardCrypto::generate_key();
        let plaintext = b"Hello, World!";

        let ciphertext = crypto.encrypt(plaintext, &key).unwrap();
        let decrypted = crypto.decrypt(&ciphertext, &key).unwrap();

        assert_eq!(plaintext, &decrypted[..]);
    }

    #[test]
    fn test_key_exchange() {
        let alice = StandardKeyExchange::new();
        let bob = StandardKeyExchange::new();

        let alice_public = alice.public_key();
        let bob_public = bob.public_key();

        let alice_shared = alice.compute_shared_secret(&bob_public).unwrap();
        let bob_shared = bob.compute_shared_secret(&alice_public).unwrap();

        assert_eq!(alice_shared, bob_shared);
    }

    #[test]
    fn test_signing() {
        let signer = StandardSigner::new();
        let message = b"Test message";

        let signature = signer.sign(message);
        assert!(signer.verify(message, &signature).is_ok());

        // Verify with wrong message should fail
        assert!(signer.verify(b"Wrong message", &signature).is_err());
    }
}
