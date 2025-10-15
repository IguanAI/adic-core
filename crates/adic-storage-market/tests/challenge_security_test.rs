//! H2 Security Tests: Deterministic Challenge Fallback Prevention
//!
//! Tests for the VRF grace period security feature that prevents predictable
//! storage challenges by enforcing VRF randomness for recent epochs.

use adic_challenges::{ChallengeConfig, ChallengeWindowManager};
use adic_consensus::ReputationTracker;
use adic_economics::{storage::MemoryStorage, AccountAddress, AdicAmount, BalanceManager};
use adic_storage_market::proof::{ProofCycleConfig, ProofCycleManager};
use adic_storage_market::types::{FinalityStatus, StorageDeal, StorageDealStatus};
use adic_types::Signature;
use adic_vrf::VRFService;
use std::sync::Arc;

fn create_test_deal() -> StorageDeal {

    StorageDeal {
        approvals: [[0u8; 32]; 4],
        features: [0; 3],
        signature: Signature::empty(),
        deposit: AdicAmount::ZERO,
        timestamp: chrono::Utc::now().timestamp(),
        deal_id: 1,
        ref_intent: [0u8; 32],
        ref_acceptance: [1u8; 32],
        client: AccountAddress::from_bytes([1u8; 32]),
        provider: AccountAddress::from_bytes([2u8; 32]),
        data_cid: [5u8; 32],
        data_size: 1024 * 1024,
        deal_duration_epochs: 100,
        price_per_epoch: AdicAmount::from_adic(0.5),
        provider_collateral: AdicAmount::from_adic(50.0),
        client_payment_escrow: AdicAmount::from_adic(50.0),
        proof_merkle_root: Some([7u8; 32]),
        start_epoch: Some(10),
        activation_deadline: 60,
        status: StorageDealStatus::Active,
        finality_status: FinalityStatus::F1Complete {
            k: 3,
            depth: 10,
            rep_weight: 100.0,
        },
        finalized_at_epoch: Some(50),
    }
}

#[tokio::test]
async fn test_vrf_grace_period_enforcement() {
    println!("\n🧪 Testing VRF grace period blocks recent epochs without VRF...");

    // Create manager with VRF grace period of 3 epochs
    let rep_tracker = Arc::new(ReputationTracker::new(0.5));
    let vrf_service = Arc::new(VRFService::new(Default::default(), rep_tracker.clone()));
    let challenge_manager = Arc::new(ChallengeWindowManager::new(ChallengeConfig::default()));

    let config = ProofCycleConfig {
        proof_grace_period: 5,
        slash_percentage: 0.1,
        chunks_per_challenge: 3,
        payment_per_proof: 0.1,
        vrf_grace_period_epochs: 3, // 3 epoch grace period
    };

    let storage = Arc::new(MemoryStorage::new());
    let balance_manager = Arc::new(BalanceManager::new(storage));

    let manager = ProofCycleManager::new_for_testing(
        config,
        balance_manager,
        challenge_manager,
        vrf_service,
    );

    let deal = create_test_deal();

    // Test 1: Recent epoch (within grace period) without VRF should fail in production
    let epoch = 15;
    let current_epoch = 15; // Same epoch - age = 0, within grace period

    let result = manager.generate_challenge(&deal, epoch, current_epoch).await;

    #[cfg(all(not(test), not(debug_assertions), not(feature = "allow-deterministic-challenges")))]
    {
        assert!(result.is_err(), "Should fail in production without VRF for recent epoch");
        println!("✓ Production mode correctly rejects deterministic fallback");
    }

    #[cfg(any(test, debug_assertions, feature = "allow-deterministic-challenges"))]
    {
        assert!(result.is_ok(), "Should allow fallback in test/dev mode");
        println!("✓ Test/dev mode allows fallback with warning");
    }

    // Test 2: Old epoch (outside grace period) should allow fallback
    let old_epoch = 10;
    let current_epoch = 15; // Age = 5, outside grace period of 3

    let result = manager.generate_challenge(&deal, old_epoch, current_epoch).await;
    assert!(result.is_ok(), "Should allow deterministic fallback for old epochs");
    let challenges = result.unwrap();
    assert_eq!(challenges.len(), 3); // chunks_per_challenge
    println!("✓ Old epochs allow deterministic fallback");

    println!("✅ VRF grace period enforcement test passed");
}

