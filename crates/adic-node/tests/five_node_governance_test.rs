//! Five-Node Governance Integration Test (Simplified)
//!
//! Tests BLS signature collection workflow in a governance context.
//! This test focuses on the network layer integration rather than full governance logic.

use adic_crypto::bls::{generate_threshold_keys, BLSThresholdSigner, ThresholdConfig, dst};
use adic_network::protocol::{GovernanceConfig, GovernanceMessage, GovernanceProtocol, GovernanceEvent};

/// Test BLS threshold signature collection for governance receipts
#[tokio::test]
async fn test_five_node_bls_governance_receipt() {
    // Setup: 5-node governance committee with 3-of-5 threshold
    let (pk_set, sk_shares) = generate_threshold_keys(5, 3).expect("Failed to generate BLS keys");
    let signer = BLSThresholdSigner::new(ThresholdConfig::new(5, 3).unwrap());

    // Create governance protocol (simulates network layer on each node)
    let protocol = GovernanceProtocol::new(GovernanceConfig::default());

    let proposal_id = [42u8; 32];

    // Phase 1: Governance tally complete, ready for receipt generation
    let receipt_message = format!("PASS {} epoch=150 yes=330.0 no=0.0", hex::encode(&proposal_id[..8]));
    let message_bytes = receipt_message.as_bytes();
    let message_hash = blake3::hash(message_bytes);

    // Phase 2: Coordinator initiates BLS signature collection
    let request = GovernanceMessage::SignatureShareRequest {
        proposal_id,
        requester: adic_types::PublicKey::from_bytes([99u8; 32]),
        message_hash: *message_hash.as_bytes(),
        timestamp: 1000,
    };

    protocol.handle_message(request).await.expect("Failed to start signature collection");

    // Phase 3: Each committee member signs the receipt
    for i in 0..5 {
        let share = signer
            .sign_share(&sk_shares[i], i, message_bytes, dst::GOV_RECEIPT)
            .expect("Failed to create signature share");

        let share_msg = GovernanceMessage::SignatureShare {
            proposal_id,
            signer: adic_types::PublicKey::from_bytes([i as u8; 32]),
            share: share.clone(),
            timestamp: 1000 + i as u64,
        };

        protocol.handle_message(share_msg).await.expect("Failed to submit signature share");
    }

    // Phase 4: Verify all 5 shares collected
    let collected_shares = protocol.get_signature_shares(&proposal_id).await.unwrap();
    assert_eq!(collected_shares.len(), 5, "Should collect all 5 signature shares");

    // Phase 5: Aggregate shares into threshold signature (only need 3)
    let threshold_shares = &collected_shares[0..3];
    let threshold_sig = signer
        .aggregate_shares(&pk_set, threshold_shares)
        .expect("Failed to aggregate BLS signatures");

    // Phase 6: Verify threshold signature
    let is_valid = signer
        .verify(&pk_set, message_bytes, dst::GOV_RECEIPT, &threshold_sig)
        .expect("Verification failed");

    assert!(is_valid, "Threshold signature should be valid");

    // Phase 7: Broadcast receipt with threshold signature
    // (In real implementation, this would propagate via gossip network)
    let receipt = GovernanceMessage::Receipt {
        message_id: adic_types::MessageId::from_bytes([1u8; 32]),
        proposal_id,
        result: "pass".to_string(),
        credits_yes: 330.0,
        credits_no: 0.0,
        threshold_sig: threshold_sig.as_hex().as_bytes().to_vec(),
        timestamp: 2000,
    };

    protocol.handle_message(receipt).await.expect("Failed to process receipt");

    // Verify events were emitted
    let mut sig_share_events = 0;
    while let Some(event) = protocol.try_recv_event().await {
        match event {
            GovernanceEvent::SignatureShareReceived { .. } => sig_share_events += 1,
            GovernanceEvent::ReceiptReceived { proposal_id: pid, result } => {
                assert_eq!(pid, proposal_id);
                assert_eq!(result, "pass");
            }
            _ => {}
        }
    }

    assert_eq!(sig_share_events, 5, "Should emit 5 signature share events");
}

