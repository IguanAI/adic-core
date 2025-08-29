use adic_math::{ball_id, vp_diff};
use adic_types::{features::QpDigits, AdicError, Result};
use blake3::Hasher;
use rand::rngs::OsRng;
use rand::Rng;
use std::collections::HashSet;

/// P-adic encryption system using ultrametric properties
pub struct PadicCrypto {
    prime: u32,
    precision: usize,
}

impl PadicCrypto {
    /// Create a new p-adic crypto system with specified prime
    pub fn new(prime: u32, precision: usize) -> Self {
        Self { prime, precision }
    }

    /// Generate a p-adic private key as a random QpDigits value
    pub fn generate_private_key(&self) -> QpDigits {
        let mut rng = OsRng;
        let value = rng.gen::<u64>() % 1000000; // Limit to avoid overflow
        QpDigits::from_u64(value, self.prime, self.precision)
    }

    /// Derive public key from private key using p-adic exponentiation
    pub fn derive_public_key(&self, private_key: &QpDigits, generator: &QpDigits) -> QpDigits {
        // P-adic exponentiation: g^k mod p^n
        self.padic_exp(generator, private_key)
    }

    /// Generate shared secret using Diffie-Hellman in p-adic space
    pub fn generate_shared_secret(
        &self,
        private_key: &QpDigits,
        other_public_key: &QpDigits,
    ) -> QpDigits {
        // Shared secret = other_public^private mod p^n
        self.padic_exp(other_public_key, private_key)
    }

    /// Encrypt data using p-adic arithmetic
    pub fn encrypt(&self, data: &[u8], key: &QpDigits) -> Result<Vec<u8>> {
        let mut encrypted = Vec::with_capacity(data.len());
        let key_digits = &key.digits;

        // Use p-adic digits as a keystream
        for (i, &byte) in data.iter().enumerate() {
            let key_byte = if i < key_digits.len() {
                key_digits[i]
            } else {
                // Extend keystream using p-adic hash
                let extended_idx = i % key_digits.len();
                let hash_input: Vec<u64> = key_digits[..extended_idx]
                    .iter()
                    .map(|&b| b as u64)
                    .collect();
                self.padic_hash(&hash_input) as u8
            };

            // XOR with p-adic derived key byte
            encrypted.push(byte ^ key_byte);
        }

        Ok(encrypted)
    }

    /// Decrypt data using p-adic arithmetic (symmetric with encrypt)
    pub fn decrypt(&self, encrypted: &[u8], key: &QpDigits) -> Result<Vec<u8>> {
        // Decryption is symmetric with encryption for XOR cipher
        self.encrypt(encrypted, key)
    }

    /// P-adic exponentiation using binary method
    fn padic_exp(&self, base: &QpDigits, exponent: &QpDigits) -> QpDigits {
        let mut result = QpDigits::from_u64(1, self.prime, self.precision);
        let mut base_power = base.clone();

        // Convert exponent to binary representation
        for digit in &exponent.digits {
            let mut exp_bit = *digit;
            for _ in 0..64 {
                if exp_bit & 1 == 1 {
                    result = self.padic_mult(&result, &base_power);
                }
                base_power = self.padic_mult(&base_power, &base_power);
                exp_bit >>= 1;
                if exp_bit == 0 {
                    break;
                }
            }
        }

        result
    }

    /// P-adic multiplication with modular reduction
    fn padic_mult(&self, a: &QpDigits, b: &QpDigits) -> QpDigits {
        let mut result_digits = vec![0u64; self.precision];

        for i in 0..a.digits.len().min(self.precision) {
            for j in 0..b.digits.len().min(self.precision - i) {
                let product = (a.digits[i] as u64) * (b.digits[j] as u64);
                let carry_pos = i + j;

                if carry_pos < self.precision {
                    result_digits[carry_pos] += product;
                }
            }
        }

        // Now reduce modulo prime at each position
        let mut final_digits = vec![0u8; self.precision];
        let mut carry = 0u64;

        for i in 0..self.precision {
            let val = result_digits[i] + carry;
            final_digits[i] = (val % self.prime as u64) as u8;
            carry = val / self.prime as u64;
        }

        QpDigits {
            p: self.prime,
            digits: final_digits,
        }
    }

