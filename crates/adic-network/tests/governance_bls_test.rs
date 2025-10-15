//! Test BLS signature collection via governance protocol
//!
//! Verifies that:
//! - Signature share requests initiate collection
//! - Committee members can submit signature shares
//! - Shares are collected and deduplicated
//! - Aggregation workflow completes successfully

use adic_crypto::bls::{generate_threshold_keys, BLSThresholdSigner, ThresholdConfig, dst};
use adic_network::protocol::{GovernanceConfig, GovernanceMessage, GovernanceProtocol};
use adic_types::PublicKey;

#[tokio::test]
async fn test_bls_signature_collection_workflow() {
    // Setup: Create BLS threshold keys (5 participants, threshold 3)
    let (pk_set, sk_shares) = generate_threshold_keys(5, 3).expect("Failed to generate keys");
    let signer = BLSThresholdSigner::new(ThresholdConfig::new(5, 3).unwrap());

    // Setup: Create governance protocol
    let protocol = GovernanceProtocol::new(GovernanceConfig::default());

    let proposal_id = [1u8; 32];
    let message = b"test governance receipt";
    let dst_tag = dst::GOV_RECEIPT;

    // Compute message hash for the receipt
    let mut prefixed_message = Vec::with_capacity(dst_tag.len() + message.len());
    prefixed_message.extend_from_slice(dst_tag);
    prefixed_message.extend_from_slice(message);
    let message_hash = blake3::hash(&prefixed_message);

    // Step 1: Initiate signature collection
    let request = GovernanceMessage::SignatureShareRequest {
        proposal_id,
        requester: PublicKey::from_bytes([99u8; 32]),
        message_hash: *message_hash.as_bytes(),
        timestamp: 1000,
    };

    protocol.handle_message(request).await.expect("Failed to handle request");

    // Verify collection started
    let shares_before = protocol.get_signature_shares(&proposal_id).await;
    assert!(shares_before.is_some());
    assert_eq!(shares_before.unwrap().len(), 0);

    // Step 2: Committee members submit signature shares
    for i in 0..3 {
        let share = signer
            .sign_share(&sk_shares[i], i, message, dst_tag)
            .expect("Failed to sign share");

        let share_msg = GovernanceMessage::SignatureShare {
            proposal_id,
            signer: PublicKey::from_bytes([i as u8; 32]),
            share: share.clone(),
            timestamp: 1000 + i as u64,
        };

        protocol.handle_message(share_msg).await.expect("Failed to handle share");
    }

    // Verify shares collected
    let collected_shares = protocol.get_signature_shares(&proposal_id).await.unwrap();
    assert_eq!(collected_shares.len(), 3);

    // Step 3: Aggregate shares into threshold signature
    let aggregated_sig = signer
        .aggregate_shares(&pk_set, &collected_shares)
        .expect("Failed to aggregate shares");

    // Verify threshold signature
    let is_valid = signer
        .verify(&pk_set, message, dst_tag, &aggregated_sig)
        .expect("Verification failed");

    assert!(is_valid, "Threshold signature verification failed");

    // Step 4: Complete collection
    let final_shares = protocol.complete_signature_collection(&proposal_id).await;
    assert!(final_shares.is_some());
    assert_eq!(final_shares.unwrap().len(), 3);

    // Verify collection removed
    assert!(protocol.get_signature_shares(&proposal_id).await.is_none());
}

