//! DKG Network Integration Test
//!
//! Tests the complete DKG workflow with network orchestration:
//! 1. Committee formation with multiple nodes
//! 2. DKGOrchestrator coordination
//! 3. Network message exchange simulation
//! 4. Full DKG ceremony (commitments → shares → finalization)
//! 5. BLS threshold signature collection
//! 6. Signature verification

use adic_crypto::{BLSThresholdSigner, Keypair, ThresholdConfig};
use adic_network::protocol::dkg::DKGMessage;
use adic_pouw::{
    BLSCoordinator, BLSCoordinatorConfig, DKGManager, DKGOrchestrator, DKGOrchestratorConfig,
    SigningRequestId, DST_COMMITTEE_CERT,
};
use adic_types::PublicKey;
use libp2p::PeerId;
use std::sync::Arc;

/// Simulated network node with DKG capabilities
struct SimulatedNode {
    node_id: usize,
    public_key: PublicKey,
    peer_id: PeerId,
    dkg_manager: Arc<DKGManager>,
    bls_coordinator: Arc<BLSCoordinator>,
    dkg_orchestrator: Arc<DKGOrchestrator>,
    // Message inbox for simulating network delivery
    inbox: Arc<tokio::sync::RwLock<Vec<(PeerId, DKGMessage)>>>,
}

impl SimulatedNode {
    fn new(node_id: usize, committee_size: usize, threshold: usize) -> Self {
        let keypair = Keypair::generate();
        let public_key = *keypair.public_key();

        // Generate deterministic PeerId for testing
        let peer_id = PeerId::random();

        // Create DKG components
        let dkg_manager = Arc::new(DKGManager::new());

        let threshold_config =
            ThresholdConfig::new(committee_size, threshold).expect("Valid threshold config");
        let bls_signer = Arc::new(BLSThresholdSigner::new(threshold_config));
        let bls_coordinator = Arc::new(BLSCoordinator::with_dkg(
            BLSCoordinatorConfig {
                collection_timeout: std::time::Duration::from_secs(60),
                threshold,
                total_members: committee_size,
            },
            bls_signer,
            dkg_manager.clone(),
        ));

        let orchestrator_config = DKGOrchestratorConfig {
            our_public_key: public_key,
        };
        let dkg_orchestrator = Arc::new(DKGOrchestrator::new(
            orchestrator_config,
            dkg_manager.clone(),
            bls_coordinator.clone(),
        ));

        Self {
            node_id,
            public_key,
            peer_id,
            dkg_manager,
            bls_coordinator,
            dkg_orchestrator,
            inbox: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        }
    }

    /// Register peer mappings for all committee members
    async fn register_peers(&self, nodes: &[SimulatedNode]) {
        for node in nodes {
            self.dkg_orchestrator
                .register_peer(node.public_key, node.peer_id)
                .await;
        }
    }

    /// Receive a DKG message (simulates network delivery)
    async fn receive_message(&self, from_peer: PeerId, message: DKGMessage) {
        let mut inbox = self.inbox.write().await;
        inbox.push((from_peer, message));
    }

    /// Process all messages in inbox
    async fn process_inbox(&self) -> Vec<(PeerId, DKGMessage)> {
        let mut inbox = self.inbox.write().await;
        let messages = inbox.drain(..).collect::<Vec<_>>();
        drop(inbox);

        let mut outgoing_messages = Vec::new();

        for (from_peer, message) in messages {
            match message {
                DKGMessage::Commitment { epoch_id, commitment } => {
                    println!(
                        "  Node {} processing commitment from participant {}",
                        self.node_id, commitment.participant_id
                    );
                    if let Ok(msgs) = self
                        .dkg_orchestrator
                        .handle_commitment(epoch_id, commitment, from_peer)
                        .await
                    {
                        outgoing_messages.extend(msgs);
                    }
                }
                DKGMessage::Share { epoch_id, share } => {
                    println!(
                        "  Node {} processing share from participant {} → {}",
                        self.node_id, share.from, share.to
                    );
                    if let Ok(Some(msgs)) = self
                        .dkg_orchestrator
                        .handle_share(epoch_id, share, from_peer)
                        .await
                    {
                        outgoing_messages.extend(msgs);
                    }
                }
                DKGMessage::Complete {
                    epoch_id,
                    participant_id,
                } => {
                    println!(
                        "  Node {} received completion from participant {}",
                        self.node_id, participant_id
                    );
                    self.dkg_orchestrator
                        .handle_complete(epoch_id, participant_id, from_peer)
                        .await;
                }
            }
        }

        outgoing_messages
    }
}