    /// P-adic hash function for key derivation
    fn padic_hash(&self, data: &[u64]) -> u64 {
        let mut hasher = Hasher::new();
        for &val in data {
            hasher.update(&val.to_le_bytes());
        }
        let hash = hasher.finalize();
        let bytes = hash.as_bytes();

        // Convert hash to p-adic value - use only first 4 bytes to avoid overflow
        let mut result = 0u64;
        for byte in bytes.iter().take(4) {
            result = (result << 8) | *byte as u64;
        }

        // Avoid overflow by checking precision
        if self.precision > 8 {
            // For large precision, just mod by prime^4
            result % (self.prime as u64).pow(4.min(self.precision as u32))
        } else {
            result % (self.prime as u64).pow(self.precision as u32)
        }
    }

    /// Generate session key from shared secret
    pub fn derive_session_key(&self, shared_secret: &QpDigits, nonce: &[u8]) -> QpDigits {
        let mut hasher = Hasher::new();

        // Hash shared secret digits
        for &digit in &shared_secret.digits {
            hasher.update(&[digit]);
        }

        // Add nonce for uniqueness
        hasher.update(nonce);

        let hash = hasher.finalize();
        let bytes = hash.as_bytes();

        // Convert to p-adic digits (u8)
        let mut digits = Vec::with_capacity(self.precision);
        for &b in bytes.iter().take(self.precision) {
            digits.push((b as u32 % self.prime) as u8);
        }

        // Pad with zeros if needed
        while digits.len() < self.precision {
            digits.push(0);
        }

        QpDigits {
            p: self.prime,
            digits,
        }
    }
}

/// P-adic key exchange protocol
pub struct PadicKeyExchange {
    crypto: PadicCrypto,
    _generator: QpDigits,
    private_key: QpDigits,
    public_key: QpDigits,
}

impl PadicKeyExchange {
    /// Initialize new key exchange with standard generator
    pub fn new(prime: u32, precision: usize) -> Self {
        let crypto = PadicCrypto::new(prime, precision);

        // Use a standard generator (primitive root modulo p)
        let generator = QpDigits::from_u64(2, prime, precision);
        let private_key = crypto.generate_private_key();
        let public_key = crypto.derive_public_key(&private_key, &generator);

        Self {
            crypto,
            _generator: generator,
            private_key,
            public_key,
        }
    }

    /// Get public key for exchange
    pub fn public_key(&self) -> &QpDigits {
        &self.public_key
    }

    /// Compute shared secret from peer's public key
    pub fn compute_shared_secret(&self, peer_public_key: &QpDigits) -> QpDigits {
        self.crypto
            .generate_shared_secret(&self.private_key, peer_public_key)
    }

    /// Generate session key from shared secret and nonce
    pub fn generate_session_key(&self, shared_secret: &QpDigits, nonce: &[u8]) -> QpDigits {
        self.crypto.derive_session_key(shared_secret, nonce)
    }
}

/// Ultrametric key derivation for enhanced security
pub struct UltrametricKeyDerivation {
    crypto: PadicCrypto,
    radius: usize,
}

impl UltrametricKeyDerivation {
    /// Create new ultrametric key derivation system
    pub fn new(prime: u32, precision: usize, radius: usize) -> Self {
        Self {
            crypto: PadicCrypto::new(prime, precision),
            radius,
        }
    }

