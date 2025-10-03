#![allow(clippy::excessive_nesting)]

pub mod codecs;
pub mod deposit_verifier;
pub mod dns_version;
pub mod metrics;
pub mod peer;
pub mod pipeline;
pub mod protocol;
pub mod resilience;
pub mod routing;
pub mod security;
pub mod sync;
pub mod transport;

#[cfg(test)]
mod codecs_tests;
#[cfg(test)]
mod peer_tests;

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use libp2p::{identity::Keypair, Multiaddr, PeerId};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use adic_consensus::ConsensusEngine;
use adic_finality::FinalityEngine;
use adic_storage::StorageEngine;
use adic_types::{AdicError, AdicMessage, MessageId, Result};

use crate::deposit_verifier::{DepositVerifier, RealDepositVerifier};
use crate::metrics::NetworkMetrics;
use crate::peer::PeerManager;
use crate::pipeline::{MessagePipeline, PipelineConfig};
use crate::protocol::{
    ConsensusProtocol, DiscoveryConfig, DiscoveryProtocol, GossipProtocol, StreamProtocol,
    SyncProtocol, UpdateProtocol, UpdateProtocolConfig,
};
use crate::resilience::NetworkResilience;
use crate::routing::HypertangleRouter;
use crate::security::SecurityManager;
use crate::sync::StateSync;
use crate::transport::HybridTransport;
pub use crate::transport::{NetworkMessage, TransportConfig};

#[derive(Clone)]
pub struct NetworkConfig {
    pub transport: TransportConfig,
    pub pipeline: PipelineConfig,
    pub max_peers: usize,
    pub bootstrap_peers: Vec<Multiaddr>,
    pub listen_addresses: Vec<Multiaddr>,
    pub enable_metrics: bool,
    pub data_dir: std::path::PathBuf,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            transport: TransportConfig::default(),
            pipeline: PipelineConfig::default(),
            max_peers: 100,
            bootstrap_peers: vec![],
            listen_addresses: vec!["/ip4/0.0.0.0/tcp/9000".parse().unwrap()],
            enable_metrics: true,
            data_dir: std::path::PathBuf::from("./data"),
        }
    }
}

#[derive(Clone)]
pub struct NetworkEngine {
    config: NetworkConfig,
    _keypair: Keypair,
    peer_id: PeerId,
    storage: Arc<StorageEngine>,
    consensus: Arc<ConsensusEngine>, // Added for C1-C3 checks
    finality: Arc<FinalityEngine>,
    transport: Arc<RwLock<HybridTransport>>,
    peer_manager: Arc<PeerManager>,
    gossip: Arc<GossipProtocol>,
    sync_protocol: Arc<SyncProtocol>,
    consensus_protocol: Arc<ConsensusProtocol>,
    stream_protocol: Arc<StreamProtocol>,
    discovery_protocol: Arc<DiscoveryProtocol>,
    update_protocol: Arc<UpdateProtocol>,
    router: Arc<HypertangleRouter>,
    pipeline: Arc<MessagePipeline>,
    state_sync: Arc<StateSync>,
    resilience: Arc<NetworkResilience>,
    security: Arc<SecurityManager>,
    metrics: Option<Arc<NetworkMetrics>>,
    shutdown_signal: Arc<AtomicBool>,
}

impl NetworkEngine {
    pub async fn new(
        config: NetworkConfig,
        keypair: Keypair,
        storage: Arc<StorageEngine>,
        consensus: Arc<ConsensusEngine>,
        finality: Arc<FinalityEngine>,
    ) -> Result<Self> {
        let peer_id = PeerId::from(keypair.public());
        info!(
            peer_id = %peer_id,
            "🚀 Network engine initialization started"
        );

        // Initialize transport
        debug!("Initializing transport layer");
        let mut transport = HybridTransport::new(config.transport.clone(), keypair.clone());
        transport.initialize().await?;
        debug!("Transport layer initialized");

        // Initialize peer manager with real deposit verifier
        debug!("Creating peer manager");
        let deposit_manager = Arc::new(adic_consensus::DepositManager::new(
            adic_consensus::DEFAULT_DEPOSIT_AMOUNT,
        ));
        let deposit_verifier: Arc<dyn DepositVerifier> =
            Arc::new(RealDepositVerifier::new(deposit_manager));
        let peer_manager = Arc::new(PeerManager::new(
            &keypair,
            config.max_peers,
            deposit_verifier,
        ));
        debug!(max_peers = config.max_peers, "Peer manager created");

        // Initialize protocols
        debug!("Initializing network protocols");
        let gossip = Arc::new(GossipProtocol::new(&keypair, Default::default())?);

        let sync_protocol = Arc::new(SyncProtocol::new(Default::default()));
        let consensus_protocol = Arc::new(ConsensusProtocol::new(Default::default()));
        let stream_protocol = Arc::new(StreamProtocol::new(Default::default()));
        let (update_protocol, _update_events) = UpdateProtocol::new(
            UpdateProtocolConfig::default(),
            config.data_dir.clone(),
            peer_id,
        )
        .map_err(|e| AdicError::Network(format!("Failed to initialize update protocol: {}", e)))?;
        let update_protocol = Arc::new(update_protocol);
        debug!("Network protocols initialized");

        // Initialize discovery protocol
        debug!("Initializing discovery protocol");
        let discovery_config = DiscoveryConfig {
            bootstrap_nodes: config.bootstrap_peers.clone(),
            min_peers: config.max_peers / 4, // Aim for at least 25% of max
            max_peers: config.max_peers,
            ..Default::default()
        };
        let mut discovery_protocol = DiscoveryProtocol::new(discovery_config, peer_id);
        discovery_protocol.set_peer_manager(Arc::clone(&peer_manager));
        let discovery_protocol = Arc::new(discovery_protocol);

        // Initialize routing
        debug!("Initializing hypertangle router");
        let router = Arc::new(HypertangleRouter::new(peer_id));

        // Initialize pipeline
        debug!("Initializing message pipeline");
        let pipeline = Arc::new(MessagePipeline::new(config.pipeline.clone()));
        pipeline.start_cleanup_task().await;
        debug!("Pipeline cleanup task started");

        // Initialize state sync
        let state_sync = Arc::new(StateSync::new(
            Default::default(),
            storage.clone(),
            finality.clone(),
        ));

        // Initialize resilience
        let resilience = Arc::new(NetworkResilience::new(Default::default()));

        // Initialize security
        let security = Arc::new(SecurityManager::new());

        // Initialize metrics if enabled
        let metrics = if config.enable_metrics {
            Some(Arc::new(NetworkMetrics::new()))
        } else {
            None
        };

        // Subscribe to default topics
        debug!("Subscribing to default topics");
        gossip.subscribe("adic/messages").await?;
        gossip.subscribe("adic/tips").await?;
        gossip.subscribe("adic/finality").await?;
        gossip.subscribe("adic/conflicts").await?;
        debug!(
            topics = ?["adic/messages", "adic/tips", "adic/finality", "adic/conflicts"],
            "Topic subscriptions complete"
        );

        // Capture values before moving config
        let max_peers = config.max_peers;
        let bootstrap_peers_count = config.bootstrap_peers.len();

        let result = Ok(Self {
            config,
            _keypair: keypair,
            peer_id,
            storage,
            consensus,
            finality,
            transport: Arc::new(RwLock::new(transport)),
            peer_manager,
            gossip,
            sync_protocol,
            consensus_protocol,
            stream_protocol,
            discovery_protocol,
            update_protocol,
            router,
            pipeline,
            state_sync,
            resilience,
            security,
            metrics,
            shutdown_signal: Arc::new(AtomicBool::new(false)),
        });

        info!(
            peer_id = %peer_id,
            max_peers = max_peers,
            bootstrap_peers = bootstrap_peers_count,
            "🌐 Network engine initialized"
        );
        result
    }

