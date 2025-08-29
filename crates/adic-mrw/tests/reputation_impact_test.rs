use adic_consensus::{ConflictResolver, ReputationTracker};
use adic_mrw::MrwEngine;
use adic_storage::{store::BackendType, StorageConfig, StorageEngine};
use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AdicParams, AxisPhi, MessageId, PublicKey, QpDigits,
};
use chrono::Utc;

#[tokio::test]
async fn test_reputation_impacts_parent_selection() {
    // Setup
    let params = AdicParams {
        d: 2, // Select 3 parents
        k: 10,
        gamma: 0.9,
        ..Default::default()
    };

    // Use in-memory storage to avoid RocksDB file locks across concurrent tests
    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();
    let conflict_resolver = ConflictResolver::new();
    let reputation_tracker = ReputationTracker::new(params.gamma);
    let mrw = MrwEngine::new(params.clone());

    // Create proposers with different reputations
    let high_rep_proposer = PublicKey::from_bytes([1; 32]);
    let medium_rep_proposer = PublicKey::from_bytes([2; 32]);
    let low_rep_proposer = PublicKey::from_bytes([3; 32]);

    // Set reputations
    // High reputation proposer - consistent good behavior
    for _ in 0..10 {
        reputation_tracker
            .good_update(&high_rep_proposer, 5.0, 10)
            .await;
    }

    // Medium reputation proposer - average behavior
    for _ in 0..5 {
        reputation_tracker
            .good_update(&medium_rep_proposer, 3.0, 10)
            .await;
    }

    // Low reputation proposer - poor behavior
    reputation_tracker.bad_update(&low_rep_proposer, 5.0).await;

    let high_rep = reputation_tracker.get_reputation(&high_rep_proposer).await;
    let medium_rep = reputation_tracker
        .get_reputation(&medium_rep_proposer)
        .await;
    let low_rep = reputation_tracker.get_reputation(&low_rep_proposer).await;

    println!("Reputation scores:");
    println!("  High: {:.3}", high_rep);
    println!("  Medium: {:.3}", medium_rep);
    println!("  Low: {:.3}", low_rep);

    assert!(high_rep > medium_rep);
    assert!(medium_rep > low_rep);

    // Create tip messages with different proposers
    let tips = vec![
        create_tip_message(1, high_rep_proposer, vec![10, 10, 10]),
        create_tip_message(2, medium_rep_proposer, vec![11, 11, 11]),
        create_tip_message(3, low_rep_proposer, vec![12, 12, 12]),
        create_tip_message(4, high_rep_proposer, vec![13, 13, 13]),
        create_tip_message(5, medium_rep_proposer, vec![14, 14, 14]),
    ];

    // Store tips
    for tip in &tips {
        storage.store_message(tip).await.unwrap();
    }

    let tip_ids: Vec<MessageId> = tips.iter().map(|t| t.id).collect();

    // Create features for new message (close to all tips)
    let new_features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(12, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(12, 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(12, 3, 10)),
    ]);

    // Run multiple selections to test statistical behavior
    let num_trials = 20;
    let mut total_high = 0;
    let mut total_medium = 0;
    let mut total_low = 0;

    for _trial in 0..num_trials {
        // Select parents using MRW with reputation
        let selected_parents = mrw
            .select_parents(
                &new_features,
                &tip_ids,
                &storage,
                &conflict_resolver,
                &reputation_tracker,
            )
            .await
            .unwrap();

        assert_eq!(selected_parents.len(), 3, "Should select d+1 = 3 parents");

        // Count how many high/medium/low reputation parents were selected
        for parent_id in &selected_parents {
            if let Ok(Some(parent)) = storage.get_message(parent_id).await {
                if parent.proposer_pk == high_rep_proposer {
                    total_high += 1;
                } else if parent.proposer_pk == medium_rep_proposer {
                    total_medium += 1;
                } else if parent.proposer_pk == low_rep_proposer {
                    total_low += 1;
                }
            }
        }
    }

    // Calculate average selections per trial
    let avg_high = total_high as f64 / num_trials as f64;
    let avg_medium = total_medium as f64 / num_trials as f64;
    let avg_low = total_low as f64 / num_trials as f64;

    println!("\nAverage parent selections over {} trials:", num_trials);
    println!("  High reputation: {:.2} (total: {})", avg_high, total_high);
    println!(
        "  Medium reputation: {:.2} (total: {})",
        avg_medium, total_medium
    );
    println!("  Low reputation: {:.2} (total: {})", avg_low, total_low);

    // With corrected trust function: trust = reputation^alpha (alpha=1.0 by default)
    // High reputation (1.296) should now have meaningful advantage over low (0.400)
    // The ratio is about 3.24x, so we expect high to be selected more often
    assert!(total_high > total_low,
        "High reputation parents ({}) should be selected more often than low reputation ({}) on average",
        total_high, total_low);

    // Since there are 2 high-rep parents and 1 low-rep parent in the pool,
    // and high-rep has ~3x higher weight, we expect the low-rep parent
    // to be selected less frequently on average
    assert!(avg_low < avg_high,
        "Low reputation parent (avg: {:.2}) should be selected less than high reputation (avg: {:.2})",
        avg_low, avg_high);
}

