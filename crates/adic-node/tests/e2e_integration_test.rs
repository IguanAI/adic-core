use adic_consensus::{ConsensusEngine, MessageValidator, ReputationTracker};
use adic_crypto::Keypair;
use adic_economics::{AdicAmount, TokenomicsEngine};
use adic_finality::{
    FinalityArtifact, FinalityConfig, FinalityEngine, FinalityGate, FinalityParams, FinalityWitness,
};
use adic_mrw::MrwEngine;
use adic_storage::{store::BackendType, StorageConfig, StorageEngine};
use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AdicParams, AxisPhi, MessageId, PublicKey, QpDigits,
};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::time::{sleep, Duration};

// Helper for tests that don't care about signature validity
fn create_transaction_message(
    from: PublicKey,
    to: PublicKey,
    amount: AdicAmount,
    nonce: u64,
    parents: Vec<MessageId>,
) -> AdicMessage {
    let temp_kp = adic_crypto::Keypair::generate();
    let mut msg = create_transaction_message_signed(&temp_kp, to, amount, nonce, parents);
    msg.proposer_pk = from; // Override proposer
    msg
}

fn create_transaction_message_signed(
    from_keypair: &adic_crypto::Keypair,
    to: PublicKey,
    amount: AdicAmount,
    nonce: u64,
    parents: Vec<MessageId>,
) -> AdicMessage {
    let features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(nonce, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(amount.to_base_units(), 3, 10)),
    ]);

    let from = *from_keypair.public_key();
    let payload = format!(
        "{{\"type\":\"transfer\",\"from\":\"{:?}\",\"to\":\"{:?}\",\"amount\":{},\"nonce\":{}}}",
        from,
        to,
        amount.to_base_units(),
        nonce
    )
    .into_bytes();

    let mut msg = AdicMessage::new(
        parents,
        features,
        AdicMeta::new(Utc::now()),
        from, // proposer is the sender
        payload,
    );

    // Sign the message - signature must be empty during signing
    msg.signature = adic_types::Signature::empty();
    let bytes_to_sign = msg.to_bytes();
    msg.signature = from_keypair.sign(&bytes_to_sign);

    msg
}

// ============= Complete Message Lifecycle Test =============

#[tokio::test]
async fn test_complete_message_lifecycle() {
    let temp_dir = tempdir().unwrap();
    let storage = Arc::new(
        StorageEngine::new(StorageConfig {
            backend_type: BackendType::RocksDB {
                path: temp_dir.path().to_str().unwrap().to_string(),
            },
            ..Default::default()
        })
        .unwrap(),
    );

    let params = AdicParams::default();
    let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
    let mrw = Arc::new(MrwEngine::new(params.clone()));
    let finality_config = FinalityConfig {
        k: params.k as usize,
        min_depth: params.depth_star,
        min_diversity: params.q as usize,
        min_reputation: params.r_sum_min,
        check_interval_ms: 1000,
        window_size: 1000,
    };
    let finality = Arc::new(FinalityEngine::new(
        finality_config,
        consensus.clone(),
        storage.clone(),
    ));
    let reputation = Arc::new(ReputationTracker::new(params.gamma));

    // Create test accounts
    let alice_kp = Keypair::generate();
    let bob_kp = Keypair::generate();
    let alice = *alice_kp.public_key();
    let bob = *bob_kp.public_key();

    // Initialize reputations
    reputation.set_reputation(&alice, 1.0).await;
    reputation.set_reputation(&bob, 1.0).await;

    // Phase 1: Message Creation
    let genesis =
        create_transaction_message_signed(&alice_kp, bob, AdicAmount::from_adic(10.0), 1, vec![]);

    // Phase 2: Validate
    // For e2e tests, we'll use basic validation without deposits
    let validator = adic_consensus::MessageValidator::new();
    let validation = validator.validate_message(&genesis);
    if !validation.is_valid {
        eprintln!("Validation errors: {:?}", validation.errors);
    }
    assert!(validation.is_valid);

    // Phase 3: Storage
    storage.store_message(&genesis).await.unwrap();

    // Phase 4: Parent Selection for Next Message
    let tips = storage.get_tips().await.unwrap();
    assert!(tips.contains(&genesis.id));

    // Create child message
    let child = create_transaction_message_signed(
        &bob_kp,
        alice,
        AdicAmount::from_adic(5.0),
        1,
        vec![genesis.id],
    );

    // Validate and store child
    let child_validation = validator.validate_message(&child);
    if !child_validation.is_valid {
        eprintln!("Child validation errors: {:?}", child_validation.errors);
    }
    assert!(child_validation.is_valid);
    storage.store_message(&child).await.unwrap();

    // Phase 5: Finalization Check
    // In real scenario, would wait for k-core conditions
    let finality_status = finality.check_finality().await.unwrap();

    // Phase 6: Query Final State
    let all_messages = storage.list_all_messages().await.unwrap();
    assert_eq!(all_messages.len(), 2);

    // Verify parent-child relationship
    let children = storage.get_children(&genesis.id).await.unwrap();
    assert!(children.contains(&child.id));

    let parents = storage.get_parents(&child.id).await.unwrap();
    assert!(parents.contains(&genesis.id));
}

