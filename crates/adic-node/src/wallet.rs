use adic_crypto::{Keypair, PadicCrypto};
use adic_economics::AccountAddress;
use adic_types::{PublicKey, QpDigits, Signature};
use anyhow::{Context, Result};
use base64::{engine::general_purpose, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::genesis::derive_keypair_from_seed;

const PBKDF2_ITERATIONS: u32 = 100_000;
const SALT_LEN: usize = 32;

#[derive(Debug, Serialize, Deserialize)]
pub struct WalletData {
    pub version: u32,
    pub address: String,             // Bech32 format (adic1...)
    pub hex_address: Option<String>, // Hex format for compatibility
    pub public_key: String,
    pub encrypted_private_key: String, // Base64 encoded, p-adic encrypted (v3+)
    #[serde(default)]
    pub salt: String, // Base64 encoded salt for key derivation (v3+)
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub node_id: Option<String>,
}

pub struct NodeWallet {
    keypair: Keypair,
    address: AccountAddress,
    _path: PathBuf,
    _node_id: String,
}

impl NodeWallet {
    /// Derive p-adic encryption key from password using iterative SHA256
    fn derive_encryption_key(password: &str, salt: &[u8]) -> QpDigits {
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
        QpDigits::from_u64(key_val, 3, 32) // p=3 as per paper
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

    /// Create a new wallet and save it
    fn create_and_save(data_dir: &Path, node_id: &str, password: &str) -> Result<Self> {
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

        // Generate salt for key derivation
        let mut salt = vec![0u8; SALT_LEN];
        rand::thread_rng().fill_bytes(&mut salt);

        // Derive encryption key using p-adic crypto
        let encryption_key = Self::derive_encryption_key(password, &salt);

        // Encrypt private key using p-adic encryption
        let padic_crypto = PadicCrypto::new(3, 32); // p=3 as per paper
        let encrypted_key = padic_crypto
            .encrypt(&keypair.to_bytes(), &encryption_key)
            .context("Failed to encrypt private key")?;

        // Create wallet data with both address formats
        let bech32_address = address
            .to_bech32()
            .unwrap_or_else(|_| hex::encode(address.as_bytes()));
        let hex_address = hex::encode(address.as_bytes());

        let wallet_data = WalletData {
            version: 3, // Increment version to indicate encryption
            address: bech32_address,
            hex_address: Some(hex_address),
            public_key: hex::encode(public_key.as_bytes()),
            encrypted_private_key: general_purpose::STANDARD.encode(encrypted_key),
            salt: general_purpose::STANDARD.encode(salt),
            created_at: chrono::Utc::now(),
            node_id: Some(node_id.to_string()),
        };

        // Save wallet data
        let json = serde_json::to_string_pretty(&wallet_data)?;
        fs::write(&wallet_path, json)
            .with_context(|| format!("Failed to write wallet to {:?}", wallet_path))?;

        info!(
            "Created new wallet for node {} with address {}",
            node_id, wallet_data.address
        );

        Ok(Self {
            keypair,
            address,
            _path: wallet_path,
            _node_id: node_id.to_string(),
        })
    }

    /// Load wallet from file
    fn load(path: &Path, node_id: &str, password: &str) -> Result<Self> {
        let json = fs::read_to_string(path)
            .with_context(|| format!("Failed to read wallet from {:?}", path))?;

        let wallet_data: WalletData = serde_json::from_str(&json)
            .with_context(|| format!("Failed to parse wallet from {:?}", path))?;

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

        // Handle both encrypted (v3+) and legacy unencrypted (v2) wallets
        let key_bytes = if wallet_data.version >= 3 {
            // Decode salt
            let salt = general_purpose::STANDARD
                .decode(&wallet_data.salt)
                .context("Failed to decode salt")?;

            // Derive decryption key
            let decryption_key = Self::derive_encryption_key(password, &salt);

            // Decrypt private key using p-adic decryption
            let padic_crypto = PadicCrypto::new(3, 32); // p=3 as per paper
            padic_crypto
                .decrypt(&encrypted_key, &decryption_key)
                .context("Failed to decrypt private key - wrong password?")?
        } else {
            // Legacy unencrypted wallet (v2)
            warn!("Loading legacy unencrypted wallet - please update to encrypted format");
            encrypted_key
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

    /// Export wallet to a file (requires password for encryption)
    pub fn export_to_file(&self, export_path: &Path, password: &str) -> Result<()> {
        // Generate new salt for export
        let mut salt = vec![0u8; SALT_LEN];
        rand::thread_rng().fill_bytes(&mut salt);

        // Derive encryption key
        let encryption_key = Self::derive_encryption_key(password, &salt);

        // Encrypt private key
        let padic_crypto = PadicCrypto::new(3, 32);
        let encrypted_key = padic_crypto
            .encrypt(&self.keypair.to_bytes(), &encryption_key)
            .context("Failed to encrypt private key for export")?;

        // Create wallet data for export
        let wallet_data = WalletData {
            version: 3,
            address: self
                .address
                .to_bech32()
                .unwrap_or_else(|_| hex::encode(self.address.as_bytes())),
            hex_address: Some(hex::encode(self.address.as_bytes())),
            public_key: hex::encode(self.public_key().as_bytes()),
            encrypted_private_key: general_purpose::STANDARD.encode(encrypted_key),
            salt: general_purpose::STANDARD.encode(salt),
            created_at: chrono::Utc::now(),
            node_id: Some(self._node_id.clone()),
        };

        // Save to file
        let json = serde_json::to_string_pretty(&wallet_data)?;
        fs::write(export_path, json)
            .with_context(|| format!("Failed to export wallet to {:?}", export_path))?;

        info!("Exported wallet to {:?}", export_path);
        Ok(())
    }

    /// Import wallet from a file (requires password to decrypt)
    pub fn import_from_file(import_path: &Path, password: &str, node_id: &str) -> Result<Self> {
        info!("Importing wallet from {:?}", import_path);

        // Load the wallet using existing load function
        Self::load(import_path, node_id, password)
    }

    /// Export wallet as encrypted JSON string
    pub fn export_to_json(&self, password: &str) -> Result<String> {
        // Generate new salt for export
        let mut salt = vec![0u8; SALT_LEN];
        rand::thread_rng().fill_bytes(&mut salt);

        // Derive encryption key
        let encryption_key = Self::derive_encryption_key(password, &salt);

        // Encrypt private key
        let padic_crypto = PadicCrypto::new(3, 32);
        let encrypted_key = padic_crypto
            .encrypt(&self.keypair.to_bytes(), &encryption_key)
            .context("Failed to encrypt private key for export")?;

        // Create wallet data
        let wallet_data = WalletData {
            version: 3,
            address: self
                .address
                .to_bech32()
                .unwrap_or_else(|_| hex::encode(self.address.as_bytes())),
            hex_address: Some(hex::encode(self.address.as_bytes())),
            public_key: hex::encode(self.public_key().as_bytes()),
            encrypted_private_key: general_purpose::STANDARD.encode(encrypted_key),
            salt: general_purpose::STANDARD.encode(salt),
            created_at: chrono::Utc::now(),
            node_id: None, // Don't include node_id in exports
        };

        serde_json::to_string(&wallet_data).context("Failed to serialize wallet data")
    }

    /// Import wallet from JSON string
    pub fn import_from_json(json: &str, password: &str, node_id: &str) -> Result<Self> {
        let wallet_data: WalletData =
            serde_json::from_str(json).context("Failed to parse wallet JSON")?;

        // Decode encrypted private key
        let encrypted_key = general_purpose::STANDARD
            .decode(&wallet_data.encrypted_private_key)
            .context("Failed to decode encrypted private key")?;

        // Decrypt private key
        let key_bytes = if wallet_data.version >= 3 {
            let salt = general_purpose::STANDARD
                .decode(&wallet_data.salt)
                .context("Failed to decode salt")?;

            let decryption_key = Self::derive_encryption_key(password, &salt);

            let padic_crypto = PadicCrypto::new(3, 32);
            padic_crypto
                .decrypt(&encrypted_key, &decryption_key)
                .context("Failed to decrypt private key - wrong password?")?
        } else {
            return Err(anyhow::anyhow!("Cannot import legacy unencrypted wallet"));
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
            "Successfully imported wallet with address {}",
            address
                .to_bech32()
                .unwrap_or_else(|_| hex::encode(address.as_bytes()))
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

        // Should fail with decryption error
        assert!(import_result.is_err());
        let err_msg = import_result.err().unwrap().to_string();
        // The p-adic decryption might fail silently and produce wrong bytes,
        // leading to a keypair reconstruction error instead of a decrypt error
        // Just verify that import fails
        assert!(!err_msg.is_empty());
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
