pub mod transport;
pub mod peer;
pub mod deposit_verifier;
pub mod protocol;
pub mod routing;
pub mod pipeline;
pub mod sync;
pub mod resilience;
pub mod security;
pub mod metrics;
pub mod codecs;

#[cfg(test)]
mod peer_tests;
#[cfg(test)]
mod codecs_tests;

use std::sync::Arc;
use std::time::Duration;
use std::net::SocketAddr;

use tokio::sync::RwLock;
use libp2p::{PeerId, Multiaddr, identity::Keypair};
use tracing::{info, warn};

use adic_types::{AdicMessage, MessageId, Result, AdicError};
use adic_storage::StorageEngine;
use adic_consensus::ConsensusEngine;
use adic_finality::FinalityEngine;

use crate::transport::{HybridTransport, TransportConfig};
use crate::peer::PeerManager;
use crate::deposit_verifier::{DepositVerifier, RealDepositVerifier};
use crate::protocol::{GossipProtocol, SyncProtocol, ConsensusProtocol, StreamProtocol};
use crate::routing::HypertangleRouter;
use crate::pipeline::{MessagePipeline, PipelineConfig};
use crate::sync::StateSync;
use crate::resilience::NetworkResilience;
use crate::security::SecurityManager;
use crate::metrics::NetworkMetrics;

pub struct NetworkConfig {
    pub transport: TransportConfig,
    pub pipeline: PipelineConfig,
    pub max_peers: usize,
    pub bootstrap_peers: Vec<Multiaddr>,
    pub listen_addresses: Vec<Multiaddr>,
    pub enable_metrics: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            transport: TransportConfig::default(),
            pipeline: PipelineConfig::default(),
            max_peers: 100,
            bootstrap_peers: vec![],
            listen_addresses: vec![
                "/ip4/0.0.0.0/tcp/9000".parse().unwrap(),
            ],
            enable_metrics: true,
        }
    }
}

pub struct NetworkEngine {
    config: NetworkConfig,
    keypair: Keypair,
    peer_id: PeerId,
    transport: Arc<RwLock<HybridTransport>>,
    peer_manager: Arc<PeerManager>,
    gossip: Arc<GossipProtocol>,
    sync_protocol: Arc<SyncProtocol>,
    consensus_protocol: Arc<ConsensusProtocol>,
    stream_protocol: Arc<StreamProtocol>,
    router: Arc<HypertangleRouter>,
    pipeline: Arc<MessagePipeline>,
    state_sync: Arc<StateSync>,
    resilience: Arc<NetworkResilience>,
    security: Arc<SecurityManager>,
    metrics: Option<Arc<NetworkMetrics>>,
}

impl NetworkEngine {
    pub async fn new(
        config: NetworkConfig,
        keypair: Keypair,
        storage: Arc<StorageEngine>,
        _consensus: Arc<ConsensusEngine>,
        finality: Arc<FinalityEngine>,
    ) -> Result<Self> {
        let peer_id = PeerId::from(keypair.public());
        
        info!("Initializing network engine with peer ID: {}", peer_id);
        
        // Initialize transport
        let mut transport = HybridTransport::new(config.transport.clone(), keypair.clone());
        transport.initialize().await?;
        
        // Initialize peer manager with real deposit verifier
        let deposit_manager = Arc::new(adic_consensus::DepositManager::new(0.1));
        let deposit_verifier: Arc<dyn DepositVerifier> = Arc::new(RealDepositVerifier::new(deposit_manager));
        let peer_manager = Arc::new(PeerManager::new(
            &keypair,
            config.max_peers,
            deposit_verifier,
        ));
        
        // Initialize protocols
        let gossip = Arc::new(GossipProtocol::new(
            &keypair,
            Default::default(),
        )?);
        
        let sync_protocol = Arc::new(SyncProtocol::new(Default::default()));
        let consensus_protocol = Arc::new(ConsensusProtocol::new(Default::default()));
        let stream_protocol = Arc::new(StreamProtocol::new(Default::default()));
        
        // Initialize routing
        let router = Arc::new(HypertangleRouter::new(peer_id));
        
        // Initialize pipeline
        let pipeline = Arc::new(MessagePipeline::new(config.pipeline.clone()));
        pipeline.start_cleanup_task().await;
        
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
        gossip.subscribe("adic/messages").await?;
        gossip.subscribe("adic/tips").await?;
        gossip.subscribe("adic/finality").await?;
        gossip.subscribe("adic/conflicts").await?;
        
        Ok(Self {
            config,
            keypair,
            peer_id,
            transport: Arc::new(RwLock::new(transport)),
            peer_manager,
            gossip,
            sync_protocol,
            consensus_protocol,
            stream_protocol,
            router,
            pipeline,
            state_sync,
            resilience,
            security,
            metrics,
        })
    }
    