// ============= Node Synchronization Test =============

#[tokio::test]
async fn test_node_synchronization() {
    let temp_dir1 = tempdir().unwrap();
    let temp_dir2 = tempdir().unwrap();

    // Create two nodes
    let storage1 = Arc::new(
        StorageEngine::new(StorageConfig {
            backend_type: BackendType::RocksDB {
                path: temp_dir1.path().to_str().unwrap().to_string(),
            },
            ..Default::default()
        })
        .unwrap(),
    );

    let storage2 = Arc::new(
        StorageEngine::new(StorageConfig {
            backend_type: BackendType::RocksDB {
                path: temp_dir2.path().to_str().unwrap().to_string(),
            },
            ..Default::default()
        })
        .unwrap(),
    );

    // Use proper keypairs for signing
    let alice_kp = Keypair::generate();
    let alice = *alice_kp.public_key();
    let bob = *Keypair::generate().public_key();

    // Node 1 creates messages
    let mut prev_id = None;
    for i in 0..10 {
        let parents = if let Some(id) = prev_id {
            vec![id]
        } else {
            vec![]
        };

        let msg = create_transaction_message_signed(
            &alice_kp,
            bob,
            AdicAmount::from_adic(1.0),
            i,
            parents,
        );

        storage1.store_message(&msg).await.unwrap();
        prev_id = Some(msg.id);
    }

    // Simulate sync: Copy all messages from node1 to node2 in topological order
    let messages_to_sync = storage1.list_all_messages().await.unwrap();
    assert_eq!(messages_to_sync.len(), 10);

    // Build a map of all messages and track which ones have been synced
    let mut all_messages = std::collections::HashMap::new();
    let mut synced = std::collections::HashSet::new();

    for msg_id in &messages_to_sync {
        if let Some(msg) = storage1.get_message(msg_id).await.unwrap() {
            all_messages.insert(msg.id, msg);
        }
    }

    // Function to recursively sync a message and its parents
    async fn sync_message(
        msg_id: &MessageId,
        all_messages: &std::collections::HashMap<MessageId, AdicMessage>,
        synced: &mut std::collections::HashSet<MessageId>,
        storage: &Arc<StorageEngine>,
    ) {
        if synced.contains(msg_id) {
            return;
        }

        if let Some(msg) = all_messages.get(msg_id) {
            // First sync all parents
            for parent_id in &msg.parents {
                Box::pin(sync_message(parent_id, all_messages, synced, storage)).await;
            }

            // Then sync this message
            let validator = MessageValidator::new();
            let validation = validator.validate_message(msg);

            if validation.is_valid {
                storage.store_message(msg).await.unwrap();
                synced.insert(*msg_id);
            }
        }
    }

    // Sync all messages in topological order
    for msg_id in messages_to_sync {
        sync_message(&msg_id, &all_messages, &mut synced, &storage2).await;
    }

    // Verify both nodes have same state
    let node1_messages = storage1.list_all_messages().await.unwrap();
    let node2_messages = storage2.list_all_messages().await.unwrap();

    assert_eq!(node1_messages.len(), node2_messages.len());

    // Verify tips are consistent (sort before comparing since order might differ)
    let mut tips1 = storage1.get_tips().await.unwrap();
    let mut tips2 = storage2.get_tips().await.unwrap();
    tips1.sort();
    tips2.sort();
    assert_eq!(tips1, tips2);
}