    pub async fn start(&self) -> Result<()> {
        info!(
            peer_id = %self.peer_id,
            listen_addresses = ?self.config.listen_addresses,
            bootstrap_peers = self.config.bootstrap_peers.len(),
            "🌐 Starting network engine"
        );

        // Start accepting incoming connections
        info!(transport = "hybrid", "⏳ Starting accept loop");
        self.start_accept_loop().await;

        // Start update protocol background tasks
        info!("Starting update protocol background tasks");
        self.update_protocol.clone().start_background_tasks();
        info!(transport = "hybrid", "✅ Accept loop started");

        // Start listening on configured addresses
        for addr in &self.config.listen_addresses {
            info!(
                address = %addr,
                "Listening on address"
            );
        }

        // Get actual listening ports for logging
        if let Some(port) = self.local_quic_port().await {
            info!(port = port, protocol = "quic", "📡 QUIC endpoint listening");
        }

        // Connect to bootstrap peers via discovery protocol
        if !self.config.bootstrap_peers.is_empty() {
            info!(
                bootstrap_count = self.config.bootstrap_peers.len(),
                "🔍 Discovering bootstrap peers"
            );
            // Clone the transport Arc to avoid holding lock during async operation
            let transport_clone = self.transport.clone();
            if let Err(e) = self
                .discovery_protocol
                .query_bootstrap_nodes_with_arc(transport_clone)
                .await
            {
                warn!(
                    error = %e,
                    bootstrap_count = self.config.bootstrap_peers.len(),
                    "⚠️ Failed to query bootstrap nodes"
                );
            } else {
                info!(
                    bootstrap_count = self.config.bootstrap_peers.len(),
                    "✅ Bootstrap discovery completed"
                );
            }
        }

        // Start background tasks
        debug!("Starting gossip handler");
        self.start_gossip_handler().await;
        debug!("Starting sync handler");
        self.start_sync_handler().await;
        debug!("Starting consensus handler");
        self.start_consensus_handler().await;
        debug!("Starting maintenance tasks");
        self.start_maintenance_tasks().await;

        info!(
            "Network engine started successfully for peer {}",
            self.peer_id
        );
        Ok(())
    }

    /// Return the local QUIC listening port if available (for tests/debugging)
    pub async fn local_quic_port(&self) -> Option<u16> {
        let transport = self.transport.read().await;
        if let Some(ep) = transport.quic_endpoint() {
            if let Ok(addr) = ep.local_addr() {
                return Some(addr.port());
            }
        }
        None
    }

    /// Get all listening addresses including actual assigned ports
    pub async fn get_listening_addresses(&self) -> Vec<Multiaddr> {
        let mut addresses = Vec::new();

        // Get QUIC address with actual port
        if let Some(port) = self.local_quic_port().await {
            // Create a Multiaddr for our QUIC address
            if let Ok(quic_addr) = format!("/ip4/127.0.0.1/udp/{}/quic", port).parse() {
                addresses.push(quic_addr);
            }
        }

        // Add configured libp2p addresses (these might still be using port 0)
        addresses.extend(self.config.listen_addresses.clone());

        addresses
    }

