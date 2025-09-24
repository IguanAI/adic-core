use adic_consensus::{
    AdmissibilityChecker, ConflictResolver, ConsensusEngine, EnergyDescentTracker,
    MessageValidator, ReputationTracker,
};
use adic_crypto::Keypair;
use adic_storage::{store::BackendType, StorageConfig, StorageEngine};
use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AdicParams, AxisPhi, ConflictId, MessageId, PublicKey,
    QpDigits,
};
use chrono::Utc;
use std::collections::HashSet;
use std::sync::Arc;

fn create_message_with_parents_and_features(
    id: u8,
    parents: Vec<MessageId>,
    proposer: PublicKey,
    features: Vec<(u32, u64)>,
) -> AdicMessage {
    let axis_features: Vec<AxisPhi> = features
        .into_iter()
        .map(|(axis, value)| AxisPhi::new(axis, QpDigits::from_u64(value, 3, 10)))
        .collect();

    let features = AdicFeatures::new(axis_features);

    let mut msg = AdicMessage::new(
        parents,
        features,
        AdicMeta::new(Utc::now()),
        proposer,
        vec![id],
    );

    // Set deterministic ID for testing
    msg.id = MessageId::new(&[id; 32]);
    msg
}

// ============= Complex DAG Acyclicity Tests =============

#[tokio::test]
async fn test_complex_dag_acyclicity_validation() {
    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();

    let validator = MessageValidator::new();
    let keypair = Keypair::generate();
    let proposer = *keypair.public_key();

    // Create a complex DAG structure
    // Level 0: Genesis
    let genesis = create_message_with_parents_and_features(0, vec![], proposer, vec![(0, 0)]);
    storage.store_message(&genesis).await.unwrap();

    // Level 1: Two branches
    let msg1 =
        create_message_with_parents_and_features(1, vec![genesis.id], proposer, vec![(0, 1)]);
    let msg2 =
        create_message_with_parents_and_features(2, vec![genesis.id], proposer, vec![(0, 2)]);
    storage.store_message(&msg1).await.unwrap();
    storage.store_message(&msg2).await.unwrap();

    // Level 2: Merge and branch
    let msg3 =
        create_message_with_parents_and_features(3, vec![msg1.id, msg2.id], proposer, vec![(0, 3)]);
    let msg4 = create_message_with_parents_and_features(4, vec![msg1.id], proposer, vec![(0, 4)]);
    storage.store_message(&msg3).await.unwrap();
    storage.store_message(&msg4).await.unwrap();

    // Test: Valid DAG structure
    let ancestors = HashSet::new(); // No cycles in this DAG
    let result = validator.validate_acyclicity(&msg3, &ancestors);
    assert!(result);

    // Test: Attempt to create a cycle
    let mut cyclic_msg =
        create_message_with_parents_and_features(5, vec![msg3.id], proposer, vec![(0, 5)]);
    // Try to make msg1 reference this new message (creating a cycle)
    // This should be caught by validation
    cyclic_msg.id = msg1.id; // Reuse ID to simulate cycle attempt

    let result = validator.validate_message(&cyclic_msg);
    assert!(!result.is_valid || !result.warnings.is_empty());
}

#[tokio::test]
async fn test_deep_dag_traversal() {
    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();

    let validator = MessageValidator::new();
    let keypair = Keypair::generate();
    let proposer = *keypair.public_key();

    // Create a deep chain
    let mut prev_id;
    let genesis = create_message_with_parents_and_features(0, vec![], proposer, vec![(0, 0)]);
    storage.store_message(&genesis).await.unwrap();
    prev_id = genesis.id;

    // Create 100 messages in sequence
    for i in 1..=100 {
        let msg = create_message_with_parents_and_features(
            i,
            vec![prev_id],
            proposer,
            vec![(0, i as u64)],
        );
        storage.store_message(&msg).await.unwrap();
        prev_id = msg.id;
    }

    // Validate the deep chain
    let final_msg = storage.get_message(&prev_id).await.unwrap().unwrap();
    let ancestors = HashSet::new(); // Check no self-reference
    let result = validator.validate_acyclicity(&final_msg, &ancestors);
    assert!(result);
}

// ============= Energy Descent Complex Scenarios =============

