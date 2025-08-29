use adic_crypto::Keypair;
use adic_node::{AdicNode, NodeConfig};
use adic_storage::{store::BackendType, StorageConfig, StorageEngine};
use adic_types::{AdicFeatures, AdicMessage, AdicMeta, AxisPhi, MessageId, QpDigits};
use chrono::Utc;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn test_node_basic_operations() {
    // Test basic node operations - simplified test
    let mut config = NodeConfig::default();
    // Use in-memory storage for this test
    config.storage.backend = "memory".to_string();
    let node = Arc::new(AdicNode::new(config).await.unwrap());

    // The actual persistence and recovery tests should be in the storage module
    // This test just verifies the node can be created
    let stats = node.get_stats().await.unwrap();
    assert_eq!(
        stats.message_count, 0,
        "New node should start with no messages"
    );
}

#[tokio::test]
async fn test_storage_persistence_and_recovery() {
    // Create a temporary directory for RocksDB
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_rocksdb");

    // Configure storage with RocksDB backend
    let storage_config = StorageConfig {
        backend_type: BackendType::RocksDB {
            path: db_path.to_str().unwrap().to_string(),
        },
        cache_size: 100,
        flush_interval_ms: 100,
        max_batch_size: 10,
    };

    // Create some test messages and store them
    let message_ids: Vec<MessageId> = {
        // First storage instance - write data
        let storage = Arc::new(StorageEngine::new(storage_config.clone()).unwrap());

        // Store messages directly in storage
        let mut ids = Vec::new();
        for i in 0..5 {
            let features =
                AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(i as u64, 3, 10))]);

            let msg = AdicMessage::new(
                vec![],
                features,
                AdicMeta::new(Utc::now()),
                *Keypair::generate().public_key(),
                format!("Test message {}", i).into_bytes(),
            );

            storage.store_message(&msg).await.unwrap();
            ids.push(msg.id);
            println!("Stored message {}: {}", i, hex::encode(msg.id.as_bytes()));
        }

        // Explicitly drop to close RocksDB properly
        drop(storage);

        ids
    };

    // Simulate crash and recovery - create new storage instance with same DB path
    {
        println!("\n=== Simulating recovery after crash ===");

        let storage = Arc::new(StorageEngine::new(storage_config.clone()).unwrap());

        // Verify all messages were persisted and recovered
        for (i, msg_id) in message_ids.iter().enumerate() {
            let recovered_msg = storage.get_message(msg_id).await.unwrap();
            assert!(recovered_msg.is_some(), "Message {} should be recovered", i);

            if let Some(msg) = recovered_msg {
                let content = String::from_utf8_lossy(&msg.payload);
                println!("Recovered message {}: {}", i, content);
                assert_eq!(content, format!("Test message {}", i));
            }
        }

        println!("✅ All messages successfully recovered from RocksDB");

        // Submit a new message to verify the storage is functional
        let features = AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(99, 3, 10))]);

        let new_msg = AdicMessage::new(
            vec![],
            features,
            AdicMeta::new(Utc::now()),
            *Keypair::generate().public_key(),
            b"Post-recovery message".to_vec(),
        );

        storage.store_message(&new_msg).await.unwrap();
        println!(
            "New message after recovery: {}",
            hex::encode(new_msg.id.as_bytes())
        );

        // Verify the new message was stored
        let retrieved = storage.get_message(&new_msg.id).await.unwrap();
        assert!(retrieved.is_some(), "New message should be stored");
    }
}