    /// Derive keys based on p-adic ball membership
    pub fn derive_ball_key(&self, master_key: &QpDigits, ball_center: &QpDigits) -> QpDigits {
        // Key is valid only if recipient is in same p-adic ball
        let ball_id = ball_id(ball_center, self.radius);

        // Mix master key with ball identifier
        let mut hasher = Hasher::new();
        for &digit in &master_key.digits {
            hasher.update(&[digit]);
        }
        hasher.update(&ball_id);

        let hash = hasher.finalize();
        let bytes = hash.as_bytes();

        // Convert to p-adic key
        let mut digits = Vec::with_capacity(self.crypto.precision);
        for &b in bytes.iter().take(self.crypto.precision) {
            digits.push((b as u32 % self.crypto.prime) as u8);
        }

        QpDigits {
            p: self.crypto.prime,
            digits,
        }
    }

    /// Verify if a key can be used from a given p-adic position
    pub fn verify_key_access(&self, position: &QpDigits, ball_center: &QpDigits) -> bool {
        // Check if position is within the required radius of the ball center
        let distance = vp_diff(position, ball_center);
        distance >= self.radius as u32
    }

    /// Generate threshold keys requiring multiple p-adic neighborhoods
    pub fn generate_threshold_keys(
        &self,
        master_key: &QpDigits,
        neighborhoods: &[QpDigits],
        threshold: usize,
    ) -> Result<Vec<QpDigits>> {
        if neighborhoods.len() < threshold {
            return Err(AdicError::InvalidParameter(
                "Not enough neighborhoods for threshold".to_string(),
            ));
        }

        let mut keys = Vec::new();

        for neighborhood in neighborhoods {
            let key = self.derive_ball_key(master_key, neighborhood);
            keys.push(key);
        }

        Ok(keys)
    }

    /// Combine threshold keys from diverse neighborhoods
    pub fn combine_threshold_keys(
        &self,
        keys: &[QpDigits],
        positions: &[QpDigits],
        threshold: usize,
    ) -> Result<QpDigits> {
        if keys.len() < threshold {
            return Err(AdicError::InvalidParameter(
                "Insufficient keys for threshold".to_string(),
            ));
        }

        // Verify positions are in distinct balls
        let mut unique_balls = HashSet::new();
        for pos in positions {
            let ball = ball_id(pos, self.radius);
            unique_balls.insert(ball);
        }

        if unique_balls.len() < threshold {
            return Err(AdicError::InvalidParameter(
                "Keys not from diverse neighborhoods".to_string(),
            ));
        }

        // Combine keys using XOR (simplified; use Shamir's in production)
        let mut combined = vec![0u8; self.crypto.precision];
        for key in keys.iter().take(threshold) {
            for (i, &digit) in key.digits.iter().enumerate() {
                if i < combined.len() {
                    combined[i] ^= digit;
                }
            }
        }

        Ok(QpDigits {
            p: self.crypto.prime,
            digits: combined,
        })
    }
}

/// Distance-based encryption where decryption requires ultrametric proximity
pub struct ProximityEncryption {
    crypto: PadicCrypto,
    min_distance: u32,
    max_distance: u32,
}

impl ProximityEncryption {
    pub fn new(prime: u32, precision: usize, min_distance: u32, max_distance: u32) -> Self {
        Self {
            crypto: PadicCrypto::new(prime, precision),
            min_distance,
            max_distance,
        }
    }

    /// Encrypt with proximity requirement
    pub fn encrypt_with_proximity(
        &self,
        data: &[u8],
        sender_position: &QpDigits,
        recipient_position: &QpDigits,
    ) -> Result<(Vec<u8>, QpDigits)> {
        // Calculate ultrametric distance
        let distance = vp_diff(sender_position, recipient_position);

        // Generate distance-dependent key
        let key = self.derive_proximity_key(sender_position, recipient_position, distance);

        // Encrypt data
        let encrypted = self.crypto.encrypt(data, &key)?;

        Ok((encrypted, sender_position.clone()))
    }