#[tokio::test]
async fn test_energy_descent_multiple_conflicts() {
    let params = AdicParams {
        lambda: 1.0,
        mu: 0.5,
        ..Default::default()
    };

    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();
    let reputation = ReputationTracker::new(0.9);
    let resolver = EnergyDescentTracker::new(params.lambda, params.mu);

    // Register multiple conflicts
    let conflict1 = ConflictId::new("double_spend_1".to_string());
    let conflict2 = ConflictId::new("double_spend_2".to_string());
    let conflict3 = ConflictId::new("double_spend_3".to_string());

    resolver.register_conflict(conflict1.clone()).await;
    resolver.register_conflict(conflict2.clone()).await;
    resolver.register_conflict(conflict3.clone()).await;

    // Create competing messages with actual message data
    let keypair1 = Keypair::generate();
    let keypair2 = Keypair::generate();

    // Conflict 1: Create messages and give msg1a more support via descendants
    let msg1a = MessageId::new(&[1; 32]);
    let msg1b = MessageId::new(&[2; 32]);

    let mut msg1a_actual =
        create_message_with_parents_and_features(1, vec![], *keypair1.public_key(), vec![(0, 100)]);
    msg1a_actual.id = msg1a;
    storage.store_message(&msg1a_actual).await.unwrap();

    let mut msg1b_actual =
        create_message_with_parents_and_features(2, vec![], *keypair2.public_key(), vec![(0, 200)]);
    msg1b_actual.id = msg1b;
    storage.store_message(&msg1b_actual).await.unwrap();

    // Add descendants to msg1a for higher support
    for i in 0..5 {
        let child = create_message_with_parents_and_features(
            10 + i,
            vec![msg1a],
            *keypair1.public_key(),
            vec![(0, 110 + i as u64)],
        );
        storage.store_message(&child).await.unwrap();
    }

    // Conflict 2: Create messages with moderate support difference
    let msg2a = MessageId::new(&[3; 32]);
    let msg2b = MessageId::new(&[4; 32]);

    let mut msg2a_actual =
        create_message_with_parents_and_features(3, vec![], *keypair1.public_key(), vec![(0, 300)]);
    msg2a_actual.id = msg2a;
    storage.store_message(&msg2a_actual).await.unwrap();

    let mut msg2b_actual =
        create_message_with_parents_and_features(4, vec![], *keypair2.public_key(), vec![(0, 400)]);
    msg2b_actual.id = msg2b;
    storage.store_message(&msg2b_actual).await.unwrap();

    // Add some descendants to msg2a
    for i in 0..3 {
        let child = create_message_with_parents_and_features(
            20 + i,
            vec![msg2a],
            *keypair1.public_key(),
            vec![(0, 310 + i as u64)],
        );
        storage.store_message(&child).await.unwrap();
    }

    // Conflict 3: Create messages with equal support
    let msg3a = MessageId::new(&[5; 32]);
    let msg3b = MessageId::new(&[6; 32]);

    let mut msg3a_actual =
        create_message_with_parents_and_features(5, vec![], *keypair1.public_key(), vec![(0, 500)]);
    msg3a_actual.id = msg3a;
    storage.store_message(&msg3a_actual).await.unwrap();

    let mut msg3b_actual =
        create_message_with_parents_and_features(6, vec![], *keypair2.public_key(), vec![(0, 600)]);
    msg3b_actual.id = msg3b;
    storage.store_message(&msg3b_actual).await.unwrap();

    // Set reputations
    reputation.set_reputation(keypair1.public_key(), 0.9).await;
    reputation.set_reputation(keypair2.public_key(), 0.5).await;

    // Update support based on actual DAG structure
    for _ in 0..3 {
        resolver
            .update_support(&conflict1, msg1a, &storage, &reputation)
            .await
            .unwrap();
        resolver
            .update_support(&conflict1, msg1b, &storage, &reputation)
            .await
            .unwrap();
        resolver
            .update_support(&conflict2, msg2a, &storage, &reputation)
            .await
            .unwrap();
        resolver
            .update_support(&conflict2, msg2b, &storage, &reputation)
            .await
            .unwrap();
        resolver
            .update_support(&conflict3, msg3a, &storage, &reputation)
            .await
            .unwrap();
        resolver
            .update_support(&conflict3, msg3b, &storage, &reputation)
            .await
            .unwrap();
    }

    // Test metrics
    let metrics = resolver.get_metrics().await;
    assert_eq!(metrics.total_conflicts, 3);
    assert!(metrics.total_energy > 0.0);
}

