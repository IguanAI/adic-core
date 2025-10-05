//! OpenAPI/Swagger documentation configuration for ADIC-DAG API

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "ADIC-DAG Node API",
        version = "0.2.0",
        description = "REST API for interacting with ADIC-DAG nodes. \n\n\
                      ADIC-DAG is a feeless, reputation-weighted distributed ledger that organizes \
                      messages as a higher-dimensional directed acyclic hypergraph with ultrametric \
                      security guarantees.",
        contact(
            name = "ADIC Core Team",
            url = "https://github.com/IguanAI/adic-core"
        ),
        license(
            name = "MIT",
            url = "https://opensource.org/licenses/MIT"
        )
    ),
    tags(
        (name = "Core", description = "Core node operations and health checks"),
        (name = "Messages", description = "Message submission and retrieval"),
        (name = "Network", description = "Network and peer information"),
        (name = "Finality", description = "Finality status and artifacts"),
        (name = "Economics", description = "Token balances and transactions"),
        (name = "Wallet", description = "Wallet management"),
        (name = "Streaming", description = "WebSocket and SSE event streaming"),
    ),
    components(
        schemas(
            SubmitMessageRequest,
            SubmitMessageResponse,
            SubmitTransfer,
            SubmitFeatures,
            AxisValue,
            MessageResponse,
            StatusResponse,
            NetworkStatusResponse,
            PeerInfoResponse,
            FinalityInfo,
            ErrorResponse,
        )
    ),
    servers(
        (url = "http://localhost:8080", description = "Local development server"),
        (url = "http://localhost:8081", description = "Local development server (node 2)"),
    )
)]
pub struct ApiDoc;

// Re-export types for OpenAPI schemas
use utoipa::ToSchema;

#[derive(serde::Serialize, serde::Deserialize, ToSchema)]
pub struct SubmitMessageRequest {
    /// Message content (arbitrary string)
    #[schema(example = "Hello ADIC-DAG!")]
    pub content: String,

    /// Optional feature vector for p-adic ball placement
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<SubmitFeatures>,

    /// Optional explicit parent message IDs (hex-encoded)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!(["abc123...", "def456..."]))]
    pub parents: Option<Vec<String>>,

    /// Optional signature (hex-encoded)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,

    /// Optional proposer public key (hex-encoded)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposer_pk: Option<String>,

    /// Optional value transfer
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer: Option<SubmitTransfer>,
}

#[derive(serde::Serialize, serde::Deserialize, ToSchema)]
pub struct SubmitMessageResponse {
    /// Unique message ID (hex-encoded)
    #[schema(example = "a1b2c3d4e5f6...")]
    pub message_id: String,

    /// Amount of ADIC escrowed as deposit
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 0.1)]
    pub deposit_escrowed: Option<f64>,
}

#[derive(serde::Serialize, serde::Deserialize, ToSchema)]
pub struct SubmitTransfer {
    /// Sender address (hex-encoded, 32 bytes)
    pub from: String,

    /// Recipient address (hex-encoded, 32 bytes)
    pub to: String,

    /// Amount in base units
    #[schema(example = 1000000)]
    pub amount: u64,

    /// Nonce for replay protection
    #[schema(example = 1)]
    pub nonce: u64,
}

#[derive(serde::Serialize, serde::Deserialize, ToSchema)]
pub struct SubmitFeatures {
    /// Axis values for p-adic ball placement
    pub axes: Vec<AxisValue>,
}

#[derive(serde::Serialize, serde::Deserialize, ToSchema)]
pub struct AxisValue {
    /// Axis index (0 to d-1)
    #[schema(example = 0)]
    pub axis: u32,

    /// P-adic value on this axis
    #[schema(example = 42)]
    pub value: u64,
}

#[derive(serde::Serialize, serde::Deserialize, ToSchema)]
pub struct MessageResponse {
    /// Message ID
    pub id: String,

    /// Message content
    pub content: String,

    /// Timestamp
    pub timestamp: i64,

    /// Proposer public key
    pub proposer: String,

    /// Parent message IDs
    pub parents: Vec<String>,

    /// Finality information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finality: Option<FinalityInfo>,
}

#[derive(serde::Serialize, serde::Deserialize, ToSchema)]
pub struct StatusResponse {
    /// Node version
    #[schema(example = "0.2.0")]
    pub version: String,

    /// Node ID
    pub node_id: String,

    /// Total messages in DAG
    #[schema(example = 1234)]
    pub message_count: usize,

    /// Number of peers
    #[schema(example = 8)]
    pub peer_count: usize,

    /// Network name
    #[schema(example = "devnet")]
    pub network: String,

    /// Chain ID
    #[schema(example = "adic-devnet")]
    pub chain_id: String,
}

#[derive(serde::Serialize, serde::Deserialize, ToSchema)]
pub struct NetworkStatusResponse {
    /// Total peer count
    pub peer_count: usize,

    /// Connected peers
    pub connected_peers: usize,

    /// Messages sent
    pub messages_sent: u64,

    /// Messages received
    pub messages_received: u64,

    /// Bytes sent
    pub bytes_sent: u64,

    /// Bytes received
    pub bytes_received: u64,
}

#[derive(serde::Serialize, serde::Deserialize, ToSchema)]
pub struct PeerInfoResponse {
    /// Peer ID
    pub peer_id: String,

    /// Network addresses
    pub addresses: Vec<String>,

    /// Connection state
    #[schema(example = "connected")]
    pub connection_state: String,

    /// Reputation score
    #[schema(example = 0.95)]
    pub reputation_score: f64,

    /// Latency in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,

    /// Protocol version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, ToSchema)]
pub struct FinalityInfo {
    /// F1 (k-core) finalized
    pub f1_finalized: bool,

    /// F2 (persistent homology) stabilized
    pub f2_stabilized: bool,

    /// K-core size
    #[serde(skip_serializing_if = "Option::is_none")]
    pub k_core: Option<usize>,

    /// Depth in DAG
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<usize>,
}

#[derive(serde::Serialize, serde::Deserialize, ToSchema)]
pub struct ErrorResponse {
    /// Error message
    #[schema(example = "Message not found")]
    pub error: String,
}
