//! Unified configuration loader for ADIC networks
//!
//! This module provides a single entry point for loading network-specific configurations.
//! Each network (mainnet, testnet, devnet) has its own authoritative configuration file
//! that contains all consensus parameters, economic settings, and network topology.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::info;

/// Complete network configuration loaded from .toml files
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UnifiedConfig {
    pub network: NetworkConfig,
    pub consensus: ConsensusConfig,
    pub api: ApiConfig,
    pub storage: StorageConfig,
    pub logging: LoggingConfig,
    pub genesis: GenesisConfig,
    pub node: NodeConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NetworkConfig {
    pub name: String,
    pub chain_id: String,
    pub p2p_port: u16,
    pub quic_port: u16,
    pub max_peers: usize,
    pub dns_seeds: Vec<String>,
    pub bootstrap_peers: Vec<String>,
    pub use_production_tls: bool,
    pub ca_cert_path: String,
    pub node_cert_path: String,
    pub node_key_path: String,
    #[serde(default)]
    pub auto_update: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConsensusConfig {
    // Core p-adic parameters
    pub p: u32,
    pub d: u32,
    pub rho: Vec<u32>,
    pub q: u32,

    // Finality parameters
    pub k: u32,
    pub depth_star: u32,
    pub r_min: f64,
    pub r_sum_min: f64,
    pub delta: u32,
    pub epsilon: f64,

    // Economic parameters
    pub deposit: f64,
    pub lambda: f64,
    pub beta: f64,
    pub mu: f64,
    pub gamma: f64,

    // Additional parameters
    pub alpha: f64,
    #[serde(default = "default_beta_adm")]
    pub beta_adm: f64,

    // Epoch duration for time-based operations
    #[serde(default = "default_epoch_duration_secs_unified")]
    pub epoch_duration_secs: u64,
}

fn default_beta_adm() -> f64 {
    1.0
}

fn default_epoch_duration_secs_unified() -> u64 {
    3600 // 1 hour epochs by default
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub max_connections: usize,
    #[serde(default = "default_rate_limit")]
    pub rate_limit_requests_per_minute: u32,
}

fn default_rate_limit() -> u32 {
    60
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    pub backend: String,
    pub cache_size: usize,
    pub snapshot_interval: u64,
    pub max_snapshots: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
    pub use_emojis: bool,
    pub show_fields_at: String,
    pub show_boot_banner: bool,
    pub show_emoji_legend: bool,
    pub file_rotation_size_mb: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GenesisConfig {
    pub timestamp: String,
    pub deposit_amount: f64,
    pub genesis_identities: Vec<String>,
    pub allocations: Vec<(String, u64)>,
    pub parameters: GenesisParameters,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GenesisParameters {
    pub p: u32,
    pub d: u32,
    pub rho: Vec<u32>,
    pub q: u32,
    pub k: u32,
    pub depth_star: u32,
    pub homology_window: u32,
    pub alpha: f64,
    pub beta: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NodeConfig {
    pub data_dir: String,
    pub validator: bool,
    pub bootstrap: bool,
    pub name: String,
}

impl UnifiedConfig {
    /// Load configuration for the specified network
    ///
    /// Tries to load from `config/{network}.toml` first, falls back to embedded
    /// configuration if file doesn't exist.
    pub fn load(network: &str) -> Result<Self> {
        info!(network = network, "Loading network configuration");

        // Validate network name
        match network {
            "mainnet" | "testnet" | "devnet" => {}
            _ => {
                anyhow::bail!(
                    "Invalid network '{}'. Valid options: mainnet, testnet, devnet",
                    network
                );
            }
        }

        // Try to load from file system first
        let config_path = PathBuf::from(format!("config/{}.toml", network));

        let config_contents = if config_path.exists() {
            info!(path = %config_path.display(), "Loading configuration from file");
            std::fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read config file: {}", config_path.display()))?
        } else {
            // Fallback to embedded configuration
            info!("Configuration file not found, using embedded defaults");
            match network {
                "mainnet" => include_str!("../../../config/mainnet.toml").to_string(),
                "testnet" => include_str!("../../../config/testnet.toml").to_string(),
                "devnet" => include_str!("../../../config/devnet.toml").to_string(),
                _ => unreachable!(),
            }
        };

        let mut config: UnifiedConfig = toml::from_str(&config_contents)
            .with_context(|| format!("Failed to parse {} configuration", network))?;

        // Apply environment variable overrides (runtime settings only)
        config.apply_env_overrides();

        // Validate configuration
        config.validate()?;

        // SECURITY: Additional validation for mainnet genesis configuration
        config.validate_genesis_security()?;

        info!(
            chain_id = %config.network.chain_id,
            p2p_port = config.network.p2p_port,
            api_port = config.api.port,
            "Configuration loaded successfully"
        );

        Ok(config)
    }

    /// Apply environment variable overrides for runtime settings
    ///
    /// Only runtime settings (ports, paths, log levels) can be overridden.
    /// Consensus parameters are immutable and must be changed in config files.
    fn apply_env_overrides(&mut self) {
        // API port override
        if let Ok(port) = std::env::var("API_PORT") {
            if let Ok(p) = port.parse::<u16>() {
                info!(
                    old = self.api.port,
                    new = p,
                    "Overriding API port from environment"
                );
                self.api.port = p;
            }
        }

        // P2P port override
        if let Ok(port) = std::env::var("P2P_PORT") {
            if let Ok(p) = port.parse::<u16>() {
                info!(
                    old = self.network.p2p_port,
                    new = p,
                    "Overriding P2P port from environment"
                );
                self.network.p2p_port = p;
            }
        }

        // Data directory override
        if let Ok(data_dir) = std::env::var("DATA_DIR") {
            info!(old = %self.node.data_dir, new = %data_dir, "Overriding data directory from environment");
            self.node.data_dir = data_dir;
        }

        // Log level override
        if let Ok(log_level) = std::env::var("LOG_LEVEL") {
            info!(old = %self.logging.level, new = %log_level, "Overriding log level from environment");
            self.logging.level = log_level;
        }

        // API host override
        if let Ok(host) = std::env::var("API_HOST") {
            info!(old = %self.api.host, new = %host, "Overriding API host from environment");
            self.api.host = host;
        }

        // Node name override
        if let Ok(name) = std::env::var("NODE_NAME") {
            info!(old = %self.node.name, new = %name, "Overriding node name from environment");
            self.node.name = name;
        }

        // Bootstrap mode override
        if let Ok(bootstrap) = std::env::var("BOOTSTRAP") {
            if let Ok(is_bootstrap) = bootstrap.parse::<bool>() {
                info!(old = self.node.bootstrap, new = is_bootstrap, "Overriding bootstrap mode from environment");
                self.node.bootstrap = is_bootstrap;
            }
        }

        // Bootstrap peers override (comma-separated)
        if let Ok(peers) = std::env::var("BOOTSTRAP_PEERS") {
            if !peers.is_empty() {
                let peer_list: Vec<String> =
                    peers.split(',').map(|s| s.trim().to_string()).collect();
                info!(
                    count = peer_list.len(),
                    "Overriding bootstrap peers from environment"
                );
                self.network.bootstrap_peers = peer_list;
            }
        }
    }

    /// Validate configuration parameters
    fn validate(&self) -> Result<()> {
        // Validate p is prime (2 or 3 supported)
        if self.consensus.p != 2 && self.consensus.p != 3 {
            anyhow::bail!("Invalid prime p={}, must be 2 or 3", self.consensus.p);
        }

        // Validate dimension
        if self.consensus.d == 0 || self.consensus.d > 10 {
            anyhow::bail!("Invalid dimension d={}, must be 1-10", self.consensus.d);
        }

        // Validate rho length matches dimension
        let expected_len = self.consensus.d as usize;
        if self.consensus.rho.len() != expected_len {
            anyhow::bail!(
                "rho vector length {} doesn't match dimension d={}",
                self.consensus.rho.len(),
                expected_len
            );
        }

        // Validate finality parameters
        if self.consensus.k == 0 {
            anyhow::bail!("k-core parameter k must be > 0");
        }

        if self.consensus.depth_star == 0 {
            anyhow::bail!("Depth parameter depth_star must be > 0");
        }

        if self.consensus.q == 0 {
            anyhow::bail!("Diversity parameter q must be > 0");
        }

        // Validate economic parameters
        if self.consensus.deposit < 0.0 {
            anyhow::bail!("Deposit amount must be non-negative");
        }

        if self.consensus.gamma <= 0.0 || self.consensus.gamma >= 1.0 {
            anyhow::bail!("Reputation decay gamma must be in (0, 1)");
        }

        // Validate genesis parameters match consensus parameters
        if self.genesis.parameters.p != self.consensus.p {
            anyhow::bail!(
                "Genesis p={} doesn't match consensus p={}",
                self.genesis.parameters.p,
                self.consensus.p
            );
        }

        if self.genesis.parameters.d != self.consensus.d {
            anyhow::bail!(
                "Genesis d={} doesn't match consensus d={}",
                self.genesis.parameters.d,
                self.consensus.d
            );
        }

        if self.genesis.parameters.k != self.consensus.k {
            anyhow::bail!(
                "Genesis k={} doesn't match consensus k={}",
                self.genesis.parameters.k,
                self.consensus.k
            );
        }

        Ok(())
    }

    /// Validate genesis configuration security (defense-in-depth)
    /// This provides early warning if genesis config has been tampered with
    fn validate_genesis_security(&self) -> Result<()> {
        use sha2::{Digest, Sha256};

        // For mainnet, verify the genesis allocations match canonical
        if self.network.chain_id == "adic-dag-v1" {
            // Calculate hash of the genesis configuration
            let canonical_json = serde_json::json!({
                "chain_id": self.genesis.allocations,
                "timestamp": self.genesis.timestamp,
                "deposit_amount": self.genesis.deposit_amount,
                "allocations": self.genesis.allocations,
                "genesis_identities": self.genesis.genesis_identities,
                "parameters": {
                    "p": self.genesis.parameters.p,
                    "d": self.genesis.parameters.d,
                    "rho": self.genesis.parameters.rho,
                    "q": self.genesis.parameters.q,
                    "k": self.genesis.parameters.k,
                    "depth_star": self.genesis.parameters.depth_star,
                    "homology_window": self.genesis.parameters.homology_window,
                    "alpha": self.genesis.parameters.alpha,
                    "beta": self.genesis.parameters.beta,
                }
            });

            let json_string = serde_json::to_string(&canonical_json)?;
            let mut hasher = Sha256::new();
            hasher.update(json_string.as_bytes());
            let calculated_hash = hex::encode(hasher.finalize());

            // This is the canonical hash for mainnet
            let canonical_hash = "e03dffb732c202021e35225771c033b1217b0e6241be360ad88f6d7ac43675f8";

            if calculated_hash != canonical_hash {
                tracing::warn!(
                    calculated = %calculated_hash,
                    canonical = %canonical_hash,
                    "⚠️  SECURITY WARNING: Mainnet configuration has non-canonical genesis allocations. \
                     This configuration will be rejected when starting a bootstrap node. \
                     If you are seeing this for official mainnet, the configuration file may have been tampered with."
                );
            }
        }

        Ok(())
    }

    /// Get the network name
    pub fn network_name(&self) -> &str {
        &self.network.name
    }

    /// Get the chain ID
    pub fn chain_id(&self) -> &str {
        &self.network.chain_id
    }

    /// Convert UnifiedConfig to the legacy NodeConfig format for compatibility
    ///
    /// This consumes self because the conversion owns the config data
    #[allow(clippy::wrong_self_convention)]
    pub fn to_node_config(self) -> crate::config::NodeConfig {
        use chrono::DateTime;

        // Convert timestamp string to DateTime<Utc>
        let timestamp = DateTime::parse_from_rfc3339(&self.genesis.timestamp)
            .expect("Invalid timestamp format in genesis config")
            .with_timezone(&chrono::Utc);

        // Convert GenesisConfig from config_loader format to genesis format
        let genesis_config = crate::genesis::GenesisConfig {
            allocations: self.genesis.allocations,
            deposit_amount: self.genesis.deposit_amount,
            timestamp,
            chain_id: self.network.chain_id.clone(),
            genesis_identities: self.genesis.genesis_identities,
            parameters: crate::genesis::GenesisParameters {
                p: self.genesis.parameters.p,
                d: self.genesis.parameters.d,
                rho: self.genesis.parameters.rho,
                q: self.genesis.parameters.q,
                k: self.genesis.parameters.k,
                depth_star: self.genesis.parameters.depth_star,
                homology_window: self.genesis.parameters.homology_window,
                alpha: self.genesis.parameters.alpha,
                beta: self.genesis.parameters.beta,
            },
        };

        crate::config::NodeConfig {
            node: crate::config::NodeSettings {
                data_dir: std::path::PathBuf::from(self.node.data_dir),
                keypair_path: None, // Not specified in unified config
                validator: self.node.validator,
                name: self.node.name,
                bootstrap: Some(self.node.bootstrap),
            },
            consensus: crate::config::ConsensusConfig {
                p: self.consensus.p,
                d: self.consensus.d,
                rho: self.consensus.rho,
                q: self.consensus.q,
                k: self.consensus.k,
                depth_star: self.consensus.depth_star,
                delta: self.consensus.delta,
                r_sum_min: self.consensus.r_sum_min,
                r_min: self.consensus.r_min,
                deposit: self.consensus.deposit,
                lambda: self.consensus.lambda,
                alpha: self.consensus.alpha,
                beta: self.consensus.beta,
                mu: self.consensus.mu,
                gamma: self.consensus.gamma,
                epoch_duration_secs: self.consensus.epoch_duration_secs,
            },
            storage: crate::config::StorageConfig {
                backend: self.storage.backend,
                cache_size: self.storage.cache_size,
                snapshot_interval: self.storage.snapshot_interval,
                max_snapshots: self.storage.max_snapshots,
            },
            api: crate::config::ApiConfig {
                enabled: self.api.enabled,
                host: self.api.host,
                port: self.api.port,
                max_connections: self.api.max_connections,
            },
            network: crate::config::NetworkConfig {
                enabled: true, // Always enabled when using network configs
                p2p_port: self.network.p2p_port,
                quic_port: self.network.quic_port,
                bootstrap_peers: self.network.bootstrap_peers,
                dns_seeds: self.network.dns_seeds,
                max_peers: self.network.max_peers,
                use_production_tls: self.network.use_production_tls,
                ca_cert_path: if self.network.ca_cert_path.is_empty() {
                    None
                } else {
                    Some(self.network.ca_cert_path)
                },
                node_cert_path: if self.network.node_cert_path.is_empty() {
                    None
                } else {
                    Some(self.network.node_cert_path)
                },
                node_key_path: if self.network.node_key_path.is_empty() {
                    None
                } else {
                    Some(self.network.node_key_path)
                },
                auto_update: self.network.auto_update,
                asn: None,
                region: None,
            },
            logging: crate::config::LoggingConfig {
                level: self.logging.level,
                format: self.logging.format,
                use_emojis: self.logging.use_emojis,
                show_fields_at: self.logging.show_fields_at,
                file_output: None, // Not specified in unified config
                file_rotation_size_mb: self.logging.file_rotation_size_mb,
                show_boot_banner: self.logging.show_boot_banner,
                show_emoji_legend: self.logging.show_emoji_legend,
                module_filters: std::collections::HashMap::new(),
                field_filters: std::collections::HashMap::new(),
            },
            genesis: Some(genesis_config),
            applications: crate::config::ApplicationsConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_mainnet_config() {
        let config = UnifiedConfig::load("mainnet").expect("Failed to load mainnet config");
        assert_eq!(config.network.name, "mainnet");
        assert_eq!(config.network.chain_id, "adic-dag-v1");
        assert_eq!(config.consensus.p, 3);
        assert_eq!(config.consensus.d, 3);
        assert_eq!(config.consensus.k, 20);
        assert_eq!(config.consensus.depth_star, 12);
        assert_eq!(config.consensus.q, 3);
    }

    #[test]
    fn test_load_testnet_config() {
        let config = UnifiedConfig::load("testnet").expect("Failed to load testnet config");
        assert_eq!(config.network.name, "testnet");
        assert_eq!(config.network.chain_id, "adic-testnet");
        assert_eq!(config.consensus.k, 20);
        assert_eq!(config.consensus.r_min, 0.5); // Lower than mainnet
    }

    #[test]
    fn test_load_devnet_config() {
        let config = UnifiedConfig::load("devnet").expect("Failed to load devnet config");
        assert_eq!(config.network.name, "devnet");
        assert_eq!(config.network.chain_id, "adic-devnet");
        assert_eq!(config.consensus.k, 2); // Minimal for fast testing
        assert_eq!(config.consensus.depth_star, 2);
    }

    #[test]
    fn test_invalid_network() {
        let result = UnifiedConfig::load("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_config_validation() {
        let config = UnifiedConfig::load("mainnet").unwrap();
        assert!(config.validate().is_ok());
    }
}
