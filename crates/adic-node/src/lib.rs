pub mod api;
pub mod api_wallet;
pub mod api_wallet_tx;
pub mod auth;
pub mod cli;
pub mod config;
pub mod copyover;
pub mod economics_api;
pub mod genesis;
pub mod logging;
pub mod metrics;
pub mod node;
pub mod progress_display;
pub mod update_manager;
pub mod update_verifier;
pub mod wallet;
pub mod wallet_registry;

pub use config::NodeConfig;
pub use metrics::Metrics;
pub use node::{AdicNode, NodeStats};

// Alias for test compatibility
pub type Node = AdicNode;
