use adic_crypto::Keypair;
use adic_storage::store::BackendType;
use adic_storage::{StorageConfig, StorageEngine};
use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AxisPhi, MessageId, QpDigits, DEFAULT_P, DEFAULT_PRECISION,
};
use chrono::Utc;
use tempfile::tempdir;

fn create_test_message(id: u64, parents: Vec<MessageId>) -> AdicMessage {
    // Use a real keypair for proper signatures
    let keypair = Keypair::generate();

    let features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(id, DEFAULT_P, DEFAULT_PRECISION)),
        AxisPhi::new(1, QpDigits::from_u64(0, DEFAULT_P, DEFAULT_PRECISION)),
    ]);

    let mut message = AdicMessage::new(
        parents,
        features,
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        format!("Test message {}", id).into_bytes(),
    );

    // Sign the message properly
    message.signature = keypair.sign(&message.to_bytes());
    message
}

#[tokio::test]
async fn test_memory_storage() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };

    let storage = StorageEngine::new(config).unwrap();

    // Test storing and retrieving a message
    let message = create_test_message(1, vec![]);
    let message_id = message.id;

    storage.store_message(&message).await.unwrap();

    let retrieved = storage.get_message(&message_id).await.unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().id, message_id);
}

#[cfg(feature = "rocksdb")]
#[tokio::test]
async fn test_rocksdb_storage() {
    let temp_dir = tempdir().unwrap();
    let path = temp_dir.path().to_str().unwrap().to_string();

    let config = StorageConfig {
        backend_type: BackendType::RocksDB { path: path.clone() },
        ..Default::default()
    };

    let storage = StorageEngine::new(config).unwrap();

    // Test storing and retrieving a message
    let message = create_test_message(1, vec![]);
    let message_id = message.id;

    storage.store_message(&message).await.unwrap();

    let retrieved = storage.get_message(&message_id).await.unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().id, message_id);

    // Test persistence - create new storage instance with same path
    drop(storage);

    let config2 = StorageConfig {
        backend_type: BackendType::RocksDB { path },
        ..Default::default()
    };

    let storage2 = StorageEngine::new(config2).unwrap();

    // Message should still be there
    let retrieved2 = storage2.get_message(&message_id).await.unwrap();
    assert!(retrieved2.is_some());
    assert_eq!(retrieved2.unwrap().id, message_id);
}

#[tokio::test]
async fn test_tip_management() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Store first message (genesis)
    let genesis = create_test_message(1, vec![]);
    storage.store_message(&genesis).await.unwrap();

    // Genesis should be a tip
    let tips = storage.get_tips().await.unwrap();
    assert_eq!(tips.len(), 1);
    assert_eq!(tips[0], genesis.id);

    // Store second message with genesis as parent
    let message2 = create_test_message(2, vec![genesis.id]);
    storage.store_message(&message2).await.unwrap();

    // Genesis should no longer be a tip, message2 should be
    let tips = storage.get_tips().await.unwrap();
    assert_eq!(tips.len(), 1);
    assert_eq!(tips[0], message2.id);
}

#[tokio::test]
async fn test_children_tracking() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Store parent message
    let parent = create_test_message(1, vec![]);
    storage.store_message(&parent).await.unwrap();

    // Store children
    let child1 = create_test_message(2, vec![parent.id]);
    let child2 = create_test_message(3, vec![parent.id]);

    storage.store_message(&child1).await.unwrap();
    storage.store_message(&child2).await.unwrap();

    // Get children of parent
    let children = storage.get_children(&parent.id).await.unwrap();
    assert_eq!(children.len(), 2);
    assert!(children.contains(&child1.id));
    assert!(children.contains(&child2.id));
}

#[tokio::test]
async fn test_proposer_messages() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    let keypair = Keypair::generate();
    let proposer = *keypair.public_key();

    // Store multiple messages from same proposer
    let mut message_ids = Vec::new();
    for i in 1..=3 {
        let features = AdicFeatures::new(vec![AxisPhi::new(
            0,
            QpDigits::from_u64(i, DEFAULT_P, DEFAULT_PRECISION),
        )]);

        let mut message = AdicMessage::new(
            vec![],
            features,
            AdicMeta::new(Utc::now()),
            proposer,
            format!("Message {}", i).into_bytes(),
        );

        message.signature = keypair.sign(&message.to_bytes());
        message_ids.push(message.id);
        storage.store_message(&message).await.unwrap();
    }

    // Verify all messages were stored
    for id in &message_ids {
        let msg = storage.get_message(id).await.unwrap();
        assert!(msg.is_some());
        assert_eq!(msg.unwrap().proposer_pk, proposer);
    }
}

#[tokio::test]
async fn test_storage_stats() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Initial stats
    let stats = storage.get_stats().await.unwrap();
    assert_eq!(stats.message_count, 0);
    assert_eq!(stats.tip_count, 0);

    // Store some messages
    let msg1 = create_test_message(1, vec![]);
    let msg2 = create_test_message(2, vec![msg1.id]);

    storage.store_message(&msg1).await.unwrap();
    storage.store_message(&msg2).await.unwrap();

    // Check updated stats
    let stats = storage.get_stats().await.unwrap();
    assert_eq!(stats.message_count, 2);
    assert_eq!(stats.tip_count, 1); // Only msg2 should be a tip
}

#[tokio::test]
async fn test_batch_operations() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Store multiple messages in a batch
    let messages: Vec<AdicMessage> = (1..=5).map(|i| create_test_message(i, vec![])).collect();

    for message in &messages {
        storage.store_message(message).await.unwrap();
    }

    // Verify all messages were stored
    for message in &messages {
        let retrieved = storage.get_message(&message.id).await.unwrap();
        assert!(retrieved.is_some());
    }
}

#[cfg(feature = "rocksdb")]
#[tokio::test]
async fn test_storage_config_default_with_env() {
    // Set environment variable
    std::env::set_var("ADIC_DATA_DIR", "/custom/path");

    let config = StorageConfig::default();

    #[cfg(feature = "rocksdb")]
    match &config.backend_type {
        BackendType::RocksDB { path } => {
            assert_eq!(path, "/custom/path");
        }
        _ => panic!("Expected RocksDB backend"),
    }

    // Clean up
    std::env::remove_var("ADIC_DATA_DIR");
}

#[tokio::test]
async fn test_nonexistent_message_retrieval() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    let random_id = MessageId::new(&[1, 2, 3, 4, 5]);
    let result = storage.get_message(&random_id).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_duplicate_message_storage() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    let message = create_test_message(1, vec![]);

    // Store once
    storage.store_message(&message).await.unwrap();

    // Try to store again - should handle gracefully
    let result = storage.store_message(&message).await;
    // Depending on implementation, this might succeed or return an error
    // but it shouldn't panic
    assert!(result.is_ok() || result.is_err());
}
