use adic_consensus::ConsensusEngine;
use adic_crypto::Keypair as AdicKeypair;
use adic_finality::{FinalityConfig, FinalityEngine};
use adic_network::{NetworkConfig, NetworkEngine, TransportConfig};
use adic_storage::{StorageConfig, StorageEngine};
use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AdicParams, AxisPhi, MessageId, QpDigits, DEFAULT_P,
    DEFAULT_PRECISION,
};
use chrono::Utc;
use libp2p::identity::Keypair;
use std::env;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::{sleep, timeout};
use tracing::{debug, info, warn};

/// A test node with all required components
struct TestNode {
    id: String,
    keypair: AdicKeypair,
    storage: Arc<StorageEngine>,
    consensus: Arc<ConsensusEngine>,
    finality: Arc<FinalityEngine>,
    network: Arc<RwLock<NetworkEngine>>,
    _tcp_port: u16,
    _quic_port: u16,
}

impl TestNode {
    async fn new(node_id: u8, tcp_port: u16, _quic_port: u16) -> Self {
        info!(
            "Creating test node {} on TCP:{} (QUIC dynamic)",
            node_id, tcp_port
        );

        // Generate keys
        let adic_keypair = AdicKeypair::generate();
        let libp2p_keypair = Keypair::generate_ed25519();

        // Create storage - use in-memory to avoid RocksDB file locks in concurrent tests
        let storage = Arc::new(
            StorageEngine::new(StorageConfig {
                backend_type: adic_storage::store::BackendType::Memory,
                ..Default::default()
            })
            .unwrap(),
        );

        // Create consensus with permissive params suitable for small test
        let params = AdicParams {
            k: 1,           // minimal core size
            depth_star: 0,  // allow shallow depth
            q: 1,           // minimal diversity
            r_sum_min: 0.0, // no reputation sum threshold
            ..Default::default()
        };
        let consensus = Arc::new(ConsensusEngine::new(params.clone()));

        // Create finality engine
        let finality = Arc::new(FinalityEngine::new(
            FinalityConfig::from(&params),
            consensus.clone(),
        ));

        // Create network config
        let config = NetworkConfig {
            listen_addresses: vec![format!("/ip4/127.0.0.1/tcp/{}", tcp_port).parse().unwrap()],
            bootstrap_peers: vec![],
            max_peers: 10,
            enable_metrics: false,
            transport: TransportConfig {
                // Let OS choose a free port to avoid collisions in CI/dev
                quic_listen_addr: "127.0.0.1:0".parse().unwrap(),
                libp2p_listen_addrs: vec![format!("/ip4/127.0.0.1/tcp/{}", tcp_port)
                    .parse()
                    .unwrap()],
                ..Default::default()
            },
            ..Default::default()
        };

        // Create network engine
        let network = NetworkEngine::new(
            config,
            libp2p_keypair,
            storage.clone(),
            consensus.clone(),
            finality.clone(),
        )
        .await
        .unwrap();

        Self {
            id: format!("node-{}", node_id),
            keypair: adic_keypair,
            storage,
            consensus,
            finality,
            network: Arc::new(RwLock::new(network)),
            _tcp_port: tcp_port,
            _quic_port: 0,
        }
    }

    async fn actual_quic_port(&self) -> u16 {
        // Inspect transport endpoint to learn the bound port
        let net = self.network.read().await;
        net.local_quic_port().await.unwrap_or(0)
    }

