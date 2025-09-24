use adic_consensus::ConsensusEngine;
use adic_finality::{FinalityConfig, FinalityEngine};
use adic_network::{NetworkConfig, NetworkEngine, TransportConfig};
use adic_storage::{StorageConfig, StorageEngine};
use adic_types::AdicParams;
use libp2p::identity::Keypair;
use std::sync::Arc;
use tempfile::tempdir;
use tracing::info;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_single_node_creation() {
    let _ = tracing_subscriber::fmt::try_init();
    info!("=== Starting Single Node Creation Test ===");

    // Simple parameters
    let params = AdicParams::default();

    info!("Creating keypair...");
    let keypair = Keypair::generate_ed25519();

    info!("Setting up storage...");
    let temp_dir = tempdir().unwrap();
    let storage_config = StorageConfig {
        backend_type: adic_storage::store::BackendType::RocksDB {
            path: temp_dir
                .path()
                .join("test_node")
                .to_str()
                .unwrap()
                .to_string(),
        },
        ..Default::default()
    };
    let storage = Arc::new(StorageEngine::new(storage_config).unwrap());
    std::mem::forget(temp_dir); // Prevent cleanup during test

    info!("Creating consensus engine...");
    let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));

    info!("Creating finality engine...");
    let finality = Arc::new(FinalityEngine::new(
        FinalityConfig::from(&params),
        consensus.clone(),
        storage.clone(),
    ));

    info!("Creating network config...");
    let config = NetworkConfig {
        listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
        bootstrap_peers: vec![],
        max_peers: 5,
        enable_metrics: false,
        transport: TransportConfig {
            quic_listen_addr: "127.0.0.1:0".parse().unwrap(),
            use_production_tls: false,
            ..Default::default()
        },
        ..Default::default()
    };

    info!("Creating NetworkEngine...");
    let network = NetworkEngine::new(config, keypair, storage, consensus, finality).await;

    info!("NetworkEngine creation result: {:?}", network.is_ok());

    if let Ok(net) = network {
        let net = Arc::new(net);
        info!("Starting network engine...");
        let start_result = net.start().await;
        info!("Start result: {:?}", start_result.is_ok());

        if start_result.is_ok() {
            info!("Network started successfully");

            // Get some basic stats
            let stats = net.get_network_stats().await;
            info!("Network stats: {:?}", stats);
        }
    }

    info!("=== Single Node Creation Test Complete ===");
}
