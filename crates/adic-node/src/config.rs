use adic_types::AdicParams;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use crate::genesis::GenesisConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    pub node: NodeSettings,
    pub consensus: ConsensusConfig,
    pub storage: StorageConfig,
    pub api: ApiConfig,
    pub network: NetworkConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub genesis: Option<GenesisConfig>,
    #[serde(default)]
    pub applications: ApplicationsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSettings {
    pub data_dir: PathBuf,
    pub keypair_path: Option<PathBuf>,
    pub validator: bool,
    pub name: String,
    #[serde(default)]
    pub bootstrap: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusConfig {
    pub p: u32,
    pub d: u32,
    pub rho: Vec<u32>,
    pub q: u32,
    pub k: u32,
    pub depth_star: u32,
    pub delta: u32,
    pub r_sum_min: f64,
    pub r_min: f64,
    pub deposit: f64,
    pub lambda: f64,
    pub alpha: f64,
    pub beta: f64,
    pub mu: f64,
    pub gamma: f64,
    /// Epoch duration in seconds (used for governance, PoUW, and time-based operations)
    #[serde(default = "default_epoch_duration_secs")]
    pub epoch_duration_secs: u64,
}

fn default_epoch_duration_secs() -> u64 {
    3600 // 1 hour epochs by default
}

impl From<ConsensusConfig> for AdicParams {
    fn from(config: ConsensusConfig) -> Self {
        AdicParams {
            p: config.p,
            d: config.d,
            rho: config.rho,
            q: config.q,
            k: config.k,
            depth_star: config.depth_star,
            delta: config.delta,
            r_sum_min: config.r_sum_min,
            r_min: config.r_min,
            deposit: config.deposit,
            lambda: config.lambda,
            alpha: config.alpha,
            beta: config.beta,
            mu: config.mu,
            gamma: config.gamma,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub backend: String,
    pub cache_size: usize,
    pub snapshot_interval: u64,
    pub max_snapshots: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub max_connections: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub enabled: bool,
    pub p2p_port: u16,
    pub quic_port: u16,
    pub bootstrap_peers: Vec<String>,
    pub dns_seeds: Vec<String>,
    pub max_peers: usize,
    pub use_production_tls: bool,
    pub ca_cert_path: Option<String>,
    pub node_cert_path: Option<String>,
    pub node_key_path: Option<String>,
    #[serde(default)]
    pub auto_update: bool,
    /// Optional self-reported ASN for network diversity
    /// Nodes voluntarily share for quorum selection diversity caps
    #[serde(default)]
    pub asn: Option<u32>,
    /// Optional self-reported region for network diversity
    /// ISO 3166-1 alpha-2 code (e.g., "us", "eu") or custom region
    #[serde(default)]
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
    pub use_emojis: bool,
    pub show_fields_at: String, // "always", "debug", "trace", "never"
    pub file_output: Option<PathBuf>,
    pub file_rotation_size_mb: usize,
    pub show_boot_banner: bool,
    pub show_emoji_legend: bool,
    #[serde(default)]
    pub module_filters: HashMap<String, String>,
    #[serde(default)]
    pub field_filters: HashMap<String, bool>, // Which fields to always show/hide
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: "pretty".to_string(),
            use_emojis: true,
            show_fields_at: "debug".to_string(), // Show fields at debug level and above
            file_output: None,
            file_rotation_size_mb: 100,
            show_boot_banner: true,
            show_emoji_legend: true,
            module_filters: HashMap::new(),
            field_filters: HashMap::new(),
        }
    }
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            node: NodeSettings {
                data_dir: PathBuf::from("./data"),
                keypair_path: None,
                validator: false,
                name: "adic-node".to_string(),
                bootstrap: Some(false),
            },
            consensus: ConsensusConfig {
                p: 3,
                d: 3,
                rho: vec![2, 2, 1],
                q: 3,
                k: 20,
                depth_star: 12, // Phase-0 default
                delta: 5,
                r_sum_min: 4.0, // Phase-0 default
                r_min: 1.0,     // Phase-0 default
                deposit: 0.1,   // Per whitepaper: refundable anti-spam deposit
                lambda: 1.0,    // Phase-0 default
                alpha: 1.0,     // MRW reputation exponent, default per PDF Section 1.2
                beta: 0.5,      // Phase-0 default
                mu: 1.0,        // Phase-0 default
                gamma: 0.9,
                epoch_duration_secs: default_epoch_duration_secs(),
            },
            storage: StorageConfig {
                backend: "rocksdb".to_string(),
                cache_size: 10000,
                snapshot_interval: 3600,
                max_snapshots: 10,
            },
            api: ApiConfig {
                enabled: true,
                host: "127.0.0.1".to_string(),
                port: 8080,
                max_connections: 100,
            },
            network: NetworkConfig {
                enabled: false,
                p2p_port: 9000,
                quic_port: 9001,
                bootstrap_peers: vec![],
                dns_seeds: vec!["_seeds.adicl1.com".to_string()],
                max_peers: 50,
                use_production_tls: true,  // Secure by default - use config files to disable for dev
                ca_cert_path: None,
                node_cert_path: None,
                node_key_path: None,
                auto_update: false,
                asn: None,
                region: None,
            },
            logging: LoggingConfig::default(),
            genesis: None,
            applications: ApplicationsConfig::default(),
        }
    }
}

