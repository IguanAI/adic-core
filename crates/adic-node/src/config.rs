use adic_types::AdicParams;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    pub node: NodeSettings,
    pub consensus: ConsensusConfig,
    pub storage: StorageConfig,
    pub api: ApiConfig,
    pub network: NetworkConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSettings {
    pub data_dir: PathBuf,
    pub keypair_path: Option<PathBuf>,
    pub validator: bool,
    pub name: String,
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
    pub beta: f64,
    pub mu: f64,
    pub gamma: f64,
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
    pub bootstrap_peers: Vec<String>,
    pub max_peers: usize,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            node: NodeSettings {
                data_dir: PathBuf::from("./data"),
                keypair_path: None,
                validator: false,
                name: "adic-node".to_string(),
            },
            consensus: ConsensusConfig {
                p: 3,
                d: 3,
                rho: vec![2, 2, 1],
                q: 3,
                k: 20,
                depth_star: 12,     // Phase-0 default
                delta: 5,
                r_sum_min: 4.0,     // Phase-0 default
                r_min: 1.0,         // Phase-0 default
                deposit: 1.0,
                lambda: 1.0,        // Phase-0 default
                beta: 0.5,          // Phase-0 default
                mu: 1.0,            // Phase-0 default
                gamma: 0.9,
            },
            storage: StorageConfig {
                backend: "memory".to_string(),
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
                bootstrap_peers: vec![],
                max_peers: 50,
            },
        }
    }
}

impl NodeConfig {
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn default_with_paths(data_dir: PathBuf, p2p_port: u16, api_port: u16) -> Result<Self> {
        let mut config = Self::default();
        config.node.data_dir = data_dir;
        config.network.p2p_port = p2p_port;
        config.api.port = api_port;
        Ok(config)
    }

    pub fn adic_params(&self) -> AdicParams {
        self.consensus.clone().into()
    }
}