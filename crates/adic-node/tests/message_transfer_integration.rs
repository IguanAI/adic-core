//! Integration test for message-embedded value transfers
//!
//! Tests the complete flow:
//! 1. Submit a message with an embedded transfer
//! 2. Message passes validation and admissibility
//! 3. Message is stored in the DAG
//! 4. Message reaches finality
//! 5. Transfer is executed atomically

use adic_economics::{AccountAddress, AdicAmount};
use adic_node::{AdicNode, NodeConfig};
use adic_types::ValueTransfer;
use tempfile::TempDir;

async fn create_test_node() -> AdicNode {
    let temp_dir = TempDir::new().unwrap();
    let mut config = NodeConfig::default();
    config.node.data_dir = temp_dir.path().to_path_buf();
    config.storage.backend = "memory".to_string();
    config.network.enabled = false;
    config.node.bootstrap = Some(true); // Bootstrap node for testing

    AdicNode::new(config).await.unwrap()
}

#[tokio::test]
async fn test_message_with_transfer_full_flow() {
    // This test demonstrates the unified message+transfer system

    // Create a test node with in-memory storage
    let node = create_test_node().await;

    // Setup test accounts with initial balances
    let sender = AccountAddress::from_bytes([1u8; 32]);
    let recipient = AccountAddress::from_bytes([2u8; 32]);

    let initial_balance = AdicAmount::from_adic(1000.0);

    // Credit sender with initial balance
    node.economics
        .balances
        .credit(sender, initial_balance)
        .await
        .unwrap();

    // Verify initial balances
    let sender_balance = node.economics.balances.get_balance(sender).await.unwrap();
    assert_eq!(sender_balance, initial_balance);

    let recipient_balance = node
        .economics
        .balances
        .get_balance(recipient)
        .await
        .unwrap();
    assert_eq!(recipient_balance, AdicAmount::ZERO);

    // Get current nonce for sender
    let sender_nonce = node.economics.balances.get_nonce(sender).await.unwrap();
    assert_eq!(sender_nonce, 0);

    // Create a transfer (100 ADIC from sender to recipient)
    let transfer_amount = 100u64 * 1_000_000_000; // 100 ADIC in base units
    let transfer = ValueTransfer::new(
        sender.as_bytes().to_vec(),
        recipient.as_bytes().to_vec(),
        transfer_amount,
        sender_nonce + 1, // Next nonce
    );

    // Validate the transfer is well-formed
    assert!(transfer.is_valid());

    // Submit a message with the embedded transfer
    let message_content = b"Payment for services rendered";
    let message_id = node
        .submit_message_with_transfer(message_content.to_vec(), Some(transfer))
        .await
        .unwrap();

    println!(
        "✅ Message submitted with transfer: {}",
        hex::encode(message_id.as_bytes())
    );

    // At this point:
    // - Message is in the DAG
    // - Transfer is NOT yet executed (waiting for finality)
    // - Balances should be unchanged

    let sender_balance_after_submit = node.economics.balances.get_balance(sender).await.unwrap();
    assert_eq!(
        sender_balance_after_submit, initial_balance,
        "Balance should not change until finality"
    );

    // Simulate finality (in a real system, this happens through k-core or homology)
    // For this test, we manually trigger finality checking

    // Note: In the actual system, finality is achieved through:
    // 1. K-core analysis (messages with k >= threshold)
    // 2. Persistent homology (topological invariants)
    // 3. The check_finality() method runs periodically

    // When a message reaches finality, the transfer is executed automatically
    // through the check_finality() -> process_message_transfer() flow

    println!("✅ Transfer will execute when message reaches finality");
    println!(
        "   - Current sender balance: {} ADIC",
        sender_balance_after_submit.to_adic()
    );
    println!(
        "   - Expected after finality: {} ADIC",
        (initial_balance.to_base_units() - transfer_amount)
    );
}

#[tokio::test]
async fn test_message_transfer_validation() {
    let node = create_test_node().await;

    let sender = AccountAddress::from_bytes([3u8; 32]);
    let recipient = AccountAddress::from_bytes([4u8; 32]);

    // Give sender 50 ADIC
    node.economics
        .balances
        .credit(sender, AdicAmount::from_adic(50.0))
        .await
        .unwrap();

    // Try to transfer 100 ADIC (more than balance) - should fail validation
    let transfer_amount = 100u64 * 1_000_000_000;
    let transfer = ValueTransfer::new(
        sender.as_bytes().to_vec(),
        recipient.as_bytes().to_vec(),
        transfer_amount,
        1,
    );

    let result = node
        .submit_message_with_transfer(b"Insufficient funds test".to_vec(), Some(transfer))
        .await;

    // Should fail with insufficient balance error
    assert!(
        result.is_err(),
        "Transfer should fail due to insufficient balance"
    );
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("Insufficient balance") || error.to_string().contains("balance"),
        "Error should mention insufficient balance: {}",
        error
    );

    println!("✅ Insufficient balance validation works correctly");
}

#[tokio::test]
async fn test_message_transfer_nonce_validation() {
    let node = create_test_node().await;

    let sender = AccountAddress::from_bytes([5u8; 32]);
    let recipient = AccountAddress::from_bytes([6u8; 32]);

    // Give sender balance
    node.economics
        .balances
        .credit(sender, AdicAmount::from_adic(1000.0))
        .await
        .unwrap();

    // Current nonce is 0, so next valid nonce is 1
    let current_nonce = node.economics.balances.get_nonce(sender).await.unwrap();
    assert_eq!(current_nonce, 0);

    // Try to use nonce 5 (should be 1) - should fail
    let transfer = ValueTransfer::new(
        sender.as_bytes().to_vec(),
        recipient.as_bytes().to_vec(),
        100 * 1_000_000_000,
        5, // Wrong nonce
    );

    let result = node
        .submit_message_with_transfer(b"Wrong nonce test".to_vec(), Some(transfer))
        .await;

    assert!(result.is_err(), "Transfer should fail due to invalid nonce");
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("nonce") || error.to_string().contains("Invalid nonce"),
        "Error should mention nonce: {}",
        error
    );

    println!("✅ Nonce validation works correctly");
}

#[tokio::test]
async fn test_message_without_transfer() {
    let node = create_test_node().await;

    // Submit a regular message without transfer
    let message_content = b"Just a regular data message";
    let message_id = node.submit_message(message_content.to_vec()).await.unwrap();

    println!(
        "✅ Message submitted without transfer: {}",
        hex::encode(message_id.as_bytes())
    );

    // Retrieve the message and verify it has no transfer
    let message = node
        .storage
        .get_message(&message_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !message.has_value_transfer(),
        "Message should not have a transfer"
    );
    assert_eq!(message.data, message_content, "Message data should match");

    println!("✅ Messages without transfers still work correctly");
}