/// Application layer configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApplicationsConfig {
    /// Enable PoUW task management
    pub pouw: Option<PoUWConfig>,
    /// Enable governance proposals and voting
    pub governance: Option<GovernanceAppConfig>,
    /// Enable decentralized storage market
    pub storage_market: Option<StorageMarketAppConfig>,
}

impl Default for ApplicationsConfig {
    fn default() -> Self {
        Self {
            pouw: None,
            governance: None,
            storage_market: None,
        }
    }
}

/// PoUW application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoUWConfig {
    #[serde(default = "default_min_deposit")]
    pub min_deposit: f64,
    #[serde(default = "default_max_task_duration_epochs")]
    pub max_task_duration_epochs: u64,
    #[serde(default = "default_min_sponsor_reputation")]
    pub min_sponsor_reputation: f64,
    #[serde(default = "default_challenge_window_epochs")]
    pub challenge_window_epochs: u64,
    #[serde(default = "default_max_workers_per_task")]
    pub max_workers_per_task: u8,
}

fn default_min_deposit() -> f64 {
    1.0
}
fn default_max_task_duration_epochs() -> u64 {
    100
}
fn default_min_sponsor_reputation() -> f64 {
    100.0
}
fn default_challenge_window_epochs() -> u64 {
    5
}
fn default_max_workers_per_task() -> u8 {
    10
}

impl Default for PoUWConfig {
    fn default() -> Self {
        Self {
            min_deposit: default_min_deposit(),
            max_task_duration_epochs: default_max_task_duration_epochs(),
            min_sponsor_reputation: default_min_sponsor_reputation(),
            challenge_window_epochs: default_challenge_window_epochs(),
            max_workers_per_task: default_max_workers_per_task(),
        }
    }
}

/// Governance application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceAppConfig {
    #[serde(default = "default_rmax")]
    pub rmax: f64,
    #[serde(default = "default_min_quorum")]
    pub min_quorum: f64,
    #[serde(default = "default_voting_period_epochs")]
    pub voting_period_epochs: u64,
    #[serde(default = "default_operational_threshold")]
    pub operational_threshold: f64,
    #[serde(default = "default_constitutional_threshold")]
    pub constitutional_threshold: f64,
}

fn default_rmax() -> f64 {
    100_000.0
}
fn default_min_quorum() -> f64 {
    0.1
}
fn default_voting_period_epochs() -> u64 {
    10
}
fn default_operational_threshold() -> f64 {
    0.51
}
fn default_constitutional_threshold() -> f64 {
    0.67
}

impl Default for GovernanceAppConfig {
    fn default() -> Self {
        Self {
            rmax: default_rmax(),
            min_quorum: default_min_quorum(),
            voting_period_epochs: default_voting_period_epochs(),
            operational_threshold: default_operational_threshold(),
            constitutional_threshold: default_constitutional_threshold(),
        }
    }
}

impl GovernanceAppConfig {
    /// Convert to adic_governance::LifecycleConfig
    pub fn to_lifecycle_config(&self, epoch_duration_secs: u64) -> adic_governance::LifecycleConfig {
        use adic_governance::LifecycleConfig;

        LifecycleConfig {
            min_proposer_reputation: 100.0, // Reasonable default
            voting_duration_secs: (self.voting_period_epochs * epoch_duration_secs) as i64,
            min_quorum: self.min_quorum,
            rmax: self.rmax,
            gamma_f1: 1.5, // F1 timelock multiplier
            gamma_f2: 3.0, // F2 timelock multiplier (stronger for constitutional)
            min_timelock_secs: 3600.0, // 1 hour minimum
            committee_size: 100, // Quorum committee size
            bls_threshold: 67,   // 67-of-100 for BLS signatures
        }
    }

