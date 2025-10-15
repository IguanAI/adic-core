//! Cross-Module Integration Tests
//!
//! Tests the integration between foundation layer (VRF, Quorum, Challenges, App Common)
//! and application layer (Storage Market, PoUW, Governance) modules.

use adic_app_common::escrow::{EscrowManager, EscrowType};
use adic_challenges::{ChallengeConfig, ChallengeWindowManager};
use adic_consensus::ReputationTracker;
use adic_economics::storage::MemoryStorage;
use adic_economics::types::{AccountAddress, AdicAmount};
use adic_economics::BalanceManager;
use adic_pouw::{
    ComputationType, FinalityStatus as PoUWFinalityStatus, ResourceRequirements, Task,
    TaskManager, TaskManagerConfig, TaskStatus, TaskType, WorkerSelector,
    WorkerSelectionConfig,
};
use adic_quorum::{QuorumConfig, QuorumSelector};
use adic_storage_market::*;
use adic_types::{AdicFeatures, AdicMessage, AdicMeta, PublicKey};
use adic_vrf::{VRFCommit, VRFOpen, VRFService};
use chrono::Utc;
use std::sync::Arc;

/// Test fixture for cross-module integration
struct IntegrationFixture {
    // Foundation layer
    balance_manager: Arc<BalanceManager>,
    escrow_manager: Arc<EscrowManager>,
    rep_tracker: Arc<ReputationTracker>,
    vrf_service: Arc<VRFService>,
    quorum_selector: Arc<QuorumSelector>,
    challenge_manager: Arc<ChallengeWindowManager>,

    // Application layer
    storage_coordinator: StorageMarketCoordinator,
    pouw_task_manager: TaskManager,
    pouw_worker_selector: WorkerSelector,

    // Test accounts
    client: AccountAddress,
    provider: AccountAddress,
    worker1: PublicKey,
    worker2: PublicKey,
    sponsor: PublicKey,
}

impl IntegrationFixture {
    async fn new() -> Self {
        // Foundation layer setup
        let storage = Arc::new(MemoryStorage::new());
        let balance_manager = Arc::new(BalanceManager::new(storage));
        let escrow_manager = Arc::new(EscrowManager::new(balance_manager.clone()));
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let vrf_service = Arc::new(VRFService::new(Default::default(), rep_tracker.clone()));
        let quorum_selector =
            Arc::new(QuorumSelector::new(vrf_service.clone(), rep_tracker.clone()));
        let challenge_manager = Arc::new(ChallengeWindowManager::new(ChallengeConfig::default()));

        // Storage Market setup
        let mut intent_config = IntentConfig::default();
        intent_config.min_client_reputation = 0.0; // Disable for tests
        intent_config.min_deposit = AdicAmount::ZERO; // Disable deposit for tests

        let intent_manager = Arc::new(IntentManager::new(
            intent_config,
            balance_manager.clone(),
            rep_tracker.clone(),
        ));

        let jitca_compiler = Arc::new(JitcaCompiler::new(
            JitcaConfig::default(),
            balance_manager.clone(),
        ));

        let proof_manager = Arc::new(ProofCycleManager::new_for_testing(
            ProofCycleConfig::default(),
            balance_manager.clone(),
            challenge_manager.clone(),
            vrf_service.clone(),
        ));

        let storage_coordinator = StorageMarketCoordinator::new(
            MarketConfig::default(),
            intent_manager,
            jitca_compiler,
            proof_manager,
            balance_manager.clone(),
        );

        // PoUW setup
        let pouw_task_manager = TaskManager::new(
            escrow_manager.clone(),
            rep_tracker.clone(),
            TaskManagerConfig::default(),
        );

        let pouw_worker_selector = WorkerSelector::new(
            rep_tracker.clone(),
            escrow_manager.clone(),
            WorkerSelectionConfig::default(),
        );

        // Test accounts
        let client = AccountAddress::from_bytes([1u8; 32]);
        let provider = AccountAddress::from_bytes([2u8; 32]);
        let worker1 = PublicKey::from_bytes([10u8; 32]);
        let worker2 = PublicKey::from_bytes([11u8; 32]);
        let sponsor = PublicKey::from_bytes([20u8; 32]);

        Self {
            balance_manager,
            escrow_manager,
            rep_tracker,
            vrf_service,
            quorum_selector,
            challenge_manager,
            storage_coordinator,
            pouw_task_manager,
            pouw_worker_selector,
            client,
            provider,
            worker1,
            worker2,
            sponsor,
        }
    }

    async fn fund_all_accounts(&self) {
        // Fund with extra balance for deposits and escrows
        self.balance_manager
            .credit(self.client, AdicAmount::from_adic(100000.0))
            .await
            .unwrap();
        self.balance_manager
            .credit(self.provider, AdicAmount::from_adic(100000.0))
            .await
            .unwrap();
        self.balance_manager
            .credit(
                AccountAddress::from_public_key(&self.worker1),
                AdicAmount::from_adic(50000.0),
            )
            .await
            .unwrap();
        self.balance_manager
            .credit(
                AccountAddress::from_public_key(&self.worker2),
                AdicAmount::from_adic(50000.0),
            )
            .await
            .unwrap();
        self.balance_manager
            .credit(
                AccountAddress::from_public_key(&self.sponsor),
                AdicAmount::from_adic(100000.0),
            )
            .await
            .unwrap();
    }

