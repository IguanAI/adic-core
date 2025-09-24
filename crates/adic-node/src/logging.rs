use crate::config::LoggingConfig;
use tracing::Level;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

/// Emoji mappings for different modules and operations
#[allow(dead_code)]
pub struct EmojiMap;

impl EmojiMap {
    /// Get emoji for a module
    #[allow(dead_code)]
    pub fn module(module: &str) -> &'static str {
        match module {
            m if m.contains("network") || m.contains("p2p") => "🌐",
            m if m.contains("consensus") || m.contains("energy") => "⚡",
            m if m.contains("economic") || m.contains("balance") => "💎",
            m if m.contains("storage") || m.contains("rocksdb") => "🗄️",
            m if m.contains("crypto") || m.contains("security") => "🔐",
            m if m.contains("api") || m.contains("rpc") => "📡",
            m if m.contains("finality") => "🏁",
            m if m.contains("genesis") || m.contains("init") => "🧬",
            m if m.contains("config") => "⚙️",
            _ => "🚀",
        }
    }

    /// Get emoji for status/action
    #[allow(dead_code)]
    pub fn status(action: &str) -> &'static str {
        let action_lower = action.to_lowercase();
        match action_lower {
            s if s.contains("success") || s.contains("complete") || s.contains("ready") => "✅",
            s if s.contains("wait") || s.contains("progress") || s.contains("loading") => "⏳",
            s if s.contains("warn") => "⚠️",
            s if s.contains("error") || s.contains("fail") => "❌",
            s if s.contains("retry") || s.contains("reconnect") => "🔄",
            s if s.contains("metric") || s.contains("stat") => "📊",
            s if s.contains("discover") || s.contains("search") || s.contains("find") => "🔍",
            s if s.contains("handshake") || s.contains("connect") => "🤝",
            s if s.contains("message") || s.contains("data") || s.contains("packet") => "📦",
            s if s.contains("target") || s.contains("goal") => "🎯",
            _ => "",
        }
    }

    /// Format a log message with appropriate emoji
    #[allow(dead_code)]
    pub fn format_with_emoji(_level: &Level, module: &str, message: &str) -> String {
        let module_emoji = Self::module(module);
        let status_emoji = Self::status(message);

        let emoji = if !status_emoji.is_empty() {
            status_emoji
        } else {
            module_emoji
        };

        format!("{} {}", emoji, message)
    }
}

/// Display the emoji legend
pub fn display_emoji_legend() {
    println!("\n╔══════════════════════════════════════════════════════════════════╗");
    println!("║                         EMOJI LEGEND                             ║");
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║ MODULE INDICATORS                                                ║");
    println!("║ 🌐 Network/P2P    ⚡ Consensus     💎 Economics    💾 Storage     ║");
    println!("║ 🔐 Security       📡 API/RPC      🏁 Finality     🧬 Genesis     ║");
    println!("║ ⚙️ Configuration  🚀 Boot/Startup  🗄️ Database                   ║");
    println!("║                                                                  ║");
    println!("║ OPERATION INDICATORS                                             ║");
    println!("║ 💰 Credit/Add     💸 Debit/Remove  📝 Transaction  🔄 State Change║");
    println!("║ 🔒 Lock/Secure    🔓 Unlock/Release 🔗 Connection  🤝 Handshake  ║");
    println!("║ 🔍 Search/Query   📦 Data/Message  🎯 Target Met   📊 Metrics    ║");
    println!("║                                                                  ║");
    println!("║ STATUS INDICATORS                                                ║");
    println!("║ ✅ Success        ⏳ In Progress   ⚠️ Warning      ❌ Error        ║");
    println!("║ 🛑 Shutdown       ✨ Initialized   🔁 Retry        ⏸️  Paused      ║");
    println!("╚══════════════════════════════════════════════════════════════════╝\n");
}

/// Display the dramatic boot banner
pub fn display_boot_banner(version: &str) {
    println!("\n╔══════════════════════════════════════════════════════════════════╗");
    println!("║                                                                  ║");
    println!("║     █████╗ ██████╗ ██╗ ██████╗    ███╗   ██╗ ██████╗ ██████╗   ║");
    println!("║    ██╔══██╗██╔══██╗██║██╔════╝    ████╗  ██║██╔═══██╗██╔══██╗  ║");
    println!("║    ███████║██║  ██║██║██║         ██╔██╗ ██║██║   ██║██║  ██║  ║");
    println!("║    ██╔══██║██║  ██║██║██║         ██║╚██╗██║██║   ██║██║  ██║  ║");
    println!("║    ██║  ██║██████╔╝██║╚██████╗    ██║ ╚████║╚██████╔╝██████╔╝  ║");
    println!("║    ╚═╝  ╚═╝╚═════╝ ╚═╝ ╚═════╝    ╚═╝  ╚═══╝ ╚═════╝ ╚═════╝   ║");
    println!("║                                                                  ║");
    println!("║              Adaptive Distributed Intelligence Chain             ║");
    println!(
        "║                         Version {}                           ║",
        format_version_centered(version)
    );
    println!("╚══════════════════════════════════════════════════════════════════╝");
}

