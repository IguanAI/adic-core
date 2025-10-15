//! End-to-End Integration Tests for Storage Market
//!
//! Tests the complete lifecycle of storage deals from intent publication
//! through proof submission to deal completion.

use adic_challenges::{ChallengeConfig, ChallengeWindowManager};
use adic_consensus::ReputationTracker;
use adic_economics::{AccountAddress, AdicAmount, BalanceManager};
use adic_storage_market::*;
use adic_vrf::VRFService;
use std::sync::Arc;

/// Test fixture for storage market integration tests
struct MarketFixture {
    coordinator: StorageMarketCoordinator,
    client: AccountAddress,
    provider: AccountAddress,
    balance_manager: Arc<BalanceManager>,
}

impl MarketFixture {
    async fn new() -> Self {
        let storage = Arc::new(adic_economics::storage::MemoryStorage::new());
        let balance_manager = Arc::new(BalanceManager::new(storage));

        let rep_tracker = Arc::new(ReputationTracker::new(0.9));

        // Disable reputation checks for tests
        let mut intent_config = IntentConfig::default();
        intent_config.min_client_reputation = 0.0;

        let intent_manager = Arc::new(IntentManager::new(
            intent_config,
            balance_manager.clone(),
            rep_tracker.clone(),
        ));

        let jitca_compiler = Arc::new(JitcaCompiler::new(
            JitcaConfig::default(),
            balance_manager.clone(),
        ));

        let challenge_manager = Arc::new(ChallengeWindowManager::new(
            ChallengeConfig::default(),
        ));
        let vrf_service = Arc::new(VRFService::new(
            Default::default(),
            rep_tracker,
        ));

        let proof_manager = Arc::new(ProofCycleManager::new_for_testing(
            ProofCycleConfig::default(),
            balance_manager.clone(),
            challenge_manager,
            vrf_service,
        ));

        let coordinator = StorageMarketCoordinator::new(
            MarketConfig::default(),
            intent_manager,
            jitca_compiler,
            proof_manager,
            balance_manager.clone(),
        );

        let client = AccountAddress::from_bytes([1u8; 32]);
        let provider = AccountAddress::from_bytes([2u8; 32]);

        Self {
            coordinator,
            client,
            provider,
            balance_manager,
        }
    }

    async fn fund_accounts(&self, client_funds: AdicAmount, provider_funds: AdicAmount) {
        self.balance_manager
            .credit(self.client, client_funds)
            .await
            .unwrap();
        self.balance_manager
            .credit(self.provider, provider_funds)
            .await
            .unwrap();
    }

    async fn create_test_intent(&self) -> StorageDealIntent {
        let mut intent = StorageDealIntent::new(
            self.client,
            [5u8; 32], // data_cid
            1024 * 1024, // 1 MB
            100,       // 100 epochs
            AdicAmount::from_adic(1.0),
            1,
            vec![SettlementRail::AdicNative],
        );
        intent.deposit = AdicAmount::from_adic(0.1);
        intent
    }

    async fn create_test_acceptance(&self, intent_id: [u8; 32]) -> ProviderAcceptance {
        let mut acceptance = ProviderAcceptance::new(
            intent_id,
            self.provider,
            AdicAmount::from_adic(0.5), // Price per epoch
            AdicAmount::from_adic(50.0), // Collateral
            80.0, // Reputation
        );
        acceptance.deposit = AdicAmount::from_adic(0.1); // Set anti-spam deposit
        acceptance
    }
}

/// Helper function to create valid Merkle proofs for testing
fn create_valid_merkle_proof(chunk_index: u64) -> MerkleProof {
    // For testing, use consistent chunk data across all chunks
    // This allows all proofs to verify against the same root
    let chunk_data = vec![0u8; 4096];

    // Single-leaf tree (no siblings needed)
    MerkleProof {
        chunk_index,
        chunk_data,
        sibling_hashes: vec![],
    }
}

/// Compute Merkle root for test consistency
/// All test chunks have the same data, so they all hash to the same value
fn compute_test_merkle_root() -> [u8; 32] {
    // All test chunks are zeros
    let chunk_data = vec![0u8; 4096];
    *blake3::hash(&chunk_data).as_bytes()
}

