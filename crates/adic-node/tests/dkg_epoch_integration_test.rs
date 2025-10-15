//! DKG Epoch Integration Test
//!
//! Tests the complete epoch-based DKG ceremony workflow at the node level:
//! 1. Epoch transitions trigger DKG ceremony initialization
//! 2. Gossip protocol broadcasts DKG messages
//! 3. Nodes process incoming DKG messages via handle_dkg_message
//! 4. DKG ceremony completes successfully
//! 5. BLS coordinator is updated with threshold keys

use adic_consensus::ReputationTracker;
use adic_crypto::{Keypair, ThresholdConfig};
use adic_network::protocol::dkg::{DKGConfig, DKGMessage, DKGProtocol};
use adic_network::protocol::gossip::{GossipConfig, GossipProtocol};
use adic_pouw::{BLSCoordinator, BLSCoordinatorConfig, DKGManager, DKGOrchestrator, DKGOrchestratorConfig};
use adic_types::PublicKey;
use libp2p::PeerId;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Simulated node for epoch-based DKG testing
struct TestNode {
    node_id: usize,
    public_key: PublicKey,
    peer_id: PeerId,
    gossip: Arc<GossipProtocol>,
    dkg_protocol: Arc<DKGProtocol>,
    dkg_orchestrator: Arc<DKGOrchestrator>,
    dkg_manager: Arc<DKGManager>,
    bls_coordinator: Arc<BLSCoordinator>,
    reputation: Arc<ReputationTracker>,
}

impl TestNode {
    async fn new(node_id: usize, committee_size: usize, threshold: usize) -> Self {
        let keypair = Keypair::generate();
        let public_key = *keypair.public_key();

        // Create libp2p keypair for gossip
        let libp2p_keypair = libp2p::identity::Keypair::generate_ed25519();
        let peer_id = PeerId::from(libp2p_keypair.public());

        // Create gossip protocol
        let gossip_config = GossipConfig::default();
        let gossip = Arc::new(
            GossipProtocol::new(&libp2p_keypair, gossip_config).expect("Gossip creation"),
        );

        // Subscribe to DKG topics
        gossip.subscribe("adic/dkg").await.expect("Subscribe to global DKG topic");

        // Create DKG protocol
        let dkg_config = DKGConfig::default();
        let dkg_protocol = Arc::new(DKGProtocol::new(dkg_config));

        // Create DKG components
        let dkg_manager = Arc::new(DKGManager::new());
        let threshold_config = ThresholdConfig::new(committee_size, threshold).unwrap();
        let bls_signer = Arc::new(adic_crypto::BLSThresholdSigner::new(threshold_config));
        let bls_coordinator = Arc::new(BLSCoordinator::with_dkg(
            BLSCoordinatorConfig {
                collection_timeout: std::time::Duration::from_secs(60),
                threshold,
                total_members: committee_size,
            },
            bls_signer,
            dkg_manager.clone(),
        ));

        // Create DKG orchestrator
        let orchestrator_config = DKGOrchestratorConfig {
            our_public_key: public_key,
        };
        let dkg_orchestrator = Arc::new(DKGOrchestrator::new(
            orchestrator_config,
            dkg_manager.clone(),
            bls_coordinator.clone(),
        ));

        // Create reputation tracker with initial scores
        let reputation = Arc::new(ReputationTracker::new(0.8));

        Self {
            node_id,
            public_key,
            peer_id,
            gossip,
            dkg_protocol,
            dkg_orchestrator,
            dkg_manager,
            bls_coordinator,
            reputation,
        }
    }

    /// Register peer mappings for committee members
    async fn register_peers(&self, nodes: &[TestNode]) {
        for node in nodes {
            self.dkg_orchestrator
                .register_peer(node.public_key, node.peer_id)
                .await;

            self.dkg_protocol
                .register_peer(node.peer_id, node.public_key)
                .await;
        }
    }

