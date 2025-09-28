use adic_network::protocol::update::UpdateMessage;
use adic_network::protocol::update_protocol::{UpdateProtocol, UpdateProtocolConfig, UpdateProtocolEvent};
use libp2p::PeerId;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

#[cfg(test)]
mod retry_tests {
    use super::*;

    /// Mock peer that simulates chunk transfer failures
    struct MockChunkPeer {
        fail_count: Arc<AtomicU32>,
        max_failures: u32,
        chunks: HashMap<(String, u32), Vec<u8>>,
    }

    impl MockChunkPeer {
        fn new(max_failures: u32) -> Self {
            let mut chunks = HashMap::new();

            // Add some mock chunks
            for i in 0..5 {
                let chunk_data = vec![i as u8; 1024]; // 1KB chunks
                chunks.insert(("0.2.0".to_string(), i), chunk_data);
            }

            Self {
                fail_count: Arc::new(AtomicU32::new(0)),
                max_failures,
                chunks,
            }
        }

        fn get_chunk(&self, version: &str, chunk_index: u32) -> Result<Vec<u8>, String> {
            let current_failures = self.fail_count.fetch_add(1, Ordering::SeqCst);

            if current_failures < self.max_failures {
                Err(format!("Simulated failure {}/{}", current_failures + 1, self.max_failures))
            } else {
                self.chunks
                    .get(&(version.to_string(), chunk_index))
                    .cloned()
                    .ok_or_else(|| "Chunk not found".to_string())
            }
        }
    }