#[tokio::test]
async fn test_full_deal_lifecycle() {
    let fixture = MarketFixture::new().await;

    // Fund both parties
    fixture
        .fund_accounts(
            AdicAmount::from_adic(100.0),
            AdicAmount::from_adic(100.0),
        )
        .await;

    // 1. Client publishes intent
    let intent = fixture.create_test_intent().await;
    let intent_id = fixture
        .coordinator
        .publish_intent(intent.clone())
        .await
        .unwrap();

    println!("✓ Intent published: {}", hex::encode(&intent_id));

    // 2. Finalize intent (required before acceptance)
    fixture
        .coordinator
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

    println!("✓ Intent finalized");

    // 3. Provider submits acceptance
    let acceptance = fixture.create_test_acceptance(intent_id).await;
    let acceptance_id = fixture
        .coordinator
        .submit_acceptance(acceptance.clone())
        .await
        .unwrap();

    println!("✓ Acceptance submitted: {}", hex::encode(&acceptance_id));

    // 4. Finalize acceptance
    fixture
        .coordinator
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

    println!("✓ Intent and acceptance finalized");

    // 3. Compile deal
    let deal_id = fixture
        .coordinator
        .compile_deal(&intent_id, &acceptance_id)
        .await
        .unwrap();

    println!("✓ Deal compiled: {}", deal_id);

    // Verify deal state
    let deal = fixture.coordinator.get_deal(deal_id).await.unwrap();
    assert_eq!(deal.status, StorageDealStatus::PendingActivation);
    assert_eq!(deal.client, fixture.client);
    assert_eq!(deal.provider, fixture.provider);

    // 4. Activate deal
    let test_merkle_root = compute_test_merkle_root();
    let activation = DealActivation {
        approvals: [[0u8; 32]; 4],
        features: [0; 3],
        signature: adic_types::Signature::empty(),
        deposit: AdicAmount::ZERO,
        timestamp: chrono::Utc::now().timestamp(),
        ref_deal: deal_id,
        provider: fixture.provider,
        data_merkle_root: test_merkle_root,
        chunk_count: 256,
        activated_at_epoch: 20,
        finality_status: FinalityStatus::Pending,
        finalized_at_epoch: None,
    };

    fixture
        .coordinator
        .activate_deal(deal_id, activation)
        .await
        .unwrap();

    println!("✓ Deal activated");

    // Verify deal is now active
    let deal = fixture.coordinator.get_deal(deal_id).await.unwrap();
    assert_eq!(deal.status, StorageDealStatus::Active);
    assert_eq!(deal.proof_merkle_root, Some(test_merkle_root));

    // 5. Submit proofs
    fixture.coordinator.advance_epoch().await; // Epoch 1

    for proof_round in 0..3 {
        fixture.coordinator.advance_epoch().await;
        let epoch = fixture.coordinator.get_current_epoch().await;

        // Generate challenges for this epoch
        let deal = fixture.coordinator.get_deal(deal_id).await.unwrap();
        let challenges = fixture
            .coordinator
            .proof_manager
            .generate_challenge(&deal, epoch, epoch)
            .await
            .unwrap();

        // Provider creates Merkle proofs for challenged chunks
        let merkle_proofs: Vec<MerkleProof> = challenges
            .iter()
            .map(|&chunk_index| create_valid_merkle_proof(chunk_index))
            .collect();

        let proof = StorageProof {
            approvals: [[0u8; 32]; 4],
            features: [0; 3],
            signature: adic_types::Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: chrono::Utc::now().timestamp(),
            deal_id,
            provider: fixture.provider,
            proof_epoch: epoch,
            challenge_indices: challenges,
            merkle_proofs,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
        };

        fixture
            .coordinator
            .submit_proof(proof)
            .await
            .unwrap();

        println!("✓ Proof {} submitted for epoch {}", proof_round + 1, epoch);

        // Check proof stats
        let stats = fixture
            .coordinator
            .proof_manager
            .get_proof_stats(deal_id)
            .await;
        assert_eq!(stats.total_proofs, proof_round + 1);
        assert!(stats.total_payment_released > AdicAmount::ZERO);
    }

    // 6. Complete deal
    fixture.coordinator.advance_epoch().await;
    for _ in 0..100 {
        fixture.coordinator.advance_epoch().await;
    }

    fixture
        .coordinator
        .complete_deal(deal_id)
        .await
        .unwrap();

    println!("✓ Deal completed");

    // Verify final state
    let deal = fixture.coordinator.get_deal(deal_id).await.unwrap();
    assert_eq!(deal.status, StorageDealStatus::Completed);

    // Check market stats
    let stats = fixture.coordinator.get_market_stats().await;
    assert_eq!(stats.total_intents, 1);
    assert_eq!(stats.total_acceptances, 1);
    assert_eq!(stats.compiled_deals, 1);
    assert_eq!(stats.completed_deals, 1);

    println!("\n✅ Full deal lifecycle completed successfully!");
}