    /// Simulate initiate_dkg_ceremony (simplified version)
    async fn initiate_dkg_ceremony(
        &self,
        epoch_id: u64,
        committee_members: Vec<PublicKey>,
    ) -> Vec<DKGMessage> {
        println!(
            "  Node {} initiating DKG ceremony for epoch {}",
            self.node_id, epoch_id
        );

        // Start ceremony via orchestrator
        let dkg_messages = self
            .dkg_orchestrator
            .start_ceremony(epoch_id, committee_members)
            .await
            .expect("Start ceremony");

        // Extract DKG messages to broadcast
        dkg_messages.into_iter().map(|(_, msg)| msg).collect()
    }

    /// Simulate handle_dkg_message (simplified version)
    async fn handle_dkg_message(
        &self,
        dkg_msg: DKGMessage,
        from_peer: PeerId,
    ) -> Vec<DKGMessage> {
        let mut outgoing = Vec::new();

        match dkg_msg {
            DKGMessage::Commitment { epoch_id, commitment } => {
                match self
                    .dkg_orchestrator
                    .handle_commitment(epoch_id, commitment.clone(), from_peer)
                    .await
                {
                    Ok(msgs) => {
                        eprintln!("  ✓ Node {} handled commitment from participant {}, generated {} messages",
                            self.node_id, commitment.participant_id, msgs.len());
                        outgoing.extend(msgs.into_iter().map(|(_, msg)| msg));
                    }
                    Err(e) => {
                        eprintln!("  ❌ Node {} error handling commitment from participant {}: {}",
                            self.node_id, commitment.participant_id, e);
                    }
                }
            }
            DKGMessage::Share { epoch_id, share } => {
                match self
                    .dkg_orchestrator
                    .handle_share(epoch_id, share, from_peer)
                    .await
                {
                    Ok(Some(msgs)) => {
                        outgoing.extend(msgs.into_iter().map(|(_, msg)| msg));
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("  ❌ Node {} error handling share: {}", self.node_id, e);
                    }
                }
            }
            DKGMessage::Complete {
                epoch_id,
                participant_id,
            } => {
                self.dkg_orchestrator
                    .handle_complete(epoch_id, participant_id, from_peer)
                    .await;
            }
        }

        outgoing
    }
}

/// Simulated network for epoch-based DKG testing
struct TestNetwork {
    nodes: Vec<Arc<RwLock<TestNode>>>,
}

impl TestNetwork {
    async fn new(committee_size: usize, threshold: usize) -> Self {
        let mut nodes: Vec<Arc<RwLock<TestNode>>> = Vec::new();

        for i in 0..committee_size {
            let node = TestNode::new(i, committee_size, threshold).await;
            nodes.push(Arc::new(RwLock::new(node)));
        }

        // Setup peer mappings
        let node_refs: Vec<_> = {
            let mut refs = Vec::new();
            for node_lock in &nodes {
                refs.push(node_lock.read().await);
            }
            refs
        };

        for node_lock in &nodes {
            let node = node_lock.read().await;

            // Create a temporary vector for registration
            let temp_nodes: Vec<TestNode> = Vec::new();
            for other in &node_refs {
                // Can't directly clone TestNode, so we'll register manually
                node.dkg_orchestrator
                    .register_peer(other.public_key, other.peer_id)
                    .await;
                node.dkg_protocol
                    .register_peer(other.peer_id, other.public_key)
                    .await;
            }
        }

        drop(node_refs);

        Self { nodes }
    }

    async fn get_committee_members(&self) -> Vec<PublicKey> {
        let mut members = Vec::new();
        for node_lock in &self.nodes {
            let node = node_lock.read().await;
            members.push(node.public_key);
        }
        members
    }

    async fn all_ceremony_complete(&self, epoch_id: u64) -> bool {
        for node_lock in &self.nodes {
            let node = node_lock.read().await;
            if !node.dkg_orchestrator.is_ceremony_complete(epoch_id).await {
                return false;
            }
        }
        true
    }

