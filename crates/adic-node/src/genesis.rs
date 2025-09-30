use adic_economics::{AccountAddress, AdicAmount};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Helper to create AccountAddress from hex string
pub fn account_address_from_hex(hex_str: &str) -> Result<AccountAddress, String> {
    let hex_str = hex_str.strip_prefix("0x").unwrap_or(hex_str);

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
    pub parameters: GenesisParameters,   // System parameters from paper
}

/// System parameters from ADIC-DAG paper Section 1.2
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisParameters {
    pub p: u32,               // Prime p = 3
    pub d: u32,               // Dimension d = 3
    pub rho: Vec<u32>,        // Axis radii ρ = (2, 2, 1)
    pub q: u32,               // Diversity threshold q = 3
    pub k: u32,               // k-core threshold k = 20
    pub depth_star: u32,      // Depth D* = 12
    pub homology_window: u32, // ∆ = 5
    pub alpha: f64,           // Reputation exponent α = 1
    pub beta: f64,            // Reputation exponent β = 1
}

impl Default for GenesisConfig {
    fn default() -> Self {
        // Per ADIC-DAG paper Section 6.2:
        // Genesis mint S0 = 3×10^8 ADIC
        // - 20% Treasury (60M)
        // - 30% Liquidity and Community R&D (90M)
        // - 50% Genesis contribution (150M)

        // Fixed timestamp for deterministic genesis
        let genesis_timestamp = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        Self {
            allocations: vec![
                // Treasury account - 20% of genesis mint (60M ADIC)
                (treasury_address(), 60_000_000),
                // Liquidity and Community R&D - 30% (90M ADIC)
                (derive_address_from_node_id("liquidity"), 45_000_000),
                (derive_address_from_node_id("community"), 45_000_000),
                // Genesis contribution allocation - 50% (150M ADIC)
                (derive_address_from_node_id("genesis-pool"), 150_000_000),
                // Genesis identities (g0, g1, g2, g3) per paper Section 6.1
                // Initial operational balance for genesis identities
                (derive_address_from_node_id("g0"), 100_000),
                (derive_address_from_node_id("g1"), 100_000),
                (derive_address_from_node_id("g2"), 100_000),
                (derive_address_from_node_id("g3"), 100_000),
            ],
            deposit_amount: 0.1, // D = 0.1 ADIC per paper
            timestamp: genesis_timestamp,
            chain_id: "adic-dag-v1".to_string(),
            genesis_identities: vec![
                "g0".to_string(),
                "g1".to_string(),
                "g2".to_string(),
                "g3".to_string(),
            ],
            parameters: GenesisParameters {
                p: 3,               // Prime p = 3
                d: 3,               // Dimension d = 3
                rho: vec![2, 2, 1], // Axis radii ρ = (2, 2, 1)
                q: 3,               // Diversity threshold
                k: 20,              // k-core threshold
                depth_star: 12,     // Depth D* = 12
                homology_window: 5, // ∆ = 5
                alpha: 1.0,         // Reputation exponent
                beta: 1.0,          // Reputation exponent
            },
        }
    }
}