fn format_version_centered(version: &str) -> String {
    let width = 9; // Available space for version
    let _padding = (width - version.len()) / 2;
    format!("{:^width$}", version, width = width)
}

/// Initialize the logging system based on configuration
pub fn init_logging(config: &LoggingConfig, cli_verbose: u8) -> anyhow::Result<()> {
    // Determine log level from config or CLI
    let log_level = if cli_verbose > 0 {
        match cli_verbose {
            1 => "debug",
            _ => "trace",
        }
    } else {
        &config.level
    };

    // Build the env filter with module-specific filters
    let mut filter =
        EnvFilter::new(std::env::var("RUST_LOG").unwrap_or_else(|_| format!("adic={}", log_level)));

    // Add module-specific filters
    for (module, level) in &config.module_filters {
        filter = filter.add_directive(format!("{}={}", module, level).parse()?);
    }

    // Create the base subscriber
    let subscriber = tracing_subscriber::registry().with(filter);

    // Configure the format layer based on config
    match config.format.as_str() {
        "json" => {
            // JSON format for machine parsing
            let json_layer = fmt::layer()
                .json()
                .with_current_span(true)
                .with_span_list(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_line_number(true)
                .with_file(true);

            if let Some(file_path) = &config.file_output {
                // Add file output
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(file_path)?;

                let file_layer = fmt::layer().json().with_writer(file).with_ansi(false);

                subscriber.with(json_layer).with(file_layer).init();
            } else {
                subscriber.with(json_layer).init();
            }
        }
        "compact" => {
            // Compact format for minimal output
            let compact_layer = fmt::layer()
                .compact()
                .with_target(false)
                .with_thread_ids(false)
                .with_thread_names(false)
                .with_line_number(false)
                .with_file(false);

            if let Some(file_path) = &config.file_output {
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(file_path)?;

                let file_layer = fmt::layer().compact().with_writer(file).with_ansi(false);

                subscriber.with(compact_layer).with(file_layer).init();
            } else {
                subscriber.with(compact_layer).init();
            }
        }
        _ => {
            // Default "pretty" format with optional emojis
            // Determine whether to show source location based on log level
            let show_location = matches!(log_level, "debug" | "trace");

            let pretty_layer = fmt::layer()
                .with_target(show_location)
                .with_thread_ids(false)
                .with_thread_names(false)
                .with_line_number(show_location)
                .with_file(show_location);

            if let Some(file_path) = &config.file_output {
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(file_path)?;

                let file_layer = fmt::layer().with_writer(file).with_ansi(false);

                subscriber.with(pretty_layer).with(file_layer).init();
            } else {
                subscriber.with(pretty_layer).init();
            }
        }
    }

    Ok(())
}

// ============================================================================
// Helper Macros for Structured Logging
// ============================================================================

/// Log a state transition with automatic context
#[macro_export]
macro_rules! log_state_change {
    ($old:expr, $new:expr, $($field:tt)*) => {
        tracing::info!(
            old_state = ?$old,
            new_state = ?$new,
            $($field)*
        )
    };
}

/// Log an operation with performance metrics
#[macro_export]
macro_rules! log_with_metrics {
    ($op:expr, $duration:expr, $($field:tt)*) => {
        tracing::info!(
            operation = $op,
            duration_ms = $duration.as_millis() as u64,
            $($field)*
        )
    };
}

/// Log a network event with peer context
#[macro_export]
macro_rules! log_network_event {
    ($peer_id:expr, $event:expr, $($field:tt)*) => {
        tracing::info!(
            peer_id = %$peer_id,
            event = $event,
            $($field)*
        )
    };
}

/// Log an economic transaction with balance context
#[macro_export]
macro_rules! log_transaction {
    ($from:expr, $to:expr, $amount:expr, $($field:tt)*) => {
        tracing::info!(
            from = %$from,
            to = %$to,
            amount = $amount,
            $($field)*
        )
    };
}

/// Log a consensus event with DAG context
#[macro_export]
macro_rules! log_consensus_event {
    ($event:expr, $height:expr, $($field:tt)*) => {
        tracing::info!(
            consensus_event = $event,
            dag_height = $height,
            $($field)*
        )
    };
}

/// Log storage operations with size/performance context
#[macro_export]
macro_rules! log_storage_op {
    ($op:expr, $key:expr, $size:expr, $($field:tt)*) => {
        tracing::info!(
            storage_op = $op,
            key = %$key,
            size_bytes = $size,
            $($field)*
        )
    };
}

/// Log with automatic module emoji based on module path
#[macro_export]
macro_rules! info_with_emoji {
    ($($arg:tt)*) => {
        {
            let message = format!($($arg)*);
            let module = module_path!();
            let emoji = $crate::logging::EmojiMap::module(module);
            tracing::info!("{} {}", emoji, message);
        }
    };
}

/// Debug log with structured fields
#[macro_export]
macro_rules! debug_with_context {
    ($msg:expr, $($field:tt)*) => {
        tracing::debug!(
            $($field)*,
            $msg
        )
    };
}
