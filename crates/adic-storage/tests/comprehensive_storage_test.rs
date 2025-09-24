use adic_crypto::Keypair;
use adic_storage::store::BackendType;
use adic_storage::{StorageConfig, StorageEngine};
use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AxisPhi, MessageId, QpDigits, DEFAULT_P, DEFAULT_PRECISION,
};
use chrono::Utc;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Barrier;

fn create_test_message_with_features(
    id: u64,
    parents: Vec<MessageId>,
    features: Vec<(u32, u64)>, // (axis, value)
) -> AdicMessage {
    let keypair = Keypair::generate();

    let axis_features: Vec<AxisPhi> = features
        .into_iter()
        .map(|(axis, value)| {
            AxisPhi::new(
                axis,
                QpDigits::from_u64(value, DEFAULT_P, DEFAULT_PRECISION),
            )
        })
        .collect();

    let features = AdicFeatures::new(axis_features);

    let mut message = AdicMessage::new(
        parents,
        features,
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        format!("Message {}", id).into_bytes(),
    );

    message.signature = keypair.sign(&message.to_bytes());
    message
}

// ============= Index Operations Tests =============

#[tokio::test]
async fn test_index_by_ball_operations() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Create messages with specific p-adic features
    let msg1 = create_test_message_with_features(1, vec![], vec![(0, 10), (1, 20)]);
    let msg2 = create_test_message_with_features(2, vec![], vec![(0, 11), (1, 20)]);
    let msg3 = create_test_message_with_features(3, vec![], vec![(0, 10), (1, 21)]);

    // Store messages
    storage.store_message(&msg1).await.unwrap();
    storage.store_message(&msg2).await.unwrap();
    storage.store_message(&msg3).await.unwrap();

    // Index messages by their p-adic balls
    let ball_id_1 = msg1.features.phi[0].qp_digits.ball_id(3);
    storage
        .index_by_ball(0, &ball_id_1, &msg1.id)
        .await
        .unwrap();

    let ball_id_2 = msg2.features.phi[0].qp_digits.ball_id(3);
    storage
        .index_by_ball(0, &ball_id_2, &msg2.id)
        .await
        .unwrap();

    let ball_id_3 = msg3.features.phi[0].qp_digits.ball_id(3);
    storage
        .index_by_ball(0, &ball_id_3, &msg3.id)
        .await
        .unwrap();

    // Query ball members
    let members = storage.get_ball_members(0, &ball_id_1).await.unwrap();

    // Messages 1 and 3 should be in the same ball if their features match at radius 3
    if ball_id_1 == ball_id_3 {
        assert!(members.contains(&msg1.id));
        assert!(members.contains(&msg3.id));
    }
}

#[tokio::test]
async fn test_multi_axis_ball_indexing() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Create message with multiple axes
    let msg = create_test_message_with_features(1, vec![], vec![(0, 100), (1, 200), (2, 300)]);
    storage.store_message(&msg).await.unwrap();

    // Index on all axes
    for (axis, feature) in msg.features.phi.iter().enumerate() {
        let ball_id = feature.qp_digits.ball_id(5);
        storage
            .index_by_ball(axis as u32, &ball_id, &msg.id)
            .await
            .unwrap();
    }

    // Verify indexing on each axis
    for (axis, feature) in msg.features.phi.iter().enumerate() {
        let ball_id = feature.qp_digits.ball_id(5);
        let members = storage
            .get_ball_members(axis as u32, &ball_id)
            .await
            .unwrap();
        assert!(members.contains(&msg.id));
    }
}

// ============= Metadata Operations Tests =============

#[tokio::test]
async fn test_metadata_operations() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    let msg = create_test_message_with_features(1, vec![], vec![(0, 42)]);
    storage.store_message(&msg).await.unwrap();

    // Store various types of metadata
    storage
        .put_metadata(&msg.id, "type", b"transaction")
        .await
        .unwrap();
    storage
        .put_metadata(&msg.id, "priority", b"high")
        .await
        .unwrap();
    storage
        .put_metadata(&msg.id, "custom_data", &[1, 2, 3, 4, 5])
        .await
        .unwrap();

    // Retrieve metadata
    let type_data = storage.get_metadata(&msg.id, "type").await.unwrap();
    assert_eq!(type_data, Some(b"transaction".to_vec()));

    let priority = storage.get_metadata(&msg.id, "priority").await.unwrap();
    assert_eq!(priority, Some(b"high".to_vec()));

    let custom = storage.get_metadata(&msg.id, "custom_data").await.unwrap();
    assert_eq!(custom, Some(vec![1, 2, 3, 4, 5]));

    // Non-existent metadata
    let missing = storage.get_metadata(&msg.id, "nonexistent").await.unwrap();
    assert_eq!(missing, None);
}

