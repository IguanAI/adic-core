/// Basic usage example for ADIC PoUW Framework
///
/// This example demonstrates:
/// 1. Setting up the PoUW framework
/// 2. Submitting a computation task
/// 3. Worker selection and execution
/// 4. Validation and reward distribution
///
/// Run with: cargo run --example basic_usage

use adic_pouw::*;
use adic_app_common::escrow::EscrowManager;
use adic_consensus::ReputationTracker;
use adic_crypto::Keypair;
use adic_economics::storage::MemoryStorage;
use adic_economics::types::{AccountAddress, AdicAmount};
use adic_economics::BalanceManager;
use adic_quorum::QuorumSelector;
use adic_types::PublicKey;
use adic_vrf::VRFService;
use chrono::Utc;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    println!("🚀 ADIC PoUW Framework - Basic Usage Example\n");

    // ============================================================================
    // STEP 1: Initialize Foundation Layer
    // ============================================================================
    println!("📦 Step 1: Initializing foundation layer...");

    let storage = Arc::new(MemoryStorage::new());
    let balance_mgr = Arc::new(BalanceManager::new(storage));
    let escrow_mgr = Arc::new(EscrowManager::new(balance_mgr.clone()));
    let rep_tracker = Arc::new(ReputationTracker::new(0.9));
    let vrf_service = Arc::new(VRFService::new(Default::default(), rep_tracker.clone()));
    let quorum_selector = Arc::new(QuorumSelector::new(vrf_service.clone(), rep_tracker.clone()));

    println!("   ✅ Foundation layer ready\n");

    // ============================================================================
    // STEP 2: Initialize PoUW Components
    // ============================================================================
    println!("⚙️ Step 2: Initializing PoUW components...");

    let task_manager = TaskManager::new(
        escrow_mgr.clone(),
        rep_tracker.clone(),
        TaskManagerConfig::default(),
    );

    let worker_selector = WorkerSelector::new(
        rep_tracker.clone(),
        escrow_mgr.clone(),
        WorkerSelectionConfig::default(),
    );

    let reputation_mgr = PoUWReputationManager::new(
        rep_tracker.clone(),
        ReputationConfig::default(),
    );

    let reward_mgr = RewardManager::new(
        escrow_mgr.clone(),
        RewardConfig::default(),
    );

    println!("   ✅ PoUW components ready\n");

    // ============================================================================
    // STEP 3: Setup Test Accounts
    // ============================================================================
    println!("👤 Step 3: Setting up accounts...");

    let sponsor = PublicKey::from_bytes([1u8; 32]);
    let worker1 = PublicKey::from_bytes([10u8; 32]);
    let worker2 = PublicKey::from_bytes([11u8; 32]);
    let worker3 = PublicKey::from_bytes([12u8; 32]);

    // Fund accounts
    balance_mgr.credit(AccountAddress::from_public_key(&sponsor), AdicAmount::from_adic(10000.0))
        .await.unwrap();
    balance_mgr.credit(AccountAddress::from_public_key(&worker1), AdicAmount::from_adic(1000.0))
        .await.unwrap();
    balance_mgr.credit(AccountAddress::from_public_key(&worker2), AdicAmount::from_adic(1000.0))
        .await.unwrap();
    balance_mgr.credit(AccountAddress::from_public_key(&worker3), AdicAmount::from_adic(1000.0))
        .await.unwrap();

    // Set reputations
    rep_tracker.set_reputation(&sponsor, 500.0).await;
    rep_tracker.set_reputation(&worker1, 1000.0).await;
    rep_tracker.set_reputation(&worker2, 950.0).await;
    rep_tracker.set_reputation(&worker3, 900.0).await;

    println!("   ✅ Sponsor funded: 10,000 ADIC");
    println!("   ✅ Workers funded: 1,000 ADIC each");
    println!("   ✅ Reputations set\n");

    // ============================================================================
    // STEP 4: Submit Task
    // ============================================================================
    println!("📝 Step 4: Submitting computation task...");

    let task = Task {
        task_id: [1u8; 32],
        sponsor,
        task_type: TaskType::Compute {
            computation_type: ComputationType::HashVerification,
            resource_requirements: ResourceRequirements {
                max_cpu_ms: 10_000,
                max_memory_mb: 512,
                max_storage_mb: 1024,
                max_network_kb: 5120,
            },
        },
        input_cid: "QmExampleInput123".to_string(),
        expected_output_schema: Some("hash:blake3".to_string()),
        reward: AdicAmount::from_adic(300.0),
        collateral_requirement: AdicAmount::from_adic(150.0),
        deadline_epoch: 100,
        min_reputation: 800.0,
        worker_count: 3,
        created_at: Utc::now(),
        status: TaskStatus::Submitted,
        finality_status: FinalityStatus::Pending,
    };

    let task_id = task_manager.submit_task(task.clone(), 10).await?;

    println!("   ✅ Task submitted");
    println!("   📋 Task ID: {}", hex::encode(&task_id[..8]));
    println!("   💰 Reward: {} ADIC", 300.0);
    println!("   🔒 Collateral: {} ADIC", 150.0);
    println!("   👥 Workers needed: {}\n", task.worker_count);

    // ============================================================================
    // STEP 5: Mark Task as Finalized
    // ============================================================================
    println!("🔒 Step 5: Waiting for finality...");

    task_manager.mark_task_finalized(&task_id, 15).await?;

    let finalized_task = task_manager.get_task(&task_id).await.unwrap();
    println!("   ✅ Task finalized at epoch 15");
    println!("   📊 Status: {:?}\n", finalized_task.status);

    // ============================================================================
    // STEP 6: Worker Selection
    // ============================================================================
    println!("🎲 Step 6: Selecting workers via VRF...");

    let candidates = vec![worker1, worker2, worker3];
    let assignments = worker_selector
        .select_workers(&task, candidates, 15)
        .await?;

    println!("   ✅ Selected {} workers", assignments.len());
    for (i, assignment) in assignments.iter().enumerate() {
        println!("   👷 Worker {}: {}", i + 1, hex::encode(&assignment.worker.as_bytes()[..8]));
    }
    println!();

    task_manager.record_worker_assignments(&task_id, assignments.clone(), 15).await?;

    // ============================================================================
    // STEP 7: Task Execution (simulated)
    // ============================================================================
    println!("⚙️ Step 7: Workers executing task...");

    let input_data = b"Example computation input data";

    // All 3 workers execute and submit results
    for (i, _worker) in [worker1, worker2, worker3].iter().enumerate() {
        let worker_keypair = Keypair::generate();
        let executor = TaskExecutor::new(worker_keypair, ExecutorConfig::default());
        let result = executor.execute_task(&task, input_data).await?;
        task_manager.submit_work_result(result.clone(), 20).await?;

        if i == 0 {
            println!("   ✅ Execution complete (showing worker 1 results)");
            println!("   📤 Output CID: {}", result.output_cid);
            println!("   ⏱️ Execution time: {} ms", result.execution_metrics.cpu_time_ms);
            println!("   💾 Memory used: {} MB", result.execution_metrics.memory_used_mb);
            println!("   🔐 Proof type: {:?}", result.execution_proof.proof_type);
        }
    }

    println!("   ✅ All {} workers submitted results\n", 3);

    // Get first work result for validation
    let work_results = task_manager.get_work_results(&task_id).await.unwrap();
    let work_result = work_results.first().unwrap().clone();

    // ============================================================================
    // STEP 8: Validation
    // ============================================================================
    println!("🔍 Step 8: Quorum validation...");

    let validator = ResultValidator::new(
        quorum_selector.clone(),
        ValidatorConfig::default(),
    );

    let validation = validator
        .validate_result(&task_id, &work_result, 20)
        .await?;

    println!("   ✅ Validation complete");
    println!("   📊 Result: {:?}", validation.validation_result);
    println!("   👥 Validators: {}", validation.validators.len());
    println!("   ✅ Votes for: {}", validation.votes_for);
    println!("   ❌ Votes against: {}\n", validation.votes_against);

    task_manager.record_validation_report(validation.clone(), 20).await?;

    // ============================================================================
    // STEP 9: Challenge Period
    // ============================================================================
    println!("⏱️ Step 9: Challenge period (24 epochs)...");

    let challenge_expiry = task_manager.begin_challenge_period(&task_id, 25).await?;

    println!("   ✅ Challenge window opened");
    println!("   ⏰ Expires at epoch: {}", challenge_expiry);
    println!("   ⌛ Waiting for challenges... (none submitted)\n");

    // ============================================================================
    // STEP 10: Reputation Updates
    // ============================================================================
    println!("📊 Step 10: Updating reputation...");

    let quality_score = reputation_mgr.calculate_quality_score(
        work_result.execution_metrics.cpu_time_ms,
        5000, // expected time
        ValidationResult::Accepted,
        false, // no disputes
    );

    let rep_update = reputation_mgr
        .update_reputation(&worker1, TaskOutcome::Success, quality_score, 0.0, 30)
        .await?;

    println!("   ✅ Reputation updated for worker 1");
    println!("   📈 Old reputation: {:.2}", rep_update.old_reputation);
    println!("   📈 New reputation: {:.2}", rep_update.new_reputation);
    println!("   ⭐ Quality score: {:.2}\n", quality_score);

    // ============================================================================
    // STEP 11: Reward Distribution
    // ============================================================================
    println!("💰 Step 11: Distributing rewards...");

    let workers_with_scores = vec![
        (worker1, quality_score, 2.0), // 2x speed bonus
        (worker2, 0.90, 1.0),          // normal speed
        (worker3, 0.85, 0.9),          // slightly slower
    ];

    let distributions = reward_mgr
        .distribute_rewards(
            &task_id,
            task.reward,
            &workers_with_scores,
            &validation.validators,
            30,
        )
        .await?;

    println!("   ✅ Rewards distributed: {} payments", distributions.len());

    let mut total_distributed = 0u64;
    for dist in &distributions {
        total_distributed += dist.amount.to_base_units();
        println!(
            "   💵 {} → {} ADIC ({:?})",
            hex::encode(&dist.recipient.as_bytes()[..6]),
            dist.amount.to_base_units() as f64 / 1_000_000.0,
            dist.reward_type
        );
    }

    println!("\n   📊 Total distributed: {} ADIC\n", total_distributed as f64 / 1_000_000.0);

    // ============================================================================
    // STEP 12: Task Status
    // ============================================================================
    println!("📊 Step 12: Checking task status...");

    let current_task = task_manager.get_task(&task_id).await.unwrap();
    println!("   ✅ Task status: {:?}\n", current_task.status);

    // ============================================================================
    // STEP 13: Statistics
    // ============================================================================
    println!("📊 Step 13: Final statistics...\n");

    let task_stats = task_manager.get_task_stats().await;
    println!("   📋 Task Statistics:");
    println!("      Total tasks: {}", task_stats.total_tasks);
    println!("      Completed: {}", task_stats.completed_tasks);
    println!("      Failed: {}", task_stats.failed_tasks);
    println!();

    let reward_stats = reward_mgr.get_stats().await;
    println!("   💰 Reward Statistics:");
    println!("      Tasks rewarded: {}", reward_stats.total_tasks_rewarded);
    println!("      Total rewards: {} ADIC",
             reward_stats.total_rewards_distributed.to_base_units() as f64 / 1_000_000.0);

    println!("\n🎉 PoUW Framework Example Complete!\n");
    println!("This example demonstrated:");
    println!("  ✓ Task submission with escrow");
    println!("  ✓ VRF-based worker selection");
    println!("  ✓ Task execution with proofs");
    println!("  ✓ Quorum validation");
    println!("  ✓ Challenge period");
    println!("  ✓ Reputation updates");
    println!("  ✓ Multi-tier reward distribution");
    println!("  ✓ Task completion and statistics\n");

    Ok(())
}