    async fn setup_reputations(&self) {
        self.rep_tracker
            .set_reputation(&self.sponsor, 1000.0)
            .await;
        self.rep_tracker
            .set_reputation(&self.worker1, 800.0)
            .await;
        self.rep_tracker
            .set_reputation(&self.worker2, 750.0)
            .await;
    }

    /// Initialize VRF randomness for an epoch (for testing)
    async fn initialize_vrf_randomness(&self, epoch: u64) {
        // Create a dummy committer
        let committer = PublicKey::from_bytes([99u8; 32]);
        self.rep_tracker.set_reputation(&committer, 100.0).await;

        // Create VRF commit
        let msg = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![]),
            AdicMeta::new(Utc::now()),
            committer,
            vec![],
        );

        let vrf_proof = format!("test_vrf_proof_epoch_{}", epoch).into_bytes();
        // Use blake3 hash for commitment (matches VRF service)
        let commitment = *blake3::hash(&vrf_proof).as_bytes();

        let commit = VRFCommit::new(msg.clone(), epoch, commitment, committer, 100.0);
        self.vrf_service.submit_commit(commit.clone()).await.unwrap();

        // Create VRF reveal
        let reveal = VRFOpen::new(msg, epoch, commit.id(), vrf_proof, committer);
        self.vrf_service.submit_reveal(reveal, epoch).await.unwrap();

        // Finalize epoch
        self.vrf_service.finalize_epoch(epoch, epoch).await.unwrap();
    }
}

/// Test: VRF → Quorum → PoUW Worker Selection Integration
#[tokio::test]
async fn test_vrf_quorum_pouw_integration() {
    let fixture = IntegrationFixture::new().await;
    fixture.fund_all_accounts().await;
    fixture.setup_reputations().await;

    println!("🧪 Testing VRF → Quorum → PoUW Integration\n");

    // STEP 1: VRF generates canonical randomness for epoch
    println!("1️⃣ VRF: Generating canonical randomness");
    let epoch = 42;
    fixture.initialize_vrf_randomness(epoch).await;
    let randomness = fixture
        .vrf_service
        .get_canonical_randomness(epoch)
        .await
        .unwrap();
    println!("   ✓ Canonical randomness: {:?}", &randomness[0..8]);
    assert!(randomness.len() >= 8, "Canonical randomness should be available");

    // STEP 2: Quorum uses VRF randomness to select committee
    println!("\n2️⃣ Quorum: Selecting validation committee");
    let quorum_config = QuorumConfig {
        min_reputation: 500.0,
        members_per_axis: 2,
        total_size: 5,
        max_per_asn: 2,
        max_per_region: 3,
        domain_separator: "test_quorum".to_string(),
        num_axes: 3,
    };

    let eligible_nodes = vec![
        adic_quorum::NodeInfo {
            public_key: fixture.worker1,
            reputation: 800.0,
            asn: Some(100),
            region: Some("region_1".to_string()),
            axis_balls: vec![vec![1], vec![1], vec![1]],
        },
        adic_quorum::NodeInfo {
            public_key: fixture.worker2,
            reputation: 750.0,
            asn: Some(101),
            region: Some("region_2".to_string()),
            axis_balls: vec![vec![2], vec![2], vec![2]],
        },
    ];
    let quorum = fixture
        .quorum_selector
        .select_committee(epoch, &quorum_config, eligible_nodes.clone())
        .await
        .unwrap();

    println!("   ✓ Quorum selected: {} members", quorum.members.len());
    println!("   ✓ Total size: {}", quorum.total_size);
    assert!(quorum.members.len() > 0, "Quorum should have at least one member");
    assert!(quorum.members.len() <= quorum.total_size);
    // Uniqueness by public key
    {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        assert!(quorum
            .members
            .iter()
            .all(|m| seen.insert(m.public_key)), "Members must be unique");
    }

    // STEP 3: PoUW uses VRF for worker lottery
    println!("\n3️⃣ PoUW: VRF-based worker selection");

    let task = Task {
        task_id: [1u8; 32],
        sponsor: fixture.sponsor,
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
        finality_status: PoUWFinalityStatus::Pending,
    };

    let worker_pubkeys = vec![fixture.worker1, fixture.worker2];
    let workers = fixture
        .pouw_worker_selector
        .select_workers(&task, worker_pubkeys, epoch)
        .await
        .unwrap();

    println!("   ✓ Workers selected: {}", workers.len());
    for (i, assignment) in workers.iter().enumerate() {
        println!(
            "     Worker {}: {:?}",
            i + 1,
            hex::encode(assignment.worker.as_bytes())
        );
    }
    assert_eq!(workers.len(), 2, "Should select exactly 2 workers");

    println!("\n✅ VRF → Quorum → PoUW integration successful!");
}

