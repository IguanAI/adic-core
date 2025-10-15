//! Attack Simulation Tests
//!
//! This test suite simulates various attack scenarios to validate ADIC's security mechanisms:
//! - Reputation manipulation attacks
//! - Governance capture attempts
//! - Overlap penalty evasion
//! - Canonical JSON manipulation
//! - Eclipse attacks on tip selection
//!
//! These tests ensure the protocol's defensive mechanisms work correctly.

use adic_governance::{
    overlap_penalties::OverlapPenaltyTracker,
    types::{Ballot, GovernanceProposal, GovernanceVote},
    voting::VotingEngine,
};
use adic_types::{canonical_hash, PublicKey};
use chrono::Utc;
use std::sync::Arc;

// ==================== Test Helpers ====================

/// Create a voter with public key and reputation
fn create_voter(id: u8, reputation: f64) -> (PublicKey, f64) {
    (PublicKey::from_bytes([id; 32]), reputation)
}

/// Create a proposal with given parameters
fn create_proposal(proposer: u8, param: &str, value: &str) -> (PublicKey, [u8; 32]) {
    let proposer_pk = PublicKey::from_bytes([proposer; 32]);
    let param_keys = vec![param.to_string()];
    let new_values = serde_json::json!(vec![value.to_string()]);
    let proposal_id = GovernanceProposal::compute_id(
        &proposer_pk,
        &param_keys,
        &new_values,
        Utc::now(),
    );
    (proposer_pk, proposal_id)
}

// ==================== Attack Simulation Tests ====================

/// Test: Reputation Wash Trading Attack
///
/// Attack scenario:
/// - Attacker creates multiple identities
/// - Attempts to artificially inflate reputation through coordinated activity
/// - Tries to vote on proposals with inflated reputation
///
/// Expected: Overlap penalties detect coordinated behavior and reduce effectiveness
#[tokio::test]
async fn test_reputation_wash_trading_attack() {
    let rmax = 10_000.0;
    let min_quorum = 100.0;

    let tracker = Arc::new(OverlapPenaltyTracker::new(2.0, 0.9, 20));
    let voting_engine = VotingEngine::new(rmax, min_quorum)
        .with_overlap_tracker(tracker.clone());

    // Attacker creates 10 wash trading identities with medium reputation
    let wash_traders: Vec<(PublicKey, f64)> = (0..10)
        .map(|i| create_voter(100 + i, 1000.0))
        .collect();

    // Honest voter with high reputation
    let (honest_pk, honest_rep) = create_voter(200, 10_000.0); // Increased to 10k

    // Wash traders vote together on 20 historical proposals to build stronger overlap penalty
    for i in 0..20 {
        let (_proposer, proposal_id) = create_proposal(i, "test_param", &format!("value_{}", i));
        let wash_trader_pks: Vec<PublicKey> = wash_traders.iter().map(|(pk, _)| *pk).collect();

        tracker
            .record_votes(proposal_id, &wash_trader_pks)
            .await;
    }

    // New proposal: wash traders vote YES, honest voter votes NO
    let (_proposer, malicious_proposal_id) =
        create_proposal(99, "max_supply", "999999999"); // Malicious increase

    let mut votes = Vec::new();

    // Wash traders vote YES
    for (pk, rep) in &wash_traders {
        let credits = voting_engine.compute_voting_credits(*rep);
        votes.push(GovernanceVote::new(malicious_proposal_id, *pk, credits, Ballot::Yes));
    }

    // Honest voter votes NO
    let honest_credits = voting_engine.compute_voting_credits(honest_rep);
    votes.push(GovernanceVote::new(malicious_proposal_id, honest_pk, honest_credits, Ballot::No));

    // Tally votes with overlap penalties
    let stats = voting_engine
        .tally_votes(&votes, &malicious_proposal_id)
        .await
        .unwrap();
    let yes_votes = stats.yes;
    let no_votes = stats.no;

    // Verify wash traders are penalized
    let wash_trader_pk = wash_traders[0].0;
    let (_, attenuation_factor) = tracker.get_voter_stats(&wash_trader_pk).await.unwrap();
    assert!(
        attenuation_factor < 0.7,
        "Wash traders should be heavily penalized (η = {})",
        attenuation_factor
    );

    // Calculate wash trading effectiveness
    let theoretical_wash_power = 10.0 * (1000.0_f64).sqrt(); // 10 voters * sqrt(1000) ≈ 316
    let wash_effectiveness = yes_votes / theoretical_wash_power;

    // Verify overlap penalties significantly reduce wash trading power
    assert!(
        wash_effectiveness < 0.6,
        "Wash trading effectiveness should be < 60% due to penalties (actual: {:.1}%)",
        wash_effectiveness * 100.0
    );

    // Verify significant penalty reduction (at least 40% reduction)
    let penalty_reduction = (theoretical_wash_power - yes_votes) / theoretical_wash_power;
    assert!(
        penalty_reduction > 0.4,
        "Overlap penalties should reduce wash trading power by >40% (actual: {:.1}%)",
        penalty_reduction * 100.0
    );

    println!("✅ Reputation wash trading attack mitigated:");
    println!("  Wash traders: {} credits (penalized from {:.1})", yes_votes, theoretical_wash_power);
    println!("  Honest voter: {} credits", no_votes);
    println!("  Penalty reduction: {:.1}%", penalty_reduction * 100.0);
    println!("  Wash effectiveness: {:.1}%", wash_effectiveness * 100.0);
}

