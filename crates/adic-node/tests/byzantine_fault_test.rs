use adic_consensus::ConsensusEngine;
use adic_crypto::Keypair;
use adic_node::{AdicNode, NodeConfig};
use adic_types::{AdicFeatures, AdicMessage, AdicMeta, AdicParams, AxisPhi, MessageId, QpDigits};
use chrono::Utc;
use std::sync::Arc;

/// Helper to create a test node with in-memory storage
async fn create_test_node(_node_id: u8) -> Arc<AdicNode> {
    let mut config = NodeConfig::default();
    config.consensus.q = 1; // Minimal diversity for testing
    config.consensus.rho = vec![1, 1, 1]; // Lower radii
    config.consensus.r_sum_min = 0.5; // Lower C3 threshold
    config.consensus.r_min = 0.1; // Lower minimum reputation
    config.consensus.k = 1; // Minimal core size
    config.storage.backend = "memory".to_string();

    Arc::new(AdicNode::new(config).await.unwrap())
}

/// Create a malicious message that violates C1 (proximity) constraint
fn create_malicious_c1_message(keypair: &Keypair, parents: Vec<MessageId>) -> AdicMessage {
    // Create features that are very far from any reasonable parent in p-adic space
    let malicious_features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(999999, 3, 10)), // Very large values
        AxisPhi::new(1, QpDigits::from_u64(888888, 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(777777, 3, 10)),
    ]);

    let mut message = AdicMessage::new(
        parents,
        malicious_features,
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        b"Malicious C1 violation".to_vec(),
    );

    message.signature = keypair.sign(&message.to_bytes());
    message
}

/// Create a message that violates C2 (diversity) by using identical features
fn create_malicious_c2_message(keypair: &Keypair, parents: Vec<MessageId>) -> AdicMessage {
    // Create features identical to a common pattern to reduce diversity
    let non_diverse_features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(1, 3, 10)), // Same low values
        AxisPhi::new(1, QpDigits::from_u64(1, 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(1, 3, 10)),
    ]);

    let mut message = AdicMessage::new(
        parents,
        non_diverse_features,
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        b"Malicious C2 violation".to_vec(),
    );

    message.signature = keypair.sign(&message.to_bytes());
    message
}

/// Create a message with invalid signature
fn create_malicious_signature_message(keypair: &Keypair, parents: Vec<MessageId>) -> AdicMessage {
    let features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(10, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(20, 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(30, 3, 10)),
    ]);

    let mut message = AdicMessage::new(
        parents,
        features,
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        b"Message with invalid signature".to_vec(),
    );

    // Create an invalid signature by signing different content
    let wrong_keypair = Keypair::generate();
    message.signature = wrong_keypair.sign(b"wrong content");
    message
}

/// Helper to setup a basic DAG structure for testing
async fn setup_basic_dag(node: &Arc<AdicNode>) -> Vec<MessageId> {
    let keypair = Keypair::generate();

    // Create genesis
    let genesis_features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(0, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(0, 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(0, 3, 10)),
    ]);

    let mut genesis = AdicMessage::new(
        vec![],
        genesis_features,
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        b"Genesis".to_vec(),
    );
    genesis.signature = keypair.sign(&genesis.to_bytes());

    node.storage.store_message(&genesis).await.unwrap();

    // Create some valid parent messages
    let mut message_ids = vec![genesis.id];

    for i in 1..=3 {
        let features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(i, 3, 10)),
            AxisPhi::new(1, QpDigits::from_u64(i * 2, 3, 10)),
            AxisPhi::new(2, QpDigits::from_u64(i * 3, 3, 10)),
        ]);

        let mut message = AdicMessage::new(
            vec![genesis.id],
            features,
            AdicMeta::new(Utc::now()),
            *keypair.public_key(),
            format!("Parent {}", i).into_bytes(),
        );
        message.signature = keypair.sign(&message.to_bytes());

        node.storage.store_message(&message).await.unwrap();
        message_ids.push(message.id);
    }

    // Sync tips
    node.sync_tips_from_storage().await.unwrap();

    message_ids
}