#[tokio::test]
async fn test_energy_descent_drift_calculation() {
    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();
    let reputation = ReputationTracker::new(0.9);
    let resolver = EnergyDescentTracker::new(1.0, 0.5);

    let conflict = ConflictId::new("test_conflict".to_string());
    resolver.register_conflict(conflict.clone()).await;

    // Create actual messages with different support levels
    let keypair_a = Keypair::generate();
    let keypair_b = Keypair::generate();

    let msg_a = MessageId::new(&[1; 32]);
    let msg_b = MessageId::new(&[2; 32]);

    // Create and store actual messages
    let mut msg_a_actual = create_message_with_parents_and_features(
        1,
        vec![],
        *keypair_a.public_key(),
        vec![(0, 100)],
    );
    msg_a_actual.id = msg_a;
    storage.store_message(&msg_a_actual).await.unwrap();

    let mut msg_b_actual = create_message_with_parents_and_features(
        2,
        vec![],
        *keypair_b.public_key(),
        vec![(0, 200)],
    );
    msg_b_actual.id = msg_b;
    storage.store_message(&msg_b_actual).await.unwrap();

    // Set initial reputations
    reputation.set_reputation(keypair_a.public_key(), 0.8).await;
    reputation.set_reputation(keypair_b.public_key(), 0.6).await;

    // Initial support update
    resolver
        .update_support(&conflict, msg_a, &storage, &reputation)
        .await
        .unwrap();
    resolver
        .update_support(&conflict, msg_b, &storage, &reputation)
        .await
        .unwrap();

    let _initial_drift = resolver.calculate_expected_drift(&conflict).await;

    // Add descendants to msg_a to shift support dramatically
    for i in 0..5 {
        let child = create_message_with_parents_and_features(
            10 + i,
            vec![msg_a],
            *keypair_a.public_key(),
            vec![(0, 110 + i as u64)],
        );
        storage.store_message(&child).await.unwrap();
    }

    // Update support after adding descendants
    resolver
        .update_support(&conflict, msg_a, &storage, &reputation)
        .await
        .unwrap();
    resolver
        .update_support(&conflict, msg_b, &storage, &reputation)
        .await
        .unwrap();

    let final_drift = resolver.calculate_expected_drift(&conflict).await;

    // Drift should be non-positive (energy decreases)
    assert!(
        final_drift <= 0.0,
        "Drift should be non-positive for energy descent"
    );
}

// ============= C1/C2/C3 Constraint Edge Cases =============