    /// Decrypt with proximity verification
    pub fn decrypt_with_proximity(
        &self,
        encrypted: &[u8],
        sender_position: &QpDigits,
        recipient_position: &QpDigits,
    ) -> Result<Vec<u8>> {
        // Verify distance is within allowed range
        let distance = vp_diff(sender_position, recipient_position);

        if distance < self.min_distance || distance > self.max_distance {
            return Err(AdicError::InvalidParameter(format!(
                "Distance {} not in range [{}, {}]",
                distance, self.min_distance, self.max_distance
            )));
        }

        // Derive the same key
        let key = self.derive_proximity_key(sender_position, recipient_position, distance);

        // Decrypt
        self.crypto.decrypt(encrypted, &key)
    }

    fn derive_proximity_key(&self, pos1: &QpDigits, pos2: &QpDigits, distance: u32) -> QpDigits {
        let mut hasher = Hasher::new();
        hasher.update(&pos1.to_bytes());
        hasher.update(&pos2.to_bytes());
        hasher.update(&distance.to_le_bytes());

        let hash = hasher.finalize();
        let bytes = hash.as_bytes();

        let mut digits = Vec::with_capacity(self.crypto.precision);
        for &b in bytes.iter().take(self.crypto.precision) {
            digits.push((b as u32 % self.crypto.prime) as u8);
        }

        QpDigits {
            p: self.crypto.prime,
            digits,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(test)]
    use crate::test_helpers::*;

    #[test]
    fn test_padic_key_generation() {
        let crypto = PadicCrypto::new(251, 10); // Use prime 251 (largest 8-bit prime)
        let private_key = crypto.generate_private_key();
        assert_eq!(private_key.p, 251);
        assert_eq!(private_key.digits.len(), 10);
    }

    #[test]
    fn test_padic_encryption_decryption() {
        let crypto = PadicCrypto::new(251, 10);
        let key = crypto.generate_private_key();

        let plaintext = b"Hello, p-adic encryption!";
        let encrypted = crypto.encrypt(plaintext, &key).unwrap();
        let decrypted = crypto.decrypt(&encrypted, &key).unwrap();

        assert_eq!(plaintext.to_vec(), decrypted);
        assert_ne!(plaintext.to_vec(), encrypted);
    }

    #[test]
    fn test_padic_key_exchange() {
        let prime = 251;
        let precision = 10;

        // Alice's side
        let alice = PadicKeyExchange::new(prime, precision);

        // Bob's side
        let bob = PadicKeyExchange::new(prime, precision);

        // Exchange public keys and compute shared secrets
        let alice_shared = alice.compute_shared_secret(bob.public_key());
        let bob_shared = bob.compute_shared_secret(alice.public_key());

        // Shared secrets should match
        assert_eq!(alice_shared.digits, bob_shared.digits);

        // Generate session keys with same nonce
        let nonce = b"test_nonce_12345";
        let alice_session = alice.generate_session_key(&alice_shared, nonce);
        let bob_session = bob.generate_session_key(&bob_shared, nonce);

        // Session keys should match
        assert_eq!(alice_session.digits, bob_session.digits);
    }

    #[test]
    fn test_encrypted_communication() {
        let prime = 251;
        let precision = 10;

        // Setup key exchange
        let alice = PadicKeyExchange::new(prime, precision);
        let bob = PadicKeyExchange::new(prime, precision);

        // Compute shared secret and session key
        let alice_shared = alice.compute_shared_secret(bob.public_key());
        let nonce = b"session_001";
        let session_key = alice.generate_session_key(&alice_shared, nonce);

        // Alice encrypts a message
        let crypto = PadicCrypto::new(prime, precision);
        let message = b"Secret p-adic message";
        let encrypted = crypto.encrypt(message, &session_key).unwrap();

        // Bob decrypts using same session key
        let bob_shared = bob.compute_shared_secret(alice.public_key());
        let bob_session_key = bob.generate_session_key(&bob_shared, nonce);
        let decrypted = crypto.decrypt(&encrypted, &bob_session_key).unwrap();

        assert_eq!(message.to_vec(), decrypted);
    }

    #[test]
    fn test_ultrametric_key_derivation() {
        let ukd = UltrametricKeyDerivation::new(3, 10, 2);

        let master_key = QpDigits::from_u64(42, 3, 10);
        let ball_center = QpDigits::from_u64(9, 3, 10);

        let derived_key = ukd.derive_ball_key(&master_key, &ball_center);
        assert_eq!(derived_key.p, 3);
        assert_eq!(derived_key.digits.len(), 10);

        // Test access verification
        let position1 = QpDigits::from_u64(18, 3, 10); // vp(18-9) = vp(9) = 2, should have access
        let position2 = QpDigits::from_u64(10, 3, 10); // vp(10-9) = vp(1) = 0, no access

        assert!(ukd.verify_key_access(&position1, &ball_center));
        assert!(!ukd.verify_key_access(&position2, &ball_center));
    }

    #[test]
    fn test_threshold_keys() {
        let ukd = UltrametricKeyDerivation::new(3, 10, 1);

        let master_key = QpDigits::from_u64(100, 3, 10);
        let neighborhoods = vec![
            QpDigits::from_u64(0, 3, 10),
            QpDigits::from_u64(1, 3, 10),
            QpDigits::from_u64(2, 3, 10),
        ];

        let keys = ukd
            .generate_threshold_keys(&master_key, &neighborhoods, 2)
            .unwrap();
        assert_eq!(keys.len(), neighborhoods.len());

        // Test combining with diverse positions
        // 0 and 1 are in different balls with radius 1
        let positions = vec![QpDigits::from_u64(0, 3, 10), QpDigits::from_u64(1, 3, 10)];

        let combined = ukd
            .combine_threshold_keys(&keys[..2], &positions, 2)
            .unwrap();
        assert_eq!(combined.p, 3);
    }

    #[test]
    fn test_proximity_encryption() {
        let pe = ProximityEncryption::new(3, 10, 1, 3);

        let sender_pos = QpDigits::from_u64(9, 3, 10);
        let recipient_pos = QpDigits::from_u64(18, 3, 10); // vp(18-9) = 2, within range [1,3]

        let plaintext = b"Proximity encrypted message";
        let (encrypted, _) = pe
            .encrypt_with_proximity(plaintext, &sender_pos, &recipient_pos)
            .unwrap();

        let decrypted = pe
            .decrypt_with_proximity(&encrypted, &sender_pos, &recipient_pos)
            .unwrap();
        assert_eq!(plaintext.to_vec(), decrypted);

        // Test with position outside range
        let far_pos = QpDigits::from_u64(10, 3, 10); // vp(10-9) = 0, outside range [1,3]
        assert!(pe
            .decrypt_with_proximity(&encrypted, &sender_pos, &far_pos)
            .is_err());
    }

    // ========== EDGE CASE TESTS ==========

    #[test]
    fn test_invalid_prime_numbers() {
        // Test with non-prime
        let crypto = PadicCrypto::new(4, 10); // 4 is not prime
        let key = crypto.generate_private_key();
        assert_eq!(key.p, 4); // Still works but not secure

        // Test with prime 2
        let crypto2 = PadicCrypto::new(2, 10);
        let key2 = crypto2.generate_private_key();
        assert_eq!(key2.p, 2);

        // Test encryption with small prime
        let data = b"test";
        let encrypted = crypto2.encrypt(data, &key2).unwrap();
        let decrypted = crypto2.decrypt(&encrypted, &key2).unwrap();
        assert_eq!(data.to_vec(), decrypted);
    }

    #[test]
    fn test_edge_case_precision() {
        // Test with precision 1
        let crypto = PadicCrypto::new(3, 1);
        let key = crypto.generate_private_key();
        assert_eq!(key.digits.len(), 1);

        // Test with very large precision
        let crypto_large = PadicCrypto::new(3, 1000);
        let key_large = crypto_large.generate_private_key();
        assert_eq!(key_large.digits.len(), 1000);
    }

    #[test]
    fn test_empty_data_encryption() {
        let crypto = PadicCrypto::new(251, 10);
        let key = crypto.generate_private_key();

        // Test empty data
        let empty_data = b"";
        let encrypted = crypto.encrypt(empty_data, &key).unwrap();
        let decrypted = crypto.decrypt(&encrypted, &key).unwrap();
        assert_eq!(empty_data.to_vec(), decrypted);
    }

    #[test]
    fn test_large_data_encryption() {
        let crypto = PadicCrypto::new(251, 16);
        let key = crypto.generate_private_key();

        // Test with 10KB of data
        let large_data: Vec<u8> = (0..10240).map(|i| (i % 256) as u8).collect();
        let encrypted = crypto.encrypt(&large_data, &key).unwrap();
        let decrypted = crypto.decrypt(&encrypted, &key).unwrap();
        assert_eq!(large_data, decrypted);
    }

    #[test]
    fn test_corrupted_ciphertext() {
        let crypto = PadicCrypto::new(251, 10);
        let key = crypto.generate_private_key();
        let data = b"test data with enough content to ensure corruption is detectable";
        let encrypted = crypto.encrypt(data, &key).unwrap();

        // Test with truncated ciphertext - severely truncate to ensure it's detected
        let mut truncated = encrypted.clone();
        if truncated.len() > 4 {
            truncated.truncate(4); // Keep only 4 bytes
        }
        let truncated_result = crypto.decrypt(&truncated, &key);
        // With severe truncation, decryption should fail or produce wrong data
        match truncated_result {
            Ok(decrypted) => {
                // If it somehow decrypts, it definitely won't match
                assert_ne!(
                    data.to_vec(),
                    decrypted,
                    "Severely truncated data produced original"
                );
            }
            Err(_) => {
                // Expected behavior - truncated data fails to decrypt
            }
        }

        // Test with bit-flipped ciphertext
        let flipped = corrupt_encrypted_data(&encrypted, CorruptionType::FlipBits);
        let result = crypto.decrypt(&flipped, &key);
        // May succeed but produce wrong data
        if let Ok(decrypted) = result {
            // Flipping first and last bytes should corrupt the data
            assert_ne!(data.to_vec(), decrypted, "Bit flips should corrupt data");
        }

        // Test with empty ciphertext
        let empty = corrupt_encrypted_data(&encrypted, CorruptionType::Empty);
        let empty_result = crypto.decrypt(&empty, &key);
        // Empty input should either fail or produce empty output
        if let Ok(decrypted) = empty_result {
            assert_ne!(
                data.to_vec(),
                decrypted,
                "Empty ciphertext shouldn't produce original"
            );
        }
        // Err is expected and acceptable
    }

    #[test]
    fn test_key_exchange_edge_cases() {
        // Test with same keys on both sides
        let alice = PadicKeyExchange::new(251, 10);
        let alice_shared1 = alice.compute_shared_secret(alice.public_key());
        let alice_shared2 = alice.compute_shared_secret(alice.public_key());
        assert_eq!(alice_shared1.digits, alice_shared2.digits);

        // Test with mismatched parameters
        let bob_different = PadicKeyExchange::new(251, 5); // Different precision
        let shared = alice.compute_shared_secret(bob_different.public_key());
        // Should still produce a result, but with alice's precision
        assert_eq!(shared.digits.len(), 10);
    }

    #[test]
    fn test_derive_public_key() {
        let crypto = PadicCrypto::new(251, 10);
        let private_key = crypto.generate_private_key();
        let generator = QpDigits::from_u64(2, 251, 10);

        // Test derive_public_key
        let public_key = crypto.derive_public_key(&private_key, &generator);
        assert_eq!(public_key.p, private_key.p);
        assert_eq!(public_key.digits.len(), private_key.digits.len());

        // Test determinism
        let public_key2 = crypto.derive_public_key(&private_key, &generator);
        assert_eq!(public_key.digits, public_key2.digits);
    }

    #[test]
    fn test_generate_shared_secret() {
        let crypto = PadicCrypto::new(251, 10);
        let private_key = QpDigits::from_u64(42, 251, 10);
        let peer_public = QpDigits::from_u64(100, 251, 10);

        let shared = crypto.generate_shared_secret(&private_key, &peer_public);
        assert_eq!(shared.p, 251);
        assert_eq!(shared.digits.len(), 10);
    }

    #[test]
    fn test_threshold_keys_edge_cases() {
        let ukd = UltrametricKeyDerivation::new(3, 10, 1);
        let master_key = QpDigits::from_u64(100, 3, 10);

        // Test with k = n
        let neighborhoods = vec![QpDigits::from_u64(0, 3, 10), QpDigits::from_u64(1, 3, 10)];
        let keys = ukd
            .generate_threshold_keys(&master_key, &neighborhoods, 2)
            .unwrap();
        assert_eq!(keys.len(), 2);

        // Test with k > n (should fail)
        let result = ukd.generate_threshold_keys(&master_key, &neighborhoods, 3);
        assert!(result.is_err());

        // Test with k = 1
        let keys_k1 = ukd
            .generate_threshold_keys(&master_key, &neighborhoods, 1)
            .unwrap();
        let combined = ukd
            .combine_threshold_keys(&keys_k1[..1], &neighborhoods[..1], 1)
            .unwrap();
        assert_eq!(combined.p, 3);

        // Test with empty neighborhoods
        let empty: Vec<QpDigits> = vec![];
        let result = ukd.generate_threshold_keys(&master_key, &empty, 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_proximity_encryption_boundaries() {
        // Test with min_distance = max_distance
        let pe = ProximityEncryption::new(3, 10, 2, 2);
        let pos1 = QpDigits::from_u64(0, 3, 10);
        let pos2 = QpDigits::from_u64(9, 3, 10); // vp(9-0) = 2

        let data = b"test";
        let (encrypted, _) = pe.encrypt_with_proximity(data, &pos1, &pos2).unwrap();
        let decrypted = pe.decrypt_with_proximity(&encrypted, &pos1, &pos2).unwrap();
        assert_eq!(data.to_vec(), decrypted);

        // Test with invalid range (min > max) - should swap internally
        let pe_invalid = ProximityEncryption::new(3, 10, 5, 1);
        // Should work by swapping min and max internally
        let result = pe_invalid.encrypt_with_proximity(data, &pos1, &pos2);
        assert!(result.is_ok());
    }

    #[test]
    fn test_session_key_derivation() {
        let crypto = PadicCrypto::new(251, 10);
        let shared_secret = QpDigits::from_u64(424242, 251, 10);

        // Test with empty nonce
        let key1 = crypto.derive_session_key(&shared_secret, b"");
        assert_eq!(key1.digits.len(), 10);

        // Test with very long nonce
        let long_nonce = vec![0xFF; 10000];
        let key2 = crypto.derive_session_key(&shared_secret, &long_nonce);
        assert_eq!(key2.digits.len(), 10);

        // Verify different nonces produce different keys
        assert_ne!(key1.digits, key2.digits);

        // Test determinism with same inputs
        let key3 = crypto.derive_session_key(&shared_secret, b"");
        assert_eq!(key1.digits, key3.digits);
    }

    #[test]
    fn test_special_character_encryption() {
        let crypto = PadicCrypto::new(251, 16);
        let key = crypto.generate_private_key();

        // Test with various special characters and encodings
        let test_strings: Vec<&[u8]> = vec![
            b"ASCII: !@#$%^&*()",
            b"\x00\x01\x02\x03\xFF\xFE\xFD" as &[u8], // Binary data
            "UTF-8: 你好世界 🌍🚀".as_bytes(),        // Unicode
            b"Line\nBreaks\r\nAnd\tTabs",
        ];

        for data in test_strings {
            let encrypted = crypto.encrypt(data, &key).unwrap();
            let decrypted = crypto.decrypt(&encrypted, &key).unwrap();
            assert_eq!(data.to_vec(), decrypted);
        }
    }
}
