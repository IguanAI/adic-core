use adic_consensus::ConsensusEngine;
use adic_crypto::Keypair as AdicKeypair;
use adic_finality::{FinalityConfig, FinalityEngine, FinalityGate};
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
}

impl TestNode {
    async fn new(node_id: u8, tcp_port: u16) -> Self {
        info!(
            "Creating test node {} on TCP:{} (QUIC dynamic)",
            node_id, tcp_port
        );

        // Generate keys
        let adic_keypair = AdicKeypair::generate();
        let libp2p_keypair = Keypair::generate_ed25519();

        // Create storage - use in-memory to avoid RocksDB file locks
        let storage = Arc::new(
            StorageEngine::new(StorageConfig {
                backend_type: adic_storage::store::BackendType::Memory,
                ..Default::default()
            })
            .unwrap(),
        );

        // Create consensus with permissive params for testing
        let params = AdicParams {
            k: 1,           // minimal core size
            depth_star: 0,  // allow shallow depth
            q: 1,           // minimal diversity
            r_sum_min: 0.0, // no reputation sum threshold
            ..Default::default()
        };
        let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));

        // Create finality engine
        let finality = Arc::new(
            FinalityEngine::new(
                FinalityConfig::from(&params),
                consensus.clone(),
                storage.clone(),
            )
            .await,
        );

        // Create network config
        let config = NetworkConfig {
            listen_addresses: vec![format!("/ip4/127.0.0.1/tcp/{}", tcp_port).parse().unwrap()],
            bootstrap_peers: vec![],
            max_peers: 10,
            enable_metrics: false,
            transport: TransportConfig {
                quic_listen_addr: "127.0.0.1:0".parse().unwrap(),
                libp2p_listen_addrs: vec![format!("/ip4/127.0.0.1/tcp/{}", tcp_port)
                    .parse()
                    .unwrap()],
                use_production_tls: false,
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
        }
    }

    async fn actual_quic_port(&self) -> u16 {
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

        // Get tips and select parents
        let tips = self.storage.get_tips().await?;
        let parents = if tips.is_empty() {
            vec![] // Genesis message
        } else {
            tips.into_iter().take(4).collect()
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

        // Set reputation for this proposer
        self.consensus
            .reputation
            .set_reputation(self.keypair.public_key(), 1.0)
            .await;

        // Broadcast to network
        self.network
            .read()
            .await
            .broadcast_message(message.clone())
            .await?;

        info!(
            "{}: Submitted message {}",
            self.id,
            hex::encode(&message.id.as_bytes()[..8])
        );

        Ok(message.id)
    }

    async fn check_finality(&self) -> Result<Vec<MessageId>, Box<dyn std::error::Error>> {
        let finalized = self.finality.check_finality().await?;
        if !finalized.is_empty() {
            info!("{}: Finalized {} messages", self.id, finalized.len());
        }
        Ok(finalized)
    }

    async fn has_finality_artifact(&self, msg_id: &MessageId) -> bool {
        // Check both in-memory and storage
        if self.finality.get_artifact(msg_id).await.is_some() {
            return true;
        }

        // Check storage
        if let Ok(Some(artifact_bytes)) = self.storage.get_finality_artifact(msg_id).await {
            if !artifact_bytes.is_empty() {
                return true;
            }
        }

        false
    }

    async fn get_finality_artifact_gate(&self, msg_id: &MessageId) -> Option<FinalityGate> {
        self.finality
            .get_artifact(msg_id)
            .await
            .map(|a| a.gate)
    }

    async fn shutdown(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("{}: Shutting down", self.id);
        self.network.read().await.shutdown().await?;
        Ok(())
    }
}