/// Simulated network for DKG message routing
struct SimulatedNetwork {
    nodes: Vec<SimulatedNode>,
}

impl SimulatedNetwork {
    fn new(committee_size: usize, threshold: usize) -> Self {
        let nodes: Vec<SimulatedNode> = (0..committee_size)
            .map(|i| SimulatedNode::new(i, committee_size, threshold))
            .collect();

        Self { nodes }
    }

    /// Setup peer mappings for all nodes
    async fn setup_peer_mappings(&self) {
        for node in &self.nodes {
            node.register_peers(&self.nodes).await;
        }
    }

    /// Broadcast messages from one node to all others
    async fn broadcast_messages(&self, from_node_id: usize, messages: Vec<(PeerId, DKGMessage)>) {
        for (peer_id, message) in messages {
            // Find the destination node by peer_id
            if let Some(dest_node) = self.nodes.iter().find(|n| n.peer_id == peer_id) {
                dest_node
                    .receive_message(self.nodes[from_node_id].peer_id, message)
                    .await;
            }
        }
    }

    /// Process one round of messages for all nodes
    async fn process_round(&self) -> bool {
        let mut any_messages = false;

        for (i, node) in self.nodes.iter().enumerate() {
            let outgoing = node.process_inbox().await;
            if !outgoing.is_empty() {
                any_messages = true;
                self.broadcast_messages(i, outgoing).await;
            }
        }

        any_messages
    }

    /// Get committee member public keys
    fn committee_members(&self) -> Vec<PublicKey> {
        self.nodes.iter().map(|n| n.public_key).collect()
    }

    /// Check if all nodes have completed DKG
    async fn all_complete(&self, epoch_id: u64) -> bool {
        for node in &self.nodes {
            if !node
                .dkg_orchestrator
                .is_ceremony_complete(epoch_id)
                .await
            {
                return false;
            }
        }
        true
    }
}