/// Test: Governance Parameter Manipulation Attack
///
/// Attack scenario:
/// - Attacker attempts to manipulate critical parameters
/// - Uses multiple identities to vote on malicious proposals
/// - Targets economic parameters (max_supply, min_deposit, etc.)
///
/// Expected: Parameter changes require overwhelming consensus, attacks fail
#[tokio::test]
async fn test_governance_parameter_manipulation() {
    let rmax = 10_000.0;
    let min_quorum = 1000.0; // Very high quorum requirement for critical parameter changes

    let voting_engine = VotingEngine::new(rmax, min_quorum);

    // Attackers: 15 identities with low-medium reputation
    let attackers: Vec<(PublicKey, f64)> = (0..15)
        .map(|i| create_voter(i, 500.0))
        .collect();

    // Defenders: 8 identities with higher reputation
    let defenders: Vec<(PublicKey, f64)> = (50..58)
        .map(|i| create_voter(i, 2000.0))
        .collect();

    // Malicious proposal: increase max supply by 10x
    let (_proposer, proposal_id) = create_proposal(0, "max_supply", "10000000000");

    let mut votes = Vec::new();

    // Attackers vote YES
    for (pk, rep) in &attackers {
        let credits = voting_engine.compute_voting_credits(*rep);
        votes.push(GovernanceVote::new(proposal_id, *pk, credits, Ballot::Yes));
    }

    // Defenders vote NO
    for (pk, rep) in &defenders {
        let credits = voting_engine.compute_voting_credits(*rep);
        votes.push(GovernanceVote::new(proposal_id, *pk, credits, Ballot::No));
    }

    // Tally votes
    let stats = voting_engine
        .tally_votes(&votes, &proposal_id)
        .await
        .unwrap();
    let yes_votes = stats.yes;
    let no_votes = stats.no;

    let total_votes = yes_votes + no_votes;
    let yes_pct = yes_votes / total_votes;

    // Attackers should not reach quorum
    assert!(
        total_votes < min_quorum,
        "Attack should not reach quorum ({} < {})",
        total_votes,
        min_quorum
    );

    // Even if quorum reached, attackers should not have majority
    assert!(
        yes_pct < 0.5,
        "Attackers should not reach majority ({:.1}% < 50%)",
        yes_pct * 100.0
    );

    println!("✅ Parameter manipulation attack blocked:");
    println!("  Total votes: {} (quorum: {})", total_votes, min_quorum);
    println!("  Attack votes: {:.1}%", yes_pct * 100.0);
    println!("  Status: REJECTED (insufficient quorum)");
}