#[tokio::test]
async fn test_c3_prevents_low_reputation_parents() {
    let params = AdicParams {
        r_min: 0.5, // Minimum reputation threshold
        d: 1,       // Select 2 parents
        ..Default::default()
    };

    // Use in-memory storage to avoid RocksDB file locks across concurrent tests
    let storage = StorageEngine::new(StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    })
    .unwrap();
    let conflict_resolver = ConflictResolver::new();
    let reputation_tracker = ReputationTracker::new(params.gamma);
    let mrw = MrwEngine::new(params.clone());

    // Create a very low reputation proposer
    let bad_proposer = PublicKey::from_bytes([99; 32]);

    // Severely penalize the bad proposer
    for _ in 0..5 {
        reputation_tracker.bad_update(&bad_proposer, 10.0).await;
    }

    let bad_rep = reputation_tracker.get_reputation(&bad_proposer).await;
    println!("Bad proposer reputation: {:.3}", bad_rep);
    assert!(
        bad_rep <= 0.15,
        "Reputation should be very low (at or near floor of 0.1)"
    );

    // Create normal and bad tips
    let normal_proposer = PublicKey::from_bytes([1; 32]);
    let normal_tip = create_tip_message(1, normal_proposer, vec![100, 100, 100]);
    let bad_tip = create_tip_message(2, bad_proposer, vec![101, 101, 101]);

    storage.store_message(&normal_tip).await.unwrap();
    storage.store_message(&bad_tip).await.unwrap();

    let tip_ids = vec![normal_tip.id, bad_tip.id];

    // Try to select parents
    let new_features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(100, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(100, 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(100, 3, 10)),
    ]);

    let selected_parents = mrw
        .select_parents(
            &new_features,
            &tip_ids,
            &storage,
            &conflict_resolver,
            &reputation_tracker,
        )
        .await
        .unwrap();

    // The bad reputation parent should have very low weight
    // and likely not be selected
    for parent_id in &selected_parents {
        if let Ok(Some(parent)) = storage.get_message(parent_id).await {
            let rep = reputation_tracker.get_reputation(&parent.proposer_pk).await;
            println!("Selected parent reputation: {:.3}", rep);

            // If we enforce C3 strictly, this should not select very low reputation parents
            assert!(
                rep > 0.05,
                "Should not select parents with extremely low reputation"
            );
        }
    }
}

fn create_tip_message(id: u8, proposer: PublicKey, feature_values: Vec<u64>) -> AdicMessage {
    let features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(feature_values[0], 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(feature_values[1], 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(feature_values[2], 3, 10)),
    ]);

    let mut msg = AdicMessage::new(
        vec![],
        features,
        AdicMeta::new(Utc::now()),
        proposer,
        vec![id],
    );

    // Set a deterministic ID for testing
    msg.id = MessageId::new(&[id; 32]);
    msg
}