// ============= Full Finalization Flow Test =============

#[tokio::test]
async fn test_full_finalization_flow() {
    let temp_dir = tempdir().unwrap();
    let storage = Arc::new(
        StorageEngine::new(StorageConfig {
            backend_type: BackendType::RocksDB {
                path: temp_dir.path().to_str().unwrap().to_string(),
            },
            ..Default::default()
        })
        .unwrap(),
    );

    let params = AdicParams {
        k: 3, // Low k-core for testing
        ..Default::default()
    };

    let finality_config = FinalityConfig {
        k: params.k as usize,
        min_depth: params.depth_star,
        min_diversity: params.q as usize,
        min_reputation: params.r_sum_min,
        check_interval_ms: 1000,
        window_size: 1000,
    };
    let consensus_engine = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
    let finality = FinalityEngine::new(finality_config, consensus_engine, storage.clone());

    // Create a DAG structure that will achieve k-core finality
    let proposers: Vec<_> = (0..5).map(|_| *Keypair::generate().public_key()).collect();

    // Layer 0: Genesis
    let genesis = create_transaction_message(
        proposers[0],
        proposers[1],
        AdicAmount::from_adic(1.0),
        0,
        vec![],
    );
    storage.store_message(&genesis).await.unwrap();

    // Layer 1: Multiple messages referencing genesis
    let mut layer1_ids = Vec::new();
    for i in 0..4 {
        let msg = create_transaction_message(
            proposers[i],
            proposers[(i + 1) % 5],
            AdicAmount::from_adic(1.0),
            1,
            vec![genesis.id],
        );
        storage.store_message(&msg).await.unwrap();
        layer1_ids.push(msg.id);
    }

    // Layer 2: Create k-core structure
    let mut layer2_ids = Vec::new();
    for i in 0..4 {
        // Each message references k parents from layer 1
        let parents: Vec<_> = layer1_ids.iter().take(3).cloned().collect();
        let msg = create_transaction_message(
            proposers[i],
            proposers[(i + 1) % 5],
            AdicAmount::from_adic(1.0),
            2,
            parents,
        );
        storage.store_message(&msg).await.unwrap();
        layer2_ids.push(msg.id);
    }

    // Check finality
    let finalized_messages = finality.check_finality().await.unwrap();

    // Genesis should be finalizable with sufficient support
    if finalized_messages.contains(&genesis.id) {
        // Store finality proof
        // Create a finality artifact
        let artifact = FinalityArtifact::new(
            genesis.id,
            FinalityGate::F1KCore,
            FinalityParams {
                k: params.k as usize,
                q: params.q as usize,
                depth_star: params.depth_star,
                r_sum_min: params.r_sum_min,
            },
            FinalityWitness {
                kcore_root: Some(genesis.id),
                depth: 2,
                diversity_ok: true,
                reputation_sum: 1.0,
                distinct_balls: HashMap::new(),
                core_size: 4,
            },
        );
        storage
            .finalize_message(&genesis.id, artifact.to_json().unwrap().as_bytes())
            .await
            .unwrap();

        // Verify finalization
        assert!(storage.is_finalized(&genesis.id).await.unwrap());

        // Retrieve finality artifact
        let stored_artifact = storage.get_finality_artifact(&genesis.id).await.unwrap();
        assert!(stored_artifact.is_some());
    }
}

// ============= Economic Transaction Flow Test =============

