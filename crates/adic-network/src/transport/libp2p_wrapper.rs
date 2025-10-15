// SECURITY NOTE: This wrapper is a minimal stub because libp2p transport is disabled by default.
// ADIC uses QUIC (via Quinn) as the primary transport, not libp2p TCP.
// libp2p is only used for gossipsub (pubsub) and Kademlia (peer discovery), not as a transport.
//
// The libp2p transport in HybridTransport is disabled by default (libp2p_listen_addrs: vec![]).
// This stub exists to satisfy the type system when libp2p transport is completely disabled.

use adic_types::{AdicError, Result};
use libp2p::{identity::Keypair, PeerId};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Minimal libp2p transport wrapper (disabled by default in favor of QUIC)
///
/// SECURITY: This wrapper no longer creates actual libp2p TCP transports because:
/// 1. We removed tcp, noise, yamux features to minimize attack surface
/// 2. QUIC is the primary transport with production-grade TLS
/// 3. libp2p is only used for gossipsub and Kademlia, not for connections
#[derive(Clone)]
pub struct LibP2PTransportWrapper {
    _transport: Arc<Mutex<Option<()>>>, // Placeholder, transport is disabled
    peer_id: PeerId,
}

impl LibP2PTransportWrapper {
    pub fn new(keypair: &Keypair) -> Result<Self> {
        tracing::debug!("LibP2PTransportWrapper::new - Creating minimal wrapper (transport disabled)");
        let peer_id = PeerId::from(keypair.public());
        tracing::debug!("LibP2PTransportWrapper::new - PeerId created: {}", peer_id);

        // SECURITY: No actual transport created - we use QUIC instead
        // This is just a PeerId holder for libp2p protocol compatibility
        tracing::info!(
            "LibP2P transport wrapper initialized (stub only - using QUIC for connections)"
        );

        Ok(Self {
            _transport: Arc::new(Mutex::new(None)),
            peer_id,
        })
    }

    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub async fn dial(&self, addr: libp2p::Multiaddr) -> Result<()> {
        // SECURITY: This is intentionally a no-op
        // Actual connections are made via QUIC in HybridTransport
        tracing::warn!(
            "LibP2P transport dial called but transport is disabled. Use QUIC for connections: {}",
            addr
        );
        Err(AdicError::Network(
            "LibP2P transport is disabled - use QUIC instead".into(),
        ))
    }

    pub async fn listen_on(&self, addr: libp2p::Multiaddr) -> Result<()> {
        // SECURITY: This is intentionally a no-op
        // Actual listening is done via QUIC in HybridTransport
        tracing::warn!(
            "LibP2P transport listen called but transport is disabled. Use QUIC for listening: {}",
            addr
        );
        Err(AdicError::Network(
            "LibP2P transport is disabled - use QUIC instead".into(),
        ))
    }
}

// LibP2PTransportWrapper is Send + Sync because:
// 1. Arc<Mutex<Option<()>>> is Send + Sync
// 2. PeerId is Send + Sync
// The compiler can derive this automatically.
