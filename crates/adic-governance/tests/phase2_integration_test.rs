//! Phase 2 Integration Tests
//!
//! Comprehensive tests verifying all Phase 2 features work together:
//! - Overlap penalties (PoUW III §4.2)
//! - Canonical JSON serialization
//! - Deterministic message hashing
//! - End-to-end governance flows

use adic_governance::{
    GovernanceProposal, GovernanceVote, OverlapPenaltyTracker, ProposalClass, VotingEngine,
};
use adic_types::{canonical_hash, to_canonical_json, verify_canonical_match, PublicKey};
use chrono::Utc;
use std::sync::Arc;

#[tokio::test]
async fn test_canonical_json_deterministic_proposal_ids() {
    // Test that proposal IDs are deterministic across multiple computations
    let proposer = PublicKey::from_bytes([1; 32]);
    let param_keys = vec!["param1".to_string(), "param2".to_string()];
    let new_values = serde_json::json!({"param1": 100, "param2": 200});
    let timestamp = Utc::now();

    // Compute ID multiple times
    let id1 = GovernanceProposal::compute_id(&proposer, &param_keys, &new_values, timestamp);
    let id2 = GovernanceProposal::compute_id(&proposer, &param_keys, &new_values, timestamp);
    let id3 = GovernanceProposal::compute_id(&proposer, &param_keys, &new_values, timestamp);

    // All IDs should be identical
    assert_eq!(id1, id2);
    assert_eq!(id2, id3);

    // Different inputs should produce different IDs
    let different_values = serde_json::json!({"param1": 999, "param2": 200});
    let id4 = GovernanceProposal::compute_id(&proposer, &param_keys, &different_values, timestamp);

    assert_ne!(id1, id4);
}

#[tokio::test]
async fn test_canonical_json_key_ordering_invariance() {
    // Test that key ordering doesn't affect hash
    use serde::Serialize;

    #[derive(Serialize)]
    struct Message1 {
        a: u64,
        b: String,
        c: Vec<u8>,
    }

    #[derive(Serialize)]
    struct Message2 {
        c: Vec<u8>,
        b: String,
        a: u64,
    }

    let msg1 = Message1 {
        a: 42,
        b: "test".to_string(),
        c: vec![1, 2, 3],
    };

    let msg2 = Message2 {
        c: vec![1, 2, 3],
        b: "test".to_string(),
        a: 42,
    };

    // Different struct field orders should produce same canonical hash
    assert!(verify_canonical_match(&msg1, &msg2).unwrap());
}

#[tokio::test]
async fn test_overlap_penalties_in_governance_lifecycle() {
    // Test overlap penalties integrate properly with full governance lifecycle
    let overlap_tracker = Arc::new(OverlapPenaltyTracker::new(2.0, 0.9, 20));
    let voting_engine = VotingEngine::new(100_000.0, 0.1)
        .with_overlap_tracker(overlap_tracker.clone());

    let proposal_id = [1u8; 32];

    // Create voting ring: 3 voters who vote together repeatedly
    let ring_voters = vec![
        PublicKey::from_bytes([10; 32]),
        PublicKey::from_bytes([11; 32]),
        PublicKey::from_bytes([12; 32]),
    ];

    // Independent voter
    let independent = PublicKey::from_bytes([99; 32]);

    // Simulate 10 previous proposals where ring voters voted together
    for i in 0..10 {
        overlap_tracker
            .record_votes([i as u8; 32], &ring_voters)
            .await;
    }

    // Now vote on current proposal
    let votes = vec![
        GovernanceVote::new(proposal_id, ring_voters[0], 100.0, adic_governance::Ballot::Yes),
        GovernanceVote::new(proposal_id, ring_voters[1], 100.0, adic_governance::Ballot::Yes),
        GovernanceVote::new(proposal_id, ring_voters[2], 100.0, adic_governance::Ballot::Yes),
        GovernanceVote::new(proposal_id, independent, 100.0, adic_governance::Ballot::No),
    ];

    let stats = voting_engine
        .tally_votes(&votes, &proposal_id)
        .await
        .unwrap();

    // Ring voters should have penalties applied
    // Original: 3 × 100 = 300 yes, 1 × 100 = 100 no
    // With penalties: yes should be < 300 due to overlap
    assert!(stats.yes < 300.0, "Ring voters should have penalties");
    assert_eq!(stats.no, 100.0, "Independent voter should not be penalized");

    // Total participation should reflect penalties
    assert!(stats.total_participation < 400.0);
    assert!(stats.total_participation > 100.0);
}

