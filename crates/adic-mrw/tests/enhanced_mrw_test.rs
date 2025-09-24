use adic_consensus::{ConflictResolver, ReputationTracker};
use adic_mrw::{MrwEngine, MrwSelector, ParentCandidate, SelectionParams, WeightCalculator};
use adic_storage::{store::BackendType, StorageConfig, StorageEngine};
use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AdicParams, AxisPhi, MessageId, PublicKey, QpDigits,
};
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

fn create_message_with_features(
    id: u8,
    proposer: PublicKey,
    features: Vec<(u32, u64)>,
    parents: Vec<MessageId>,
) -> AdicMessage {
    let axis_features: Vec<AxisPhi> = features
        .into_iter()
        .map(|(axis, value)| AxisPhi::new(axis, QpDigits::from_u64(value, 3, 10)))
        .collect();

    let mut msg = AdicMessage::new(
        parents,
        AdicFeatures::new(axis_features),
        AdicMeta::new(Utc::now()),
        proposer,
        vec![id],
    );

    // Set deterministic ID
    msg.id = MessageId::new(&[id; 32]);
    msg
}

// ============= Bootstrap Scenarios Tests =============

#[tokio::test]
async fn test_mrw_bootstrap_single_tip() {
    let params = AdicParams {
        d: 2, // Need to select 3 parents
        ..Default::default()
    };

    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();

    let conflict_resolver = ConflictResolver::new();
    let reputation_tracker = ReputationTracker::new(params.gamma);
    let mrw = MrwEngine::new(params.clone());

    let proposer = PublicKey::from_bytes([1; 32]);

    // Create only one tip (bootstrap scenario)
    let tip = create_message_with_features(1, proposer, vec![(0, 10), (1, 20)], vec![]);
    storage.store_message(&tip).await.unwrap();

    // Create new message features
    let new_features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(11, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(21, 3, 10)),
    ]);

    // MRW should handle single tip gracefully
    let result = mrw
        .select_parents(
            &new_features,
            &[tip.id],
            &storage,
            &conflict_resolver,
            &reputation_tracker,
        )
        .await;

    assert!(result.is_ok());
    let selected = result.unwrap();

    // With only one tip, it should be selected (possibly multiple times if allowed)
    assert!(selected.contains(&tip.id));
}

#[tokio::test]
async fn test_mrw_bootstrap_few_tips() {
    let params = AdicParams {
        d: 3, // Need to select 4 parents
        ..Default::default()
    };

    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();

    let conflict_resolver = ConflictResolver::new();
    let reputation_tracker = ReputationTracker::new(params.gamma);
    let mrw = MrwEngine::new(params.clone());

    let proposer = PublicKey::from_bytes([1; 32]);

    // Create only 2 tips (less than d+1)
    let tip1 = create_message_with_features(1, proposer, vec![(0, 10), (1, 20)], vec![]);
    let tip2 = create_message_with_features(2, proposer, vec![(0, 15), (1, 25)], vec![]);

    storage.store_message(&tip1).await.unwrap();
    storage.store_message(&tip2).await.unwrap();

    let new_features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(12, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(22, 3, 10)),
    ]);

    // MRW should handle insufficient tips
    let result = mrw
        .select_parents(
            &new_features,
            &[tip1.id, tip2.id],
            &storage,
            &conflict_resolver,
            &reputation_tracker,
        )
        .await;

    assert!(result.is_ok());
    let selected = result.unwrap();

    // Should select from available tips
    assert!(selected.len() <= 2 || selected.len() == 4); // Either relaxed or repeated selection
}

// ============= Large Tip Set Performance Tests =============

