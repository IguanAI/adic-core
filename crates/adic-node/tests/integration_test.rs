use adic_consensus::{ConsensusEngine, AdmissibilityChecker};
use adic_crypto::Keypair;
use adic_finality::{FinalityEngine, FinalityConfig};
use adic_math::{vp_diff, proximity_score};
use adic_mrw::{MrwSelector, SelectionParams, ParentCandidate, WeightCalculator};
use adic_storage::{StorageEngine, StorageConfig, StorageBackend};
use adic_types::*;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::test]
async fn test_full_consensus_flow() {
    // Initialize parameters
    let params = AdicParams::default();
    
    // Create storage
    let storage_config = StorageConfig::default();
    let storage = StorageEngine::new(storage_config).unwrap();
    
    // Create consensus engine
    let consensus = Arc::new(ConsensusEngine::new(params.clone()));
    let checker = AdmissibilityChecker::new(params.clone());
    
    // Create finality engine
    let finality_config = FinalityConfig::from(&params);
    let finality = FinalityEngine::new(finality_config, consensus.clone());
    
    // Generate keypair
    let keypair = Keypair::generate();
    
    // Create genesis message
    let genesis = create_test_message(vec![], 0, &keypair);
    storage.store_message(&genesis).await.unwrap();
    
    // Create chain of messages
    let mut current_tip = genesis.id;
    let mut all_messages = vec![genesis];
    
    for i in 1..10 {
        let message = create_test_message(vec![current_tip], i, &keypair);
        
        // Store message
        storage.store_message(&message).await.unwrap();
        
        // Add to finality engine
        let mut ball_ids = HashMap::new();
        for axis_phi in &message.features.phi {
            ball_ids.insert(axis_phi.axis.0, axis_phi.qp_digits.ball_id(3));
        }
        
        finality.add_message(
            message.id,
            vec![current_tip],
            1.0,
            ball_ids,
        ).await.unwrap();
        
        current_tip = message.id;
        all_messages.push(message);
    }
    
    // Check finality
    let finalized = finality.check_finality().await.unwrap();
    
    // Verify storage
    let stats = storage.get_stats().await.unwrap();
    assert_eq!(stats.message_count, 10);
    assert_eq!(stats.tip_count, 1);
    
    // Verify we can retrieve messages
    for msg in &all_messages {
        let retrieved = storage.get_message(&msg.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, msg.id);
    }
}

#[tokio::test]
async fn test_admissibility_constraints() {
    let params = AdicParams::default();
    let checker = AdmissibilityChecker::new(params.clone());
    let keypair = Keypair::generate();
    
    // Create parent messages with specific features
    let parent_features = vec![
        vec![
            QpDigits::from_u64(10, 3, 10),
            QpDigits::from_u64(20, 3, 10),
            QpDigits::from_u64(30, 3, 10),
        ],
        vec![
            QpDigits::from_u64(11, 3, 10),
            QpDigits::from_u64(21, 3, 10),
            QpDigits::from_u64(31, 3, 10),
        ],
        vec![
            QpDigits::from_u64(12, 3, 10),
            QpDigits::from_u64(22, 3, 10),
            QpDigits::from_u64(32, 3, 10),
        ],
        vec![
            QpDigits::from_u64(13, 3, 10),
            QpDigits::from_u64(23, 3, 10),
            QpDigits::from_u64(33, 3, 10),
        ],
    ];
    
    let parent_reputations = vec![1.0, 1.5, 2.0, 0.5];
    
    // Create a message that should pass admissibility
    let message = AdicMessage::new(
        vec![MessageId::new(b"p1"), MessageId::new(b"p2"), MessageId::new(b"p3"), MessageId::new(b"p4")],
        AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(10, 3, 10)),
            AxisPhi::new(1, QpDigits::from_u64(20, 3, 10)),
            AxisPhi::new(2, QpDigits::from_u64(30, 3, 10)),
        ]),
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        b"test".to_vec(),
    );
    
    let result = checker.check_message(&message, &parent_features, &parent_reputations).unwrap();
    
    // Check individual constraints
    println!("C1 (Proximity): {}", result.c1_passed);
    println!("C2 (Diversity): {}", result.c2_passed);
    println!("C3 (Reputation): {}", result.c3_passed);
    println!("Overall admissible: {}", result.is_admissible);
}