#[tokio::test]
async fn test_ring_detection_integration() {
    // Test that ring detection works with real voting patterns
    let tracker = Arc::new(OverlapPenaltyTracker::new(2.0, 0.9, 20));

    let ring_members = vec![
        PublicKey::from_bytes([1; 32]),
        PublicKey::from_bytes([2; 32]),
        PublicKey::from_bytes([3; 32]),
        PublicKey::from_bytes([4; 32]),
    ];

    let independent = PublicKey::from_bytes([99; 32]);

    // Ring votes together on 15 proposals
    for i in 0..15 {
        tracker
            .record_votes([i as u8; 32], &ring_members)
            .await;
    }

    // Independent votes alone on different proposals
    for i in 15..30 {
        tracker
            .record_votes([i as u8; 32], &[independent])
            .await;
    }

    // Detect rings with lower overlap threshold
    let rings = tracker.detect_rings(2, 0.5).await;

    // Should detect at least 1 ring with ring_members
    assert!(!rings.is_empty(), "Should detect at least one ring");

    // Find the ring with our members
    let mut found_ring = false;
    for ring in &rings {
        if ring.len() >= 3 && ring_members.iter().filter(|m| ring.contains(m)).count() >= 3 {
            found_ring = true;

            // Independent should not be in this ring
            assert!(
                !ring.contains(&independent),
                "Independent voter should not be in ring"
            );
        }
    }

    assert!(found_ring, "Should find a ring containing the coordinated voters");
}

#[tokio::test]
async fn test_canonical_json_with_nested_governance_structures() {
    // Test canonical JSON handles complex nested governance structures
    use adic_governance::{
        AxisAction, AxisCatalogChange, MigrationPlan, ProposalStatus, TreasuryGrant,
    };
    use adic_economics::types::AdicAmount;

    let proposer = PublicKey::from_bytes([5; 32]);
    let recipient = PublicKey::from_bytes([6; 32]);

    let axis_change = AxisCatalogChange {
        action: AxisAction::Add,
        axis_id: 4,
        encoder_spec_cid: "QmTest123".to_string(),
        ultrametric_proof_cid: "QmProof456".to_string(),
        security_analysis_cid: "QmSecurity789".to_string(),
        migration_plan: MigrationPlan {
            ramp_epochs: 100,
            weight_schedule: vec![(0, 0.0), (50, 0.5), (100, 1.0)],
        },
    };

    let treasury_grant = TreasuryGrant {
        recipient,
        total_amount: AdicAmount::from_adic(1000.0),
        schedule: adic_governance::GrantSchedule::Streamed {
            rate_per_epoch: AdicAmount::from_adic(10.0),
            duration_epochs: 100,
        },
        proposal_id: [0u8; 32],
    };

    let proposal = GovernanceProposal {
        proposal_id: [1u8; 32],
        class: ProposalClass::Constitutional,
        proposer_pk: proposer,
        param_keys: vec!["key1".to_string(), "key2".to_string()],
        new_values: serde_json::json!({"key1": 100, "key2": 200}),
        axis_changes: Some(axis_change),
        treasury_grant: Some(treasury_grant),
        enact_epoch: 1000,
        rationale_cid: "QmRationale".to_string(),
        creation_timestamp: Utc::now(),
        voting_end_timestamp: Utc::now(),
        status: ProposalStatus::Voting,
        tally_yes: 0.0,
        tally_no: 0.0,
        tally_abstain: 0.0,
    };

    // Should serialize successfully
    let json = to_canonical_json(&proposal).unwrap();

    // Should be valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed.is_object());

    // Should hash consistently
    let hash1 = canonical_hash(&proposal).unwrap();
    let hash2 = canonical_hash(&proposal).unwrap();
    assert_eq!(hash1, hash2);
}

