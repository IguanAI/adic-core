use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use libp2p::{
    identity::Keypair,
    kad::{store::MemoryStore, Behaviour as Kademlia, Config as KademliaConfig},
    Multiaddr, PeerId, StreamProtocol,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

use adic_math::distance::padic_distance;
use adic_types::{features::QpDigits, AdicError, PublicKey, Result};

#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub peer_id: PeerId,
    pub addresses: Vec<Multiaddr>,
    pub public_key: PublicKey,
    pub reputation_score: f64,
    pub latency_ms: Option<u64>,
    pub bandwidth_mbps: Option<f64>,
    pub last_seen: Instant,
    pub connection_state: ConnectionState,
    pub message_stats: MessageStats,
    pub padic_location: Option<QpDigits>, // p-adic coordinates for distance calculations
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Failed,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageStats {
    pub messages_sent: u64,
    pub messages_received: u64,
    pub messages_validated: u64,
    pub messages_invalid: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
}

#[derive(Debug, Clone)]
pub struct PeerScore {
    pub latency_weight: f64,
    pub bandwidth_weight: f64,
    pub reputation_weight: f64,
    pub uptime_weight: f64,
    pub validity_weight: f64,
}

impl Default for PeerScore {
    fn default() -> Self {
        Self {
            latency_weight: 0.25,
            bandwidth_weight: 0.20,
            reputation_weight: 0.30,
            uptime_weight: 0.15,
            validity_weight: 0.10,
        }
    }
}

pub struct PeerManager {
    local_peer_id: PeerId,
    peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    kademlia: Arc<RwLock<Kademlia<MemoryStore>>>,
    scoring: PeerScore,
    max_peers: usize,
    min_peers: usize,
    peer_timeout: Duration,
    handshake_timeout: Duration,
    deposit_verifier: Arc<dyn DepositVerifier>,
    event_sender: mpsc::UnboundedSender<PeerEvent>,
    event_receiver: Arc<RwLock<mpsc::UnboundedReceiver<PeerEvent>>>,
}

// DepositVerifier trait moved to deposit_verifier module
use crate::deposit_verifier::DepositVerifier;

#[derive(Debug, Clone)]
pub enum PeerEvent {
    PeerDiscovered(PeerId, Vec<Multiaddr>),
    PeerConnected(PeerId),
    PeerDisconnected(PeerId),
    PeerScoreUpdated(PeerId, f64),
    PeerMisbehaved(PeerId, String),
}

impl PeerManager {
    pub fn new(
        keypair: &Keypair,
        max_peers: usize,
        deposit_verifier: Arc<dyn DepositVerifier>,
    ) -> Self {
        let local_peer_id = PeerId::from(keypair.public());

        // Create Kademlia with default protocol name
        let protocol = StreamProtocol::new("/ipfs/kad/1.0.0");
        let mut kad_config = KademliaConfig::new(protocol);
        kad_config.set_query_timeout(Duration::from_secs(30));
        let store = MemoryStore::new(local_peer_id);
        let kademlia = Kademlia::with_config(local_peer_id, store, kad_config);

        let (event_sender, event_receiver) = mpsc::unbounded_channel();

        Self {
            local_peer_id,
            peers: Arc::new(RwLock::new(HashMap::new())),
            kademlia: Arc::new(RwLock::new(kademlia)),
            scoring: PeerScore::default(),
            max_peers,
            min_peers: max_peers / 4,
            peer_timeout: Duration::from_secs(120),
            handshake_timeout: Duration::from_secs(10),
            deposit_verifier,
            event_sender,
            event_receiver: Arc::new(RwLock::new(event_receiver)),
        }
    }

    pub async fn add_peer(&self, peer_info: PeerInfo) -> Result<()> {
        let mut peers = self.peers.write().await;

        if peers.len() >= self.max_peers {
            // Find and remove lowest scoring peer
            if let Some(worst_peer) = self.find_worst_peer(&peers).await {
                peers.remove(&worst_peer);
                self.event_sender
                    .send(PeerEvent::PeerDisconnected(worst_peer))
                    .map_err(|e| AdicError::Network(format!("Failed to send event: {}", e)))?;
            }
        }

        let peer_id = peer_info.peer_id;
        peers.insert(peer_id, peer_info);

        self.event_sender
            .send(PeerEvent::PeerConnected(peer_id))
            .map_err(|e| AdicError::Network(format!("Failed to send event: {}", e)))?;

        info!("Added peer: {}", peer_id);
        Ok(())
    }

    pub async fn remove_peer(&self, peer_id: &PeerId) -> Option<PeerInfo> {
        let mut peers = self.peers.write().await;
        let peer = peers.remove(peer_id);

        if peer.is_some() {
            self.event_sender
                .send(PeerEvent::PeerDisconnected(*peer_id))
                .ok();
            info!("Removed peer: {}", peer_id);
        }

        peer
    }

    pub async fn get_peer(&self, peer_id: &PeerId) -> Option<PeerInfo> {
        let peers = self.peers.read().await;
        peers.get(peer_id).cloned()
    }

    pub async fn update_peer_score(&self, peer_id: &PeerId, delta: f64) {
        let mut peers = self.peers.write().await;
        if let Some(peer) = peers.get_mut(peer_id) {
            peer.reputation_score = (peer.reputation_score + delta).clamp(0.0, 100.0);

            self.event_sender
                .send(PeerEvent::PeerScoreUpdated(*peer_id, peer.reputation_score))
                .ok();
        }
    }

    pub async fn update_peer_stats(
        &self,
        peer_id: &PeerId,
        sent: bool,
        bytes: u64,
        valid: Option<bool>,
    ) {
        let mut peers = self.peers.write().await;
        if let Some(peer) = peers.get_mut(peer_id) {
            if sent {
                peer.message_stats.messages_sent += 1;
                peer.message_stats.bytes_sent += bytes;
            } else {
                peer.message_stats.messages_received += 1;
                peer.message_stats.bytes_received += bytes;

                if let Some(is_valid) = valid {
                    if is_valid {
                        peer.message_stats.messages_validated += 1;
                    } else {
                        peer.message_stats.messages_invalid += 1;
                    }
                }
            }

            peer.last_seen = Instant::now();
        }
    }

    pub async fn calculate_peer_score(&self, peer_info: &PeerInfo) -> f64 {
        let mut score = 0.0;

        // Latency score (lower is better)
        if let Some(latency) = peer_info.latency_ms {
            let latency_score = 1.0 / (1.0 + latency as f64 / 100.0);
            score += latency_score * self.scoring.latency_weight;
        }

        // Bandwidth score (higher is better)
        if let Some(bandwidth) = peer_info.bandwidth_mbps {
            let bandwidth_score = bandwidth.min(100.0) / 100.0;
            score += bandwidth_score * self.scoring.bandwidth_weight;
        }

        // Reputation score
        let reputation_score = peer_info.reputation_score / 100.0;
        score += reputation_score * self.scoring.reputation_weight;

        // Uptime score
        let uptime = peer_info.last_seen.elapsed().as_secs();
        let uptime_score = 1.0 - (uptime as f64 / 3600.0).min(1.0);
        score += uptime_score * self.scoring.uptime_weight;

        // Validity score
        let total_messages =
            peer_info.message_stats.messages_validated + peer_info.message_stats.messages_invalid;
        if total_messages > 0 {
            let validity_score =
                peer_info.message_stats.messages_validated as f64 / total_messages as f64;
            score += validity_score * self.scoring.validity_weight;
        }

        score
    }

    pub fn local_peer_id(&self) -> &PeerId {
        &self.local_peer_id
    }

    pub async fn ensure_min_peers(&self) {
        let peers = self.peers.read().await;
        let connected_count = peers
            .values()
            .filter(|p| p.connection_state == ConnectionState::Connected)
            .count();

        if connected_count < self.min_peers {
            info!(
                "Connected peers ({}) below minimum ({}), discovering more",
                connected_count, self.min_peers
            );

            // Trigger peer discovery via Kademlia
            let mut kad = self.kademlia.write().await;
            let _ = kad.bootstrap();
        }
    }

    pub async fn handle_handshake(&self, peer: PeerId) -> Result<()> {
        // Set handshake timeout
        let timeout_result = tokio::time::timeout(
            self.handshake_timeout,
            self.perform_handshake_internal(peer),
        )
        .await;

        match timeout_result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(AdicError::Network(format!(
                "Handshake timeout with peer {}",
                peer
            ))),
        }
    }

    async fn perform_handshake_internal(&self, peer: PeerId) -> Result<()> {
        // Actual handshake logic would go here
        debug!("Performing handshake with peer {}", peer);
        Ok(())
    }

    pub async fn select_peers_weighted(&self, count: usize) -> Vec<PeerId> {
        let peers = self.peers.read().await;
        let mut peer_scores: Vec<(PeerId, f64)> = Vec::new();

        for (peer_id, peer_info) in peers.iter() {
            if peer_info.connection_state == ConnectionState::Connected {
                let score = self.calculate_peer_score(peer_info).await;
                peer_scores.push((*peer_id, score));
            }
        }

        peer_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        peer_scores
            .into_iter()
            .take(count)
            .map(|(peer_id, _)| peer_id)
            .collect()
    }

    pub async fn find_peers_by_padic_distance(
        &self,
        target: &QpDigits,
        count: usize,
    ) -> Vec<PeerId> {
        let peers = self.peers.read().await;
        let mut peer_distances: Vec<(PeerId, f64)> = Vec::new();

        for (peer_id, peer_info) in peers.iter() {
            if let Some(ref location) = peer_info.padic_location {
                let distance = padic_distance(location, target);
                peer_distances.push((*peer_id, distance));
            }
        }

        peer_distances.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        peer_distances
            .into_iter()
            .take(count)
            .map(|(peer_id, _)| peer_id)
            .collect()
    }

    async fn find_worst_peer(&self, peers: &HashMap<PeerId, PeerInfo>) -> Option<PeerId> {
        let mut worst_peer = None;
        let mut worst_score = f64::MAX;

        for (peer_id, peer_info) in peers.iter() {
            let score = self.calculate_peer_score(peer_info).await;
            if score < worst_score {
                worst_score = score;
                worst_peer = Some(*peer_id);
            }
        }

        worst_peer
    }

    pub async fn perform_handshake(&self, peer_id: &PeerId, deposit_proof: &[u8]) -> Result<bool> {
        // Verify deposit
        if !self
            .deposit_verifier
            .verify_deposit(peer_id, deposit_proof)
            .await
        {
            warn!("Peer {} failed deposit verification", peer_id);
            return Ok(false);
        }

        // Generate and verify PoW challenge
        let challenge = self.deposit_verifier.generate_pow_challenge().await;
        // In a real implementation, send challenge and receive response
        // For now, we'll simulate this
        let response = vec![0u8; 32]; // Placeholder

        if !self
            .deposit_verifier
            .verify_pow_response(&challenge, &response)
            .await
        {
            warn!("Peer {} failed PoW verification", peer_id);
            return Ok(false);
        }

        debug!("Handshake successful with peer {}", peer_id);
        Ok(true)
    }

    pub async fn cleanup_stale_peers(&self) {
        let mut peers = self.peers.write().await;
        let now = Instant::now();
        let mut to_remove = Vec::new();

        for (peer_id, peer_info) in peers.iter() {
            if now.duration_since(peer_info.last_seen) > self.peer_timeout {
                to_remove.push(*peer_id);
            }
        }

        for peer_id in to_remove {
            peers.remove(&peer_id);
            self.event_sender
                .send(PeerEvent::PeerDisconnected(peer_id))
                .ok();
            info!("Removed stale peer: {}", peer_id);
        }
    }

    pub async fn get_connected_peers(&self) -> Vec<PeerId> {
        let peers = self.peers.read().await;
        peers
            .iter()
            .filter(|(_, info)| info.connection_state == ConnectionState::Connected)
            .map(|(id, _)| *id)
            .collect()
    }

    pub async fn peer_count(&self) -> usize {
        let peers = self.peers.read().await;
        peers.len()
    }

    pub async fn connected_peer_count(&self) -> usize {
        let peers = self.peers.read().await;
        peers
            .iter()
            .filter(|(_, info)| info.connection_state == ConnectionState::Connected)
            .count()
    }

    pub fn event_stream(&self) -> Arc<RwLock<mpsc::UnboundedReceiver<PeerEvent>>> {
        self.event_receiver.clone()
    }
}

