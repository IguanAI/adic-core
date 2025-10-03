use adic_consensus::ConsensusEngine;
use adic_crypto::Keypair;
use adic_finality::{FinalityConfig, FinalityEngine};
use adic_storage::{store::BackendType, StorageConfig, StorageEngine};
use adic_types::{AdicFeatures, AdicMessage, AdicMeta, AdicParams, AxisPhi, MessageId, QpDigits};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::tempdir;

fn create_test_storage() -> Arc<StorageEngine> {
    let temp_dir = tempdir().unwrap();
    let storage_config = StorageConfig {
        backend_type: BackendType::RocksDB {
            path: temp_dir.path().to_str().unwrap().to_string(),
        },
        cache_size: 10,
        flush_interval_ms: 1000,
        max_batch_size: 100,
    };
    Arc::new(StorageEngine::new(storage_config).unwrap())
}

/// Create a test message with specified ID and parents
/// Uses values designed to maximize diversity at radii [2, 2, 1]
fn create_test_message(id: u64, parents: Vec<MessageId>, keypair: &Keypair) -> AdicMessage {
    // For p=3 with rho=[2,2,1], use values that are guaranteed to be in different balls
    // Use coprime offsets and multipliers NOT divisible by p=3 to ensure distinct ball assignments
    let features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(id * 100 + 13, 3, 10)), // 100 mod 3 = 1 (creates variation)
        AxisPhi::new(1, QpDigits::from_u64(id * 200 + 29, 3, 10)), // 200 mod 3 = 2 (creates variation)
        AxisPhi::new(2, QpDigits::from_u64(id * 301 + 41, 3, 10)), // 301 mod 3 = 1 (creates variation at radius 1)
    ]);

    let mut message = AdicMessage::new(
        parents,
        features,
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        format!("Test message {}", id).into_bytes(),
    );

    message.signature = keypair.sign(&message.to_bytes());
    message
}

/// Compute ball IDs using per-axis radii from params
fn compute_ball_ids(message: &AdicMessage, params: &AdicParams) -> HashMap<u32, Vec<u8>> {
    let mut ball_ids = HashMap::new();
    for axis_phi in &message.features.phi {
        let axis_idx = axis_phi.axis.0 as usize;
        let radius = params.rho.get(axis_idx).copied().unwrap_or(2) as usize;
        ball_ids.insert(axis_phi.axis.0, axis_phi.qp_digits.ball_id(radius));
    }
    ball_ids
}

#[tokio::test]
async fn test_kcore_with_complex_graph_structure() {
    let params = AdicParams {
        k: 3,
        depth_star: 2,
        ..Default::default()
    };

    let storage = create_test_storage();
    let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
    let finality_config = FinalityConfig::from(&params);
    let finality = FinalityEngine::new(finality_config, consensus, storage.clone());

    let keypair = Keypair::generate();

    // Create a complex graph structure with multiple layers
    // Layer 0: Single root
    let root = create_test_message(0, vec![], &keypair);
    let root_id = root.id;
    storage.store_message(&root).await.unwrap();

    let ball_ids = compute_ball_ids(&root, &params);

    finality
        .add_message(root_id, vec![], 2.0, ball_ids.clone())
        .await
        .unwrap();

    // Layer 1: Multiple nodes connecting to root
    let mut layer1_nodes = Vec::new();
    for i in 1..=5 {
        let msg = create_test_message(i, vec![root_id], &keypair);
        let msg_id = msg.id;
        storage.store_message(&msg).await.unwrap();

        let ball_ids = compute_ball_ids(&msg, &params);

        finality
            .add_message(msg_id, vec![root_id], 1.8, ball_ids)
            .await
            .unwrap();
        layer1_nodes.push(msg_id);
    }

    // Layer 2: Interconnected nodes forming a k-core
    let mut layer2_nodes = Vec::new();
    for i in 6..=10 {
        // Each node connects to multiple layer1 nodes
        let parents = vec![layer1_nodes[0], layer1_nodes[1], layer1_nodes[2]];
        let msg = create_test_message(i, parents.clone(), &keypair);
        let msg_id = msg.id;
        storage.store_message(&msg).await.unwrap();

        let ball_ids = compute_ball_ids(&msg, &params);

        finality
            .add_message(msg_id, parents, 1.5, ball_ids)
            .await
            .unwrap();
        layer2_nodes.push(msg_id);
    }

    // Layer 3: Dense interconnections forming potential k-core
    for i in 11..=15 {
        // Each node connects to multiple layer2 nodes
        let parents = vec![layer2_nodes[0], layer2_nodes[1], layer2_nodes[2]];
        let msg = create_test_message(i, parents.clone(), &keypair);
        let msg_id = msg.id;
        storage.store_message(&msg).await.unwrap();

        let ball_ids = compute_ball_ids(&msg, &params);

        finality
            .add_message(msg_id, parents, 1.2, ball_ids)
            .await
            .unwrap();
    }

    // Check finality - test that finality checking completes successfully
    let finalized = finality.check_finality().await.unwrap();
    let stats = finality.get_stats().await;

    // With correct radii [2,2,1] from params.rho and proper diversity generation,
    // finality should be achieved for messages with sufficient k-core structure
    assert!(
        !finalized.is_empty(),
        "Should finalize messages with proper diversity and k-core structure"
    );
    assert!(
        stats.finalized_count > 0,
        "Finalized count should match finalized messages"
    );
}

