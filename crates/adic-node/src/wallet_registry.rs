use adic_economics::AccountAddress;
use adic_types::PublicKey;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Registry for mapping wallet addresses to their public keys
/// This enables verification of signatures from external wallets
#[derive(Clone)]
pub struct WalletRegistry {
    /// In-memory cache of address -> public key mappings
    registry: Arc<RwLock<HashMap<AccountAddress, RegisteredWallet>>>,
    /// Storage backend for persistence
    storage: Arc<adic_storage::StorageEngine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredWallet {
    pub address: AccountAddress,
    pub public_key: PublicKey,
    pub registered_at: chrono::DateTime<chrono::Utc>,
    pub last_used: Option<chrono::DateTime<chrono::Utc>>,
    pub metadata: WalletMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WalletMetadata {
    pub label: Option<String>,
    pub wallet_type: Option<String>, // "hardware", "software", "multisig"
    pub trusted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletRegistrationRequest {
    pub address: String,    // Hex or bech32 address
    pub public_key: String, // Hex encoded public key
    pub signature: String,  // Hex signature of registration message
    pub metadata: Option<WalletMetadata>,
}

impl WalletRegistry {
    /// Create a new wallet registry
    pub fn new(storage: Arc<adic_storage::StorageEngine>) -> Self {
        Self {
            registry: Arc::new(RwLock::new(HashMap::new())),
            storage,
        }
    }

    /// Load registry from storage on startup
    pub async fn load_from_storage(&self) -> Result<()> {
        // Load all registered wallets from storage
        // For now, we'll use a simple key-value approach
        // In production, this would use a proper index

        info!("Loading wallet registry from storage");

        // Storage key prefix for wallet registry
        let _prefix = b"wallet_registry:";

        // Get all wallet registry entries
        // Note: This is simplified - production would use proper indexing
        let _entries = self
            .storage
            .get_messages_since(&adic_types::MessageId::new(&[0; 32]), 1000)
            .await?;

        let mut registry = self.registry.write().await;
        let loaded_count = 0;

        // For now, just initialize empty - proper storage integration would load persisted wallets
        registry.clear();

        info!("Wallet registry initialized with {} wallets", loaded_count);
        Ok(())
    }

    /// Register a new external wallet
    pub async fn register_wallet(&self, request: WalletRegistrationRequest) -> Result<()> {
        let start_time = Instant::now();

        info!(
            operation = "wallet_registration",
            address = %request.address,
            has_metadata = request.metadata.is_some(),
            "🔐 Starting wallet registration"
        );

        // Parse address
        let address = if request.address.starts_with("adic") {
            AccountAddress::from_bech32(&request.address)?
        } else {
            let bytes = hex::decode(&request.address)?;
            if bytes.len() != 32 {
                return Err(anyhow!("Invalid address length"));
            }
            let mut addr_bytes = [0u8; 32];
            addr_bytes.copy_from_slice(&bytes);
            AccountAddress::from_bytes(addr_bytes)
        };

        // Parse public key
        let pubkey_bytes = hex::decode(&request.public_key)?;
        if pubkey_bytes.len() != 32 {
            return Err(anyhow!("Invalid public key length"));
        }
        let mut pk_bytes = [0u8; 32];
        pk_bytes.copy_from_slice(&pubkey_bytes);
        let public_key = PublicKey::from_bytes(pk_bytes);

        // Verify that the public key matches the address
        let derived_address = AccountAddress::from_public_key(&public_key);
        if derived_address != address {
            return Err(anyhow!("Public key does not match address"));
        }

        // Parse and verify signature
        let signature_bytes = hex::decode(&request.signature)?;
        if signature_bytes.len() != 64 {
            return Err(anyhow!("Invalid signature length"));
        }

        // Create registration message that should have been signed
        let registration_message = format!(
            "ADIC Wallet Registration\nAddress: {}\nPublic Key: {}\nTimestamp: {}",
            request.address,
            request.public_key,
            chrono::Utc::now().timestamp()
        );

        // Verify signature using ed25519-dalek directly
        use ed25519_dalek::{Signature as DalekSignature, Verifier, VerifyingKey};

        let verifying_key = VerifyingKey::from_bytes(public_key.as_bytes())
            .map_err(|_| anyhow!("Invalid public key for verification"))?;

        let mut sig_array = [0u8; 64];
        sig_array.copy_from_slice(&signature_bytes);
        let dalek_sig = DalekSignature::from_bytes(&sig_array);

        if verifying_key
            .verify(registration_message.as_bytes(), &dalek_sig)
            .is_err()
        {
            error!(
                operation = "wallet_registration",
                address = %request.address,
                "❌ Failed signature verification during wallet registration"
            );
            return Err(anyhow!("Invalid registration signature"));
        }

        // Create registered wallet entry
        let registered_wallet = RegisteredWallet {
            address,
            public_key,
            registered_at: chrono::Utc::now(),
            last_used: None,
            metadata: request.metadata.unwrap_or_default(),
        };

        // Store in registry
        let mut registry = self.registry.write().await;

        if registry.contains_key(&address) {
            warn!(
                "Wallet {} is already registered, updating registration",
                request.address
            );
        }

        let was_update = registry.contains_key(&address);
        registry.insert(address, registered_wallet.clone());

        let duration_ms = start_time.elapsed().as_millis() as u64;

        // Persist to storage (simplified - production would use proper storage API)
        // For now we just log the registration
        info!(
            operation = "wallet_registration",
            address = %request.address,
            public_key = %request.public_key,
            is_update = was_update,
            trusted = registered_wallet.metadata.trusted,
            wallet_type = ?registered_wallet.metadata.wallet_type,
            duration_ms = duration_ms,
            "✅ Successfully registered external wallet"
        );

        Ok(())
    }

    /// Get public key for an address
    pub async fn get_public_key(&self, address: &AccountAddress) -> Option<PublicKey> {
        let registry = self.registry.read().await;
        registry.get(address).map(|w| w.public_key)
    }

    /// Check if an address is registered
    pub async fn is_registered(&self, address: &AccountAddress) -> bool {
        let registry = self.registry.read().await;
        registry.contains_key(address)
    }

    /// Update last used timestamp for a wallet
    pub async fn mark_used(&self, address: &AccountAddress) -> Result<()> {
        let mut registry = self.registry.write().await;

        if let Some(wallet) = registry.get_mut(address) {
            let now = chrono::Utc::now();
            let previous_used = wallet.last_used;
            wallet.last_used = Some(now);

            debug!(
                operation = "mark_wallet_used",
                address = %hex::encode(address.as_bytes()),
                previous_used = ?previous_used,
                time_since_last_use_hours = previous_used.map(|t| (now - t).num_hours()).unwrap_or(0),
                "Updated wallet last used timestamp"
            );
            Ok(())
        } else {
            debug!(
                operation = "mark_wallet_used",
                address = %hex::encode(address.as_bytes()),
                "Attempted to mark non-existent wallet as used"
            );
            Err(anyhow!("Wallet not found in registry"))
        }
    }

    /// Get wallet information
    pub async fn get_wallet_info(&self, address: &AccountAddress) -> Option<RegisteredWallet> {
        let registry = self.registry.read().await;
        registry.get(address).cloned()
    }

    /// List all registered wallets
    pub async fn list_wallets(&self) -> Vec<RegisteredWallet> {
        let registry = self.registry.read().await;
        registry.values().cloned().collect()
    }

    /// Remove a wallet from registry
    pub async fn unregister_wallet(&self, address: &AccountAddress) -> Result<()> {
        let start_time = Instant::now();
        let address_hex = hex::encode(address.as_bytes());

        info!(
            operation = "wallet_unregistration",
            address = %address_hex,
            "🔐 Starting wallet unregistration"
        );

        let mut registry = self.registry.write().await;

        if let Some(removed_wallet) = registry.remove(address) {
            let duration_ms = start_time.elapsed().as_millis() as u64;

            info!(
                operation = "wallet_unregistration",
                address = %address_hex,
                was_trusted = removed_wallet.metadata.trusted,
                wallet_type = ?removed_wallet.metadata.wallet_type,
                registered_duration_hours = removed_wallet.last_used.map(|t| (t - removed_wallet.registered_at).num_hours()).unwrap_or(0),
                duration_ms = duration_ms,
                "✅ Successfully unregistered wallet"
            );
            Ok(())
        } else {
            warn!(
                operation = "wallet_unregistration",
                address = %address_hex,
                "⚠️ Attempted to unregister non-existent wallet"
            );
            Err(anyhow!("Wallet not found in registry"))
        }
    }

    /// Get registry statistics
    pub async fn get_stats(&self) -> RegistryStats {
        let registry = self.registry.read().await;

        let total_wallets = registry.len();
        let trusted_wallets = registry.values().filter(|w| w.metadata.trusted).count();

        // Count active wallets (used in last 24 hours)
        let now = chrono::Utc::now();
        let active_wallets = registry
            .values()
            .filter(|w| {
                w.last_used
                    .map(|t| (now - t).num_hours() < 24)
                    .unwrap_or(false)
            })
            .count();

        // Find last registration time
        let last_registration = registry.values().map(|w| w.registered_at).max();

        // Count wallet types
        let mut wallet_types = std::collections::HashMap::new();
        for wallet in registry.values() {
            let wallet_type = wallet
                .metadata
                .wallet_type
                .as_deref()
                .unwrap_or("standard")
                .to_string();
            *wallet_types.entry(wallet_type).or_insert(0) += 1;
        }

        RegistryStats {
            total_wallets,
            active_wallets,
            trusted_wallets,
            last_registration,
            wallet_types,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistryStats {
    pub total_wallets: usize,
    pub active_wallets: usize,
    pub trusted_wallets: usize,
    pub last_registration: Option<chrono::DateTime<chrono::Utc>>,
    pub wallet_types: std::collections::HashMap<String, usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_crypto::Keypair;
    use adic_storage::{StorageConfig, StorageEngine};

    fn create_test_storage() -> Arc<StorageEngine> {
        Arc::new(
            StorageEngine::new(StorageConfig {
                backend_type: adic_storage::store::BackendType::Memory,
                ..Default::default()
            })
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn test_wallet_registration() {
        let storage = create_test_storage();
        let registry = WalletRegistry::new(storage);

        // Generate a test keypair
        let keypair = Keypair::generate();
        let public_key = keypair.public_key();
        let address = AccountAddress::from_public_key(public_key);

        // Create registration message
        let registration_message = format!(
            "ADIC Wallet Registration\nAddress: {}\nPublic Key: {}\nTimestamp: {}",
            hex::encode(address.as_bytes()),
            hex::encode(public_key.as_bytes()),
            chrono::Utc::now().timestamp()
        );

        // Sign the message
        let signature = keypair.sign(registration_message.as_bytes());

        // Create registration request
        let request = WalletRegistrationRequest {
            address: hex::encode(address.as_bytes()),
            public_key: hex::encode(public_key.as_bytes()),
            signature: hex::encode(signature.as_bytes()),
            metadata: Some(WalletMetadata {
                label: Some("Test Wallet".to_string()),
                wallet_type: Some("software".to_string()),
                trusted: true,
            }),
        };

        // Register the wallet
        registry.register_wallet(request).await.unwrap();

        // Verify registration
        assert!(registry.is_registered(&address).await);
        assert_eq!(registry.get_public_key(&address).await, Some(*public_key));

        // Check wallet info
        let info = registry.get_wallet_info(&address).await.unwrap();
        assert_eq!(info.address, address);
        assert_eq!(info.public_key, *public_key);
        assert_eq!(info.metadata.label, Some("Test Wallet".to_string()));
    }

    #[tokio::test]
    async fn test_invalid_registration() {
        let storage = create_test_storage();
        let registry = WalletRegistry::new(storage);

        // Generate two different keypairs
        let keypair1 = Keypair::generate();
        let keypair2 = Keypair::generate();

        let public_key1 = keypair1.public_key();
        let address1 = AccountAddress::from_public_key(public_key1);
        let public_key2 = keypair2.public_key();

        // Try to register with mismatched public key and address
        let registration_message = format!(
            "ADIC Wallet Registration\nAddress: {}\nPublic Key: {}\nTimestamp: {}",
            hex::encode(address1.as_bytes()),
            hex::encode(public_key2.as_bytes()), // Wrong public key
            chrono::Utc::now().timestamp()
        );

        let signature = keypair2.sign(registration_message.as_bytes());

        let request = WalletRegistrationRequest {
            address: hex::encode(address1.as_bytes()),
            public_key: hex::encode(public_key2.as_bytes()),
            signature: hex::encode(signature.as_bytes()),
            metadata: None,
        };

        // Should fail due to mismatched address and public key
        let result = registry.register_wallet(request).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not match"));
    }
}