#[tokio::test]
async fn test_concurrent_deals() {
    let fixture = MarketFixture::new().await;

    // Fund accounts generously for multiple deals
    fixture
        .fund_accounts(
            AdicAmount::from_adic(1000.0),
            AdicAmount::from_adic(1000.0),
        )
        .await;

    let mut deal_ids = Vec::new();

    // Create 5 concurrent deals
    for i in 0..5 {
        // Add delay to ensure unique timestamps for all messages
        if i > 0 {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }

        // 1. Publish intent
        let mut intent = fixture.create_test_intent().await;
        intent.data_cid[0] = i as u8; // Make unique
        let intent_id = fixture.coordinator.publish_intent(intent).await.unwrap();

        // 2. Finalize intent
        fixture
            .coordinator
            .update_intent_finality(
                &intent_id,
                FinalityStatus::F1Complete {
                    k: 3,
                    depth: 10,
                    rep_weight: 100.0,
                },
                10 + i,
            )
            .await
            .unwrap();

        // 3. Submit acceptance
        let acceptance = fixture.create_test_acceptance(intent_id).await;
        let acceptance_id = fixture
            .coordinator
            .submit_acceptance(acceptance)
            .await
            .unwrap();

        // 4. Finalize acceptance
        fixture
            .coordinator
            .update_acceptance_finality(
                &acceptance_id,
                FinalityStatus::F1Complete {
                    k: 3,
                    depth: 10,
                    rep_weight: 100.0,
                },
                15 + i,
            )
            .await
            .unwrap();

        // 5. Compile
        let deal_id = fixture
            .coordinator
            .compile_deal(&intent_id, &acceptance_id)
            .await
            .unwrap();

        deal_ids.push(deal_id);
        println!("✓ Deal {} compiled (ID: {})", i + 1, deal_id);
    }

    // Activate all deals
    let test_merkle_root = compute_test_merkle_root();
    for (i, &deal_id) in deal_ids.iter().enumerate() {
        let activation = DealActivation {
            approvals: [[0u8; 32]; 4],
            features: [0; 3],
            signature: adic_types::Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: chrono::Utc::now().timestamp(),
            ref_deal: deal_id,
            provider: fixture.provider,
            data_merkle_root: test_merkle_root,
            chunk_count: 256,
            activated_at_epoch: 20 + i as u64,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
        };

        fixture
            .coordinator
            .activate_deal(deal_id, activation)
            .await
            .unwrap();

        println!("✓ Deal {} activated", deal_id);
    }

    // Verify all deals are active
    let stats = fixture.coordinator.get_market_stats().await;
    assert_eq!(stats.active_deals, 5);
    assert_eq!(stats.compiled_deals, 5);

    println!("\n✅ {} concurrent deals managed successfully!", deal_ids.len());
}

#[tokio::test]
async fn test_missed_proof_slashing() {
    let fixture = MarketFixture::new().await;

    // Fund accounts
    fixture
        .fund_accounts(
            AdicAmount::from_adic(100.0),
            AdicAmount::from_adic(100.0),
        )
        .await;

    // Create and activate a deal
    let intent = fixture.create_test_intent().await;
    let intent_id = fixture.coordinator.publish_intent(intent).await.unwrap();

    // Finalize intent first
    fixture
        .coordinator
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

    let acceptance = fixture.create_test_acceptance(intent_id).await;
    let acceptance_id = fixture
        .coordinator
        .submit_acceptance(acceptance)
        .await
        .unwrap();

    fixture
        .coordinator
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
        .coordinator
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
        data_merkle_root: [7u8; 32],
        chunk_count: 256,
        activated_at_epoch: 20,
        finality_status: FinalityStatus::Pending,
        finalized_at_epoch: None,
    };

    fixture
        .coordinator
        .activate_deal(deal_id, activation)
        .await
        .unwrap();

    println!("✓ Deal activated");

    // Advance many epochs without submitting proofs
    // This should trigger slashing after grace period
    for _ in 0..200 {
        fixture.coordinator.advance_epoch().await;
        fixture
            .coordinator
            .check_proof_deadlines()
            .await
            .unwrap();
    }

    // Deal should be marked as failed
    let deal = fixture.coordinator.get_deal(deal_id).await.unwrap();
    assert_eq!(
        deal.status,
        StorageDealStatus::Failed,
        "Deal should be slashed for missed proofs"
    );

    println!("✅ Slashing triggered correctly for missed proofs");
}