    async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("{}: Starting network", self.id);
        self.network.read().await.start().await?;
        Ok(())
    }

    async fn connect_to(&self, other_quic_port: u16) -> Result<(), Box<dyn std::error::Error>> {
        let addr = format!("127.0.0.1:{}", other_quic_port).parse().unwrap();
        info!("{}: Connecting to QUIC port {}", self.id, other_quic_port);
        self.network.read().await.connect_peer(addr).await?;
        Ok(())
    }

    async fn submit_message(
        &self,
        content: Vec<u8>,
    ) -> Result<MessageId, Box<dyn std::error::Error>> {
        // Create message with proper features
        let features = AdicFeatures::new(vec![
            AxisPhi::new(
                0,
                QpDigits::from_u64(Utc::now().timestamp() as u64, DEFAULT_P, DEFAULT_PRECISION),
            ),
            AxisPhi::new(1, QpDigits::from_u64(0, DEFAULT_P, DEFAULT_PRECISION)),
            AxisPhi::new(2, QpDigits::from_u64(0, DEFAULT_P, DEFAULT_PRECISION)),
        ]);

        // Get tips and select parents (simplified - just use all tips for now)
        let tips = self.storage.get_tips().await?;
        let parents = if tips.is_empty() {
            info!("{}: No tips found, creating genesis-like message", self.id);
            vec![] // Genesis message
        } else {
            info!("{}: Found {} tips for parents", self.id, tips.len());
            tips.into_iter().take(4).collect() // Take up to d+1 parents
        };

        // Create and sign message
        let mut message = AdicMessage::new(
            parents.clone(),
            features,
            AdicMeta::new(Utc::now()),
            *self.keypair.public_key(),
            content,
        );
        message.signature = self.keypair.sign(&message.to_bytes());

        // Escrow deposit
        self.consensus
            .deposits
            .escrow(message.id, *self.keypair.public_key())
            .await?;

        // Validate
        let validation = self.consensus.validate_and_slash(&message).await?;
        if !validation.is_valid {
            return Err(format!("Message validation failed: {:?}", validation.errors).into());
        }

        // DO NOT store message locally - let it propagate through the network
        // The network will store it when it receives it back

        // Broadcast to network (network will handle storage and finality)
        self.network
            .read()
            .await
            .broadcast_message(message.clone())
            .await?;

        info!(
            "{}: Submitted message {} with proposer {}",
            self.id,
            hex::encode(&message.id.as_bytes()[..8]),
            hex::encode(&message.proposer_pk.as_bytes()[..8])
        );

        Ok(message.id)
    }

    async fn check_finality(&self) -> Result<Vec<MessageId>, Box<dyn std::error::Error>> {
        let finalized = self.finality.check_finality().await?;
        if !finalized.is_empty() {
            info!("{}: Finalized {} messages", self.id, finalized.len());
            for msg_id in &finalized {
                debug!(
                    "{}: Message {} is final",
                    self.id,
                    hex::encode(&msg_id.as_bytes()[..8])
                );
            }
        }
        Ok(finalized)
    }

    async fn get_stats(&self) -> (usize, usize, usize) {
        let storage_stats = self.storage.get_stats().await.unwrap_or_default();
        let finality_stats = self.finality.get_stats().await;

        (
            storage_stats.message_count,
            storage_stats.tip_count,
            finality_stats.finalized_count,
        )
    }

    async fn shutdown(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("{}: Shutting down", self.id);
        self.network.read().await.shutdown().await?;
        Ok(())
    }
}