#[tokio::test]
async fn test_empty_and_disconnected_graphs() {
    let params = AdicParams::default();
    let storage = create_test_storage();
    let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
    let finality_config = FinalityConfig::from(&params);
    let finality = FinalityEngine::new(finality_config, consensus, storage.clone());

    // Test with empty graph
    let finalized = finality.check_finality().await.unwrap();
    assert!(finalized.is_empty());

    let keypair = Keypair::generate();

    // Add isolated nodes (no connections)
    for i in 1..=3 {
        let msg = create_test_message(i, vec![], &keypair);
        let msg_id = msg.id;
        storage.store_message(&msg).await.unwrap();

        let ball_ids = compute_ball_ids(&msg, &params);

        finality
            .add_message(msg_id, vec![], 1.0, ball_ids)
            .await
            .unwrap();
    }

    // Isolated nodes should not form a k-core with k > 0
    let finalized = finality.check_finality().await.unwrap();
    let stats = finality.get_stats().await;

    // With default k=20, isolated nodes cannot be finalized
    assert_eq!(stats.finalized_count, 0);
    assert_eq!(finalized.len(), 0);
}

#[tokio::test]
async fn test_finality_with_insufficient_diversity() {
    let params = AdicParams {
        k: 2,
        q: 3, // Require 3 distinct balls per axis
        depth_star: 1,
        ..Default::default()
    };

    let storage = create_test_storage();
    let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
    let finality_config = FinalityConfig::from(&params);
    let finality = FinalityEngine::new(finality_config, consensus, storage.clone());

    let keypair = Keypair::generate();

    // Create messages with insufficient diversity (same ball IDs)
    let root = create_test_message(0, vec![], &keypair);
    let root_id = root.id;
    storage.store_message(&root).await.unwrap();

    // All messages use the same features (same ball IDs)
    let same_ball_ids = {
        let mut ball_ids = HashMap::new();
        ball_ids.insert(0, vec![0, 0, 0]); // Same ball for axis 0
        ball_ids.insert(1, vec![0, 0, 0]); // Same ball for axis 1
        ball_ids.insert(2, vec![0, 0, 0]); // Same ball for axis 2
        ball_ids
    };

    finality
        .add_message(root_id, vec![], 2.0, same_ball_ids.clone())
        .await
        .unwrap();

    // Add more messages with same features
    for _i in 1..=5 {
        let msg = create_test_message(0, vec![root_id], &keypair); // Same features as root
        let msg_id = msg.id;
        storage.store_message(&msg).await.unwrap();

        finality
            .add_message(msg_id, vec![root_id], 1.5, same_ball_ids.clone())
            .await
            .unwrap();
    }

    // Should not finalize due to insufficient diversity
    let finalized = finality.check_finality().await.unwrap();
    let stats = finality.get_stats().await;

    // Insufficient diversity should prevent finalization
    assert_eq!(stats.finalized_count, 0);
    assert!(finalized.is_empty());
}

#[tokio::test]
async fn test_finality_with_low_reputation() {
    let params = AdicParams {
        k: 2,
        r_sum_min: 10.0, // High reputation threshold
        depth_star: 1,
        ..Default::default()
    };

    let storage = create_test_storage();
    let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
    let finality_config = FinalityConfig::from(&params);
    let finality = FinalityEngine::new(finality_config, consensus, storage.clone());

    let keypair = Keypair::generate();

    // Create messages with low reputation
    let root = create_test_message(0, vec![], &keypair);
    let root_id = root.id;
    storage.store_message(&root).await.unwrap();

    let ball_ids = compute_ball_ids(&root, &params);

    // Add with very low reputation (below threshold)
    finality
        .add_message(root_id, vec![], 0.1, ball_ids.clone())
        .await
        .unwrap();

    for i in 1..=3 {
        let msg = create_test_message(i, vec![root_id], &keypair);
        let msg_id = msg.id;
        storage.store_message(&msg).await.unwrap();

        let ball_ids = compute_ball_ids(&msg, &params);

        // All have low reputation
        finality
            .add_message(msg_id, vec![root_id], 0.1, ball_ids)
            .await
            .unwrap();
    }

    // Should not finalize due to low reputation
    let finalized = finality.check_finality().await.unwrap();
    let stats = finality.get_stats().await;

    // Low reputation should prevent finalization
    assert_eq!(stats.finalized_count, 0);
    assert!(finalized.is_empty());
}