    /// Convert to adic_network::protocol::GovernanceConfig
    pub fn to_protocol_config(&self, epoch_duration_secs: u64) -> adic_network::protocol::GovernanceConfig {
        use adic_network::protocol::GovernanceConfig;
        use std::time::Duration;

        GovernanceConfig {
            voting_period: Duration::from_secs(self.voting_period_epochs * epoch_duration_secs),
            min_proposal_reputation: 100.0, // Reasonable default
            min_vote_reputation: 10.0,      // Lower threshold for voting
            max_concurrent_proposals: 10,   // Reasonable limit
        }
    }
}

/// Storage market application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageMarketAppConfig {
    #[serde(default = "default_min_deal_duration")]
    pub min_deal_duration: u64,
    #[serde(default = "default_max_deal_duration")]
    pub max_deal_duration: u64,
    #[serde(default = "default_proof_window_epochs")]
    pub proof_window_epochs: u64,
    #[serde(default = "default_challenge_count")]
    pub challenge_count: u64,
    #[serde(default = "default_collateral_multiplier")]
    pub collateral_multiplier: f64,
}

fn default_min_deal_duration() -> u64 {
    10
}
fn default_max_deal_duration() -> u64 {
    1000
}
fn default_proof_window_epochs() -> u64 {
    5
}
fn default_challenge_count() -> u64 {
    10
}
fn default_collateral_multiplier() -> f64 {
    1.5
}

impl Default for StorageMarketAppConfig {
    fn default() -> Self {
        Self {
            min_deal_duration: default_min_deal_duration(),
            max_deal_duration: default_max_deal_duration(),
            proof_window_epochs: default_proof_window_epochs(),
            challenge_count: default_challenge_count(),
            collateral_multiplier: default_collateral_multiplier(),
        }
    }
}