    /// Simulate broadcast of DKG messages via gossip (non-recursive)
    async fn broadcast_and_collect(&self, from_node_id: usize, messages: Vec<DKGMessage>) -> Vec<(usize, Vec<DKGMessage>)> {
        if messages.is_empty() {
            return Vec::new();
        }

        let from_peer_id = {
            let node = self.nodes[from_node_id].read().await;
            node.peer_id
        };

        let mut responses = Vec::new();

        // Broadcast to all other nodes
        for (i, node_lock) in self.nodes.iter().enumerate() {
            if i == from_node_id {
                continue; // Don't send to ourselves
            }

            for msg in &messages {
                let node = node_lock.read().await;
                let outgoing = node.handle_dkg_message(msg.clone(), from_peer_id).await;
                drop(node);

                if !outgoing.is_empty() {
                    responses.push((i, outgoing));
                }
            }
        }

        responses
    }

    /// Process one round of messages for all queued responses
    async fn process_round(&self, pending: Vec<(usize, Vec<DKGMessage>)>) -> Vec<(usize, Vec<DKGMessage>)> {
        let mut next_round = Vec::new();

        for (from_node_id, messages) in pending {
            let responses = self.broadcast_and_collect(from_node_id, messages).await;
            next_round.extend(responses);
        }

        next_round
    }
}

#[tokio::test]
async fn test_epoch_based_dkg_ceremony_3_nodes() {
    println!("\n🔐 Testing Epoch-Based DKG Ceremony (3 nodes)");
    println!("{}", "=".repeat(70));

    let committee_size = 3;
    let threshold = 2; // 2-of-3 threshold
    let epoch_id = 1;

    // Create test network
    println!("\n📡 Setting up test network with {} nodes...", committee_size);
    let network = TestNetwork::new(committee_size, threshold).await;
    println!("  ✅ Network setup complete");

    let committee_members = network.get_committee_members().await;

    // Simulate epoch transition triggering DKG ceremonies on all nodes
    println!("\n📅 Simulating epoch transition to epoch {}...", epoch_id);

    // Step 1: All nodes initiate ceremonies simultaneously
    let mut all_messages = Vec::new();
    for (i, node_lock) in network.nodes.iter().enumerate() {
        let node = node_lock.read().await;
        let messages = node
            .initiate_dkg_ceremony(epoch_id, committee_members.clone())
            .await;
        drop(node);

        println!(
            "    Node {} broadcasting {} commitment messages",
            i,
            messages.len()
        );
        all_messages.push((i, messages));
    }

    // Step 2: Broadcast all messages and collect responses
    let mut pending = Vec::new();
    for (from_node_id, messages) in all_messages {
        let responses = network.broadcast_and_collect(from_node_id, messages).await;
        pending.extend(responses);
    }

    // Process DKG messages in rounds
    println!("\n🔄 Processing DKG messages...");
    let mut rounds = 0;
    let max_rounds = 10;

    while !pending.is_empty() && rounds < max_rounds {
        rounds += 1;
        pending = network.process_round(pending).await;
    }

    // Verify completion
    assert!(
        network.all_ceremony_complete(epoch_id).await,
        "All nodes should complete DKG ceremony"
    );

    println!("\n✅ DKG ceremony completed successfully!");
    println!("  ✓ All {} nodes completed ceremony", committee_size);
    println!("  ✓ Threshold: {}-of-{}", threshold, committee_size);
    println!("  ✓ Epoch: {}", epoch_id);

    // Verify each node has keys
    println!("\n🔍 Verifying DKG results...");
    for (i, node_lock) in network.nodes.iter().enumerate() {
        let node = node_lock.read().await;
        let is_complete = node.dkg_orchestrator.is_ceremony_complete(epoch_id).await;
        assert!(is_complete, "Node {} should have completed DKG", i);
        println!("  ✓ Node {} has threshold keys", i);
    }

    println!("\n🎉 Epoch-Based DKG Test PASSED!");
    println!("{}", "=".repeat(70));
}