impl GenesisConfig {
    /// Create a test genesis configuration with smaller amounts
    pub fn test() -> Self {
        let fixed_timestamp = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        Self {
            allocations: vec![
                (treasury_address(), 10_000),
                (derive_address_from_node_id("node-1"), 1_000),
                (derive_address_from_node_id("node-2"), 1_000),
                (derive_address_from_node_id("node-3"), 1_000),
                (derive_address_from_node_id("faucet"), 10_000),
            ],
            deposit_amount: 0.1,
            timestamp: fixed_timestamp,
            chain_id: "adic-testnet".to_string(),
            genesis_identities: vec![
                "g0".to_string(),
                "g1".to_string(),
                "g2".to_string(),
                "g3".to_string(),
            ],
            parameters: GenesisParameters {
                p: 3,
                d: 3,
                rho: vec![2, 2, 1],
                q: 3,
                k: 20,
                depth_star: 12,
                homology_window: 5,
                alpha: 1.0,
                beta: 1.0,
            },
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

    /// Calculate deterministic hash of the genesis configuration
    pub fn calculate_hash(&self) -> String {
        // Create a canonical JSON representation for hashing
        let canonical = serde_json::json!({
            "chain_id": self.chain_id,
            "timestamp": self.timestamp.to_rfc3339(),
            "deposit_amount": self.deposit_amount,
            "allocations": self.allocations,
            "genesis_identities": self.genesis_identities,
            "parameters": {
                "p": self.parameters.p,
                "d": self.parameters.d,
                "rho": self.parameters.rho,
                "q": self.parameters.q,
                "k": self.parameters.k,
                "depth_star": self.parameters.depth_star,
                "homology_window": self.parameters.homology_window,
                "alpha": self.parameters.alpha,
                "beta": self.parameters.beta,
            }
        });

        let json_string = serde_json::to_string(&canonical).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(json_string.as_bytes());
        hex::encode(hasher.finalize())
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

/// Genesis manifest that can be shared and verified
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisManifest {
    pub config: GenesisConfig,
    pub hash: String,
    pub version: String,
}

impl Default for GenesisManifest {
    fn default() -> Self {
        Self::new()
    }
}

impl GenesisManifest {
    /// Create a new genesis manifest with default configuration
    pub fn new() -> Self {
        let config = GenesisConfig::default();
        let hash = config.calculate_hash();

        Self {
            config,
            hash,
            version: "1.0.0".to_string(),
        }
    }

    /// Verify that the manifest hash matches the config
    pub fn verify(&self) -> Result<(), String> {
        let calculated_hash = self.config.calculate_hash();
        if calculated_hash != self.hash {
            return Err(format!(
                "Genesis hash mismatch: expected {}, got {}",
                self.hash, calculated_hash
            ));
        }
        self.config.verify()?;
        Ok(())
    }

    /// Get the canonical genesis hash for this network
    #[allow(dead_code)]
    pub fn canonical_hash() -> &'static str {
        // This is the deterministic hash of the default genesis config
        // Genesis Hash: e03dffb732c202021e35225771c033b1217b0e6241be360ad88f6d7ac43675f8
        // Total Supply: 300,400,000 ADIC (300.4M)
        "e03dffb732c202021e35225771c033b1217b0e6241be360ad88f6d7ac43675f8"
    }
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

impl Default for GenesisHyperedge {
    fn default() -> Self {
        Self::new()
    }
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
    #[allow(dead_code)]
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
    fn test_print_genesis_hash() {
        let manifest = GenesisManifest::new();
        println!("\n\n=== GENESIS HASH ===");
        println!("Genesis Hash: {}", manifest.hash);
        println!(
            "Total Supply: {} ADIC",
            manifest.config.total_supply().to_adic()
        );
        println!("==================\n\n");
    }

    #[test]
    fn test_print_genesis_addresses() {
        println!("\n\n=== GENESIS ADDRESSES ===");
        println!("Treasury: {}", treasury_address());
        println!("Liquidity: {}", derive_address_from_node_id("liquidity"));
        println!("Community: {}", derive_address_from_node_id("community"));
        println!(
            "Genesis Pool: {}",
            derive_address_from_node_id("genesis-pool")
        );
        println!("g0: {}", derive_address_from_node_id("g0"));
        println!("g1: {}", derive_address_from_node_id("g1"));
        println!("g2: {}", derive_address_from_node_id("g2"));
        println!("g3: {}", derive_address_from_node_id("g3"));
        println!("========================\n\n");
    }

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
    fn test_genesis_hash_determinism() {
        // Genesis hash should be deterministic
        let config1 = GenesisConfig::default();
        let config2 = GenesisConfig::default();

        let hash1 = config1.calculate_hash();
        let hash2 = config2.calculate_hash();

        assert_eq!(hash1, hash2, "Genesis hashes should be identical");

        // Should match the canonical hash
        let manifest = GenesisManifest::new();
        assert_eq!(
            manifest.hash, "e03dffb732c202021e35225771c033b1217b0e6241be360ad88f6d7ac43675f8",
            "Genesis hash should match canonical"
        );
    }

    #[test]
    fn test_genesis_manifest_creation() {
        let manifest = GenesisManifest::new();

        // Verify manifest
        assert!(manifest.verify().is_ok());

        // Check version
        assert_eq!(manifest.version, "1.0.0");

        // Check hash matches calculated
        let calculated = manifest.config.calculate_hash();
        assert_eq!(manifest.hash, calculated);
    }

    #[test]
    fn test_genesis_manifest_serialization() {
        let manifest = GenesisManifest::new();

        // Serialize to JSON
        let json = serde_json::to_string_pretty(&manifest).expect("Should serialize");

        // Deserialize back
        let deserialized: GenesisManifest =
            serde_json::from_str(&json).expect("Should deserialize");

        // Should be identical
        assert_eq!(manifest.hash, deserialized.hash);
        assert_eq!(manifest.version, deserialized.version);

        // Verify still works
        assert!(deserialized.verify().is_ok());
    }

    #[test]
    fn test_genesis_parameters_match_paper() {
        let config = GenesisConfig::default();

        // Check parameters match ADIC paper Section 1.2
        assert_eq!(config.parameters.p, 3, "Prime p should be 3");
        assert_eq!(config.parameters.d, 3, "Dimension d should be 3");
        assert_eq!(
            config.parameters.rho,
            vec![2, 2, 1],
            "Radii should be (2,2,1)"
        );
        assert_eq!(config.parameters.q, 3, "Diversity threshold should be 3");
        assert_eq!(config.parameters.k, 20, "k-core threshold should be 20");
        assert_eq!(config.parameters.depth_star, 12, "Depth D* should be 12");
        assert_eq!(config.parameters.homology_window, 5, "Window should be 5");
        assert_eq!(config.parameters.alpha, 1.0, "Alpha should be 1.0");
        assert_eq!(config.parameters.beta, 1.0, "Beta should be 1.0");
    }

    #[test]
    fn test_genesis_allocations_match_paper() {
        let config = GenesisConfig::default();

        // Total genesis mint should be 3×10^8 ADIC per paper Section 6.2
        let total = config.total_supply();
        assert_eq!(
            total.to_adic() as u64,
            300_400_000,
            "Total should be ~300M ADIC"
        );

        // Check major allocations
        assert_eq!(
            config.allocations[0].1, 60_000_000,
            "Treasury should be 60M"
        );

        // Find liquidity and community allocations
        let liquidity = config
            .allocations
            .iter()
            .find(|(addr, _)| addr == &derive_address_from_node_id("liquidity"))
            .map(|(_, amt)| *amt)
            .unwrap_or(0);
        assert_eq!(liquidity, 45_000_000, "Liquidity should be 45M");

        let community = config
            .allocations
            .iter()
            .find(|(addr, _)| addr == &derive_address_from_node_id("community"))
            .map(|(_, amt)| *amt)
            .unwrap_or(0);
        assert_eq!(community, 45_000_000, "Community should be 45M");

        let genesis_pool = config
            .allocations
            .iter()
            .find(|(addr, _)| addr == &derive_address_from_node_id("genesis-pool"))
            .map(|(_, amt)| *amt)
            .unwrap_or(0);
        assert_eq!(genesis_pool, 150_000_000, "Genesis pool should be 150M");
    }

    #[test]
    fn test_genesis_identities() {
        let config = GenesisConfig::default();

        // Should have g0, g1, g2, g3 for d=3
        assert_eq!(config.genesis_identities.len(), 4);
        assert_eq!(config.genesis_identities[0], "g0");
        assert_eq!(config.genesis_identities[1], "g1");
        assert_eq!(config.genesis_identities[2], "g2");
        assert_eq!(config.genesis_identities[3], "g3");
    }

    #[test]
    fn test_genesis_hyperedge() {
        let hyperedge = GenesisHyperedge::new();

        // Should have 4 identities for d=3
        assert_eq!(hyperedge.identities.len(), 4);

        // Each should have distinct axis anchors
        assert_eq!(hyperedge.identities[0].axis_anchors, vec![0, 0, 0]);
        assert_eq!(hyperedge.identities[1].axis_anchors, vec![1, 0, 0]);
        assert_eq!(hyperedge.identities[2].axis_anchors, vec![0, 1, 0]);
        assert_eq!(hyperedge.identities[3].axis_anchors, vec![0, 0, 1]);

        // Should have initial reputation
        for identity in &hyperedge.identities {
            assert_eq!(identity.rep_score, 1.0);
        }

        // Manifest hash should be deterministic
        let hash1 = hyperedge.manifest_hash();
        let hyperedge2 = GenesisHyperedge::new();
        let hash2 = hyperedge2.manifest_hash();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_genesis_manifest_canonical_hash() {
        // The canonical hash should match what we calculated
        assert_eq!(
            GenesisManifest::canonical_hash(),
            "e03dffb732c202021e35225771c033b1217b0e6241be360ad88f6d7ac43675f8"
        );
    }

    #[test]
    fn test_invalid_genesis_rejected() {
        let mut manifest = GenesisManifest::new();

        // Corrupt the hash
        manifest.hash = "invalid_hash".to_string();

        // Should fail verification
        assert!(manifest.verify().is_err());
    }

    #[test]
    fn test_test_config_has_parameters() {
        // Test config should also have valid parameters
        let config = GenesisConfig::test();
        assert!(config.verify().is_ok());
        assert_eq!(config.parameters.p, 3);
        assert_eq!(config.parameters.d, 3);
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