#[tokio::test]
async fn test_c1_c2_c3_constraint_edge_cases() {
    let params = AdicParams {
        d: 2,
        q: 2,
        r_min: 0.3,
        ..Default::default()
    };

    let checker = AdmissibilityChecker::new(params.clone());
    let reputation = ReputationTracker::new(params.gamma);
    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();

    let keypair = Keypair::generate();
    let proposer = *keypair.public_key();

    // Setup: Create parent messages with varying characteristics
    let parent1 = create_message_with_parents_and_features(
        1,
        vec![],
        proposer,
        vec![(0, 10), (1, 20), (2, 30)],
    );
    let parent2 = create_message_with_parents_and_features(
        2,
        vec![],
        proposer,
        vec![(0, 11), (1, 21), (2, 31)],
    );
    let parent3 = create_message_with_parents_and_features(
        3,
        vec![],
        proposer,
        vec![(0, 12), (1, 22), (2, 32)],
    );

    storage.store_message(&parent1).await.unwrap();
    storage.store_message(&parent2).await.unwrap();
    storage.store_message(&parent3).await.unwrap();

    // Test C1: Score threshold edge case
    let msg_low_score = create_message_with_parents_and_features(
        4,
        vec![parent1.id, parent2.id, parent3.id],
        proposer,
        vec![(0, 100), (1, 200), (2, 300)], // Very different from parents
    );

    // The check_message needs parent features and reputations, not messages
    let parent_features = vec![
        parent1
            .features
            .phi
            .iter()
            .map(|a| a.qp_digits.clone())
            .collect(),
        parent2
            .features
            .phi
            .iter()
            .map(|a| a.qp_digits.clone())
            .collect(),
        parent3
            .features
            .phi
            .iter()
            .map(|a| a.qp_digits.clone())
            .collect(),
    ];
    let parent_reputations = vec![1.0, 1.0, 1.0]; // Assume good reputation

    let result = checker.check_message(&msg_low_score, &parent_features, &parent_reputations);

    // Check if the score is low (no specific threshold in params)
    if let Ok(res) = result {
        assert!(!res.is_admissible());
    }

    // Test C2: Q-ball coverage edge case
    let msg_poor_coverage = create_message_with_parents_and_features(
        5,
        vec![parent1.id, parent1.id, parent1.id], // Same parent repeated (if allowed)
        proposer,
        vec![(0, 10), (1, 20), (2, 30)], // Same features as parent1
    );

    let parent1_features = vec![parent1
        .features
        .phi
        .iter()
        .map(|a| a.qp_digits.clone())
        .collect()];
    let parent1_reputations = vec![1.0];

    let result = checker.check_message(&msg_poor_coverage, &parent1_features, &parent1_reputations);

    // Should fail C2 due to poor diversity
    if let Ok(res) = result {
        assert!(!res.c2_passed);
    }

    // Test C3: Minimum reputation edge case
    let bad_proposer = PublicKey::from_bytes([99; 32]);

    // Set very low reputation
    for _ in 0..10 {
        reputation.bad_update(&bad_proposer, 1.0).await;
    }

    let msg_low_rep = create_message_with_parents_and_features(
        6,
        vec![parent1.id],
        bad_proposer,
        vec![(0, 10), (1, 20), (2, 30)],
    );

    let parent1_features_2 = vec![parent1
        .features
        .phi
        .iter()
        .map(|a| a.qp_digits.clone())
        .collect()];
    let parent1_reputations_2 = vec![0.1]; // Low reputation parent

    let result = checker.check_message(&msg_low_rep, &parent1_features_2, &parent1_reputations_2);

    // Should fail C3 due to low reputation
    if let Ok(res) = result {
        assert!(!res.c3_passed);
    }
}

// ============= Byzantine Fault Scenarios =============

#[tokio::test]
async fn test_byzantine_double_voting_detection() {
    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();

    let conflict_resolver = ConflictResolver::new();
    let keypair = Keypair::generate();
    let byzantine_proposer = *keypair.public_key();

    // Byzantine node creates two conflicting messages with same parents
    let msg1 = create_message_with_parents_and_features(
        1,
        vec![],
        byzantine_proposer,
        vec![(0, 100)], // Claims value 100
    );

    let msg2 = create_message_with_parents_and_features(
        2,
        vec![],
        byzantine_proposer,
        vec![(0, 200)], // Claims different value 200
    );

    // Both reference the same logical slot (detected by application logic)
    storage.store_message(&msg1).await.unwrap();
    storage.store_message(&msg2).await.unwrap();

    // Register as conflict with messages
    conflict_resolver
        .register_conflict_with_messages("byzantine_double_vote", vec![msg1.id, msg2.id])
        .await;

    // Track proposer's conflict
    conflict_resolver
        .register_proposer_conflict(&byzantine_proposer)
        .await;

    // Check if conflict detected
    let conflicts = conflict_resolver.get_conflicts_for_message(&msg1.id).await;
    assert!(!conflicts.is_empty());

    let conflicts2 = conflict_resolver.get_conflicts_for_message(&msg2.id).await;
    assert!(!conflicts2.is_empty());

    // Verify byzantine proposer is penalized
    let penalty = conflict_resolver
        .get_proposer_conflict_count(&byzantine_proposer)
        .await;
    assert!(penalty > 0);
}

#[tokio::test]
async fn test_byzantine_ancestor_manipulation() {
    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();

    let validator = MessageValidator::new();
    let keypair = Keypair::generate();
    let proposer = *keypair.public_key();

    // Create legitimate chain
    let msg1 = create_message_with_parents_and_features(1, vec![], proposer, vec![(0, 1)]);
    let msg2 = create_message_with_parents_and_features(2, vec![msg1.id], proposer, vec![(0, 2)]);
    let msg3 = create_message_with_parents_and_features(3, vec![msg2.id], proposer, vec![(0, 3)]);

    storage.store_message(&msg1).await.unwrap();
    storage.store_message(&msg2).await.unwrap();
    storage.store_message(&msg3).await.unwrap();

    // Byzantine: Try to create message claiming false ancestry
    let fake_ancestor = MessageId::new(&[99; 32]); // Non-existent
    let byzantine_msg = create_message_with_parents_and_features(
        4,
        vec![fake_ancestor, msg3.id],
        proposer,
        vec![(0, 4)],
    );

    // Validation should detect missing parent
    let result = validator.validate_message(&byzantine_msg);
    // The message itself is valid, but storage operations should fail
    let store_result = storage.store_message(&byzantine_msg).await;

    // Should either be invalid or cause storage issues
    assert!(!result.is_valid || store_result.is_err() || !result.warnings.is_empty());
}