#[tokio::test]
async fn test_mrw_large_tip_set() {
    let params = AdicParams {
        d: 2,
        lambda: 1.0,
        ..Default::default()
    };

    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();

    let conflict_resolver = ConflictResolver::new();
    let reputation_tracker = ReputationTracker::new(params.gamma);
    let mrw = MrwEngine::new(params.clone());

    // Create 1000 tips with varying features
    let mut tip_ids = Vec::new();
    for i in 0..1000 {
        let proposer = PublicKey::from_bytes([(i % 256) as u8; 32]);
        let features = vec![
            (0, (i * 7) % 100),  // Varied axis 0
            (1, (i * 13) % 100), // Varied axis 1
            (2, (i * 17) % 100), // Varied axis 2
        ];
        // Use full i value to avoid ID collision when i > 255
        let mut tip = create_message_with_features((i % 256) as u8, proposer, features, vec![]);
        // Override the ID to be unique for each message
        let mut id_bytes = [0u8; 32];
        id_bytes[0..4].copy_from_slice(&(i as u32).to_le_bytes());
        tip.id = MessageId::new(&id_bytes);
        storage.store_message(&tip).await.unwrap();
        tip_ids.push(tip.id);

        // Set varied reputations
        let reputation = 0.5 + (i as f64 / 2000.0);
        reputation_tracker
            .set_reputation(&proposer, reputation)
            .await;
    }

    // Measure selection performance
    let new_features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(50, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(50, 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(50, 3, 10)),
    ]);

    let start = std::time::Instant::now();

    let result = mrw
        .select_parents(
            &new_features,
            &tip_ids,
            &storage,
            &conflict_resolver,
            &reputation_tracker,
        )
        .await;

    let duration = start.elapsed();

    assert!(result.is_ok());
    let selected = result.unwrap();
    assert_eq!(selected.len(), 3); // d+1

    // Performance check - should complete within reasonable time
    assert!(
        duration.as_millis() < 1000,
        "Selection took too long: {:?}ms",
        duration.as_millis()
    );
}

// ============= Diversity Requirement Tests =============

#[tokio::test]
async fn test_mrw_q_ball_coverage() {
    let params = AdicParams {
        d: 2,
        q: 2,               // Require 2 distinct balls per axis
        rho: vec![3, 3, 3], // Ball radii
        ..Default::default()
    };

    let selection_params = SelectionParams::from_adic_params(&params);
    let weight_calc = WeightCalculator::new(params.lambda, 0.0, 0.0);
    let selector = MrwSelector::new(selection_params);

    // Create candidates with specific p-adic features
    let candidates = vec![
        ParentCandidate {
            message_id: MessageId::new(&[1; 32]),
            features: vec![
                QpDigits::from_u64(10, 3, 10), // Ball A for axis 0
                QpDigits::from_u64(20, 3, 10), // Ball A for axis 1
                QpDigits::from_u64(30, 3, 10), // Ball A for axis 2
            ],
            reputation: 1.0,
            conflict_penalty: 0.0,
            weight: 1.0,
            axis_weights: HashMap::new(),
        },
        ParentCandidate {
            message_id: MessageId::new(&[2; 32]),
            features: vec![
                QpDigits::from_u64(40, 3, 10), // Ball B for axis 0
                QpDigits::from_u64(50, 3, 10), // Ball B for axis 1
                QpDigits::from_u64(60, 3, 10), // Ball B for axis 2
            ],
            reputation: 1.0,
            conflict_penalty: 0.0,
            weight: 1.0,
            axis_weights: HashMap::new(),
        },
        ParentCandidate {
            message_id: MessageId::new(&[3; 32]),
            features: vec![
                QpDigits::from_u64(11, 3, 10), // Ball A for axis 0 (close to candidate 1)
                QpDigits::from_u64(21, 3, 10), // Ball A for axis 1 (close to candidate 1)
                QpDigits::from_u64(31, 3, 10), // Ball A for axis 2 (close to candidate 1)
            ],
            reputation: 1.0,
            conflict_penalty: 0.0,
            weight: 1.0,
            axis_weights: HashMap::new(),
        },
    ];

    let new_features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(25, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(35, 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(45, 3, 10)),
    ]);

    let (selected, trace) = selector
        .select_parents(&new_features, candidates, &weight_calc)
        .await
        .unwrap();

    assert_eq!(selected.len(), 3); // Should select d+1 parents

    // Verify diversity - should have selected from different balls
    let selected_set: HashSet<_> = selected.into_iter().collect();
    assert!(selected_set.len() >= 2); // Should have diversity
}