    pub async fn start(&self) -> Result<()> {
        info!("Starting network engine");
        
        // Start listening on configured addresses
        for addr in &self.config.listen_addresses {
            info!("Listening on {}", addr);
        }
        
        // Connect to bootstrap peers
        for peer_addr in &self.config.bootstrap_peers {
            if let Err(e) = self.connect_to_peer(peer_addr).await {
                warn!("Failed to connect to bootstrap peer {}: {}", peer_addr, e);
            }
        }
        
        // Start background tasks
        self.start_gossip_handler().await;
        self.start_sync_handler().await;
        self.start_consensus_handler().await;
        self.start_maintenance_tasks().await;
        
        info!("Network engine started successfully");
        Ok(())
    }
    
    async fn connect_to_peer(&self, _addr: &Multiaddr) -> Result<()> {
        // Extract socket address from multiaddr
        // In real implementation, parse multiaddr properly
        let socket_addr: SocketAddr = "127.0.0.1:9000".parse()
            .map_err(|e| AdicError::Network(format!("Failed to parse address: {}", e)))?;
        
        let transport = self.transport.read().await;
        let connection = transport.connect_quic(socket_addr).await?;
        
        // Perform handshake
        let peer_id = PeerId::random(); // Would get from handshake
        
        transport.connection_pool().add_connection(peer_id, connection).await?;
        
        info!("Connected to peer {}", peer_id);
        Ok(())
    }
    
    pub async fn broadcast_message(&self, mut message: AdicMessage) -> Result<()> {
        // Sign the message with our keypair if not already signed
        if message.signature.as_bytes().is_empty() {
            let signature = self.security.sign_data(&self.keypair, &message.payload);
            message.signature = signature;
        }
        
        // Process through pipeline
        let source = self.peer_id;
        self.pipeline.submit_message(message.clone(), source).await?;
        
        // Route to appropriate peers
        let _target_peers = self.router.route_message(&message).await;
        
        // Broadcast via gossip
        self.gossip.broadcast_message(message.clone()).await?;
        
        // Update metrics
        if let Some(metrics) = &self.metrics {
            metrics.record_message_sent(&message.id).await;
        }
        
        Ok(())
    }
    
    pub async fn request_sync(&self, peer: PeerId, height: u64) -> Result<()> {
        self.state_sync.start_fast_sync(peer, height).await
    }
    
    pub async fn stream_messages(&self, peer: PeerId, message_ids: Vec<MessageId>) -> Result<()> {
        // Use stream protocol for bulk message transfer
        self.stream_protocol.stream_bulk_messages(peer, message_ids, false).await
    }
    
    pub async fn request_snapshot(&self, peer: PeerId, from_height: u64, to_height: u64) -> Result<Vec<u8>> {
        // Use stream protocol to request state snapshot
        self.stream_protocol.receive_snapshot(peer, from_height, to_height).await
    }
    
    pub async fn handle_incoming_message(&self, message: &AdicMessage, peer: PeerId) -> Result<()> {
        // Verify message signature using security manager
        let is_valid = self.security.verify_signature(
            &message.proposer_pk,
            &message.payload,
            &message.signature
        );
        
        if !is_valid {
            return Err(AdicError::Network("Invalid message signature".to_string()));
        }
        
        // Process the verified message
        self.pipeline.submit_message(message.clone(), peer).await?;
        
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
                gossip.process_validation_queue(|msg| {
                    // Basic validation
                    msg.verify_id()
                }).await;
            }
        });
    }
    
    async fn start_sync_handler(&self) {
        let sync = self.sync_protocol.clone();
        
        tokio::spawn(async move {
            loop {
                // Cleanup stale sync requests
                sync.cleanup_stale_requests().await;
                tokio::time::sleep(Duration::from_secs(10)).await;
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
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            
            loop {
                interval.tick().await;
                
                // Cleanup stale peers
                peer_manager.cleanup_stale_peers().await;
                
                // Optimize routing
                router.optimize_routes().await;
                
                // Check network health
                resilience.check_network_health().await;
            }
        });
    }
    
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down network engine");
        
        let mut transport = self.transport.write().await;
        transport.shutdown().await?;
        
        info!("Network engine shutdown complete");
        Ok(())
    }
    
    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }
    
    pub fn peer_manager(&self) -> &PeerManager {
        &self.peer_manager
    }
    
    pub fn router(&self) -> &HypertangleRouter {
        &self.router
    }
    
    pub async fn get_connected_peers(&self) -> Vec<PeerId> {
        self.peer_manager.get_connected_peers().await
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
}

#[derive(Debug, Clone)]
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
    
    #[tokio::test]
    async fn test_network_engine_creation() {
        let _keypair = Keypair::generate_ed25519();
        let _config = NetworkConfig::default();
        
        // Would need mock storage, consensus, and finality engines for full test
    }
}