#[tokio::test]
async fn test_metadata_update_and_overwrite() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    let msg = create_test_message_with_features(1, vec![], vec![(0, 10)]);
    storage.store_message(&msg).await.unwrap();

    // Store initial metadata
    storage
        .put_metadata(&msg.id, "version", b"1.0")
        .await
        .unwrap();

    // Verify initial value
    let v1 = storage.get_metadata(&msg.id, "version").await.unwrap();
    assert_eq!(v1, Some(b"1.0".to_vec()));

    // Update metadata
    storage
        .put_metadata(&msg.id, "version", b"2.0")
        .await
        .unwrap();

    // Verify updated value
    let v2 = storage.get_metadata(&msg.id, "version").await.unwrap();
    assert_eq!(v2, Some(b"2.0".to_vec()));
}

// ============= Bulk Operations Tests =============

#[tokio::test]
async fn test_bulk_message_retrieval() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Store multiple messages
    let mut message_ids = Vec::new();
    for i in 1..=10 {
        let msg = create_test_message_with_features(i, vec![], vec![(0, i)]);
        message_ids.push(msg.id);
        storage.store_message(&msg).await.unwrap();
    }

    // Bulk retrieve
    let messages = storage.get_messages_bulk(&message_ids).await.unwrap();
    assert_eq!(messages.len(), 10);

    // Verify all messages retrieved
    let retrieved_ids: HashSet<_> = messages.iter().map(|m| m.id).collect();
    let expected_ids: HashSet<_> = message_ids.iter().cloned().collect();
    assert_eq!(retrieved_ids, expected_ids);
}

#[tokio::test]
async fn test_get_messages_in_range() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Store messages with timestamps
    let base_time = Utc::now();
    let mut all_ids = Vec::new();

    for i in 0..20 {
        let mut msg = create_test_message_with_features(i, vec![], vec![(0, i)]);
        msg.meta.timestamp = base_time + chrono::Duration::seconds(i as i64);
        all_ids.push(msg.id);
        storage.store_message(&msg).await.unwrap();
    }

    // Get messages in range
    let start = base_time + chrono::Duration::seconds(5);
    let end = base_time + chrono::Duration::seconds(15);
    let messages = storage
        .get_messages_in_range(start, end, 100, None)
        .await
        .unwrap();

    // Should get messages 5-15 (11 messages)
    assert!(messages.len() >= 10 && messages.len() <= 11);
}

#[tokio::test]
async fn test_get_messages_since_checkpoint() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Store messages in sequence
    let mut checkpoint_id = MessageId::new(&[0; 32]);

    // Store first batch
    for i in 0..5 {
        let msg = create_test_message_with_features(i, vec![], vec![(0, i)]);
        if i == 2 {
            checkpoint_id = msg.id; // Use message 2 as checkpoint
        }
        storage.store_message(&msg).await.unwrap();
    }

    // Store more messages after checkpoint
    for i in 5..10 {
        let msg = create_test_message_with_features(i, vec![], vec![(0, i)]);
        storage.store_message(&msg).await.unwrap();
    }

    // Get messages since checkpoint
    let messages = storage
        .get_messages_since(&checkpoint_id, 100)
        .await
        .unwrap();

    // Should get messages after the checkpoint
    assert!(!messages.is_empty());
}

// ============= Finalization Tests =============

#[tokio::test]
async fn test_message_finalization() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    let msg = create_test_message_with_features(1, vec![], vec![(0, 100)]);
    storage.store_message(&msg).await.unwrap();

    // Initially not finalized
    assert!(!storage.is_finalized(&msg.id).await.unwrap());
    assert_eq!(storage.get_finality_artifact(&msg.id).await.unwrap(), None);

    // Finalize with artifact
    let artifact = b"finality_proof_data";
    storage.finalize_message(&msg.id, artifact).await.unwrap();

    // Check finalization status
    assert!(storage.is_finalized(&msg.id).await.unwrap());

    // Retrieve artifact
    let retrieved = storage.get_finality_artifact(&msg.id).await.unwrap();
    assert_eq!(retrieved, Some(artifact.to_vec()));
}

