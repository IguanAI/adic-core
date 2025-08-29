use adic_consensus::ConsensusEngine;
use adic_finality::{FinalityConfig, FinalityEngine};
use adic_network::{NetworkConfig, NetworkEngine, TransportConfig};
use adic_storage::{StorageConfig, StorageEngine};
use adic_types::AdicParams;
use libp2p::identity::Keypair;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test(flavor = "current_thread")]
async fn test_network_creation_deadlock() {
    println!("DEADLOCK TEST - Starting");

    // Minimal setup
    let params = AdicParams::default();
    let keypair = Keypair::generate_ed25519();

    println!("DEADLOCK TEST - Creating storage");
    let temp_dir = tempdir().unwrap();
    let storage_config = StorageConfig {
        backend_type: adic_storage::store::BackendType::RocksDB {
            path: temp_dir.path().join("test").to_str().unwrap().to_string(),
        },
        ..Default::default()
    };
    let storage = Arc::new(StorageEngine::new(storage_config).unwrap());
    std::mem::forget(temp_dir);

    println!("DEADLOCK TEST - Creating consensus");
    let consensus = Arc::new(ConsensusEngine::new(params.clone()));

    println!("DEADLOCK TEST - Creating finality");
    let finality = Arc::new(FinalityEngine::new(
        FinalityConfig::from(&params),
        consensus.clone(),
    ));

    println!("DEADLOCK TEST - Creating config");
    let config = NetworkConfig {
        listen_addresses: vec![],
        bootstrap_peers: vec![],
        max_peers: 1,
        enable_metrics: false,
        transport: TransportConfig {
            quic_listen_addr: "127.0.0.1:0".parse().unwrap(),
            use_production_tls: false,
            ..Default::default()
        },
        ..Default::default()
    };

    println!("DEADLOCK TEST - About to create NetworkEngine");

    // Set a timeout for NetworkEngine creation
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        NetworkEngine::new(config, keypair, storage, consensus, finality),
    )
    .await;

    match result {
        Ok(Ok(_)) => println!("DEADLOCK TEST - NetworkEngine created successfully"),
        Ok(Err(e)) => println!("DEADLOCK TEST - NetworkEngine creation failed: {}", e),
        Err(_) => {
            println!("DEADLOCK TEST - TIMEOUT! NetworkEngine creation took more than 5 seconds")
        }
    }

    println!("DEADLOCK TEST - Complete");
}