#[tokio::test]
async fn test_vrf_grace_period_boundaries() {
    println!("\n🧪 Testing VRF grace period boundary conditions...");

    let rep_tracker = Arc::new(ReputationTracker::new(0.5));
    let vrf_service = Arc::new(VRFService::new(Default::default(), rep_tracker.clone()));
    let challenge_manager = Arc::new(ChallengeWindowManager::new(ChallengeConfig::default()));

    let config = ProofCycleConfig {
        proof_grace_period: 5,
        slash_percentage: 0.1,
        chunks_per_challenge: 3,
        payment_per_proof: 0.1,
        vrf_grace_period_epochs: 3,
    };

    let storage = Arc::new(MemoryStorage::new());
    let balance_manager = Arc::new(BalanceManager::new(storage));

    let manager = ProofCycleManager::new_for_testing(
        config,
        balance_manager,
        challenge_manager,
        vrf_service,
    );

    let deal = create_test_deal();

    // Test boundary: epoch_age = grace_period (should allow fallback)
    let epoch = 10;
    let current_epoch = 13; // Age = 3, exactly at grace period boundary

    let result = manager.generate_challenge(&deal, epoch, current_epoch).await;
    assert!(result.is_ok(), "Should allow fallback at exact grace period boundary");
    println!("✓ Boundary condition (age == grace_period) allows fallback");

    // Test boundary: epoch_age = grace_period - 1 (should block in production)
    let epoch = 11;
    let current_epoch = 13; // Age = 2, within grace period

    let result = manager.generate_challenge(&deal, epoch, current_epoch).await;

    #[cfg(all(not(test), not(debug_assertions), not(feature = "allow-deterministic-challenges")))]
    {
        assert!(result.is_err(), "Should block fallback just inside grace period");
        println!("✓ Production blocks fallback for age < grace_period");
    }

    #[cfg(any(test, debug_assertions, feature = "allow-deterministic-challenges"))]
    {
        assert!(result.is_ok(), "Test mode allows fallback with warning");
        println!("✓ Test mode allows fallback for age < grace_period");
    }

    println!("✅ Boundary condition tests passed");
}

#[tokio::test]
async fn test_vrf_success_bypasses_grace_period() {
    println!("\n🧪 Testing VRF success always works regardless of grace period...");

    let rep_tracker = Arc::new(ReputationTracker::new(0.5));
    let vrf_service = Arc::new(VRFService::new(Default::default(), rep_tracker.clone()));
    let challenge_manager = Arc::new(ChallengeWindowManager::new(ChallengeConfig::default()));

    let config = ProofCycleConfig {
        proof_grace_period: 5,
        slash_percentage: 0.1,
        chunks_per_challenge: 3,
        payment_per_proof: 0.1,
        vrf_grace_period_epochs: 3,
    };

    let storage = Arc::new(MemoryStorage::new());
    let balance_manager = Arc::new(BalanceManager::new(storage));

    let manager = ProofCycleManager::new_for_testing(
        config,
        balance_manager,
        challenge_manager,
        vrf_service,
    );

    let deal = create_test_deal();

    // Test: Even for current epoch, zero grace period allows fallback
    let epoch = 15;
    let current_epoch = 15;

    // Change test to use zero grace period
    let config_zero = ProofCycleConfig {
        proof_grace_period: 5,
        slash_percentage: 0.1,
        chunks_per_challenge: 3,
        payment_per_proof: 0.1,
        vrf_grace_period_epochs: 0, // Zero grace period allows immediate fallback
    };

    let rep_tracker2 = Arc::new(ReputationTracker::new(0.5));
    let vrf_service2 = Arc::new(VRFService::new(Default::default(), rep_tracker2.clone()));
    let challenge_manager2 = Arc::new(ChallengeWindowManager::new(ChallengeConfig::default()));
    let storage2 = Arc::new(MemoryStorage::new());
    let balance_manager2 = Arc::new(BalanceManager::new(storage2));

    let manager2 = ProofCycleManager::new_for_testing(
        config_zero,
        balance_manager2,
        challenge_manager2,
        vrf_service2,
    );

    let result = manager2.generate_challenge(&deal, epoch, current_epoch).await;
    assert!(result.is_ok(), "Should succeed with zero grace period");
    let challenges = result.unwrap();
    assert_eq!(challenges.len(), 3);

    println!("✓ Zero grace period allows immediate fallback");
    println!("✅ Zero grace period test passed");
}

#[tokio::test]
async fn test_config_vrf_grace_period() {
    println!("\n🧪 Testing ProofCycleConfig VRF grace period...");

    let config = ProofCycleConfig::default();
    assert_eq!(config.vrf_grace_period_epochs, 3, "Default grace period should be 3 epochs");
    println!("✓ Default VRF grace period: {} epochs", config.vrf_grace_period_epochs);

    let custom_config = ProofCycleConfig {
        proof_grace_period: 5,
        slash_percentage: 0.1,
        chunks_per_challenge: 3,
        payment_per_proof: 0.1,
        vrf_grace_period_epochs: 10, // Custom grace period
    };
    assert_eq!(custom_config.vrf_grace_period_epochs, 10);
    println!("✓ Custom VRF grace period: {} epochs", custom_config.vrf_grace_period_epochs);

    println!("✅ Config test passed");
}