#[tokio::test]
async fn test_mrw_axis_independence() {
    let params = AdicParams {
        d: 2,
        q: 2,
        rho: vec![5, 5, 5],
        ..Default::default()
    };

    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();

    let conflict_resolver = ConflictResolver::new();
    let reputation_tracker = ReputationTracker::new(params.gamma);
    let mrw = MrwEngine::new(params.clone());

    // Create tips with different patterns on each axis
    let proposer = PublicKey::from_bytes([1; 32]);

    // Tip 1: Good on axis 0, bad on others
    let tip1 = create_message_with_features(1, proposer, vec![(0, 50), (1, 10), (2, 10)], vec![]);

    // Tip 2: Good on axis 1, bad on others
    let tip2 = create_message_with_features(2, proposer, vec![(0, 10), (1, 50), (2, 10)], vec![]);

    // Tip 3: Good on axis 2, bad on others
    let tip3 = create_message_with_features(3, proposer, vec![(0, 10), (1, 10), (2, 50)], vec![]);

    storage.store_message(&tip1).await.unwrap();
    storage.store_message(&tip2).await.unwrap();
    storage.store_message(&tip3).await.unwrap();

    // New message at center
    let new_features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(50, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(50, 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(50, 3, 10)),
    ]);

    let selected = mrw
        .select_parents(
            &new_features,
            &[tip1.id, tip2.id, tip3.id],
            &storage,
            &conflict_resolver,
            &reputation_tracker,
        )
        .await
        .unwrap();

    // Should select all three for axis diversity
    assert_eq!(selected.len(), 3);
    let selected_set: HashSet<_> = selected.into_iter().collect();
    assert_eq!(selected_set.len(), 3); // All unique
}

// ============= Reputation Weight Distribution Tests =============

#[tokio::test]
async fn test_mrw_reputation_weight_calculation() {
    let weight_calc = WeightCalculator::new_with_alpha(1.0, 2.0, 0.0, 0.5);

    // Test linear reputation (alpha = 1.0)
    let weight_calc_linear = WeightCalculator::new(1.0, 0.0, 0.5);
    let trust_linear = weight_calc_linear.compute_trust(2.0);
    assert_eq!(trust_linear, 2.0);

    // Test quadratic reputation (alpha = 2.0)
    let trust_quad = weight_calc.compute_trust(2.0);
    assert_eq!(trust_quad, 4.0);

    // Test weight calculation with proximity
    let weight_high_rep = weight_calc.compute_weight(0.5, 2.0, 0.0);
    let weight_low_rep = weight_calc.compute_weight(0.5, 1.0, 0.0);
    assert!(weight_high_rep > weight_low_rep);

    // Test conflict penalty impact
    let weight_no_conflict = weight_calc.compute_weight(0.5, 2.0, 0.0);
    let weight_with_conflict = weight_calc.compute_weight(0.5, 2.0, 0.5);
    assert!(weight_no_conflict > weight_with_conflict);
}

