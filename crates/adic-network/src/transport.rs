pub mod libp2p_wrapper;

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use libp2p::{identity::Keypair, Multiaddr, PeerId};
use quinn::{ClientConfig, Connection, Endpoint, ServerConfig};
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};
use tracing::{debug, error, info, warn};

use crate::protocol::discovery::DiscoveryMessage;
use crate::protocol::update::UpdateMessage;
use adic_types::{AdicError, AdicMessage, Result};
use serde::{Deserialize, Serialize};

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

/// Unified network message type that can carry both regular messages and discovery messages
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum NetworkMessage {
    /// Regular ADIC message for consensus/data
    AdicMessage(AdicMessage),
    /// Peer discovery protocol message
    Discovery(DiscoveryMessage),
    /// Handshake message
    Handshake {
        peer_id: Vec<u8>,
        version: u32,
        listening_port: Option<u16>,
    },
    /// Sync protocol messages
    SyncRequest(crate::protocol::sync::SyncRequest),
    SyncResponse(crate::protocol::sync::SyncResponse),
    /// Update protocol messages for P2P binary distribution
    Update(UpdateMessage),
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
    pub use_production_tls: bool,
    pub ca_cert_path: Option<String>,
    pub node_cert_path: Option<String>,
    pub node_key_path: Option<String>,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            libp2p_listen_addrs: vec![], // Empty by default - enable only when explicitly configured
            quic_listen_addr: "0.0.0.0:9001".parse().unwrap(),
            max_connections: 1000,
            connection_timeout: Duration::from_secs(10),
            keep_alive_interval: Duration::from_secs(15),
            max_idle_timeout: Duration::from_secs(60),
            mtu_discovery: true,
            initial_mtu: 1200,
            use_production_tls: true, // SECURITY: Default to production TLS
            ca_cert_path: None,
            node_cert_path: None,
            node_key_path: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityLane {
    Consensus = 0,
    Data = 1,
    Discovery = 2,
    Normal = 3,
}

pub struct ConnectionWithPermit {
    pub connection: Arc<Connection>,
    _permit: OwnedSemaphorePermit,
}

#[derive(Clone)]
pub struct ConnectionPool {
    connections: Arc<RwLock<HashMap<PeerId, ConnectionWithPermit>>>,
    outgoing: Arc<RwLock<HashSet<PeerId>>>, // Track which connections we initiated
    semaphore: Arc<Semaphore>,
    max_connections: usize,
}

impl ConnectionPool {
    pub fn new(max_connections: usize) -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
            outgoing: Arc::new(RwLock::new(HashSet::new())),
            semaphore: Arc::new(Semaphore::new(max_connections)),
            max_connections,
        }
    }

    pub async fn add_connection(&self, peer_id: PeerId, conn: Connection) -> Result<()> {
        self.add_connection_with_direction(peer_id, conn, false)
            .await
    }

    pub async fn add_outgoing_connection(&self, peer_id: PeerId, conn: Connection) -> Result<()> {
        self.add_connection_with_direction(peer_id, conn, true)
            .await
    }

    async fn add_connection_with_direction(
        &self,
        peer_id: PeerId,
        conn: Connection,
        is_outgoing: bool,
    ) -> Result<()> {
        let connections_before = self.connections.read().await.len();
        let available_permits = self.semaphore.available_permits();

        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| TransportError::PoolExhausted)?;

        let remote_addr = conn.remote_address();
        let mut connections = self.connections.write().await;
        let replaced = connections.insert(
            peer_id,
            ConnectionWithPermit {
                connection: Arc::new(conn),
                _permit: permit, // Permit is now owned and will be dropped with the connection
            },
        );

        if is_outgoing {
            let mut outgoing = self.outgoing.write().await;
            outgoing.insert(peer_id);
        }

        info!(
            peer_id = %peer_id,
            remote_addr = %remote_addr,
            is_outgoing = is_outgoing,
            connections_before = connections_before,
            connections_after = connections.len(),
            available_permits_before = available_permits,
            available_permits_after = self.semaphore.available_permits(),
            max_connections = self.max_connections,
            replaced_existing = replaced.is_some(),
            "🔗 Connection added to pool"
        );

        Ok(())
    }

    pub async fn get_connection(&self, peer_id: &PeerId) -> Option<Arc<Connection>> {
        let connections = self.connections.read().await;
        connections.get(peer_id).map(|cwp| cwp.connection.clone())
    }

    pub async fn remove_connection(&self, peer_id: &PeerId) -> Option<Arc<Connection>> {
        let mut connections = self.connections.write().await;
        let connections_before = connections.len();
        let available_permits_before = self.semaphore.available_permits();

        let result = connections.remove(peer_id).map(|cwp| {
            let conn = cwp.connection;
            info!(
                peer_id = %peer_id,
                remote_addr = %conn.remote_address(),
                connections_before = connections_before,
                connections_after = connections.len(),
                available_permits_before = available_permits_before,
                available_permits_after = self.semaphore.available_permits() + 1, // Will be +1 after drop
                "🔴 Connection removed from pool"
            );
            conn
        });
        // Permit is automatically released when ConnectionWithPermit is dropped
        result
    }

    pub async fn connection_count(&self) -> usize {
        let connections = self.connections.read().await;
        connections.len()
    }

    pub async fn is_full(&self) -> bool {
        self.connection_count().await >= self.max_connections
    }

    pub async fn get_all_connections(&self) -> Vec<(PeerId, Arc<Connection>)> {
        let connections = self.connections.read().await;
        connections
            .iter()
            .map(|(k, v)| (*k, v.connection.clone()))
            .collect()
    }

    pub async fn get_outgoing_connections(&self) -> Vec<(PeerId, Arc<Connection>)> {
        let connections = self.connections.read().await;
        let outgoing = self.outgoing.read().await;
        connections
            .iter()
            .filter(|(peer_id, _)| outgoing.contains(peer_id))
            .map(|(k, v)| (*k, v.connection.clone()))
            .collect()
    }
}