#[tokio::test]
async fn test_duplicate_signature_shares_rejected() {
    let (_, sk_shares) = generate_threshold_keys(5, 3).unwrap();
    let signer = BLSThresholdSigner::new(ThresholdConfig::new(5, 3).unwrap());

    let protocol = GovernanceProtocol::new(GovernanceConfig::default());

    let proposal_id = [2u8; 32];
    let message = b"test message";
    let message_hash = blake3::hash(message);

    // Start collection
    protocol
        .handle_message(GovernanceMessage::SignatureShareRequest {
            proposal_id,
            requester: PublicKey::from_bytes([99u8; 32]),
            message_hash: *message_hash.as_bytes(),
            timestamp: 1000,
        })
        .await
        .unwrap();

    // Submit share from participant 0
    let share1 = signer.sign_share(&sk_shares[0], 0, message, dst::GOV_RECEIPT).unwrap();
    let signer_pk = PublicKey::from_bytes([0u8; 32]);

    protocol
        .handle_message(GovernanceMessage::SignatureShare {
            proposal_id,
            signer: signer_pk,
            share: share1.clone(),
            timestamp: 1001,
        })
        .await
        .unwrap();

    // Verify 1 share collected
    assert_eq!(protocol.get_signature_shares(&proposal_id).await.unwrap().len(), 1);

    // Submit duplicate share from same participant
    protocol
        .handle_message(GovernanceMessage::SignatureShare {
            proposal_id,
            signer: signer_pk,
            share: share1,
            timestamp: 1002,
        })
        .await
        .unwrap();

    // Verify still only 1 share (duplicate rejected)
    assert_eq!(protocol.get_signature_shares(&proposal_id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn test_signature_shares_for_unknown_collection_ignored() {
    let (_, sk_shares) = generate_threshold_keys(5, 3).unwrap();
    let signer = BLSThresholdSigner::new(ThresholdConfig::new(5, 3).unwrap());

    let protocol = GovernanceProtocol::new(GovernanceConfig::default());

    let proposal_id = [3u8; 32];
    let message = b"test";

    // Submit share WITHOUT starting collection
    let share = signer.sign_share(&sk_shares[0], 0, message, dst::GOV_RECEIPT).unwrap();

    protocol
        .handle_message(GovernanceMessage::SignatureShare {
            proposal_id,
            signer: PublicKey::from_bytes([0u8; 32]),
            share,
            timestamp: 1000,
        })
        .await
        .unwrap();

    // Verify no collection exists
    assert!(protocol.get_signature_shares(&proposal_id).await.is_none());
}

#[tokio::test]
async fn test_multiple_concurrent_signature_collections() {
    let (pk_set, sk_shares) = generate_threshold_keys(5, 3).unwrap();
    let signer = BLSThresholdSigner::new(ThresholdConfig::new(5, 3).unwrap());

    let protocol = GovernanceProtocol::new(GovernanceConfig::default());

    let proposal_a = [10u8; 32];
    let proposal_b = [20u8; 32];
    let message_a = b"proposal A";
    let message_b = b"proposal B";

    // Start two collections
    for (proposal_id, message) in &[(proposal_a, message_a), (proposal_b, message_b)] {
        let hash = blake3::hash(*message);
        protocol
            .handle_message(GovernanceMessage::SignatureShareRequest {
                proposal_id: *proposal_id,
                requester: PublicKey::from_bytes([99u8; 32]),
                message_hash: *hash.as_bytes(),
                timestamp: 1000,
            })
            .await
            .unwrap();
    }

    // Add shares to proposal A
    for i in 0..3 {
        let share = signer.sign_share(&sk_shares[i], i, message_a, dst::GOV_RECEIPT).unwrap();
        protocol
            .handle_message(GovernanceMessage::SignatureShare {
                proposal_id: proposal_a,
                signer: PublicKey::from_bytes([i as u8; 32]),
                share,
                timestamp: 1000 + i as u64,
            })
            .await
            .unwrap();
    }

    // Add shares to proposal B
    for i in 0..3 {
        let share = signer.sign_share(&sk_shares[i], i, message_b, dst::GOV_RECEIPT).unwrap();
        protocol
            .handle_message(GovernanceMessage::SignatureShare {
                proposal_id: proposal_b,
                signer: PublicKey::from_bytes([i as u8; 32]),
                share,
                timestamp: 2000 + i as u64,
            })
            .await
            .unwrap();
    }

    // Verify both collections have shares
    let shares_a = protocol.get_signature_shares(&proposal_a).await.unwrap();
    let shares_b = protocol.get_signature_shares(&proposal_b).await.unwrap();

    assert_eq!(shares_a.len(), 3);
    assert_eq!(shares_b.len(), 3);

    // Aggregate and verify both signatures
    let sig_a = signer.aggregate_shares(&pk_set, &shares_a).unwrap();
    let sig_b = signer.aggregate_shares(&pk_set, &shares_b).unwrap();

    assert!(signer.verify(&pk_set, message_a, dst::GOV_RECEIPT, &sig_a).unwrap());
    assert!(signer.verify(&pk_set, message_b, dst::GOV_RECEIPT, &sig_b).unwrap());
}