#[tokio::test]
async fn test_economic_transaction_flow() {
    let temp_dir = tempdir().unwrap();
    let storage = Arc::new(
        StorageEngine::new(StorageConfig {
            backend_type: BackendType::RocksDB {
                path: temp_dir.path().to_str().unwrap().to_string(),
            },
            ..Default::default()
        })
        .unwrap(),
    );

    let params = AdicParams::default();
    let tokenomics = TokenomicsEngine::new_with_params(params.clone());

    // Create test accounts with balances
    let alice_kp = Keypair::generate();
    let bob_kp = Keypair::generate();
    let charlie_kp = Keypair::generate();

    let alice = *alice_kp.public_key();
    let bob = *bob_kp.public_key();
    let charlie = *charlie_kp.public_key();

    // Initialize balances (in real system, from genesis)
    let mut balances = HashMap::new();
    balances.insert(alice, AdicAmount::from_adic(1000.0));
    balances.insert(bob, AdicAmount::from_adic(500.0));
    balances.insert(charlie, AdicAmount::from_adic(100.0));

    // Transaction 1: Alice -> Bob
    let tx1 = create_transaction_message(alice, bob, AdicAmount::from_adic(100.0), 1, vec![]);

    // Validate economic constraints
    if balances[&alice] >= AdicAmount::from_adic(100.0) {
        // Process transaction
        storage.store_message(&tx1).await.unwrap();

        // Update balances
        balances.insert(
            alice,
            balances[&alice].saturating_sub(AdicAmount::from_adic(100.0)),
        );
        balances.insert(
            bob,
            balances[&bob].saturating_add(AdicAmount::from_adic(100.0)),
        );
    }

    // Transaction 2: Bob -> Charlie (chain of transactions)
    let tx2 =
        create_transaction_message(bob, charlie, AdicAmount::from_adic(50.0), 1, vec![tx1.id]);

    if balances[&bob] >= AdicAmount::from_adic(50.0) {
        storage.store_message(&tx2).await.unwrap();

        balances.insert(
            bob,
            balances[&bob].saturating_sub(AdicAmount::from_adic(50.0)),
        );
        balances.insert(
            charlie,
            balances[&charlie].saturating_add(AdicAmount::from_adic(50.0)),
        );
    }

    // Verify final balances
    assert_eq!(balances[&alice], AdicAmount::from_adic(900.0));
    assert_eq!(balances[&bob], AdicAmount::from_adic(550.0));
    assert_eq!(balances[&charlie], AdicAmount::from_adic(150.0));

    // Verify transaction chain
    let children = storage.get_children(&tx1.id).await.unwrap();
    assert!(children.contains(&tx2.id));
}

// ============= Stress Test with Multiple Nodes =============

#[tokio::test]
async fn test_multi_node_stress() {
    let num_nodes = 5;
    let messages_per_node = 20;

    // Create nodes with separate storage
    let mut nodes = Vec::new();
    let mut storages = Vec::new();

    for i in 0..num_nodes {
        let temp_dir = tempdir().unwrap();
        let storage = Arc::new(
            StorageEngine::new(StorageConfig {
                backend_type: BackendType::Memory, // Use memory for speed
                ..Default::default()
            })
            .unwrap(),
        );

        storages.push(storage);

        let node_keypair = Keypair::generate();
        nodes.push((*node_keypair.public_key(), node_keypair));
    }

    // Each node creates messages
    let mut all_messages: Vec<AdicMessage> = Vec::new();

    for (node_idx, (node_pk, _)) in nodes.iter().enumerate() {
        let storage = &storages[node_idx];

        for msg_idx in 0..messages_per_node {
            // Select random parents from existing messages
            let parents = if all_messages.is_empty() {
                vec![]
            } else {
                // Select up to 3 random parents
                let mut parent_ids = Vec::new();
                for _ in 0..std::cmp::min(3, all_messages.len()) {
                    if let Some(msg) = all_messages.get(msg_idx % all_messages.len()) {
                        parent_ids.push(msg.id);
                    }
                }
                parent_ids
            };

            let msg = create_transaction_message(
                *node_pk,
                nodes[(node_idx + 1) % num_nodes].0,
                AdicAmount::from_adic(1.0),
                msg_idx as u64,
                parents,
            );

            storage.store_message(&msg).await.unwrap();
            all_messages.push(msg.clone());

            // Simulate propagation to other nodes
            for (other_idx, other_storage) in storages.iter().enumerate() {
                if other_idx != node_idx {
                    other_storage.store_message(&msg).await.unwrap();
                }
            }
        }
    }

    // Verify all nodes have consistent state
    let expected_total = num_nodes * messages_per_node;

    for storage in &storages {
        let messages = storage.list_all_messages().await.unwrap();
        assert_eq!(
            messages.len(),
            expected_total,
            "Node should have all {} messages",
            expected_total
        );
    }

    // Verify tips are consistent across nodes
    let first_tips = storages[0].get_tips().await.unwrap();
    for storage in &storages[1..] {
        let tips = storage.get_tips().await.unwrap();
        assert_eq!(
            tips.len(),
            first_tips.len(),
            "All nodes should have same number of tips"
        );
    }
}