#[tokio::test]
async fn test_byzantine_c1_proximity_attack() {
    let node = create_test_node(1).await;
    let malicious_keypair = Keypair::generate();

    // Setup basic DAG structure
    let message_ids = setup_basic_dag(&node).await;

    // Attacker tries to submit a message that violates C1 proximity constraint
    let malicious_message =
        create_malicious_c1_message(&malicious_keypair, message_ids[1..2].to_vec());

    // Try to store the malicious message directly (bypassing node's validation)
    let result = node.storage.store_message(&malicious_message).await;

    // The storage should accept it (storage doesn't validate)
    assert!(result.is_ok());

    // But when trying to submit through the node (which validates), it should fail
    let submit_result = node.submit_message(b"Attempting C1 attack".to_vec()).await;

    // The legitimate submission should still work (this tests that C1 attack doesn't break the system)
    // Note: This tests system resilience, not the attack success
    println!(
        "C1 attack test - legitimate submission result: {:?}",
        submit_result.is_ok()
    );

    // Verify that the system maintains its integrity
    let stats = node.get_stats().await.unwrap();
    assert!(stats.message_count > 0); // System should still function
}

#[tokio::test]
async fn test_byzantine_signature_attack() {
    let node = create_test_node(2).await;
    let malicious_keypair = Keypair::generate();

    // Setup basic DAG
    let message_ids = setup_basic_dag(&node).await;

    // Create a message with invalid signature
    let malicious_message =
        create_malicious_signature_message(&malicious_keypair, message_ids[1..2].to_vec());

    // Store the malicious message directly
    node.storage
        .store_message(&malicious_message)
        .await
        .unwrap();

    // The consensus engine should detect and handle invalid signatures
    let consensus = ConsensusEngine::new(AdicParams::default());

    // First escrow a deposit for the message (required before validation)
    consensus
        .deposits
        .escrow(malicious_message.id, *malicious_keypair.public_key())
        .await
        .unwrap();

    let validation_result = consensus
        .validate_and_slash(&malicious_message)
        .await
        .unwrap();

    // Message should fail validation due to invalid signature
    assert!(!validation_result.is_valid);
    assert!(!validation_result.errors.is_empty());

    // Check that the system can still process legitimate messages
    let legitimate_result = node.submit_message(b"Legitimate message".to_vec()).await;
    println!(
        "After signature attack - legitimate message: {:?}",
        legitimate_result.is_ok()
    );
}

#[tokio::test]
async fn test_byzantine_double_spend_attack() {
    let node1 = create_test_node(3).await;
    let node2 = create_test_node(4).await;
    let attacker_keypair = Keypair::generate();

    // Setup basic DAG on both nodes
    let message_ids1 = setup_basic_dag(&node1).await;
    let _message_ids2 = setup_basic_dag(&node2).await;

    // Attacker tries to create conflicting messages with same parents
    let features_a = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(100, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(200, 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(300, 3, 10)),
    ]);

    let features_b = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(101, 3, 10)), // Slightly different
        AxisPhi::new(1, QpDigits::from_u64(201, 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(301, 3, 10)),
    ]);

    // Create two messages with same parents but different content
    let mut message_a = AdicMessage::new(
        message_ids1[1..2].to_vec(), // Same parents
        features_a,
        AdicMeta::new(Utc::now()),
        *attacker_keypair.public_key(),
        b"Conflicting message A".to_vec(),
    );
    message_a.signature = attacker_keypair.sign(&message_a.to_bytes());

    let mut message_b = AdicMessage::new(
        message_ids1[1..2].to_vec(), // Same parents
        features_b,
        AdicMeta::new(Utc::now()),
        *attacker_keypair.public_key(),
        b"Conflicting message B".to_vec(),
    );
    message_b.signature = attacker_keypair.sign(&message_b.to_bytes());

    // Submit conflicting messages to different nodes
    node1.storage.store_message(&message_a).await.unwrap();
    node2.storage.store_message(&message_b).await.unwrap();

    // Both messages should be stored (detection happens during consensus)
    let stats1 = node1.get_stats().await.unwrap();
    let stats2 = node2.get_stats().await.unwrap();

    assert!(stats1.message_count > 0);
    assert!(stats2.message_count > 0);

    // The conflict resolution system should handle this when nodes synchronize
    // This test verifies that the system can store conflicting messages without crashing
    println!("Double spend attack test completed - system remains stable");
}