#[tokio::test]
async fn test_end_to_end_governance_flow_with_phase2_features() {
    // Complete governance flow using all Phase 2 features
    let overlap_tracker = Arc::new(OverlapPenaltyTracker::new(2.0, 0.9, 20));
    let voting_engine = VotingEngine::new(100_000.0, 0.1).with_overlap_tracker(overlap_tracker.clone());

    // Step 1: Create proposal with canonical ID
    let proposer = PublicKey::from_bytes([1; 32]);
    let param_keys = vec!["parameter_alpha".to_string()];
    let new_values = serde_json::json!({"parameter_alpha": 42});
    let timestamp = Utc::now();

    let proposal_id = GovernanceProposal::compute_id(&proposer, &param_keys, &new_values, timestamp);

    // Verify ID is deterministic
    let proposal_id_2 = GovernanceProposal::compute_id(&proposer, &param_keys, &new_values, timestamp);
    assert_eq!(proposal_id, proposal_id_2);

    // Step 2: Create voting ring history
    let ring_voters = vec![
        PublicKey::from_bytes([10; 32]),
        PublicKey::from_bytes([11; 32]),
        PublicKey::from_bytes([12; 32]),
    ];

    for i in 0..5 {
        overlap_tracker
            .record_votes([i as u8; 32], &ring_voters)
            .await;
    }

    // Step 3: Cast votes
    let votes = vec![
        GovernanceVote::new(proposal_id, ring_voters[0], 100.0, adic_governance::Ballot::Yes),
        GovernanceVote::new(proposal_id, ring_voters[1], 100.0, adic_governance::Ballot::Yes),
        GovernanceVote::new(proposal_id, ring_voters[2], 50.0, adic_governance::Ballot::No),
        GovernanceVote::new(
            proposal_id,
            PublicKey::from_bytes([99; 32]),
            75.0,
            adic_governance::Ballot::Yes,
        ),
    ];

    // Step 4: Tally with overlap penalties
    let stats = voting_engine
        .tally_votes(&votes, &proposal_id)
        .await
        .unwrap();

    // Verify penalties were applied to ring
    // Original yes: 200, no: 50
    // With penalties: yes should be < 200
    assert!(stats.yes < 200.0, "Ring yes votes should be penalized");
    assert!(stats.yes > 75.0, "But not reduced to just independent voter");

    // Step 5: Check quorum (10% of 100,000 max = 10,000 required credits)
    // We have ~275 credits, so this should fail
    let total_eligible = voting_engine.compute_voting_credits(100_000.0) * 100.0; // Assume 100 voters
    let quorum_check = voting_engine.check_quorum(&stats, total_eligible);
    assert!(quorum_check.is_err(), "Should fail quorum with small participation");

    // Step 6: Verify threshold logic
    let result = voting_engine.determine_result(&stats, 0.5).unwrap();
    let yes_pct = stats.yes / (stats.yes + stats.no);

    if yes_pct > 0.5 {
        assert_eq!(result, adic_governance::VoteResult::Pass);
    } else {
        assert_eq!(result, adic_governance::VoteResult::Fail);
    }
}

#[tokio::test]
async fn test_canonical_json_null_handling() {
    // Test that null values are properly omitted in canonical form
    let proposal_id = [1u8; 32];
    let voter = PublicKey::from_bytes([1; 32]);

    let vote = GovernanceVote::new(proposal_id, voter, 100.0, adic_governance::Ballot::Yes);

    let json = to_canonical_json(&vote).unwrap();

    // Should not contain explicit null fields
    assert!(!json.contains("null"), "Canonical JSON should not contain 'null'");

    // Should be valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed.is_object());
}

#[tokio::test]
async fn test_overlap_penalty_decay_over_time() {
    // Test that overlap penalties decay over proposal window
    let tracker = Arc::new(OverlapPenaltyTracker::new(2.0, 0.8, 10)); // decay=0.8, window=10

    let voters = vec![
        PublicKey::from_bytes([1; 32]),
        PublicKey::from_bytes([2; 32]),
    ];

    // Record 10 proposals with co-voting (fills window)
    for i in 0..10 {
        tracker.record_votes([i as u8; 32], &voters).await;
    }

    // Apply penalty after full window
    let penalty1 = tracker.apply_penalty(&voters[0], 100.0).await;

    // Record 5 more proposals (old ones decay out of window)
    for i in 10..15 {
        tracker.record_votes([i as u8; 32], &voters).await;
    }

    // Penalty should be similar (within window still)
    let _penalty2 = tracker.apply_penalty(&voters[0], 100.0).await;

    // Record 10 more proposals where voters DON'T co-vote
    for i in 15..25 {
        tracker.record_votes([i as u8; 32], &[voters[0]]).await;
    }

    // Now penalty should be much lighter (old co-votes decayed)
    let penalty3 = tracker.apply_penalty(&voters[0], 100.0).await;

    assert!(penalty3 > penalty1, "Penalty should decrease as co-votes age out");
}

#[test]
fn test_canonical_json_number_format_consistency() {
    // Test that numbers are consistently formatted
    use serde::Serialize;

    #[derive(Serialize)]
    struct Numbers {
        int: i64,
        float: f64,
        zero: i32,
    }

    let nums = Numbers {
        int: 42,
        float: 3.14159,
        zero: 0,
    };

    let json = to_canonical_json(&nums).unwrap();

    // Integers should not have decimal points
    assert!(json.contains(r#""int":42"#));
    assert!(json.contains(r#""zero":0"#));

    // Floats should maintain precision
    assert!(json.contains(r#""float":3.14159"#));
}

#[tokio::test]
async fn test_voting_credits_validation_with_canonical_hash() {
    // Test that voting credits are properly validated
    let engine = VotingEngine::new(100_000.0, 0.1);

    // Test valid credits
    let reputation: f64 = 10_000.0;
    let expected_credits = reputation.sqrt(); // 100.0
    assert!(engine.validate_credits(expected_credits, reputation).is_ok());

    // Test invalid credits (should fail)
    let invalid_credits = 50.0; // Wrong value
    assert!(engine.validate_credits(invalid_credits, reputation).is_err());
}