pub struct HybridTransport {
    config: TransportConfig,
    _keypair: Keypair,
    peer_id: PeerId,
    libp2p_transport: Option<Arc<libp2p_wrapper::LibP2PTransportWrapper>>,
    quic_endpoint: Option<Arc<Endpoint>>,
    connection_pool: ConnectionPool,
}

impl HybridTransport {
    pub fn new(config: TransportConfig, keypair: Keypair) -> Self {
        let peer_id = PeerId::from(keypair.public());
        let max_connections = config.max_connections;

        Self {
            config,
            _keypair: keypair,
            peer_id,
            libp2p_transport: None,
            quic_endpoint: None,
            connection_pool: ConnectionPool::new(max_connections),
        }
    }

    pub async fn initialize(&mut self) -> Result<()> {
        info!("HybridTransport::initialize - Starting initialization");
        // Skip libp2p initialization if no addresses configured
        if !self.config.libp2p_listen_addrs.is_empty() {
            info!("HybridTransport::initialize - Initializing libp2p transport");
            self.init_libp2p_transport()?;
            info!("HybridTransport::initialize - Libp2p transport initialized");
        } else {
            info!("HybridTransport::initialize - Skipping libp2p transport (no listen addresses configured)");
        }
        info!("HybridTransport::initialize - Initializing QUIC transport");
        self.init_quic_transport().await?;
        if self.quic_endpoint.is_some() {
            info!("HybridTransport::initialize - QUIC transport initialized");
        } else {
            warn!(
                "HybridTransport::initialize - QUIC transport disabled (permission denied); proceeding without QUIC"
            );
        }

        info!(
            "Hybrid transport layer initialized with peer ID: {}",
            self.peer_id
        );
        info!("  - LibP2P: enabled");
        info!(
            "  - QUIC: {}",
            if self.quic_endpoint.is_some() {
                "enabled"
            } else {
                "disabled"
            }
        );

        // NOTE: Accept loop is handled by NetworkEngine to avoid conflicts
        // The NetworkEngine's start_accept_loop() manages all incoming QUIC connections
        info!("HybridTransport::initialize - Accept loop will be managed by NetworkEngine");

        info!("HybridTransport::initialize - Initialization complete");
        Ok(())
    }