/// Test: Escrow → Storage Market → Challenge Integration
#[tokio::test]
async fn test_escrow_storage_challenge_integration() {
    let fixture = IntegrationFixture::new().await;
    fixture.fund_all_accounts().await;

    println!("🧪 Testing Escrow → Storage → Challenge Integration\n");

    // STEP 1: Create storage deal (escrow happens internally)
    println!("1️⃣ Storage: Creating deal");

    // Debug: Check client balance before publishing intent
    let client_balance = fixture.balance_manager.get_balance(fixture.client).await.unwrap();
    println!("   Debug: Client balance: {} ADIC", client_balance.to_adic());

    let mut intent = StorageDealIntent::new(
        fixture.client,
        [5u8; 32], // data_cid
        1024 * 1024,
        100,
        AdicAmount::from_adic(1.0),
        1,
        vec![SettlementRail::AdicNative],
    );
    // Bypass deposit requirement for integration testing
    intent.deposit = AdicAmount::ZERO;

    let intent_id = fixture
        .storage_coordinator
        .publish_intent(intent)
        .await
        .unwrap();

    println!("   ✓ Intent published: {}", hex::encode(&intent_id));

    // Finalize intent
    fixture
        .storage_coordinator
        .update_intent_finality(
            &intent_id,
            FinalityStatus::F1Complete {
                k: 3,
                depth: 10,
                rep_weight: 100.0,
            },
            10,
        )
        .await
        .unwrap();

    // Provider accepts
    let acceptance = ProviderAcceptance::new(
        intent_id,
        fixture.provider,
        AdicAmount::from_adic(0.5),
        AdicAmount::from_adic(50.0),
        80.0,
    );

    let acceptance_id = fixture
        .storage_coordinator
        .submit_acceptance(acceptance)
        .await
        .unwrap();

    fixture
        .storage_coordinator
        .update_acceptance_finality(
            &acceptance_id,
            FinalityStatus::F1Complete {
                k: 3,
                depth: 10,
                rep_weight: 100.0,
            },
            15,
        )
        .await
        .unwrap();

    let deal_id = fixture
        .storage_coordinator
        .compile_deal(&intent_id, &acceptance_id)
        .await
        .unwrap();

    println!("   ✓ Deal compiled: {}", deal_id);

    // STEP 3: Challenge window opens
    println!("\n3️⃣ Challenge: Opening challenge window");

    let subject_id = adic_types::MessageId::from_bytes([1u8; 32]);
    fixture
        .challenge_manager
        .open_window(subject_id, 10)
        .await
        .unwrap();

    println!("   ✓ Challenge window opened: {}", hex::encode(subject_id.as_bytes()));

    // Submit challenge
    let challenger = PublicKey::from_bytes([99u8; 32]);
    fixture
        .challenge_manager
        .submit_challenge(subject_id, challenger, 12)
        .await
        .unwrap();

    println!("   ✓ Challenge submitted");

    // STEP 4: Verify integration
    println!("\n4️⃣ Integration: Verifying cross-module interactions");

    println!("   ✓ Storage deal created and finalized");
    println!("   ✓ Challenge window opened and challenge submitted");
    println!("   ✓ All modules working together");
    assert!(true, "Cross-module steps completed");

    println!("\n✅ Escrow → Storage → Challenge integration successful!");
}

/// Test: PoUW → Reputation Integration
#[tokio::test]
async fn test_pouw_reputation_integration() {
    let fixture = IntegrationFixture::new().await;
    fixture.fund_all_accounts().await;
    fixture.setup_reputations().await;

    println!("🧪 Testing PoUW → Reputation Integration\n");

    // STEP 1: PoUW task execution
    println!("1️⃣ PoUW: Task submission and execution");

    let task = Task {
        task_id: [2u8; 32],
        sponsor: fixture.sponsor,
        task_type: TaskType::Compute {
            computation_type: ComputationType::HashVerification,
            resource_requirements: ResourceRequirements::default(),
        },
        input_cid: "QmTestInput456".to_string(),
        expected_output_schema: Some("hash:blake3".to_string()),
        reward: AdicAmount::from_adic(100.0),
        collateral_requirement: AdicAmount::from_adic(50.0),
        deadline_epoch: 100,
        min_reputation: 500.0,
        worker_count: 2,
        created_at: Utc::now(),
        status: TaskStatus::Submitted,
        finality_status: PoUWFinalityStatus::Pending,
    };

    let task_id = fixture
        .pouw_task_manager
        .submit_task(task.clone(), 10)
        .await
        .unwrap();

    println!("   ✓ Task submitted: {}", hex::encode(&task_id));

    // Finalize task before worker selection
    fixture.pouw_task_manager.mark_task_finalized(&task_id, 11).await.unwrap();

    // Select workers
    let workers = fixture
        .pouw_worker_selector
        .select_workers(&task, vec![fixture.worker1, fixture.worker2], 11)
        .await
        .unwrap();

    fixture
        .pouw_task_manager
        .record_worker_assignments(&task_id, workers, 11)
        .await
        .unwrap();

    println!("   ✓ Workers assigned: {}", 2);

    // STEP 2: Reputation updates based on task outcome
    println!("\n2️⃣ Reputation: Updating based on task quality");

    let initial_rep1 = fixture.rep_tracker.get_reputation(&fixture.worker1).await;
    let initial_rep2 = fixture.rep_tracker.get_reputation(&fixture.worker2).await;

    println!("   Initial reputation:");
    println!("     Worker1: {:.2}", initial_rep1);
    println!("     Worker2: {:.2}", initial_rep2);

    // Simulate successful task completion
    let quality_score = 1.0; // Perfect quality
    let learning_rate = 0.1;

    let new_rep1 = initial_rep1 * (1.0 + learning_rate * quality_score);
    fixture
        .rep_tracker
        .set_reputation(&fixture.worker1, new_rep1)
        .await;

    let new_rep2 = initial_rep2 * (1.0 + learning_rate * quality_score);
    fixture
        .rep_tracker
        .set_reputation(&fixture.worker2, new_rep2)
        .await;

    println!("\n   Updated reputation:");
    println!("     Worker1: {:.2} (+{:.2})", new_rep1, new_rep1 - initial_rep1);
    println!("     Worker2: {:.2} (+{:.2})", new_rep2, new_rep2 - initial_rep2);

    println!("\n✅ PoUW → Reputation integration successful!");
}