/// Test Byzantine fault tolerance: threshold met with only 3 of 5 nodes
#[tokio::test]
async fn test_bls_threshold_with_byzantine_faults() {
    let (pk_set, sk_shares) = generate_threshold_keys(5, 3).unwrap();
    let signer = BLSThresholdSigner::new(ThresholdConfig::new(5, 3).unwrap());

    let protocol = GovernanceProtocol::new(GovernanceConfig::default());

    let proposal_id = [88u8; 32];
    let receipt_message = b"PASS proposal_88 epoch=200";
    let message_hash = blake3::hash(receipt_message);

    // Start collection
    protocol
        .handle_message(GovernanceMessage::SignatureShareRequest {
            proposal_id,
            requester: adic_types::PublicKey::from_bytes([99u8; 32]),
            message_hash: *message_hash.as_bytes(),
            timestamp: 1000,
        })
        .await
        .unwrap();

    // Only 3 honest nodes submit shares (2 Byzantine nodes offline/malicious)
    for i in 0..3 {
        let share = signer
            .sign_share(&sk_shares[i], i, receipt_message, dst::GOV_RECEIPT)
            .unwrap();

        protocol
            .handle_message(GovernanceMessage::SignatureShare {
                proposal_id,
                signer: adic_types::PublicKey::from_bytes([i as u8; 32]),
                share,
                timestamp: 1000 + i as u64,
            })
            .await
            .unwrap();
    }

    // Verify threshold still met with just 3 shares
    let shares = protocol.get_signature_shares(&proposal_id).await.unwrap();
    assert_eq!(shares.len(), 3);

    let threshold_sig = signer.aggregate_shares(&pk_set, &shares).unwrap();
    assert!(signer.verify(&pk_set, receipt_message, dst::GOV_RECEIPT, &threshold_sig).unwrap());
}

/// Test cross-proposal signature collection isolation
#[tokio::test]
async fn test_concurrent_governance_receipts() {
    let (pk_set, sk_shares) = generate_threshold_keys(5, 3).unwrap();
    let signer = BLSThresholdSigner::new(ThresholdConfig::new(5, 3).unwrap());

    let protocol = GovernanceProtocol::new(GovernanceConfig::default());

    let proposal_a = [10u8; 32];
    let proposal_b = [20u8; 32];
    let message_a = b"PASS proposal_A epoch=100";
    let message_b = b"PASS proposal_B epoch=100";

    // Start both collections
    for (pid, msg) in &[(proposal_a, message_a), (proposal_b, message_b)] {
        let hash = blake3::hash(*msg);
        protocol
            .handle_message(GovernanceMessage::SignatureShareRequest {
                proposal_id: *pid,
                requester: adic_types::PublicKey::from_bytes([99u8; 32]),
                message_hash: *hash.as_bytes(),
                timestamp: 1000,
            })
            .await
            .unwrap();
    }

    // Submit shares for both proposals
    for i in 0..3 {
        let share_a = signer.sign_share(&sk_shares[i], i, message_a, dst::GOV_RECEIPT).unwrap();
        let share_b = signer.sign_share(&sk_shares[i], i, message_b, dst::GOV_RECEIPT).unwrap();

        protocol
            .handle_message(GovernanceMessage::SignatureShare {
                proposal_id: proposal_a,
                signer: adic_types::PublicKey::from_bytes([i as u8; 32]),
                share: share_a,
                timestamp: 1000 + i as u64,
            })
            .await
            .unwrap();

        protocol
            .handle_message(GovernanceMessage::SignatureShare {
                proposal_id: proposal_b,
                signer: adic_types::PublicKey::from_bytes([i as u8; 32]),
                share: share_b,
                timestamp: 2000 + i as u64,
            })
            .await
            .unwrap();
    }

    // Verify both collections complete independently
    let shares_a = protocol.get_signature_shares(&proposal_a).await.unwrap();
    let shares_b = protocol.get_signature_shares(&proposal_b).await.unwrap();

    assert_eq!(shares_a.len(), 3);
    assert_eq!(shares_b.len(), 3);

    // Verify both signatures are valid
    let sig_a = signer.aggregate_shares(&pk_set, &shares_a).unwrap();
    let sig_b = signer.aggregate_shares(&pk_set, &shares_b).unwrap();

    assert!(signer.verify(&pk_set, message_a, dst::GOV_RECEIPT, &sig_a).unwrap());
    assert!(signer.verify(&pk_set, message_b, dst::GOV_RECEIPT, &sig_b).unwrap());
}