    async fn start_accept_loop(&self) {
        debug!("Starting connection accept loop");
        let network = self.clone();

        tokio::spawn(async move {
            info!(
                "start_accept_loop - Inside spawned task for peer {}",
                network.peer_id
            );
            loop {
                // Check shutdown signal first
                if network.shutdown_signal.load(Ordering::Relaxed) {
                    info!(
                        peer_id = %network.peer_id,
                        "Accept loop shutting down"
                    );
                    break;
                }

                // Try to accept a new connection with a shorter scope for the read lock
                debug!("Accept loop - About to acquire transport read lock");
                let endpoint_opt = {
                    debug!("Accept loop - Acquiring transport read lock...");
                    let transport = network.transport.read().await;
                    debug!("Accept loop - Transport read lock acquired");
                    transport.quic_endpoint()
                };

                if let Some(endpoint) = endpoint_opt {
                    // Use a timeout for accept to make it interruptible
                    match tokio::time::timeout(Duration::from_millis(500), endpoint.accept()).await
                    {
                        Ok(Some(connecting)) => {
                            let net = network.clone();
                            tokio::spawn(async move {
                                match connecting.await {
                                    Ok(connection) => {
                                        let remote_addr = connection.remote_address();
                                        let connection_start = std::time::Instant::now();

                                        info!(
                                            remote_addr = %remote_addr,
                                            protocol = "quic",
                                            direction = "incoming",
                                            "🔗 Connection accepted"
                                        );

                                        // Perform handshake as responder
                                        match net.perform_handshake(&connection, false).await {
                                            Ok(remote_peer_id) => {
                                                let handshake_duration = connection_start.elapsed();
                                                // Add to connection pool
                                                let transport = net.transport.read().await;
                                                if let Err(e) = transport
                                                    .connection_pool()
                                                    .add_connection(
                                                        remote_peer_id,
                                                        connection.clone(),
                                                    )
                                                    .await
                                                {
                                                    error!(
                                                        "Failed to add connection to pool: {}",
                                                        e
                                                    );
                                                } else {
                                                    // Add peer to the peer manager
                                                    use crate::peer::{
                                                        ConnectionState, MessageStats, PeerInfo,
                                                    };
                                                    use adic_types::PublicKey;
                                                    let peer_info = PeerInfo {
                                                        peer_id: remote_peer_id,
                                                        addresses: vec![],
                                                        public_key: PublicKey::from_bytes([0; 32]), // Will be updated later
                                                        reputation_score: 50.0,
                                                        latency_ms: None,
                                                        bandwidth_mbps: None,
                                                        last_seen: std::time::Instant::now(),
                                                        connection_state:
                                                            ConnectionState::Connected,
                                                        message_stats: MessageStats::default(),
                                                        padic_location: None,
                                                    };

                                                    if let Err(e) =
                                                        net.peer_manager.add_peer(peer_info).await
                                                    {
                                                        debug!(
                                                            peer_id = %remote_peer_id,
                                                            error = %e,
                                                            "Failed to add peer to peer manager"
                                                        );
                                                    }

                                                    // Start receiving messages
                                                    net.start_quic_receiver(
                                                        connection,
                                                        remote_peer_id,
                                                    )
                                                    .await;
                                                    info!(
                                                        local_peer = %net.peer_id,
                                                        remote_peer = %remote_peer_id,
                                                        remote_addr = %remote_addr,
                                                        handshake_duration_ms = handshake_duration.as_millis(),
                                                        "✅ Handshake succeeded"
                                                    );

                                                    // Trigger message sync with the newly connected peer
                                                    info!(
                                                        "Starting incremental sync with peer {}",
                                                        remote_peer_id
                                                    );
                                                    if let Err(e) = net
                                                        .sync_protocol
                                                        .start_incremental_sync(remote_peer_id)
                                                        .await
                                                    {
                                                        warn!(
                                                            "Failed to start sync with peer {}: {}",
                                                            remote_peer_id, e
                                                        );
                                                    }
                                                    // Send the actual sync request
                                                    if let Err(e) = net.send_sync_request(remote_peer_id,
                                                        crate::protocol::sync::SyncRequest::GetFrontier).await {
                                                        warn!(
                                                            peer_id = %remote_peer_id,
                                                            error = %e,
                                                            "Failed to send sync request"
                                                        );
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                let handshake_duration = connection_start.elapsed();

                                                warn!(
                                                    remote_addr = %remote_addr,
                                                    error = %e,
                                                    handshake_duration_ms = handshake_duration.as_millis(),
                                                    direction = "incoming",
                                                    "❌ Handshake failed"
                                                );
                                                // Close the connection
                                                connection.close(0u32.into(), b"handshake_failed");
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!(
                                            error = %e,
                                            error_type = std::any::type_name::<anyhow::Error>(),
                                            "❌ Failed to accept connection"
                                        );
                                    }
                                }
                            });
                        }
                        Ok(None) => {
                            // No incoming connection, continue loop
                        }
                        Err(_) => {
                            // Timeout occurred, continue loop (this allows checking shutdown signal)
                        }
                    }
                } else {
                    // No endpoint available, wait a bit
                    debug!("Accept loop - No endpoint available, sleeping");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        });
    }

    #[allow(dead_code)]
    async fn connect_to_peer(&self, _addr: &Multiaddr) -> Result<()> {
        // Extract socket address from multiaddr
        // In real implementation, parse multiaddr properly
        let socket_addr: SocketAddr = "127.0.0.1:9000"
            .parse()
            .map_err(|e| AdicError::Network(format!("Failed to parse address: {}", e)))?;

        let transport = self.transport.read().await;
        let connection = transport.connect_quic(socket_addr).await?;

        // Perform handshake
        let peer_id = PeerId::random(); // Would get from handshake

        transport
            .connection_pool()
            .add_connection(peer_id, connection)
            .await?;

        info!(
            peer_id = %peer_id,
            "🤝 Peer connected"
        );
        Ok(())
    }

    pub async fn connect_peer(&self, addr: SocketAddr) -> Result<()> {
        // Connect via QUIC transport
        let transport = self.transport.read().await;
        let connection = transport.connect_quic(addr).await?;

        // Perform handshake - send our peer ID and get theirs
        let remote_peer_id = self.perform_handshake(&connection, true).await?;

        // Add as outgoing connection (we initiated)
        transport
            .connection_pool()
            .add_outgoing_connection(remote_peer_id, connection.clone())
            .await?;

        // Add peer to the peer manager
        use crate::peer::{ConnectionState, MessageStats, PeerInfo};
        use adic_types::PublicKey;
        let peer_info = PeerInfo {
            peer_id: remote_peer_id,
            addresses: vec![],
            public_key: PublicKey::from_bytes([0; 32]), // Will be updated later
            reputation_score: 50.0,
            latency_ms: None,
            bandwidth_mbps: None,
            last_seen: std::time::Instant::now(),
            connection_state: ConnectionState::Connected,
            message_stats: MessageStats::default(),
            padic_location: None,
        };

        if let Err(e) = self.peer_manager.add_peer(peer_info).await {
            debug!(
                peer_id = %remote_peer_id,
                error = %e,
                "Failed to add peer to peer manager"
            );
        }

        // Start receiving messages from this peer
        self.start_quic_receiver(connection, remote_peer_id).await;

        info!(
            peer_id = %remote_peer_id,
            address = %addr,
            "🤝 Peer connected"
        );

        // Trigger message sync with the newly connected peer
        debug!(
            peer_id = %remote_peer_id,
            "Starting incremental sync"
        );
        if let Err(e) = self
            .sync_protocol
            .start_incremental_sync(remote_peer_id)
            .await
        {
            warn!(
                peer_id = %remote_peer_id,
                error = %e,
                "Failed to start sync with peer"
            );
        }
        // Send the actual sync request
        if let Err(e) = self
            .send_sync_request(
                remote_peer_id,
                crate::protocol::sync::SyncRequest::GetFrontier,
            )
            .await
        {
            warn!(
                "Failed to send sync request to peer {}: {}",
                remote_peer_id, e
            );
        }

        Ok(())
    }

    async fn perform_handshake(
        &self,
        connection: &quinn::Connection,
        initiator: bool,
    ) -> Result<PeerId> {
        // Get our local QUIC port for sharing with peer
        let local_port = {
            let transport = self.transport.read().await;
            transport.local_quic_port().await
        };

        if initiator {
            // Send our handshake message first
            let mut stream = connection.open_uni().await.map_err(|e| {
                AdicError::Network(format!("Failed to open handshake stream: {}", e))
            })?;

            let msg = NetworkMessage::Handshake {
                peer_id: self.peer_id.to_bytes(),
                version: 1,
                listening_port: local_port,
            };
            let data = serde_json::to_vec(&msg).map_err(|e| {
                AdicError::Serialization(format!("Failed to serialize handshake: {}", e))
            })?;

            stream
                .write_all(&data)
                .await
                .map_err(|e| AdicError::Network(format!("Failed to send handshake: {}", e)))?;
            stream.finish().map_err(|e| {
                AdicError::Network(format!("Failed to finish handshake stream: {}", e))
            })?;

            // Wait for their handshake response
            let mut stream = connection.accept_uni().await.map_err(|e| {
                AdicError::Network(format!("Failed to accept handshake response: {}", e))
            })?;

            let mut buffer = Vec::new();
            let mut chunk = vec![0u8; 1024];
            loop {
                match stream.read(&mut chunk).await {
                    Ok(Some(n)) => buffer.extend_from_slice(&chunk[..n]),
                    Ok(None) => break,
                    Err(e) => {
                        return Err(AdicError::Network(format!(
                            "Failed to read handshake response: {}",
                            e
                        )))
                    }
                }
            }

            let response: NetworkMessage = serde_json::from_slice(&buffer).map_err(|e| {
                AdicError::Serialization(format!("Failed to deserialize handshake response: {}", e))
            })?;

            match response {
                NetworkMessage::Handshake {
                    peer_id,
                    listening_port,
                    ..
                } => {
                    if let Some(port) = listening_port {
                        if let Ok(pid) = PeerId::from_bytes(&peer_id) {
                            debug!(
                                peer = %pid,
                                port = port,
                                "Peer listening port discovered"
                            );
                        }
                    }
                    PeerId::from_bytes(&peer_id).map_err(|e| {
                        AdicError::Network(format!("Invalid peer ID in handshake: {}", e))
                    })
                }
                _ => Err(AdicError::Network(
                    "Invalid handshake response message type".into(),
                )),
            }
        } else {
            // Wait for their handshake first
            let mut stream = connection
                .accept_uni()
                .await
                .map_err(|e| AdicError::Network(format!("Failed to accept handshake: {}", e)))?;

            let mut buffer = Vec::new();
            let mut chunk = vec![0u8; 1024];
            loop {
                match stream.read(&mut chunk).await {
                    Ok(Some(n)) => buffer.extend_from_slice(&chunk[..n]),
                    Ok(None) => break,
                    Err(e) => {
                        return Err(AdicError::Network(format!(
                            "Failed to read handshake: {}",
                            e
                        )))
                    }
                }
            }

            let request: NetworkMessage = serde_json::from_slice(&buffer).map_err(|e| {
                AdicError::Serialization(format!("Failed to deserialize handshake: {}", e))
            })?;

            // Send our handshake back
            let mut stream = connection.open_uni().await.map_err(|e| {
                AdicError::Network(format!("Failed to open handshake response stream: {}", e))
            })?;

            let msg = NetworkMessage::Handshake {
                peer_id: self.peer_id.to_bytes(),
                version: 1,
                listening_port: local_port,
            };
            let data = serde_json::to_vec(&msg).map_err(|e| {
                AdicError::Serialization(format!("Failed to serialize handshake response: {}", e))
            })?;

            stream.write_all(&data).await.map_err(|e| {
                AdicError::Network(format!("Failed to send handshake response: {}", e))
            })?;
            stream.finish().map_err(|e| {
                AdicError::Network(format!("Failed to finish handshake response stream: {}", e))
            })?;

            match request {
                NetworkMessage::Handshake {
                    peer_id,
                    listening_port,
                    ..
                } => {
                    if let Some(port) = listening_port {
                        if let Ok(pid) = PeerId::from_bytes(&peer_id) {
                            debug!(
                                peer = %pid,
                                port = port,
                                "Peer listening port discovered"
                            );
                        }
                    }
                    PeerId::from_bytes(&peer_id).map_err(|e| {
                        AdicError::Network(format!("Invalid peer ID in handshake: {}", e))
                    })
                }
                _ => Err(AdicError::Network(
                    "Invalid handshake request message type".into(),
                )),
            }
        }
    }

    async fn start_quic_receiver(&self, connection: quinn::Connection, remote_peer_id: PeerId) {
        let network = self.clone();

        tokio::spawn(async move {
            loop {
                match connection.accept_uni().await {
                    Ok(mut stream) => {
                        // Read all data from stream
                        let mut buffer = Vec::new();
                        let mut chunk = vec![0u8; 8192]; // 8KB chunks

                        loop {
                            match stream.read(&mut chunk).await {
                                Ok(Some(n)) => {
                                    buffer.extend_from_slice(&chunk[..n]);
                                }
                                Ok(None) => {
                                    // End of stream
                                    break;
                                }
                                Err(e) => {
                                    warn!("Failed to read from stream: {}", e);
                                    break;
                                }
                            }
                        }

                        if !buffer.is_empty() {
                            // Update peer stats to keep peer alive
                            network
                                .peer_manager
                                .update_peer_stats(
                                    &remote_peer_id,
                                    false,
                                    buffer.len() as u64,
                                    None,
                                )
                                .await;

                            // Try to deserialize as NetworkMessage first, fallback to AdicMessage for backward compatibility
                            if let Ok(network_msg) =
                                serde_json::from_slice::<NetworkMessage>(&buffer)
                            {
                                if let Err(e) = network
                                    .handle_network_message(network_msg, remote_peer_id)
                                    .await
                                {
                                    warn!("Failed to handle network message: {}", e);
                                }
                            } else if let Ok(adic_msg) =
                                serde_json::from_slice::<AdicMessage>(&buffer)
                            {
                                // Backward compatibility: handle raw AdicMessage
                                info!("[{}] QUIC: Received ADIC message {} (proposer {}) from peer {}",
                                    network.peer_id,
                                    hex::encode(&adic_msg.id.as_bytes()[..8]),
                                    hex::encode(&adic_msg.proposer_pk.as_bytes()[..8]),
                                    remote_peer_id);
                                if let Err(e) = network
                                    .handle_incoming_message(&adic_msg, remote_peer_id)
                                    .await
                                {
                                    warn!("Failed to handle ADIC message: {}", e);
                                }
                            } else {
                                debug!(
                                    "Failed to deserialize message from peer {}",
                                    remote_peer_id
                                );
                            }
                        }
                    }
                    Err(quinn::ConnectionError::ApplicationClosed(_)) => {
                        info!("Connection closed by peer {}", remote_peer_id);
                        // Update peer state to disconnected
                        if let Err(e) = network
                            .peer_manager
                            .update_peer_connection_state(
                                &remote_peer_id,
                                crate::peer::ConnectionState::Disconnected,
                            )
                            .await
                        {
                            warn!("Failed to update peer state: {}", e);
                        }
                        // Remove connection from pool
                        if let Ok(transport) = network.transport.try_read() {
                            transport
                                .connection_pool()
                                .remove_connection(&remote_peer_id)
                                .await;
                        }
                        break;
                    }
                    Err(e) => {
                        warn!("Error accepting stream from {}: {}", remote_peer_id, e);
                        // Update peer state to disconnected
                        if let Err(e) = network
                            .peer_manager
                            .update_peer_connection_state(
                                &remote_peer_id,
                                crate::peer::ConnectionState::Disconnected,
                            )
                            .await
                        {
                            warn!("Failed to update peer state: {}", e);
                        }
                        // Remove connection from pool
                        if let Ok(transport) = network.transport.try_read() {
                            transport
                                .connection_pool()
                                .remove_connection(&remote_peer_id)
                                .await;
                        }
                        break;
                    }
                }
            }
        });
    }

    pub async fn get_pending_messages(&self) -> Vec<AdicMessage> {
        // Get messages from the pipeline's pending queue
        self.pipeline.get_pending_messages().await
    }

    pub async fn broadcast_message(&self, message: AdicMessage) -> Result<()> {
        // Messages must be pre-signed with ADIC keypair before broadcasting
        if message.signature.as_bytes().is_empty() {
            return Err(AdicError::SignatureVerification);
        }

        // Store the message locally first
        info!(
            "[{}] Storing our own message {} before broadcasting",
            self.peer_id,
            hex::encode(&message.id.as_bytes()[..8])
        );
        self.storage
            .store_message(&message)
            .await
            .map_err(|e| AdicError::Storage(format!("Failed to store message: {}", e)))?;

        // Add to finality engine
        let parent_ids: Vec<MessageId> = message.parents.clone();
        let mut ball_ids = HashMap::new();
        for axis_phi in &message.features.phi {
            ball_ids.insert(axis_phi.axis.0, axis_phi.qp_digits.ball_id(3));
        }

        self.finality
            .add_message(message.id, parent_ids, 1.0, ball_ids)
            .await
            .map_err(|e| {
                AdicError::FinalityFailed(format!("Failed to add message to finality: {}", e))
            })?;

        info!(
            "Stored message {} locally before broadcasting",
            hex::encode(&message.id.as_bytes()[..8])
        );

        // Process through pipeline
        let source = self.peer_id;
        self.pipeline
            .submit_message(message.clone(), source)
            .await?;

        // Route to appropriate peers
        let _target_peers = self.router.route_message(&message).await;

        // Broadcast via gossip (if peers available)
        let _ = self.gossip.broadcast_message(message.clone()).await;

        // Also relay directly via QUIC connections
        self.relay_via_quic(&message).await?;

        // Update metrics
        if let Some(metrics) = &self.metrics {
            metrics.record_message_sent(&message.id).await;
        }

        // Trigger sync with peers to share this new message
        self.trigger_sync_with_peers().await;

        Ok(())
    }

    async fn relay_via_quic(&self, message: &AdicMessage) -> Result<()> {
        // Get OUTGOING QUIC connections only (to avoid loops)
        let transport = self.transport.read().await;
        let pool = transport.connection_pool();

        // Serialize message
        let data = serde_json::to_vec(message)
            .map_err(|e| AdicError::Serialization(format!("Failed to serialize message: {}", e)))?;

        // Get ALL connections to ensure full message propagation
        let connections = pool.get_all_connections().await;
        info!(
            "Broadcasting message {} to {} peers",
            hex::encode(&message.id.as_bytes()[..8]),
            connections.len()
        );

        for (peer_id, conn) in connections {
            // Open a unidirectional stream to send the message
            match conn.open_uni().await {
                Ok(mut stream) => {
                    // Write the serialized message
                    if let Err(e) = stream.write_all(&data).await {
                        warn!("Failed to send message to peer {}: {}", peer_id, e);
                    } else {
                        let _ = stream.finish();
                        info!(
                            "[{}] Successfully sent message {} to peer {}",
                            self.peer_id,
                            hex::encode(&message.id.as_bytes()[..8]),
                            peer_id
                        );
                    }
                }
                Err(e) => {
                    warn!("Failed to open stream to peer {}: {}", peer_id, e);
                }
            }
        }

        Ok(())
    }

    pub async fn request_sync(&self, peer: PeerId, height: u64) -> Result<()> {
        self.state_sync.start_fast_sync(peer, height).await
    }

    pub async fn stream_messages(&self, peer: PeerId, message_ids: Vec<MessageId>) -> Result<()> {
        // Use stream protocol for bulk message transfer
        self.stream_protocol
            .stream_bulk_messages(peer, message_ids, false)
            .await
    }

    pub async fn request_snapshot(
        &self,
        peer: PeerId,
        from_height: u64,
        to_height: u64,
    ) -> Result<Vec<u8>> {
        // Use stream protocol to request state snapshot
        self.stream_protocol
            .receive_snapshot(peer, from_height, to_height)
            .await
    }

    /// Handle incoming network messages and route them appropriately
    pub async fn handle_network_message(
        &self,
        message: NetworkMessage,
        from_peer: PeerId,
    ) -> Result<()> {
        match message {
            NetworkMessage::AdicMessage(adic_msg) => {
                info!(
                    "[{}] QUIC: Received ADIC message {} (proposer {}) from peer {}",
                    self.peer_id,
                    hex::encode(&adic_msg.id.as_bytes()[..8]),
                    hex::encode(&adic_msg.proposer_pk.as_bytes()[..8]),
                    from_peer
                );
                self.handle_incoming_message(&adic_msg, from_peer).await
            }
            NetworkMessage::Discovery(discovery_msg) => {
                debug!(
                    "Received discovery message from peer {}: {:?}",
                    from_peer, discovery_msg
                );
                self.handle_discovery_message(discovery_msg, from_peer)
                    .await
            }
            NetworkMessage::Handshake {
                peer_id: _,
                version,
                listening_port,
            } => {
                debug!(
                    "Received handshake from peer {} (version {}, port {:?})",
                    from_peer, version, listening_port
                );
                // Store peer info if we got their listening port
                if let Some(port) = listening_port {
                    // Update peer manager with the listening port information
                    // This could be used for future connections
                    info!("Peer {} is listening on port {}", from_peer, port);
                }
                Ok(())
            }
            NetworkMessage::SyncRequest(sync_request) => {
                debug!(
                    "Received sync request from peer {}: {:?}",
                    from_peer, sync_request
                );
                self.handle_sync_request(sync_request, from_peer).await
            }
            NetworkMessage::SyncResponse(sync_response) => {
                debug!(
                    "Received sync response from peer {}: {:?}",
                    from_peer, sync_response
                );
                self.handle_sync_response(sync_response, from_peer).await
            }
            NetworkMessage::Update(update_msg) => {
                debug!(
                    "Received update message from peer {}: {:?}",
                    from_peer, update_msg
                );
                self.handle_update_message(update_msg, from_peer).await
            }
        }
    }

    /// Handle discovery messages with transport access
    async fn handle_discovery_message(
        &self,
        message: crate::protocol::discovery::DiscoveryMessage,
        from_peer: PeerId,
    ) -> Result<()> {
        use crate::protocol::discovery::DiscoveryMessage;

        let transport = self.transport.read().await;

        match message {
            DiscoveryMessage::GetPeers { limit, .. } => {
                self.discovery_protocol
                    .handle_get_peers(from_peer, limit, &transport)
                    .await
            }
            DiscoveryMessage::Peers { peers } => {
                for peer_info in peers {
                    self.discovery_protocol.add_peer(peer_info).await;
                }
                Ok(())
            }
            DiscoveryMessage::Announce {
                peer_id,
                addresses,
                capabilities,
            } => {
                // Handle peer announcement
                let peer_info = crate::protocol::discovery::PeerInfo {
                    peer_id: peer_id.clone(),
                    addresses,
                    last_seen: chrono::Utc::now().timestamp(),
                    reputation: 0.5, // Default reputation
                };

                self.discovery_protocol.add_peer(peer_info).await;
                if let Ok(peer_id) = libp2p::PeerId::from_bytes(&peer_id) {
                    info!(
                        "Peer {} announced itself with capabilities: {:?}",
                        peer_id, capabilities
                    );
                }
                Ok(())
            }
            DiscoveryMessage::Ping { nonce } => {
                // Respond with pong
                let pong = DiscoveryMessage::Pong { nonce };
                transport.send_discovery_message(&from_peer, pong).await
            }
            DiscoveryMessage::Pong { .. } => {
                // Update peer liveness - this is handled by the discovery protocol
                Ok(())
            }
        }
    }

    pub async fn handle_incoming_message(
        &self,
        message: &AdicMessage,
        from_peer: PeerId,
    ) -> Result<()> {
        // Check if we already have this message
        let existing = self.storage.get_message(&message.id).await;
        if let Ok(Some(existing_msg)) = existing {
            debug!(
                "[{}] Already have message {} (proposer {}) from peer {} - existing proposer: {} , skipping",
                self.peer_id,
                hex::encode(&message.id.as_bytes()[..8]),
                hex::encode(&message.proposer_pk.as_bytes()[..8]),
                from_peer,
                hex::encode(&existing_msg.proposer_pk.as_bytes()[..8])
            );
            return Ok(());
        }

        // Verify message signature before accepting it using ADIC crypto
        if !self.security.verify_message_signature(message) {
            warn!(
                "Invalid signature on message {} from peer {}, rejecting",
                hex::encode(&message.id.as_bytes()[..8]),
                from_peer
            );
            return Err(AdicError::SignatureVerification);
        }

        // SECURITY: Perform C1-C3 admissibility checks before accepting the message
        // This is critical for preventing malicious messages from entering the DAG
        if !message.parents.is_empty() {
            // Genesis messages have no parents
            // Get parent features and reputations
            let mut parent_features = Vec::new();
            let mut parent_reputations = Vec::new();

            for parent_id in &message.parents {
                if let Ok(Some(parent)) = self.storage.get_message(parent_id).await {
                    // Extract parent features
                    let mut features = Vec::new();
                    for axis_phi in &parent.features.phi {
                        features.push(axis_phi.qp_digits.clone());
                    }
                    parent_features.push(features);

                    // Get parent reputation (using default for now)
                    parent_reputations.push(
                        self.consensus
                            .reputation
                            .get_reputation(&parent.proposer_pk)
                            .await,
                    );
                } else {
                    warn!(
                        "Parent message {} not found for message {}",
                        hex::encode(&parent_id.as_bytes()[..8]),
                        hex::encode(&message.id.as_bytes()[..8])
                    );
                    return Err(AdicError::InvalidMessage("Missing parent".into()));
                }
            }

            // Check C1-C3 admissibility
            let admissibility_result = self
                .consensus
                .admissibility()
                .check_message(message, &parent_features, &parent_reputations)
                .map_err(|e| AdicError::AdmissibilityFailed(e.to_string()))?;

            if !admissibility_result.is_admissible() {
                warn!(
                    "Message {} failed admissibility checks: {}",
                    hex::encode(&message.id.as_bytes()[..8]),
                    admissibility_result.details
                );
                return Err(AdicError::AdmissibilityFailed(admissibility_result.details));
            }

            info!(
                "Message {} passed C1-C3 admissibility checks (score: {:.2})",
                hex::encode(&message.id.as_bytes()[..8]),
                admissibility_result.score
            );
        }

        info!(
            "Storing NEW message {} from peer {} (signature verified)",
            hex::encode(&message.id.as_bytes()[..8]),
            from_peer
        );

        // Store the message
        self.storage
            .store_message(message)
            .await
            .map_err(|e| AdicError::Storage(format!("Failed to store message: {}", e)))?;

        // Add to finality engine
        let parent_ids: Vec<MessageId> = message.parents.clone();
        let mut ball_ids = HashMap::new();
        for axis_phi in &message.features.phi {
            ball_ids.insert(axis_phi.axis.0, axis_phi.qp_digits.ball_id(3));
        }

        self.finality
            .add_message(
                message.id, parent_ids, 1.0, // Default weight
                ball_ids,
            )
            .await
            .map_err(|e| {
                AdicError::FinalityFailed(format!("Failed to add message to finality: {}", e))
            })?;

        // Process through pipeline
        self.pipeline
            .submit_message(message.clone(), from_peer)
            .await?;

        // Relay to other peers (avoid sending back to original sender)
        let transport = self.transport.read().await;
        let pool = transport.connection_pool();
        let connections = pool.get_all_connections().await;

        let data = serde_json::to_vec(message)
            .map_err(|e| AdicError::Serialization(format!("Failed to serialize message: {}", e)))?;

        let mut relay_count = 0;
        for (peer_id, conn) in connections {
            if peer_id != from_peer {
                // Don't send back to sender
                match conn.open_uni().await {
                    Ok(mut stream) => {
                        if let Err(e) = stream.write_all(&data).await {
                            warn!("Failed to relay message to peer {}: {}", peer_id, e);
                        } else {
                            let _ = stream.finish();
                            debug!(
                                "Relayed message {} to peer {}",
                                hex::encode(&message.id.as_bytes()[..8]),
                                peer_id
                            );
                            relay_count += 1;
                        }
                    }
                    Err(e) => {
                        warn!("Failed to open stream to peer {}: {}", peer_id, e);
                    }
                }
            }
        }

        info!(
            "Stored and relayed message {} from {} to {} other peers",
            hex::encode(&message.id.as_bytes()[..8]),
            from_peer,
            relay_count
        );

        Ok(())
    }

    async fn start_gossip_handler(&self) {
        let gossip = self.gossip.clone();
        let _pipeline = self.pipeline.clone();
        let _metrics = self.metrics.clone();

        tokio::spawn(async move {
            let _events = gossip.event_stream();

            loop {
                // Process gossip events
                tokio::time::sleep(Duration::from_millis(100)).await;

                // Process validation queue
                gossip
                    .process_validation_queue(|msg| {
                        // Basic validation
                        msg.verify_id()
                    })
                    .await;
            }
        });
    }

    async fn start_sync_handler(&self) {
        let sync = self.sync_protocol.clone();
        let network = self.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));

            loop {
                interval.tick().await;

                // Cleanup stale sync requests
                sync.cleanup_stale_requests().await;

                // Periodic sync with all connected peers
                let transport = network.transport.read().await;
                let pool = transport.connection_pool();
                let connections = pool.get_all_connections().await;
                drop(transport); // Release lock early

                for (peer_id, _conn) in connections {
                    info!("Periodic sync with peer {}", peer_id);

                    // Start incremental sync
                    if let Err(e) = sync.start_incremental_sync(peer_id).await {
                        warn!("Failed to start periodic sync with {}: {}", peer_id, e);
                    }

                    // Send sync request
                    if let Err(e) = network
                        .send_sync_request(peer_id, crate::protocol::sync::SyncRequest::GetFrontier)
                        .await
                    {
                        warn!("Failed to send periodic sync request to {}: {}", peer_id, e);
                    }
                }
            }
        });
    }

    async fn start_consensus_handler(&self) {
        let consensus = self.consensus_protocol.clone();

        tokio::spawn(async move {
            loop {
                // Cleanup old proposals
                consensus.cleanup_old_proposals().await;
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }

    async fn start_maintenance_tasks(&self) {
        let peer_manager = self.peer_manager.clone();
        let router = self.router.clone();
        let resilience = self.resilience.clone();
        let discovery = self.discovery_protocol.clone();
        let transport = self.transport.clone();
        let shutdown_signal = self.shutdown_signal.clone();
        let peer_id = self.peer_id;
        let network = self.clone();

        tokio::spawn(async move {
            info!("Maintenance task starting for peer {}", peer_id);
            let mut interval = tokio::time::interval(Duration::from_secs(30));

            loop {
                // Check shutdown signal
                if shutdown_signal.load(Ordering::Relaxed) {
                    info!("Maintenance task shutting down for peer {}", peer_id);
                    break;
                }

                interval.tick().await;

                // Cleanup stale peers
                peer_manager.cleanup_stale_peers().await;

                // Run peer discovery if needed
                if discovery.needs_discovery().await {
                    // Use a timeout for the transport read lock
                    match tokio::time::timeout(Duration::from_millis(100), transport.read()).await {
                        Ok(transport_guard) => {
                            if let Ok(new_peers) = discovery.discover_peers(&transport_guard).await
                            {
                                info!(
                                    "Discovered {} new peers through discovery protocol",
                                    new_peers.len()
                                );
                            }
                        }
                        Err(_) => {
                            debug!("Transport lock timeout in maintenance task");
                        }
                    }
                }

                // Start receivers for any pending bootstrap connections
                let pending_connections = discovery.take_pending_bootstrap_connections().await;
                for (peer_id, connection) in pending_connections {
                    info!("Starting QUIC receiver for bootstrap peer {}", peer_id);
                    network.start_quic_receiver(connection, peer_id).await;
                }

                // Cleanup stale discovered peers
                discovery.cleanup_stale_peers().await;

                // Attempt to reconnect to important disconnected peers
                let disconnected_peers = peer_manager.get_disconnected_peers().await;
                for (peer_id, addresses) in disconnected_peers.iter().take(3) {
                    // Only try to reconnect to high-reputation peers
                    if let Some(peer_info) = peer_manager.get_peer(peer_id).await {
                        if peer_info.reputation_score > 70.0 {
                            info!(
                                peer_id = %peer_id,
                                reputation = peer_info.reputation_score,
                                addresses = addresses.len(),
                                "🔄 Attempting to reconnect to important peer"
                            );

                            // Try to reconnect using the first available address
                            if let Some(addr) = addresses.first() {
                                if let Some(socket_addr) = network.multiaddr_to_socket_addr(addr) {
                                    match timeout(
                                        Duration::from_secs(10),
                                        network.connect_peer(socket_addr),
                                    )
                                    .await
                                    {
                                        Ok(Ok(_)) => {
                                            info!(
                                                peer_id = %peer_id,
                                                "✅ Successfully reconnected to peer"
                                            );
                                        }
                                        Ok(Err(e)) => {
                                            debug!(
                                                peer_id = %peer_id,
                                                error = %e,
                                                "Failed to reconnect to peer"
                                            );
                                        }
                                        Err(_) => {
                                            debug!(
                                                peer_id = %peer_id,
                                                "Reconnection attempt timed out"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Optimize routing
                router.optimize_routes().await;

                // Check network health
                resilience.check_network_health().await;
            }
        });
    }

    /// Handle incoming sync requests from peers
    async fn handle_sync_request(
        &self,
        request: crate::protocol::sync::SyncRequest,
        from_peer: PeerId,
    ) -> Result<()> {
        use crate::protocol::sync::{SyncRequest, SyncResponse};

        let response = match request {
            SyncRequest::GetFrontier => {
                // Get tips as our frontier - tips represent the latest messages
                let tips = self
                    .storage
                    .get_tips()
                    .await
                    .map_err(|e| AdicError::Storage(format!("Failed to get tips: {}", e)))?;
                info!(
                    "Sending frontier of {} tips to peer {}",
                    tips.len(),
                    from_peer
                );
                SyncResponse::Frontier(tips)
            }
            SyncRequest::GetMessages(message_ids) => {
                // Fetch requested messages from storage
                let mut messages = Vec::new();
                for id in &message_ids {
                    if let Ok(Some(msg)) = self.storage.get_message(id).await {
                        messages.push(msg);
                    }
                }
                info!(
                    "Sending {} messages to peer {} (requested {})",
                    messages.len(),
                    from_peer,
                    message_ids.len()
                );
                SyncResponse::Messages(messages)
            }
            _ => {
                // Handle other sync request types if needed
                SyncResponse::Error("Unsupported sync request type".to_string())
            }
        };

        // Send response back to peer
        self.send_sync_response(from_peer, response).await
    }

    /// Handle incoming sync responses from peers
    async fn handle_sync_response(
        &self,
        response: crate::protocol::sync::SyncResponse,
        from_peer: PeerId,
    ) -> Result<()> {
        use crate::protocol::sync::SyncResponse;

        match response {
            SyncResponse::Frontier(peer_tips) => {
                info!(
                    "Received frontier of {} tips from peer {}",
                    peer_tips.len(),
                    from_peer
                );

                // Get our local tips
                let our_tips = self
                    .storage
                    .get_tips()
                    .await
                    .map_err(|e| AdicError::Storage(format!("Failed to get tips: {}", e)))?;
                let our_set: HashSet<_> = our_tips.into_iter().collect();

                // Find tips we don't have - these represent branches we're missing
                let missing: Vec<MessageId> = peer_tips
                    .into_iter()
                    .filter(|id| !our_set.contains(id))
                    .collect();

                if !missing.is_empty() {
                    info!(
                        "Requesting {} missing tips/messages from peer {}",
                        missing.len(),
                        from_peer
                    );
                    // Request the missing messages
                    self.send_sync_request(
                        from_peer,
                        crate::protocol::sync::SyncRequest::GetMessages(missing),
                    )
                    .await?;
                } else {
                    info!("No missing tips from peer {}", from_peer);
                }
            }
            SyncResponse::Messages(messages) => {
                info!(
                    "Received {} messages from peer {} via sync",
                    messages.len(),
                    from_peer
                );
                // Process each message
                for msg in messages {
                    // Handle the message as if it came through normal channels
                    if let Err(e) = self.handle_incoming_message(&msg, from_peer).await {
                        warn!(
                            "Failed to process synced message {}: {}",
                            hex::encode(&msg.id.as_bytes()[..8]),
                            e
                        );
                    }
                }
            }
            SyncResponse::Error(err) => {
                warn!("Sync error from peer {}: {}", from_peer, err);
            }
            _ => {
                debug!("Unhandled sync response type from peer {}", from_peer);
            }
        }

        Ok(())
    }

    /// Send a sync request to a peer
    async fn send_sync_request(
        &self,
        peer: PeerId,
        request: crate::protocol::sync::SyncRequest,
    ) -> Result<()> {
        let msg = NetworkMessage::SyncRequest(request);
        let data = serde_json::to_vec(&msg).map_err(|e| {
            AdicError::Serialization(format!("Failed to serialize sync request: {}", e))
        })?;

        // Get connection to peer
        let transport = self.transport.read().await;
        let pool = transport.connection_pool();

        if let Some(conn) = pool.get_connection(&peer).await {
            match conn.open_uni().await {
                Ok(mut stream) => {
                    stream.write_all(&data).await.map_err(|e| {
                        AdicError::Network(format!("Failed to send sync request: {}", e))
                    })?;
                    stream.finish().map_err(|e| {
                        AdicError::Network(format!("Failed to finish stream: {}", e))
                    })?;

                    // Update peer stats to keep peer alive
                    self.peer_manager
                        .update_peer_stats(&peer, true, data.len() as u64, None)
                        .await;

                    debug!("Sent sync request to peer {}", peer);
                    Ok(())
                }
                Err(e) => Err(AdicError::Network(format!(
                    "Failed to open stream to peer {}: {}",
                    peer, e
                ))),
            }
        } else {
            Err(AdicError::Network(format!(
                "No connection to peer {}",
                peer
            )))
        }
    }

    /// Send a sync response to a peer
    async fn send_sync_response(
        &self,
        peer: PeerId,
        response: crate::protocol::sync::SyncResponse,
    ) -> Result<()> {
        let msg = NetworkMessage::SyncResponse(response);
        let data = serde_json::to_vec(&msg).map_err(|e| {
            AdicError::Serialization(format!("Failed to serialize sync response: {}", e))
        })?;

        // Get connection to peer
        let transport = self.transport.read().await;
        let pool = transport.connection_pool();

        if let Some(conn) = pool.get_connection(&peer).await {
            match conn.open_uni().await {
                Ok(mut stream) => {
                    stream.write_all(&data).await.map_err(|e| {
                        AdicError::Network(format!("Failed to send sync response: {}", e))
                    })?;
                    stream.finish().map_err(|e| {
                        AdicError::Network(format!("Failed to finish stream: {}", e))
                    })?;

                    // Update peer stats to keep peer alive
                    self.peer_manager
                        .update_peer_stats(&peer, true, data.len() as u64, None)
                        .await;

                    debug!("Sent sync response to peer {}", peer);
                    Ok(())
                }
                Err(e) => Err(AdicError::Network(format!(
                    "Failed to open stream to peer {}: {}",
                    peer, e
                ))),
            }
        } else {
            Err(AdicError::Network(format!(
                "No connection to peer {}",
                peer
            )))
        }
    }

    /// Trigger sync with all connected peers (used after adding new messages)
    async fn trigger_sync_with_peers(&self) {
        let transport = self.transport.read().await;
        let pool = transport.connection_pool();
        let connections = pool.get_all_connections().await;
        drop(transport);

        for (peer_id, _) in connections {
            // Send our frontier to trigger sync
            if let Err(e) = self
                .send_sync_request(peer_id, crate::protocol::sync::SyncRequest::GetFrontier)
                .await
            {
                debug!("Failed to trigger sync with {}: {}", peer_id, e);
            }
        }
    }

    /// Get access to the transport layer for sending messages
    pub async fn transport(&self) -> tokio::sync::RwLockReadGuard<'_, HybridTransport> {
        self.transport.read().await
    }

    pub async fn shutdown(&self) -> Result<()> {
        // Signal all background tasks to shut down
        self.shutdown_signal.store(true, Ordering::Relaxed);

        // Give background tasks a moment to notice the shutdown signal
        info!("[SHUTDOWN] Sleeping 200ms for background tasks to notice shutdown signal...");
        tokio::time::sleep(Duration::from_millis(200)).await;
        info!("[SHUTDOWN] Sleep completed, checking current lock holders...");

        // Try to get some info about current transport lock state
        info!("[SHUTDOWN] Checking if transport read lock can be acquired quickly...");
        match self.transport.try_read() {
            Ok(guard) => {
                info!("[SHUTDOWN] Quick read lock acquired successfully, releasing immediately");
                drop(guard);
            }
            Err(_) => {
                warn!("[SHUTDOWN] Transport read lock is currently held by another task");
                warn!(
                    "[SHUTDOWN] This indicates that background tasks are still using the transport"
                );
            }
        }

        info!("[SHUTDOWN] Attempting to acquire transport write lock with timeout...");
        // Use a timeout for the transport lock to avoid indefinite hanging
        match tokio::time::timeout(Duration::from_secs(3), self.transport.write()).await {
            Ok(mut transport) => {
                info!("[SHUTDOWN] Transport write lock acquired successfully!");
                info!("[SHUTDOWN] Calling transport.shutdown()...");
                match transport.shutdown().await {
                    Ok(()) => {
                        info!("[SHUTDOWN] Transport shutdown completed successfully");
                    }
                    Err(e) => {
                        error!("[SHUTDOWN] Transport shutdown failed: {}", e);
                        return Err(e);
                    }
                }
            }
            Err(_) => {
                error!("[SHUTDOWN] Transport write lock acquisition timed out after 3 seconds!");
                error!("[SHUTDOWN] This indicates a deadlock - some background task is holding a read lock");

                // Try to identify what might be holding the lock
                info!("[SHUTDOWN] Attempting to identify lock holders...");
                self.debug_potential_lock_holders().await;

                warn!("[SHUTDOWN] Forcing shutdown due to timeout");
                // In case of timeout, we'll just log and continue
                // This prevents tests from hanging indefinitely
            }
        }

        info!(
            "========== SHUTDOWN COMPLETE for peer {} ==========",
            self.peer_id
        );
        Ok(())
    }

    async fn debug_potential_lock_holders(&self) {}

    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    pub fn peer_manager(&self) -> &PeerManager {
        &self.peer_manager
    }

    pub fn state_sync(&self) -> &StateSync {
        &self.state_sync
    }

    pub fn router(&self) -> &HypertangleRouter {
        &self.router
    }

    pub fn get_update_protocol(&self) -> Option<Arc<UpdateProtocol>> {
        Some(self.update_protocol.clone())
    }

    pub async fn get_connected_peers(&self) -> Vec<PeerId> {
        self.peer_manager.get_connected_peers().await
    }

    pub async fn get_all_peers_info(&self) -> Vec<peer::PeerInfo> {
        self.peer_manager.get_all_peers_info().await
    }

    pub async fn get_network_stats(&self) -> NetworkStats {
        NetworkStats {
            peer_count: self.peer_manager.peer_count().await,
            connected_peers: self.peer_manager.connected_peer_count().await,
            messages_sent: 0, // Would get from metrics
            messages_received: 0,
            bytes_sent: 0,
            bytes_received: 0,
        }
    }

    fn multiaddr_to_socket_addr(&self, addr: &Multiaddr) -> Option<std::net::SocketAddr> {
        use libp2p::multiaddr::Protocol;

        let mut ip = None;
        let mut port = None;

        for proto in addr.iter() {
            match proto {
                Protocol::Ip4(addr) => ip = Some(std::net::IpAddr::V4(addr)),
                Protocol::Ip6(addr) => ip = Some(std::net::IpAddr::V6(addr)),
                Protocol::Tcp(p) | Protocol::Udp(p) => port = Some(p),
                _ => {}
            }
        }

        if let (Some(ip), Some(port)) = (ip, port) {
            Some(std::net::SocketAddr::new(ip, port))
        } else {
            None
        }
    }

    /// Handle update messages for P2P binary distribution
    async fn handle_update_message(
        &self,
        message: crate::protocol::update::UpdateMessage,
        from_peer: PeerId,
    ) -> Result<()> {
        // Process through update protocol
        match self
            .update_protocol
            .handle_message(message.clone(), from_peer)
            .await
            .map_err(|e| AdicError::Network(format!("Update protocol error: {}", e)))?
        {
            Some(response) => {
                // Send response back to peer
                let transport = self.transport.read().await;
                transport.send_update_message(&from_peer, response).await?;
            }
            None => {
                // No response needed
            }
        }
        Ok(())
    }

    /// Get version information for all known peers
    pub async fn get_peer_versions(&self) -> HashMap<String, String> {
        // Get all peer versions from the update protocol
        let all_versions = self.update_protocol.get_all_peer_versions().await;

        // Convert PeerId to String for the result
        let mut result = HashMap::new();
        for (version, peer_ids) in all_versions {
            for peer_id in peer_ids {
                result.insert(peer_id.to_string(), version.clone());
            }
        }

        result
    }

    /// Get current node version
    pub fn get_current_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NetworkStats {
    pub peer_count: usize,
    pub connected_peers: usize,
    pub messages_sent: u64,
    pub messages_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    #[tokio::test]
    async fn test_network_engine_creation() {
        let _keypair = Keypair::generate_ed25519();
        let _config = NetworkConfig::default();

        // Would need mock storage, consensus, and finality engines for full test
    }

    #[tokio::test]
    async fn test_multiaddr_to_socket_addr() {
        use adic_consensus::ConsensusEngine;
        use adic_finality::{FinalityConfig, FinalityEngine};
        use adic_storage::{StorageConfig, StorageEngine};
        use adic_types::AdicParams;
        use tempfile::tempdir;

        // Create a minimal NetworkEngine for testing
        let params = AdicParams::default();
        let keypair = Keypair::generate_ed25519();

        let temp_dir = tempdir().unwrap();
        let storage_config = StorageConfig {
            backend_type: adic_storage::store::BackendType::RocksDB {
                path: temp_dir.path().join("test").to_str().unwrap().to_string(),
            },
            ..Default::default()
        };
        let storage = Arc::new(StorageEngine::new(storage_config).unwrap());
        let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
        let finality = Arc::new(FinalityEngine::new(
            FinalityConfig::from(&params),
            consensus.clone(),
            storage.clone(),
        ));

        let config = NetworkConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            ..Default::default()
        };

        // Try to create network engine, skip test if port is unavailable
        let network = match NetworkEngine::new(config, keypair, storage, consensus, finality).await
        {
            Ok(n) => n,
            Err(e) if e.to_string().contains("Address already in use") => {
                eprintln!("Skipping test due to port conflict: {}", e);
                return;
            }
            Err(e) => panic!("Unexpected error: {}", e),
        };

        // Test IPv4 address
        let addr_ipv4: Multiaddr = "/ip4/127.0.0.1/tcp/9000".parse().unwrap();
        let socket_addr = network.multiaddr_to_socket_addr(&addr_ipv4);
        assert!(socket_addr.is_some());
        assert_eq!(
            socket_addr.unwrap(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 9000)
        );

        // Test IPv6 address
        let addr_ipv6: Multiaddr = "/ip6/::1/tcp/9001".parse().unwrap();
        let socket_addr = network.multiaddr_to_socket_addr(&addr_ipv6);
        assert!(socket_addr.is_some());
        assert_eq!(
            socket_addr.unwrap(),
            SocketAddr::new(IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1)), 9001)
        );

        // Test UDP address
        let addr_udp: Multiaddr = "/ip4/192.168.1.1/udp/5000".parse().unwrap();
        let socket_addr = network.multiaddr_to_socket_addr(&addr_udp);
        assert!(socket_addr.is_some());
        assert_eq!(
            socket_addr.unwrap(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)), 5000)
        );

        // Test invalid address (no port)
        let addr_no_port: Multiaddr = "/ip4/127.0.0.1".parse().unwrap();
        let socket_addr = network.multiaddr_to_socket_addr(&addr_no_port);
        assert!(socket_addr.is_none());

        // Test invalid address (no IP)
        let addr_no_ip: Multiaddr = "/tcp/9000".parse().unwrap();
        let socket_addr = network.multiaddr_to_socket_addr(&addr_no_ip);
        assert!(socket_addr.is_none());

        std::mem::forget(temp_dir); // Prevent cleanup during test
    }
}