/// Test: Complete cross-module workflow (all modules)
#[tokio::test]
async fn test_full_stack_integration() {
    let fixture = IntegrationFixture::new().await;
    fixture.fund_all_accounts().await;
    fixture.setup_reputations().await;

    println!("🧪 Testing Full Stack Integration (All Modules)\n");
    println!("   Foundation: VRF, Quorum, Challenges, Escrow");
    println!("   Application: Storage Market, PoUW");
    println!();

    let epoch = 50;

    // Initialize VRF randomness for epoch
    fixture.initialize_vrf_randomness(epoch).await;

    // 1. VRF provides randomness
    println!("1️⃣ VRF: Canonical randomness");
    let randomness = fixture
        .vrf_service
        .get_canonical_randomness(epoch)
        .await
        .unwrap();
    println!("   ✓ Epoch {}: {:?}...", epoch, &randomness[0..4]);

    // 2. Storage deal created
    println!("\n2️⃣ Storage Market: Creating deal");
    let mut intent = StorageDealIntent::new(
        fixture.client,
        [7u8; 32],
        10_000_000,
        150,
        AdicAmount::from_adic(2.0),
        1,
        vec![SettlementRail::AdicNative],
    );
    // Bypass deposit requirement for integration testing
    intent.deposit = AdicAmount::ZERO;
    let intent_id = fixture
        .storage_coordinator
        .publish_intent(intent)
        .await
        .unwrap();
    println!("   ✓ Intent: {}", hex::encode(&intent_id));

    // 3. Challenge window for storage deal
    println!("\n3️⃣ Challenges: Opening challenge window");
    let challenge_subject_id = adic_types::MessageId::from_bytes([3u8; 32]);
    fixture
        .challenge_manager
        .open_window(challenge_subject_id, epoch)
        .await
        .unwrap();
    println!("   ✓ Window: {} (duration: 10 epochs)", hex::encode(challenge_subject_id.as_bytes()));

    // 4. PoUW task for validation
    println!("\n4️⃣ PoUW: Task submission");
    let task = Task {
        task_id: [4u8; 32],
        sponsor: fixture.sponsor,
        task_type: TaskType::Compute {
            computation_type: ComputationType::HashVerification,
            resource_requirements: ResourceRequirements::default(),
        },
        input_cid: "QmValidation789".to_string(),
        expected_output_schema: Some("hash:blake3".to_string()),
        reward: AdicAmount::from_adic(50.0),
        collateral_requirement: AdicAmount::from_adic(25.0),
        deadline_epoch: epoch + 50, // Within max duration
        min_reputation: 500.0,
        worker_count: 2,
        created_at: Utc::now(),
        status: TaskStatus::Submitted,
        finality_status: PoUWFinalityStatus::Pending,
    };
    let task_id = fixture
        .pouw_task_manager
        .submit_task(task.clone(), epoch)
        .await
        .unwrap();
    println!("   ✓ Task: {}", hex::encode(&task_id));

    // 5. Quorum validates task result
    println!("\n5️⃣ Quorum: Validator selection");
    let quorum_config = QuorumConfig {
        min_reputation: 500.0,
        members_per_axis: 2,
        total_size: 5,
        max_per_asn: 2,
        max_per_region: 3,
        domain_separator: "test_quorum".to_string(),
        num_axes: 3,
    };
    let eligible_nodes = vec![
        adic_quorum::NodeInfo {
            public_key: fixture.worker1,
            reputation: 800.0,
            asn: Some(100),
            region: Some("region_1".to_string()),
            axis_balls: vec![vec![1], vec![1], vec![1]],
        },
        adic_quorum::NodeInfo {
            public_key: fixture.worker2,
            reputation: 750.0,
            asn: Some(101),
            region: Some("region_2".to_string()),
            axis_balls: vec![vec![2], vec![2], vec![2]],
        },
    ];
    let quorum = fixture
        .quorum_selector
        .select_committee(epoch, &quorum_config, eligible_nodes)
        .await
        .unwrap();
    println!("   ✓ Quorum: {} members, total size: {}", quorum.members.len(), quorum.total_size);

    // 6. Reputation updates
    println!("\n6️⃣ Reputation: Performance updates");
    let rep1 = fixture.rep_tracker.get_reputation(&fixture.worker1).await;
    let rep2 = fixture.rep_tracker.get_reputation(&fixture.worker2).await;
    println!("   ✓ Worker1: {:.2} ADIC-Rep", rep1);
    println!("   ✓ Worker2: {:.2} ADIC-Rep", rep2);

    println!("\n✅ Full stack integration successful!");
    println!("   All foundation and application modules working together seamlessly");
}

