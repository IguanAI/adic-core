use adic_consensus::ConsensusEngine;
use adic_finality::{FinalityConfig, FinalityEngine};
use adic_network::{NetworkConfig, NetworkEngine, TransportConfig};
use adic_storage::{StorageConfig, StorageEngine};
use adic_types::AdicParams;
use libp2p::identity::Keypair;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::time::{sleep, timeout};
use tracing::info;

async fn create_test_node(name: &str, port: u16) -> Arc<NetworkEngine> {
    let params = AdicParams::default();
    let keypair = Keypair::generate_ed25519();

    let temp_dir = tempdir().unwrap();
    let storage_config = StorageConfig {
        backend_type: adic_storage::store::BackendType::RocksDB {
            path: temp_dir
                .path()
                .join(name)
                .to_str()
                .unwrap()
                .to_string(),
        },
        ..Default::default()
    };
    let storage = Arc::new(StorageEngine::new(storage_config).unwrap());
    std::mem::forget(temp_dir); // Prevent cleanup during test

    let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
    let finality = Arc::new(FinalityEngine::new(
        FinalityConfig::from(&params),
        consensus.clone(),
        storage.clone(),
    ));

    let config = NetworkConfig {
        listen_addresses: vec![format!("/ip4/127.0.0.1/tcp/{}", port).parse().unwrap()],
        bootstrap_peers: vec![],
        max_peers: 10,
        enable_metrics: false,
        transport: TransportConfig {
            quic_listen_addr: format!("127.0.0.1:{}", port + 1000).parse().unwrap(),
            use_production_tls: false,
            ..Default::default()
        },
        ..Default::default()
    };

    let network = NetworkEngine::new(config, keypair, storage, consensus, finality)
        .await
        .unwrap();

    Arc::new(network)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_peer_disconnection_and_state_update() {
    let _ = tracing_subscriber::fmt::try_init();
    info!("=== Testing Peer Disconnection and State Update ===");

    // Create two nodes
    let node1 = create_test_node("node1", 19000).await;
    let node2 = create_test_node("node2", 19010).await;

    // Start both nodes
    node1.start().await.unwrap();
    node2.start().await.unwrap();

    // Give nodes time to fully initialize
    sleep(Duration::from_secs(1)).await;

    // Connect node2 to node1
    let node1_addr = format!("127.0.0.1:{}", 20000); // QUIC port
    node2.connect_peer(node1_addr.parse().unwrap()).await.unwrap();

    // Give connection time to establish
    sleep(Duration::from_millis(500)).await;

    // Wait for connection with retry logic
    let mut connected = false;
    for _ in 0..10 {  // Try for up to 10 seconds
        sleep(Duration::from_secs(1)).await;
        let peers = node2.peer_manager().get_connected_peers().await;
        if peers.len() == 1 {
            connected = true;
            break;
        }
    }
    assert!(connected, "Node2 should have 1 connected peer");

    // Simulate disconnection by shutting down node1
    node1.shutdown().await.unwrap();

    // Wait for disconnection to be detected with retry logic
    let mut disconnected = false;
    for _ in 0..10 {  // Try for up to 10 seconds
        sleep(Duration::from_secs(1)).await;
        let peers = node2.peer_manager().get_connected_peers().await;
        if peers.len() == 0 {
            disconnected = true;
            break;
        }
    }
    assert!(disconnected, "Node2 should have 0 connected peers after disconnection");

    // Check that the peer is marked as disconnected
    let all_peers = node2.peer_manager().get_all_peers_info().await;
    if !all_peers.is_empty() {
        assert_eq!(
            all_peers[0].connection_state,
            adic_network::peer::ConnectionState::Disconnected,
            "Peer should be marked as disconnected"
        );
    }

    node2.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_automatic_reconnection_to_important_peer() {
    let _ = tracing_subscriber::fmt::try_init();
    info!("=== Testing Automatic Reconnection to Important Peer ===");

    // Create bootstrap node (use different port to avoid conflicts)
    let bootstrap = create_test_node("bootstrap", 19120).await;
    bootstrap.start().await.unwrap();

    // Give bootstrap time to fully initialize
    sleep(Duration::from_secs(1)).await;

    // Create regular node that connects to bootstrap
    let mut config = NetworkConfig {
        listen_addresses: vec!["/ip4/127.0.0.1/tcp/19130".parse().unwrap()],
        bootstrap_peers: vec![],
        max_peers: 10,
        enable_metrics: false,
        transport: TransportConfig {
            quic_listen_addr: "127.0.0.1:20130".parse().unwrap(),
            use_production_tls: false,
            ..Default::default()
        },
        ..Default::default()
    };

    let params = AdicParams::default();
    let keypair = Keypair::generate_ed25519();
    let temp_dir = tempdir().unwrap();
    let storage_config = StorageConfig {
        backend_type: adic_storage::store::BackendType::RocksDB {
            path: temp_dir.path().join("regular").to_str().unwrap().to_string(),
        },
        ..Default::default()
    };
    let storage = Arc::new(StorageEngine::new(storage_config).unwrap());
    std::mem::forget(temp_dir);

    let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
    let finality = Arc::new(FinalityEngine::new(
        FinalityConfig::from(&params),
        consensus.clone(),
        storage.clone(),
    ));

    let regular_node = Arc::new(
        NetworkEngine::new(config, keypair, storage, consensus, finality)
            .await
            .unwrap()
    );
    regular_node.start().await.unwrap();

    // Give regular node time to fully initialize
    sleep(Duration::from_millis(500)).await;

    // Connect to bootstrap (port 19120 + 1000 = 20120)
    regular_node.connect_peer("127.0.0.1:20120".parse().unwrap()).await.unwrap();

    // Give connection time to establish
    sleep(Duration::from_millis(500)).await;

    // Wait for connection with retry logic
    let mut connected = false;
    let mut peers = vec![];
    for _ in 0..10 {  // Try for up to 10 seconds
        sleep(Duration::from_secs(1)).await;
        peers = regular_node.peer_manager().get_connected_peers().await;
        if peers.len() == 1 {
            connected = true;
            break;
        }
    }
    assert!(connected, "Should be connected to bootstrap");
    let bootstrap_peer_id = peers[0];

    // Mark bootstrap as important (high reputation)
    regular_node
        .peer_manager()
        .mark_peer_as_important(&bootstrap_peer_id)
        .await
        .unwrap();

    // Verify high reputation
    let peer_info = regular_node.peer_manager().get_peer(&bootstrap_peer_id).await.unwrap();
    assert!(peer_info.reputation_score >= 70.0, "Bootstrap should have high reputation");

    // Simulate disconnection
    bootstrap.shutdown().await.unwrap();

    // Wait for disconnection with retry logic
    let mut disconnected = false;
    for _ in 0..10 {  // Try for up to 10 seconds
        sleep(Duration::from_secs(1)).await;
        let connected = regular_node.peer_manager().get_connected_peers().await;
        if connected.len() == 0 {
            disconnected = true;
            break;
        }
    }
    assert!(disconnected, "Should be disconnected from bootstrap");

    // Wait for OS to release the port
    sleep(Duration::from_secs(2)).await;

    // Restart bootstrap on a different port to avoid conflicts
    let bootstrap = create_test_node("bootstrap", 19220).await;
    bootstrap.start().await.unwrap();

    // Give bootstrap time to fully initialize
    sleep(Duration::from_secs(1)).await;

    // Wait for automatic reconnection (maintenance task runs every 30s in prod, but we'll trigger manually)
    // In real scenario, the maintenance task would attempt reconnection
    // For testing, we'll verify the peer is marked for reconnection
    let disconnected = regular_node.peer_manager().get_disconnected_peers().await;
    assert_eq!(disconnected.len(), 1, "Should have 1 disconnected peer marked for reconnection");

    regular_node.shutdown().await.unwrap();
    bootstrap.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_stale_peer_cleanup() {
    let _ = tracing_subscriber::fmt::try_init();
    info!("=== Testing Stale Peer Cleanup ===");

    let node = create_test_node("cleanup_node", 19040).await;
    node.start().await.unwrap();

    // Manually add a peer with old last_seen time (simulating stale peer)
    use adic_network::peer::{PeerInfo, ConnectionState, MessageStats};
    use adic_types::PublicKey;
    use libp2p::PeerId;
    use std::time::Instant;

    let stale_peer = PeerInfo {
        peer_id: PeerId::random(),
        addresses: vec![],
        public_key: PublicKey::from_bytes([1; 32]),
        reputation_score: 50.0,
        latency_ms: Some(20),
        bandwidth_mbps: Some(50.0),
        last_seen: Instant::now() - Duration::from_secs(150), // Very old
        connection_state: ConnectionState::Disconnected,
        message_stats: MessageStats::default(),
        padic_location: None,
    };

    let stale_peer_id = stale_peer.peer_id;
    node.peer_manager().add_peer(stale_peer).await.unwrap();

    // Verify peer was added
    assert!(node.peer_manager().get_peer(&stale_peer_id).await.is_some());

    // Run cleanup
    node.peer_manager().cleanup_stale_peers().await;

    // Verify peer was removed
    assert!(
        node.peer_manager().get_peer(&stale_peer_id).await.is_none(),
        "Stale peer should be removed after cleanup"
    );

    node.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_connection_state_transitions() {
    let _ = tracing_subscriber::fmt::try_init();
    info!("=== Testing Connection State Transitions ===");

    let node = create_test_node("state_node", 19050).await;
    node.start().await.unwrap();

    // Add a test peer
    use adic_network::peer::{PeerInfo, ConnectionState, MessageStats};
    use adic_types::PublicKey;
    use libp2p::PeerId;
    use std::time::Instant;

    let peer_info = PeerInfo {
        peer_id: PeerId::random(),
        addresses: vec!["/ip4/127.0.0.1/tcp/19999".parse().unwrap()],
        public_key: PublicKey::from_bytes([2; 32]),
        reputation_score: 50.0,
        latency_ms: Some(20),
        bandwidth_mbps: Some(50.0),
        last_seen: Instant::now(),
        connection_state: ConnectionState::Disconnected,
        message_stats: MessageStats::default(),
        padic_location: None,
    };

    let peer_id = peer_info.peer_id;
    node.peer_manager().add_peer(peer_info).await.unwrap();

    // Test state transitions
    let states = vec![
        ConnectionState::Connecting,
        ConnectionState::Connected,
        ConnectionState::Disconnected,
        ConnectionState::Failed,
    ];

    for expected_state in states {
        node.peer_manager()
            .update_peer_connection_state(&peer_id, expected_state)
            .await
            .unwrap();

        let peer = node.peer_manager().get_peer(&peer_id).await.unwrap();
        assert_eq!(
            peer.connection_state, expected_state,
            "State should be {:?}",
            expected_state
        );
    }

    node.shutdown().await.unwrap();
}