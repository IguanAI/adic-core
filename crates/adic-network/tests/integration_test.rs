use adic_consensus::ConsensusEngine;
use adic_crypto::Keypair as AdicKeypair;
use adic_finality::{FinalityConfig, FinalityEngine};
use adic_network::{NetworkConfig, NetworkEngine};
use adic_storage::{StorageConfig, StorageEngine};
use adic_types::AdicParams;
use libp2p::identity::Keypair;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_network_engine_creation() {
    // Create components
    let keypair = Keypair::generate_ed25519();
    // Use temp directory to avoid RocksDB lock conflicts
    let temp_dir = tempdir().unwrap();
    let storage_config = StorageConfig {
        backend_type: adic_storage::store::BackendType::RocksDB {
            path: temp_dir
                .path()
                .join("storage")
                .to_str()
                .unwrap()
                .to_string(),
        },
        ..Default::default()
    };
    let storage = Arc::new(StorageEngine::new(storage_config).unwrap());
    let params = AdicParams::default();
    let consensus = Arc::new(ConsensusEngine::new(params.clone()));
    let finality = Arc::new(FinalityEngine::new(
        FinalityConfig::from(&params),
        consensus.clone(),
    ));

    // Create network configuration with random port (0 = auto-assign)
    let config = NetworkConfig {
        listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
        bootstrap_peers: vec![],
        max_peers: 10,
        enable_metrics: false,
        transport: adic_network::TransportConfig {
            quic_listen_addr: "127.0.0.1:0".parse().unwrap(), // Random QUIC port
            ..Default::default()
        },
        ..Default::default()
    };

    // Create network engine
    let network = NetworkEngine::new(config, keypair, storage, consensus, finality).await;

    assert!(network.is_ok());
    let network = network.unwrap();
    assert_eq!(network.get_connected_peers().await.len(), 0);
}

#[tokio::test]
async fn test_network_start() {
    // Create components
    let keypair = Keypair::generate_ed25519();
    // Use temp directory to avoid RocksDB lock conflicts
    let temp_dir = tempdir().unwrap();
    let storage_config = StorageConfig {
        backend_type: adic_storage::store::BackendType::RocksDB {
            path: temp_dir
                .path()
                .join("storage")
                .to_str()
                .unwrap()
                .to_string(),
        },
        ..Default::default()
    };
    let storage = Arc::new(StorageEngine::new(storage_config).unwrap());
    let params = AdicParams::default();
    let consensus = Arc::new(ConsensusEngine::new(params.clone()));
    let finality = Arc::new(FinalityEngine::new(
        FinalityConfig::from(&params),
        consensus.clone(),
    ));

    // Create network configuration with random port (0 = auto-assign)
    let config = NetworkConfig {
        listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
        bootstrap_peers: vec![],
        max_peers: 10,
        enable_metrics: false,
        transport: adic_network::TransportConfig {
            quic_listen_addr: "127.0.0.1:0".parse().unwrap(), // Random QUIC port
            ..Default::default()
        },
        ..Default::default()
    };

    // Create and start network engine
    let network = NetworkEngine::new(config, keypair, storage, consensus, finality)
        .await
        .unwrap();

    let result = network.start().await;
    assert!(result.is_ok());

    // Shutdown cleanly
    let _ = network.shutdown().await;
}

#[tokio::test]
async fn test_message_broadcast() {
    use adic_types::{AdicFeatures, AdicMessage, AdicMeta};
    use chrono::Utc;

    // Create components
    let keypair = Keypair::generate_ed25519();
    // Use temp directory to avoid RocksDB lock conflicts
    let temp_dir = tempdir().unwrap();
    let storage_config = StorageConfig {
        backend_type: adic_storage::store::BackendType::RocksDB {
            path: temp_dir
                .path()
                .join("storage")
                .to_str()
                .unwrap()
                .to_string(),
        },
        ..Default::default()
    };
    let storage = Arc::new(StorageEngine::new(storage_config).unwrap());
    let params = AdicParams::default();
    let consensus = Arc::new(ConsensusEngine::new(params.clone()));
    let finality = Arc::new(FinalityEngine::new(
        FinalityConfig::from(&params),
        consensus.clone(),
    ));

    // Create network configuration with random port (0 = auto-assign)
    let config = NetworkConfig {
        listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
        bootstrap_peers: vec![],
        max_peers: 10,
        enable_metrics: false,
        transport: adic_network::TransportConfig {
            quic_listen_addr: "127.0.0.1:0".parse().unwrap(), // Random QUIC port
            ..Default::default()
        },
        ..Default::default()
    };

    // Create network engine
    let network = NetworkEngine::new(config, keypair, storage, consensus, finality)
        .await
        .unwrap();

    network.start().await.unwrap();

    // Create and sign a test message
    let adic_keypair = AdicKeypair::generate();
    let mut message = AdicMessage::new(
        vec![],
        AdicFeatures::new(vec![]),
        AdicMeta::new(Utc::now()),
        *adic_keypair.public_key(),
        b"test".to_vec(),
    );
    message.signature = adic_keypair.sign(&message.to_bytes());

    // Try to broadcast (should succeed even with no peers)
    let result = network.broadcast_message(message).await;
    if let Err(e) = &result {
        eprintln!("Broadcast failed: {:?}", e);
    }
    assert!(result.is_ok());

    // Shutdown cleanly
    let _ = network.shutdown().await;
}