#[tokio::test]
async fn test_get_recently_finalized() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Store and finalize messages
    let mut finalized_ids = Vec::new();
    for i in 1..=5 {
        let msg = create_test_message_with_features(i, vec![], vec![(0, i)]);
        storage.store_message(&msg).await.unwrap();
        storage.finalize_message(&msg.id, b"proof").await.unwrap();
        finalized_ids.push(msg.id);
    }

    // Store non-finalized messages
    for i in 6..=10 {
        let msg = create_test_message_with_features(i, vec![], vec![(0, i)]);
        storage.store_message(&msg).await.unwrap();
    }

    // Get recently finalized
    let recent = storage.get_recently_finalized(3).await.unwrap();
    assert_eq!(recent.len(), 3);

    // All returned should be from finalized set
    for id in &recent {
        assert!(finalized_ids.contains(id));
    }
}

// ============= Conflict Management Tests =============

#[tokio::test]
async fn test_conflict_set_management() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Create conflicting messages (e.g., double-spend)
    let msg1 = create_test_message_with_features(1, vec![], vec![(0, 100)]);
    let msg2 = create_test_message_with_features(2, vec![], vec![(0, 100)]);
    let msg3 = create_test_message_with_features(3, vec![], vec![(0, 100)]);

    storage.store_message(&msg1).await.unwrap();
    storage.store_message(&msg2).await.unwrap();
    storage.store_message(&msg3).await.unwrap();

    // Add to conflict set
    let conflict_id = "double_spend_tx_123";
    storage
        .add_to_conflict(conflict_id, &msg1.id)
        .await
        .unwrap();
    storage
        .add_to_conflict(conflict_id, &msg2.id)
        .await
        .unwrap();
    storage
        .add_to_conflict(conflict_id, &msg3.id)
        .await
        .unwrap();

    // Retrieve conflict set
    let conflict_set = storage.get_conflict_set(conflict_id).await.unwrap();
    assert_eq!(conflict_set.len(), 3);
    assert!(conflict_set.contains(&msg1.id));
    assert!(conflict_set.contains(&msg2.id));
    assert!(conflict_set.contains(&msg3.id));
}

#[tokio::test]
async fn test_multiple_conflict_sets() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Store messages
    let msg1 = create_test_message_with_features(1, vec![], vec![(0, 10)]);
    let msg2 = create_test_message_with_features(2, vec![], vec![(0, 20)]);
    let msg3 = create_test_message_with_features(3, vec![], vec![(0, 30)]);

    storage.store_message(&msg1).await.unwrap();
    storage.store_message(&msg2).await.unwrap();
    storage.store_message(&msg3).await.unwrap();

    // Create two different conflict sets
    storage
        .add_to_conflict("conflict_a", &msg1.id)
        .await
        .unwrap();
    storage
        .add_to_conflict("conflict_a", &msg2.id)
        .await
        .unwrap();

    storage
        .add_to_conflict("conflict_b", &msg2.id)
        .await
        .unwrap();
    storage
        .add_to_conflict("conflict_b", &msg3.id)
        .await
        .unwrap();

    // Verify conflict sets
    let set_a = storage.get_conflict_set("conflict_a").await.unwrap();
    assert_eq!(set_a.len(), 2);
    assert!(set_a.contains(&msg1.id));
    assert!(set_a.contains(&msg2.id));

    let set_b = storage.get_conflict_set("conflict_b").await.unwrap();
    assert_eq!(set_b.len(), 2);
    assert!(set_b.contains(&msg2.id));
    assert!(set_b.contains(&msg3.id));
}

// ============= Reputation Storage Tests =============

#[tokio::test]
async fn test_reputation_storage() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    let keypair1 = Keypair::generate();
    let keypair2 = Keypair::generate();
    let pubkey1 = *keypair1.public_key();
    let pubkey2 = *keypair2.public_key();

    // Initially no reputation
    assert_eq!(storage.get_reputation(&pubkey1).await.unwrap(), None);

    // Update reputation
    storage.update_reputation(&pubkey1, 0.8).await.unwrap();
    storage.update_reputation(&pubkey2, 0.3).await.unwrap();

    // Retrieve reputation
    assert_eq!(storage.get_reputation(&pubkey1).await.unwrap(), Some(0.8));
    assert_eq!(storage.get_reputation(&pubkey2).await.unwrap(), Some(0.3));

    // Update existing reputation
    storage.update_reputation(&pubkey1, 0.9).await.unwrap();
    assert_eq!(storage.get_reputation(&pubkey1).await.unwrap(), Some(0.9));
}

// ============= Concurrent Access Tests =============

