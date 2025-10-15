/// Advanced example: Dispute Resolution in ADIC PoUW
///
/// This example demonstrates:
/// 1. Challenge submission
/// 2. Evidence verification
/// 3. Arbitration committee selection
/// 4. Arbitration decision processing
/// 5. Slashing and reputation penalties
/// 6. Challenger rewards
///
/// Run with: cargo run --example dispute_resolution

use adic_pouw::*;
use adic_app_common::escrow::EscrowManager;
use adic_consensus::ReputationTracker;
use adic_crypto::Keypair;
use adic_economics::storage::MemoryStorage;
use adic_economics::types::{AccountAddress, AdicAmount};
use adic_economics::BalanceManager;
use adic_types::PublicKey;
use adic_vrf::VRFService;
use chrono::Utc;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    println!("⚖️ ADIC PoUW Framework - Dispute Resolution Example\n");

    // ============================================================================
    // Setup
    // ============================================================================
    println!("📦 Initializing components...");

    let storage = Arc::new(MemoryStorage::new());
    let balance_mgr = Arc::new(BalanceManager::new(storage));
    let escrow_mgr = Arc::new(EscrowManager::new(balance_mgr.clone()));
    let rep_tracker = Arc::new(ReputationTracker::new(0.9));
    let vrf_service = Arc::new(VRFService::new(Default::default(), rep_tracker.clone()));

    let dispute_mgr = DisputeManager::new(
        rep_tracker.clone(),
        DisputeConfig {
            min_challenger_stake: AdicAmount::from_adic(10.0),
            challenge_period_epochs: 24,
            arbitration_committee_size: 7,
            arbitration_threshold: 0.67,
            challenger_reward_fraction: 0.3,
            ..Default::default()
        },
    );

    let reputation_mgr = PoUWReputationManager::new(
        rep_tracker.clone(),
        ReputationConfig::default(),
    );

    let reward_mgr = RewardManager::new(
        escrow_mgr.clone(),
        RewardConfig::default(),
    );

    let executor = TaskExecutor::new(
        Keypair::generate(),
        ExecutorConfig::default(),
    );

    println!("   ✅ Components ready\n");

    // ============================================================================
    // Setup Accounts
    // ============================================================================
    println!("👤 Setting up accounts...");

    let sponsor = PublicKey::from_bytes([1u8; 32]);
    let worker = PublicKey::from_bytes([10u8; 32]);
    let challenger = PublicKey::from_bytes([20u8; 32]);

    balance_mgr.credit(AccountAddress::from_public_key(&sponsor), AdicAmount::from_adic(5000.0))
        .await.unwrap();
    balance_mgr.credit(AccountAddress::from_public_key(&worker), AdicAmount::from_adic(500.0))
        .await.unwrap();
    balance_mgr.credit(AccountAddress::from_public_key(&challenger), AdicAmount::from_adic(200.0))
        .await.unwrap();

    rep_tracker.set_reputation(&sponsor, 500.0).await;
    rep_tracker.set_reputation(&worker, 1000.0).await;
    rep_tracker.set_reputation(&challenger, 800.0).await;

    println!("   ✅ Accounts funded and reputations set\n");

    // ============================================================================
    // SCENARIO 1: Task with Incorrect Result
    // ============================================================================
    println!("📋 SCENARIO 1: Detecting Incorrect Result\n");

    let task = Task {
        task_id: [1u8; 32],
        sponsor,
        task_type: TaskType::Compute {
            computation_type: ComputationType::DataProcessing,
            resource_requirements: ResourceRequirements::default(),
        },
        input_cid: "QmTaskInput123".to_string(),
        expected_output_schema: Some("processed_data".to_string()),
        reward: AdicAmount::from_adic(100.0),
        collateral_requirement: AdicAmount::from_adic(50.0),
        deadline_epoch: 100,
        min_reputation: 500.0,
        worker_count: 1,
        created_at: Utc::now(),
        status: TaskStatus::Assigned,
        finality_status: FinalityStatus::F2Complete,
    };

    println!("⚙️ Worker executing task...");
    let work_result = executor.execute_task(&task, b"input data").await?;
    println!("   ✅ Task executed");
    println!("   📤 Output CID: {}\n", work_result.output_cid);

    // ============================================================================
    // Open Challenge Window
    // ============================================================================
    println!("⏱️ Opening challenge window...");

    let window_id = dispute_mgr
        .open_challenge_window(&task.task_id, &work_result.result_id, 10)
        .await?;

    println!("   ✅ Challenge window opened");
    println!("   🆔 Window ID: {}\n", window_id);

    // ============================================================================
    // Submit Challenge
    // ============================================================================
    println!("⚖️ Challenger submits dispute...");

    let evidence = DisputeEvidence::ReExecutionProof {
        different_output_cid: "QmDifferentOutput456".to_string(),
        execution_trace: vec![
            blake3::hash(b"state1").into(),
            blake3::hash(b"state2").into(),
            blake3::hash(b"state3").into(),
        ],
    };

    let challenger_stake = AdicAmount::from_adic(15.0);

    let dispute_id = dispute_mgr
        .submit_challenge(
            &task.task_id,
            &work_result.result_id,
            &worker,
            &challenger,
            challenger_stake,
            evidence.clone(),
            10,
        )
        .await?;

    println!("   ✅ Challenge submitted");
    println!("   🆔 Dispute ID: {}", hex::encode(&dispute_id[..8]));
    println!("   💰 Challenger stake: {} ADIC", 15.0);
    println!("   📋 Evidence type: Re-execution proof\n");

    // ============================================================================
    // Verify Evidence
    // ============================================================================
    println!("🔍 Verifying challenge evidence...");

    let is_valid = dispute_mgr
        .verify_evidence(&evidence, &work_result)
        .await?;

    println!("   ✅ Evidence verification: {}", if is_valid { "VALID" } else { "INVALID" });

    if !is_valid {
        println!("   ❌ Evidence invalid, challenge rejected\n");
        return Ok(());
    }

    println!("   ✅ Re-execution produced different output");
    println!("   ⚠️ Potential fraud detected\n");

    // ============================================================================
    // Select Arbitration Committee
    // ============================================================================
    println!("👥 Selecting arbitration committee...");

    let committee = dispute_mgr
        .select_arbitration_committee(&dispute_id, 15)
        .await?;

    println!("   ✅ Committee selected: {} arbitrators", committee.len());
    println!("   📊 Committee members:");
    for (i, arbitrator) in committee.iter().enumerate() {
        println!("      {}. {}", i + 1, hex::encode(&arbitrator.as_bytes()[..8]));
    }
    println!();

    // ============================================================================
    // Arbitration Voting
    // ============================================================================
    println!("🗳️ Arbitration committee voting...");

    // Simulate voting: 5 for challenger, 2 for worker
    let votes_for_challenger = 5;
    let votes_for_worker = 2;

    let threshold_met = votes_for_challenger as f64 / committee.len() as f64
        >= 0.67; // arbitration threshold

    println!("   📊 Votes for challenger: {}", votes_for_challenger);
    println!("   📊 Votes for worker: {}", votes_for_worker);
    println!("   📊 Threshold (67%): {}", threshold_met);
    println!();

    // ============================================================================
    // Process Arbitration Decision
    // ============================================================================
    println!("⚖️ Processing arbitration decision...");

    let decision = ArbitrationDecision {
        dispute_id,
        arbitrators: committee.clone(),
        votes_for_challenger,
        votes_for_worker,
        outcome: DisputeOutcome::ChallengeAccepted,
        reasoning: "Re-execution proof verified. Worker produced incorrect result. \
                    Challenger's claim is substantiated.".to_string(),
        decided_at_epoch: 20,
    };

    let outcome = dispute_mgr
        .process_arbitration(&dispute_id, decision.clone())
        .await?;

    println!("   ✅ Decision processed");
    println!("   ⚖️ Outcome: {:?}", outcome);
    println!("   📝 Reasoning: {}\n", decision.reasoning);

    // ============================================================================
    // Slash Worker Collateral
    // ============================================================================
    println!("⚠️ Slashing worker collateral...");

    let slashing_event = reward_mgr
        .slash_collateral(
            &task.task_id,
            &worker,
            task.collateral_requirement,
            SlashingReason::IncorrectResult,
            blake3::hash(b"evidence").into(),
            Some(challenger),
        )
        .await?;

    println!("   ✅ Collateral slashed");
    println!("   👷 Slashed account: {}", hex::encode(&slashing_event.slashed_account.as_bytes()[..8]));
    println!("   💸 Amount: {} ADIC", task.collateral_requirement.to_base_units() as f64 / 1_000_000.0);
    println!("   📋 Reason: {:?}\n", slashing_event.reason);

    // ============================================================================
    // Calculate Challenger Reward
    // ============================================================================
    println!("💰 Calculating challenger reward...");

    let challenger_reward = dispute_mgr.calculate_challenger_reward(task.collateral_requirement);

    println!("   ✅ Reward calculated");
    println!("   💵 Challenger receives: {} ADIC (30% of slashed amount)",
             challenger_reward.to_base_units() as f64 / 1_000_000.0);
    println!("   🏛️ Treasury receives: {} ADIC (70% of slashed amount)\n",
             (task.collateral_requirement.to_base_units() - challenger_reward.to_base_units()) as f64 / 1_000_000.0);

    // ============================================================================
    // Update Reputations
    // ============================================================================
    println!("📊 Updating reputations...");

    // Worker loses reputation
    let worker_old_rep = rep_tracker.get_reputation(&worker).await;
    let severity = reputation_mgr.calculate_severity("incorrect_result", 9);

    let worker_rep_update = reputation_mgr
        .update_reputation(&worker, TaskOutcome::Slashed, 0.0, severity, 25)
        .await?;

    println!("   👷 Worker reputation:");
    println!("      Old: {:.2}", worker_rep_update.old_reputation);
    println!("      New: {:.2}", worker_rep_update.new_reputation);
    println!("      Change: {:.2}", worker_rep_update.new_reputation - worker_rep_update.old_reputation);

    // Challenger gains reputation
    let challenger_old_rep = rep_tracker.get_reputation(&challenger).await;
    let challenger_rep_update = reputation_mgr
        .update_validator_reputation(&challenger, true, 25)
        .await?;

    println!("   🔍 Challenger reputation:");
    println!("      Old: {:.2}", challenger_rep_update.old_reputation);
    println!("      New: {:.2}", challenger_rep_update.new_reputation);
    println!("      Change: +{:.2}\n", challenger_rep_update.new_reputation - challenger_rep_update.old_reputation);

    // ============================================================================
    // SCENARIO 2: Invalid Challenge
    // ============================================================================
    println!("📋 SCENARIO 2: Invalid Challenge (Challenger Loses)\n");

    let task2 = Task {
        task_id: [2u8; 32],
        sponsor,
        task_type: TaskType::Compute {
            computation_type: ComputationType::HashVerification,
            resource_requirements: ResourceRequirements::default(),
        },
        input_cid: "QmTask2Input".to_string(),
        expected_output_schema: None,
        reward: AdicAmount::from_adic(50.0),
        collateral_requirement: AdicAmount::from_adic(25.0),
        deadline_epoch: 150,
        min_reputation: 500.0,
        worker_count: 1,
        created_at: Utc::now(),
        status: TaskStatus::Assigned,
        finality_status: FinalityStatus::F2Complete,
    };

    let work_result2 = executor.execute_task(&task2, b"valid input").await?;

    println!("⚙️ Task 2 executed correctly");

    dispute_mgr.open_challenge_window(&task2.task_id, &work_result2.result_id, 30)
        .await?;

    // Malicious challenger submits false challenge
    let false_evidence = DisputeEvidence::InputOutputMismatch {
        expected_output: "expected".to_string(),
        actual_output: "expected".to_string(), // Same! Invalid claim
    };

    let dispute_id2 = dispute_mgr
        .submit_challenge(
            &task2.task_id,
            &work_result2.result_id,
            &worker,
            &challenger,
            AdicAmount::from_adic(12.0),
            false_evidence.clone(),
            30,
        )
        .await?;

    println!("   ⚖️ False challenge submitted\n");

    // Verify evidence (should fail)
    let is_valid2 = dispute_mgr
        .verify_evidence(&false_evidence, &work_result2)
        .await?;

    println!("🔍 Evidence verification: {}", if is_valid2 { "VALID" } else { "INVALID" });

    if !is_valid2 {
        println!("   ❌ Challenge evidence is invalid");
        println!("   ⚠️ Expected and actual outputs match");
        println!("   💸 Challenger loses stake");
        println!("   📉 Challenger reputation penalty\n");

        // Penalize malicious challenger
        let malicious_severity = reputation_mgr.calculate_severity("incorrect_result", 5);
        let challenger_penalty = reputation_mgr
            .update_reputation(&challenger, TaskOutcome::Failure, 0.0, malicious_severity, 35)
            .await?;

        println!("   📊 Challenger penalty:");
        println!("      Old reputation: {:.2}", challenger_penalty.old_reputation);
        println!("      New reputation: {:.2}", challenger_penalty.new_reputation);
        println!("      Penalty: {:.2}\n", challenger_penalty.old_reputation - challenger_penalty.new_reputation);
    }

    // ============================================================================
    // Statistics
    // ============================================================================
    println!("📊 Final Dispute Statistics\n");

    let dispute_stats = dispute_mgr.get_stats().await;
    println!("   Total disputes: {}", dispute_stats.total_disputes);
    println!("   Submitted: {}", dispute_stats.submitted);
    println!("   Under review: {}", dispute_stats.under_review);
    println!("   Accepted: {}", dispute_stats.accepted);
    println!("   Rejected: {}", dispute_stats.rejected);

    let reward_stats = reward_mgr.get_stats().await;
    println!("\n   Slashing Events:");
    println!("      Total slashed: {} ADIC",
             reward_stats.total_collateral_slashed.to_base_units() as f64 / 1_000_000.0);
    println!("      Tasks affected: {}", reward_stats.total_tasks_slashed);

    println!("\n🎉 Dispute Resolution Example Complete!\n");
    println!("This example demonstrated:");
    println!("  ✓ Challenge submission with evidence");
    println!("  ✓ Evidence verification (re-execution, I/O mismatch)");
    println!("  ✓ Arbitration committee selection");
    println!("  ✓ Voting and decision processing");
    println!("  ✓ Collateral slashing for fraud");
    println!("  ✓ Challenger rewards (30% of slashed)");
    println!("  ✓ Reputation penalties for fraudulent workers");
    println!("  ✓ Reputation penalties for invalid challenges");
    println!("  ✓ Dispute statistics tracking\n");

    Ok(())
}