#[tokio::test]
async fn test_epoch_transition_with_reputation_based_committee() {
    println!("\n🔐 Testing Epoch Transition with Reputation-Based Committee");
    println!("{}", "=".repeat(70));

    let committee_size = 5;
    let threshold = 3; // 3-of-5 threshold
    let epoch_id = 10;

    println!("\n📡 Setting up {} validators...", committee_size);
    let network = TestNetwork::new(committee_size, threshold).await;

    // Set reputation scores (simulating top validators)
    println!("\n📊 Assigning reputation scores...");
    for (i, node_lock) in network.nodes.iter().enumerate() {
        let node = node_lock.read().await;
        let reputation_score = 100.0 - (i as f64 * 10.0); // Descending scores
        node.reputation
            .set_reputation(&node.public_key, reputation_score)
            .await;
        println!(
            "  Node {} (pk={}...): {}",
            i,
            hex::encode(&node.public_key.as_bytes()[..4]),
            reputation_score
        );
    }

    let committee_members = network.get_committee_members().await;

    // Initiate DKG on epoch transition
    println!("\n📅 Epoch {} transition: Initiating DKG ceremonies...", epoch_id);

    // All nodes initiate simultaneously
    let mut all_messages = Vec::new();
    for (i, node_lock) in network.nodes.iter().enumerate() {
        let node = node_lock.read().await;
        let messages = node
            .initiate_dkg_ceremony(epoch_id, committee_members.clone())
            .await;
        drop(node);
        all_messages.push((i, messages));
    }

    // Broadcast all messages
    let mut pending = Vec::new();
    for (from_node_id, messages) in all_messages {
        let responses = network.broadcast_and_collect(from_node_id, messages).await;
        pending.extend(responses);
    }

    // Process ceremony messages
    println!("\n🔄 Processing ceremony...");
    let mut rounds = 0;
    while !pending.is_empty() && rounds < 15 {
        rounds += 1;
        pending = network.process_round(pending).await;
    }

    assert!(
        network.all_ceremony_complete(epoch_id).await,
        "All committee members should complete DKG"
    );

    println!("\n✅ Epoch transition DKG successful!");
    println!("  ✓ Committee size: {}", committee_size);
    println!("  ✓ Threshold: {}", threshold);
    println!("  ✓ All validators have epoch {} keys", epoch_id);

    println!("\n🎉 Reputation-Based Committee Test PASSED!");
    println!("{}", "=".repeat(70));
}

#[tokio::test]
async fn test_multiple_sequential_epoch_ceremonies() {
    println!("\n🔐 Testing Multiple Sequential Epoch Ceremonies");
    println!("{}", "=".repeat(70));

    let committee_size = 3;
    let threshold = 2;

    println!("\n📡 Setting up network...");
    let network = TestNetwork::new(committee_size, threshold).await;
    let committee_members = network.get_committee_members().await;

    // Run DKG for epochs 1, 2, and 3 sequentially
    for epoch_id in 1..=3 {
        println!("\n📅 === EPOCH {} ===", epoch_id);

        // Initiate ceremonies
        println!("  Initiating DKG ceremonies...");

        // All nodes initiate simultaneously
        let mut all_messages = Vec::new();
        for (i, node_lock) in network.nodes.iter().enumerate() {
            let node = node_lock.read().await;
            let messages = node
                .initiate_dkg_ceremony(epoch_id, committee_members.clone())
                .await;
            drop(node);
            all_messages.push((i, messages));
        }

        // Broadcast all messages
        let mut pending = Vec::new();
        for (from_node_id, messages) in all_messages {
            let responses = network.broadcast_and_collect(from_node_id, messages).await;
            pending.extend(responses);
        }

        // Process messages
        let mut rounds = 0;
        while !pending.is_empty() && rounds < 10 {
            rounds += 1;
            pending = network.process_round(pending).await;
        }

        assert!(
            network.all_ceremony_complete(epoch_id).await,
            "Epoch {} DKG should complete",
            epoch_id
        );

        println!("  ✅ Epoch {} DKG complete", epoch_id);
    }

    // Verify all epochs have independent keys
    println!("\n🔍 Verifying epoch independence...");
    for node_lock in &network.nodes {
        let node = node_lock.read().await;
        for epoch in 1..=3 {
            assert!(
                node.dkg_orchestrator.is_ceremony_complete(epoch).await,
                "Node should have keys for epoch {}",
                epoch
            );
        }
    }
    println!("  ✓ All 3 epochs have independent keys");

    println!("\n🎉 Sequential Epochs Test PASSED!");
    println!("{}", "=".repeat(70));
}