#[tokio::test]
async fn test_storage_snapshot_and_restore() {
    // Test snapshot/restore functionality
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("snapshot_test_db");
    let snapshot_dir = temp_dir.path().join("snapshots");
    std::fs::create_dir_all(&snapshot_dir).unwrap();

    let storage_config = StorageConfig {
        backend_type: BackendType::RocksDB {
            path: db_path.to_str().unwrap().to_string(),
        },
        cache_size: 100,
        flush_interval_ms: 100,
        max_batch_size: 10,
    };

    // Create and populate storage
    let storage1 = StorageEngine::new(storage_config.clone()).unwrap();

    // Store test messages
    let mut message_ids = Vec::new();
    for i in 0..3 {
        let features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(i as u64, 3, 10)),
            AxisPhi::new(1, QpDigits::from_u64(i as u64, 3, 10)),
            AxisPhi::new(2, QpDigits::from_u64(i as u64, 3, 10)),
        ]);

        let msg = AdicMessage::new(
            vec![],
            features,
            AdicMeta::new(Utc::now()),
            *Keypair::generate().public_key(),
            format!("Snapshot test {}", i).into_bytes(),
        );

        storage1.store_message(&msg).await.unwrap();
        message_ids.push(msg.id);
    }

    // Create snapshot
    let snapshot_manager = adic_storage::snapshot::SnapshotManager::new(snapshot_dir.clone(), 10);
    let backend = storage1.backend();
    let snapshot_path = snapshot_manager.create_snapshot(&**backend).await.unwrap();
    println!("Created snapshot at: {:?}", snapshot_path);

    // Drop original storage
    drop(storage1);

    // Delete the original database
    std::fs::remove_dir_all(&db_path).unwrap();

    // Create new storage instance
    let storage2 = StorageEngine::new(storage_config.clone()).unwrap();

    // Verify data is gone
    for msg_id in &message_ids {
        let msg = storage2.get_message(msg_id).await.unwrap();
        assert!(msg.is_none(), "Message should not exist in new database");
    }

    // Restore from snapshot
    let backend2 = storage2.backend();
    let snapshot = snapshot_manager
        .load_latest()
        .await
        .unwrap()
        .expect("Should have at least one snapshot");
    snapshot.restore(&**backend2).await.unwrap();

    // Verify all messages are restored
    for (i, msg_id) in message_ids.iter().enumerate() {
        let msg = storage2.get_message(msg_id).await.unwrap();
        assert!(
            msg.is_some(),
            "Message {} should be restored from snapshot",
            i
        );

        if let Some(msg) = msg {
            let content = String::from_utf8_lossy(&msg.payload);
            assert_eq!(content, format!("Snapshot test {}", i));
            println!("Restored message {}: {}", i, content);
        }
    }

    println!("✅ All messages successfully restored from snapshot");
}

#[tokio::test]
async fn test_concurrent_persistence() {
    // Test that concurrent operations are properly persisted
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("concurrent_test_db");

    let storage_config = StorageConfig {
        backend_type: BackendType::RocksDB {
            path: db_path.to_str().unwrap().to_string(),
        },
        cache_size: 100,
        flush_interval_ms: 50,
        max_batch_size: 10,
    };

    let storage = Arc::new(StorageEngine::new(storage_config.clone()).unwrap());

    // Spawn multiple tasks writing concurrently
    let mut handles = Vec::new();
    for i in 0..10 {
        let storage_clone = storage.clone();
        let handle = tokio::spawn(async move {
            let features = AdicFeatures::new(vec![
                AxisPhi::new(0, QpDigits::from_u64(i as u64, 3, 10)),
                AxisPhi::new(1, QpDigits::from_u64(i as u64, 3, 10)),
                AxisPhi::new(2, QpDigits::from_u64(i as u64, 3, 10)),
            ]);

            let msg = AdicMessage::new(
                vec![],
                features,
                AdicMeta::new(Utc::now()),
                *Keypair::generate().public_key(),
                format!("Concurrent {}", i).into_bytes(),
            );

            storage_clone.store_message(&msg).await.unwrap();
            msg.id
        });
        handles.push(handle);
    }

    // Wait for all writes to complete
    let mut written_ids = Vec::new();
    for handle in handles {
        let msg_id = handle.await.unwrap();
        written_ids.push(msg_id);
    }

    // Drop and recreate storage to ensure persistence
    drop(storage);

    let storage2 = StorageEngine::new(storage_config).unwrap();

    // Verify all concurrent writes were persisted
    for (i, msg_id) in written_ids.iter().enumerate() {
        let msg = storage2.get_message(msg_id).await.unwrap();
        assert!(
            msg.is_some(),
            "Concurrent message {} should be persisted",
            i
        );
    }

    println!("✅ All concurrent writes successfully persisted");
}