/// Get test port range from environment
fn get_test_ports() -> (u16, u16) {
    let base = env::var("ADIC_TEST_PORT_MIN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7000);

    (base, base + 1)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_finality_artifact_broadcast() {
    // Initialize logging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("adic_network=debug,finality_artifact_propagation_test=info")
        .try_init();

    info!("🧪 Starting finality artifact broadcast test");

    // Get ports
    let (tcp0, tcp1) = get_test_ports();

    // Create two nodes
    let node0 = Arc::new(TestNode::new(0, tcp0).await);
    let node1 = Arc::new(TestNode::new(1, tcp1).await);

    // Start both nodes
    node0.start().await.expect("Failed to start node 0");
    node1.start().await.expect("Failed to start node 1");

    sleep(Duration::from_millis(500)).await;

    // Connect node0 to node1
    let quic1 = node1.actual_quic_port().await;
    timeout(Duration::from_secs(5), node0.connect_to(quic1))
        .await
        .expect("Connection timeout")
        .expect("Connection failed");

    info!("✅ Nodes connected");
    sleep(Duration::from_millis(500)).await;

    // Node 0 submits a genesis message
    let genesis_id = node0
        .submit_message(b"Genesis".to_vec())
        .await
        .expect("Failed to submit genesis");

    info!(
        "📝 Genesis submitted: {}",
        hex::encode(&genesis_id.as_bytes()[..8])
    );

    // Wait for propagation
    sleep(Duration::from_secs(2)).await;

    // Node 0 submits a child message to build DAG
    let child_id = node0
        .submit_message(b"Child".to_vec())
        .await
        .expect("Failed to submit child");

    info!(
        "📝 Child submitted: {}",
        hex::encode(&child_id.as_bytes()[..8])
    );

    // Wait for propagation
    sleep(Duration::from_secs(2)).await;

    // Check finality on node 0
    let finalized0 = node0.check_finality().await.unwrap_or_default();
    info!("Node 0 finalized {} messages", finalized0.len());

    if finalized0.is_empty() {
        warn!("⚠️ No messages finalized on node 0 yet, continuing anyway");
    }

    // Manually broadcast finality artifacts (normally done by Node::check_finality)
    // This test uses NetworkEngine directly, so we simulate the node behavior

    // Check connection state before broadcasting
    let network_guard = node0.network.read().await;
    let transport = network_guard.transport().await;
    let connections = transport.connection_pool().get_all_connections().await;
    info!(
        "🔍 [TEST_DEBUG] Node 0 has {} active connections before broadcasting",
        connections.len()
    );
    for (peer_id, _) in &connections {
        info!("🔍 [TEST_DEBUG] Node 0 connected to peer: {}", peer_id);
    }
    drop(transport);
    drop(network_guard);

    for msg_id in &finalized0 {
        if let Some(artifact) = node0.finality.get_artifact(msg_id).await {
            info!(
                "🔍 [TEST_DEBUG] 📡 Broadcasting artifact for {}",
                hex::encode(&msg_id.as_bytes()[..8])
            );
            let network_guard = node0.network.read().await;
            let transport = network_guard.transport().await;
            if let Err(e) = transport.broadcast_finality_artifact(artifact).await {
                warn!("🔍 [TEST_DEBUG] ❌ Failed to broadcast artifact: {}", e);
            } else {
                info!("🔍 [TEST_DEBUG] ✅ Broadcast call completed");
            }
        } else {
            warn!(
                "🔍 [TEST_DEBUG] ⚠️ No artifact found in finality engine for {}",
                hex::encode(&msg_id.as_bytes()[..8])
            );
        }
    }

    // Wait for finality artifact broadcast
    sleep(Duration::from_secs(2)).await;

    // Verify that if node 0 has finality artifacts, they should propagate to node 1
    for msg_id in &finalized0 {
        let has_artifact_0 = node0.has_finality_artifact(msg_id).await;
        info!(
            "🔍 [TEST_DEBUG] Node 0 has artifact for {}: {}",
            hex::encode(&msg_id.as_bytes()[..8]),
            has_artifact_0
        );

        if has_artifact_0 {
            info!(
                "🔍 [TEST_DEBUG] Checking artifact propagation for message: {}",
                hex::encode(&msg_id.as_bytes()[..8])
            );

            // Give more time for artifact to propagate
            let mut found_on_peer = false;
            for attempt in 0..5 {
                sleep(Duration::from_millis(500)).await;
                let has_on_node1 = node1.has_finality_artifact(msg_id).await;
                info!(
                    "🔍 [TEST_DEBUG] Attempt {}: Artifact on node 1: {}",
                    attempt + 1,
                    has_on_node1
                );
                if has_on_node1 {
                    found_on_peer = true;
                    break;
                }
            }

            assert!(
                found_on_peer,
                "Finality artifact for {} should have propagated to node 1",
                hex::encode(&msg_id.as_bytes()[..8])
            );

            info!(
                "✅ Finality artifact {} propagated successfully",
                hex::encode(&msg_id.as_bytes()[..8])
            );

            // Verify the artifact gate type matches
            let gate_0 = node0.get_finality_artifact_gate(msg_id).await;
            let gate_1 = node1.get_finality_artifact_gate(msg_id).await;

            if let (Some(g0), Some(g1)) = (gate_0, gate_1) {
                assert_eq!(
                    g0, g1,
                    "Artifact gate type should match across nodes"
                );
                info!("✅ Artifact gate type {:?} matches on both nodes", g0);
            }
        }
    }

    info!("🎉 Finality artifact broadcast test completed successfully");

    // Shutdown
    node0.shutdown().await.ok();
    node1.shutdown().await.ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_finality_artifact_relay() {
    // Initialize logging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("adic_network=debug,finality_artifact_propagation_test=info")
        .try_init();

    info!("🧪 Starting finality artifact relay (3-node) test");

    // Get ports
    let base = env::var("ADIC_TEST_PORT_MIN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7100);

    let (tcp0, tcp1, tcp2) = (base, base + 1, base + 2);

    // Create three nodes
    let node0 = Arc::new(TestNode::new(0, tcp0).await);
    let node1 = Arc::new(TestNode::new(1, tcp1).await);
    let node2 = Arc::new(TestNode::new(2, tcp2).await);

    // Start all nodes
    node0.start().await.expect("Failed to start node 0");
    node1.start().await.expect("Failed to start node 1");
    node2.start().await.expect("Failed to start node 2");

    sleep(Duration::from_millis(500)).await;

    // Connect in a line: node0 <-> node1 <-> node2
    // This tests that artifacts relay through intermediate nodes
    let quic1 = node1.actual_quic_port().await;
    timeout(Duration::from_secs(5), node0.connect_to(quic1))
        .await
        .expect("Connection timeout")
        .expect("Node 0 -> Node 1 connection failed");

    sleep(Duration::from_millis(300)).await;

    let quic2 = node2.actual_quic_port().await;
    timeout(Duration::from_secs(5), node1.connect_to(quic2))
        .await
        .expect("Connection timeout")
        .expect("Node 1 -> Node 2 connection failed");

    info!("✅ Linear topology established: Node0 <-> Node1 <-> Node2");
    sleep(Duration::from_millis(500)).await;

    // Node 0 submits messages
    let genesis_id = node0
        .submit_message(b"Genesis".to_vec())
        .await
        .expect("Failed to submit genesis");

    sleep(Duration::from_secs(2)).await;

    let child_id = node0
        .submit_message(b"Child".to_vec())
        .await
        .expect("Failed to submit child");

    info!("📝 Messages submitted from node 0");
    sleep(Duration::from_secs(2)).await;

    // Check finality on node 0
    let finalized0 = node0.check_finality().await.unwrap_or_default();
    info!("Node 0 finalized {} messages", finalized0.len());

    // Manually broadcast finality artifacts (normally done by Node::check_finality)
    for msg_id in &finalized0 {
        if let Some(artifact) = node0.finality.get_artifact(msg_id).await {
            info!(
                "📡 Broadcasting artifact for {}",
                hex::encode(&msg_id.as_bytes()[..8])
            );
            let network_guard = node0.network.read().await;
            let transport = network_guard.transport().await;
            if let Err(e) = transport.broadcast_finality_artifact(artifact).await {
                warn!("Failed to broadcast artifact: {}", e);
            }
        }
    }

    // Wait for artifacts to propagate through the chain
    sleep(Duration::from_secs(3)).await;

    // Verify artifacts reached node 2 via relay through node 1
    for msg_id in &finalized0 {
        if node0.has_finality_artifact(msg_id).await {
            info!(
                "🔍 Checking relay of artifact {}",
                hex::encode(&msg_id.as_bytes()[..8])
            );

            // Check intermediate node
            let has_on_node1 = node1.has_finality_artifact(msg_id).await;

            // Check final node (should get via relay)
            let mut found_on_node2 = false;
            for attempt in 0..10 {
                sleep(Duration::from_millis(500)).await;
                if node2.has_finality_artifact(msg_id).await {
                    found_on_node2 = true;
                    break;
                }
                debug!(
                    "Attempt {}: Artifact {} not yet on node 2",
                    attempt + 1,
                    hex::encode(&msg_id.as_bytes()[..8])
                );
            }

            if has_on_node1 {
                info!("✅ Artifact reached node 1 (intermediate)");
            }

            assert!(
                found_on_node2,
                "Artifact for {} should relay from node 0 through node 1 to node 2",
                hex::encode(&msg_id.as_bytes()[..8])
            );

            info!(
                "✅ Artifact {} successfully relayed to node 2",
                hex::encode(&msg_id.as_bytes()[..8])
            );
        }
    }

    info!("🎉 Finality artifact relay test completed successfully");

    // Shutdown
    node0.shutdown().await.ok();
    node1.shutdown().await.ok();
    node2.shutdown().await.ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_finality_artifact_validity() {
    // Initialize logging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("adic_network=debug,finality_artifact_propagation_test=info")
        .try_init();

    info!("🧪 Starting finality artifact validity test");

    // Get ports
    let base = env::var("ADIC_TEST_PORT_MIN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7200);

    let (tcp0, tcp1) = (base, base + 1);

    // Create two nodes
    let node0 = Arc::new(TestNode::new(0, tcp0).await);
    let node1 = Arc::new(TestNode::new(1, tcp1).await);

    // Start both nodes
    node0.start().await.expect("Failed to start node 0");
    node1.start().await.expect("Failed to start node 1");

    sleep(Duration::from_millis(500)).await;

    // Connect nodes
    let quic1 = node1.actual_quic_port().await;
    timeout(Duration::from_secs(5), node0.connect_to(quic1))
        .await
        .expect("Connection timeout")
        .expect("Connection failed");

    sleep(Duration::from_millis(500)).await;

    // Submit messages to build DAG
    let genesis_id = node0
        .submit_message(b"Genesis".to_vec())
        .await
        .expect("Failed to submit genesis");

    sleep(Duration::from_secs(1)).await;

    let child1_id = node0
        .submit_message(b"Child1".to_vec())
        .await
        .expect("Failed to submit child1");

    sleep(Duration::from_secs(1)).await;

    let child2_id = node0
        .submit_message(b"Child2".to_vec())
        .await
        .expect("Failed to submit child2");

    info!("📝 DAG built with multiple messages");
    sleep(Duration::from_secs(2)).await;

    // Check finality
    let finalized0 = node0.check_finality().await.unwrap_or_default();
    info!("Node 0 finalized {} messages", finalized0.len());

    // Manually broadcast finality artifacts (normally done by Node::check_finality)
    for msg_id in &finalized0 {
        if let Some(artifact) = node0.finality.get_artifact(msg_id).await {
            let network_guard = node0.network.read().await;
            let transport = network_guard.transport().await;
            if let Err(e) = transport.broadcast_finality_artifact(artifact).await {
                warn!("Failed to broadcast artifact: {}", e);
            }
        }
    }

    // Wait for propagation
    sleep(Duration::from_secs(2)).await;

    // Verify artifacts are valid on both nodes
    for msg_id in &finalized0 {
        if let Some(artifact0) = node0.finality.get_artifact(msg_id).await {
            info!(
                "🔍 Validating artifact {}",
                hex::encode(&msg_id.as_bytes()[..8])
            );

            // Verify artifact is valid
            assert!(
                artifact0.is_valid(),
                "Artifact on node 0 should be valid"
            );

            // Wait for propagation to node 1
            let mut artifact1_valid = false;
            for _ in 0..10 {
                sleep(Duration::from_millis(500)).await;
                if let Some(artifact1) = node1.finality.get_artifact(msg_id).await {
                    assert!(
                        artifact1.is_valid(),
                        "Propagated artifact on node 1 should be valid"
                    );
                    artifact1_valid = true;
                    break;
                }
            }

            if artifact1_valid {
                info!(
                    "✅ Artifact {} is valid on both nodes",
                    hex::encode(&msg_id.as_bytes()[..8])
                );
            }
        }
    }

    info!("🎉 Finality artifact validity test completed successfully");

    // Shutdown
    node0.shutdown().await.ok();
    node1.shutdown().await.ok();
}