#[tokio::test]
async fn test_concurrent_writes() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = Arc::new(StorageEngine::new(config).unwrap());

    let num_writers = 10;
    let messages_per_writer = 20;
    let barrier = Arc::new(Barrier::new(num_writers));

    let mut handles = Vec::new();

    for writer_id in 0..num_writers {
        let storage_clone = storage.clone();
        let barrier_clone = barrier.clone();

        let handle = tokio::spawn(async move {
            // Synchronize start
            barrier_clone.wait().await;

            for msg_id in 0..messages_per_writer {
                let global_id = writer_id * 1000 + msg_id;
                let msg = create_test_message_with_features(
                    global_id as u64,
                    vec![],
                    vec![(0, global_id as u64)],
                );
                storage_clone.store_message(&msg).await.unwrap();
            }
        });

        handles.push(handle);
    }

    // Wait for all writers
    for handle in handles {
        handle.await.unwrap();
    }

    // Verify all messages stored
    let all_messages = storage.list_all_messages().await.unwrap();
    assert_eq!(all_messages.len(), num_writers * messages_per_writer);
}

#[tokio::test]
async fn test_concurrent_reads() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = Arc::new(StorageEngine::new(config).unwrap());

    // Pre-populate storage
    let mut message_ids = Vec::new();
    for i in 0..50 {
        let msg = create_test_message_with_features(i, vec![], vec![(0, i)]);
        message_ids.push(msg.id);
        storage.store_message(&msg).await.unwrap();
    }

    let num_readers = 20;
    let barrier = Arc::new(Barrier::new(num_readers));
    let message_ids = Arc::new(message_ids);

    let mut handles = Vec::new();

    for _ in 0..num_readers {
        let storage_clone = storage.clone();
        let barrier_clone = barrier.clone();
        let ids_clone = message_ids.clone();

        let handle = tokio::spawn(async move {
            // Synchronize start
            barrier_clone.wait().await;

            // Each reader reads all messages multiple times
            for _ in 0..10 {
                for id in ids_clone.iter() {
                    let msg = storage_clone.get_message(id).await.unwrap();
                    assert!(msg.is_some());
                }
            }
        });

        handles.push(handle);
    }

    // Wait for all readers
    for handle in handles {
        handle.await.unwrap();
    }
}

#[tokio::test]
async fn test_concurrent_mixed_operations() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = Arc::new(StorageEngine::new(config).unwrap());

    let barrier = Arc::new(Barrier::new(3));

    // Writer task
    let storage_writer = storage.clone();
    let barrier_writer = barrier.clone();
    let writer = tokio::spawn(async move {
        barrier_writer.wait().await;
        for i in 0..100 {
            let msg = create_test_message_with_features(i, vec![], vec![(0, i)]);
            storage_writer.store_message(&msg).await.unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        }
    });

    // Reader task
    let storage_reader = storage.clone();
    let barrier_reader = barrier.clone();
    let reader = tokio::spawn(async move {
        barrier_reader.wait().await;
        for _ in 0..50 {
            let tips = storage_reader.get_tips().await.unwrap();
            let stats = storage_reader.get_stats().await.unwrap();
            assert!(tips.len() <= stats.message_count);
            tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;
        }
    });

    // Metadata writer task
    let storage_meta = storage.clone();
    let barrier_meta = barrier.clone();
    let meta_writer = tokio::spawn(async move {
        barrier_meta.wait().await;
        for i in 0..50 {
            // Try to add metadata to any existing message
            let all = storage_meta.list_all_messages().await.unwrap();
            if !all.is_empty() {
                let id = &all[i % all.len()];
                storage_meta
                    .put_metadata(id, "test", &[i as u8])
                    .await
                    .unwrap();
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;
        }
    });

    // Wait for all tasks
    writer.await.unwrap();
    reader.await.unwrap();
    meta_writer.await.unwrap();
}

// ============= Edge Cases and Error Handling =============

#[tokio::test]
async fn test_delete_message() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    let msg = create_test_message_with_features(1, vec![], vec![(0, 42)]);
    storage.store_message(&msg).await.unwrap();

    // Verify message exists
    assert!(storage.get_message(&msg.id).await.unwrap().is_some());

    // Delete message
    storage.delete_message(&msg.id).await.unwrap();

    // Verify message is gone
    assert!(storage.get_message(&msg.id).await.unwrap().is_none());

    // Delete non-existent should not error
    storage.delete_message(&msg.id).await.unwrap();
}

