#![allow(dead_code)]
#![allow(clippy::assertions_on_constants)]

use adic_network::protocol::update::{UpdateMessage, VersionInfo};
use adic_network::protocol::update_protocol::{
    UpdateProtocol, UpdateProtocolConfig, UpdateProtocolEvent,
};
use libp2p::PeerId;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(test)]
mod update_tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn create_test_version_info() -> VersionInfo {
        VersionInfo {
            version: "0.2.0".to_string(),
            binary_hash: "abc123def456".to_string(),
            signature: vec![1, 2, 3, 4],
            chunk_count: 10,
            total_chunks: 10,
            release_timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            size_bytes: 1024 * 1024 * 10, // 10MB
        }
    }

    #[tokio::test]
    async fn test_update_protocol_initialization() {
        let config = UpdateProtocolConfig {
            max_concurrent_transfers: 5,
            chunk_timeout: Duration::from_secs(30),
            max_retries: 3,
            upload_rate_limit: 0,
            download_rate_limit: 0,
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let peer_id = PeerId::random();
        let (protocol, mut _event_receiver) =
            UpdateProtocol::new(config, temp_dir.path().to_path_buf(), peer_id).unwrap();
        // Protocol initialized successfully: assert initial state reasonable
        let all_versions = protocol.get_all_peer_versions().await;
        assert!(all_versions.is_empty(), "No versions should be tracked initially");
    }

    #[tokio::test]
    async fn test_version_announcement_handling() {
        let config = UpdateProtocolConfig {
            max_concurrent_transfers: 5,
            chunk_timeout: Duration::from_secs(30),
            max_retries: 3,
            upload_rate_limit: 0,
            download_rate_limit: 0,
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let peer_id = PeerId::random();
        let (protocol, mut event_receiver) =
            UpdateProtocol::new(config, temp_dir.path().to_path_buf(), peer_id).unwrap();
        let peer_id = PeerId::random();

        // Create a version announcement
        let msg = UpdateMessage::VersionAnnounce {
            version: "0.2.0".to_string(),
            binary_hash: "test_hash".to_string(),
            signature: vec![1, 2, 3],
            total_chunks: 5,
        };

        // Handle the announcement
        let _response = protocol.handle_message(msg, peer_id).await;

        // Check for version discovered event
        let got_event = tokio::time::timeout(Duration::from_secs(1), async {
            while let Some(event) = event_receiver.recv().await {
                if let UpdateProtocolEvent::VersionDiscovered(_, _) = event {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        assert!(got_event, "Should receive VersionDiscovered event");
    }

    #[tokio::test]
    async fn test_chunk_request() {
        let config = UpdateProtocolConfig {
            max_concurrent_transfers: 5,
            chunk_timeout: Duration::from_secs(30),
            max_retries: 3,
            upload_rate_limit: 0,
            download_rate_limit: 0,
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let peer_id = PeerId::random();
        let (protocol, _event_receiver) =
            UpdateProtocol::new(config, temp_dir.path().to_path_buf(), peer_id).unwrap();
        let peer_id = PeerId::random();

        // Request a chunk
        let result = protocol
            .request_chunk(peer_id, "0.2.0".to_string(), 0)
            .await;

        // Should succeed (even if chunk not available, request is sent)
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_chunk_transfer() {
        let config = UpdateProtocolConfig {
            max_concurrent_transfers: 5,
            chunk_timeout: Duration::from_secs(30),
            max_retries: 3,
            upload_rate_limit: 0,
            download_rate_limit: 0,
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let peer_id = PeerId::random();
        let (protocol, mut event_receiver) =
            UpdateProtocol::new(config, temp_dir.path().to_path_buf(), peer_id).unwrap();
        let peer_id = PeerId::random();

        // Simulate receiving a chunk
        let test_data = vec![0u8; 1024];
        let mut hasher = Sha256::new();
        hasher.update(&test_data);
        let chunk_hash = format!("{:x}", hasher.finalize());

        let msg = UpdateMessage::BinaryChunk {
            version: "0.2.0".to_string(),
            chunk_index: 0,
            total_chunks: 1,
            data: test_data.clone(),
            chunk_hash: chunk_hash.clone(),
        };

        // Handle the chunk
        let _response = protocol.handle_message(msg, peer_id).await;

        // Check for chunk download completed event
        let got_chunk_event = tokio::time::timeout(Duration::from_secs(1), async {
            while let Some(event) = event_receiver.recv().await {
                if let UpdateProtocolEvent::ChunkDownloadCompleted(_, _, _) = event {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        assert!(got_chunk_event, "Should receive ChunkDownloadCompleted event");
    }

    #[tokio::test]
    async fn test_version_query_response() {
        let config = UpdateProtocolConfig {
            max_concurrent_transfers: 5,
            chunk_timeout: Duration::from_secs(30),
            max_retries: 3,
            upload_rate_limit: 0,
            download_rate_limit: 0,
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let peer_id = PeerId::random();
        let (protocol, _event_receiver) =
            UpdateProtocol::new(config, temp_dir.path().to_path_buf(), peer_id).unwrap();
        let peer_id = PeerId::random();

        // First announce a version
        let announce_msg = UpdateMessage::VersionAnnounce {
            version: "0.2.0".to_string(),
            binary_hash: "hash123".to_string(),
            signature: vec![1, 2, 3],
            total_chunks: 10,
        };

        let _response = protocol.handle_message(announce_msg, peer_id).await;

        // Handle version query
        let query_msg = UpdateMessage::VersionQuery;
        let response = protocol.handle_message(query_msg, peer_id).await;

        // Should generate a response
        assert!(response.is_ok());
    }

    #[tokio::test]
    async fn test_peer_version_tracking() {
        let config = UpdateProtocolConfig {
            max_concurrent_transfers: 5,
            chunk_timeout: Duration::from_secs(30),
            max_retries: 3,
            upload_rate_limit: 0,
            download_rate_limit: 0,
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let peer_id = PeerId::random();
        let (protocol, _event_receiver) =
            UpdateProtocol::new(config, temp_dir.path().to_path_buf(), peer_id).unwrap();
        let peer1 = PeerId::random();
        let peer2 = PeerId::random();

        // Announce version from peer1
        let msg1 = UpdateMessage::VersionAnnounce {
            version: "0.2.0".to_string(),
            binary_hash: "hash1".to_string(),
            signature: vec![1, 2, 3],
            total_chunks: 10,
        };

        let _response = protocol.handle_message(msg1, peer1).await;

        // Announce different version from peer2
        let msg2 = UpdateMessage::VersionAnnounce {
            version: "0.3.0".to_string(),
            binary_hash: "hash2".to_string(),
            signature: vec![4, 5, 6],
            total_chunks: 15,
        };

        let _response = protocol.handle_message(msg2, peer2).await;

        // Check tracked peers
        let peers_with_v2 = protocol.get_peers_with_version("0.2.0").await;
        assert!(peers_with_v2.contains(&peer1));

        let peers_with_v3 = protocol.get_peers_with_version("0.3.0").await;
        assert!(peers_with_v3.contains(&peer2));
    }

    #[tokio::test]
    async fn test_update_completion() {
        let config = UpdateProtocolConfig {
            max_concurrent_transfers: 5,
            chunk_timeout: Duration::from_secs(30),
            max_retries: 3,
            upload_rate_limit: 0,
            download_rate_limit: 0,
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let peer_id = PeerId::random();
        let (protocol, _event_receiver) =
            UpdateProtocol::new(config, temp_dir.path().to_path_buf(), peer_id).unwrap();
        let peer_id = PeerId::random();

        // Test successful completion
        let success_msg = UpdateMessage::UpdateComplete {
            version: "0.2.0".to_string(),
            peer_id: peer_id.to_string(),
            success: true,
            error_message: None,
        };

        let response = protocol.handle_message(success_msg, peer_id).await;
        assert!(response.is_ok());

        // Test failed completion
        let failure_msg = UpdateMessage::UpdateComplete {
            version: "0.2.0".to_string(),
            peer_id: peer_id.to_string(),
            success: false,
            error_message: Some("Verification failed".to_string()),
        };

        let response = protocol.handle_message(failure_msg, peer_id).await;
        assert!(response.is_ok());
    }

    #[tokio::test]
    async fn test_concurrent_chunk_requests() {
        let config = UpdateProtocolConfig {
            max_concurrent_transfers: 3,
            chunk_timeout: Duration::from_secs(30),
            max_retries: 3,
            upload_rate_limit: 0,
            download_rate_limit: 0,
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let peer_id = PeerId::random();
        let (protocol, _event_receiver) =
            UpdateProtocol::new(config, temp_dir.path().to_path_buf(), peer_id).unwrap();
        let peer_id = PeerId::random();

        // Send multiple chunk requests sequentially
        // (Protocol doesn't implement Clone, so we can't test concurrent requests easily)
        for i in 0..5 {
            let result = protocol
                .request_chunk(peer_id, "0.2.0".to_string(), i)
                .await;
            assert!(result.is_ok());
        }
    }

    #[tokio::test]
    async fn test_all_peer_versions() {
        let config = UpdateProtocolConfig::default();
        let temp_dir = tempfile::tempdir().unwrap();
        let peer_id = PeerId::random();
        let (protocol, _event_receiver) =
            UpdateProtocol::new(config, temp_dir.path().to_path_buf(), peer_id).unwrap();

        let peer1 = PeerId::random();
        let peer2 = PeerId::random();
        let peer3 = PeerId::random();

        // Announce same version from multiple peers
        let msg = UpdateMessage::VersionAnnounce {
            version: "0.2.0".to_string(),
            binary_hash: "hash123".to_string(),
            signature: vec![1, 2, 3],
            total_chunks: 10,
        };

        let _response = protocol.handle_message(msg.clone(), peer1).await;
        let _response = protocol.handle_message(msg.clone(), peer2).await;
        let _response = protocol.handle_message(msg, peer3).await;

        // Get all peer versions
        let all_versions = protocol.get_all_peer_versions().await;

        assert!(all_versions.contains_key("0.2.0"));
        let peers = &all_versions["0.2.0"];
        assert_eq!(peers.len(), 3);
        assert!(peers.contains(&peer1));
        assert!(peers.contains(&peer2));
        assert!(peers.contains(&peer3));
    }
}
