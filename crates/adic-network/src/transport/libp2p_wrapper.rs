use crate::transport::TransportError;
use adic_types::{AdicError, Result};
use libp2p::{
    core::{muxing::StreamMuxerBox, transport::Boxed},
    identity::Keypair,
    noise, tcp, yamux, PeerId, Transport,
};
use std::sync::Arc;
use tokio::sync::Mutex;

// Type alias for complex transport type
type TransportType = Boxed<(PeerId, StreamMuxerBox)>;

/// Thread-safe wrapper for libp2p transport
#[derive(Clone)]
pub struct LibP2PTransportWrapper {
    transport: Arc<Mutex<Option<TransportType>>>,
    peer_id: PeerId,
}

impl LibP2PTransportWrapper {
    pub fn new(keypair: &Keypair) -> Result<Self> {
        tracing::debug!("LibP2PTransportWrapper::new - Starting");
        let peer_id = PeerId::from(keypair.public());
        tracing::debug!("LibP2PTransportWrapper::new - PeerId created: {}", peer_id);

        // Build the transport
        tracing::debug!("LibP2PTransportWrapper::new - Creating TCP transport");
        let transport = tcp::tokio::Transport::new(tcp::Config::default().nodelay(true))
            .upgrade(libp2p::core::upgrade::Version::V1)
            .authenticate(
                noise::Config::new(keypair)
                    .map_err(|e| TransportError::InitializationFailed(e.to_string()))?,
            )
            .multiplex(yamux::Config::default())
            .boxed();
        tracing::debug!("LibP2PTransportWrapper::new - Transport created successfully");

        Ok(Self {
            transport: Arc::new(Mutex::new(Some(transport))),
            peer_id,
        })
    }

    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub async fn dial(&self, addr: libp2p::Multiaddr) -> Result<()> {
        let mut transport_guard = self.transport.lock().await;
        if let Some(_transport) = transport_guard.as_mut() {
            // In a real implementation, we'd handle the connection here
            // For now, just log the attempt
            tracing::debug!("Attempting to dial {} via libp2p", addr);
            Ok(())
        } else {
            Err(AdicError::Network("Transport not initialized".into()))
        }
    }

    pub async fn listen_on(&self, addr: libp2p::Multiaddr) -> Result<()> {
        let mut transport_guard = self.transport.lock().await;
        if let Some(_transport) = transport_guard.as_mut() {
            // In a real implementation, we'd start listening here
            tracing::info!("Listening on {} via libp2p", addr);
            Ok(())
        } else {
            Err(AdicError::Network("Transport not initialized".into()))
        }
    }
}

// Safety: LibP2PTransportWrapper is Send + Sync because:
// 1. Arc<Mutex<T>> is Send + Sync when T is Send
// 2. Option<Boxed<(PeerId, StreamMuxerBox)>> is Send
// 3. PeerId is Send + Sync
// The unsafe impl is not actually needed since the compiler can derive this automatically.
// Removing the unsafe impl to let the compiler verify thread safety.