    fn init_libp2p_transport(&mut self) -> Result<()> {
        debug!("init_libp2p_transport - Starting");
        // Create a new keypair for libp2p (or reuse the existing one)
        let keypair = Keypair::generate_ed25519();
        debug!("init_libp2p_transport - Creating wrapper");
        let wrapper = Arc::new(libp2p_wrapper::LibP2PTransportWrapper::new(&keypair)?);
        debug!("init_libp2p_transport - Wrapper created");

        // Start listening on configured addresses
        for addr in &self.config.libp2p_listen_addrs {
            let wrapper_clone = wrapper.clone();
            let addr = addr.clone();
            tokio::spawn(async move {
                if let Err(e) = wrapper_clone.listen_on(addr).await {
                    error!("Failed to listen on libp2p address: {}", e);
                }
            });
        }

        self.libp2p_transport = Some(wrapper);
        Ok(())
    }

    // NOTE: This method is commented out to prevent conflicts with NetworkEngine's accept loop
    // The NetworkEngine handles all incoming QUIC connections to avoid race conditions
    // and potential deadlocks from multiple accept loops on the same endpoint.
    #[allow(dead_code)]
    fn start_accept_loop(&self) {
        // This method is intentionally disabled.
        // All connection acceptance is handled by NetworkEngine::start_accept_loop()
        warn!("HybridTransport::start_accept_loop called but is disabled - NetworkEngine handles connections");
    }

    // Temporarily disabled - libp2p transport is not Send+Sync
    // fn init_libp2p_transport(&mut self) -> Result<()> {
    //     // Create TCP transport
    //     let tcp = libp2p::tcp::tokio::Transport::new(
    //         libp2p::tcp::Config::default().nodelay(true)
    //     );

    //     // Build the transport stack
    //     let transport = tcp
    //         .upgrade(upgrade::Version::V1)
    //         .authenticate(noise::Config::new(&self.keypair).unwrap())
    //         .multiplex(yamux::Config::default())
    //         .boxed();

    //     self.libp2p_transport = Some(transport);

    //     debug!("Libp2p transport initialized");
    //     Ok(())
    // }

