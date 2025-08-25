use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod node;
mod api;
mod cli;
mod metrics;

#[derive(Parser)]
#[command(name = "adic")]
#[command(about = "ADIC Core - P-adic DAG Consensus Node", long_about = None)]
struct Cli {
    /// Configuration file path
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Verbosity level (can be repeated)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the ADIC node
    Start {
        /// Data directory for storage
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,

        /// Port to listen on for P2P connections
        #[arg(short, long, default_value = "9000")]
        port: u16,

        /// Port for HTTP API
        #[arg(long, default_value = "8080")]
        api_port: u16,

        /// Enable validator mode
        #[arg(long)]
        validator: bool,
    },

    /// Initialize a new node configuration
    Init {
        /// Output directory for configuration
        #[arg(short, long, default_value = ".")]
        output: PathBuf,
    },

    /// Generate a new keypair
    Keygen {
        /// Output file for the keypair
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Create and submit a test message (local testing)
    Test {
        /// Number of messages to create
        #[arg(short, long, default_value = "10")]
        count: usize,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file if it exists (ignore if it doesn't)
    let _ = dotenv::dotenv();
    
    let cli = Cli::parse();

    // Initialize logging
    let log_level = match cli.verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| format!("adic={}", log_level)),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    match cli.command {
        Commands::Start { data_dir, port, api_port, validator } => {
            // Priority order: CLI args > ENV vars > Config file > Defaults
            
            // 1. Start with config file or defaults
            let mut config = if let Some(config_path) = cli.config {
                config::NodeConfig::from_file(&config_path)?
            } else if Path::new("./adic-config.toml").exists() {
                // Load existing config file if present
                config::NodeConfig::from_file(Path::new("./adic-config.toml"))?
            } else {
                config::NodeConfig::default()
            };
            
            // 2. Apply environment variable overrides (medium priority)
            // Note: from_file already calls apply_env_overrides, but we call again
            // in case we loaded from default
            config.apply_env_overrides();
            
            // 3. Apply CLI argument overrides (highest priority)
            // Only override if CLI args were explicitly provided (not defaults)
            // Check if args differ from clap defaults to know if user specified them
            if data_dir != PathBuf::from("./data") {
                config.node.data_dir = data_dir;
            }
            if port != 9000 {
                config.network.p2p_port = port;
            }
            if api_port != 8080 {
                config.api.port = api_port;
            }
            // Validator flag is always explicit (no default true state)
            if validator {
                config.node.validator = validator;
            }
            
            info!("Starting ADIC node...");
            info!("Data directory: {:?}", config.node.data_dir);
            info!("P2P port: {}", config.network.p2p_port);
            info!("API port: {}", config.api.port);
            info!("Validator mode: {}", config.node.validator);

            // Create and start node
            let node = node::AdicNode::new(config.clone()).await?;
            
            info!("Node initialized successfully");
            info!("Node ID: {}", node.node_id());
            
            // Start API server using config port
            let api_handle = api::start_api_server(node.clone(), config.api.port);
            
            // Start the node
            let node_handle = tokio::spawn(async move {
                if let Err(e) = node.run().await {
                    warn!("Node error: {}", e);
                }
            });
            
            // Wait for shutdown signal
            tokio::signal::ctrl_c().await?;
            info!("Shutting down gracefully...");
            
            // Cancel tasks
            api_handle.abort();
            node_handle.abort();
            
            Ok(())
        }

        Commands::Init { output } => {
            info!("Initializing new node configuration...");
            
            // Create output directory if it doesn't exist
            std::fs::create_dir_all(&output)?;
            
            let config = config::NodeConfig::default();
            let config_path = output.join("adic-config.toml");
            config.save_to_file(&config_path)?;
            info!("Configuration saved to: {:?}", config_path);
            
            // Also generate a keypair
            let keypair = adic_crypto::Keypair::generate();
            let key_path = output.join("node.key");
            std::fs::write(&key_path, keypair.to_bytes())?;
            info!("Keypair saved to: {:?}", key_path);
            info!("Public key: {}", hex::encode(keypair.public_key().as_bytes()));
            
            Ok(())
        }

        Commands::Keygen { output } => {
            info!("Generating new keypair...");
            let keypair = adic_crypto::Keypair::generate();
            
            if let Some(path) = output {
                std::fs::write(&path, keypair.to_bytes())?;
                info!("Keypair saved to: {:?}", path);
            } else {
                println!("Private key: {}", hex::encode(keypair.to_bytes()));
            }
            println!("Public key: {}", hex::encode(keypair.public_key().as_bytes()));
            
            Ok(())
        }

        Commands::Test { count } => {
            info!("Running local test with {} messages", count);
            cli::run_local_test(count).await?;
            Ok(())
        }
    }
}