#[tokio::test]
async fn test_empty_conflict_set() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Get non-existent conflict set
    let empty_set = storage.get_conflict_set("nonexistent").await.unwrap();
    assert!(empty_set.is_empty());
}

#[tokio::test]
async fn test_flush_operation() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Store messages
    for i in 0..10 {
        let msg = create_test_message_with_features(i, vec![], vec![(0, i)]);
        storage.store_message(&msg).await.unwrap();
    }

    // Flush should succeed
    storage.flush().await.unwrap();

    // Data should still be accessible after flush
    let stats = storage.get_stats().await.unwrap();
    assert_eq!(stats.message_count, 10);
}

#[tokio::test]
async fn test_large_metadata_values() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    let msg = create_test_message_with_features(1, vec![], vec![(0, 100)]);
    storage.store_message(&msg).await.unwrap();

    // Store large metadata (1MB)
    let large_data = vec![0xFF; 1024 * 1024];
    storage
        .put_metadata(&msg.id, "large", &large_data)
        .await
        .unwrap();

    // Retrieve and verify
    let retrieved = storage.get_metadata(&msg.id, "large").await.unwrap();
    assert_eq!(retrieved, Some(large_data));
}

// ============= Optimized Query Tests =============

#[tokio::test]
async fn test_get_messages_in_time_range() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Create messages with different timestamps
    let base_time = Utc::now().timestamp_millis();
    let mut messages = vec![];

    for i in 0..10 {
        let mut msg = create_test_message_with_features(i, vec![], vec![(0, i)]);
        // Manually set timestamp for predictable testing
        msg.meta.timestamp =
            chrono::DateTime::from_timestamp_millis(base_time + (i as i64 * 1000)).unwrap();
        storage.store_message(&msg).await.unwrap();
        messages.push(msg);
    }

    // Test: Get messages in specific time range
    let start = base_time + 2000; // After message 2
    let end = base_time + 7000; // Before message 7

    let results = storage
        .backend()
        .get_messages_in_time_range(start, Some(end), 100)
        .await
        .unwrap();

    // Should get messages 2-6 (5 messages)
    assert_eq!(results.len(), 5);

    // Verify ordering (should be chronological)
    for i in 0..results.len() - 1 {
        let msg1 = storage.get_message(&results[i]).await.unwrap().unwrap();
        let msg2 = storage.get_message(&results[i + 1]).await.unwrap().unwrap();
        assert!(msg1.meta.timestamp <= msg2.meta.timestamp);
    }
}

#[tokio::test]
async fn test_get_messages_after_timestamp() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    let base_time = Utc::now().timestamp_millis();
    let mut messages = vec![];

    // Create 20 messages
    for i in 0..20 {
        let mut msg = create_test_message_with_features(i, vec![], vec![(0, i)]);
        msg.meta.timestamp =
            chrono::DateTime::from_timestamp_millis(base_time + (i as i64 * 1000)).unwrap();
        storage.store_message(&msg).await.unwrap();
        messages.push(msg);
    }

    // Test: Get messages after timestamp with limit
    let after_timestamp = base_time + 5000; // After message 5
    let limit = 5;

    let results = storage
        .backend()
        .get_messages_after_timestamp(after_timestamp, limit)
        .await
        .unwrap();

    assert_eq!(results.len(), 5);

    // All results should be at or after the specified timestamp
    for msg_id in &results {
        let msg = storage.get_message(msg_id).await.unwrap().unwrap();
        assert!(msg.meta.timestamp.timestamp_millis() >= after_timestamp);
    }
}

#[tokio::test]
async fn test_timestamp_index_ordering() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    // Create messages with random timestamps to test ordering
    let base_time = Utc::now().timestamp_millis();
    let timestamps = [
        base_time + 5000,
        base_time + 1000,
        base_time + 9000,
        base_time + 3000,
        base_time + 7000,
    ];

    for (i, ts) in timestamps.iter().enumerate() {
        let mut msg = create_test_message_with_features(i as u64, vec![], vec![(0, i as u64)]);
        msg.meta.timestamp = chrono::DateTime::from_timestamp_millis(*ts).unwrap();
        storage.store_message(&msg).await.unwrap();
    }

    // Get all messages in time range
    let results = storage
        .backend()
        .get_messages_in_time_range(base_time, Some(base_time + 10000), 100)
        .await
        .unwrap();

    assert_eq!(results.len(), 5);

    // Verify they are in chronological order
    let mut prev_timestamp = 0i64;
    for msg_id in &results {
        let msg = storage.get_message(msg_id).await.unwrap().unwrap();
        let current_timestamp = msg.meta.timestamp.timestamp_millis();
        assert!(current_timestamp >= prev_timestamp);
        prev_timestamp = current_timestamp;
    }
}