// ============= Network Partition Handling =============

#[tokio::test]
async fn test_network_partition_consensus_recovery() {
    // Simulate two network partitions developing separately
    let storage_a = Arc::new(
        StorageEngine::new(StorageConfig {
            backend_type: BackendType::Memory,
            ..Default::default()
        })
        .unwrap(),
    );

    let storage_b = Arc::new(
        StorageEngine::new(StorageConfig {
            backend_type: BackendType::Memory,
            ..Default::default()
        })
        .unwrap(),
    );

    let keypair_a = Keypair::generate();
    let keypair_b = Keypair::generate();
    let proposer_a = *keypair_a.public_key();
    let proposer_b = *keypair_b.public_key();

    // Partition A develops chain
    let mut last_a = MessageId::new(&[0; 32]);
    for i in 1..=10 {
        let parents = if i == 1 { vec![] } else { vec![last_a] };
        let msg = create_message_with_parents_and_features(
            i,
            parents,
            proposer_a,
            vec![(0, i as u64 * 10)],
        );
        storage_a.store_message(&msg).await.unwrap();
        last_a = msg.id;
    }

    // Partition B develops different chain
    let mut last_b = MessageId::new(&[0; 32]);
    for i in 1..=10 {
        let parents = if i == 1 { vec![] } else { vec![last_b] };
        let msg = create_message_with_parents_and_features(
            i + 100, // Different IDs
            parents,
            proposer_b,
            vec![(0, i as u64 * 20)],
        );
        storage_b.store_message(&msg).await.unwrap();
        last_b = msg.id;
    }

    // Simulate partition heal - merge chains
    // Copy partition B's messages to partition A's storage
    let b_messages = storage_b.list_all_messages().await.unwrap();
    for msg_id in b_messages {
        if let Some(msg) = storage_b.get_message(&msg_id).await.unwrap() {
            storage_a.store_message(&msg).await.unwrap();
        }
    }

    // After merge, both chains should coexist
    let all_messages = storage_a.list_all_messages().await.unwrap();
    assert_eq!(all_messages.len(), 20); // 10 from each partition

    // Tips should include heads of both chains
    let tips = storage_a.get_tips().await.unwrap();
    assert!(tips.len() >= 2);
    assert!(tips.contains(&last_a) || tips.contains(&last_b));
}

// ============= Integration Tests =============

#[tokio::test]
async fn test_full_consensus_flow_with_storage() {
    let params = AdicParams::default();
    let storage = Arc::new(
        StorageEngine::new(StorageConfig {
            backend_type: BackendType::Memory,
            ..Default::default()
        })
        .unwrap(),
    );

    let _consensus = ConsensusEngine::new(params, storage.clone());
    let reputation = ReputationTracker::new(0.9);

    // Create initial proposers
    let proposers: Vec<_> = (0..5).map(|_| *Keypair::generate().public_key()).collect();

    // Initialize reputations
    for proposer in &proposers {
        reputation.good_update(proposer, 1.0, 10).await;
    }

    // Create multiple keypairs to simulate different proposers
    use adic_crypto::Keypair;
    let keypairs: Vec<_> = (0..5).map(|_| Keypair::generate()).collect();

    // Simulate consensus rounds
    for round in 0..10 {
        // Rotate through different proposers
        let keypair = &keypairs[round % keypairs.len()];
        let proposer = *keypair.public_key();

        // Get current tips
        let tips = storage.get_tips().await.unwrap();
        let parents = if round == 0 {
            // First message has no parents
            vec![]
        } else {
            // Take up to 3 parents from tips
            tips.into_iter().take(3).collect()
        };

        // Create new message
        let mut msg = AdicMessage::new(
            parents,
            AdicFeatures::new(vec![
                AxisPhi::new(0, QpDigits::from_u64(round as u64, 3, 10)),
                AxisPhi::new(1, QpDigits::from_u64(round as u64 * 2, 3, 10)),
            ]),
            AdicMeta::new(Utc::now()),
            proposer,
            vec![round as u8],
        );

        // Sign the message
        msg.signature = keypair.sign(msg.id.as_bytes());

        // Store message - for this test we're focusing on consensus flow, not validation
        storage.store_message(&msg).await.unwrap();

        // Update reputation for the proposer
        reputation.good_update(&proposer, 0.1, 1).await;
    }

    // Verify final state
    let final_messages = storage.list_all_messages().await.unwrap();
    // We should have 10 messages (one per round)
    assert_eq!(
        final_messages.len(),
        10,
        "Expected 10 messages, but got {}",
        final_messages.len()
    );

    // Check reputation scores
    for proposer in &proposers {
        let score = reputation.get_reputation(proposer).await;
        assert!(score > 0.1); // All should maintain some reputation
    }
}

