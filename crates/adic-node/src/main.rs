use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod api_governance;
mod api_storage_market;
mod api_sse;
mod api_wallet;
mod api_wallet_tx;
mod api_ws;
mod auth;
mod cli;
mod config;
mod config_loader;
mod copyover;
mod economics_api;
mod events;
mod genesis;
mod logging;
mod metrics;
mod node;
mod openapi;
mod progress_display;
mod update_manager;
mod update_verifier;
mod wallet;
mod wallet_registry;

#[derive(Parser)]
#[command(name = "adic")]
#[command(about = "ADIC Core - P-adic DAG Consensus Node", long_about = None)]
#[command(version)]
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
        /// Network to connect to (mainnet, testnet, devnet)
        #[arg(long, default_value = "mainnet")]
        network: String,

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

        /// Copyover recovery mode (internal use)
        #[arg(long, hide = true)]
        copyover_recovery: bool,

        /// Copyover pipe file descriptor (internal use)
        #[arg(long, hide = true)]
        copyover_pipe: Option<i32>,

        /// Copyover API listener file descriptor (internal use)
        #[arg(long, hide = true)]
        copyover_fd: Option<i32>,
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

    /// Generate a genesis configuration file
    Genesis {
        /// Output file path
        #[arg(short, long, default_value = "genesis.json")]
        output: PathBuf,

        /// Network type (mainnet, testnet, devnet)
        #[arg(short, long, default_value = "testnet")]
        network: String,
    },

    /// Wallet management commands
    Wallet {
        #[command(subcommand)]
        command: WalletCommands,
    },

    /// Manage node updates
    Update {
        #[command(subcommand)]
        subcommand: UpdateCommands,
    },

    /// Governance operations (proposals, voting)
    Governance {
        #[command(subcommand)]
        command: GovernanceCommands,
    },

    /// Storage market operations
    StorageMarket {
        #[command(subcommand)]
        command: StorageMarketCommands,
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

#[derive(Subcommand)]
enum UpdateCommands {
    /// Check for available updates
    Check,

    /// Download the latest update
    Download,

    /// Apply a downloaded update (triggers copyover)
    Apply {
        /// Path to the new binary (defaults to ./adic.new)
        #[arg(long)]
        binary: Option<PathBuf>,
    },

    /// Show current update status
    Status,

    /// Seed a binary for P2P distribution
    Seed {
        /// Path to the binary to seed
        #[arg(long)]
        binary: PathBuf,

        /// Version string (e.g., "0.2.0")
        #[arg(long)]
        version: String,
    },
}

#[derive(Subcommand)]
enum GovernanceCommands {
    /// Submit a governance proposal
    Propose {
        /// Parameter keys to update (comma-separated, e.g., "k,delta")
        #[arg(long)]
        params: String,

        /// New values as JSON (e.g., '{"k": 25, "delta": 10}')
        #[arg(long)]
        values: String,

        /// Proposal class: constitutional or operational
        #[arg(long, default_value = "operational")]
        class: String,

        /// IPFS CID of rationale document
        #[arg(long)]
        rationale: String,

        /// Epoch at which to enact (0 = auto-calculate)
        #[arg(long, default_value = "0")]
        enact_epoch: u64,

        /// Network to connect to
        #[arg(long, default_value = "testnet")]
        network: String,
    },

    /// Vote on a proposal
    Vote {
        /// Proposal ID (hex)
        #[arg(long)]
        proposal_id: String,

        /// Vote: yes, no, or abstain
        #[arg(long)]
        ballot: String,

        /// Network to connect to
        #[arg(long, default_value = "testnet")]
        network: String,
    },

    /// List all proposals
    List {
        /// Filter by status (voting, tallying, succeeded, enacted, rejected, failed)
        #[arg(long)]
        status: Option<String>,

        /// Network to connect to
        #[arg(long, default_value = "testnet")]
        network: String,
    },

    /// Show proposal details
    Show {
        /// Proposal ID (hex)
        #[arg(long)]
        proposal_id: String,

        /// Network to connect to
        #[arg(long, default_value = "testnet")]
        network: String,
    },

    /// Show current governance parameters
    Parameters {
        /// Network to connect to
        #[arg(long, default_value = "testnet")]
        network: String,
    },
}

#[derive(Subcommand)]
enum StorageMarketCommands {
    /// Publish a storage deal intent
    PublishIntent {
        /// Data CID (hex, 64 chars = 32 bytes)
        #[arg(long)]
        data_cid: String,

        /// Data size in bytes
        #[arg(long)]
        size: u64,

        /// Duration in epochs
        #[arg(long)]
        duration: u64,

        /// Max price per epoch (ADIC)
        #[arg(long)]
        max_price: f64,

        /// Required redundancy (number of providers)
        #[arg(long, default_value = "1")]
        redundancy: u8,

        /// Settlement rail (adic, filecoin, arweave)
        #[arg(long, default_value = "adic")]
        rail: String,

        /// Node API URL
        #[arg(long, default_value = "http://localhost:8080")]
        api_url: String,
    },

    /// List storage intents
    ListIntents {
        /// Filter by client address (hex)
        #[arg(long)]
        client: Option<String>,

        /// Filter by status (pending, finalized)
        #[arg(long)]
        status: Option<String>,

        /// Limit results
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Offset for pagination
        #[arg(long, default_value = "0")]
        offset: usize,

        /// API URL
        #[arg(long, default_value = "http://localhost:8080")]
        api_url: String,
    },

    /// Get intent details
    GetIntent {
        /// Intent ID (hex)
        #[arg(long)]
        id: String,

        /// API URL
        #[arg(long, default_value = "http://localhost:8080")]
        api_url: String,
    },

    /// Submit provider acceptance
    AcceptIntent {
        /// Intent ID to accept (hex)
        #[arg(long)]
        intent_id: String,

        /// Price per epoch (ADIC)
        #[arg(long)]
        price: f64,

        /// Collateral amount (ADIC)
        #[arg(long)]
        collateral: f64,

        /// API URL
        #[arg(long, default_value = "http://localhost:8080")]
        api_url: String,
    },

    /// List acceptances
    ListAcceptances {
        /// Filter by provider address (hex)
        #[arg(long)]
        provider: Option<String>,

        /// Filter by intent ID (hex)
        #[arg(long)]
        intent_id: Option<String>,

        /// Limit results
        #[arg(long, default_value = "50")]
        limit: usize,

        /// API URL
        #[arg(long, default_value = "http://localhost:8080")]
        api_url: String,
    },

    /// Compile a deal from intent and acceptance
    CompileDeal {
        /// Intent ID (hex)
        #[arg(long)]
        intent_id: String,

        /// Acceptance ID (hex)
        #[arg(long)]
        acceptance_id: String,

        /// API URL
        #[arg(long, default_value = "http://localhost:8080")]
        api_url: String,
    },

    /// Activate a deal
    ActivateDeal {
        /// Deal ID
        #[arg(long)]
        deal_id: u64,

        /// Merkle root of data (hex, 64 chars = 32 bytes)
        #[arg(long)]
        merkle_root: String,

        /// Chunk count
        #[arg(long)]
        chunks: u64,

        /// API URL
        #[arg(long, default_value = "http://localhost:8080")]
        api_url: String,
    },

    /// List deals
    ListDeals {
        /// Filter by client address (hex)
        #[arg(long)]
        client: Option<String>,

        /// Filter by provider address (hex)
        #[arg(long)]
        provider: Option<String>,

        /// Filter by status (published, pendingactivation, active, completed, failed)
        #[arg(long)]
        status: Option<String>,

        /// Limit results
        #[arg(long, default_value = "50")]
        limit: usize,

        /// API URL
        #[arg(long, default_value = "http://localhost:8080")]
        api_url: String,
    },

    /// Get deal details
    GetDeal {
        /// Deal ID
        #[arg(long)]
        id: u64,

        /// API URL
        #[arg(long, default_value = "http://localhost:8080")]
        api_url: String,
    },

    /// Get challenges for a deal and epoch
    GetChallenges {
        /// Deal ID
        #[arg(long)]
        deal_id: u64,

        /// Epoch number
        #[arg(long)]
        epoch: u64,

        /// API URL
        #[arg(long, default_value = "http://localhost:8080")]
        api_url: String,
    },

    /// Show market statistics
    Stats {
        /// API URL
        #[arg(long, default_value = "http://localhost:8080")]
        api_url: String,
    },

    /// Show provider statistics
    ProviderStats {
        /// Provider address (hex)
        #[arg(long)]
        address: String,

        /// API URL
        #[arg(long, default_value = "http://localhost:8080")]
        api_url: String,
    },

    /// Show client statistics
    ClientStats {
        /// Client address (hex)
        #[arg(long)]
        address: String,

        /// API URL
        #[arg(long, default_value = "http://localhost:8080")]
        api_url: String,
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
        logging::display_boot_banner(env!("CARGO_PKG_VERSION"));
    }

    // Show emoji legend only for the start command
    let is_start_command = matches!(cli.command, Commands::Start { .. });
    if is_start_command && logging_config.show_emoji_legend && logging_config.use_emojis {
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
            network,
            data_dir,
            port,
            quic_port,
            api_port,
            validator,
            copyover_recovery,
            copyover_pipe,
            copyover_fd,
        } => {
            // Check if we're recovering from copyover
            if copyover_recovery {
                if let Some(pipe_fd) = copyover_pipe {
                    info!("🔄 Recovering from copyover (pipe_fd={})", pipe_fd);

                    // Recover state from copyover
                    let state = copyover::CopyoverManager::recover_from_copyover(pipe_fd)?;

                    info!(
                        version = %state.version,
                        data_dir = %state.data_dir,
                        "📥 Recovered copyover state"
                    );

                    // Restore API listener from copyover_fd if provided
                    let api_listener = if let Some(api_fd) = copyover_fd {
                        use std::os::unix::io::FromRawFd;
                        let std_listener = unsafe { std::net::TcpListener::from_raw_fd(api_fd) };
                        std_listener.set_nonblocking(true)?;
                        Some(tokio::net::TcpListener::from_std(std_listener)?)
                    } else {
                        None
                    };

                    // Load config from recovered path
                    let config_path = PathBuf::from(&state.config_path);
                    let mut config = if config_path.exists() {
                        config::NodeConfig::from_file(&config_path)?
                    } else {
                        config::NodeConfig::default()
                    };

                    // Apply recovered data directory
                    config.node.data_dir = PathBuf::from(&state.data_dir);

                    info!("✅ Copyover recovery complete, starting node");

                    // Start node with recovered config
                    let mut node = node::AdicNode::new(config.clone()).await?;

                    // Configure governance if enabled
                    if let Some(ref gov_config) = config.applications.governance {
                        let lifecycle_config = gov_config.to_lifecycle_config(config.consensus.epoch_duration_secs);
                        let protocol_config = gov_config.to_protocol_config(config.consensus.epoch_duration_secs);
                        node = node.with_governance_manager(lifecycle_config, protocol_config)?;
                        info!(
                            epoch_duration_secs = config.consensus.epoch_duration_secs,
                            "🗳️  Governance system enabled (copyover recovery)"
                        );
                    }

                    info!(
                        node_id = %node.node_id(),
                        "🎯 Node initialized with recovered state"
                    );

                    // Start the HTTP API server with recovered listener if available
                    let server_handle = api::start_api_server_with_listener(
                        node.clone(),
                        config.api.host.clone(),
                        config.api.port,
                        api_listener,
                    );

                    // Run the node
                    tokio::select! {
                        result = node.run() => {
                            if let Err(e) = result {
                                error!("Node error: {}", e);
                            }
                        }
                        _ = tokio::signal::ctrl_c() => {
                            info!("Received shutdown signal");
                        }
                    }

                    // Cleanup
                    info!("Shutting down...");
                    // Node doesn't have explicit shutdown, just abort the server
                    server_handle.abort();

                    return Ok(());
                }
            }

            // Normal startup path
            // Priority order: CLI args > ENV vars > Network config file

            // 1. Load network-specific configuration (applies env overrides automatically)
            let unified_config = config_loader::UnifiedConfig::load(&network)?;

            info!(
                network = %unified_config.network_name(),
                chain_id = %unified_config.chain_id(),
                "📡 Network configuration loaded"
            );

            // 2. Convert to legacy NodeConfig format for compatibility
            let mut config = unified_config.to_node_config();

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
                version = env!("CARGO_PKG_VERSION"),
                data_dir = ?config.node.data_dir,
                p2p_port = config.network.p2p_port,
                quic_port = config.network.quic_port,
                api_port = config.api.port,
                validator = config.node.validator,
                "🧬 Starting ADIC node"
            );

            // Create and start node
            let mut node = node::AdicNode::new(config.clone()).await?;

            // Configure governance if enabled
            if let Some(ref gov_config) = config.applications.governance {
                let lifecycle_config = gov_config.to_lifecycle_config(config.consensus.epoch_duration_secs);
                let protocol_config = gov_config.to_protocol_config(config.consensus.epoch_duration_secs);

                node = node.with_governance_manager(lifecycle_config, protocol_config)?;
                info!(
                    rmax = gov_config.rmax,
                    min_quorum = gov_config.min_quorum,
                    voting_period_epochs = gov_config.voting_period_epochs,
                    epoch_duration_secs = config.consensus.epoch_duration_secs,
                    "🗳️  Governance system enabled"
                );
            }

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
                    let error_msg = e.to_string();
                    warn!(
                        error = %error_msg,
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

        Commands::Genesis { output, network } => {
            info!(
                network = %network,
                output = ?output,
                "🧬 Generating genesis configuration"
            );

            let genesis_config = match network.as_str() {
                "mainnet" => {
                    info!("Using mainnet genesis parameters (300M ADIC)");
                    genesis::GenesisConfig::default()
                }
                "testnet" | "devnet" => {
                    info!("Using test genesis parameters");
                    genesis::GenesisConfig::test()
                }
                _ => {
                    warn!(
                        network = %network,
                        "⚠️ Unknown network type, using testnet"
                    );
                    genesis::GenesisConfig::test()
                }
            };

            // Verify the config before saving
            genesis_config
                .verify()
                .map_err(|e| anyhow::anyhow!("Genesis configuration validation failed: {}", e))?;

            // Serialize to JSON
            let json = serde_json::to_string_pretty(&genesis_config)
                .context("Failed to serialize genesis config")?;

            // Write to file
            std::fs::write(&output, json).context("Failed to write genesis file")?;

            let total_supply = genesis_config.total_supply();

            info!(
                path = ?output,
                allocations = genesis_config.allocations.len(),
                total_supply = %total_supply.to_adic(),
                genesis_hash = %genesis_config.calculate_hash(),
                "✅ Genesis configuration saved"
            );

            println!("✅ Genesis configuration saved to {:?}", output);
            println!("   Network:        {}", network);
            println!("   Chain ID:       {}", genesis_config.chain_id);
            println!("   Total supply:   {} ADIC", total_supply.to_adic());
            println!("   Allocations:    {}", genesis_config.allocations.len());
            println!("   Genesis hash:   {}", genesis_config.calculate_hash());

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

        Commands::Update { subcommand } => handle_update_command(subcommand, cli.config).await,

        Commands::Governance { command } => {
            info!("🗳️  Governance command");
            match command {
                GovernanceCommands::Propose {
                    params,
                    values,
                    class,
                    rationale,
                    enact_epoch,
                    network,
                } => {
                    adic_node::cli_governance::handle_propose(
                        params,
                        values,
                        class,
                        rationale,
                        enact_epoch,
                        network,
                        cli.config,
                    )
                    .await
                }
                GovernanceCommands::Vote {
                    proposal_id,
                    ballot,
                    network,
                } => {
                    adic_node::cli_governance::handle_vote(
                        proposal_id,
                        ballot,
                        network,
                        cli.config,
                    )
                    .await
                }
                GovernanceCommands::List { status, network } => {
                    adic_node::cli_governance::handle_list(
                        status,
                        network,
                        cli.config,
                    )
                    .await
                }
                GovernanceCommands::Show {
                    proposal_id,
                    network,
                } => {
                    adic_node::cli_governance::handle_show(
                        proposal_id,
                        network,
                        cli.config,
                    )
                    .await
                }
                GovernanceCommands::Parameters { network } => {
                    adic_node::cli_governance::handle_parameters(
                        network,
                        cli.config,
                    )
                    .await
                }
            }
        }

        Commands::StorageMarket { command } => {
            info!("💾 Storage market command");
            // Convert main's StorageMarketCommands to cli module's version
            handle_storage_market_cli(command).await
        }
    }
}

async fn handle_update_command(cmd: UpdateCommands, config_path: Option<PathBuf>) -> Result<()> {
    use adic_network::dns_version::DnsVersionDiscovery;

    match cmd {
        UpdateCommands::Check => {
            info!("🔍 Checking for updates...");

            // Get current version from Cargo.toml
            let current_version = env!("CARGO_PKG_VERSION");

            // Check DNS for updates
            let dns = DnsVersionDiscovery::new("adic.network.adicl1.com".to_string())?;
            match dns.check_for_update(current_version).await {
                Ok(Some(update)) => {
                    println!("🆕 Update available!");
                    println!("  Current version: {}", current_version);
                    println!("  Latest version:  {}", update.version);
                    println!("  SHA256:         {}", &update.sha256_hash[..16]);
                    if let Some(date) = update.release_date {
                        println!("  Release date:   {}", date);
                    }
                    println!("\nRun 'adic update download' to download the update");
                }
                Ok(None) => {
                    println!("✅ You're on the latest version ({})", current_version);
                }
                Err(e) => {
                    warn!("⚠️ Failed to check for updates: {}", e);
                    println!("Failed to check for updates. Please try again later.");
                }
            }
            Ok(())
        }

        UpdateCommands::Download => {
            use crate::progress_display::SimpleProgress;

            info!("📥 Checking for updates to download...");

            // Get current version
            let current_version = env!("CARGO_PKG_VERSION");

            // Check DNS for updates
            let dns = DnsVersionDiscovery::new("adic.network.adicl1.com".to_string())?;
            match dns.check_for_update(current_version).await {
                Ok(Some(update)) => {
                    println!("Found update: v{}", update.version);
                    println!("SHA256: {}", &update.sha256_hash[..32]);
                    println!();

                    // In standalone mode, we can't use P2P yet
                    // This is a placeholder for future HTTP fallback
                    println!("Note: Standalone download requires P2P network connection.");
                    println!("Please run 'adic start' to connect to the network first,");
                    println!("or download the binary manually from the official source.");

                    // Demonstrate what the progress would look like
                    println!("\nWhen P2P is available, download will show progress like this:\n");

                    // Show a demo progress bar
                    use crate::progress_display::DownloadProgressBar;
                    use std::io::IsTerminal;

                    if std::io::stderr().is_terminal() {
                        let demo_bar = DownloadProgressBar::new_chunk_progress(&update.version, 10);
                        for i in 1..=10 {
                            demo_bar.update_chunk(i, 10, 2.5, 4);
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                        demo_bar.start_verification();
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        demo_bar.finish_success(&update.version);
                    } else {
                        let mut simple_progress = SimpleProgress::new(&update.version);
                        for i in 1..=10 {
                            simple_progress.update(i as u64 * 1024 * 1024, 10 * 1024 * 1024);
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                        simple_progress.finish_success();
                    }
                }
                Ok(None) => {
                    println!(
                        "✅ You're already on the latest version ({})",
                        current_version
                    );
                }
                Err(e) => {
                    warn!("⚠️ Failed to check for updates: {}", e);
                    println!("Failed to check for updates. Please try again later.");
                }
            }

            Ok(())
        }

        UpdateCommands::Apply { binary } => {
            let binary_path = binary.unwrap_or_else(|| PathBuf::from("./adic.new"));

            if !binary_path.exists() {
                return Err(anyhow::anyhow!("Binary not found at {:?}", binary_path));
            }

            info!("🔄 Applying update via copyover...");

            let mut copyover = copyover::CopyoverManager::new();

            // In standalone mode, we don't have API fd to preserve
            // This would be populated when running as a node
            copyover.prepare_state(
                None, // No API fd in CLI mode
                config_path
                    .unwrap_or_else(|| PathBuf::from("./adic-config.toml"))
                    .to_string_lossy()
                    .to_string(),
                "./data".to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            )?;

            // Execute copyover
            copyover
                .safe_copyover(&binary_path.to_string_lossy())
                .await?;

            Ok(())
        }

        UpdateCommands::Status => {
            println!("Update Status:");
            println!("  Current version: {}", env!("CARGO_PKG_VERSION"));
            println!("  Update system:   Ready");
            println!("\nUse 'adic update check' to check for updates");
            Ok(())
        }

        UpdateCommands::Seed { binary, version } => {
            use adic_network::protocol::binary_store::BinaryStore;
            use std::path::PathBuf;

            info!("🌱 Seeding binary for P2P distribution");

            if !binary.exists() {
                return Err(anyhow::anyhow!("Binary file not found: {:?}", binary));
            }

            // Load config to get data directory
            let config_path = config_path.unwrap_or_else(|| PathBuf::from("./adic-config.toml"));
            let config = if config_path.exists() {
                config::NodeConfig::from_file(&config_path)?
            } else {
                config::NodeConfig::default()
            };

            // Create binary store
            let data_dir = PathBuf::from(&config.node.data_dir);
            let binary_store = BinaryStore::new(data_dir)?;

            println!(
                "Adding binary {} version {} to store...",
                binary.display(),
                version
            );

            // Add binary to store (creates chunks and metadata)
            let metadata = binary_store.add_binary(version.clone(), &binary).await?;

            println!("✅ Binary successfully added to P2P seed store");
            println!("  Version:      {}", metadata.version);
            println!("  Binary hash:  {}", &metadata.binary_hash[..16]);
            println!("  Total size:   {} bytes", metadata.total_size);
            println!("  Chunks:       {}", metadata.total_chunks);
            println!("\nBinary is now available for P2P distribution to peers.");
            println!(
                "Restart the node to begin seeding: adic --config {} start",
                config_path.display()
            );

            Ok(())
        }
    }
}

/// Convert main's StorageMarketCommands to CLI handler's version
async fn handle_storage_market_cli(cmd: StorageMarketCommands) -> Result<()> {
    use adic_node::cli_storage_market;

    // Convert to CLI module's enum variant
    let cli_cmd = match cmd {
        StorageMarketCommands::PublishIntent { data_cid, size, duration, max_price, redundancy, rail, api_url } => {
            cli_storage_market::StorageMarketCommands::PublishIntent { data_cid, size, duration, max_price, redundancy, rail, api_url }
        }
        StorageMarketCommands::ListIntents { client, status, limit, offset, api_url } => {
            cli_storage_market::StorageMarketCommands::ListIntents { client, status, limit, offset, api_url }
        }
        StorageMarketCommands::GetIntent { id, api_url } => {
            cli_storage_market::StorageMarketCommands::GetIntent { id, api_url }
        }
        StorageMarketCommands::AcceptIntent { intent_id, price, collateral, api_url } => {
            cli_storage_market::StorageMarketCommands::AcceptIntent { intent_id, price, collateral, api_url }
        }
        StorageMarketCommands::ListAcceptances { provider, intent_id, limit, api_url } => {
            cli_storage_market::StorageMarketCommands::ListAcceptances { provider, intent_id, limit, api_url }
        }
        StorageMarketCommands::CompileDeal { intent_id, acceptance_id, api_url } => {
            cli_storage_market::StorageMarketCommands::CompileDeal { intent_id, acceptance_id, api_url }
        }
        StorageMarketCommands::ActivateDeal { deal_id, merkle_root, chunks, api_url } => {
            cli_storage_market::StorageMarketCommands::ActivateDeal { deal_id, merkle_root, chunks, api_url }
        }
        StorageMarketCommands::ListDeals { client, provider, status, limit, api_url } => {
            cli_storage_market::StorageMarketCommands::ListDeals { client, provider, status, limit, api_url }
        }
        StorageMarketCommands::GetDeal { id, api_url } => {
            cli_storage_market::StorageMarketCommands::GetDeal { id, api_url }
        }
        StorageMarketCommands::GetChallenges { deal_id, epoch, api_url } => {
            cli_storage_market::StorageMarketCommands::GetChallenges { deal_id, epoch, api_url }
        }
        StorageMarketCommands::Stats { api_url } => {
            cli_storage_market::StorageMarketCommands::Stats { api_url }
        }
        StorageMarketCommands::ProviderStats { address, api_url } => {
            cli_storage_market::StorageMarketCommands::ProviderStats { address, api_url }
        }
        StorageMarketCommands::ClientStats { address, api_url } => {
            cli_storage_market::StorageMarketCommands::ClientStats { address, api_url }
        }
    };

    cli_storage_market::handle_storage_market_command(cli_cmd).await
}