#[tokio::test]
async fn test_empty_range_queries() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    let base_time = Utc::now().timestamp_millis();

    // Store a few messages
    for i in 0..3 {
        let mut msg = create_test_message_with_features(i, vec![], vec![(0, i)]);
        msg.meta.timestamp =
            chrono::DateTime::from_timestamp_millis(base_time + (i as i64 * 1000)).unwrap();
        storage.store_message(&msg).await.unwrap();
    }

    // Test: Query range with no messages
    let results = storage
        .backend()
        .get_messages_in_time_range(base_time - 10000, Some(base_time - 5000), 100)
        .await
        .unwrap();

    assert_eq!(results.len(), 0);

    // Test: Query after all messages
    let results = storage
        .backend()
        .get_messages_after_timestamp(base_time + 10000, 10)
        .await
        .unwrap();

    assert_eq!(results.len(), 0);
}

#[tokio::test]
async fn test_large_range_query_performance() {
    use std::time::Instant;

    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    let base_time = Utc::now().timestamp_millis();

    // Store 1000 messages
    for i in 0..1000 {
        let mut msg = create_test_message_with_features(i, vec![], vec![(0, i)]);
        msg.meta.timestamp =
            chrono::DateTime::from_timestamp_millis(base_time + (i as i64 * 100)).unwrap();
        storage.store_message(&msg).await.unwrap();
    }

    // Measure query time for middle 100 messages
    let start_query = Instant::now();
    let results = storage
        .backend()
        .get_messages_in_time_range(
            base_time + 45000,       // Skip first 450 messages
            Some(base_time + 55000), // Get next 100 messages
            100,
        )
        .await
        .unwrap();
    let query_duration = start_query.elapsed();

    // Should get approximately 100 messages (depends on exact timing)
    assert!(results.len() >= 95 && results.len() <= 105);

    // Query should be fast (< 50ms even with 1000 messages)
    assert!(
        query_duration.as_millis() < 50,
        "Query took {:?}, expected < 50ms",
        query_duration
    );

    // Verify we got the right range
    for msg_id in &results {
        let msg = storage.get_message(msg_id).await.unwrap().unwrap();
        let ts = msg.meta.timestamp.timestamp_millis();
        assert!(ts >= base_time + 45000 && ts <= base_time + 55000);
    }
}

#[tokio::test]
async fn test_pagination_with_cursor() {
    let config = StorageConfig {
        backend_type: BackendType::Memory,
        ..Default::default()
    };
    let storage = StorageEngine::new(config).unwrap();

    let base_time = Utc::now().timestamp_millis();
    let mut all_messages = vec![];

    // Store 15 messages
    for i in 0..15 {
        let mut msg = create_test_message_with_features(i, vec![], vec![(0, i)]);
        msg.meta.timestamp =
            chrono::DateTime::from_timestamp_millis(base_time + (i as i64 * 1000)).unwrap();
        storage.store_message(&msg).await.unwrap();
        all_messages.push(msg.id);
    }

    // Test pagination with limit of 5
    let page_size = 5;
    let mut collected: Vec<MessageId> = vec![];

    // Page 1
    let page1 = storage
        .backend()
        .get_messages_after_timestamp(base_time - 1000, page_size)
        .await
        .unwrap();
    assert_eq!(page1.len(), 5);
    collected.extend(&page1);

    // Page 2 - use timestamp after last message from page 1 as cursor
    let last_msg = storage.get_message(&page1[4]).await.unwrap().unwrap();
    let cursor_timestamp = last_msg.meta.timestamp.timestamp_millis();

    let page2 = storage
        .backend()
        .get_messages_after_timestamp(cursor_timestamp + 1, page_size)
        .await
        .unwrap();
    assert_eq!(page2.len(), 5);
    collected.extend(&page2);

    // Page 3
    let last_msg = storage.get_message(&page2[4]).await.unwrap().unwrap();
    let cursor_timestamp = last_msg.meta.timestamp.timestamp_millis();

    let page3 = storage
        .backend()
        .get_messages_after_timestamp(cursor_timestamp + 1, page_size)
        .await
        .unwrap();
    assert_eq!(page3.len(), 5);
    collected.extend(&page3);

    // Verify we got all unique messages
    let unique: HashSet<_> = collected.iter().collect();
    assert_eq!(unique.len(), 15);
}
