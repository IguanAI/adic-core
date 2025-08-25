#[cfg(test)]
mod tests {
    use crate::peer::{PeerInfo, PeerEvent, PeerScore, ConnectionState, MessageStats};
    use crate::deposit_verifier::{DepositVerifier, RealDepositVerifier};
    use libp2p::{identity::Keypair, PeerId};
    use adic_types::{PublicKey, features::QpDigits};
    use adic_math::distance::padic_distance;
    use std::time::Instant;
    use std::sync::Arc;
    
    fn create_test_peer_info(id: u8) -> PeerInfo {
        let keypair = Keypair::generate_ed25519();
        let peer_id = PeerId::from(keypair.public());
        
        PeerInfo {
            peer_id,
            addresses: vec![],
            public_key: PublicKey::from_bytes([id; 32]),
            reputation_score: 1.0,
            latency_ms: Some(50),
            bandwidth_mbps: Some(100.0),
            last_seen: Instant::now(),
            connection_state: ConnectionState::Connected,
            message_stats: MessageStats::default(),
            padic_location: Some(QpDigits::from_u64(id as u64, 3, 5)),
        }
    }
    
    #[test]
    fn test_connection_state() {
        assert_eq!(ConnectionState::Disconnected, ConnectionState::Disconnected);
        assert_ne!(ConnectionState::Connected, ConnectionState::Disconnected);
        assert_ne!(ConnectionState::Connecting, ConnectionState::Failed);
    }
    
    #[test]
    fn test_message_stats_default() {
        let stats = MessageStats::default();
        assert_eq!(stats.messages_sent, 0);
        assert_eq!(stats.messages_received, 0);
        assert_eq!(stats.messages_validated, 0);
        assert_eq!(stats.messages_invalid, 0);
        assert_eq!(stats.bytes_sent, 0);
        assert_eq!(stats.bytes_received, 0);
    }
    
    #[test]
    fn test_message_stats_updates() {
        let mut stats = MessageStats::default();
        
        stats.messages_sent += 10;
        stats.messages_received += 15;
        stats.messages_validated += 12;
        stats.messages_invalid += 3;
        stats.bytes_sent += 1024;
        stats.bytes_received += 2048;
        
        assert_eq!(stats.messages_sent, 10);
        assert_eq!(stats.messages_received, 15);
        assert_eq!(stats.messages_validated, 12);
        assert_eq!(stats.messages_invalid, 3);
        assert_eq!(stats.bytes_sent, 1024);
        assert_eq!(stats.bytes_received, 2048);
    }
    
    #[test]
    fn test_peer_score_default() {
        let score = PeerScore::default();
        
        assert_eq!(score.latency_weight, 0.25);
        assert_eq!(score.bandwidth_weight, 0.20);
        assert_eq!(score.reputation_weight, 0.30);
        assert_eq!(score.uptime_weight, 0.15);
        assert_eq!(score.validity_weight, 0.10);
        
        // Weights should sum to 1.0
        let total = score.latency_weight + score.bandwidth_weight + 
                   score.reputation_weight + score.uptime_weight + 
                   score.validity_weight;
        assert!((total - 1.0).abs() < 0.001);
    }
    
    #[test]
    fn test_peer_info_creation() {
        let peer = create_test_peer_info(1);
        
        assert_eq!(peer.reputation_score, 1.0);
        assert_eq!(peer.latency_ms, Some(50));
        assert_eq!(peer.bandwidth_mbps, Some(100.0));
        assert_eq!(peer.connection_state, ConnectionState::Connected);
        assert!(peer.padic_location.is_some());
    }
    
    #[test]
    fn test_peer_info_padic_distance() {
        let peer1 = create_test_peer_info(10);
        let peer2 = create_test_peer_info(11);
        
        let loc1 = peer1.padic_location.unwrap();
        let loc2 = peer2.padic_location.unwrap();
        
        let distance = padic_distance(&loc1, &loc2);
        assert!(distance > 0.0);
    }
    
