use adic_crypto::Keypair;
#[cfg(feature = "legacy-padic-wallet")]
use adic_crypto::PadicCrypto;
use adic_economics::AccountAddress;
#[cfg(feature = "legacy-padic-wallet")]
use adic_types::QpDigits;
use adic_types::{PublicKey, Signature};
use anyhow::{Context, Result};
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
#[cfg(feature = "legacy-padic-wallet")]
use sha2::{Digest, Sha256};
use sodiumoxide::crypto::pwhash;
use sodiumoxide::crypto::secretbox;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{error, info, warn};

use crate::genesis::derive_keypair_from_seed;

#[cfg(feature = "legacy-padic-wallet")]
const PBKDF2_ITERATIONS: u32 = 100_000;

#[derive(Debug, Serialize, Deserialize)]
pub struct WalletData {
    pub version: u32,
    pub address: String,             // Bech32 format (adic1...)
    pub hex_address: Option<String>, // Hex format for compatibility
    pub public_key: String,
    pub encrypted_private_key: String, // Base64 encoded
    #[serde(default)]
    pub salt: String, // Base64 encoded salt for key derivation
    #[serde(default)]
    pub nonce: String, // Base64 encoded nonce for v4+ (XSalsa20-Poly1305)
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub node_id: Option<String>,
}

impl WalletData {
    /// Check if this wallet uses secure encryption
    pub fn is_secure(&self) -> bool {
        self.version >= 4
    }

    /// Get human-readable encryption method
    pub fn encryption_method(&self) -> &'static str {
        match self.version {
            4.. => "XSalsa20-Poly1305 (secure)",
            3 => "P-adic XOR (INSECURE - legacy only)",
            _ => "Unencrypted (INSECURE)",
        }
    }
}

pub struct NodeWallet {
    keypair: Keypair,
    address: AccountAddress,
    _path: PathBuf,
    _node_id: String,
}

impl NodeWallet {
    /// Derive encryption key from password using Argon2id (v4+)
    ///
    /// Uses sodiumoxide's pwhash (Argon2id13) with interactive parameters:
    /// - Memory: ~64 MB
    /// - Operations: ~4 iterations
    /// - Parallelism: 1 thread
    fn derive_encryption_key_v4(password: &str, salt: &pwhash::Salt) -> Result<secretbox::Key> {
        let mut key = secretbox::Key([0; secretbox::KEYBYTES]);

        pwhash::derive_key(
            &mut key.0,
            password.as_bytes(),
            salt,
            pwhash::OPSLIMIT_INTERACTIVE,
            pwhash::MEMLIMIT_INTERACTIVE,
        )
        .map_err(|_| anyhow::anyhow!("Key derivation failed"))?;

        Ok(key)
    }

    /// Encrypt keypair using XSalsa20-Poly1305 AEAD (v4)
    fn encrypt_keypair_v4(
        keypair_bytes: &[u8],
        password: &str,
    ) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
        // Generate random salt for key derivation
        let salt = pwhash::gen_salt();

        // Derive encryption key using Argon2id
        let key = Self::derive_encryption_key_v4(password, &salt)?;

        // Generate random nonce
        let nonce = secretbox::gen_nonce();

        // Encrypt with authenticated encryption (AEAD)
        let ciphertext = secretbox::seal(keypair_bytes, &nonce, &key);