/// Get test port range from environment
fn get_test_ports() -> (u16, u16, u16, u16, u16, u16) {
    let base = env::var("ADIC_TEST_PORT_MIN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6960);

    (
        base,
        base + 1, // Node 0: TCP, QUIC
        base + 2,
        base + 3, // Node 1: TCP, QUIC
        base + 4,
        base + 5, // Node 2: TCP, QUIC
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_three_node_finality() {
    // Initialize logging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("adic_network=debug,three_node_finality_test=info")
        .try_init();

    info!("Starting 3-node finality test");

    // Get ports
    let (tcp0, quic0, tcp1, quic1, tcp2, quic2) = get_test_ports();

    // Create three nodes
    let node0 = Arc::new(TestNode::new(0, tcp0, quic0).await);
    let node1 = Arc::new(TestNode::new(1, tcp1, quic1).await);
    let node2 = Arc::new(TestNode::new(2, tcp2, quic2).await);

    // Start all nodes
    node0.start().await.expect("Failed to start node 0");
    node1.start().await.expect("Failed to start node 1");
    node2.start().await.expect("Failed to start node 2");

    // Give nodes time to initialize
    sleep(Duration::from_millis(500)).await;

    // Connect nodes in a ring: 0->1, 1->2, 2->0
    let quic1 = node1.actual_quic_port().await;
    match timeout(Duration::from_secs(5), node0.connect_to(quic1)).await {
        Ok(Ok(_)) => info!("Node 0 connected to Node 1"),
        Ok(Err(e)) => warn!("Node 0 failed to connect to Node 1: {}", e),
        Err(_) => warn!("Node 0 connection to Node 1 timed out"),
    }

    sleep(Duration::from_millis(500)).await;

    let quic2 = node2.actual_quic_port().await;
    match timeout(Duration::from_secs(5), node1.connect_to(quic2)).await {
        Ok(Ok(_)) => info!("Node 1 connected to Node 2"),
        Ok(Err(e)) => warn!("Node 1 failed to connect to Node 2: {}", e),
        Err(_) => warn!("Node 1 connection to Node 2 timed out"),
    }

    sleep(Duration::from_millis(500)).await;

    let quic0 = node0.actual_quic_port().await;
    match timeout(Duration::from_secs(5), node2.connect_to(quic0)).await {
        Ok(Ok(_)) => info!("Node 2 connected to Node 0"),
        Ok(Err(e)) => warn!("Node 2 failed to connect to Node 0: {}", e),
        Err(_) => warn!("Node 2 connection to Node 0 timed out"),
    }

    sleep(Duration::from_millis(500)).await;

    info!("Ring network topology established");

    // Submit genesis message from node 0 ONLY
    let genesis_id = node0
        .submit_message(b"Genesis".to_vec())
        .await
        .expect("Failed to submit genesis");
    info!(
        "Genesis message: {}",
        hex::encode(&genesis_id.as_bytes()[..8])
    );

    // Wait for genesis to propagate
    sleep(Duration::from_secs(2)).await;

    // Submit messages from each node to build the DAG
    let mut message_ids = vec![genesis_id];

    for round in 0..5 {
        info!("Round {}: Submitting messages", round);

        // Each node submits a message (but not another genesis)
        for (i, node) in [&node0, &node1, &node2].iter().enumerate() {
            // Skip the first message for nodes 1 and 2 in round 0 to avoid genesis conflicts
            if round == 0 && i > 0 {
                continue;
            }

            let content = format!("Round {} from Node {}", round, i);
            match node.submit_message(content.into_bytes()).await {
                Ok(msg_id) => {
                    message_ids.push(msg_id);
                    info!("Node {} submitted message in round {}", i, round);
                }
                Err(e) => {
                    warn!("Node {} failed to submit in round {}: {}", i, round, e);
                }
            }

            // Small delay between submissions
            sleep(Duration::from_millis(200)).await;
        }

        // Check finality after each round
        sleep(Duration::from_millis(500)).await;

        for (i, node) in [&node0, &node1, &node2].iter().enumerate() {
            let finalized = node.check_finality().await.unwrap_or_default();
            if !finalized.is_empty() {
                info!(
                    "Node {} has {} finalized messages after round {}",
                    i,
                    finalized.len(),
                    round
                );
            }
        }

        // Give time for propagation
        sleep(Duration::from_secs(1)).await;
    }

    info!("All messages submitted, checking final state");

    // Final finality check
    sleep(Duration::from_secs(2)).await;

    let mut total_finalized = 0;
    for (i, node) in [&node0, &node1, &node2].iter().enumerate() {
        let (msg_count, tip_count, finalized_count) = node.get_stats().await;
        info!(
            "Node {} final stats: {} messages, {} tips, {} finalized",
            i, msg_count, tip_count, finalized_count
        );
        total_finalized += finalized_count;
    }

    // Verify that at least some messages achieved finality
    assert!(
        total_finalized > 0,
        "No messages achieved finality across any node"
    );

    info!(
        "Test completed successfully! Total finalized messages: {}",
        total_finalized / 3
    );

    // Shutdown nodes
    for node in [&node0, &node1, &node2] {
        node.shutdown().await.ok();
    }
}