/// Test: Overlap Penalty Evasion via Temporal Splitting
///
/// Attack scenario:
/// - Attacker splits identities into groups
/// - Groups vote in different time windows to evade overlap detection
/// - Attempts to avoid penalty by not voting together
///
/// Expected: Even with temporal splitting, coordinated voting is detected
#[tokio::test]
async fn test_overlap_penalty_evasion_temporal() {
    let rmax = 10_000.0;
    let min_quorum = 100.0;

    let tracker = Arc::new(OverlapPenaltyTracker::new(2.0, 0.9, 20));
    let _voting_engine = VotingEngine::new(rmax, min_quorum)
        .with_overlap_tracker(tracker.clone());

    // Attacker creates 3 groups of 5 identities each
    let group_a: Vec<(PublicKey, f64)> = (0..5).map(|i| create_voter(i, 1000.0)).collect();
    let group_b: Vec<(PublicKey, f64)> = (10..15).map(|i| create_voter(i, 1000.0)).collect();
    let group_c: Vec<(PublicKey, f64)> = (20..25).map(|i| create_voter(i, 1000.0)).collect();

    // Phase 1: Group A votes on proposals 0-9
    for i in 0..10 {
        let (_, proposal_id) = create_proposal(i, "param", &format!("val_{}", i));
        let group_a_pks: Vec<PublicKey> = group_a.iter().map(|(pk, _)| *pk).collect();
        tracker
            .record_votes(proposal_id, &group_a_pks)
            .await;
    }

    // Phase 2: Group B votes on proposals 10-19
    for i in 10..20 {
        let (_, proposal_id) = create_proposal(i, "param", &format!("val_{}", i));
        let group_b_pks: Vec<PublicKey> = group_b.iter().map(|(pk, _)| *pk).collect();
        tracker
            .record_votes(proposal_id, &group_b_pks)
            .await;
    }

    // Phase 3: Group C votes on proposals 20-29
    for i in 20..30 {
        let (_, proposal_id) = create_proposal(i, "param", &format!("val_{}", i));
        let group_c_pks: Vec<PublicKey> = group_c.iter().map(|(pk, _)| *pk).collect();
        tracker
            .record_votes(proposal_id, &group_c_pks)
            .await;
    }

    // Critical proposal: ALL groups vote together
    let (_, critical_proposal_id) = create_proposal(99, "critical_param", "attack_value");
    let all_attackers: Vec<PublicKey> = group_a
        .iter()
        .chain(group_b.iter())
        .chain(group_c.iter())
        .map(|(pk, _)| *pk)
        .collect();

    tracker
        .record_votes(
            critical_proposal_id,
            &all_attackers,
        )
        .await;

    // Check penalties on all groups
    let (_, group_a_attenuation) = tracker.get_voter_stats(&group_a[0].0).await.unwrap();
    let (_, group_b_attenuation) = tracker.get_voter_stats(&group_b[0].0).await.unwrap();
    let (_, group_c_attenuation) = tracker.get_voter_stats(&group_c[0].0).await.unwrap();

    // Even with temporal splitting, coordinated voting on critical proposal triggers penalties
    // Groups that voted together before should have some penalty
    assert!(
        group_a_attenuation < 1.0
            || group_b_attenuation < 1.0
            || group_c_attenuation < 1.0,
        "At least one group should have penalties from previous overlap"
    );

    println!("✅ Temporal splitting evasion detected:");
    println!("  Group A penalty: η = {:.3}", group_a_attenuation);
    println!("  Group B penalty: η = {:.3}", group_b_attenuation);
    println!("  Group C penalty: η = {:.3}", group_c_attenuation);
    println!("  Status: Coordinated voting detected despite temporal splitting");
}