#[tokio::test]
async fn test_insufficient_funds() {
    let fixture = MarketFixture::new().await;

    // Fund client with insufficient balance
    fixture
        .fund_accounts(
            AdicAmount::from_adic(0.01), // Not enough for deposit
            AdicAmount::from_adic(100.0),
        )
        .await;

    let intent = fixture.create_test_intent().await;

    // Should fail due to insufficient funds
    let result = fixture.coordinator.publish_intent(intent).await;
    assert!(result.is_err());

    println!("✅ Insufficient funds correctly rejected");
}

#[tokio::test]
async fn test_market_statistics() {
    let fixture = MarketFixture::new().await;

    // Initial stats should be zero
    let stats = fixture.coordinator.get_market_stats().await;
    assert_eq!(stats.total_intents, 0);
    assert_eq!(stats.compiled_deals, 0);
    assert_eq!(stats.active_deals, 0);

    // Fund accounts
    fixture
        .fund_accounts(
            AdicAmount::from_adic(1000.0),
            AdicAmount::from_adic(1000.0),
        )
        .await;

    // Create 3 intents with delays to ensure unique timestamps
    for i in 0..3 {
        // Add delay BEFORE creating intent to ensure unique timestamp
        if i > 0 {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }

        let mut intent = fixture.create_test_intent().await;
        intent.data_cid[0] = i as u8;

        let intent_id = fixture.coordinator.publish_intent(intent).await.unwrap();
        println!("✓ Intent {} published: {}", i + 1, hex::encode(&intent_id));
    }

    let stats = fixture.coordinator.get_market_stats().await;
    println!("Total intents: {}, expected: 3", stats.total_intents);
    assert_eq!(stats.total_intents, 3);
    assert_eq!(stats.compiled_deals, 0);

    println!("✅ Market statistics tracking works correctly");
}

#[tokio::test]
async fn test_deal_queries() {
    let fixture = MarketFixture::new().await;

    fixture
        .fund_accounts(
            AdicAmount::from_adic(1000.0),
            AdicAmount::from_adic(1000.0),
        )
        .await;

    // Create 2 deals for the client
    let mut deal_ids = Vec::new();
    for i in 0..2 {
        let mut intent = fixture.create_test_intent().await;
        intent.data_cid[0] = i as u8;
        let intent_id = fixture.coordinator.publish_intent(intent).await.unwrap();

        fixture
            .coordinator
            .update_intent_finality(
                &intent_id,
                FinalityStatus::F1Complete {
                    k: 3,
                    depth: 10,
                    rep_weight: 100.0,
                },
                10 + i,
            )
            .await
            .unwrap();

        let acceptance = fixture.create_test_acceptance(intent_id).await;
        let acceptance_id = fixture
            .coordinator
            .submit_acceptance(acceptance)
            .await
            .unwrap();

        fixture
            .coordinator
            .update_acceptance_finality(
                &acceptance_id,
                FinalityStatus::F1Complete {
                    k: 3,
                    depth: 10,
                    rep_weight: 100.0,
                },
                15 + i,
            )
            .await
            .unwrap();

        let deal_id = fixture
            .coordinator
            .compile_deal(&intent_id, &acceptance_id)
            .await
            .unwrap();
        deal_ids.push(deal_id);
    }

    // Query client deals
    let client_deals = fixture
        .coordinator
        .get_client_deals(fixture.client)
        .await;
    assert_eq!(client_deals.len(), 2);

    // Query provider deals
    let provider_deals = fixture
        .coordinator
        .get_provider_deals(fixture.provider)
        .await;
    assert_eq!(provider_deals.len(), 2);

    println!("✅ Deal queries work correctly");
}

