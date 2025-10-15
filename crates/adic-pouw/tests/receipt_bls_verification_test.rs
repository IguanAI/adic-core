//! Integration tests for PoUW receipt BLS signature verification
//!
//! Tests the complete receipt validation pipeline with BLS threshold signatures:
//! - Receipt creation with BLS signatures
//! - BLS signature verification
//! - Receipt validation with and without signatures
//! - Enforcement modes (optional vs required)

use adic_challenges::{AdjudicationConfig, DisputeAdjudicator};
use adic_consensus::ReputationTracker;
use adic_crypto::{BLSThresholdSigner, ThresholdConfig};
use adic_pouw::{ChunkAcceptance, ChunkRejection, PoUWReceipt, ReceiptValidator};
use adic_quorum::QuorumSelector;
use adic_types::PublicKey;
use adic_vrf::{VRFConfig, VRFService};
use std::sync::Arc;

/// Helper to create a minimal adjudicator for testing
fn create_test_adjudicator() -> Arc<DisputeAdjudicator> {
    let rep_tracker = Arc::new(ReputationTracker::new(0.8));
    let vrf_config = VRFConfig::default();
    let vrf_service = Arc::new(VRFService::new(vrf_config, rep_tracker.clone()));
    let quorum_selector = Arc::new(QuorumSelector::new(vrf_service, rep_tracker));

    let adjudicator_config = AdjudicationConfig::default();
    Arc::new(DisputeAdjudicator::new(quorum_selector, adjudicator_config))
}

/// Helper to create a mock receipt without BLS signature
fn create_mock_receipt(epoch: u64, accepted_count: usize) -> PoUWReceipt {
    let mut accepted = Vec::new();
    for i in 0..accepted_count {
        accepted.push(ChunkAcceptance {
            chunk_id: i as u64,
            worker_pk: PublicKey::from_bytes([i as u8; 32]),
            output_hash: [i as u8; 32],
        });
    }

    PoUWReceipt {
        version: 1,
        hook_id: [1u8; 32],
        epoch_id: epoch,
        receipt_seq: 0,
        prev_receipt_hash: [0u8; 32],
        accepted,
        rejected: vec![],
        agg_pk: vec![],  // Empty - no signature
        sig_qk: None,    // No signature
    }
}

/// Helper to create a receipt with BLS signature
fn create_signed_receipt(
    epoch: u64,
    accepted_count: usize,
    signer: &BLSThresholdSigner,
    secret_keys: &[threshold_crypto::SecretKeyShare],
    pub_key_set: &threshold_crypto::PublicKeySet,
) -> PoUWReceipt {
    use adic_pouw::receipt_validator::DST_POUW_RECEIPT;

    let mut receipt = create_mock_receipt(epoch, accepted_count);

    // Build the canonical message for signing
    let mut message = Vec::new();
    message.extend_from_slice(&receipt.hook_id);
    message.extend_from_slice(&receipt.epoch_id.to_le_bytes());
    message.extend_from_slice(&(receipt.accepted.len() as u32).to_le_bytes());
    message.extend_from_slice(&(receipt.rejected.len() as u32).to_le_bytes());

    for acceptance in &receipt.accepted {
        message.extend_from_slice(&acceptance.output_hash);
        message.extend_from_slice(acceptance.worker_pk.as_bytes());
    }

    // Sign with threshold (7 of 10 shares)
    let threshold = 7;
    let mut shares = Vec::new();
    for (i, sk) in secret_keys.iter().take(threshold).enumerate() {
        let share = signer
            .sign_share(sk, i, &message, DST_POUW_RECEIPT)
            .expect("Sign share should succeed");
        shares.push(share);
    }

    // Aggregate shares
    let bls_sig = signer
        .aggregate_shares(pub_key_set, &shares)
        .expect("Aggregation should succeed");

    // Serialize public key set
    let agg_pk = bincode::serialize(pub_key_set).expect("Serialization should succeed");

    receipt.agg_pk = agg_pk;
    receipt.sig_qk = Some(bls_sig);

    receipt
}

#[tokio::test]
async fn test_receipt_without_bls_signature() {
    // Create validator without BLS enforcement
    let adjudicator = create_test_adjudicator();
    let validator = ReceiptValidator::new(adjudicator, 10);

    // Create receipt without signature
    let receipt = create_mock_receipt(100, 5);

    // Validate - should succeed (no enforcement)
    let result = validator.validate_receipt(&receipt, 100).await;

    assert!(result.is_valid, "Receipt should be valid without BLS enforcement");
    assert_eq!(result.validation_status, adic_pouw::receipt_validator::ValidationStatus::PendingChallenge);

    println!("✅ Receipt validation succeeds without BLS signature (no enforcement)");
}

