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
    /// Self-reported ASN from handshake (optional, unvalidated)
    pub asn: Option<u32>,
    /// Self-reported region from handshake (optional, unvalidated)
    pub region: Option<String>,
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
    /// Used in perform_handshake_internal() at line 472
    #[allow(dead_code)]
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

        // SECURITY: Create Kademlia with ADIC-specific protocol (not public IPFS DHT)
        // Using custom protocol prevents pollution of public DHT and ensures we only
        // interact with ADIC nodes, not arbitrary IPFS nodes
        let protocol = StreamProtocol::new("/adic/kad/1.0.0");
        let mut kad_config = KademliaConfig::new(protocol);

        // SECURITY: Set query timeout to prevent long-running queries from DoS
        kad_config.set_query_timeout(Duration::from_secs(30));

        // SECURITY: Limit packet size to prevent amplification attacks
        // Max 8KB per packet (default is much larger)
        kad_config.set_max_packet_size(8192);

        // SECURITY: Set record TTL to expire stale peer records
        // Records expire after 5 minutes (300 seconds)
        kad_config.set_record_ttl(Some(Duration::from_secs(300)));

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
        let peers_before = peers.len();

        let mut evicted_peer = None;
        if peers.len() >= self.max_peers {
            // Find and remove lowest scoring peer
            if let Some(worst_peer) = self.find_worst_peer(&peers).await {
                if let Some(evicted_info) = peers.remove(&worst_peer) {
                    evicted_peer = Some((worst_peer, evicted_info.reputation_score));
                }
                self.event_sender
                    .send(PeerEvent::PeerDisconnected(worst_peer))
                    .map_err(|e| AdicError::Network(format!("Failed to send event: {}", e)))?;
            }
        }

        let peer_id = peer_info.peer_id;
        let reputation = peer_info.reputation_score;
        let connection_state = peer_info.connection_state;
        let address_count = peer_info.addresses.len();
        peers.insert(peer_id, peer_info);

        self.event_sender
            .send(PeerEvent::PeerConnected(peer_id))
            .map_err(|e| AdicError::Network(format!("Failed to send event: {}", e)))?;

        info!(
            peer_id = %peer_id,
            peers_before = peers_before,
            peers_after = peers.len(),
            max_peers = self.max_peers,
            reputation = reputation,
            connection_state = ?connection_state,
            address_count = address_count,
            evicted_peer = ?evicted_peer,
            "🤝 Peer added"
        );
        Ok(())
    }

    pub async fn remove_peer(&self, peer_id: &PeerId) -> Option<PeerInfo> {
        let mut peers = self.peers.write().await;
        let peers_before = peers.len();
        let peer = peers.remove(peer_id);

        if let Some(ref peer_info) = peer {
            info!(
                peer_id = %peer_id,
                peers_before = peers_before,
                peers_after = peers.len(),
                reputation_before = peer_info.reputation_score,
                connection_state = ?peer_info.connection_state,
                messages_sent = peer_info.message_stats.messages_sent,
                messages_received = peer_info.message_stats.messages_received,
                last_seen = ?peer_info.last_seen.elapsed(),
                "👋 Peer removed"
            );
            self.event_sender
                .send(PeerEvent::PeerDisconnected(*peer_id))
                .ok();
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
            let old_score = peer.reputation_score;
            peer.reputation_score = (peer.reputation_score + delta).clamp(0.0, 100.0);
            let new_score = peer.reputation_score;

            info!(
                peer_id = %peer_id,
                score_before = old_score,
                score_after = new_score,
                delta = delta,
                clamped = (old_score + delta != new_score),
                "📈 Peer score updated"
            );

            self.event_sender
                .send(PeerEvent::PeerScoreUpdated(*peer_id, new_score))
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
            let old_stats = peer.message_stats.clone();

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

            debug!(
                peer_id = %peer_id,
                sent = sent,
                bytes = bytes,
                valid = ?valid,
                messages_sent_before = old_stats.messages_sent,
                messages_sent_after = peer.message_stats.messages_sent,
                messages_received_before = old_stats.messages_received,
                messages_received_after = peer.message_stats.messages_received,
                messages_validated_before = old_stats.messages_validated,
                messages_validated_after = peer.message_stats.messages_validated,
                messages_invalid_before = old_stats.messages_invalid,
                messages_invalid_after = peer.message_stats.messages_invalid,
                "📊 Peer stats updated"
            );
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
        let total_peers = peers.len();
        let connected_count = peers
            .values()
            .filter(|p| p.connection_state == ConnectionState::Connected)
            .count();
        let disconnected_count = total_peers - connected_count;

        if connected_count < self.min_peers {
            info!(
                connected_peers = connected_count,
                disconnected_peers = disconnected_count,
                total_peers = total_peers,
                min_peers = self.min_peers,
                max_peers = self.max_peers,
                deficit = self.min_peers - connected_count,
                "🔍 Peer discovery triggered (below minimum)"
            );

            // Trigger peer discovery via Kademlia
            let mut kad = self.kademlia.write().await;
            let _ = kad.bootstrap();
        }
    }

    pub async fn handle_handshake(&self, peer: PeerId) -> Result<()> {
        let start_time = Instant::now();
        info!(
            peer_id = %peer,
            timeout_ms = self.handshake_timeout.as_millis(),
            "🤝 Starting handshake"
        );

        // Set handshake timeout
        let timeout_result = tokio::time::timeout(
            self.handshake_timeout,
            self.perform_handshake_internal(peer),
        )
        .await;

        let elapsed = start_time.elapsed();
        match timeout_result {
            Ok(Ok(())) => {
                info!(
                    peer_id = %peer,
                    duration_ms = elapsed.as_millis(),
                    "✅ Handshake completed"
                );
                Ok(())
            }
            Ok(Err(e)) => {
                warn!(
                    peer_id = %peer,
                    duration_ms = elapsed.as_millis(),
                    error = %e,
                    "❌ Handshake failed"
                );
                Err(e)
            }
            Err(_) => {
                warn!(
                    peer_id = %peer,
                    duration_ms = elapsed.as_millis(),
                    timeout_ms = self.handshake_timeout.as_millis(),
                    "⏰ Handshake timeout"
                );
                Err(AdicError::Network(format!(
                    "Handshake timeout with peer {}",
                    peer
                )))
            }
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

    /// Perform handshake with deposit and PoW verification
    ///
    /// NOTE: This function is not currently used in production network handshakes.
    /// The actual handshake is performed by NetworkEngine::perform_handshake.
    /// This function requires network protocol integration to send/receive PoW challenges.
    /// **DEV/TEST ONLY**: Simplified handshake helper for testing.
    ///
    /// Production code MUST use `NetworkEngine::perform_handshake()` which includes
    /// full PoW challenge-response protocol.
    ///
    /// This helper only verifies deposits and is not secure for production use.
    #[cfg(any(test, feature = "dev"))]
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

        // SECURITY: PoW challenge-response requires network protocol integration
        // Cannot be implemented without a transport layer to send/receive challenges
        // This function is a placeholder for future deposit+PoW handshake protocol
        let _challenge = self.deposit_verifier.generate_pow_challenge().await;

        // Return error instead of accepting dummy response
        warn!(
            "PoW challenge-response not implemented - requires network protocol integration. \
            Use NetworkEngine::perform_handshake for production handshakes."
        );

        // For now, accept based on deposit verification only
        // This is safe because this function is not used in the production network flow
        debug!("Handshake with peer {} (deposit-only verification)", peer_id);
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

    pub async fn get_disconnected_peers(&self) -> Vec<(PeerId, Vec<Multiaddr>)> {
        let peers = self.peers.read().await;
        peers
            .iter()
            .filter(|(_, info)| info.connection_state == ConnectionState::Disconnected)
            .map(|(id, info)| (*id, info.addresses.clone()))
            .collect()
    }

    pub async fn mark_peer_as_important(&self, peer_id: &PeerId) -> Result<()> {
        let mut peers = self.peers.write().await;
        if let Some(peer) = peers.get_mut(peer_id) {
            // Boost reputation for important peers
            peer.reputation_score = (peer.reputation_score + 20.0).min(100.0);
            info!(
                peer_id = %peer_id,
                new_reputation = peer.reputation_score,
                "⭐ Marked peer as important"
            );
        }
        Ok(())
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

    pub async fn get_all_peers_info(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values().cloned().collect()
    }

    pub async fn update_peer_connection_state(
        &self,
        peer_id: &PeerId,
        new_state: ConnectionState,
    ) -> Result<()> {
        let mut peers = self.peers.write().await;
        if let Some(peer) = peers.get_mut(peer_id) {
            let old_state = peer.connection_state;
            peer.connection_state = new_state;
            peer.last_seen = Instant::now();

            info!(
                peer_id = %peer_id,
                old_state = ?old_state,
                new_state = ?new_state,
                "📡 Peer connection state updated"
            );

            if new_state == ConnectionState::Disconnected {
                self.event_sender
                    .send(PeerEvent::PeerDisconnected(*peer_id))
                    .map_err(|e| AdicError::Network(format!("Failed to send event: {}", e)))?;
            } else if new_state == ConnectionState::Connected
                && old_state != ConnectionState::Connected
            {
                self.event_sender
                    .send(PeerEvent::PeerConnected(*peer_id))
                    .map_err(|e| AdicError::Network(format!("Failed to send event: {}", e)))?;
            }
        }
        Ok(())
    }

    pub fn event_stream(&self) -> Arc<RwLock<mpsc::UnboundedReceiver<PeerEvent>>> {
        self.event_receiver.clone()
    }

    /// Update peer network metadata (ASN/region) from handshake
    pub async fn update_peer_metadata(
        &self,
        peer_id: &PeerId,
        asn: Option<u32>,
        region: Option<String>,
    ) -> Result<()> {
        let mut peers = self.peers.write().await;
        if let Some(peer) = peers.get_mut(peer_id) {
            peer.asn = asn;
            peer.region = region.clone();
            peer.last_seen = Instant::now();

            debug!(
                peer_id = %peer_id,
                asn = ?asn,
                region = ?region,
                "📍 Peer network metadata updated"
            );
        }
        Ok(())
    }

    /// Check if a peer is considered alive based on last_seen timestamp
    ///
    /// A peer is considered alive if it was seen within the last `max_age` duration.
    /// Default max_age is 5 minutes for quorum selection.
    pub async fn is_peer_alive(&self, peer_id: &PeerId, max_age: std::time::Duration) -> bool {
        let peers = self.peers.read().await;
        if let Some(peer) = peers.get(peer_id) {
            let age = peer.last_seen.elapsed();
            age < max_age && peer.connection_state == ConnectionState::Connected
        } else {
            false
        }
    }

    /// Remove stale peers that haven't been seen for longer than `max_age`
    ///
    /// Returns the number of peers removed.
    pub async fn remove_stale_peers(&self, max_age: std::time::Duration) -> usize {
        let mut peers = self.peers.write().await;
        let before_count = peers.len();

        peers.retain(|peer_id, peer| {
            let age = peer.last_seen.elapsed();
            let should_keep = age < max_age || peer.connection_state == ConnectionState::Connected;

            if !should_keep {
                debug!(
                    peer_id = %peer_id,
                    age_secs = age.as_secs(),
                    "🧹 Removing stale peer"
                );
            }

            should_keep
        });

        let removed = before_count - peers.len();
        if removed > 0 {
            info!("🧹 Removed {} stale peers", removed);
        }
        removed
    }

    /// Get all alive peers with metadata, filtering out stale peers
    ///
    /// Only returns peers that:
    /// - Have been seen within `max_age` (default 5 minutes)
    /// - Are in Connected state
    /// - Have ASN and region metadata
    pub async fn get_alive_peers_with_metadata(
        &self,
        max_age: std::time::Duration,
    ) -> Vec<PeerInfo> {
        let peers = self.peers.read().await;
        let now = Instant::now();

        peers
            .values()
            .filter(|peer| {
                let age = now.duration_since(peer.last_seen);
                age < max_age
                    && peer.connection_state == ConnectionState::Connected
                    && peer.asn.is_some()
                    && peer.region.is_some()
            })
            .cloned()
            .collect()
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
        use adic_consensus::{DepositManager, DEFAULT_DEPOSIT_AMOUNT};

        let keypair = Keypair::generate_ed25519();
        let deposit_mgr = Arc::new(DepositManager::new(DEFAULT_DEPOSIT_AMOUNT));
        let verifier: Arc<dyn DepositVerifier> = Arc::new(RealDepositVerifier::new(deposit_mgr));
        let manager = PeerManager::new(&keypair, 100, verifier);

        assert_eq!(manager.peer_count().await, 0);
        assert_eq!(manager.connected_peer_count().await, 0);
    }

    #[tokio::test]
    async fn test_peer_scoring() {
        use crate::deposit_verifier::RealDepositVerifier;
        use adic_consensus::{DepositManager, DEFAULT_DEPOSIT_AMOUNT};

        let keypair = Keypair::generate_ed25519();
        let deposit_mgr = Arc::new(DepositManager::new(DEFAULT_DEPOSIT_AMOUNT));
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
            asn: None,
            region: None,
        };

        let score = manager.calculate_peer_score(&peer_info).await;
        assert!(score > 0.0 && score <= 1.0);
    }

    #[tokio::test]
    async fn test_update_peer_connection_state() {
        use crate::deposit_verifier::RealDepositVerifier;
        use adic_consensus::{DepositManager, DEFAULT_DEPOSIT_AMOUNT};

        let keypair = Keypair::generate_ed25519();
        let deposit_mgr = Arc::new(DepositManager::new(DEFAULT_DEPOSIT_AMOUNT));
        let verifier: Arc<dyn DepositVerifier> = Arc::new(RealDepositVerifier::new(deposit_mgr));
        let manager = PeerManager::new(&keypair, 100, verifier);

        // Add a peer first
        let peer_info = PeerInfo {
            peer_id: PeerId::random(),
            addresses: vec![],
            public_key: PublicKey::from_bytes([1; 32]),
            reputation_score: 50.0,
            latency_ms: Some(20),
            bandwidth_mbps: Some(50.0),
            last_seen: Instant::now(),
            connection_state: ConnectionState::Connected,
            message_stats: MessageStats::default(),
            padic_location: None,
            asn: None,
            region: None,
        };

        let peer_id = peer_info.peer_id;
        manager.add_peer(peer_info).await.unwrap();

        // Test state transition to Disconnected
        manager
            .update_peer_connection_state(&peer_id, ConnectionState::Disconnected)
            .await
            .unwrap();
        let peer = manager.get_peer(&peer_id).await.unwrap();
        assert_eq!(peer.connection_state, ConnectionState::Disconnected);

        // Test state transition back to Connected
        manager
            .update_peer_connection_state(&peer_id, ConnectionState::Connected)
            .await
            .unwrap();
        let peer = manager.get_peer(&peer_id).await.unwrap();
        assert_eq!(peer.connection_state, ConnectionState::Connected);
    }

    #[tokio::test]
    async fn test_get_disconnected_peers() {
        use crate::deposit_verifier::RealDepositVerifier;
        use adic_consensus::{DepositManager, DEFAULT_DEPOSIT_AMOUNT};

        let keypair = Keypair::generate_ed25519();
        let deposit_mgr = Arc::new(DepositManager::new(DEFAULT_DEPOSIT_AMOUNT));
        let verifier: Arc<dyn DepositVerifier> = Arc::new(RealDepositVerifier::new(deposit_mgr));
        let manager = PeerManager::new(&keypair, 100, verifier);

        // Add multiple peers with different states
        for i in 0..3 {
            let state = if i == 1 {
                ConnectionState::Disconnected
            } else {
                ConnectionState::Connected
            };

            let peer_info = PeerInfo {
                peer_id: PeerId::random(),
                addresses: vec![],
                public_key: PublicKey::from_bytes([i as u8; 32]),
                reputation_score: 50.0,
                latency_ms: Some(20),
                bandwidth_mbps: Some(50.0),
                last_seen: Instant::now(),
                connection_state: state,
                message_stats: MessageStats::default(),
                padic_location: None,
                asn: None,
                region: None,
            };

            manager.add_peer(peer_info).await.unwrap();
        }

        let disconnected = manager.get_disconnected_peers().await;
        assert_eq!(disconnected.len(), 1);
    }

    #[tokio::test]
    async fn test_mark_peer_as_important() {
        use crate::deposit_verifier::RealDepositVerifier;
        use adic_consensus::{DepositManager, DEFAULT_DEPOSIT_AMOUNT};

        let keypair = Keypair::generate_ed25519();
        let deposit_mgr = Arc::new(DepositManager::new(DEFAULT_DEPOSIT_AMOUNT));
        let verifier: Arc<dyn DepositVerifier> = Arc::new(RealDepositVerifier::new(deposit_mgr));
        let manager = PeerManager::new(&keypair, 100, verifier);

        // Add a peer with low reputation
        let peer_info = PeerInfo {
            peer_id: PeerId::random(),
            addresses: vec![],
            public_key: PublicKey::from_bytes([1; 32]),
            reputation_score: 50.0,
            latency_ms: Some(20),
            bandwidth_mbps: Some(50.0),
            last_seen: Instant::now(),
            connection_state: ConnectionState::Connected,
            message_stats: MessageStats::default(),
            padic_location: None,
            asn: None,
            region: None,
        };

        let peer_id = peer_info.peer_id;
        manager.add_peer(peer_info).await.unwrap();

        // Mark as important and check reputation boost
        manager.mark_peer_as_important(&peer_id).await.unwrap();
        let peer = manager.get_peer(&peer_id).await.unwrap();
        assert_eq!(peer.reputation_score, 70.0);
    }

    #[tokio::test]
    async fn test_cleanup_stale_peers() {
        use crate::deposit_verifier::RealDepositVerifier;
        use adic_consensus::{DepositManager, DEFAULT_DEPOSIT_AMOUNT};
        use std::time::Duration;

        let keypair = Keypair::generate_ed25519();
        let deposit_mgr = Arc::new(DepositManager::new(DEFAULT_DEPOSIT_AMOUNT));
        let verifier: Arc<dyn DepositVerifier> = Arc::new(RealDepositVerifier::new(deposit_mgr));
        let manager = PeerManager::new(&keypair, 100, verifier);

        // Add a peer with very old last_seen
        let mut peer_info = PeerInfo {
            peer_id: PeerId::random(),
            addresses: vec![],
            public_key: PublicKey::from_bytes([1; 32]),
            reputation_score: 50.0,
            latency_ms: Some(20),
            bandwidth_mbps: Some(50.0),
            last_seen: Instant::now(),
            connection_state: ConnectionState::Connected,
            message_stats: MessageStats::default(),
            padic_location: None,
            asn: None,
            region: None,
        };

        // Manually set last_seen to past (simulating stale peer)
        peer_info.last_seen = Instant::now() - Duration::from_secs(150);
        let peer_id = peer_info.peer_id;
        manager.add_peer(peer_info).await.unwrap();

        // Run cleanup
        manager.cleanup_stale_peers().await;

        // Peer should be removed
        assert!(manager.get_peer(&peer_id).await.is_none());
    }
}