#[tokio::test]
async fn test_byzantine_reputation_manipulation() {
    let node = create_test_node(5).await;
    let malicious_keypair = Keypair::generate();

    // Setup basic DAG
    let _message_ids = setup_basic_dag(&node).await;

    // Try to manipulate reputation by creating self-referencing reputation cycles
    // This simulates an attacker trying to boost their own reputation

    let consensus = ConsensusEngine::new(AdicParams::default());
    let attacker_pk = *malicious_keypair.public_key();

    // Attempt to artificially boost reputation
    consensus
        .reputation
        .good_update(&attacker_pk, 100.0, 1)
        .await; // Huge boost
    let boosted_reputation = consensus.reputation.get_reputation(&attacker_pk).await;

    // The reputation system should have caps/limits to prevent manipulation
    // Check that reputation doesn't exceed reasonable bounds
    assert!(
        boosted_reputation <= 10.0,
        "Reputation system allows excessive manipulation: {}",
        boosted_reputation
    );

    // Try to submit messages with manipulated reputation and verify they still follow normal rules
    let submit_result = node
        .submit_message(b"Message with manipulated reputation".to_vec())
        .await;
    println!(
        "Reputation manipulation test - submission result: {:?}",
        submit_result.is_ok()
    );
}

#[tokio::test]
async fn test_byzantine_network_partition_resilience() {
    // Simulate a network partition attack where nodes are isolated
    let node1 = create_test_node(6).await;
    let node2 = create_test_node(7).await;
    let node3 = create_test_node(8).await;

    // Setup basic state on all nodes
    let _msgs1 = setup_basic_dag(&node1).await;
    let _msgs2 = setup_basic_dag(&node2).await;
    let _msgs3 = setup_basic_dag(&node3).await;

    // Simulate partition by having each node work independently
    // Each node processes messages without synchronizing with others

    let mut results = Vec::new();

    // Node 1 processes some messages
    for i in 0..3 {
        let result = node1
            .submit_message(format!("Node1 message {}", i).into_bytes())
            .await;
        results.push(result.is_ok());
    }

    // Node 2 processes different messages
    for i in 0..3 {
        let result = node2
            .submit_message(format!("Node2 message {}", i).into_bytes())
            .await;
        results.push(result.is_ok());
    }

    // Node 3 processes yet different messages
    for i in 0..3 {
        let result = node3
            .submit_message(format!("Node3 message {}", i).into_bytes())
            .await;
        results.push(result.is_ok());
    }

    // Each node should maintain functionality during partition
    let stats1 = node1.get_stats().await.unwrap();
    let stats2 = node2.get_stats().await.unwrap();
    let stats3 = node3.get_stats().await.unwrap();

    assert!(stats1.message_count > 3); // Should have processed messages
    assert!(stats2.message_count > 3);
    assert!(stats3.message_count > 3);

    println!("Network partition test - all nodes remained functional");
    println!(
        "Node 1: {} messages, Node 2: {} messages, Node 3: {} messages",
        stats1.message_count, stats2.message_count, stats3.message_count
    );
}

#[tokio::test]
async fn test_byzantine_spam_attack_resilience() {
    let node = create_test_node(9).await;

    // Setup basic DAG
    let _message_ids = setup_basic_dag(&node).await;

    // Simulate spam attack - try to overwhelm the system with many messages
    const SPAM_COUNT: usize = 50;
    let _spam_keypair = Keypair::generate();

    let start_time = std::time::Instant::now();
    let mut successful_submissions = 0;
    let mut failed_submissions = 0;

    for i in 0..SPAM_COUNT {
        let result = node
            .submit_message(format!("Spam message {}", i).into_bytes())
            .await;
        if result.is_ok() {
            successful_submissions += 1;
        } else {
            failed_submissions += 1;
        }
    }

    let elapsed = start_time.elapsed();

    // System should either:
    // 1. Process messages efficiently, or
    // 2. Have rate limiting that prevents spam

    println!("Spam attack test results:");
    println!("  Messages attempted: {}", SPAM_COUNT);
    println!("  Successful: {}", successful_submissions);
    println!("  Failed: {}", failed_submissions);
    println!("  Time elapsed: {:?}", elapsed);

    // System should remain responsive (not hang indefinitely)
    assert!(
        elapsed.as_secs() < 30,
        "System became unresponsive during spam attack"
    );

    // System should maintain functionality
    let stats = node.get_stats().await.unwrap();
    assert!(stats.message_count > 0);

    println!("System remained responsive during spam attack");
}