#[tokio::test]
async fn test_receipt_with_valid_bls_signature() {
    // Setup BLS infrastructure
    let threshold_config = ThresholdConfig::new(10, 7).expect("Valid threshold config");
    let (pub_key_set, secret_keys) = adic_crypto::generate_threshold_keys(10, 7)
        .expect("Key generation should succeed");

    let signer = Arc::new(BLSThresholdSigner::new(threshold_config));

    // Create validator with BLS verification
    let adjudicator = create_test_adjudicator();
    let validator = ReceiptValidator::new(adjudicator, 10)
        .with_bls_verification(signer.clone(), false);

    // Create receipt with valid BLS signature
    let receipt = create_signed_receipt(100, 5, &signer, &secret_keys, &pub_key_set);

    // Validate - should succeed
    let result = validator.validate_receipt(&receipt, 100).await;

    assert!(result.is_valid, "Receipt with valid BLS signature should be valid");
    assert_eq!(result.validation_status, adic_pouw::receipt_validator::ValidationStatus::PendingChallenge);

    println!("✅ Receipt validation succeeds with valid BLS signature");
}

#[tokio::test]
async fn test_receipt_with_invalid_bls_signature() {
    // Setup BLS infrastructure
    let threshold_config = ThresholdConfig::new(10, 7).expect("Valid threshold config");
    let (pub_key_set, secret_keys) = adic_crypto::generate_threshold_keys(10, 7)
        .expect("Key generation should succeed");

    let signer = Arc::new(BLSThresholdSigner::new(threshold_config));

    // Create validator with BLS verification (NO ENFORCEMENT)
    let adjudicator = create_test_adjudicator();
    let validator = ReceiptValidator::new(adjudicator, 10)
        .with_bls_verification(signer.clone(), false);

    // Create receipt with valid signature
    let mut receipt = create_signed_receipt(100, 5, &signer, &secret_keys, &pub_key_set);

    // Tamper with the receipt data (invalidating the signature)
    receipt.epoch_id = 999; // Different epoch

    // Validate - should still pass because enforcement is disabled
    let result = validator.validate_receipt(&receipt, 100).await;

    assert!(result.is_valid, "Receipt should be valid without enforcement even if signature is invalid");

    println!("✅ Receipt validation succeeds even with invalid BLS signature (no enforcement)");
}

#[tokio::test]
async fn test_receipt_missing_signature_with_enforcement() {
    // Setup BLS infrastructure
    let threshold_config = ThresholdConfig::new(10, 7).expect("Valid threshold config");
    let signer = Arc::new(BLSThresholdSigner::new(threshold_config));

    // Create validator with BLS ENFORCEMENT enabled
    let adjudicator = create_test_adjudicator();
    let validator = ReceiptValidator::new(adjudicator, 10)
        .with_bls_verification(signer.clone(), true); // ENFORCE

    // Create receipt without signature
    let receipt = create_mock_receipt(100, 5);

    // Validate - should FAIL due to missing signature
    let result = validator.validate_receipt(&receipt, 100).await;

    assert!(!result.is_valid, "Receipt should be invalid when BLS enforcement is enabled and signature is missing");
    assert_eq!(result.validation_status, adic_pouw::receipt_validator::ValidationStatus::Invalid);

    println!("✅ Receipt validation fails without BLS signature when enforcement is enabled");
}

#[tokio::test]
async fn test_receipt_invalid_signature_with_enforcement() {
    // Setup BLS infrastructure
    let threshold_config = ThresholdConfig::new(10, 7).expect("Valid threshold config");
    let (pub_key_set, secret_keys) = adic_crypto::generate_threshold_keys(10, 7)
        .expect("Key generation should succeed");

    let signer = Arc::new(BLSThresholdSigner::new(threshold_config));

    // Create validator with BLS ENFORCEMENT enabled
    let adjudicator = create_test_adjudicator();
    let validator = ReceiptValidator::new(adjudicator, 10)
        .with_bls_verification(signer.clone(), true); // ENFORCE

    // Create receipt with valid signature
    let mut receipt = create_signed_receipt(100, 5, &signer, &secret_keys, &pub_key_set);

    // Tamper with the receipt data (invalidating the signature)
    receipt.epoch_id = 999;

    // Validate - should FAIL due to invalid signature
    let result = validator.validate_receipt(&receipt, 100).await;

    assert!(!result.is_valid, "Receipt should be invalid when signature verification fails and enforcement is enabled");
    assert_eq!(result.validation_status, adic_pouw::receipt_validator::ValidationStatus::Invalid);

    println!("✅ Receipt validation fails with invalid BLS signature when enforcement is enabled");
}