impl NodeConfig {
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        // Don't apply env overrides here - let main.rs control the precedence order
        Ok(config)
    }

    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn _default_with_paths(data_dir: PathBuf, p2p_port: u16, api_port: u16) -> Result<Self> {
        let mut config = Self::default();
        config.node.data_dir = data_dir;
        config.network.p2p_port = p2p_port;
        config.api.port = api_port;
        config.apply_env_overrides();
        Ok(config)
    }

    /// Apply environment variable overrides
    ///
    /// Note: This is primarily used for testing. Production code uses UnifiedConfig
    /// in config_loader.rs which has its own environment override implementation.
    #[allow(dead_code)]
    pub fn apply_env_overrides(&mut self) {
        // Node configuration
        if let Ok(data_dir) = env::var("DATA_DIR") {
            self.node.data_dir = PathBuf::from(data_dir);
        }
        if let Ok(node_mode) = env::var("NODE_MODE") {
            self.node.validator = node_mode == "validator";
        }
        if let Ok(name) = env::var("NODE_ID") {
            if !name.is_empty() {
                self.node.name = name;
            }
        }

        // API configuration
        if let Ok(api_host) = env::var("API_HOST") {
            self.api.host = api_host;
        }
        if let Ok(api_port) = env::var("API_PORT") {
            if let Ok(port) = api_port.parse() {
                self.api.port = port;
            }
        }

        // Network configuration
        if let Ok(p2p_port) = env::var("P2P_PORT") {
            if let Ok(port) = p2p_port.parse() {
                self.network.p2p_port = port;
            }
        }
        if let Ok(quic_port) = env::var("QUIC_PORT") {
            if let Ok(port) = quic_port.parse() {
                self.network.quic_port = port;
            }
        }
        if let Ok(max_peers) = env::var("MAX_PEERS") {
            if let Ok(max) = max_peers.parse() {
                self.network.max_peers = max;
            }
        }
        if let Ok(bootstrap) = env::var("BOOTSTRAP_PEERS") {
            if !bootstrap.is_empty() {
                self.network.bootstrap_peers =
                    bootstrap.split(',').map(|s| s.trim().to_string()).collect();
            }
        }
        if let Ok(dns_seeds) = env::var("DNS_SEEDS") {
            if !dns_seeds.is_empty() {
                self.network.dns_seeds =
                    dns_seeds.split(',').map(|s| s.trim().to_string()).collect();
            }
        }

        // Storage configuration
        if let Ok(cache_size) = env::var("ROCKSDB_CACHE_SIZE_MB") {
            if let Ok(size) = cache_size.parse::<usize>() {
                self.storage.cache_size = size * 1024 * 1024; // Convert MB to bytes
            }
        }

        // Consensus parameters (optional overrides)
        if let Ok(p) = env::var("ADIC_P") {
            if let Ok(val) = p.parse() {
                self.consensus.p = val;
            }
        }
        if let Ok(d) = env::var("ADIC_D") {
            if let Ok(val) = d.parse() {
                self.consensus.d = val;
            }
        }
        if let Ok(k) = env::var("ADIC_K") {
            if let Ok(val) = k.parse() {
                self.consensus.k = val;
            }
        }
        if let Ok(deposit) = env::var("ADIC_DEPOSIT") {
            if let Ok(val) = deposit.parse() {
                self.consensus.deposit = val;
            }
        }

        // Metrics
        if let Ok(_metrics) = env::var("METRICS_ENABLED") {
            // This would need to be added to the config struct
            // For now, metrics are always enabled
        }

        // Debug mode
        if let Ok(_debug) = env::var("DEBUG_MODE") {
            // This would affect logging level, already handled by RUST_LOG
        }

        // Logging configuration
        if let Ok(log_level) = env::var("LOG_LEVEL") {
            self.logging.level = log_level;
        }
        if let Ok(log_format) = env::var("LOG_FORMAT") {
            if log_format == "json" || log_format == "pretty" || log_format == "compact" {
                self.logging.format = log_format;
            }
        }
        if let Ok(use_emojis) = env::var("LOG_USE_EMOJIS") {
            self.logging.use_emojis = use_emojis.to_lowercase() == "true" || use_emojis == "1";
        }
        if let Ok(log_file) = env::var("LOG_FILE") {
            self.logging.file_output = Some(PathBuf::from(log_file));
        }
        if let Ok(show_banner) = env::var("LOG_SHOW_BANNER") {
            self.logging.show_boot_banner =
                show_banner.to_lowercase() == "true" || show_banner == "1";
        }
    }

    pub fn adic_params(&self) -> AdicParams {
        self.consensus.clone().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_env_overrides() {
        // Set some test environment variables
        env::set_var("DATA_DIR", "/test/data");
        env::set_var("API_HOST", "192.168.1.1");
        env::set_var("API_PORT", "9090");
        env::set_var("P2P_PORT", "8000");
        env::set_var("NODE_MODE", "validator");
        env::set_var("MAX_PEERS", "100");
        env::set_var("BOOTSTRAP_PEERS", "peer1,peer2,peer3");
        env::set_var("ADIC_P", "5");
        env::set_var("ADIC_K", "30");

        let mut config = NodeConfig::default();
        config.apply_env_overrides();

        assert_eq!(config.node.data_dir, PathBuf::from("/test/data"));
        assert_eq!(config.api.host, "192.168.1.1");
        assert_eq!(config.api.port, 9090);
        assert_eq!(config.network.p2p_port, 8000);
        assert!(config.node.validator);
        assert_eq!(config.network.max_peers, 100);
        assert_eq!(
            config.network.bootstrap_peers,
            vec!["peer1", "peer2", "peer3"]
        );
        assert_eq!(config.consensus.p, 5);
        assert_eq!(config.consensus.k, 30);

        // Clean up
        env::remove_var("DATA_DIR");
        env::remove_var("API_HOST");
        env::remove_var("API_PORT");
        env::remove_var("P2P_PORT");
        env::remove_var("NODE_MODE");
        env::remove_var("MAX_PEERS");
        env::remove_var("BOOTSTRAP_PEERS");
        env::remove_var("ADIC_P");
        env::remove_var("ADIC_K");
    }

    #[test]
    fn test_genesis_config_parsing() {
        use crate::genesis::GenesisConfig;

        // Test config with genesis
        let config = NodeConfig {
            genesis: Some(GenesisConfig::default()),
            ..Default::default()
        };

        // Serialize to TOML
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(
            toml_str.contains("[genesis]"),
            "TOML should contain [genesis] section"
        );
        assert!(
            toml_str.contains("allocations"),
            "TOML should contain allocations"
        );
        assert!(
            toml_str.contains("chain_id"),
            "TOML should contain chain_id"
        );

        // Deserialize back
        let parsed: NodeConfig = toml::from_str(&toml_str).unwrap();
        assert!(
            parsed.genesis.is_some(),
            "Parsed config should have genesis"
        );

        let genesis = parsed.genesis.unwrap();
        assert_eq!(genesis.chain_id, "adic-dag-v1");
        assert!(!genesis.allocations.is_empty());
        assert_eq!(genesis.genesis_identities.len(), 4);

        // Test config without genesis
        let config_no_genesis = NodeConfig::default();
        let toml_str2 = toml::to_string_pretty(&config_no_genesis).unwrap();
        let parsed2: NodeConfig = toml::from_str(&toml_str2).unwrap();
        assert!(
            parsed2.genesis.is_none(),
            "Config without genesis should parse as None"
        );
    }
}