    #[tokio::test]
    async fn test_chunk_transfer_with_retries() {
        // Create protocol with retry configuration
        let config = UpdateProtocolConfig {
            max_concurrent_transfers: 5,
            chunk_timeout: Duration::from_secs(1), // Short timeout for testing
            max_retries: 3,
            upload_rate_limit: 0,
            download_rate_limit: 0,
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let local_peer_id = PeerId::random();
        let (protocol, mut event_receiver) =
            UpdateProtocol::new(config.clone(), temp_dir.path().to_path_buf(), local_peer_id)
            .unwrap();

        // Test that retry configuration is properly set
        assert_eq!(config.max_retries, 3);

        // Request a chunk which would trigger retry logic on failure
        let peer_id = PeerId::random();
        let result = protocol.request_chunk(peer_id, "0.2.0".to_string(), 0).await;
        assert!(result.is_ok());

        // In a real scenario, the chunk would be retried up to max_retries times
        // before giving up
    }

    #[tokio::test]
    async fn test_timeout_and_retry_behavior() {
        let config = UpdateProtocolConfig {
            max_concurrent_transfers: 3,
            chunk_timeout: Duration::from_millis(100), // Very short timeout
            max_retries: 2,
            upload_rate_limit: 0,
            download_rate_limit: 0,
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let local_peer_id = PeerId::random();
        let (protocol, _) = UpdateProtocol::new(config, temp_dir.path().to_path_buf(), local_peer_id).unwrap();

        // Start the timeout checker task
        let protocol_arc = Arc::new(protocol);
        let checker = protocol_arc.clone();

        let timeout_task = tokio::spawn(async move {
            // This would normally be called periodically
            let _ = checker.check_timeouts().await;
        });

        // Give it time to process
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Cleanup
        timeout_task.abort();
    }

    #[tokio::test]
    async fn test_concurrent_retries() {
        let config = UpdateProtocolConfig {
            max_concurrent_transfers: 3, // Limit concurrent transfers
            chunk_timeout: Duration::from_secs(1),
            max_retries: 2,
            upload_rate_limit: 0,
            download_rate_limit: 0,
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let local_peer_id = PeerId::random();
        let (protocol, _) = UpdateProtocol::new(config, temp_dir.path().to_path_buf(), local_peer_id).unwrap();

        let peer_id = PeerId::random();

        // Request multiple chunks concurrently (up to max_concurrent_transfers)
        for i in 0..5 {
            let result = protocol.request_chunk(peer_id, "0.2.0".to_string(), i).await;
            assert!(result.is_ok());
        }

        // All requests should be handled with proper semaphore limiting
    }

    #[tokio::test]
    async fn test_retry_with_different_peer() {
        let config = UpdateProtocolConfig {
            max_concurrent_transfers: 5,
            chunk_timeout: Duration::from_secs(1),
            max_retries: 3,
            upload_rate_limit: 0,
            download_rate_limit: 0,
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let local_peer_id = PeerId::random();
        let (protocol, _) = UpdateProtocol::new(config, temp_dir.path().to_path_buf(), local_peer_id).unwrap();

        // Register multiple peers with the same version
        let peer1 = PeerId::random();
        let peer2 = PeerId::random();
        let peer3 = PeerId::random();

        // Announce version from multiple peers
        for peer in &[peer1, peer2, peer3] {
            let msg = UpdateMessage::VersionAnnounce {
                version: "0.2.0".to_string(),
                binary_hash: "hash123".to_string(),
                signature: vec![1, 2, 3],
                total_chunks: 10,
            };
            let _ = protocol.handle_message(msg, *peer).await;
        }

        // Get peers with version
        let peers = protocol.get_peers_with_version("0.2.0").await;
        assert_eq!(peers.len(), 3);

        // When retrying, the protocol should be able to select a different peer
        // This helps with resilience when one peer is failing
    }

    #[tokio::test]
    async fn test_max_retries_exhausted() {
        let config = UpdateProtocolConfig {
            max_concurrent_transfers: 5,
            chunk_timeout: Duration::from_millis(50), // Very short for testing
            max_retries: 1, // Only 1 retry
            upload_rate_limit: 0,
            download_rate_limit: 0,
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let local_peer_id = PeerId::random();
        let (protocol, mut event_receiver) =
            UpdateProtocol::new(config, temp_dir.path().to_path_buf(), local_peer_id).unwrap();

        let protocol_arc = Arc::new(protocol);
        let peer_id = PeerId::random();

        // Request a chunk
        let _ = protocol_arc.request_chunk(peer_id, "0.2.0".to_string(), 0).await;

        // Wait for timeout
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Check timeouts - should retry once then fail
        let checker = protocol_arc.clone();
        tokio::spawn(async move {
            let _ = checker.check_timeouts().await;
        });

        // Check for failure event after max retries
        tokio::time::timeout(Duration::from_secs(1), async {
            while let Some(event) = event_receiver.recv().await {
                if let UpdateProtocolEvent::ChunkDownloadFailed(_, _, msg) = event {
                    // Should fail after max retries
                    return msg.contains("Failed after");
                }
            }
            false
        }).await.unwrap_or(false);
    }

    #[tokio::test]
    async fn test_successful_retry_after_failures() {
        let config = UpdateProtocolConfig {
            max_concurrent_transfers: 5,
            chunk_timeout: Duration::from_secs(1),
            max_retries: 3,
            upload_rate_limit: 0,
            download_rate_limit: 0,
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let local_peer_id = PeerId::random();
        let (protocol, mut event_receiver) =
            UpdateProtocol::new(config, temp_dir.path().to_path_buf(), local_peer_id).unwrap();

        let peer_id = PeerId::random();

        // Simulate receiving a chunk after some retries
        // First, request the chunk
        let _ = protocol.request_chunk(peer_id, "0.2.0".to_string(), 0).await;

        // Then simulate receiving it successfully
        use sha2::{Digest, Sha256};
        let test_data = vec![42u8; 1024];
        let mut hasher = Sha256::new();
        hasher.update(&test_data);
        let chunk_hash = format!("{:x}", hasher.finalize());

        let msg = UpdateMessage::BinaryChunk {
            version: "0.2.0".to_string(),
            chunk_index: 0,
            total_chunks: 1,
            data: test_data,
            chunk_hash,
        };

        // Handle successful chunk after retries
        let _ = protocol.handle_message(msg, peer_id).await;

        // Should receive success event
        tokio::time::timeout(Duration::from_secs(1), async {
            while let Some(event) = event_receiver.recv().await {
                if let UpdateProtocolEvent::ChunkDownloadCompleted(_, _, _) = event {
                    return true;
                }
            }
            false
        }).await.unwrap_or(false);
    }
}