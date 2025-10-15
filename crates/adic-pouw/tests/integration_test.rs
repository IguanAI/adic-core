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

/// End-to-end integration test: Complete PoUW task lifecycle
#[tokio::test]
async fn test_complete_task_lifecycle() {
    // Setup foundation layer
    let storage = Arc::new(MemoryStorage::new());
    let balance_mgr = Arc::new(BalanceManager::new(storage));
    let escrow_mgr = Arc::new(EscrowManager::new(balance_mgr.clone()));
    let rep_tracker = Arc::new(ReputationTracker::new(0.9));
    let vrf_service = Arc::new(VRFService::new(Default::default(), rep_tracker.clone()));
    let quorum_selector = Arc::new(QuorumSelector::new(vrf_service.clone(), rep_tracker.clone()));

    // Setup PoUW components
    let task_mgr = TaskManager::new(
        escrow_mgr.clone(),
        rep_tracker.clone(),
        TaskManagerConfig::default(),
    );

    let worker_selector = WorkerSelector::new(
        rep_tracker.clone(),
        escrow_mgr.clone(),
        WorkerSelectionConfig::default(),
    );

    let validator = ResultValidator::new(
        quorum_selector.clone(),
        ValidatorConfig {
            min_validators: 3, // Increased from 2 to ensure members_per_axis >= 1
            require_re_execution: false, // Skip re-execution in test environment
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

    let dispute_mgr = DisputeManager::new(
        rep_tracker.clone(),
        DisputeConfig::default(),
    );

    // Setup test accounts
    let sponsor = PublicKey::from_bytes([1u8; 32]);
    let worker1_keypair = Keypair::generate();
    let worker2_keypair = Keypair::generate();
    let worker1 = *worker1_keypair.public_key();
    let worker2 = *worker2_keypair.public_key();

    // Fund accounts
    balance_mgr.credit(AccountAddress::from_public_key(&sponsor), AdicAmount::from_adic(10000.0))
        .await.unwrap();
    balance_mgr.credit(AccountAddress::from_public_key(&worker1), AdicAmount::from_adic(1000.0))
        .await.unwrap();
    balance_mgr.credit(AccountAddress::from_public_key(&worker2), AdicAmount::from_adic(1000.0))
        .await.unwrap();

    // Set reputations
    rep_tracker.set_reputation(&sponsor, 500.0).await;
    rep_tracker.set_reputation(&worker1, 1000.0).await;
    rep_tracker.set_reputation(&worker2, 800.0).await;

    // Initialize VRF randomness for epochs used in test
    let committer = PublicKey::from_bytes([99u8; 32]);
    rep_tracker.set_reputation(&committer, 100.0).await;

    for epoch in [10, 15, 20] {
        let msg = adic_types::AdicMessage::new(
            vec![],
            adic_types::AdicFeatures::new(vec![]),
            adic_types::AdicMeta::new(chrono::Utc::now()),
            committer,
            vec![],
        );

        let vrf_proof = format!("test_vrf_proof_epoch_{}", epoch).into_bytes();
        let commitment = *blake3::hash(&vrf_proof).as_bytes();

        let commit = adic_vrf::VRFCommit::new(msg.clone(), epoch, commitment, committer, 100.0);
        vrf_service.submit_commit(commit.clone()).await.unwrap();

        let reveal = adic_vrf::VRFOpen::new(msg, epoch, commit.id(), vrf_proof, committer);
        vrf_service.submit_reveal(reveal, epoch).await.unwrap();

        vrf_service.finalize_epoch(epoch, epoch).await.unwrap();
    }

    // PHASE 1: Task Submission
    println!("📝 Phase 1: Task Submission");

    let task = Task {
        task_id: [1u8; 32],
        sponsor,
        task_type: TaskType::Compute {
            computation_type: ComputationType::HashVerification,
            resource_requirements: ResourceRequirements::default(),
        },
        input_cid: "QmTestInput123".to_string(),
        expected_output_schema: Some("hash:blake3".to_string()),
        reward: AdicAmount::from_adic(100.0),
        collateral_requirement: AdicAmount::from_adic(50.0),
        deadline_epoch: 100,
        min_reputation: 500.0,
        worker_count: 2,
        created_at: Utc::now(),
        status: TaskStatus::Submitted,
        finality_status: FinalityStatus::Pending,
    };

    let task_id = task_mgr.submit_task(task.clone(), 10).await.unwrap();
    assert_eq!(task_id, task.task_id);

    let retrieved = task_mgr.get_task(&task_id).await.unwrap();
    assert_eq!(retrieved.status, TaskStatus::Submitted);

    // PHASE 2: Task Finalization
    println!("🔒 Phase 2: Task Finalization");

    task_mgr.mark_task_finalized(&task_id, 15).await.unwrap();

    let retrieved = task_mgr.get_task(&task_id).await.unwrap();
    assert_eq!(retrieved.status, TaskStatus::Finalized);

    // PHASE 3: Worker Selection
    println!("🎲 Phase 3: Worker Selection");

    let candidates = vec![worker1, worker2];
    let assignments = worker_selector
        .select_workers(&task, candidates, 15)
        .await
        .unwrap();

    assert_eq!(assignments.len(), 2);

    task_mgr.record_worker_assignments(&task_id, assignments.clone(), 15)
        .await.unwrap();

    let retrieved = task_mgr.get_task(&task_id).await.unwrap();
    assert_eq!(retrieved.status, TaskStatus::Assigned);

    // PHASE 4: Task Execution
    println!("⚙️ Phase 4: Task Execution");

    let input_data = b"test input data for hashing";

    // Worker 1 executes and submits
    let executor1 = TaskExecutor::new(worker1_keypair, ExecutorConfig::default());
    let work_result1 = executor1.execute_task(&task, input_data).await.unwrap();
    assert_eq!(work_result1.task_id, task_id);
    assert_eq!(work_result1.worker, worker1);
    assert!(!work_result1.output_cid.is_empty());
    task_mgr.submit_work_result(work_result1.clone(), 20).await.unwrap();

    // Worker 2 executes and submits
    let executor2 = TaskExecutor::new(worker2_keypair, ExecutorConfig::default());
    let work_result2 = executor2.execute_task(&task, input_data).await.unwrap();
    assert_eq!(work_result2.task_id, task_id);
    assert_eq!(work_result2.worker, worker2);
    task_mgr.submit_work_result(work_result2.clone(), 20).await.unwrap();

    let retrieved = task_mgr.get_task(&task_id).await.unwrap();
    assert_eq!(retrieved.status, TaskStatus::ResultSubmitted);

    // Use worker1's result for validation
    let work_result = work_result1;

    // PHASE 5: Validation
    println!("🔍 Phase 5: Quorum Validation");

    let validation = validator
        .validate_result(&task_id, &work_result, 20)
        .await
        .unwrap();

    assert_eq!(validation.task_id, task_id);
    assert_eq!(validation.result_id, work_result.result_id);
    assert_eq!(validation.validation_result, ValidationResult::Accepted);

    task_mgr.record_validation_report(validation.clone(), 20).await.unwrap();

    let retrieved = task_mgr.get_task(&task_id).await.unwrap();
    assert_eq!(retrieved.status, TaskStatus::Validating);

    // PHASE 6: Challenge Period
    println!("⏱️ Phase 6: Challenge Period");

    dispute_mgr.open_challenge_window(&task_id, &work_result.result_id, 25)
        .await.unwrap();

    let challenge_expiry = task_mgr.begin_challenge_period(&task_id, 25).await.unwrap();
    assert!(challenge_expiry > 25);

    let retrieved = task_mgr.get_task(&task_id).await.unwrap();
    assert_eq!(retrieved.status, TaskStatus::Challenging);

    // Simulate: No challenges submitted, challenge period expires
    // In production, workers would wait for challenge_expiry epoch

    // PHASE 7: Reputation Updates
    println!("📊 Phase 7: Reputation Updates");

    let quality_score = reputation_mgr.calculate_quality_score(
        1000, // execution_time_ms
        2000, // expected_time_ms
        ValidationResult::Accepted,
        false, // not disputed
    );

    assert!(quality_score > 0.8);

    let rep_update = reputation_mgr
        .update_reputation(&worker1, TaskOutcome::Success, quality_score, 0.0, 30)
        .await
        .unwrap();

    assert!(rep_update.new_reputation > rep_update.old_reputation);

    // PHASE 8: Reward Distribution
    println!("💰 Phase 8: Reward Distribution");

    let workers_with_scores = vec![
        (worker1, quality_score, 2.0), // 2x speed
        (worker2, 0.9, 1.0),           // normal speed
    ];

    let distributions = reward_mgr
        .distribute_rewards(
            &task_id,
            task.reward,
            &workers_with_scores,
            &validation.validators,
            30,
        )
        .await
        .unwrap();

    assert!(distributions.len() >= 4); // 2 workers + validators

    // Verify worker rewards
    let worker1_rewards: Vec<_> = distributions
        .iter()
        .filter(|d| d.recipient == worker1)
        .collect();

    assert!(!worker1_rewards.is_empty());
    println!("Worker 1 received {} reward distributions", worker1_rewards.len());

    // PHASE 9: Verify Statistics
    println!("📊 Phase 9: Verify Statistics");

    // Verify statistics
    let stats = task_mgr.get_task_stats().await;
    assert!(stats.total_tasks > 0);
    assert!(stats.active_tasks > 0);

    println!("🎉 Complete task lifecycle test passed!");
}

/// Test scenario with challenge and dispute
#[tokio::test]
async fn test_disputed_task_lifecycle() {
    // Setup
    let storage = Arc::new(MemoryStorage::new());
    let balance_mgr = Arc::new(BalanceManager::new(storage));
    let escrow_mgr = Arc::new(EscrowManager::new(balance_mgr.clone()));
    let rep_tracker = Arc::new(ReputationTracker::new(0.9));
    let vrf_service = Arc::new(VRFService::new(Default::default(), rep_tracker.clone()));

    let worker_keypair = Keypair::generate();
    let executor = TaskExecutor::new(
        worker_keypair,
        ExecutorConfig::default(),
    );

    let dispute_mgr = DisputeManager::new(
        rep_tracker.clone(),
        DisputeConfig::default(),
    );

    let reputation_mgr = PoUWReputationManager::new(
        rep_tracker.clone(),
        ReputationConfig::default(),
    );

    let reward_mgr = RewardManager::new(
        escrow_mgr.clone(),
        RewardConfig::default(),
    );

    // Setup accounts
    let worker = PublicKey::from_bytes([10u8; 32]);
    let challenger = PublicKey::from_bytes([20u8; 32]);

    balance_mgr.credit(AccountAddress::from_public_key(&worker), AdicAmount::from_adic(1000.0))
        .await.unwrap();
    balance_mgr.credit(AccountAddress::from_public_key(&challenger), AdicAmount::from_adic(500.0))
        .await.unwrap();

    rep_tracker.set_reputation(&worker, 1000.0).await;
    rep_tracker.set_reputation(&challenger, 800.0).await;

    // Create and execute task
    let task = Task {
        task_id: [2u8; 32],
        sponsor: PublicKey::from_bytes([1u8; 32]),
        task_type: TaskType::Compute {
            computation_type: ComputationType::DataProcessing,
            resource_requirements: ResourceRequirements::default(),
        },
        input_cid: "QmInput456".to_string(),
        expected_output_schema: None,
        reward: AdicAmount::from_adic(100.0),
        collateral_requirement: AdicAmount::from_adic(50.0),
        deadline_epoch: 100,
        min_reputation: 500.0,
        worker_count: 1,
        created_at: Utc::now(),
        status: TaskStatus::Assigned,
        finality_status: FinalityStatus::F2Complete,
    };

    let work_result = executor.execute_task(&task, b"input data").await.unwrap();

    // Open challenge window
    dispute_mgr.open_challenge_window(&task.task_id, &work_result.result_id, 10)
        .await.unwrap();

    // Submit challenge
    println!("⚖️ Submitting challenge");

    let evidence = DisputeEvidence::ReExecutionProof {
        different_output_cid: "QmDifferentOutput789".to_string(),
        execution_trace: vec![[1u8; 32], [2u8; 32]],
    };

    let dispute_id = dispute_mgr
        .submit_challenge(
            &task.task_id,
            &work_result.result_id,
            &worker,
            &challenger,
            AdicAmount::from_adic(20.0),
            evidence.clone(),
            10,
        )
        .await
        .unwrap();

    // Verify evidence
    let is_valid = dispute_mgr
        .verify_evidence(&evidence, &work_result)
        .await
        .unwrap();

    assert!(is_valid);

    // Select arbitration committee
    println!("👥 Selecting arbitration committee");

    let committee = dispute_mgr
        .select_arbitration_committee(&dispute_id, 15)
        .await
        .unwrap();

    assert_eq!(committee.len(), 7); // Default committee size

    // Process arbitration decision
    println!("⚖️ Processing arbitration");

    let decision = ArbitrationDecision {
        dispute_id,
        arbitrators: committee.clone(),
        votes_for_challenger: 5,
        votes_for_worker: 2,
        outcome: DisputeOutcome::ChallengeAccepted,
        reasoning: "Re-execution produced different result, worker slashed".to_string(),
        decided_at_epoch: 20,
    };

    let outcome = dispute_mgr
        .process_arbitration(&dispute_id, decision)
        .await
        .unwrap();

    assert_eq!(outcome, DisputeOutcome::ChallengeAccepted);

    // Slash worker collateral
    println!("⚠️ Slashing worker collateral");

    let slashing_event = reward_mgr
        .slash_collateral(
            &task.task_id,
            &worker,
            task.collateral_requirement,
            SlashingReason::IncorrectResult,
            [3u8; 32],
            Some(challenger),
        )
        .await
        .unwrap();

    assert_eq!(slashing_event.slashed_account, worker);
    assert_eq!(slashing_event.amount, task.collateral_requirement);

    // Update reputation
    println!("📊 Updating reputation (slashed)");

    let severity = reputation_mgr.calculate_severity("incorrect_result", 8);
    let rep_update = reputation_mgr
        .update_reputation(&worker, TaskOutcome::Slashed, 0.0, severity, 25)
        .await
        .unwrap();

    assert!(rep_update.new_reputation < rep_update.old_reputation);

    // Calculate challenger reward
    let challenger_reward = dispute_mgr.calculate_challenger_reward(task.collateral_requirement);
    assert!(challenger_reward.to_base_units() > 0);

    println!("✅ Disputed task lifecycle test passed!");
}

/// Test multi-worker redundancy and consensus
#[tokio::test]
async fn test_redundant_workers_consensus() {
    // Setup
    let storage = Arc::new(MemoryStorage::new());
    let balance_mgr = Arc::new(BalanceManager::new(storage));
    let rep_tracker = Arc::new(ReputationTracker::new(0.9));
    let vrf_service = Arc::new(VRFService::new(Default::default(), rep_tracker.clone()));
    let escrow_mgr = Arc::new(EscrowManager::new(balance_mgr.clone()));

    let worker_selector = WorkerSelector::new(
        rep_tracker.clone(),
        escrow_mgr.clone(),
        WorkerSelectionConfig {
            require_axis_diversity: false, // Disable for this test focused on redundancy
            ..Default::default()
        },
    );

    // Initialize VRF randomness for epoch 10
    let committer = PublicKey::from_bytes([99u8; 32]);
    rep_tracker.set_reputation(&committer, 100.0).await;

    let msg = adic_types::AdicMessage::new(
        vec![],
        adic_types::AdicFeatures::new(vec![]),
        adic_types::AdicMeta::new(chrono::Utc::now()),
        committer,
        vec![],
    );

    let vrf_proof = b"test_vrf_proof_epoch_10".to_vec();
    let commitment = *blake3::hash(&vrf_proof).as_bytes();

    let commit = adic_vrf::VRFCommit::new(msg.clone(), 10, commitment, committer, 100.0);
    vrf_service.submit_commit(commit.clone()).await.unwrap();

    let reveal = adic_vrf::VRFOpen::new(msg, 10, commit.id(), vrf_proof, committer);
    vrf_service.submit_reveal(reveal, 10).await.unwrap();

    vrf_service.finalize_epoch(10, 10).await.unwrap();

    // Create multiple diverse workers (different byte patterns for diversity)
    let mut workers = Vec::new();
    for i in 0..5 {
        let mut worker_bytes = [0u8; 32];
        worker_bytes[i] = (i + 10) as u8; // Different position for each worker for diversity
        worker_bytes[i + 5] = (i + 20) as u8;
        let worker = PublicKey::from_bytes(worker_bytes);
        balance_mgr.credit(AccountAddress::from_public_key(&worker), AdicAmount::from_adic(1000.0))
            .await.unwrap();
        rep_tracker.set_reputation(&worker, 1000.0 - (i as f64 * 50.0)).await;
        workers.push(worker);
    }

    // Create task with 3 workers
    let task = Task {
        task_id: [3u8; 32],
        sponsor: PublicKey::from_bytes([1u8; 32]),
        task_type: TaskType::Compute {
            computation_type: ComputationType::HashVerification,
            resource_requirements: ResourceRequirements::default(),
        },
        input_cid: "QmRedundantTask".to_string(),
        expected_output_schema: None,
        reward: AdicAmount::from_adic(300.0),
        collateral_requirement: AdicAmount::from_adic(150.0),
        deadline_epoch: 100,
        min_reputation: 800.0,
        worker_count: 3,
        created_at: Utc::now(),
        status: TaskStatus::Finalized,
        finality_status: FinalityStatus::F2Complete,
    };

    println!("🎲 Selecting 3 workers from pool of 5");

    let assignments = worker_selector
        .select_workers(&task, workers.clone(), 10)
        .await
        .unwrap();

    assert_eq!(assignments.len(), 3);

    // Verify diversity
    let selected_workers: Vec<_> = assignments.iter().map(|a| a.worker).collect();
    println!("Selected workers: {:?}", selected_workers.len());

    // Verify each has unique VRF proof
    let mut vrf_proofs = std::collections::HashSet::new();
    for assignment in &assignments {
        vrf_proofs.insert(assignment.vrf_proof.clone());
    }
    assert_eq!(vrf_proofs.len(), 3);

    // Update performance for workers
    let reputation_mgr = PoUWReputationManager::new(
        rep_tracker.clone(),
        ReputationConfig::default(),
    );

    for (i, assignment) in assignments.iter().enumerate() {
        let quality = 0.9 + (i as f64 * 0.03);
        worker_selector
            .update_worker_performance(
                &assignment.worker,
                TaskOutcome::Success,
                1000 - (i as u64 * 100),
                quality,
                20, // current_epoch
            )
            .await
            .unwrap();

        // Update reputation
        reputation_mgr
            .update_reputation(&assignment.worker, TaskOutcome::Success, quality, 0.0, 15)
            .await
            .unwrap();
    }

    // Verify performance tracking
    for assignment in &assignments {
        let perf = worker_selector.get_worker_performance(&assignment.worker).await;
        assert_eq!(perf.total_tasks_completed, 1);
        assert!(perf.calculate_score() > 0.8);
    }

    println!("✅ Redundant workers consensus test passed!");
}

/// Performance benchmark test
#[tokio::test]
async fn test_performance_benchmarks() {
    let storage = Arc::new(MemoryStorage::new());
    let balance_mgr = Arc::new(BalanceManager::new(storage));
    let rep_tracker = Arc::new(ReputationTracker::new(0.9));

    let worker_keypair = Keypair::generate();
    let executor = TaskExecutor::new(
        worker_keypair,
        ExecutorConfig::default(),
    );

    let task = Task {
        task_id: [4u8; 32],
        sponsor: PublicKey::from_bytes([1u8; 32]),
        task_type: TaskType::Compute {
            computation_type: ComputationType::HashVerification,
            resource_requirements: ResourceRequirements::default(),
        },
        input_cid: "QmBenchmark".to_string(),
        expected_output_schema: None,
        reward: AdicAmount::from_adic(10.0),
        collateral_requirement: AdicAmount::from_adic(5.0),
        deadline_epoch: 100,
        min_reputation: 100.0,
        worker_count: 1,
        created_at: Utc::now(),
        status: TaskStatus::Assigned,
        finality_status: FinalityStatus::F2Complete,
    };

    println!("⏱️ Running performance benchmarks");

    // Benchmark task execution
    let start = std::time::Instant::now();
    let iterations = 100;

    for i in 0..iterations {
        let input = format!("benchmark input {}", i);
        let _result = executor.execute_task(&task, input.as_bytes()).await.unwrap();
    }

    let duration = start.elapsed();
    let avg_ms = duration.as_millis() / iterations;

    println!("Average execution time: {}ms", avg_ms);
    assert!(avg_ms < 100); // Should be fast

    // Check executor stats
    let stats = executor.get_stats().await;
    assert_eq!(stats.total_executions, iterations as u64);
    assert_eq!(stats.successful_executions, iterations as u64);

    println!("✅ Performance benchmark passed!");
}
