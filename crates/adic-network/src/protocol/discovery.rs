use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::dns_seeds::DnsSeedDiscovery;
use adic_types::Result;

/// Configuration for peer discovery
#[derive(Clone, Debug)]
pub struct DiscoveryConfig {
    pub bootstrap_nodes: Vec<Multiaddr>,
    pub dns_seed_domains: Vec<String>,
    pub min_peers: usize,
    pub max_peers: usize,
    pub discovery_interval: Duration,
    pub peer_exchange_limit: usize,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            bootstrap_nodes: vec![],
            dns_seed_domains: vec!["_seeds.adicl1.com".to_string()],
            min_peers: 5,
            max_peers: 50,
            discovery_interval: Duration::from_secs(30),
            peer_exchange_limit: 10,
        }
    }
}

/// Message types for peer discovery protocol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DiscoveryMessage {
    /// Request for peer list
    GetPeers {
        requester: Vec<u8>, // PeerId as bytes
        limit: usize,
    },

    /// Response with peer list
    Peers { peers: Vec<PeerInfo> },

    /// Announce self to network
    Announce {
        peer_id: Vec<u8>,       // PeerId as bytes
        addresses: Vec<String>, // Multiaddr as strings
        capabilities: Vec<String>,
    },

    /// Ping to check liveness
    Ping { nonce: u64 },

    /// Pong response
    Pong { nonce: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub peer_id: Vec<u8>,       // PeerId as bytes
    pub addresses: Vec<String>, // Multiaddr as strings
    pub last_seen: i64,         // Unix timestamp
    pub reputation: f64,
}

/// Peer discovery protocol implementation
pub struct DiscoveryProtocol {
    config: DiscoveryConfig,
    local_peer_id: PeerId,
    known_peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    active_discoveries: Arc<RwLock<HashSet<PeerId>>>,
    last_discovery: Arc<RwLock<Instant>>,
    peer_manager: Option<Arc<crate::peer::PeerManager>>,
    pending_bootstrap_connections: Arc<RwLock<Vec<(PeerId, quinn::Connection)>>>,
}