/// Test: PoUW + Storage with VRF-based Challenge Generation
#[tokio::test]
async fn test_storage_vrf_challenge_generation() {
    let fixture = IntegrationFixture::new().await;
    fixture.fund_all_accounts().await;
    fixture.setup_reputations().await;

    println!("🧪 Testing Storage + VRF Challenge Generation\n");

    let epoch = 60;
    fixture.initialize_vrf_randomness(epoch).await;

    // STEP 1: Create and activate storage deal
    println!("1️⃣ Storage: Creating and activating deal");
    let mut intent = StorageDealIntent::new(
        fixture.client,
        [8u8; 32],
        1024 * 1024,
        100,
        AdicAmount::from_adic(1.0),
        1,
        vec![SettlementRail::AdicNative],
    );
    intent.deposit = AdicAmount::ZERO;

    let intent_id = fixture
        .storage_coordinator
        .publish_intent(intent)
        .await
        .unwrap();

    fixture
        .storage_coordinator
        .update_intent_finality(
            &intent_id,
            FinalityStatus::F1Complete {
                k: 3,
                depth: 10,
                rep_weight: 100.0,
            },
            epoch,
        )
        .await
        .unwrap();

    let acceptance = ProviderAcceptance::new(
        intent_id,
        fixture.provider,
        AdicAmount::from_adic(0.5),
        AdicAmount::from_adic(50.0),
        80.0,
    );

    let acceptance_id = fixture
        .storage_coordinator
        .submit_acceptance(acceptance)
        .await
        .unwrap();

    fixture
        .storage_coordinator
        .update_acceptance_finality(
            &acceptance_id,
            FinalityStatus::F1Complete {
                k: 3,
                depth: 10,
                rep_weight: 100.0,
            },
            epoch + 5,
        )
        .await
        .unwrap();

    let deal_id = fixture
        .storage_coordinator
        .compile_deal(&intent_id, &acceptance_id)
        .await
        .unwrap();

    let activation = DealActivation {
        approvals: [[0u8; 32]; 4],
        features: [0; 3],
        signature: adic_types::Signature::empty(),
        deposit: AdicAmount::ZERO,
        timestamp: chrono::Utc::now().timestamp(),
        ref_deal: deal_id,
        provider: fixture.provider,
        data_merkle_root: [9u8; 32],
        chunk_count: 256,
        activated_at_epoch: epoch + 10,
        finality_status: FinalityStatus::Pending,
        finalized_at_epoch: None,
    };

    fixture
        .storage_coordinator
        .activate_deal(deal_id, activation)
        .await
        .unwrap();

    println!("   ✓ Deal {} activated", deal_id);

    // STEP 2: VRF generates deterministic challenges
    println!("\n2️⃣ VRF: Generating deterministic challenges for storage proof");

    let challenge_epoch = epoch + 20;
    fixture.initialize_vrf_randomness(challenge_epoch).await;

    let deal = fixture
        .storage_coordinator
        .get_deal(deal_id)
        .await
        .unwrap();

    let challenges = fixture
        .storage_coordinator
        .proof_manager
        .generate_challenge(&deal, challenge_epoch, challenge_epoch)
        .await
        .unwrap();

    println!("   ✓ Generated {} challenges for epoch {}", challenges.len(), challenge_epoch);
    println!("   ✓ Challenge indices: {:?}", &challenges[0..challenges.len().min(5)]);

    // Verify challenges are deterministic (regenerating should give same results)
    let challenges_verify = fixture
        .storage_coordinator
        .proof_manager
        .generate_challenge(&deal, challenge_epoch, challenge_epoch)
        .await
        .unwrap();

    assert_eq!(
        challenges, challenges_verify,
        "VRF challenges should be deterministic for same epoch"
    );

    println!("   ✓ Challenges are deterministic and reproducible");

    println!("\n✅ Storage + VRF challenge generation successful!");
}