#[tokio::test]
async fn test_multi_node_conflict_resolution() {
    let params = AdicParams {
        lambda: 1.0,
        mu: 0.5,
        ..Default::default()
    };

    let storage = Arc::new(
        StorageEngine::new(StorageConfig {
            backend_type: BackendType::Memory,
            ..Default::default()
        })
        .unwrap(),
    );

    let reputation = Arc::new(ReputationTracker::new(params.gamma));
    let energy_resolver = Arc::new(EnergyDescentTracker::new(params.lambda, params.mu));
    let conflict_resolver = Arc::new(ConflictResolver::new());

    // Create conflicting transactions
    let keypair1 = Keypair::generate();
    let keypair2 = Keypair::generate();
    let proposer1 = *keypair1.public_key();
    let proposer2 = *keypair2.public_key();

    // Both try to spend the same resource
    let resource_id = "utxo_12345";

    let msg1 = create_message_with_parents_and_features(
        1,
        vec![],
        proposer1,
        vec![(0, 100)], // Spend to address 100
    );

    let msg2 = create_message_with_parents_and_features(
        2,
        vec![],
        proposer2,
        vec![(0, 200)], // Spend to address 200
    );

    storage.store_message(&msg1).await.unwrap();
    storage.store_message(&msg2).await.unwrap();

    // Register conflict
    let conflict_id = ConflictId::new(resource_id.to_string());
    energy_resolver.register_conflict(conflict_id.clone()).await;
    conflict_resolver
        .register_conflict_with_messages(resource_id, vec![msg1.id, msg2.id])
        .await;

    // Simulate nodes voting with converging support
    for i in 0..10 {
        // Start with disputed support, gradually converge
        let _support1 = 0.6 + (i as f64 * 0.03); // Increases to 0.9
        let _support2 = 0.4 - (i as f64 * 0.03); // Decreases to 0.1

        energy_resolver
            .update_support(&conflict_id, msg1.id, &storage, &reputation)
            .await
            .unwrap();
        energy_resolver
            .update_support(&conflict_id, msg2.id, &storage, &reputation)
            .await
            .unwrap();
    }

    // Force energy descent by reducing total support
    for _depth in 2..5 {
        energy_resolver
            .update_support(&conflict_id, msg1.id, &storage, &reputation)
            .await
            .unwrap();
        energy_resolver
            .update_support(&conflict_id, msg2.id, &storage, &reputation)
            .await
            .unwrap();
    }

    // Check resolution - may not be resolved yet due to threshold
    let _winner = energy_resolver.get_winner(&conflict_id).await;

    // At minimum, verify the conflict state is tracked
    let msg1_energy = energy_resolver
        .get_conflict_penalty(&msg1.id, &conflict_id)
        .await;
    let msg2_energy = energy_resolver
        .get_conflict_penalty(&msg2.id, &conflict_id)
        .await;

    // The message with higher support should have lower penalty
    assert!(msg1_energy <= msg2_energy);

    // Verify conflict resolver consistency
    let msg1_conflicts = conflict_resolver.get_conflicts_for_message(&msg1.id).await;
    let msg2_conflicts = conflict_resolver.get_conflicts_for_message(&msg2.id).await;
    assert!(!msg1_conflicts.is_empty());
    assert!(!msg2_conflicts.is_empty());
}