impl DiscoveryProtocol {
    pub fn new(config: DiscoveryConfig, local_peer_id: PeerId) -> Self {
        Self {
            config,
            local_peer_id,
            known_peers: Arc::new(RwLock::new(HashMap::new())),
            active_discoveries: Arc::new(RwLock::new(HashSet::new())),
            last_discovery: Arc::new(RwLock::new(Instant::now())),
            peer_manager: None,
            pending_bootstrap_connections: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Get and clear pending bootstrap connections that need receivers started
    pub async fn take_pending_bootstrap_connections(&self) -> Vec<(PeerId, quinn::Connection)> {
        let mut pending = self.pending_bootstrap_connections.write().await;
        std::mem::take(&mut *pending)
    }

    pub fn set_peer_manager(&mut self, peer_manager: Arc<crate::peer::PeerManager>) {
        self.peer_manager = Some(peer_manager);
    }

    /// Start discovery process
    pub async fn discover_peers(
        &self,
        transport: &crate::transport::HybridTransport,
    ) -> Result<Vec<PeerInfo>> {
        let mut last_discovery = self.last_discovery.write().await;
        *last_discovery = Instant::now();

        let known_peers = self.known_peers.read().await;

        // If we don't have enough peers, try bootstrap nodes
        if known_peers.len() < self.config.min_peers {
            info!(
                "Starting peer discovery with {} known peers",
                known_peers.len()
            );
            self.query_bootstrap_nodes(transport).await?;
        }

        // Exchange peers with connected nodes
        let peers_to_query: Vec<PeerId> = known_peers
            .keys()
            .take(self.config.peer_exchange_limit)
            .cloned()
            .collect();

        drop(known_peers);

        let mut discovered_peers = Vec::new();
        for peer_id in peers_to_query {
            if let Ok(peers) = self.request_peers_from(peer_id, transport).await {
                discovered_peers.extend(peers);
            }
        }

        // Update our known peers
        for peer_info in &discovered_peers {
            self.add_peer(peer_info.clone()).await;
        }

        info!("Discovered {} new peers", discovered_peers.len());
        Ok(discovered_peers)
    }

    /// Discover bootstrap nodes from DNS seeds
    pub async fn discover_dns_seeds(&self) -> Result<Vec<Multiaddr>> {
        if self.config.dns_seed_domains.is_empty() {
            debug!("No DNS seed domains configured");
            return Ok(vec![]);
        }

        info!(
            "Discovering bootstrap nodes from {} DNS seed domains",
            self.config.dns_seed_domains.len()
        );

        let dns_discovery = DnsSeedDiscovery::new(self.config.dns_seed_domains.clone()).await?;
        let seeds = dns_discovery.discover_seeds().await;

        Ok(seeds)
    }

    /// Query bootstrap nodes for initial peer list
    pub async fn query_bootstrap_nodes(
        &self,
        transport: &crate::transport::HybridTransport,
    ) -> Result<()> {
        // First, try to discover bootstrap nodes from DNS
        let mut all_bootstrap_nodes = match self.discover_dns_seeds().await {
            Ok(dns_seeds) => {
                if !dns_seeds.is_empty() {
                    info!("Using {} bootstrap nodes from DNS seeds", dns_seeds.len());
                }
                dns_seeds
            }
            Err(e) => {
                warn!("DNS seed discovery failed: {}", e);
                vec![]
            }
        };

        // Add configured bootstrap nodes (these take priority and are added after DNS seeds)
        all_bootstrap_nodes.extend(self.config.bootstrap_nodes.clone());

        if all_bootstrap_nodes.is_empty() {
            warn!("No bootstrap nodes available (neither from DNS nor configuration)");
            return Ok(());
        }

        info!(
            "Attempting to connect to {} total bootstrap nodes",
            all_bootstrap_nodes.len()
        );

        for addr in &all_bootstrap_nodes {
            // Skip if this is our own address
            if self.is_self_address(addr) {
                debug!(
                    local_peer = %self.local_peer_id,
                    bootstrap_addr = %addr,
                    "🔄 Skipping self-connection"
                );
                continue;
            }

            debug!(
                bootstrap_addr = %addr,
                "🔍 Attempting bootstrap connection"
            );

            // Parse the multiaddr to get the socket address
            if let Some(socket_addr) = self.parse_multiaddr_to_socket(addr) {
                // Check if we already have a connection to any peer at this address
                let pool = transport.connection_pool();
                let existing_connections = pool.get_all_connections().await;

                // Check if we already have an active connection to this bootstrap node
                let already_connected = existing_connections
                    .iter()
                    .any(|(_, conn)| conn.remote_address() == socket_addr);

                if already_connected {
                    debug!(
                        bootstrap_addr = %addr,
                        socket_addr = %socket_addr,
                        "⏭️  Skipping bootstrap - already connected"
                    );
                    continue;
                }

                let connect_start = std::time::Instant::now();

                match transport.connect_quic(socket_addr).await {
                    Ok(connection) => {
                        let connect_duration = connect_start.elapsed();

                        info!(
                            bootstrap_addr = %addr,
                            socket_addr = %socket_addr,
                            connect_duration_ms = connect_duration.as_millis(),
                            "🤝 Connected to bootstrap node"
                        );

                        // Perform a simple handshake by opening a stream
                        // This mimics what the network engine's perform_handshake does
                        match self.perform_bootstrap_handshake(&connection, addr).await {
                            Ok(remote_peer_id) => {
                                info!(
                                    bootstrap_addr = %addr,
                                    remote_peer_id = %remote_peer_id,
                                    "✅ Bootstrap handshake successful"
                                );

                                // Add to connection pool - clone the connection so we can use it later
                                let connection_clone = connection.clone();
                                if let Err(e) = transport
                                    .connection_pool()
                                    .add_connection(remote_peer_id, connection_clone)
                                    .await
                                {
                                    warn!(
                                        bootstrap_addr = %addr,
                                        peer_id = %remote_peer_id,
                                        error = %e,
                                        "Failed to add bootstrap connection to pool"
                                    );
                                } else {
                                    info!(
                                        bootstrap_addr = %addr,
                                        peer_id = %remote_peer_id,
                                        "Bootstrap connection added to pool"
                                    );

                                    // Store connection so NetworkEngine can start a proper receiver for it
                                    let mut pending =
                                        self.pending_bootstrap_connections.write().await;
                                    pending.push((remote_peer_id, connection.clone()));
                                    info!(
                                        "Stored bootstrap connection for receiver startup, peer_id={}",
                                        remote_peer_id
                                    );

                                    // Add bootstrap peer to PeerManager if available
                                    if let Some(ref peer_manager) = self.peer_manager {
                                        let bootstrap_peer_info = crate::peer::PeerInfo {
                                            peer_id: remote_peer_id,
                                            addresses: vec![addr.clone()],
                                            public_key: adic_types::PublicKey::from_bytes(
                                                [0u8; 32],
                                            ), // Placeholder
                                            reputation_score: 1.0,
                                            latency_ms: Some(connect_duration.as_millis() as u64),
                                            bandwidth_mbps: None,
                                            last_seen: std::time::Instant::now(),
                                            connection_state:
                                                crate::peer::ConnectionState::Connected,
                                            message_stats: Default::default(),
                                            padic_location: None,
                                        };

                                        if let Err(e) =
                                            peer_manager.add_peer(bootstrap_peer_info).await
                                        {
                                            warn!(
                                                bootstrap_addr = %addr,
                                                error = %e,
                                                "Failed to add bootstrap peer to PeerManager"
                                            );
                                        } else {
                                            info!(
                                                bootstrap_addr = %addr,
                                                peer_id = %remote_peer_id,
                                                "Bootstrap peer added to PeerManager"
                                            );
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(
                                    bootstrap_addr = %addr,
                                    error = %e,
                                    "❌ Bootstrap handshake failed"
                                );
                                connection.close(0u32.into(), b"handshake_failed");
                            }
                        }
                    }
                    Err(e) => {
                        let connect_duration = connect_start.elapsed();

                        warn!(
                            bootstrap_addr = %addr,
                            socket_addr = %socket_addr,
                            error = %e,
                            connect_duration_ms = connect_duration.as_millis(),
                            "⚠️ Failed to connect to bootstrap"
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Query bootstrap nodes with Arc to avoid deadlock
    pub async fn query_bootstrap_nodes_with_arc(
        &self,
        transport: Arc<RwLock<crate::transport::HybridTransport>>,
    ) -> Result<()> {
        // First, try to discover bootstrap nodes from DNS
        let mut all_bootstrap_nodes = match self.discover_dns_seeds().await {
            Ok(dns_seeds) => {
                if !dns_seeds.is_empty() {
                    info!("Using {} bootstrap nodes from DNS seeds", dns_seeds.len());
                }
                dns_seeds
            }
            Err(e) => {
                warn!("DNS seed discovery failed: {}", e);
                vec![]
            }
        };

        // Add configured bootstrap nodes
        all_bootstrap_nodes.extend(self.config.bootstrap_nodes.clone());

        if all_bootstrap_nodes.is_empty() {
            warn!("No bootstrap nodes available (neither from DNS nor configuration)");
            return Ok(());
        }

        for addr in &all_bootstrap_nodes {
            // Skip if this is our own address
            if self.is_self_address(addr) {
                debug!(
                    local_peer = %self.local_peer_id,
                    bootstrap_addr = %addr,
                    "🔄 Skipping self-connection"
                );
                continue;
            }

            debug!(
                bootstrap_addr = %addr,
                "🔍 Attempting bootstrap connection"
            );

            // Parse the multiaddr to get the socket address
            if let Some(socket_addr) = self.parse_multiaddr_to_socket(addr) {
                let connect_start = std::time::Instant::now();
                let transport_guard = transport.read().await;
                match transport_guard.connect_quic(socket_addr).await {
                    Ok(_connection) => {
                        let connect_duration = connect_start.elapsed();

                        info!(
                            bootstrap_addr = %addr,
                            socket_addr = %socket_addr,
                            connect_duration_ms = connect_duration.as_millis(),
                            "🤝 Connected to bootstrap node"
                        );
                        // Connection established, peer discovery will happen through normal handshake
                        // The bootstrap node should be added to our peer list automatically
                    }
                    Err(e) => {
                        let connect_duration = connect_start.elapsed();

                        warn!(
                            bootstrap_addr = %addr,
                            socket_addr = %socket_addr,
                            error = %e,
                            connect_duration_ms = connect_duration.as_millis(),
                            "⚠️ Failed to connect to bootstrap"
                        );
                    }
                }
            } else {
                warn!("Failed to parse bootstrap node address: {}", addr);
            }
        }

        Ok(())
    }

    /// Parse a multiaddr to extract socket address for QUIC connection
    fn parse_multiaddr_to_socket(&self, addr: &Multiaddr) -> Option<std::net::SocketAddr> {
        use libp2p::multiaddr::Protocol;
        use std::net::ToSocketAddrs;

        let mut host = None;
        let mut port = None;

        for protocol in addr.iter() {
            match protocol {
                Protocol::Ip4(ipv4) => host = Some(format!("{}", ipv4)),
                Protocol::Ip6(ipv6) => host = Some(format!("{}", ipv6)),
                Protocol::Dns(hostname) | Protocol::Dns4(hostname) | Protocol::Dns6(hostname) => {
                    host = Some(hostname.to_string())
                }
                Protocol::Tcp(p) | Protocol::Udp(p) => port = Some(p),
                Protocol::Quic | Protocol::QuicV1 => {} // QUIC indicators, we support these
                Protocol::P2p(_) => {}                  // Peer ID, used for verification
                _ => {}                                 // Ignore other protocols
            }
        }

        if let (Some(host), Some(port)) = (host, port) {
            // Try to resolve the hostname if it's a DNS entry
            let addr_string = format!("{}:{}", host, port);
            match addr_string.to_socket_addrs() {
                Ok(mut addrs) => addrs.next(),
                Err(e) => {
                    warn!("Failed to resolve {}: {}", addr_string, e);
                    None
                }
            }
        } else {
            None
        }
    }

    /// Request peer list from a specific peer
    pub async fn request_peers_from(
        &self,
        peer: PeerId,
        transport: &crate::transport::HybridTransport,
    ) -> Result<Vec<PeerInfo>> {
        // Mark as active discovery
        let mut active = self.active_discoveries.write().await;
        if active.contains(&peer) {
            return Ok(vec![]); // Already querying this peer
        }
        active.insert(peer);
        drop(active);

        debug!("Requesting peers from {}", peer);

        // Create GetPeers message
        let get_peers_msg = DiscoveryMessage::GetPeers {
            requester: peer.to_bytes(),
            limit: self.config.peer_exchange_limit,
        };

        // Send the discovery message
        let result = match transport.send_discovery_message(&peer, get_peers_msg).await {
            Ok(()) => {
                debug!("Sent GetPeers message to {}", peer);
                // In a real implementation, we would wait for the response
                // For now, return empty list and let the response handler populate peers
                Ok(vec![])
            }
            Err(e) => {
                warn!("Failed to send GetPeers message to {}: {}", peer, e);
                Err(e)
            }
        };

        // Remove from active discoveries
        let mut active = self.active_discoveries.write().await;
        active.remove(&peer);

        result
    }

    /// Add a discovered peer
    pub async fn add_peer(&self, peer_info: PeerInfo) {
        if let Ok(peer_id) = PeerId::from_bytes(&peer_info.peer_id) {
            let mut known_peers = self.known_peers.write().await;
            known_peers.insert(peer_id, peer_info);
        }
    }

    /// Handle incoming discovery message
    pub async fn handle_message(&self, message: DiscoveryMessage, from: PeerId) -> Result<()> {
        match message {
            DiscoveryMessage::GetPeers { limit, .. } => {
                // Note: This will be handled by NetworkEngine which has transport access
                debug!(
                    "Received GetPeers request from {} for {} peers",
                    from, limit
                );
                // Return empty for now, actual handling is in NetworkEngine
            }
            DiscoveryMessage::Peers { peers } => {
                self.handle_peers(from, peers).await?;
            }
            DiscoveryMessage::Announce {
                peer_id,
                addresses,
                capabilities,
            } => {
                self.handle_announce(peer_id, addresses, capabilities)
                    .await?;
            }
            DiscoveryMessage::Ping { nonce } => {
                self.handle_ping(from, nonce).await?;
            }
            DiscoveryMessage::Pong { nonce } => {
                self.handle_pong(from, nonce).await?;
            }
        }
        Ok(())
    }

    pub async fn handle_get_peers(
        &self,
        from: PeerId,
        limit: usize,
        transport: &crate::transport::HybridTransport,
    ) -> Result<()> {
        let known_peers = self.known_peers.read().await;
        let from_bytes = from.to_bytes();

        // Select peers to share (excluding the requester)
        let peers_to_share: Vec<PeerInfo> = known_peers
            .values()
            .filter(|p| p.peer_id != from_bytes)
            .take(limit.min(self.config.peer_exchange_limit))
            .cloned()
            .collect();

        debug!("Sharing {} peers with {}", peers_to_share.len(), from);

        // Send Peers message back
        let peers_msg = DiscoveryMessage::Peers {
            peers: peers_to_share,
        };

        transport
            .send_discovery_message(&from, peers_msg)
            .await
            .map_err(|e| {
                adic_types::AdicError::Network(format!("Failed to send peer list: {}", e))
            })?;

        Ok(())
    }

    async fn handle_peers(&self, _from: PeerId, peers: Vec<PeerInfo>) -> Result<()> {
        for peer_info in peers {
            self.add_peer(peer_info).await;
        }
        Ok(())
    }

    async fn handle_announce(
        &self,
        peer_id_bytes: Vec<u8>,
        addresses: Vec<String>,
        _capabilities: Vec<String>,
    ) -> Result<()> {
        let peer_info = PeerInfo {
            peer_id: peer_id_bytes.clone(),
            addresses,
            last_seen: chrono::Utc::now().timestamp(),
            reputation: 0.5, // Default reputation
        };

        self.add_peer(peer_info).await;
        if let Ok(peer_id) = PeerId::from_bytes(&peer_id_bytes) {
            info!("Peer {} announced itself", peer_id);
        }
        Ok(())
    }

    async fn handle_ping(&self, from: PeerId, nonce: u64) -> Result<()> {
        debug!("Received ping from {} with nonce {}", from, nonce);
        // Send pong response
        Ok(())
    }

    async fn handle_pong(&self, from: PeerId, nonce: u64) -> Result<()> {
        debug!("Received pong from {} with nonce {}", from, nonce);
        // Update peer liveness
        let mut known_peers = self.known_peers.write().await;
        if let Some(peer_info) = known_peers.get_mut(&from) {
            peer_info.last_seen = chrono::Utc::now().timestamp();
        }
        Ok(())
    }

    /// Get current known peers
    pub async fn get_known_peers(&self) -> Vec<PeerInfo> {
        let peers = self.known_peers.read().await;
        peers.values().cloned().collect()
    }

    /// Remove stale peers
    pub async fn cleanup_stale_peers(&self) {
        let mut known_peers = self.known_peers.write().await;
        let now = chrono::Utc::now().timestamp();
        let stale_threshold = 3600; // 1 hour

        known_peers.retain(|_, peer| now - peer.last_seen < stale_threshold);
    }

    /// Check if discovery is needed
    pub async fn needs_discovery(&self) -> bool {
        let known_peers = self.known_peers.read().await;
        let last_discovery = self.last_discovery.read().await;

        known_peers.len() < self.config.min_peers
            || last_discovery.elapsed() > self.config.discovery_interval
    }

    /// Check if a multiaddr belongs to our local peer
    fn is_self_address(&self, addr: &Multiaddr) -> bool {
        use libp2p::multiaddr::Protocol;

        for protocol in addr.iter() {
            if let Protocol::P2p(peer_id) = protocol {
                return peer_id == self.local_peer_id;
            }
        }
        false
    }

    /// Extract PeerId from a multiaddr
    #[allow(dead_code)]
    fn extract_peer_id_from_multiaddr(&self, addr: &Multiaddr) -> Option<PeerId> {
        use libp2p::multiaddr::Protocol;

        for protocol in addr.iter() {
            if let Protocol::P2p(peer_id) = protocol {
                return Some(peer_id);
            }
        }
        None
    }

    /// Perform handshake with bootstrap node
    async fn perform_bootstrap_handshake(
        &self,
        connection: &quinn::Connection,
        _addr: &Multiaddr,
    ) -> Result<PeerId> {
        use crate::transport::NetworkMessage;

        // As initiator, send our handshake first
        let mut stream = connection.open_uni().await.map_err(|e| {
            adic_types::error::AdicError::Network(format!("Failed to open handshake stream: {}", e))
        })?;

        let msg = NetworkMessage::Handshake {
            peer_id: self.local_peer_id.to_bytes(),
            version: 1,
            listening_port: Some(9001), // Default QUIC port, should be configurable
        };

        let data = serde_json::to_vec(&msg).map_err(|e| {
            adic_types::error::AdicError::Serialization(format!(
                "Failed to serialize handshake: {}",
                e
            ))
        })?;

        stream.write_all(&data).await.map_err(|e| {
            adic_types::error::AdicError::Network(format!("Failed to send handshake: {}", e))
        })?;

        stream.finish().map_err(|e| {
            adic_types::error::AdicError::Network(format!(
                "Failed to finish handshake stream: {}",
                e
            ))
        })?;

        // Wait for response
        let mut stream = connection.accept_uni().await.map_err(|e| {
            adic_types::error::AdicError::Network(format!(
                "Failed to accept handshake response: {}",
                e
            ))
        })?;

        let mut buffer = Vec::new();
        let mut chunk = vec![0u8; 1024];
        loop {
            match stream.read(&mut chunk).await {
                Ok(Some(n)) => buffer.extend_from_slice(&chunk[..n]),
                Ok(None) => break,
                Err(e) => {
                    return Err(adic_types::error::AdicError::Network(format!(
                        "Failed to read handshake response: {}",
                        e
                    )))
                }
            }
        }

        let response: NetworkMessage = serde_json::from_slice(&buffer).map_err(|e| {
            adic_types::error::AdicError::Serialization(format!(
                "Failed to deserialize handshake response: {}",
                e
            ))
        })?;

        match response {
            NetworkMessage::Handshake { peer_id, .. } => {
                let remote_peer_id = PeerId::from_bytes(&peer_id).map_err(|e| {
                    adic_types::error::AdicError::Network(format!(
                        "Invalid peer ID in handshake: {}",
                        e
                    ))
                })?;
                Ok(remote_peer_id)
            }
            _ => Err(adic_types::error::AdicError::Network(
                "Unexpected message type in handshake response".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_discovery_protocol() {
        let config = DiscoveryConfig::default();
        let local_peer_id = PeerId::random();
        let protocol = DiscoveryProtocol::new(config, local_peer_id);

        // Test adding peer
        let peer_id = PeerId::random();
        let peer_info = PeerInfo {
            peer_id: peer_id.to_bytes(),
            addresses: vec![],
            last_seen: chrono::Utc::now().timestamp(),
            reputation: 1.0,
        };

        protocol.add_peer(peer_info.clone()).await;

        let known_peers = protocol.get_known_peers().await;
        assert_eq!(known_peers.len(), 1);
        assert_eq!(known_peers[0].peer_id, peer_info.peer_id);
    }

    #[tokio::test]
    async fn test_needs_discovery() {
        let config = DiscoveryConfig {
            min_peers: 2,
            ..Default::default()
        };

        let local_peer_id = PeerId::random();
        let protocol = DiscoveryProtocol::new(config, local_peer_id);

        // Should need discovery (no peers)
        assert!(protocol.needs_discovery().await);

        // Add some peers
        for _ in 0..3 {
            let peer_info = PeerInfo {
                peer_id: PeerId::random().to_bytes(),
                addresses: vec![],
                last_seen: chrono::Utc::now().timestamp(),
                reputation: 1.0,
            };
            protocol.add_peer(peer_info).await;
        }

        // Should not need discovery (enough peers)
        assert!(!protocol.needs_discovery().await);
    }
}