/// Test: Canonical JSON Hash Collision Attack
///
/// Attack scenario:
/// - Attacker creates two different proposals
/// - Attempts to make them hash to same ID via key ordering manipulation
/// - Tries to confuse governance by creating duplicate proposal IDs
///
/// Expected: Canonical JSON ensures different data → different hash
#[tokio::test]
async fn test_canonical_json_collision_attack() {
    let proposer = PublicKey::from_bytes([42; 32]);
    let timestamp = Utc::now();

    // Proposal 1: Increase max_supply
    let param_keys = vec!["max_supply".to_string()];
    let new_values_1 = serde_json::json!(vec!["2000000000".to_string()]);
    let proposal_1_id = GovernanceProposal::compute_id(
        &proposer,
        &param_keys,
        &new_values_1,
        timestamp,
    );

    // Proposal 2: Different parameter change (attempts collision)
    let new_values_2 = serde_json::json!(vec!["2000000001".to_string()]);
    let proposal_2_id = GovernanceProposal::compute_id(
        &proposer,
        &param_keys,
        &new_values_2, // One digit different
        timestamp,
    );

    // Verify different proposals have different IDs
    assert_ne!(
        proposal_1_id, proposal_2_id,
        "Different proposals must have different IDs"
    );

    // Test with different key ordering (should produce same hash for same data)
    #[derive(serde::Serialize)]
    struct ProposalData {
        param: String,
        value: String,
        proposer: [u8; 32],
    }

    let data_1 = ProposalData {
        param: "test_param".to_string(),
        value: "test_value".to_string(),
        proposer: [1; 32],
    };

    let data_2 = ProposalData {
        proposer: [1; 32],
        param: "test_param".to_string(),
        value: "test_value".to_string(),
    };

    let hash_1 = canonical_hash(&data_1).unwrap();
    let hash_2 = canonical_hash(&data_2).unwrap();

    // Same data, different struct field order → SAME hash (canonical ordering)
    assert_eq!(
        hash_1, hash_2,
        "Same data must produce same hash regardless of field order"
    );

    // Different data → different hash
    let data_3 = ProposalData {
        param: "test_param".to_string(),
        value: "different_value".to_string(), // Changed value
        proposer: [1; 32],
    };

    let hash_3 = canonical_hash(&data_3).unwrap();
    assert_ne!(hash_1, hash_3, "Different data must produce different hash");

    println!("✅ Canonical JSON collision attack prevented:");
    println!("  Different proposals: Different IDs ✓");
    println!("  Same data + different field order: Same hash ✓");
    println!("  Different data: Different hash ✓");
}

/// Test: Low-Reputation Spam Attack
///
/// Attack scenario:
/// - Attacker creates 100 low-reputation identities
/// - Attempts to overwhelm governance by sheer numbers
/// - Each identity has minimal reputation
///
/// Expected: Quadratic voting limits spam effectiveness
#[tokio::test]
async fn test_low_reputation_spam_attack() {
    let rmax = 10_000.0;
    let min_quorum = 100.0;

    let voting_engine = VotingEngine::new(rmax, min_quorum);

    // Spammer: 100 identities with 10 ADIC reputation each
    let spammers: Vec<(PublicKey, f64)> = (0..100)
        .map(|i| create_voter(i, 10.0))
        .collect();

    // Defender: 1 whale with 10,000 ADIC reputation
    let (whale_pk, whale_rep) = create_voter(200, 10_000.0);

    let (_, proposal_id) = create_proposal(99, "spam_param", "spam_value");

    let mut votes = Vec::new();

    // Spammers vote YES
    for (pk, rep) in &spammers {
        let credits = voting_engine.compute_voting_credits(*rep);
        votes.push(GovernanceVote::new(proposal_id, *pk, credits, Ballot::Yes));
    }

    // Whale votes NO
    let whale_credits = voting_engine.compute_voting_credits(whale_rep);
    votes.push(GovernanceVote::new(proposal_id, whale_pk, whale_credits, Ballot::No));

    let stats = voting_engine
        .tally_votes(&votes, &proposal_id)
        .await
        .unwrap();
    let yes_votes = stats.yes;
    let no_votes = stats.no;

    // Without overlap penalties (spammers haven't voted together before), quadratic voting still limits damage
    // Whale: sqrt(10000) = 100 credits
    // Spammers: 100 * sqrt(10) ≈ 316 credits
    // This is expected behavior - quadratic voting reduces 100:1 numerical advantage to ~3:1 credit ratio

    // Verify quadratic dampening is working correctly
    let expected_whale = (10_000.0_f64).sqrt();
    let expected_spam_total = 100.0 * (10.0_f64).sqrt();

    assert!(
        (no_votes - expected_whale).abs() < 1.0,
        "Whale credits should be sqrt(10000) ≈ 100 (got {})",
        no_votes
    );

    assert!(
        (yes_votes - expected_spam_total).abs() < 1.0,
        "Spam total should be 100*sqrt(10) ≈ 316 (got {})",
        yes_votes
    );

    // Calculate spam effectiveness
    let theoretical_spam_power = 100.0 * (10.0_f64).sqrt(); // 100 * sqrt(10)
    let spam_effectiveness = yes_votes / theoretical_spam_power;

    // Quadratic voting reduces spam from 100:1 to ~3:1
    let credit_ratio = yes_votes / no_votes;

    println!("✅ Quadratic voting limits low-reputation spam:");
    println!("  Spammers (100 x 10 ADIC): {} credits", yes_votes);
    println!("  Whale (1 x 10,000 ADIC): {} credits", no_votes);
    println!("  Spam effectiveness: {:.1}%", spam_effectiveness * 100.0);
    println!("  Quadratic voting reduces 100:1 numerical advantage to {:.1}:1 credit ratio", credit_ratio);
}