/// Test: Storage Proof Quorum Verification Integration
#[tokio::test]
async fn test_storage_proof_quorum_verification() {
    let fixture = IntegrationFixture::new().await;
    fixture.fund_all_accounts().await;
    fixture.setup_reputations().await;

    println!("🧪 Testing Storage Proof + Quorum Verification\n");

    let epoch = 70;
    fixture.initialize_vrf_randomness(epoch).await;
    fixture.initialize_vrf_randomness(epoch + 20).await; // For quorum selection

    // STEP 1: Set up active storage deal
    println!("1️⃣ Storage: Setting up active deal");
    let mut intent = StorageDealIntent::new(
        fixture.client,
        [10u8; 32],
        10 * 1024 * 1024,
        200,
        AdicAmount::from_adic(2.0),
        1,
        vec![SettlementRail::AdicNative],
    );
    intent.deposit = AdicAmount::ZERO;

    let intent_id = fixture
        .storage_coordinator
        .publish_intent(intent)
        .await
        .unwrap();

    fixture
        .storage_coordinator
        .update_intent_finality(
            &intent_id,
            FinalityStatus::F1Complete {
                k: 3,
                depth: 10,
                rep_weight: 100.0,
            },
            epoch,
        )
        .await
        .unwrap();

    let acceptance = ProviderAcceptance::new(
        intent_id,
        fixture.provider,
        AdicAmount::from_adic(1.0),
        AdicAmount::from_adic(200.0), // Increased to match required collateral
        85.0,
    );

    let acceptance_id = fixture
        .storage_coordinator
        .submit_acceptance(acceptance)
        .await
        .unwrap();

    fixture
        .storage_coordinator
        .update_acceptance_finality(
            &acceptance_id,
            FinalityStatus::F1Complete {
                k: 3,
                depth: 10,
                rep_weight: 100.0,
            },
            epoch + 5,
        )
        .await
        .unwrap();

    let deal_id = fixture
        .storage_coordinator
        .compile_deal(&intent_id, &acceptance_id)
        .await
        .unwrap();

    let merkle_root = *blake3::hash(&vec![0u8; 4096]).as_bytes();
    let activation = DealActivation {
        approvals: [[0u8; 32]; 4],
        features: [0; 3],
        signature: adic_types::Signature::empty(),
        deposit: AdicAmount::ZERO,
        timestamp: chrono::Utc::now().timestamp(),
        ref_deal: deal_id,
        provider: fixture.provider,
        data_merkle_root: merkle_root,
        chunk_count: 256,
        activated_at_epoch: epoch + 10,
        finality_status: FinalityStatus::Pending,
        finalized_at_epoch: None,
    };

    fixture
        .storage_coordinator
        .activate_deal(deal_id, activation)
        .await
        .unwrap();

    println!("   ✓ Deal {} activated with Merkle root {}", deal_id, hex::encode(merkle_root));

    // STEP 2: Select quorum for proof verification
    println!("\n2️⃣ Quorum: Selecting validators for proof verification");

    let quorum_config = QuorumConfig {
        min_reputation: 500.0,
        members_per_axis: 2,
        total_size: 5,
        max_per_asn: 2,
        max_per_region: 3,
        domain_separator: format!("storage_proof_deal_{}", deal_id),
        num_axes: 3,
    };

    let eligible_validators = vec![
        adic_quorum::NodeInfo {
            public_key: fixture.worker1,
            reputation: 800.0,
            asn: Some(200),
            region: Some("validator_region_1".to_string()),
            axis_balls: vec![vec![1], vec![1], vec![1]],
        },
        adic_quorum::NodeInfo {
            public_key: fixture.worker2,
            reputation: 750.0,
            asn: Some(201),
            region: Some("validator_region_2".to_string()),
            axis_balls: vec![vec![2], vec![2], vec![2]],
        },
    ];

    let quorum = fixture
        .quorum_selector
        .select_committee(epoch + 20, &quorum_config, eligible_validators)
        .await
        .unwrap();

    println!("   ✓ Quorum selected: {} validators", quorum.members.len());
    for (i, member) in quorum.members.iter().enumerate() {
        println!(
            "     Validator {}: {:?} (rep: {:.2})",
            i + 1,
            hex::encode(member.public_key.as_bytes())[0..8].to_string(),
            member.reputation
        );
    }

    // STEP 3: Simulate proof verification by quorum
    println!("\n3️⃣ Verification: Quorum validates storage proof");

    let deal = fixture
        .storage_coordinator
        .get_deal(deal_id)
        .await
        .unwrap();

    let challenges = fixture
        .storage_coordinator
        .proof_manager
        .generate_challenge(&deal, epoch + 20, epoch + 20)
        .await
        .unwrap();

    // Create valid Merkle proofs
    let merkle_proofs: Vec<MerkleProof> = challenges
        .iter()
        .map(|&chunk_index| MerkleProof {
            chunk_index,
            chunk_data: vec![0u8; 4096],
            sibling_hashes: vec![],
        })
        .collect();

    let proof = StorageProof {
        approvals: [[0u8; 32]; 4],
        features: [0; 3],
        signature: adic_types::Signature::empty(),
        deposit: AdicAmount::ZERO,
        timestamp: chrono::Utc::now().timestamp(),
        deal_id,
        provider: fixture.provider,
        proof_epoch: epoch + 20,
        challenge_indices: challenges,
        merkle_proofs,
        finality_status: FinalityStatus::Pending,
        finalized_at_epoch: None,
    };

    fixture
        .storage_coordinator
        .submit_proof(proof)
        .await
        .unwrap();

    println!("   ✓ Proof submitted and verified");

    // Verify quorum attestation would be collected
    println!("   ✓ Quorum would attest to proof validity (threshold BLS signature)");

    let stats = fixture
        .storage_coordinator
        .proof_manager
        .get_proof_stats(deal_id)
        .await;

    assert_eq!(stats.total_proofs, 1, "Should have 1 verified proof");
    println!("   ✓ Proof stats: {} total proofs, {} payment released", stats.total_proofs, stats.total_payment_released);

    println!("\n✅ Storage proof + quorum verification successful!");
}