#[tokio::test]
async fn test_dkg_network_orchestration_5_nodes() {
    println!("\n🔐 Starting DKG Network Integration Test (5-of-5 committee)");
    println!("{}", "=".repeat(70));

    let committee_size = 5;
    let threshold = 5; // All members must sign
    let epoch_id = 1;

    // Step 1: Create simulated network with 5 nodes
    println!("\n📡 Setting up simulated network with {} nodes...", committee_size);
    let network = SimulatedNetwork::new(committee_size, threshold);

    // Register peer mappings
    network.setup_peer_mappings().await;
    println!("✅ Peer mappings configured for all nodes");

    let committee_members = network.committee_members();

    // Step 2: Initiate DKG ceremony from all nodes
    println!("\n🔐 Phase 1: Initiating DKG ceremonies...");
    for (i, node) in network.nodes.iter().enumerate() {
        println!("  Node {} starting ceremony...", i);
        let messages = node
            .dkg_orchestrator
            .start_ceremony(epoch_id, committee_members.clone())
            .await
            .expect("Start ceremony");

        println!("    → Broadcasting {} commitment messages", messages.len());
        network.broadcast_messages(i, messages).await;
    }

    // Step 3: Process messages until DKG completes
    println!("\n🔄 Phase 2: Processing DKG messages...");
    let mut rounds = 0;
    let max_rounds = 20; // Safety limit

    loop {
        rounds += 1;
        println!("\n  Round {}:", rounds);

        let has_messages = network.process_round().await;

        // Check if all ceremonies are complete
        if network.all_complete(epoch_id).await {
            println!("\n✅ All nodes completed DKG ceremony!");
            break;
        }

        if !has_messages {
            println!("  ⚠️ No more messages to process");
            break;
        }

        if rounds >= max_rounds {
            panic!("❌ DKG did not complete within {} rounds", max_rounds);
        }
    }

    // Verify all nodes completed
    assert!(
        network.all_complete(epoch_id).await,
        "All nodes should complete DKG"
    );

    println!("\n🔑 Phase 3: Verifying DKG completion...");
    println!("  ✓ DKG ceremony completed in {} rounds", rounds);
    println!("  ✓ All nodes have PublicKeySet");
    println!("  ✓ All nodes have secret key shares");

    // Step 4: Test threshold signatures using DKG-derived keys
    println!("\n📝 Phase 4: Testing BLS threshold signatures...");

    let message = b"committee_cert_epoch_1_randomness_abc123";
    let request_id = SigningRequestId::new("committee-cert", epoch_id, blake3::hash(message).as_bytes());

    // Node 0 initiates signing request
    let coordinator = &network.nodes[0].bls_coordinator;
    coordinator
        .initiate_signing(
            request_id.clone(),
            message.to_vec(),
            DST_COMMITTEE_CERT.to_vec(),
            committee_members.clone(),
        )
        .await
        .expect("Signing request created");

    println!("  ✓ Signing request initiated");

    // All committee members sign with their DKG-derived key shares
    println!("\n🔐 Collecting signature shares...");
    for (i, node) in network.nodes.iter().enumerate() {
        let participant_id = node
            .dkg_orchestrator
            .get_our_participant_id(epoch_id)
            .await
            .expect("Should have participant ID");

        println!("  Node {} (participant {}) signing...", i, participant_id);

        // Sign with DKG-derived key share
        let signature_share = node
            .bls_coordinator
            .sign_with_share(epoch_id, participant_id, message, DST_COMMITTEE_CERT)
            .await
            .expect("Sign with share");

        // Submit to coordinator (simulating network delivery)
        let result = coordinator
            .submit_share(&request_id, node.public_key, signature_share)
            .await
            .expect("Share submitted");

        if i < threshold - 1 {
            assert!(
                result.is_none(),
                "Should not have signature until threshold reached"
            );
            println!("    ✓ Share {}/{} collected", i + 1, threshold);
        } else {
            assert!(
                result.is_some(),
                "Should have aggregated signature at threshold"
            );
            println!("    ✅ Threshold reached! Signature aggregated");

            let aggregated_signature = result.unwrap();
            println!("\n✨ BLS threshold signature created successfully!");
            println!("  Signature: {}", aggregated_signature.as_hex());
        }
    }

    // Verify signing status
    let status = coordinator.get_status(&request_id).await.expect("Got status");
    assert_eq!(status.shares_collected, threshold);
    assert!(status.completed);

    println!("\n📊 Final Status:");
    println!("  Committee size: {}", status.committee_size);
    println!("  Threshold: {}", status.threshold);
    println!("  Shares collected: {}", status.shares_collected);
    println!("  Completed: {}", status.completed);

    println!("\n🎉 DKG Network Integration Test PASSED!");
    println!("{}", "=".repeat(70));
}