// DepositVerifier is now in deposit_verifier module

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::identity::Keypair;

    #[tokio::test]
    async fn test_peer_manager_creation() {
        use crate::deposit_verifier::RealDepositVerifier;
        use adic_consensus::DepositManager;

        let keypair = Keypair::generate_ed25519();
        let deposit_mgr = Arc::new(DepositManager::new(1.0));
        let verifier: Arc<dyn DepositVerifier> = Arc::new(RealDepositVerifier::new(deposit_mgr));
        let manager = PeerManager::new(&keypair, 100, verifier);

        assert_eq!(manager.peer_count().await, 0);
        assert_eq!(manager.connected_peer_count().await, 0);
    }

    #[tokio::test]
    async fn test_peer_scoring() {
        use crate::deposit_verifier::RealDepositVerifier;
        use adic_consensus::DepositManager;

        let keypair = Keypair::generate_ed25519();
        let deposit_mgr = Arc::new(DepositManager::new(1.0));
        let verifier: Arc<dyn DepositVerifier> = Arc::new(RealDepositVerifier::new(deposit_mgr));
        let manager = PeerManager::new(&keypair, 100, verifier);

        let peer_info = PeerInfo {
            peer_id: PeerId::random(),
            addresses: vec![],
            public_key: PublicKey::from_bytes([0; 32]),
            reputation_score: 50.0,
            latency_ms: Some(20),
            bandwidth_mbps: Some(50.0),
            last_seen: Instant::now(),
            connection_state: ConnectionState::Connected,
            message_stats: MessageStats {
                messages_validated: 90,
                messages_invalid: 10,
                ..Default::default()
            },
            padic_location: None,
        };

        let score = manager.calculate_peer_score(&peer_info).await;
        assert!(score > 0.0 && score <= 1.0);
    }
}