#[tokio::test]
async fn test_bls_signature_verification_method() {
    // Setup BLS infrastructure
    let threshold_config = ThresholdConfig::new(10, 7).expect("Valid threshold config");
    let (pub_key_set, secret_keys) = adic_crypto::generate_threshold_keys(10, 7)
        .expect("Key generation should succeed");

    let signer = Arc::new(BLSThresholdSigner::new(threshold_config));

    // Create validator
    let adjudicator = create_test_adjudicator();
    let validator = ReceiptValidator::new(adjudicator, 10)
        .with_bls_verification(signer.clone(), false);

    // Test 1: Valid signature
    let receipt1 = create_signed_receipt(100, 5, &signer, &secret_keys, &pub_key_set);
    let verify_result1 = validator.verify_bls_signature(&receipt1);
    assert!(verify_result1.is_ok(), "Verification should succeed for valid signature");
    assert_eq!(verify_result1.unwrap(), true, "Valid signature should return true");

    // Test 2: No signature
    let receipt2 = create_mock_receipt(100, 5);
    let verify_result2 = validator.verify_bls_signature(&receipt2);
    assert!(verify_result2.is_ok(), "Verification should succeed (returns false) for no signature");
    assert_eq!(verify_result2.unwrap(), false, "No signature should return false");

    // Test 3: Invalid signature
    let mut receipt3 = create_signed_receipt(100, 5, &signer, &secret_keys, &pub_key_set);
    receipt3.epoch_id = 999; // Tamper
    let verify_result3 = validator.verify_bls_signature(&receipt3);
    assert!(verify_result3.is_err(), "Verification should fail for invalid signature");

    println!("✅ BLS signature verification method works correctly");
}

#[tokio::test]
async fn test_receipt_with_rejections() {
    // Setup BLS infrastructure
    let threshold_config = ThresholdConfig::new(10, 7).expect("Valid threshold config");
    let (pub_key_set, secret_keys) = adic_crypto::generate_threshold_keys(10, 7)
        .expect("Key generation should succeed");

    let signer = Arc::new(BLSThresholdSigner::new(threshold_config));

    // Create receipt with both acceptances and rejections
    let mut receipt = create_mock_receipt(100, 3);
    receipt.rejected = vec![
        ChunkRejection {
            chunk_id: 10,
            worker_pk: PublicKey::from_bytes([10u8; 32]),
            rejection_reason: "Invalid output".to_string(),
        },
        ChunkRejection {
            chunk_id: 11,
            worker_pk: PublicKey::from_bytes([11u8; 32]),
            rejection_reason: "Timeout".to_string(),
        },
    ];

    // Sign the receipt (including rejections in message)
    use adic_pouw::receipt_validator::DST_POUW_RECEIPT;

    let mut message = Vec::new();
    message.extend_from_slice(&receipt.hook_id);
    message.extend_from_slice(&receipt.epoch_id.to_le_bytes());
    message.extend_from_slice(&(receipt.accepted.len() as u32).to_le_bytes());
    message.extend_from_slice(&(receipt.rejected.len() as u32).to_le_bytes());

    for acceptance in &receipt.accepted {
        message.extend_from_slice(&acceptance.output_hash);
        message.extend_from_slice(acceptance.worker_pk.as_bytes());
    }

    for rejection in &receipt.rejected {
        message.extend_from_slice(&rejection.chunk_id.to_le_bytes());
        message.extend_from_slice(rejection.worker_pk.as_bytes());
    }

    // Sign with threshold
    let threshold = 7;
    let mut shares = Vec::new();
    for (i, sk) in secret_keys.iter().take(threshold).enumerate() {
        let share = signer
            .sign_share(sk, i, &message, DST_POUW_RECEIPT)
            .expect("Sign share should succeed");
        shares.push(share);
    }

    let bls_sig = signer
        .aggregate_shares(&pub_key_set, &shares)
        .expect("Aggregation should succeed");

    receipt.agg_pk = bincode::serialize(&pub_key_set).expect("Serialization should succeed");
    receipt.sig_qk = Some(bls_sig);

    // Create validator
    let adjudicator = create_test_adjudicator();
    let validator = ReceiptValidator::new(adjudicator, 10)
        .with_bls_verification(signer.clone(), true);

    // Validate - should succeed
    let result = validator.validate_receipt(&receipt, 100).await;

    assert!(result.is_valid, "Receipt with acceptances and rejections should be valid");

    println!("✅ Receipt validation succeeds with both acceptances and rejections");
}

#[tokio::test]
async fn test_challenge_window_expiration() {
    // Setup BLS infrastructure
    let threshold_config = ThresholdConfig::new(10, 7).expect("Valid threshold config");
    let (pub_key_set, secret_keys) = adic_crypto::generate_threshold_keys(10, 7)
        .expect("Key generation should succeed");

    let signer = Arc::new(BLSThresholdSigner::new(threshold_config));

    // Create validator with 10-epoch challenge window
    let adjudicator = create_test_adjudicator();
    let validator = ReceiptValidator::new(adjudicator, 10)
        .with_bls_verification(signer.clone(), false);

    // Create receipt at epoch 100
    let receipt = create_signed_receipt(100, 5, &signer, &secret_keys, &pub_key_set);

    // Test 1: Within challenge window (epoch 105)
    let result1 = validator.validate_receipt(&receipt, 105).await;
    assert_eq!(result1.validation_status, adic_pouw::receipt_validator::ValidationStatus::PendingChallenge);

    // Test 2: Challenge window expired (epoch 111 = 100 + 10 + 1)
    let result2 = validator.validate_receipt(&receipt, 111).await;
    assert_eq!(result2.validation_status, adic_pouw::receipt_validator::ValidationStatus::FinalizedValid);

    println!("✅ Challenge window expiration works correctly");
}
