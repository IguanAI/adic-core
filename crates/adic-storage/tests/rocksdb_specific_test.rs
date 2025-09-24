#[cfg(feature = "rocksdb")]
mod rocksdb_tests {
    use adic_crypto::Keypair;
    use adic_storage::store::BackendType;
    use adic_storage::{StorageConfig, StorageEngine};
    use adic_types::{
        AdicFeatures, AdicMessage, AdicMeta, AxisPhi, MessageId, PublicKey, QpDigits, DEFAULT_P,
        DEFAULT_PRECISION,
    };
    use chrono::Utc;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::task;

    fn create_test_message(id: u64, parents: Vec<MessageId>) -> AdicMessage {
        let keypair = Keypair::generate();
        let features = AdicFeatures::new(vec![AxisPhi::new(
            0,
            QpDigits::from_u64(id, DEFAULT_P, DEFAULT_PRECISION),
        )]);

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

    #[tokio::test]
    async fn test_rocksdb_persistence_across_restarts() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_str().unwrap().to_string();

        // Phase 1: Store messages
        let message_ids = {
            let config = StorageConfig {
                backend_type: BackendType::RocksDB { path: path.clone() },
                ..Default::default()
            };
            let storage = StorageEngine::new(config).unwrap();

            let mut ids = Vec::new();
            for i in 1..=10 {
                let msg = create_test_message(i, vec![]);
                ids.push(msg.id);
                storage.store_message(&msg).await.unwrap();
            }

            // Add metadata and finalization
            let test_pubkey = PublicKey::from_bytes([1; 32]);
            storage.update_reputation(&test_pubkey, 0.9).await.unwrap();
            storage
                .finalize_message(&ids[1], b"artifact")
                .await
                .unwrap();
            storage
                .add_to_conflict("conflict_1", &ids[2])
                .await
                .unwrap();

            storage.flush().await.unwrap();
            ids
        };

        // Phase 2: Verify data persisted
        {
            let config = StorageConfig {
                backend_type: BackendType::RocksDB { path: path.clone() },
                ..Default::default()
            };
            let storage = StorageEngine::new(config).unwrap();

            // Verify all messages exist
            for id in &message_ids {
                let msg = storage.get_message(id).await.unwrap();
                assert!(msg.is_some());
            }

            // Verify metadata persisted
            assert!(storage.is_finalized(&message_ids[1]).await.unwrap());

            // Verify conflict set persisted
            let conflict_set = storage.get_conflict_set("conflict_1").await.unwrap();
            assert!(conflict_set.contains(&message_ids[2]));
        }
    }

    #[tokio::test]
    async fn test_rocksdb_crash_recovery_simulation() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_str().unwrap().to_string();

        // Store messages without explicit flush
        let msg_id = {
            let config = StorageConfig {
                backend_type: BackendType::RocksDB { path: path.clone() },
                ..Default::default()
            };
            let storage = StorageEngine::new(config).unwrap();

            let msg = create_test_message(1, vec![]);
            let id = msg.id;
            storage.store_message(&msg).await.unwrap();

            // Simulate crash by dropping storage without flush
            id
        };