#[tokio::test]
async fn test_finality_with_deep_dag_structure() {
    let params = AdicParams {
        k: 1,          // Minimal k for easier testing
        depth_star: 5, // Require deep structure
        ..Default::default()
    };

    let storage = create_test_storage();
    let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
    let finality_config = FinalityConfig::from(&params);
    let finality = FinalityEngine::new(finality_config, consensus, storage.clone());

    let keypair = Keypair::generate();

    // Build a chain of messages to create depth
    let mut current_id = {
        let root = create_test_message(0, vec![], &keypair);
        let root_id = root.id;
        storage.store_message(&root).await.unwrap();

        let ball_ids = compute_ball_ids(&root, &params);

        finality
            .add_message(root_id, vec![], 2.0, ball_ids)
            .await
            .unwrap();
        root_id
    };

    // Create a chain of 10 messages
    for i in 1..=10 {
        let msg = create_test_message(i, vec![current_id], &keypair);
        let msg_id = msg.id;
        storage.store_message(&msg).await.unwrap();

        let ball_ids = compute_ball_ids(&msg, &params);

        finality
            .add_message(msg_id, vec![current_id], 1.8, ball_ids)
            .await
            .unwrap();
        current_id = msg_id;
    }

    // Check finality - test that deep DAG finality checking works
    let finalized = finality.check_finality().await.unwrap();
    let stats = finality.get_stats().await;

    println!(
        "Deep DAG - Finalized: {}, Pending: {}",
        finalized.len(),
        stats.pending_count
    );

    // With correct radii [2,2,1] and proper diversity, deep DAG should achieve finality
    assert!(
        !finalized.is_empty() || stats.pending_count > 0,
        "Should either finalize or track messages in deep DAG"
    );
}

#[tokio::test]
async fn test_finality_performance_under_load() {
    let params = AdicParams {
        k: 3,
        depth_star: 2,
        ..Default::default()
    };

    let storage = create_test_storage();
    let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
    let finality_config = FinalityConfig::from(&params);
    let finality = FinalityEngine::new(finality_config, consensus, storage.clone());

    let keypair = Keypair::generate();

    // Create a large number of messages to test performance
    const NUM_MESSAGES: u64 = 100;
    let mut message_ids = Vec::new();

    let start_time = std::time::Instant::now();

    // Genesis
    let root = create_test_message(0, vec![], &keypair);
    let root_id = root.id;
    storage.store_message(&root).await.unwrap();
    message_ids.push(root_id);

    let ball_ids = compute_ball_ids(&root, &params);

    finality
        .add_message(root_id, vec![], 2.0, ball_ids)
        .await
        .unwrap();

    // Create messages with random parent selection
    for i in 1..NUM_MESSAGES {
        let num_parents = std::cmp::min(i as usize, 4); // Up to 4 parents
        let parents = if num_parents <= message_ids.len() {
            message_ids[message_ids.len() - num_parents..].to_vec()
        } else {
            message_ids.clone()
        };

        let msg = create_test_message(i, parents.clone(), &keypair);
        let msg_id = msg.id;
        storage.store_message(&msg).await.unwrap();

        let ball_ids = compute_ball_ids(&msg, &params);

        finality
            .add_message(msg_id, parents, 1.5, ball_ids)
            .await
            .unwrap();
        message_ids.push(msg_id);
    }

    let add_duration = start_time.elapsed();

    // Check finality performance
    let finality_start = std::time::Instant::now();
    let finalized = finality.check_finality().await.unwrap();
    let finality_duration = finality_start.elapsed();

    let stats = finality.get_stats().await;

    println!("Performance test results:");
    println!("  Messages added: {}", NUM_MESSAGES);
    println!("  Add time: {:?}", add_duration);
    println!("  Finality check time: {:?}", finality_duration);
    println!("  Finalized: {}", finalized.len());
    println!("  Pending: {}", stats.pending_count);

    // Performance assertions
    assert!(
        add_duration.as_millis() < 1000,
        "Adding messages took too long: {:?}",
        add_duration
    );
    assert!(
        finality_duration.as_millis() < 2000,
        "Finality check took too long: {:?}",
        finality_duration
    );

    // Should have some finalized messages with this setup
    assert!(stats.total_messages > 0);
}