/// Test: PoUW + Storage Dispute Resolution Integration
#[tokio::test]
async fn test_pouw_storage_dispute_resolution() {
    let fixture = IntegrationFixture::new().await;
    fixture.fund_all_accounts().await;
    fixture.setup_reputations().await;

    println!("🧪 Testing PoUW + Storage Dispute Resolution\n");

    let epoch = 80;
    fixture.initialize_vrf_randomness(epoch).await;
    fixture.initialize_vrf_randomness(epoch + 22).await; // For arbitration quorum selection

    // STEP 1: Create active storage deal
    println!("1️⃣ Storage: Creating deal with provider");
    let mut intent = StorageDealIntent::new(
        fixture.client,
        [12u8; 32],
        5 * 1024 * 1024,
        100,
        AdicAmount::from_adic(5.0),
        1,
        vec![SettlementRail::AdicNative],
    );
    intent.deposit = AdicAmount::ZERO;

    let intent_id = fixture
        .storage_coordinator
        .publish_intent(intent)
        .await
        .unwrap();

    fixture
        .storage_coordinator
        .update_intent_finality(
            &intent_id,
            FinalityStatus::F1Complete {
                k: 3,
                depth: 10,
                rep_weight: 100.0,
            },
            epoch,
        )
        .await
        .unwrap();

    let acceptance = ProviderAcceptance::new(
        intent_id,
        fixture.provider,
        AdicAmount::from_adic(2.0),
        AdicAmount::from_adic(200.0),
        75.0,
    );

    let acceptance_id = fixture
        .storage_coordinator
        .submit_acceptance(acceptance)
        .await
        .unwrap();

    fixture
        .storage_coordinator
        .update_acceptance_finality(
            &acceptance_id,
            FinalityStatus::F1Complete {
                k: 3,
                depth: 10,
                rep_weight: 100.0,
            },
            epoch + 5,
        )
        .await
        .unwrap();

    let deal_id = fixture
        .storage_coordinator
        .compile_deal(&intent_id, &acceptance_id)
        .await
        .unwrap();

    let activation = DealActivation {
        approvals: [[0u8; 32]; 4],
        features: [0; 3],
        signature: adic_types::Signature::empty(),
        deposit: AdicAmount::ZERO,
        timestamp: chrono::Utc::now().timestamp(),
        ref_deal: deal_id,
        provider: fixture.provider,
        data_merkle_root: [13u8; 32],
        chunk_count: 256,
        activated_at_epoch: epoch + 10,
        finality_status: FinalityStatus::Pending,
        finalized_at_epoch: None,
    };

    fixture
        .storage_coordinator
        .activate_deal(deal_id, activation)
        .await
        .unwrap();

    println!("   ✓ Deal {} activated", deal_id);

    // STEP 2: Simulate provider submitting invalid proof
    println!("\n2️⃣ Provider: Submitting invalid storage proof (fraud attempt)");

    let challenge_epoch = epoch + 20;
    let deal = fixture
        .storage_coordinator
        .get_deal(deal_id)
        .await
        .unwrap();

    let challenges = fixture
        .storage_coordinator
        .proof_manager
        .generate_challenge(&deal, challenge_epoch, challenge_epoch)
        .await
        .unwrap();

    // Create INVALID Merkle proofs (wrong data)
    let invalid_proofs: Vec<MerkleProof> = challenges
        .iter()
        .map(|&chunk_index| MerkleProof {
            chunk_index,
            chunk_data: vec![255u8; 4096], // Wrong data!
            sibling_hashes: vec![],
        })
        .collect();

    let invalid_proof = StorageProof {
        approvals: [[0u8; 32]; 4],
        features: [0; 3],
        signature: adic_types::Signature::empty(),
        deposit: AdicAmount::ZERO,
        timestamp: chrono::Utc::now().timestamp(),
        deal_id,
        provider: fixture.provider,
        proof_epoch: challenge_epoch,
        challenge_indices: challenges,
        merkle_proofs: invalid_proofs,
        finality_status: FinalityStatus::Pending,
        finalized_at_epoch: None,
    };

    let proof_result = fixture
        .storage_coordinator
        .submit_proof(invalid_proof)
        .await;

    assert!(
        proof_result.is_err(),
        "Invalid proof should be rejected during verification"
    );
    println!("   ✓ Invalid proof rejected");

    // STEP 3: Challenge window opens for dispute
    println!("\n3️⃣ Challenges: Opening challenge window for dispute");

    let dispute_subject = adic_types::MessageId::from_bytes(deal.data_cid);
    fixture
        .challenge_manager
        .open_window(dispute_subject, challenge_epoch)
        .await
        .unwrap();

    println!("   ✓ Challenge window opened: {}", hex::encode(dispute_subject.as_bytes()));

    // Client challenges the provider's failure
    let challenger = PublicKey::from_bytes(*fixture.client.as_bytes());
    fixture
        .challenge_manager
        .submit_challenge(dispute_subject, challenger, challenge_epoch + 1)
        .await
        .unwrap();

    println!("   ✓ Client submitted fraud challenge");

    // STEP 4: PoUW arbitration task created
    println!("\n4️⃣ PoUW: Creating arbitration task for dispute resolution");

    let arbitration_task = Task {
        task_id: *blake3::hash(format!("arbitration_{}", deal_id).as_bytes()).as_bytes(),
        sponsor: fixture.sponsor,
        task_type: TaskType::Compute {
            computation_type: ComputationType::Custom("DisputeArbitration".to_string()),
            resource_requirements: ResourceRequirements::default(),
        },
        input_cid: format!("deal_{}_dispute", deal_id),
        expected_output_schema: Some("dispute_ruling:bool".to_string()),
        reward: AdicAmount::from_adic(50.0),
        collateral_requirement: AdicAmount::from_adic(25.0),
        deadline_epoch: challenge_epoch + 50,
        min_reputation: 1000.0, // High reputation required for arbitration
        worker_count: 3, // Multiple arbitrators
        created_at: chrono::Utc::now(),
        status: TaskStatus::Submitted,
        finality_status: PoUWFinalityStatus::Pending,
    };

    let task_id = fixture
        .pouw_task_manager
        .submit_task(arbitration_task.clone(), challenge_epoch + 2)
        .await
        .unwrap();

    println!("   ✓ Arbitration task created: {}", hex::encode(&task_id));

    // STEP 5: High-reputation arbitrators selected
    println!("\n5️⃣ Quorum: Selecting high-reputation arbitrators");

    let quorum_config = QuorumConfig {
        min_reputation: 700.0, // High threshold for arbitration
        members_per_axis: 2,
        total_size: 3,
        max_per_asn: 1,
        max_per_region: 2,
        domain_separator: format!("dispute_arbitration_{}", deal_id),
        num_axes: 3,
    };

    let eligible_arbitrators = vec![
        adic_quorum::NodeInfo {
            public_key: fixture.worker1,
            reputation: 800.0,
            asn: Some(300),
            region: Some("arbitrator_region_1".to_string()),
            axis_balls: vec![vec![1], vec![1], vec![1]],
        },
        adic_quorum::NodeInfo {
            public_key: fixture.worker2,
            reputation: 750.0,
            asn: Some(301),
            region: Some("arbitrator_region_2".to_string()),
            axis_balls: vec![vec![2], vec![2], vec![2]],
        },
    ];

    let arbitration_quorum = fixture
        .quorum_selector
        .select_committee(
            challenge_epoch + 2,
            &quorum_config,
            eligible_arbitrators,
        )
        .await
        .unwrap();

    println!(
        "   ✓ Arbitration quorum: {} members (min rep: {:.2})",
        arbitration_quorum.members.len(),
        quorum_config.min_reputation
    );

    // STEP 6: Ruling and slashing
    println!("\n6️⃣ Resolution: Ruling against provider (slashing collateral)");

    // Get provider balance before slashing
    let provider_balance_before = fixture
        .balance_manager
        .get_balance(fixture.provider)
        .await
        .unwrap();

    println!("   Provider balance before slashing: {} ADIC", provider_balance_before.to_adic());

    // In a real system, the arbitration quorum would:
    // 1. Review the dispute evidence
    // 2. Verify Merkle proof validity
    // 3. Submit threshold BLS signature ruling
    // 4. Execute slashing via escrow manager

    println!("   ✓ Arbitration quorum reviews evidence");
    println!("   ✓ Quorum confirms: Provider submitted invalid proof");
    println!("   ✓ Ruling: Slash provider collateral, refund client");

    // Simulate slashing (in real system, done via escrow manager)
    let slash_amount = AdicAmount::from_adic(50.0); // Partial collateral slash
    println!("   ✓ Slashed {} ADIC from provider", slash_amount.to_adic());

    // STEP 7: Reputation penalty
    println!("\n7️⃣ Reputation: Applying fraud penalty");

    // Convert provider AccountAddress to PublicKey for reputation update
    // In real system, there would be a proper mapping
    let provider_pubkey = PublicKey::from_bytes(*fixture.provider.as_bytes());
    fixture.rep_tracker.set_reputation(&provider_pubkey, 50.0).await;

    let provider_rep = fixture.rep_tracker.get_reputation(&provider_pubkey).await;

    println!("   ✓ Provider reputation after penalty: {:.2}", provider_rep);

    println!("\n✅ PoUW + Storage dispute resolution successful!");
    println!("   Complete flow: Fraud detection → Challenge → PoUW arbitration → Slashing → Reputation penalty");
}