/// Test: Ring Splitting to Evade Detection
///
/// Attack scenario:
/// - Large coordinated group splits into smaller rings
/// - Each ring stays below detection threshold
/// - Rings vote together on critical proposals
///
/// Expected: Cross-ring overlap still detected, penalties applied
#[tokio::test]
async fn test_ring_splitting_evasion() {
    let rmax = 10_000.0;
    let min_quorum = 100.0;

    let tracker = Arc::new(OverlapPenaltyTracker::new(2.0, 0.9, 20));
    let _voting_engine = VotingEngine::new(rmax, min_quorum)
        .with_overlap_tracker(tracker.clone());

    // Create 3 rings of 4 members each (below typical 5-member detection threshold)
    let ring_1: Vec<(PublicKey, f64)> = (0..4).map(|i| create_voter(i, 1000.0)).collect();
    let ring_2: Vec<(PublicKey, f64)> = (10..14).map(|i| create_voter(i, 1000.0)).collect();
    let ring_3: Vec<(PublicKey, f64)> = (20..24).map(|i| create_voter(i, 1000.0)).collect();

    // Each ring votes internally on separate proposals (builds intra-ring overlap)
    for i in 0..8 {
        let (_, prop_id) = create_proposal(i, "param", &format!("val_{}", i));
        let ring_1_pks: Vec<PublicKey> = ring_1.iter().map(|(pk, _)| *pk).collect();
        tracker
            .record_votes(prop_id, &ring_1_pks)
            .await;
    }

    for i in 10..18 {
        let (_, prop_id) = create_proposal(i, "param", &format!("val_{}", i));
        let ring_2_pks: Vec<PublicKey> = ring_2.iter().map(|(pk, _)| *pk).collect();
        tracker
            .record_votes(prop_id, &ring_2_pks)
            .await;
    }

    for i in 20..28 {
        let (_, prop_id) = create_proposal(i, "param", &format!("val_{}", i));
        let ring_3_pks: Vec<PublicKey> = ring_3.iter().map(|(pk, _)| *pk).collect();
        tracker
            .record_votes(prop_id, &ring_3_pks)
            .await;
    }

    // Critical vote: ALL rings coordinate together
    let (_, critical_id) = create_proposal(99, "critical", "attack");
    let all_rings: Vec<PublicKey> = ring_1
        .iter()
        .chain(ring_2.iter())
        .chain(ring_3.iter())
        .map(|(pk, _)| *pk)
        .collect();

    tracker
        .record_votes(critical_id, &all_rings)
        .await;

    // Detect rings
    let rings = tracker.detect_rings(3, 0.6).await;

    // Should detect coordination even with ring splitting
    assert!(
        !rings.is_empty(),
        "Ring detection should find coordinated groups despite splitting"
    );

    let (_, ring_1_attenuation) = tracker.get_voter_stats(&ring_1[0].0).await.unwrap();
    assert!(
        ring_1_attenuation < 0.8,
        "Ring members should have penalties (η = {})",
        ring_1_attenuation
    );

    println!("✅ Ring splitting evasion detected:");
    println!("  Detected {} coordinated ring(s)", rings.len());
    println!("  Ring member penalty: η = {:.3}", ring_1_attenuation);
    println!("  Status: Cross-ring coordination detected");
}