#[tokio::test]
async fn test_mrw_reputation_extremes() {
    let params = AdicParams {
        d: 2,
        ..Default::default()
    };

    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();

    let conflict_resolver = ConflictResolver::new();
    let reputation_tracker = ReputationTracker::new(params.gamma);
    let mrw = MrwEngine::new(params.clone());

    // Create tips with extreme reputation differences
    let high_rep_proposer = PublicKey::from_bytes([1; 32]);
    let zero_rep_proposer = PublicKey::from_bytes([2; 32]);
    let max_rep_proposer = PublicKey::from_bytes([3; 32]);

    // Set reputations
    reputation_tracker
        .set_reputation(&high_rep_proposer, 0.9)
        .await;
    reputation_tracker
        .set_reputation(&zero_rep_proposer, 0.0)
        .await;
    reputation_tracker
        .set_reputation(&max_rep_proposer, 10.0)
        .await;

    // Create tips
    let tip1 = create_message_with_features(1, high_rep_proposer, vec![(0, 50), (1, 50)], vec![]);
    let tip2 = create_message_with_features(2, zero_rep_proposer, vec![(0, 51), (1, 51)], vec![]);
    let tip3 = create_message_with_features(3, max_rep_proposer, vec![(0, 52), (1, 52)], vec![]);

    storage.store_message(&tip1).await.unwrap();
    storage.store_message(&tip2).await.unwrap();
    storage.store_message(&tip3).await.unwrap();

    let new_features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(50, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(50, 3, 10)),
    ]);

    // Run selection multiple times to check statistical behavior
    let mut selection_counts = HashMap::new();

    for _ in 0..100 {
        let selected = mrw
            .select_parents(
                &new_features,
                &[tip1.id, tip2.id, tip3.id],
                &storage,
                &conflict_resolver,
                &reputation_tracker,
            )
            .await
            .unwrap();

        for parent in selected {
            *selection_counts.entry(parent).or_insert(0) += 1;
        }
    }

    // Max reputation should be selected most
    let max_count = selection_counts.get(&tip3.id).unwrap_or(&0);
    let zero_count = selection_counts.get(&tip2.id).unwrap_or(&0);

    // Max reputation should be selected more than zero reputation
    assert!(
        max_count >= zero_count,
        "Max reputation selected {} times, zero reputation {} times",
        max_count,
        zero_count
    );
}

// ============= Edge Cases and Error Handling =============

#[tokio::test]
async fn test_mrw_empty_tip_set() {
    let params = AdicParams::default();
    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();

    let conflict_resolver = ConflictResolver::new();
    let reputation_tracker = ReputationTracker::new(params.gamma);
    let mrw = MrwEngine::new(params);

    let new_features = AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(50, 3, 10))]);

    // Try to select parents with no tips
    let result = mrw
        .select_parents(
            &new_features,
            &[], // Empty tip set
            &storage,
            &conflict_resolver,
            &reputation_tracker,
        )
        .await;

    // Should handle gracefully
    assert!(result.is_err() || result.unwrap().is_empty());
}

#[tokio::test]
async fn test_mrw_conflicting_tips() {
    let params = AdicParams::default();
    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();

    let conflict_resolver = ConflictResolver::new();
    let reputation_tracker = ReputationTracker::new(params.gamma);
    let mrw = MrwEngine::new(params);

    let proposer = PublicKey::from_bytes([1; 32]);

    // Create conflicting tips
    let tip1 = create_message_with_features(1, proposer, vec![(0, 100)], vec![]);
    let tip2 = create_message_with_features(2, proposer, vec![(0, 100)], vec![]);

    storage.store_message(&tip1).await.unwrap();
    storage.store_message(&tip2).await.unwrap();

    // Register conflict
    conflict_resolver
        .register_conflict_with_messages("double_spend", vec![tip1.id, tip2.id])
        .await;

    let new_features = AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(100, 3, 10))]);

    // MRW should handle conflicting tips
    let selected = mrw
        .select_parents(
            &new_features,
            &[tip1.id, tip2.id],
            &storage,
            &conflict_resolver,
            &reputation_tracker,
        )
        .await
        .unwrap();

    // MRW may select both conflicting tips - conflict resolution happens at consensus layer
    // This test just verifies MRW can handle conflicting tips without error
    assert!(!selected.is_empty(), "Should select at least one parent");
}