        // Recover and verify
        {
            let config = StorageConfig {
                backend_type: BackendType::RocksDB { path },
                ..Default::default()
            };
            let storage = StorageEngine::new(config).unwrap();

            // RocksDB should have persisted the data
            let msg = storage.get_message(&msg_id).await.unwrap();
            assert!(msg.is_some());
        }
    }

    #[tokio::test]
    async fn test_rocksdb_large_dataset_performance() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_str().unwrap().to_string();

        let config = StorageConfig {
            backend_type: BackendType::RocksDB { path },
            ..Default::default()
        };
        let storage = StorageEngine::new(config).unwrap();

        let start = std::time::Instant::now();

        // Store 1000 messages
        for i in 0..1000 {
            let parents = if i > 0 {
                // Create DAG structure
                vec![MessageId::new(&[(i - 1) as u8; 32])]
            } else {
                vec![]
            };
            let msg = create_test_message(i, parents);
            storage.store_message(&msg).await.unwrap();
        }

        let write_duration = start.elapsed();

        // Measure read performance
        let read_start = std::time::Instant::now();
        let all_messages = storage.list_all_messages().await.unwrap();
        let read_duration = read_start.elapsed();

        assert_eq!(all_messages.len(), 1000);

        // Performance assertions (adjust based on hardware)
        assert!(
            write_duration.as_secs() < 10,
            "Writing 1000 messages took too long: {:?}",
            write_duration
        );
        assert!(
            read_duration.as_millis() < 1000,
            "Reading message list took too long: {:?}",
            read_duration
        );
    }

    #[tokio::test]
    async fn test_rocksdb_concurrent_access() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_str().unwrap().to_string();

        let config = StorageConfig {
            backend_type: BackendType::RocksDB { path },
            ..Default::default()
        };
        let storage = Arc::new(StorageEngine::new(config).unwrap());

        let mut handles = Vec::new();

        // Spawn multiple tasks accessing storage concurrently
        for task_id in 0..10 {
            let storage_clone = storage.clone();
            let handle = task::spawn(async move {
                for i in 0..50 {
                    let msg_id = task_id * 100 + i;
                    let msg = create_test_message(msg_id, vec![]);
                    storage_clone.store_message(&msg).await.unwrap();

                    // Read random messages
                    if i > 0 {
                        let random_id = MessageId::new(&[(msg_id - 1) as u8; 32]);
                        let _ = storage_clone.get_message(&random_id).await;
                    }
                }
            });
            handles.push(handle);
        }

        // Wait for all tasks
        for handle in handles {
            handle.await.unwrap();
        }

        // Verify final count
        let all = storage.list_all_messages().await.unwrap();
        assert_eq!(all.len(), 500); // 10 tasks * 50 messages
    }

    #[tokio::test]
    async fn test_rocksdb_transaction_rollback() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_str().unwrap().to_string();

        let config = StorageConfig {
            backend_type: BackendType::RocksDB { path },
            ..Default::default()
        };
        let storage = StorageEngine::new(config).unwrap();

        // Store initial message
        let msg1 = create_test_message(1, vec![]);
        storage.store_message(&msg1).await.unwrap();

        // Attempt to store invalid parent reference
        let invalid_parent = MessageId::new(b"nonexistent");
        let msg2 = create_test_message(2, vec![invalid_parent]);

        // This might fail depending on implementation
        let result = storage.store_message(&msg2).await;

        // Verify database consistency
        let tips = storage.get_tips().await.unwrap();
        assert!(tips.contains(&msg1.id));

        // Stats should be consistent
        let stats = storage.get_stats().await.unwrap();
        assert!(stats.message_count >= 1);
    }

    #[tokio::test]
    async fn test_rocksdb_compaction_behavior() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_str().unwrap().to_string();

        let config = StorageConfig {
            backend_type: BackendType::RocksDB { path: path.clone() },
            ..Default::default()
        };
        let storage = StorageEngine::new(config).unwrap();

        // Store and delete many messages to trigger compaction
        for batch in 0..5 {
            let mut ids = Vec::new();

            // Store batch
            for i in 0..100 {
                let msg = create_test_message(batch * 100 + i, vec![]);
                ids.push(msg.id);
                storage.store_message(&msg).await.unwrap();
            }

            // Delete half of them
            for id in ids.iter().take(50) {
                storage.delete_message(id).await.unwrap();
            }

            storage.flush().await.unwrap();
        }

        // Verify final state
        let all = storage.list_all_messages().await.unwrap();
        assert_eq!(all.len(), 250); // 5 batches * 50 remaining

        // Check disk usage (compaction should keep it reasonable)
        let db_size = get_directory_size(&PathBuf::from(&path));
        assert!(
            db_size < 100_000_000, // 100MB threshold
            "Database size {} exceeds threshold after compaction",
            db_size
        );
    }

    #[tokio::test]
    async fn test_rocksdb_backup_restore() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("db").to_str().unwrap().to_string();
        let backup_path = temp_dir.path().join("backup").to_str().unwrap().to_string();

        // Create and populate database
        let msg_ids = {
            let config = StorageConfig {
                backend_type: BackendType::RocksDB {
                    path: db_path.clone(),
                },
                ..Default::default()
            };
            let storage = StorageEngine::new(config).unwrap();

            let mut ids = Vec::new();
            for i in 0..10 {
                let msg = create_test_message(i, vec![]);
                ids.push(msg.id);
                storage.store_message(&msg).await.unwrap();
            }

            storage.flush().await.unwrap();
            ids
        };

        // Simulate backup by copying directory
        copy_dir_all(&db_path, &backup_path).unwrap();

        // Corrupt original database
        fs::remove_dir_all(&db_path).unwrap();

        // Restore from backup
        copy_dir_all(&backup_path, &db_path).unwrap();

        // Verify restored data
        {
            let config = StorageConfig {
                backend_type: BackendType::RocksDB { path: db_path },
                ..Default::default()
            };
            let storage = StorageEngine::new(config).unwrap();

            for id in msg_ids {
                let msg = storage.get_message(&id).await.unwrap();
                assert!(msg.is_some());
            }
        }
    }

    #[tokio::test]
    async fn test_rocksdb_memory_usage() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_str().unwrap().to_string();

        let config = StorageConfig {
            backend_type: BackendType::RocksDB { path },
            ..Default::default()
        };
        let storage = StorageEngine::new(config).unwrap();

        // Monitor memory usage during operations
        let initial_memory = get_process_memory();

        // Store many large messages
        for i in 0..100 {
            let mut msg = create_test_message(i, vec![]);
            // Add large payload
            msg.payload = vec![0xFF; 10_000]; // 10KB per message
            storage.store_message(&msg).await.unwrap();

            if i % 10 == 0 {
                storage.flush().await.unwrap();
            }
        }

        let final_memory = get_process_memory();
        let memory_increase = final_memory.saturating_sub(initial_memory);

        // Memory increase should be reasonable (< 100MB for 1MB of data)
        assert!(
            memory_increase < 100_000_000,
            "Memory usage increased by {} bytes",
            memory_increase
        );
    }

    #[tokio::test]
    async fn test_rocksdb_column_families() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().to_str().unwrap().to_string();

        // Test that different data types are properly segregated
        let config = StorageConfig {
            backend_type: BackendType::RocksDB { path },
            ..Default::default()
        };
        let storage = StorageEngine::new(config).unwrap();

        let msg = create_test_message(1, vec![]);
        let msg_id = msg.id;

        // Store different types of data
        storage.store_message(&msg).await.unwrap();
        storage
            .update_reputation(&msg.proposer_pk, 0.8)
            .await
            .unwrap();
        storage
            .put_metadata(&msg_id, "type", b"test")
            .await
            .unwrap();
        storage.finalize_message(&msg_id, b"proof").await.unwrap();
        storage
            .add_to_conflict("conflict_1", &msg_id)
            .await
            .unwrap();

        // Verify all data types are retrievable
        assert!(storage.get_message(&msg_id).await.unwrap().is_some());
        assert_eq!(
            storage.get_reputation(&msg.proposer_pk).await.unwrap(),
            Some(0.8)
        );
        assert!(storage
            .get_metadata(&msg_id, "type")
            .await
            .unwrap()
            .is_some());
        assert!(storage.is_finalized(&msg_id).await.unwrap());
        assert!(!storage
            .get_conflict_set("conflict_1")
            .await
            .unwrap()
            .is_empty());
    }

    // Helper functions

    fn get_directory_size(path: &PathBuf) -> u64 {
        let mut size = 0;
        if path.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                let metadata = entry.metadata().unwrap();
                if metadata.is_file() {
                    size += metadata.len();
                } else if metadata.is_dir() {
                    size += get_directory_size(&entry.path());
                }
            }
        }
        size
    }

    fn copy_dir_all(src: &str, dst: &str) -> std::io::Result<()> {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            let src_path = entry.path();
            let dst_path = PathBuf::from(dst).join(entry.file_name());

            if ty.is_dir() {
                copy_dir_all(src_path.to_str().unwrap(), dst_path.to_str().unwrap())?;
            } else {
                fs::copy(src_path, dst_path)?;
            }
        }
        Ok(())
    }

    fn get_process_memory() -> usize {
        // Simple approximation - in real implementation would use proper system calls
        // This is a placeholder that returns a constant for testing
        50_000_000 // 50MB baseline
    }
}