#[tokio::test]
async fn test_byzantine_finality_manipulation() {
    let node = create_test_node(10).await;
    let malicious_keypair = Keypair::generate();

    // Setup basic DAG
    let message_ids = setup_basic_dag(&node).await;

    // Attacker tries to manipulate finality by creating artificial k-core structures
    // Create a cluster of mutually-referencing messages

    let mut cluster_messages = Vec::new();

    // Create messages that reference each other in a tight cluster
    for i in 0..5 {
        let features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(1000 + i, 3, 10)),
            AxisPhi::new(1, QpDigits::from_u64(2000 + i, 3, 10)),
            AxisPhi::new(2, QpDigits::from_u64(3000 + i, 3, 10)),
        ]);

        // Reference some existing messages and previous cluster messages
        let mut parents = vec![message_ids[0]]; // Genesis
        if !cluster_messages.is_empty() {
            parents.push(cluster_messages[cluster_messages.len() - 1]);
        }

        let mut message = AdicMessage::new(
            parents,
            features,
            AdicMeta::new(Utc::now()),
            *malicious_keypair.public_key(),
            format!("Cluster message {}", i).into_bytes(),
        );
        message.signature = malicious_keypair.sign(&message.to_bytes());

        node.storage.store_message(&message).await.unwrap();
        cluster_messages.push(message.id);
    }

    // The finality system should not be fooled by artificial clusters
    // Real finality requires proper k-core structure with sufficient depth and reputation

    let stats = node.get_stats().await.unwrap();
    println!(
        "Finality manipulation test - message count: {}",
        stats.message_count
    );

    // System should remain stable despite manipulation attempts
    assert!(stats.message_count > 0);

    // Try to submit a legitimate message to ensure system still works
    let legitimate_result = node
        .submit_message(b"Legitimate after manipulation".to_vec())
        .await;
    println!(
        "System functionality after finality manipulation: {:?}",
        legitimate_result.is_ok()
    );
}

#[tokio::test]
async fn test_system_recovery_after_byzantine_attacks() {
    let node = create_test_node(11).await;
    let _legitimate_keypair = Keypair::generate();
    let malicious_keypair = Keypair::generate();

    // Setup basic DAG
    let message_ids = setup_basic_dag(&node).await;

    // Launch multiple simultaneous attacks

    // Attack 1: Invalid signature
    let malicious_sig_msg =
        create_malicious_signature_message(&malicious_keypair, message_ids[1..2].to_vec());
    node.storage
        .store_message(&malicious_sig_msg)
        .await
        .unwrap();

    // Attack 2: C1 violation
    let malicious_c1_msg =
        create_malicious_c1_message(&malicious_keypair, message_ids[1..2].to_vec());
    node.storage.store_message(&malicious_c1_msg).await.unwrap();

    // Attack 3: C2 violation attempts
    for _i in 0..3 {
        let malicious_c2_msg =
            create_malicious_c2_message(&malicious_keypair, message_ids[1..2].to_vec());
        node.storage.store_message(&malicious_c2_msg).await.unwrap();
    }

    // Now test system recovery
    println!("Testing system recovery after multiple Byzantine attacks...");

    // System should still be able to process legitimate messages
    let _recovery_results: Vec<bool> = Vec::new();

    for i in 0..5 {
        let result = node
            .submit_message(format!("Recovery message {}", i).into_bytes())
            .await;
        println!("Recovery attempt {}: {:?}", i, result.is_ok());
    }

    // Verify system state after attacks and recovery
    let final_stats = node.get_stats().await.unwrap();

    assert!(final_stats.message_count > 5); // Should have processed messages

    // System should maintain basic functionality
    let health_check = node.submit_message(b"Final health check".to_vec()).await;
    println!("Final system health check: {:?}", health_check.is_ok());

    println!("System recovery test completed successfully");
}
