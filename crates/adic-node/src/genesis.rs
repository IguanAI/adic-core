use adic_economics::{AccountAddress, AdicAmount};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Helper to create AccountAddress from hex string
pub fn account_address_from_hex(hex_str: &str) -> Result<AccountAddress, String> {
    let hex_str = if hex_str.starts_with("0x") {
        &hex_str[2..]
    } else {
        hex_str
    };

    let bytes = hex::decode(hex_str).map_err(|e| format!("Invalid hex: {}", e))?;

    if bytes.len() != 32 {
        return Err(format!("Address must be 32 bytes, got {}", bytes.len()));
    }

    let mut addr_bytes = [0u8; 32];
    addr_bytes.copy_from_slice(&bytes);
    Ok(AccountAddress::from_bytes(addr_bytes))
}

/// Genesis configuration based on ADIC-DAG paper specifications
/// Total supply: 10^9 ADIC
/// Genesis mint: 3×10^8 ADIC
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisConfig {
    pub allocations: Vec<(String, u64)>, // (address_hex, amount_in_adic)
    pub deposit_amount: f64,             // in ADIC (0.1 per paper)
    pub timestamp: DateTime<Utc>,
    pub chain_id: String,
    pub genesis_identities: Vec<String>, // g0, g1, g2, g3 for d=3
}

impl Default for GenesisConfig {
    fn default() -> Self {
        // Per ADIC-DAG paper Section 6.2:
        // Genesis mint S0 = 3×10^8 ADIC
        // - 20% Treasury (60M)
        // - 30% Liquidity and Community R&D (90M)
        // - 50% Genesis contribution (150M)

        Self {
            allocations: vec![
                // Treasury account - 20% of genesis mint (60M ADIC)
                (treasury_address(), 60_000_000),
                // Liquidity and Community R&D - 30% (90M ADIC)
                (derive_address_from_node_id("liquidity"), 45_000_000),
                (derive_address_from_node_id("community"), 45_000_000),
                // Genesis contribution allocation - 50% (150M ADIC)
                // This will be distributed based on genesis contributions
                (derive_address_from_node_id("genesis-pool"), 150_000_000),
                // Genesis identities (g0, g1, g2, g3) with initial ADIC-Rep
                // Small initial balance for operations
                (derive_address_from_node_id("g0"), 1_000),
                (derive_address_from_node_id("g1"), 1_000),
                (derive_address_from_node_id("g2"), 1_000),
                (derive_address_from_node_id("g3"), 1_000),
                // Bootstrap nodes - operational allocations
                // Using actual node addresses from running containers
                (
                    "71ef6e73551963bfc2c0e0bac722718c92626aa734d2fbcb99fba2e2361a153c".to_string(),
                    10_000,
                ),
                (
                    "5db5d180d9bc1ca7aef5afe07ffbeb4cd140fa84e8aa596ac67d16c053ea0692".to_string(),
                    10_000,
                ),
                (
                    "7b22a73f1b3a1768d2b806e0af4bed84aecf42c2d6e359d3453df183303b7385".to_string(),
                    10_000,
                ),
                (
                    "db582489535d0a0acf1e1a949c663036a2a69e485bbfda1a0c58b17895452687".to_string(),
                    10_000,
                ),
                (
                    "9ddeea748bfab933c09846d78e90debe62164f388a2d0011d40ab70da6eb034b".to_string(),
                    10_000,
                ),
                (
                    "ae53b338cd7d2bbef4fca6e114acb8ce8cb865c46f0245b589443d1ffce1b120".to_string(),
                    10_000,
                ),
                // Development faucet for testing
                (derive_address_from_node_id("faucet"), 100_000),
            ],
            deposit_amount: 0.1, // D = 0.1 ADIC per paper
            timestamp: Utc::now(),
            chain_id: "adic-dag-v1".to_string(),
            genesis_identities: vec![
                "g0".to_string(),
                "g1".to_string(),
                "g2".to_string(),
                "g3".to_string(),
            ],
        }
    }
}

impl GenesisConfig {
    /// Create a test genesis configuration with smaller amounts
    pub fn test() -> Self {
        Self {
            allocations: vec![
                (treasury_address(), 10_000),
                (derive_address_from_node_id("node-1"), 1_000),
                (derive_address_from_node_id("node-2"), 1_000),
                (derive_address_from_node_id("node-3"), 1_000),
                (derive_address_from_node_id("faucet"), 10_000),
            ],
            deposit_amount: 0.1,
            timestamp: Utc::now(),
            chain_id: "adic-testnet".to_string(),
            genesis_identities: vec![
                "g0".to_string(),
                "g1".to_string(),
                "g2".to_string(),
                "g3".to_string(),
            ],
        }
    }

    /// Convert allocations to AccountAddress and AdicAmount
    #[allow(dead_code)]
    pub fn get_allocations(&self) -> Vec<(AccountAddress, AdicAmount)> {
        self.allocations
            .iter()
            .map(|(addr_str, amount)| {
                let addr = account_address_from_hex(addr_str)
                    .unwrap_or_else(|_| AccountAddress::from_bytes([0; 32]));
                let amount = AdicAmount::from_adic(*amount as f64);
                (addr, amount)
            })
            .collect()
    }

    /// Get the total supply from genesis allocations
    pub fn total_supply(&self) -> AdicAmount {
        self.allocations
            .iter()
            .map(|(_, amount)| AdicAmount::from_adic(*amount as f64))
            .fold(AdicAmount::ZERO, |acc, amt| acc.saturating_add(amt))
    }

