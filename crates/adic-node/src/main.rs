use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod api_wallet;
mod api_wallet_tx;
mod auth;
mod cli;
mod config;
mod economics_api;
mod genesis;
mod logging;
mod metrics;
mod node;
mod wallet;
mod wallet_registry;

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

        /// Port for QUIC transport
        #[arg(long, default_value = "9001")]
        quic_port: u16,

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

        /// Parameter preset to use (v1, v2, testnet, mainnet)
        #[arg(long)]
        params: Option<String>,
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

    /// Wallet management commands
    Wallet {
        #[command(subcommand)]
        command: WalletCommands,
    },
}

#[derive(Subcommand)]
enum WalletCommands {
    /// Export wallet to a file
    Export {
        /// Path to export the wallet to
        #[arg(short, long)]
        output: PathBuf,

        /// Data directory containing the wallet
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,

        /// Node ID
        #[arg(long, default_value = "node1")]
        node_id: String,
    },

    /// Import wallet from a file
    Import {
        /// Path to import the wallet from
        #[arg(short, long)]
        input: PathBuf,

        /// Data directory to save the wallet to
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,

        /// Node ID
        #[arg(long, default_value = "node1")]
        node_id: String,
    },

    /// Show wallet information
    Info {
        /// Data directory containing the wallet
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,

        /// Node ID
        #[arg(long, default_value = "node1")]
        node_id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file if it exists (ignore if it doesn't)
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();

    // Load config early to get logging settings
    let temp_config = if let Some(ref config_path) = cli.config {
        config::NodeConfig::from_file(config_path).ok()
    } else if Path::new("./adic-config.toml").exists() {
        config::NodeConfig::from_file(Path::new("./adic-config.toml")).ok()
    } else {
        None
    };

    let logging_config = temp_config
        .as_ref()
        .map(|c| c.logging.clone())
        .unwrap_or_default();

    // Show boot banner if enabled
    if logging_config.show_boot_banner && cli.verbose == 0 && std::env::var("RUST_LOG").is_err() {
        logging::display_boot_banner("0.1.5");
    }

    // Show emoji legend if enabled
    if logging_config.show_emoji_legend && logging_config.use_emojis {
        logging::display_emoji_legend();
    }

    // Initialize logging system
    if let Err(e) = logging::init_logging(&logging_config, cli.verbose) {
        eprintln!("Failed to initialize logging: {}", e);
        // Fall back to basic logging
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
    }

    match cli.command {
        Commands::Start {
            data_dir,
            port,
            quic_port,
            api_port,
            validator,
        } => {
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
            if quic_port != 9001 {
                config.network.quic_port = quic_port;
            }
            if api_port != 8080 {
                config.api.port = api_port;
            }
            // Validator flag is always explicit (no default true state)
            if validator {
                config.node.validator = validator;
            }

            info!(
                version = "0.1.5",
                data_dir = ?config.node.data_dir,
                p2p_port = config.network.p2p_port,
                quic_port = config.network.quic_port,
                api_port = config.api.port,
                validator = config.node.validator,
                "🧬 Starting ADIC node"
            );

            // Create and start node
            let node = node::AdicNode::new(config.clone()).await?;

            info!(
                node_id = %node.node_id(),
                "✅ Node initialized successfully"
            );

            // Start API server using config port
            let api_handle =
                api::start_api_server(node.clone(), config.api.host.clone(), config.api.port);

            // Start the node
            let node_handle = tokio::spawn(async move {
                if let Err(e) = node.run().await {
                    warn!(
                        error = %e,
                        "❌ Node encountered error"
                    );
                }
            });

            info!("✅ NODE READY - All systems operational");

            // Wait for shutdown signal
            tokio::signal::ctrl_c().await?;
            info!("🛑 Shutting down gracefully");

            // Cancel tasks
            api_handle.abort();
            node_handle.abort();

            Ok(())
        }

        Commands::Init { output, params } => {
            info!(
                output_dir = ?output,
                "🧬 Initializing new node configuration"
            );

            // Create output directory if it doesn't exist
            std::fs::create_dir_all(&output)?;

            let mut config = config::NodeConfig::default();

            // Apply parameter preset if specified
            if let Some(preset) = params {
                match preset.as_str() {
                    "v1" => {
                        info!(p = 3, d = 3, "🧬 Using v1 parameters");
                        config.consensus.p = 3;
                        config.consensus.d = 3;
                        config.consensus.rho = vec![2, 2, 1];
                        config.consensus.k = 20;
                        config.consensus.depth_star = 12;
                    }
                    "v2" => {
                        info!(
                            p = 5,
                            d = 2,
                            "🧬 Using v2 parameters (enhanced performance)"
                        );
                        config.consensus.p = 5;
                        config.consensus.d = 2;
                        config.consensus.rho = vec![3, 2];
                        config.consensus.k = 15;
                        config.consensus.depth_star = 8;
                    }
                    "testnet" => {
                        info!(
                            p = 3,
                            d = 2,
                            security = "low",
                            speed = "high",
                            "🧬 Using testnet parameters"
                        );
                        config.consensus.p = 3;
                        config.consensus.d = 2;
                        config.consensus.rho = vec![1, 1];
                        config.consensus.k = 3;
                        config.consensus.depth_star = 3;
                        config.consensus.r_sum_min = 1.0;
                    }
                    "mainnet" => {
                        info!(
                            p = 7,
                            d = 4,
                            security = "high",
                            "🧬 Using mainnet parameters"
                        );
                        config.consensus.p = 7;
                        config.consensus.d = 4;
                        config.consensus.rho = vec![3, 3, 2, 2];
                        config.consensus.k = 50;
                        config.consensus.depth_star = 20;
                        config.consensus.r_sum_min = 10.0;
                    }
                    _ => {
                        warn!(
                            preset = %preset,
                            "⚠️ Unknown parameter preset, using defaults"
                        );
                    }
                }
            }

            let config_path = output.join("adic-config.toml");
            config.save_to_file(&config_path)?;
            info!(
                path = ?config_path,
                "✅ Configuration saved"
            );

            // Also generate a keypair
            let keypair = adic_crypto::Keypair::generate();
            let key_path = output.join("node.key");
            std::fs::write(&key_path, keypair.to_bytes())?;
            info!(
                path = ?key_path,
                public_key = %hex::encode(keypair.public_key().as_bytes()),
                "🔐 Keypair saved"
            );

            Ok(())
        }

        Commands::Keygen { output } => {
            info!("🔐 Generating new keypair");
            let keypair = adic_crypto::Keypair::generate();

            if let Some(path) = output {
                std::fs::write(&path, keypair.to_bytes())?;
                info!(
                    path = ?path,
                    "✅ Keypair saved"
                );
            } else {
                println!("Private key: {}", hex::encode(keypair.to_bytes()));
            }
            println!(
                "Public key: {}",
                hex::encode(keypair.public_key().as_bytes())
            );

            Ok(())
        }

        Commands::Test { count } => {
            info!(message_count = count, "🧪 Running local test");
            cli::run_local_test(count).await?;
            Ok(())
        }

        Commands::Wallet { command } => {
            use std::io::{self, Write};

            match command {
                WalletCommands::Export {
                    output,
                    data_dir,
                    node_id,
                } => {
                    info!("Exporting wallet to {:?}", output);

                    // Get password for export
                    let password =
                        rpassword::prompt_password("Enter password to encrypt exported wallet: ")
                            .context("Failed to read password")?;

                    // Load existing wallet
                    let wallet = wallet::NodeWallet::load_or_create(&data_dir, &node_id)?;

                    // Export to file
                    wallet.export_to_file(&output, &password)?;

                    info!("✅ Wallet exported successfully to {:?}", output);
                    println!(
                        "Wallet address: {}",
                        wallet
                            .address()
                            .to_bech32()
                            .unwrap_or_else(|_| hex::encode(wallet.address().as_bytes()))
                    );
                    Ok(())
                }

                WalletCommands::Import {
                    input,
                    data_dir,
                    node_id,
                } => {
                    info!("Importing wallet from {:?}", input);

                    // Check if wallet already exists
                    let wallet_path = data_dir.join("wallet.json");
                    if wallet_path.exists() {
                        warn!("Wallet already exists at {:?}", wallet_path);
                        print!("Overwrite existing wallet? [y/N]: ");
                        io::stdout().flush()?;

                        let mut response = String::new();
                        io::stdin().read_line(&mut response)?;

                        if !response.trim().eq_ignore_ascii_case("y") {
                            info!("Import cancelled");
                            return Ok(());
                        }
                    }

                    // Get password for import
                    let password =
                        rpassword::prompt_password("Enter password to decrypt imported wallet: ")
                            .context("Failed to read password")?;

                    // Import wallet
                    let wallet = wallet::NodeWallet::import_from_file(&input, &password, &node_id)?;

                    // Save to data directory
                    std::fs::create_dir_all(&data_dir)?;
                    wallet.export_to_file(&wallet_path, &password)?;

                    info!("✅ Wallet imported successfully");
                    println!(
                        "Wallet address: {}",
                        wallet
                            .address()
                            .to_bech32()
                            .unwrap_or_else(|_| hex::encode(wallet.address().as_bytes()))
                    );
                    Ok(())
                }

                WalletCommands::Info { data_dir, node_id } => {
                    // Load wallet
                    let wallet = wallet::NodeWallet::load_or_create(&data_dir, &node_id)?;
                    let info = wallet.get_info();

                    println!("Wallet Information:");
                    println!("  Address (bech32): {}", info.address);
                    println!("  Address (hex):    {}", info.hex_address);
                    println!("  Public Key:       {}", info.public_key);
                    println!("  Node ID:          {}", info.node_id);
                    Ok(())
                }
            }
        }
    }
}