#[tokio::test]
async fn test_challenge_window_creation() {
    println!("\n🧪 Testing challenge window creation on proof submission...");

    // Create basic fixture
    let fixture = MarketFixture::new().await;
    fixture
        .fund_accounts(
            AdicAmount::from_adic(1000.0),
            AdicAmount::from_adic(500.0),
        )
        .await;

    // Create and publish intent
    let intent = fixture.create_test_intent().await;
    let intent_id = fixture
        .coordinator
        .publish_intent(intent.clone())
        .await
        .unwrap();

    // Finalize intent
    fixture
        .coordinator
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

    // Create acceptance
    let acceptance = fixture.create_test_acceptance(intent_id).await;

    let acceptance_id = fixture
        .coordinator
        .submit_acceptance(acceptance)
        .await
        .unwrap();

    // Finalize acceptance
    fixture
        .coordinator
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

    // Compile deal
    let deal_id = fixture
        .coordinator
        .compile_deal(&intent_id, &acceptance_id)
        .await
        .unwrap();

    // Activate deal with proper Merkle root
    let merkle_root = compute_test_merkle_root();
    let activation = DealActivation {
        approvals: [[0u8; 32]; 4],
        features: [0; 3],
        signature: adic_types::Signature::empty(),
        deposit: AdicAmount::ZERO,
        timestamp: chrono::Utc::now().timestamp(),
        ref_deal: deal_id,
        provider: fixture.provider,
        data_merkle_root: merkle_root,
        chunk_count: 256, // Must match challenge range
        activated_at_epoch: 1,
        finality_status: FinalityStatus::Pending,
        finalized_at_epoch: None,
    };

    fixture
        .coordinator
        .activate_deal(deal_id, activation)
        .await
        .unwrap();

    // Get the deal to generate challenges
    let deal = fixture.coordinator.get_deal(deal_id).await.unwrap();

    // Generate challenge using the deal
    let current_epoch = 1;
    let challenge_indices = fixture
        .coordinator
        .proof_manager
        .generate_challenge(&deal, current_epoch, current_epoch)
        .await
        .unwrap();

    println!("📊 Challenge indices generated: {} indices", challenge_indices.len());

    // Build valid Merkle proofs using test helper
    let merkle_proofs: Vec<MerkleProof> = challenge_indices
        .iter()
        .map(|&idx| create_valid_merkle_proof(idx))
        .collect();

    // Create storage proof
    let proof = StorageProof {
        approvals: [[0u8; 32]; 4],
        features: [0; 3],
        signature: adic_types::Signature::empty(),
        deposit: AdicAmount::ZERO,
        timestamp: chrono::Utc::now().timestamp(),
        deal_id,
        provider: fixture.provider,
        proof_epoch: current_epoch,
        challenge_indices: challenge_indices.clone(),
        merkle_proofs,
        finality_status: FinalityStatus::Pending,
        finalized_at_epoch: None,
    };

    println!("📤 Submitting storage proof...");

    // Submit proof - this should open a challenge window
    fixture.coordinator.submit_proof(proof.clone()).await.unwrap();

    println!("✅ Proof submitted successfully");

    // Verify we can query the challenge window
    let window = fixture
        .coordinator
        .proof_manager
        .get_proof_challenge_window(&proof, deal_id)
        .await;

    assert!(window.is_some(), "Challenge window should exist for proof");

    let window = window.unwrap();
    println!("⏳ Challenge window created:");
    println!("   Subject ID: {}", window.metadata.id);
    println!("   Status: {:?}", window.metadata.status);
    println!("   Expiry epoch: {}", window.metadata.window_expiry_epoch);

    // Verify window is active
    assert!(
        window.is_active(current_epoch),
        "Challenge window should be active"
    );

    // Test window expiry processing
    let expired_epoch = window.metadata.window_expiry_epoch + 1;
    let finalized_count = fixture
        .coordinator
        .proof_manager
        .process_expired_windows(expired_epoch)
        .await
        .unwrap();

    assert_eq!(
        finalized_count, 1,
        "Should have finalized 1 expired window"
    );

    println!("✅ Challenge window lifecycle complete");
}