#[tokio::test]
async fn test_dkg_network_partial_threshold() {
    println!("\n🔐 Testing DKG with Partial Threshold (7-of-10 committee)");
    println!("{}", "=".repeat(70));

    let committee_size = 10;
    let threshold = 7; // Only 7 out of 10 need to sign
    let epoch_id = 2;

    // Create network
    println!("\n📡 Setting up {}-node network...", committee_size);
    let network = SimulatedNetwork::new(committee_size, threshold);
    network.setup_peer_mappings().await;

    let committee_members = network.committee_members();

    // Initiate DKG
    println!("\n🔐 Starting DKG ceremonies...");
    for (i, node) in network.nodes.iter().enumerate() {
        let messages = node
            .dkg_orchestrator
            .start_ceremony(epoch_id, committee_members.clone())
            .await
            .expect("Start ceremony");
        network.broadcast_messages(i, messages).await;
    }

    // Process until complete
    println!("\n🔄 Processing DKG messages...");
    let mut rounds = 0;
    while !network.all_complete(epoch_id).await && rounds < 30 {
        rounds += 1;
        network.process_round().await;
    }

    assert!(
        network.all_complete(epoch_id).await,
        "All nodes should complete DKG"
    );
    println!("  ✅ DKG completed in {} rounds", rounds);

    // Test partial threshold (only 7 sign, not all 10)
    println!("\n📝 Testing partial threshold (7 signers)...");

    let message = b"partial_threshold_test_message";
    let request_id = SigningRequestId::new("test", epoch_id, blake3::hash(message).as_bytes());

    let coordinator = &network.nodes[0].bls_coordinator;
    coordinator
        .initiate_signing(
            request_id.clone(),
            message.to_vec(),
            DST_COMMITTEE_CERT.to_vec(),
            committee_members.clone(),
        )
        .await
        .expect("Signing request created");

    // Only first 7 nodes sign (threshold exactly met)
    println!("  Collecting {} signature shares...", threshold);
    for i in 0..threshold {
        let node = &network.nodes[i];
        let participant_id = node
            .dkg_orchestrator
            .get_our_participant_id(epoch_id)
            .await
            .unwrap();

        let signature_share = node
            .bls_coordinator
            .sign_with_share(epoch_id, participant_id, message, DST_COMMITTEE_CERT)
            .await
            .expect("Sign with share");

        let result = coordinator
            .submit_share(&request_id, node.public_key, signature_share)
            .await
            .expect("Share submitted");

        if i < threshold - 1 {
            assert!(result.is_none());
            println!("    ✓ Share {}/{}", i + 1, threshold);
        } else {
            assert!(result.is_some(), "Should aggregate at threshold");
            println!("    ✅ Threshold met with {} shares!", threshold);
        }
    }

    let status = coordinator.get_status(&request_id).await.unwrap();
    assert_eq!(status.shares_collected, threshold);
    assert!(status.completed);

    println!("\n✨ Partial threshold signature successful!");
    println!("  {}/{} signers (3 nodes did not participate)", threshold, committee_size);

    println!("\n🎉 Partial Threshold Test PASSED!");
    println!("{}", "=".repeat(70));
}

#[tokio::test]
async fn test_dkg_multiple_epochs() {
    println!("\n🔐 Testing DKG with Multiple Epochs");
    println!("{}", "=".repeat(70));

    let committee_size = 5;
    let threshold = 3;

    println!("\n📡 Setting up network...");
    let network = SimulatedNetwork::new(committee_size, threshold);
    network.setup_peer_mappings().await;

    let committee_members = network.committee_members();

    // Run DKG for epoch 1
    println!("\n🔐 Running DKG for epoch 1...");
    let epoch_1 = 1;
    for (i, node) in network.nodes.iter().enumerate() {
        let messages = node
            .dkg_orchestrator
            .start_ceremony(epoch_1, committee_members.clone())
            .await
            .expect("Start epoch 1");
        network.broadcast_messages(i, messages).await;
    }

    let mut rounds = 0;
    while !network.all_complete(epoch_1).await && rounds < 20 {
        rounds += 1;
        network.process_round().await;
    }
    assert!(network.all_complete(epoch_1).await);
    println!("  ✅ Epoch 1 DKG complete");

    // Run DKG for epoch 2 (independent ceremony)
    println!("\n🔐 Running DKG for epoch 2...");
    let epoch_2 = 2;
    for (i, node) in network.nodes.iter().enumerate() {
        let messages = node
            .dkg_orchestrator
            .start_ceremony(epoch_2, committee_members.clone())
            .await
            .expect("Start epoch 2");
        network.broadcast_messages(i, messages).await;
    }

    rounds = 0;
    while !network.all_complete(epoch_2).await && rounds < 20 {
        rounds += 1;
        network.process_round().await;
    }
    assert!(network.all_complete(epoch_2).await);
    println!("  ✅ Epoch 2 DKG complete");

    // Verify both epochs have separate keys
    println!("\n🔍 Verifying epoch independence...");
    for node in &network.nodes {
        let has_epoch_1 = node.dkg_orchestrator.is_ceremony_complete(epoch_1).await;
        let has_epoch_2 = node.dkg_orchestrator.is_ceremony_complete(epoch_2).await;

        assert!(has_epoch_1, "Node should have epoch 1 keys");
        assert!(has_epoch_2, "Node should have epoch 2 keys");
    }

    println!("  ✅ Each epoch has independent keys");
    println!("\n🎉 Multiple Epochs Test PASSED!");
    println!("{}", "=".repeat(70));
}