    #[test]
    fn test_peer_score_calculation() {
        let score = PeerScore::default();
        let peer = create_test_peer_info(1);
        
        // Calculate weighted score
        let latency_score = if let Some(lat) = peer.latency_ms {
            1.0 / (1.0 + lat as f64 / 100.0) // Lower latency is better
        } else {
            0.5
        };
        
        let bandwidth_score = if let Some(bw) = peer.bandwidth_mbps {
            (bw / 100.0).min(1.0) // Normalize to 100 Mbps
        } else {
            0.5
        };
        
        let reputation_score = peer.reputation_score / 10.0; // Normalize to max 10
        
        let total_score = score.latency_weight * latency_score +
                         score.bandwidth_weight * bandwidth_score +
                         score.reputation_weight * reputation_score +
                         score.uptime_weight * 1.0 + // Assume full uptime for new peer
                         score.validity_weight * 1.0; // Assume no invalid messages yet
        
        assert!(total_score > 0.0);
        assert!(total_score <= 1.0);
    }
    
    #[test]
    fn test_peer_event_variants() {
        let keypair = Keypair::generate_ed25519();
        let peer_id = PeerId::from(keypair.public());
        
        let events = vec![
            PeerEvent::PeerDiscovered(peer_id, vec![]),
            PeerEvent::PeerConnected(peer_id),
            PeerEvent::PeerDisconnected(peer_id),
            PeerEvent::PeerScoreUpdated(peer_id, 0.75),
        ];
        
        for event in events {
            match event {
                PeerEvent::PeerDiscovered(id, _) => assert_eq!(id, peer_id),
                PeerEvent::PeerConnected(id) => assert_eq!(id, peer_id),
                PeerEvent::PeerDisconnected(id) => assert_eq!(id, peer_id),
                PeerEvent::PeerScoreUpdated(id, score) => {
                    assert_eq!(id, peer_id);
                    assert_eq!(score, 0.75);
                }
                PeerEvent::PeerMisbehaved(id, reason) => {
                    assert_eq!(id, peer_id);
                    assert!(!reason.is_empty());
                }
            }
        }
    }
    
    #[test]
    fn test_connection_state_transitions() {
        let mut peer = create_test_peer_info(1);
        
        // Test valid transitions
        peer.connection_state = ConnectionState::Disconnected;
        peer.connection_state = ConnectionState::Connecting;
        assert_eq!(peer.connection_state, ConnectionState::Connecting);
        
        peer.connection_state = ConnectionState::Connected;
        assert_eq!(peer.connection_state, ConnectionState::Connected);
        
        peer.connection_state = ConnectionState::Disconnected;
        assert_eq!(peer.connection_state, ConnectionState::Disconnected);
        
        peer.connection_state = ConnectionState::Failed;
        assert_eq!(peer.connection_state, ConnectionState::Failed);
    }
    
    #[tokio::test]
    async fn test_deposit_verifier_real() {
        let deposit_manager = Arc::new(adic_consensus::DepositManager::new(1.0));
        let verifier: Arc<dyn DepositVerifier> = Arc::new(RealDepositVerifier::new(deposit_manager.clone()));
        let keypair = Keypair::generate_ed25519();
        let peer_id = PeerId::from(keypair.public());
        
        // Create a test deposit
        let message_id = adic_types::MessageId::new(b"test");
        let proposer = PublicKey::from_bytes([1; 32]);
        deposit_manager.escrow(message_id, proposer).await.unwrap();
        
        // Create valid proof
        let mut proof = Vec::new();
        proof.extend_from_slice(proposer.as_bytes());
        proof.extend_from_slice(message_id.as_bytes());
        proof.extend_from_slice(&[0u8; 64]); // Dummy signature
        
        assert!(verifier.verify_deposit(&peer_id, &proof).await);
        
        let challenge = verifier.generate_pow_challenge().await;
        assert_eq!(challenge.len(), 32);
        
        // Test with invalid response (all 0xFF ensures hash won't have leading zeros)
        assert!(!verifier.verify_pow_response(&challenge, &[0xFF; 8]).await);
    }
}