        Ok((ciphertext, salt.0.to_vec(), nonce.0.to_vec()))
    }

    /// Decrypt keypair using XSalsa20-Poly1305 AEAD (v4)
    fn decrypt_keypair_v4(
        ciphertext: &[u8],
        password: &str,
        salt_bytes: &[u8],
        nonce_bytes: &[u8],
    ) -> Result<Vec<u8>> {
        // Reconstruct salt
        let salt = pwhash::Salt::from_slice(salt_bytes)
            .ok_or_else(|| anyhow::anyhow!("Invalid salt length"))?;

        // Reconstruct nonce
        let nonce = secretbox::Nonce::from_slice(nonce_bytes)
            .ok_or_else(|| anyhow::anyhow!("Invalid nonce length"))?;

        // Derive decryption key
        let key = Self::derive_encryption_key_v4(password, &salt)?;

        // Decrypt and authenticate
        secretbox::open(ciphertext, &nonce, &key)
            .map_err(|_| anyhow::anyhow!("Decryption failed - wrong password or corrupted data"))
    }

    /// Legacy v3 encryption (INSECURE - only for migration)
    #[cfg(feature = "legacy-padic-wallet")]
    fn derive_encryption_key_v3(password: &str, salt: &[u8]) -> QpDigits {
        use sha2::{Digest, Sha256};

        // Use iterative SHA256 as a simple KDF (not as secure as PBKDF2 but available)
        let mut hasher = Sha256::new();
        hasher.update(password.as_bytes());
        hasher.update(salt);

        let mut key = hasher.finalize().to_vec();

        // Iterate to increase computational cost
        for _ in 0..PBKDF2_ITERATIONS {
            let mut hasher = Sha256::new();
            hasher.update(&key);
            hasher.update(salt);
            key = hasher.finalize().to_vec();
        }

        // Convert derived key to p-adic number for encryption
        let key_val = u64::from_le_bytes(key[..8].try_into().unwrap_or([0u8; 8]));
        QpDigits::from_u64(key_val, 3, 32)
    }

    /// Load existing wallet or create a new one
    pub fn load_or_create(data_dir: &Path, node_id: &str) -> Result<Self> {
        let wallet_path = data_dir.join("wallet.json");

        // Get password from environment or use default for testnet
        let password = std::env::var("WALLET_PASSWORD").unwrap_or_else(|_| {
            warn!("Using default password for testnet - NOT SECURE FOR PRODUCTION");
            "testnet-insecure-password".to_string()
        });

        if wallet_path.exists() {
            info!("Loading existing wallet from {:?}", wallet_path);
            Self::load(&wallet_path, node_id, &password)
        } else {
            info!("Creating new wallet for node {}", node_id);
            Self::create_and_save(data_dir, node_id, &password)
        }
    }

    /// Create a new wallet and save it (v4 with secure encryption)
    fn create_and_save(data_dir: &Path, node_id: &str, password: &str) -> Result<Self> {
        // Initialize sodiumoxide
        sodiumoxide::init().map_err(|_| anyhow::anyhow!("Failed to initialize sodiumoxide"))?;

        // Ensure data directory exists
        fs::create_dir_all(data_dir)
            .with_context(|| format!("Failed to create data directory {:?}", data_dir))?;

        let wallet_path = data_dir.join("wallet.json");

        // Get seed from environment or use default
        let seed = std::env::var("WALLET_SEED")
            .unwrap_or_else(|_| format!("adic-{}-default-seed", node_id));

        // Derive keypair from seed
        let keypair = derive_keypair_from_seed(&seed);
        let public_key = keypair.public_key();
        let address = AccountAddress::from_public_key(public_key);

        // Encrypt private key using XSalsa20-Poly1305 (v4)
        let (encrypted_key, salt, nonce) = Self::encrypt_keypair_v4(&keypair.to_bytes(), password)
            .context("Failed to encrypt private key")?;

        // Create wallet data with both address formats
        let bech32_address = address
            .to_bech32()
            .unwrap_or_else(|_| hex::encode(address.as_bytes()));
        let hex_address = hex::encode(address.as_bytes());

        let wallet_data = WalletData {
            version: 4, // v4 = XSalsa20-Poly1305 AEAD
            address: bech32_address,
            hex_address: Some(hex_address),
            public_key: hex::encode(public_key.as_bytes()),
            encrypted_private_key: general_purpose::STANDARD.encode(&encrypted_key),
            salt: general_purpose::STANDARD.encode(&salt),
            nonce: general_purpose::STANDARD.encode(&nonce),
            created_at: chrono::Utc::now(),
            node_id: Some(node_id.to_string()),
        };

        // Save wallet data
        let json = serde_json::to_string_pretty(&wallet_data)?;
        fs::write(&wallet_path, json)
            .with_context(|| format!("Failed to write wallet to {:?}", wallet_path))?;

        info!(
            "Created new wallet for node {} with address {} (encryption: {})",
            node_id, wallet_data.address, wallet_data.encryption_method()
        );

        Ok(Self {
            keypair,
            address,
            _path: wallet_path,
            _node_id: node_id.to_string(),
        })
    }

    /// Load wallet from file (supports v4 secure + v3 legacy with feature flag)
    fn load(path: &Path, node_id: &str, password: &str) -> Result<Self> {
        // Initialize sodiumoxide
        sodiumoxide::init().map_err(|_| anyhow::anyhow!("Failed to initialize sodiumoxide"))?;

        let json = fs::read_to_string(path)
            .with_context(|| format!("Failed to read wallet from {:?}", path))?;

        let wallet_data: WalletData = serde_json::from_str(&json)
            .with_context(|| format!("Failed to parse wallet from {:?}", path))?;

        // Check wallet security status
        if !wallet_data.is_secure() {
            error!(
                "⚠️  INSECURE WALLET DETECTED: {} (version {})",
                path.display(),
                wallet_data.version
            );
            error!("   Encryption: {}", wallet_data.encryption_method());

            #[cfg(not(feature = "legacy-padic-wallet"))]
            {
                return Err(anyhow::anyhow!(
                    "Cannot load insecure v{} wallet in production. \
                     Please migrate to v4 using: adic wallet migrate --help",
                    wallet_data.version
                ));
            }

            #[cfg(feature = "legacy-padic-wallet")]
            {
                warn!("⚠️  Loading insecure wallet (legacy-padic-wallet feature enabled)");
                warn!("   This is ONLY for development/migration purposes");
                warn!("   Migrate to v4 immediately: adic wallet migrate");
            }
        }

        // Verify node ID matches (if specified)
        if let Some(stored_node_id) = &wallet_data.node_id {
            if stored_node_id != node_id {
                warn!(
                    "Wallet node ID mismatch: stored={}, current={}",
                    stored_node_id, node_id
                );
            }
        }

        // Decode encrypted private key
        let encrypted_key = general_purpose::STANDARD
            .decode(&wallet_data.encrypted_private_key)
            .context("Failed to decode encrypted private key")?;

        // Decrypt based on wallet version
        let key_bytes = match wallet_data.version {
            4.. => {
                // v4: XSalsa20-Poly1305 AEAD (secure)
                let salt = general_purpose::STANDARD
                    .decode(&wallet_data.salt)
                    .context("Failed to decode salt")?;

                let nonce = general_purpose::STANDARD
                    .decode(&wallet_data.nonce)
                    .context("Failed to decode nonce")?;

                Self::decrypt_keypair_v4(&encrypted_key, password, &salt, &nonce)?
            }
            3 => {
                // v3: P-adic XOR cipher (INSECURE - legacy only)
                #[cfg(feature = "legacy-padic-wallet")]
                {
                    let salt = general_purpose::STANDARD
                        .decode(&wallet_data.salt)
                        .context("Failed to decode salt")?;

                    let decryption_key = Self::derive_encryption_key_v3(password, &salt);
                    let padic_crypto = PadicCrypto::new(3, 32);
                    padic_crypto
                        .decrypt(&encrypted_key, &decryption_key)
                        .context("Failed to decrypt private key - wrong password?")?
                }
                #[cfg(not(feature = "legacy-padic-wallet"))]
                {
                    return Err(anyhow::anyhow!(
                        "v3 wallet detected but legacy-padic-wallet feature not enabled"
                    ));
                }
            }
            _ => {
                // v2 or earlier: unencrypted (INSECURE)
                return Err(anyhow::anyhow!(
                    "Unsupported wallet version: {}. Please create a new v4 wallet.",
                    wallet_data.version
                ));
            }
        };

        // Reconstruct keypair
        let keypair = Keypair::from_bytes(&key_bytes).context("Failed to reconstruct keypair")?;

        // Verify public key matches
        let public_key = keypair.public_key();
        let stored_pubkey = hex::decode(&wallet_data.public_key)?;
        if public_key.as_bytes() != &stored_pubkey[..] {
            return Err(anyhow::anyhow!("Public key mismatch in wallet"));
        }

        // Reconstruct address - try bech32 first, fall back to hex
        let address = if wallet_data.address.starts_with("adic") {
            AccountAddress::from_string(&wallet_data.address)
                .unwrap_or_else(|_| AccountAddress::from_public_key(public_key))
        } else if let Some(hex_addr) = &wallet_data.hex_address {
            AccountAddress::from_string(hex_addr)
                .unwrap_or_else(|_| AccountAddress::from_public_key(public_key))
        } else {
            // Legacy format - address field contains hex
            AccountAddress::from_string(&wallet_data.address)
                .unwrap_or_else(|_| AccountAddress::from_public_key(public_key))
        };

        info!(
            "Loaded wallet for node {} with address {}",
            node_id,
            address
                .to_bech32()
                .unwrap_or_else(|_| hex::encode(address.as_bytes()))
        );

        Ok(Self {
            keypair,
            address,
            _path: path.to_path_buf(),
            _node_id: node_id.to_string(),
        })
    }

    /// Get the wallet's address
    pub fn address(&self) -> AccountAddress {
        self.address
    }

    /// Get the wallet's public key
    pub fn public_key(&self) -> PublicKey {
        *self.keypair.public_key()
    }

    /// Get the wallet's keypair (for signing)
    pub fn keypair(&self) -> &Keypair {
        &self.keypair
    }

    /// Sign a message
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.keypair.sign(message)
    }

    /// Export wallet to a file with v4 encryption
    pub fn export_to_file(&self, export_path: &Path, password: &str) -> Result<()> {
        // Encrypt keypair using v4 (XSalsa20-Poly1305)
        let (encrypted_key, salt, nonce) = Self::encrypt_keypair_v4(&self.keypair.to_bytes(), password)
            .context("Failed to encrypt private key for export")?;

        // Create wallet data for export
        let wallet_data = WalletData {
            version: 4,
            address: self
                .address
                .to_bech32()
                .unwrap_or_else(|_| hex::encode(self.address.as_bytes())),
            hex_address: Some(hex::encode(self.address.as_bytes())),
            public_key: hex::encode(self.public_key().as_bytes()),
            encrypted_private_key: general_purpose::STANDARD.encode(&encrypted_key),
            salt: general_purpose::STANDARD.encode(&salt),
            nonce: general_purpose::STANDARD.encode(&nonce),
            created_at: chrono::Utc::now(),
            node_id: Some(self._node_id.clone()),
        };

        // Save to file
        let json = serde_json::to_string_pretty(&wallet_data)?;
        fs::write(export_path, json)
            .with_context(|| format!("Failed to export wallet to {:?}", export_path))?;

        info!("Exported wallet to {:?} ({})", export_path, wallet_data.encryption_method());
        Ok(())
    }

    /// Import wallet from a file (requires password to decrypt)
    pub fn import_from_file(import_path: &Path, password: &str, node_id: &str) -> Result<Self> {
        info!("Importing wallet from {:?}", import_path);

        // Load the wallet using existing load function
        Self::load(import_path, node_id, password)
    }

    /// Export wallet as encrypted JSON string with v4 encryption
    pub fn export_to_json(&self, password: &str) -> Result<String> {
        // Encrypt keypair using v4 (XSalsa20-Poly1305)
        let (encrypted_key, salt, nonce) = Self::encrypt_keypair_v4(&self.keypair.to_bytes(), password)
            .context("Failed to encrypt private key for export")?;

        // Create wallet data
        let wallet_data = WalletData {
            version: 4,
            address: self
                .address
                .to_bech32()
                .unwrap_or_else(|_| hex::encode(self.address.as_bytes())),
            hex_address: Some(hex::encode(self.address.as_bytes())),
            public_key: hex::encode(self.public_key().as_bytes()),
            encrypted_private_key: general_purpose::STANDARD.encode(&encrypted_key),
            salt: general_purpose::STANDARD.encode(&salt),
            nonce: general_purpose::STANDARD.encode(&nonce),
            created_at: chrono::Utc::now(),
            node_id: None, // Don't include node_id in exports
        };

        serde_json::to_string(&wallet_data).context("Failed to serialize wallet data")
    }

    /// Import wallet from JSON string (supports v4 + legacy v3 with feature flag)
    pub fn import_from_json(json: &str, password: &str, node_id: &str) -> Result<Self> {
        let wallet_data: WalletData =
            serde_json::from_str(json).context("Failed to parse wallet JSON")?;

        // Check security
        if !wallet_data.is_secure() {
            warn!("⚠️  Importing insecure {} wallet", wallet_data.encryption_method());
        }

        // Decode encrypted private key
        let encrypted_key = general_purpose::STANDARD
            .decode(&wallet_data.encrypted_private_key)
            .context("Failed to decode encrypted private key")?;

        // Decrypt based on version
        let key_bytes = match wallet_data.version {
            4.. => {
                let salt = general_purpose::STANDARD
                    .decode(&wallet_data.salt)
                    .context("Failed to decode salt")?;
                let nonce = general_purpose::STANDARD
                    .decode(&wallet_data.nonce)
                    .context("Failed to decode nonce")?;

                Self::decrypt_keypair_v4(&encrypted_key, password, &salt, &nonce)?
            }
            3 => {
                #[cfg(feature = "legacy-padic-wallet")]
                {
                    let salt = general_purpose::STANDARD
                        .decode(&wallet_data.salt)
                        .context("Failed to decode salt")?;

                    let decryption_key = Self::derive_encryption_key_v3(password, &salt);
                    let padic_crypto = PadicCrypto::new(3, 32);
                    padic_crypto
                        .decrypt(&encrypted_key, &decryption_key)
                        .context("Failed to decrypt private key - wrong password?")?
                }
                #[cfg(not(feature = "legacy-padic-wallet"))]
                {
                    return Err(anyhow::anyhow!(
                        "Cannot import v3 wallet without legacy-padic-wallet feature"
                    ));
                }
            }
            _ => {
                return Err(anyhow::anyhow!("Unsupported wallet version: {}", wallet_data.version));
            }
        };

        // Reconstruct keypair
        let keypair = Keypair::from_bytes(&key_bytes).context("Failed to reconstruct keypair")?;

        // Verify public key
        let public_key = keypair.public_key();
        let stored_pubkey = hex::decode(&wallet_data.public_key)?;
        if public_key.as_bytes() != &stored_pubkey[..] {
            return Err(anyhow::anyhow!("Public key mismatch in imported wallet"));
        }

        // Reconstruct address
        let address = AccountAddress::from_public_key(public_key);

        info!(
            "Successfully imported wallet with address {} ({})",
            address
                .to_bech32()
                .unwrap_or_else(|_| hex::encode(address.as_bytes())),
            wallet_data.encryption_method()
        );

        Ok(Self {
            keypair,
            address,
            _path: PathBuf::new(), // No path for imported wallets
            _node_id: node_id.to_string(),
        })
    }

    /// Get wallet info without private key
    pub fn get_info(&self) -> WalletInfo {
        WalletInfo {
            address: self
                .address
                .to_bech32()
                .unwrap_or_else(|_| hex::encode(self.address.as_bytes())),
            hex_address: hex::encode(self.address.as_bytes()),
            public_key: hex::encode(self.public_key().as_bytes()),
            node_id: self._node_id.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WalletInfo {
    pub address: String,
    pub hex_address: String,
    pub public_key: String,
    pub node_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_wallet_creation() {
        let temp_dir = TempDir::new().unwrap();
        let wallet = NodeWallet::load_or_create(temp_dir.path(), "test-node").unwrap();

        // Just verify address was created
        assert!(!wallet.address().as_bytes().is_empty());
    }

    #[test]
    fn test_wallet_persistence() {
        let temp_dir = TempDir::new().unwrap();

        // Create wallet
        let wallet1 = NodeWallet::load_or_create(temp_dir.path(), "test-node").unwrap();
        let addr1 = wallet1.address();

        // Load wallet again
        let wallet2 = NodeWallet::load_or_create(temp_dir.path(), "test-node").unwrap();
        let addr2 = wallet2.address();

        // Should be the same address
        assert_eq!(addr1, addr2);
    }

    #[test]
    fn test_wallet_signing() {
        let temp_dir = TempDir::new().unwrap();
        let wallet = NodeWallet::load_or_create(temp_dir.path(), "test-node").unwrap();

        let message = b"test message";
        let signature = wallet.sign(message);

        // Verify signature using ed25519-dalek
        use ed25519_dalek::{Verifier, VerifyingKey};

        let public_key = wallet.public_key();
        let public_key_bytes = public_key.as_bytes();
        let verifying_key = VerifyingKey::from_bytes(public_key_bytes).unwrap();
        let sig_bytes: [u8; 64] = signature.as_bytes().try_into().unwrap();
        let dalek_sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);

        assert!(verifying_key.verify(message, &dalek_sig).is_ok());
    }

    #[test]
    fn test_wallet_export_import() {
        let temp_dir = TempDir::new().unwrap();
        let export_path = temp_dir.path().join("exported_wallet.json");
        let password = "test_password_123";

        // Create original wallet
        let original_wallet = NodeWallet::load_or_create(temp_dir.path(), "test-node").unwrap();
        let original_address = original_wallet.address();
        let original_pubkey = original_wallet.public_key();

        // Export wallet
        original_wallet
            .export_to_file(&export_path, password)
            .unwrap();

        // Import wallet
        let imported_wallet =
            NodeWallet::import_from_file(&export_path, password, "new-node").unwrap();

        // Verify addresses and keys match
        assert_eq!(imported_wallet.address(), original_address);
        assert_eq!(imported_wallet.public_key(), original_pubkey);

        // Test signing with imported wallet
        let message = b"test message";
        let signature = imported_wallet.sign(message);

        // Verify signature
        use ed25519_dalek::{Verifier, VerifyingKey};
        let verifying_key = VerifyingKey::from_bytes(original_pubkey.as_bytes()).unwrap();
        let sig_bytes: [u8; 64] = signature.as_bytes().try_into().unwrap();
        let dalek_sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        assert!(verifying_key.verify(message, &dalek_sig).is_ok());
    }

    #[test]
    fn test_wallet_export_json() {
        let temp_dir = TempDir::new().unwrap();
        let password = "json_test_password";

        // Create wallet
        let wallet = NodeWallet::load_or_create(temp_dir.path(), "test-node").unwrap();
        let original_address = wallet.address();
        let original_pubkey = wallet.public_key();

        // Export to JSON string
        let json = wallet.export_to_json(password).unwrap();

        // Import from JSON string
        let imported_wallet =
            NodeWallet::import_from_json(&json, password, "imported-node").unwrap();

        // Verify everything matches
        assert_eq!(imported_wallet.address(), original_address);
        assert_eq!(imported_wallet.public_key(), original_pubkey);
    }

    #[test]
    fn test_wallet_import_wrong_password() {
        let temp_dir = TempDir::new().unwrap();
        let export_path = temp_dir.path().join("exported_wallet.json");
        let correct_password = "correct_password";
        let wrong_password = "wrong_password";

        // Create and export wallet
        let wallet = NodeWallet::load_or_create(temp_dir.path(), "test-node").unwrap();
        wallet
            .export_to_file(&export_path, correct_password)
            .unwrap();

        // Try to import with wrong password
        let import_result = NodeWallet::import_from_file(&export_path, wrong_password, "new-node");

        // Should fail with decryption error (v4 AEAD will catch this)
        assert!(import_result.is_err());
        let err_msg = import_result.err().unwrap().to_string();
        assert!(
            err_msg.contains("Decryption failed") || err_msg.contains("wrong password"),
            "Expected authentication failure, got: {}",
            err_msg
        );
    }

    // ========== V4 SECURITY TESTS (from C3_ANALYSIS.md) ==========

    #[test]
    fn test_v4_wallet_encryption_confidentiality() {
        // Same password, different salts → different ciphertexts (non-deterministic)
        let password = "same_password";
        let keypair_bytes = b"fake_keypair_for_testing_32bytes";

        let (ciphertext1, salt1, nonce1) =
            NodeWallet::encrypt_keypair_v4(keypair_bytes, password).unwrap();
        let (ciphertext2, salt2, nonce2) =
            NodeWallet::encrypt_keypair_v4(keypair_bytes, password).unwrap();

        // Different salts (randomized)
        assert_ne!(salt1, salt2, "Salts should be randomized");

        // Different nonces (randomized)
        assert_ne!(nonce1, nonce2, "Nonces should be randomized");

        // Different ciphertexts (due to different nonces)
        assert_ne!(
            ciphertext1, ciphertext2,
            "Same plaintext should produce different ciphertexts with different nonces"
        );
    }

    #[test]
    fn test_v4_wallet_encryption_authentication() {
        let password = "test_password";
        let keypair_bytes = b"fake_keypair_for_testing_32bytes";

        let (mut ciphertext, salt, nonce) =
            NodeWallet::encrypt_keypair_v4(keypair_bytes, password).unwrap();

        // Tamper with ciphertext (flip bits)
        if !ciphertext.is_empty() {
            ciphertext[0] ^= 0xFF;
        }

        // Decryption should fail with authentication error
        let result = NodeWallet::decrypt_keypair_v4(&ciphertext, password, &salt, &nonce);

        assert!(
            result.is_err(),
            "Tampered ciphertext should fail authentication"
        );

        let err_msg = result.err().unwrap().to_string();
        assert!(
            err_msg.contains("Decryption failed") || err_msg.contains("corrupted"),
            "Expected AEAD authentication failure, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_v4_wallet_security_properties() {
        let temp_dir = TempDir::new().unwrap();
        let password = "test_password";

        // Create v4 wallet
        let wallet = NodeWallet::load_or_create(temp_dir.path(), "test-node").unwrap();
        wallet.export_to_file(&temp_dir.path().join("wallet.json"), password).unwrap();

        // Load wallet data
        let json = std::fs::read_to_string(temp_dir.path().join("wallet.json")).unwrap();
        let wallet_data: WalletData = serde_json::from_str(&json).unwrap();

        // Verify v4 properties
        assert_eq!(wallet_data.version, 4, "Should be v4 wallet");
        assert!(wallet_data.is_secure(), "v4 wallet should be marked as secure");
        assert_eq!(
            wallet_data.encryption_method(),
            "XSalsa20-Poly1305 (secure)"
        );

        // Verify nonce is present (v4 requirement)
        assert!(!wallet_data.nonce.is_empty(), "v4 wallet must have nonce");

        // Verify salt is present
        assert!(!wallet_data.salt.is_empty(), "v4 wallet must have salt");

        // Verify encrypted key is present
        assert!(
            !wallet_data.encrypted_private_key.is_empty(),
            "Encrypted key must be present"
        );
    }

    #[test]
    fn test_v4_round_trip_encryption() {
        let password = "round_trip_password";
        let original_bytes = b"test_keypair_bytes_for_round_trip_32";

        // Encrypt
        let (ciphertext, salt, nonce) =
            NodeWallet::encrypt_keypair_v4(original_bytes, password).unwrap();

        // Decrypt
        let decrypted = NodeWallet::decrypt_keypair_v4(&ciphertext, password, &salt, &nonce).unwrap();

        // Verify round-trip
        assert_eq!(
            &decrypted[..],
            &original_bytes[..],
            "Decrypted data should match original"
        );
    }

    #[test]
    fn test_wallet_info() {
        let temp_dir = TempDir::new().unwrap();
        let wallet = NodeWallet::load_or_create(temp_dir.path(), "test-node").unwrap();

        let info = wallet.get_info();

        // Verify info contains expected fields
        assert!(!info.address.is_empty());
        assert!(!info.hex_address.is_empty());
        assert!(!info.public_key.is_empty());
        assert_eq!(info.node_id, "test-node");

        // Verify hex address is valid hex
        assert!(hex::decode(&info.hex_address).is_ok());
        assert!(hex::decode(&info.public_key).is_ok());
    }
}