    /// Verify that total supply doesn't exceed maximum
    pub fn verify(&self) -> Result<(), String> {
        let total = self.total_supply();
        if total > AdicAmount::MAX_SUPPLY {
            return Err(format!(
                "Total genesis supply {} exceeds maximum {}",
                total,
                AdicAmount::MAX_SUPPLY
            ));
        }

        // Check for duplicate addresses
        let mut seen = HashMap::new();
        for (addr, amount) in &self.allocations {
            if let Some(_prev) = seen.insert(addr.clone(), amount) {
                return Err(format!("Duplicate address in genesis: {}", addr));
            }
        }

        Ok(())
    }
}

/// Derive a deterministic address from a node ID
pub fn derive_address_from_node_id(node_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("adic-genesis-{}", node_id).as_bytes());
    let hash = hasher.finalize();

    // Take first 32 bytes for address
    let mut addr_bytes = [0u8; 32];
    addr_bytes.copy_from_slice(&hash[..32]);

    hex::encode(addr_bytes)
}

/// Get the treasury address (special well-known address)
pub fn treasury_address() -> String {
    // Treasury uses all zeros except first byte is 1
    let mut addr = [0u8; 32];
    addr[0] = 1;
    hex::encode(addr)
}

/// Derive a deterministic keypair from a seed (for node wallets)
pub fn derive_keypair_from_seed(seed: &str) -> adic_crypto::Keypair {
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    let hash = hasher.finalize();

    // Use the hash as seed for keypair generation
    adic_crypto::Keypair::from_bytes(&hash).expect("Failed to create keypair from seed")
}

/// Genesis hyperedge structure as per ADIC-DAG paper
/// Consists of d+1 system identities (g0, g1, g2, g3 for d=3)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisHyperedge {
    pub identities: Vec<GenesisIdentity>,
    pub deposit_pool: u64, // ADIC locked for bootstrap refunds
    pub rseed: f64,        // Initial ADIC-Rep score
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisIdentity {
    pub id: String,             // g0, g1, g2, g3
    pub address: String,        // Deterministic address
    pub public_key: String,     // Public key hex
    pub rep_score: f64,         // Initial ADIC-Rep
    pub axis_anchors: Vec<u64>, // P-adic anchors per axis
}

impl GenesisHyperedge {
    /// Create the genesis hyperedge for d=3
    pub fn new() -> Self {
        let rseed = 1.0; // Initial reputation score
        let deposit_pool = 10_000; // Bootstrap deposit pool

        let mut identities = Vec::new();

        // Create d+1 genesis identities (g0, g1, g2, g3)
        for i in 0..4 {
            let id = format!("g{}", i);
            let keypair = derive_keypair_from_seed(&format!("adic-genesis-{}", id));
            let address = derive_address_from_node_id(&id);

            // Set distinct p-adic anchors for diversity
            // Each genesis identity anchors different p-adic balls
            let axis_anchors = match i {
                0 => vec![0, 0, 0], // Origin in all axes
                1 => vec![1, 0, 0], // First axis offset
                2 => vec![0, 1, 0], // Second axis offset
                3 => vec![0, 0, 1], // Third axis offset
                _ => vec![0, 0, 0],
            };

            identities.push(GenesisIdentity {
                id: id.clone(),
                address,
                public_key: hex::encode(keypair.public_key().as_bytes()),
                rep_score: rseed,
                axis_anchors,
            });
        }

        Self {
            identities,
            deposit_pool,
            rseed,
        }
    }

    /// Get the genesis manifest hash for anchoring
    pub fn manifest_hash(&self) -> String {
        let mut hasher = Sha256::new();
        let json = serde_json::to_string(self).unwrap_or_default();
        hasher.update(json.as_bytes());
        let hash = hasher.finalize();
        hex::encode(hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_genesis_config() {
        let config = GenesisConfig::default();

        // Verify total supply
        assert!(config.verify().is_ok());

        // Check allocations
        let allocations = config.get_allocations();
        assert!(!allocations.is_empty());

        // Verify treasury is first
        assert_eq!(config.allocations[0].0, treasury_address());
    }

    #[test]
    fn test_deterministic_addresses() {
        // Addresses should be deterministic
        let addr1 = derive_address_from_node_id("node-1");
        let addr2 = derive_address_from_node_id("node-1");
        assert_eq!(addr1, addr2);

        // Different nodes should have different addresses
        let addr3 = derive_address_from_node_id("node-2");
        assert_ne!(addr1, addr3);
    }

    #[test]
    fn test_treasury_address() {
        let treasury = treasury_address();
        // Should start with "01" followed by zeros
        assert!(treasury.starts_with("01"));
        assert_eq!(treasury.len(), 64); // 32 bytes = 64 hex chars
    }

    #[test]
    fn test_genesis_hyperedge_creation() {
        let genesis = GenesisHyperedge::new();

        // Verify genesis hyperedge properties
        assert_eq!(genesis.identities.len(), 4); // d+1 identities for d=3
        assert_eq!(genesis.rseed, 1.0);
        assert_eq!(genesis.deposit_pool, 10_000);

        // Verify each identity
        for (i, identity) in genesis.identities.iter().enumerate() {
            assert_eq!(identity.id, format!("g{}", i));
            assert!(!identity.address.is_empty());
            assert!(!identity.public_key.is_empty());
            assert_eq!(identity.rep_score, 1.0);
            assert_eq!(identity.axis_anchors.len(), 3);
        }
    }

    #[test]
    fn test_genesis_manifest_hash() {
        let genesis = GenesisHyperedge::new();
        let hash = genesis.manifest_hash();

        // Hash should be deterministic
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64); // SHA256 = 32 bytes = 64 hex chars

        // Creating another genesis should produce same hash
        let genesis2 = GenesisHyperedge::new();
        let hash2 = genesis2.manifest_hash();
        assert_eq!(hash, hash2);
    }
}