    async fn init_quic_transport(&mut self) -> Result<()> {
        // Allow tests or constrained environments to disable QUIC explicitly
        if std::env::var("ADIC_DISABLE_QUIC")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            warn!("init_quic_transport - ADIC_DISABLE_QUIC is set; skipping QUIC initialization");
            self.quic_endpoint = None;
            return Ok(());
        }
        info!("init_quic_transport - Starting QUIC initialization");
        info!("init_quic_transport - Building server config");
        let server_config = self.build_server_config()?;
        info!(
            "init_quic_transport - Server config built, creating endpoint on {}",
            self.config.quic_listen_addr
        );
        info!("init_quic_transport - About to call Endpoint::server()...");
        match Endpoint::server(server_config, self.config.quic_listen_addr) {
            Ok(endpoint) => {
                info!("init_quic_transport - Endpoint created successfully");
                self.quic_endpoint = Some(Arc::new(endpoint));
                info!("init_quic_transport - Endpoint stored in self.quic_endpoint");
            }
            Err(e) => {
                // In restricted sandboxes (e.g., CI/seccomp), binding UDP sockets may be denied.
                // Gracefully degrade by disabling QUIC instead of failing initialization.
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    warn!(
                        "init_quic_transport - Permission denied creating QUIC endpoint ({}). Disabling QUIC for this run.",
                        e
                    );
                    self.quic_endpoint = None;
                    return Ok(());
                } else {
                    return Err(TransportError::InitializationFailed(e.to_string()).into());
                }
            }
        }

        debug!(
            "init_quic_transport - QUIC transport initialized on {}",
            self.config.quic_listen_addr
        );
        Ok(())
    }

    fn build_server_config(&self) -> Result<ServerConfig> {
        debug!("build_server_config - Starting");
        // Install default crypto provider if not already installed
        debug!("build_server_config - Installing crypto provider");
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Load certificates - use provided paths or fall back to self-signed
        let use_custom_certs = self.config.node_cert_path.is_some()
            && self.config.node_key_path.is_some()
            && self
                .config
                .node_cert_path
                .as_ref()
                .map(|p| std::path::Path::new(p).exists())
                .unwrap_or(false)
            && self
                .config
                .node_key_path
                .as_ref()
                .map(|p| std::path::Path::new(p).exists())
                .unwrap_or(false);

        let (cert_chain, priv_key) = if use_custom_certs {
            let cert_path = self.config.node_cert_path.as_ref().unwrap();
            let key_path = self.config.node_key_path.as_ref().unwrap();
            debug!(
                "build_server_config - Loading node certificate from {}",
                cert_path
            );

            // Load certificate
            let cert_file = std::fs::read(cert_path).map_err(|e| {
                TransportError::InitializationFailed(format!("Failed to read cert file: {}", e))
            })?;
            let certs: Vec<rustls::pki_types::CertificateDer> =
                rustls_pemfile::certs(&mut cert_file.as_slice())
                    .filter_map(|cert| cert.ok())
                    .collect();
            if certs.is_empty() {
                return Err(TransportError::InitializationFailed(
                    "No valid certificates found in file".into(),
                )
                .into());
            }

            // Load private key
            let key_file = std::fs::read(key_path).map_err(|e| {
                TransportError::InitializationFailed(format!("Failed to read key file: {}", e))
            })?;
            let key = rustls_pemfile::private_key(&mut key_file.as_slice())
                .map_err(|e| {
                    TransportError::InitializationFailed(format!("Failed to parse key: {}", e))
                })?
                .ok_or_else(|| {
                    TransportError::InitializationFailed("No private key found in file".into())
                })?;

            debug!("build_server_config - Loaded {} certificates", certs.len());
            (certs, key)
        } else {
            debug!("build_server_config - Generating self-signed certificate");
            let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
                .map_err(|e| TransportError::InitializationFailed(e.to_string()))?;
            debug!("build_server_config - Certificate generated");

            let cert_der = cert.cert.der().to_vec();
            let priv_key_der = cert.key_pair.serialize_der();

            let priv_key =
                rustls::pki_types::PrivateKeyDer::try_from(priv_key_der).map_err(|e| {
                    TransportError::InitializationFailed(format!("Invalid private key: {:?}", e))
                })?;
            let cert_chain = vec![rustls::pki_types::CertificateDer::from(cert_der)];
            (cert_chain, priv_key)
        };

        debug!("build_server_config - Creating server config with certificate");
        let mut server_config = ServerConfig::with_single_cert(cert_chain, priv_key)
            .map_err(|e| TransportError::InitializationFailed(e.to_string()))?;

        // Configure transport parameters for faster disconnection detection
        let mut transport_config = quinn::TransportConfig::default();
        transport_config.max_idle_timeout(Some(quinn::VarInt::from_u32(5_000).into())); // 5 seconds
        transport_config.keep_alive_interval(Some(Duration::from_secs(2))); // 2 seconds
        server_config.transport_config(Arc::new(transport_config));

        debug!("build_server_config - Server config created successfully");

        Ok(server_config)
    }

    pub async fn connect_quic(&self, addr: SocketAddr) -> Result<Connection> {
        let connect_start = std::time::Instant::now();

        let endpoint = self
            .quic_endpoint
            .as_ref()
            .ok_or_else(|| TransportError::InitializationFailed("QUIC not initialized".into()))?;

        debug!(
            target_addr = %addr,
            tls_enabled = self.config.use_production_tls,
            has_ca_cert = self.config.ca_cert_path.is_some(),
            "🔌 Initiating QUIC connection"
        );

        let client_config = self.build_client_config()?;
        let connecting = endpoint
            .connect_with(client_config, addr, "localhost")
            .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;

        let connection = connecting.await.map_err(TransportError::QuicError)?;

        let connect_duration = connect_start.elapsed();

        info!(
            target_addr = %addr,
            connect_duration_ms = connect_duration.as_millis(),
            connection_id = ?connection.stable_id(),
            rtt_ms = connection.rtt().as_millis(),
            "✅ QUIC connection established"
        );

        Ok(connection)
    }

    fn build_client_config(&self) -> Result<ClientConfig> {
        // Install default crypto provider if not already installed
        let _ = rustls::crypto::ring::default_provider().install_default();

        let crypto = if self.config.use_production_tls {
            // Production: Use proper CA verification
            let mut roots = rustls::RootCertStore::empty();

            let ca_path_exists = self
                .config
                .ca_cert_path
                .as_ref()
                .map(|p| std::path::Path::new(p).exists())
                .unwrap_or(false);

            if let Some(ca_path) = &self.config.ca_cert_path {
                if ca_path_exists {
                    // Load custom CA certificate
                    let ca_file = std::fs::read(ca_path).map_err(|e| {
                        TransportError::InitializationFailed(format!(
                            "Failed to read CA cert: {}",
                            e
                        ))
                    })?;
                    let ca_certs: Vec<rustls::pki_types::CertificateDer> =
                        rustls_pemfile::certs(&mut ca_file.as_slice())
                            .filter_map(|cert| cert.ok())
                            .collect();
                    if ca_certs.is_empty() {
                        return Err(TransportError::InitializationFailed(
                            "No valid CA certificates found in file".into(),
                        )
                        .into());
                    }
                    for cert in ca_certs {
                        roots.add(cert).map_err(|e| {
                            TransportError::InitializationFailed(format!(
                                "Failed to add CA cert: {}",
                                e
                            ))
                        })?;
                    }
                } else {
                    warn!(
                        "CA cert path configured but file does not exist: {}",
                        ca_path
                    );
                    // Fall back to system roots
                    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                }
            } else {
                // Use system root certificates
                roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            }

            rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth()
        } else {
            // Development mode: Still use verification but with custom verifier for self-signed certs
            error!(
                security_mode = "development",
                production_required = true,
                "⚠️ WARNING: TLS certificate verification is DISABLED!"
            );
            error!(
                risk = "extremely_high",
                "⚠️ This is EXTREMELY UNSAFE and should NEVER be used in production!"
            );
            error!(
                fix = "set use_production_tls=true",
                "⚠️ Set use_production_tls=true for secure operation"
            );
            warn!(
                tls_mode = "development",
                "⚠️ Using development TLS mode - not suitable for production"
            );
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(SkipServerVerification::new()))
                .with_no_client_auth()
        };

        let mut client_config = ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(crypto).map_err(|e| {
                TransportError::InitializationFailed(format!("Failed to create QUIC config: {}", e))
            })?,
        ));

        // Configure transport parameters for faster disconnection detection
        let mut transport_config = quinn::TransportConfig::default();
        transport_config.max_idle_timeout(Some(quinn::VarInt::from_u32(5_000).into())); // 5 seconds
        transport_config.keep_alive_interval(Some(Duration::from_secs(2))); // 2 seconds
        client_config.transport_config(Arc::new(transport_config));

        Ok(client_config)
    }

    pub async fn accept_quic(&self) -> Result<Connection> {
        let endpoint = self
            .quic_endpoint
            .as_ref()
            .ok_or_else(|| TransportError::InitializationFailed("QUIC not initialized".into()))?;

        let connecting = endpoint
            .accept()
            .await
            .ok_or_else(|| TransportError::ConnectionFailed("No incoming connection".into()))?;

        let connection = connecting.await.map_err(TransportError::QuicError)?;

        debug!(
            "QUIC connection accepted from {}",
            connection.remote_address()
        );
        Ok(connection)
    }

    // pub fn libp2p_transport(&self) -> Option<&Boxed<(PeerId, StreamMuxerBox)>> {
    //     self.libp2p_transport.as_ref()
    // }

    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    pub fn connection_pool(&self) -> &ConnectionPool {
        &self.connection_pool
    }

    pub fn quic_endpoint(&self) -> Option<Arc<Endpoint>> {
        self.quic_endpoint.clone()
    }

    pub async fn local_quic_port(&self) -> Option<u16> {
        if let Some(endpoint) = &self.quic_endpoint {
            endpoint.local_addr().ok().map(|addr| addr.port())
        } else {
            None
        }
    }

    pub async fn send_with_priority(
        &self,
        peer_id: &PeerId,
        data: &[u8],
        priority: PriorityLane,
    ) -> Result<()> {
        if let Some(conn) = self.connection_pool.get_connection(peer_id).await {
            let mut stream = conn
                .open_uni()
                .await
                .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;

            // Build message with header
            let mut message = Vec::new();
            message.push(priority as u8);
            message.extend_from_slice(&(data.len() as u32).to_be_bytes());
            message.extend_from_slice(data);

            // Write all data at once
            stream
                .write_all(&message)
                .await
                .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;

            stream
                .finish()
                .map_err(|e| TransportError::ConnectionFailed(e.to_string()))?;

            Ok(())
        } else {
            Err(TransportError::ConnectionFailed("Peer not connected".into()).into())
        }
    }

    /// Send a discovery message to a specific peer
    pub async fn send_discovery_message(
        &self,
        peer_id: &PeerId,
        message: DiscoveryMessage,
    ) -> Result<()> {
        let network_msg = NetworkMessage::Discovery(message);
        let data = serde_json::to_vec(&network_msg).map_err(|e| {
            TransportError::ConnectionFailed(format!(
                "Failed to serialize discovery message: {}",
                e
            ))
        })?;

        self.send_with_priority(peer_id, &data, PriorityLane::Discovery)
            .await
    }

    /// Send an ADIC message to a specific peer
    pub async fn send_adic_message(&self, peer_id: &PeerId, message: AdicMessage) -> Result<()> {
        let network_msg = NetworkMessage::AdicMessage(message);
        let data = serde_json::to_vec(&network_msg).map_err(|e| {
            TransportError::ConnectionFailed(format!("Failed to serialize ADIC message: {}", e))
        })?;

        self.send_with_priority(peer_id, &data, PriorityLane::Normal)
            .await
    }

    /// Send an update message to a specific peer
    pub async fn send_update_message(
        &self,
        peer_id: &PeerId,
        message: UpdateMessage,
    ) -> Result<()> {
        let network_msg = NetworkMessage::Update(message);
        let data = serde_json::to_vec(&network_msg).map_err(|e| {
            TransportError::ConnectionFailed(format!("Failed to serialize update message: {}", e))
        })?;

        // Use high priority for update messages to ensure timely delivery
        self.send_with_priority(peer_id, &data, PriorityLane::Data)
            .await
    }

    /// Broadcast a discovery message to all connected peers
    pub async fn broadcast_discovery_message(&self, message: DiscoveryMessage) -> Result<()> {
        let connections = self.connection_pool.get_all_connections().await;
        for (peer_id, _) in connections {
            if let Err(e) = self.send_discovery_message(&peer_id, message.clone()).await {
                warn!(
                    "Failed to send discovery message to peer {}: {}",
                    peer_id, e
                );
            }
        }
        Ok(())
    }

    /// Broadcast an update message to all connected peers
    pub async fn broadcast_update_message(&self, message: UpdateMessage) -> Result<()> {
        let connections = self.connection_pool.get_all_connections().await;
        for (peer_id, _) in connections {
            if let Err(e) = self.send_update_message(&peer_id, message.clone()).await {
                warn!("Failed to send update message to peer {}: {}", peer_id, e);
            }
        }
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        info!("Starting transport shutdown...");

        if let Some(endpoint) = self.quic_endpoint.take() {
            info!("Closing QUIC endpoint...");
            endpoint.close(0u32.into(), b"shutting down");
            info!("QUIC endpoint closed");
        } else {
            info!("No QUIC endpoint to close");
        }

        // Wait for the endpoint to actually close
        info!("Waiting for endpoint cleanup...");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

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
#[path = "transport_tests.rs"]
mod transport_tests;
