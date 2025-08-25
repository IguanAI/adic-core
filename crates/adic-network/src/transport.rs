use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use std::collections::HashMap;

use libp2p::{
    core::{muxing::StreamMuxerBox, transport::Boxed, upgrade},
    identity::Keypair,
    noise, yamux,
    Transport, PeerId, Multiaddr,
};
use quinn::{Endpoint, ServerConfig, ClientConfig, Connection};
use tokio::sync::{RwLock, Semaphore};
use thiserror::Error;
use tracing::{debug, error, info};

use adic_types::{Result, AdicError};

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    
    #[error("Transport initialization failed: {0}")]
    InitializationFailed(String),
    
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    
    #[error("Connection pool exhausted")]
    PoolExhausted,
    
    #[error("QUIC error: {0}")]
    QuicError(#[from] quinn::ConnectionError),
    
    #[error("Libp2p error: {0}")]
    Libp2pError(String),
}

impl From<TransportError> for AdicError {
    fn from(e: TransportError) -> Self {
        AdicError::Network(e.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct TransportConfig {
    pub libp2p_listen_addrs: Vec<Multiaddr>,
    pub quic_listen_addr: SocketAddr,
    pub max_connections: usize,
    pub connection_timeout: Duration,
    pub keep_alive_interval: Duration,
    pub max_idle_timeout: Duration,
    pub mtu_discovery: bool,
    pub initial_mtu: u16,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            libp2p_listen_addrs: vec!["/ip4/0.0.0.0/tcp/9000".parse().unwrap()],
            quic_listen_addr: "0.0.0.0:9001".parse().unwrap(),
            max_connections: 1000,
            connection_timeout: Duration::from_secs(10),
            keep_alive_interval: Duration::from_secs(15),
            max_idle_timeout: Duration::from_secs(60),
            mtu_discovery: true,
            initial_mtu: 1200,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityLane {
    Consensus = 0,
    Data = 1,
    Discovery = 2,
}

pub struct ConnectionPool {
    connections: Arc<RwLock<HashMap<PeerId, Arc<Connection>>>>,
    semaphore: Arc<Semaphore>,
    max_connections: usize,
}

impl ConnectionPool {
    pub fn new(max_connections: usize) -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
            semaphore: Arc::new(Semaphore::new(max_connections)),
            max_connections,
        }
    }

    pub async fn add_connection(&self, peer_id: PeerId, conn: Connection) -> Result<()> {
        let permit = self.semaphore.acquire().await
            .map_err(|_| TransportError::PoolExhausted)?;
        
        let mut connections = self.connections.write().await;
        connections.insert(peer_id, Arc::new(conn));
        std::mem::forget(permit); // Keep the permit alive
        
        Ok(())
    }

    pub async fn get_connection(&self, peer_id: &PeerId) -> Option<Arc<Connection>> {
        let connections = self.connections.read().await;
        connections.get(peer_id).cloned()
    }

    pub async fn remove_connection(&self, peer_id: &PeerId) -> Option<Arc<Connection>> {
        let mut connections = self.connections.write().await;
        let conn = connections.remove(peer_id);
        if conn.is_some() {
            self.semaphore.add_permits(1);
        }
        conn
    }

    pub async fn connection_count(&self) -> usize {
        let connections = self.connections.read().await;
        connections.len()
    }

    pub async fn is_full(&self) -> bool {
        self.connection_count().await >= self.max_connections
    }
}

pub struct HybridTransport {
    config: TransportConfig,
    keypair: Keypair,
    peer_id: PeerId,
    libp2p_transport: Option<Boxed<(PeerId, StreamMuxerBox)>>,
    quic_endpoint: Option<Endpoint>,
    connection_pool: ConnectionPool,
}

impl HybridTransport {
    pub fn new(config: TransportConfig, keypair: Keypair) -> Self {
        let peer_id = PeerId::from(keypair.public());
        let max_connections = config.max_connections;
        
        Self {
            config,
            keypair,
            peer_id,
            libp2p_transport: None,
            quic_endpoint: None,
            connection_pool: ConnectionPool::new(max_connections),
        }
    }

    pub async fn initialize(&mut self) -> Result<()> {
        self.init_libp2p_transport()?;
        self.init_quic_transport().await?;
        
        info!("Transport layer initialized with peer ID: {}", self.peer_id);
        Ok(())
    }

    fn init_libp2p_transport(&mut self) -> Result<()> {
        // Create TCP transport
        let tcp = libp2p::tcp::tokio::Transport::new(
            libp2p::tcp::Config::default().nodelay(true)
        );

        // Build the transport stack
        let transport = tcp
            .upgrade(upgrade::Version::V1)
            .authenticate(noise::Config::new(&self.keypair).unwrap())
            .multiplex(yamux::Config::default())
            .boxed();

        self.libp2p_transport = Some(transport);
        
        debug!("Libp2p transport initialized");
        Ok(())
    }

    async fn init_quic_transport(&mut self) -> Result<()> {
        let server_config = self.build_server_config()?;
        let endpoint = Endpoint::server(server_config, self.config.quic_listen_addr)
            .map_err(|e| TransportError::InitializationFailed(e.to_string()))?;

        self.quic_endpoint = Some(endpoint);
        
        debug!("QUIC transport initialized on {}", self.config.quic_listen_addr);
        Ok(())
    }

    fn build_server_config(&self) -> Result<ServerConfig> {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .map_err(|e| TransportError::InitializationFailed(e.to_string()))?;
        
        let cert_der = cert.cert.der().to_vec();
        let priv_key_der = cert.key_pair.serialize_der();
        
        let priv_key = rustls::pki_types::PrivateKeyDer::try_from(priv_key_der)
            .map_err(|e| TransportError::InitializationFailed(format!("Invalid private key: {:?}", e)))?;
        let cert_chain = vec![rustls::pki_types::CertificateDer::from(cert_der)];

        let server_config = ServerConfig::with_single_cert(cert_chain, priv_key.into())
            .map_err(|e| TransportError::InitializationFailed(e.to_string()))?;

        Ok(server_config)
    }

    pub async fn connect_quic(&self, addr: SocketAddr) -> Result<Connection> {
        let endpoint = self.quic_endpoint.as_ref()
            .ok_or_else(|| TransportError::InitializationFailed("QUIC not initialized".into()))?;

        let client_config = self.build_client_config()?;
        let connecting = endpoint
            .connect_with(client_config, addr, "localhost")
            .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;
        
        let connection = connecting.await
            .map_err(|e| TransportError::QuicError(e))?;

        debug!("QUIC connection established to {}", addr);
        Ok(connection)
    }

    fn build_client_config(&self) -> Result<ClientConfig> {
        // For development, use a simple insecure client config
        // In production, proper certificate verification should be implemented
        let crypto = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SkipServerVerification::new()))
            .with_no_client_auth();

        let client_config = ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
                .map_err(|e| TransportError::InitializationFailed(format!("Failed to create QUIC config: {}", e)))?
        ));
        Ok(client_config)
    }

    pub async fn accept_quic(&self) -> Result<Connection> {
        let endpoint = self.quic_endpoint.as_ref()
            .ok_or_else(|| TransportError::InitializationFailed("QUIC not initialized".into()))?;

        let connecting = endpoint.accept().await
            .ok_or_else(|| TransportError::ConnectionFailed("No incoming connection".into()))?;

        let connection = connecting.await
            .map_err(|e| TransportError::QuicError(e))?;
        
        debug!("QUIC connection accepted from {}", connection.remote_address());
        Ok(connection)
    }

    pub fn libp2p_transport(&self) -> Option<&Boxed<(PeerId, StreamMuxerBox)>> {
        self.libp2p_transport.as_ref()
    }

    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    pub fn connection_pool(&self) -> &ConnectionPool {
        &self.connection_pool
    }

    pub async fn send_with_priority(
        &self,
        peer_id: &PeerId,
        data: &[u8],
        priority: PriorityLane,
    ) -> Result<()> {
        if let Some(conn) = self.connection_pool.get_connection(peer_id).await {
            let mut stream = conn.open_uni().await
                .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;
            
            // Build message with header
            let mut message = Vec::new();
            message.push(priority as u8);
            message.extend_from_slice(&(data.len() as u32).to_be_bytes());
            message.extend_from_slice(data);
            
            // Write all data at once
            stream.write_all(&message).await
                .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;
            
            stream.finish()
                .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;
            
            Ok(())
        } else {
            Err(TransportError::ConnectionFailed("Peer not connected".into()).into())
        }
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(endpoint) = self.quic_endpoint.take() {
            endpoint.close(0u32.into(), b"shutting down");
        }
        
        info!("Transport layer shutdown complete");
        Ok(())
    }
}

// Simplified certificate verification for development
// In production, use proper certificate verification
#[derive(Debug)]
struct SkipServerVerification;

impl SkipServerVerification {
    fn new() -> Self {
        Self
    }
}

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    
    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_transport_initialization() {
        let keypair = Keypair::generate_ed25519();
        let config = TransportConfig::default();
        let mut transport = HybridTransport::new(config, keypair);
        
        assert!(transport.initialize().await.is_ok());
        assert!(transport.libp2p_transport.is_some());
        assert!(transport.quic_endpoint.is_some());
    }

    #[tokio::test]
    async fn test_connection_pool() {
        let pool = ConnectionPool::new(10);
        assert_eq!(pool.connection_count().await, 0);
        assert!(!pool.is_full().await);
    }
}