#[tokio::test]
async fn test_mrw_parent_selection() {
    let params = AdicParams::default();
    
    // Create MRW selector
    let selection_params = SelectionParams::from_adic_params(&params);
    let selector = MrwSelector::new(selection_params);
    let weight_calc = WeightCalculator::new(params.lambda, params.beta, params.mu);
    
    // Create candidate tips
    let mut candidates = Vec::new();
    for i in 0..10 {
        candidates.push(ParentCandidate {
            id: MessageId::new(format!("tip{}", i).as_bytes()),
            features: AdicFeatures::new(vec![
                AxisPhi::new(0, QpDigits::from_u64(i as u64, 3, 10)),
                AxisPhi::new(1, QpDigits::from_u64((i * 2) as u64, 3, 10)),
                AxisPhi::new(2, QpDigits::from_u64((i * 3) as u64, 3, 10)),
            ]),
            reputation: 1.0 + (i as f64) * 0.1,
            depth: 10 + i as u32,
            descendant_count: 100 - i * 10,
        });
    }
    
    // Create message features
    let message_features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(5, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(10, 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(15, 3, 10)),
    ]);
    
    // Select parents
    let (selected, trace) = selector.select_parents(
        &message_features,
        candidates.clone(),
        &weight_calc,
    ).await.unwrap();
    
    // Verify selection
    assert_eq!(selected.len(), (params.d + 1) as usize);
    
    // Check diversity
    let mut balls_per_axis: HashMap<u32, HashSet<Vec<u8>>> = HashMap::new();
    for parent_id in &selected {
        let parent = candidates.iter().find(|c| c.id == *parent_id).unwrap();
        for (idx, axis_phi) in parent.features.phi.iter().enumerate() {
            balls_per_axis.entry(idx as u32)
                .or_insert_with(HashSet::new)
                .insert(axis_phi.qp_digits.ball_id(3));
        }
    }
    
    // Each axis should have at least q distinct balls
    for (_axis, balls) in balls_per_axis {
        assert!(balls.len() >= params.q as usize);
    }
    
    println!("Selected {} parents with trace length {}", selected.len(), trace.len());
}

#[test]
fn test_padic_mathematics() {
    // Test vp_diff
    let x = QpDigits::from_u64(27, 3, 10); // 27 = 1*3^0 + 0*3^1 + 0*3^2 + 1*3^3
    let y = QpDigits::from_u64(9, 3, 10);  // 9 = 0*3^0 + 0*3^1 + 1*3^2
    
    let diff = vp_diff(&x, &y);
    assert_eq!(diff, 0); // They differ at position 0
    
    // Test with same prefix
    let x = QpDigits::from_u64(10, 3, 10); // 10 = 1*3^0 + 0*3^1 + 1*3^2
    let y = QpDigits::from_u64(19, 3, 10); // 19 = 1*3^0 + 0*3^1 + 2*3^2
    
    let diff = vp_diff(&x, &y);
    assert_eq!(diff, 2); // They differ at position 2
    
    // Test proximity score
    let score = proximity_score(&x, &y, 3);
    assert!(score > 0.0);
    
    // Test ball identification
    let ball_id = x.ball_id(2);
    assert_eq!(ball_id.len(), 2);
}

#[tokio::test]
async fn test_finality_kcore() {
    let params = AdicParams::default();
    let consensus = Arc::new(ConsensusEngine::new(params.clone()));
    let finality_config = FinalityConfig::from(&params);
    let finality = FinalityEngine::new(finality_config, consensus.clone());
    
    // Create a k-core structure
    // Need at least k nodes with degree >= k
    let k = params.k as usize;
    
    // Create hub node
    let hub = MessageId::new(b"hub");
    finality.add_message(hub, vec![], 2.0, HashMap::new()).await.unwrap();
    
    // Create k peripheral nodes all connecting to hub and each other
    let mut peripherals = Vec::new();
    for i in 0..k {
        let node = MessageId::new(format!("node{}", i).as_bytes());
        
        // Connect to hub and previous nodes to build k-core
        let mut parents = vec![hub];
        for j in 0..i.min(k-1) {
            parents.push(peripherals[j]);
        }
        
        finality.add_message(
            node,
            parents,
            1.5,
            HashMap::new(),
        ).await.unwrap();
        
        peripherals.push(node);
    }
    
    // Check finality
    let finalized = finality.check_finality().await.unwrap();
    
    // Get stats
    let stats = finality.get_stats().await;
    println!("Finality stats: {:?}", stats);
    
    assert!(stats.pending_count > 0);
}

// Helper function to create test messages
fn create_test_message(parents: Vec<MessageId>, index: u64, keypair: &Keypair) -> AdicMessage {
    AdicMessage::new(
        parents,
        AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(index, 3, 10)),
            AxisPhi::new(1, QpDigits::from_u64(index * 2, 3, 10)),
            AxisPhi::new(2, QpDigits::from_u64(index * 3, 3, 10)),
        ]),
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        format!("Message {}", index).into_bytes(),
    )
}

use std::collections::HashSet;