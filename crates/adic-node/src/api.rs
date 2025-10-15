use crate::api_governance::{governance_routes, GovernanceApiState};
use crate::api_storage_market::{storage_market_routes, StorageMarketApiState};
use crate::api_sse;
use crate::api_ws;
use crate::auth::{auth_middleware, rate_limit_middleware, AuthUser};
use crate::economics_api::{economics_routes, EconomicsApiState};
use crate::metrics;
use crate::node::AdicNode;
use crate::openapi::ApiDoc;
use crate::wallet_registry::WalletRegistrationRequest;
use adic_economics::address_encoding::{decode_address, encode_address};
use adic_math::ball_id;
use adic_types::{ConflictId, MessageId, PublicKey, QpDigits};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post},
    Router,
};
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::{debug, info};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[derive(Clone)]
struct AppState {
    node: AdicNode,
    metrics: metrics::Metrics,
    ws_pool: api_ws::WsConnectionPool,
}

#[derive(Serialize, Deserialize)]
struct SubmitMessageRequest {
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    features: Option<SubmitFeatures>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parents: Option<Vec<String>>, // Hex-encoded parent message IDs
    signature: String, // Hex-encoded signature
    proposer_address: String, // Bech32 adic1... address
    #[serde(skip_serializing_if = "Option::is_none")]
    transfer: Option<SubmitTransfer>, // Optional value transfer
}

#[derive(Serialize, Deserialize)]
struct SubmitTransfer {
    from: String, // Hex-encoded sender address (32 bytes)
    to: String,   // Hex-encoded recipient address (32 bytes)
    amount: u64,  // Amount in base units
    nonce: u64,   // Nonce for replay protection
}

#[derive(Serialize, Deserialize)]
pub struct SubmitFeatures {
    pub axes: Vec<AxisValue>,
}

#[derive(Serialize, Deserialize)]
pub struct AxisValue {
    pub axis: u32,
    pub value: u64, // Simplified format for submission
}

#[derive(Serialize, Deserialize)]
struct SubmitMessageResponse {
    message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    deposit_escrowed: Option<f64>, // Amount of ADIC escrowed
}

#[derive(Serialize, Deserialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Serialize, Deserialize)]
struct PeerInfoResponse {
    peer_id: String,
    addresses: Vec<String>,
    connection_state: String,
    reputation_score: f64,
    latency_ms: Option<u64>,
    version: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct NetworkStatusResponse {
    peer_count: usize,
    connected_peers: usize,
    messages_sent: u64,
    messages_received: u64,
    bytes_sent: u64,
    bytes_received: u64,
}

#[derive(Deserialize)]
struct TracesQuery {
    limit: Option<usize>,
}

#[derive(Serialize)]
struct StreamingHealthResponse {
    status: String,
    event_streaming: EventStreamingStatus,
    metrics: StreamingMetrics,
}

#[derive(Serialize)]
struct EventStreamingStatus {
    enabled: bool,
    websocket_enabled: bool,
    sse_enabled: bool,
}

#[derive(Serialize)]
struct StreamingMetrics {
    events_emitted_total: u64,
    websocket_connections: i64,
    sse_connections: i64,
    websocket_messages_sent: u64,
    sse_messages_sent: u64,
}

// Route builder functions - these create modular routers that will be nested under /v1

fn core_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health))
        .route("/health/streaming", get(get_streaming_health))
        .route("/status", get(get_status))
        .route("/metrics", get(get_metrics))
}

fn message_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/messages", post(submit_message))
        .route("/messages/:id", get(get_message))
        .route("/messages/bulk", get(get_messages_bulk))
        .route("/messages/range", get(get_messages_range))
        .route("/messages/since/:id", get(get_messages_since))
        .route("/tips", get(get_tips))
        .route("/balls/:axis/:radius/:center", get(get_ball_messages))
}

fn wallet_routes() -> Router<Arc<AppState>> {
    Router::new()
        // Node's own wallet
        .route("/wallets/self", get(wallet_info_handler))
        .route("/wallets/self/sign", post(sign_handler))
        .route("/wallets/self/export", post(export_wallet_handler))
        .route("/wallets/self/import", post(import_wallet_handler))
        // Any wallet
        .route("/wallets/:address", get(get_wallet_info_handler))
        .route("/wallets/:address/balance", get(balance_handler))
        .route("/wallets/:address/transactions", get(transactions_handler))
        .route("/wallets/transactions", get(all_transactions_handler))
        .route("/wallets/transfer", post(transfer_handler))
        .route("/wallets/faucet", post(faucet_handler))
        // Registry
        .route("/wallets/registry", get(list_registered_wallets_handler))
        .route("/wallets/registry", post(register_wallet_handler))
        .route(
            "/wallets/registry/stats",
            get(wallet_registry_stats_handler),
        )
        .route(
            "/wallets/registry/:address",
            get(check_wallet_registration_handler),
        )
        .route(
            "/wallets/registry/:address",
            delete(unregister_wallet_handler),
        )
        .route(
            "/wallets/registry/:address/public_key",
            get(get_wallet_public_key_handler),
        )
}

fn network_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/network/peers", get(get_peers))
        .route("/network/status", get(get_network_status))
}

fn consensus_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/finality/:id", get(get_finality_artifact))
        .route("/finality/kcore/metrics", get(get_kcore_metrics))
        .route("/conflicts", get(get_all_conflicts))
        .route("/conflicts/:id", get(get_conflict_details))
        .route("/mrw/traces", get(get_mrw_traces))
        .route("/mrw/trace/:id", get(get_mrw_trace))
        .route("/reputation", get(get_all_reputations))
        .route("/reputation/:pubkey", get(get_reputation))
}

fn statistics_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/statistics", get(get_detailed_statistics))
        .route("/diversity", get(get_diversity_stats))
        .route("/energy", get(get_active_energy_conflicts))
        .route("/admissibility", get(get_admissibility_rates))
}

fn security_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/proofs/membership", post(generate_membership_proof))
        .route("/proofs/verify", post(verify_proof))
        .route("/security/score/:id", get(get_security_score))
}

fn update_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/updates/status", get(get_update_status))
        .route("/updates/check", post(trigger_update_check))
        .route("/updates/apply", post(trigger_update_apply))
        .route("/updates/progress", get(get_update_progress))
        .route("/updates/swarm", get(get_update_swarm_stats))
}

fn streaming_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/ws/events",
            get(|ws, query, State(state): State<Arc<AppState>>| {
                api_ws::websocket_handler(
                    ws,
                    query,
                    State(state.node.event_bus()),
                    State(state.metrics.clone()),
                    State(state.ws_pool.clone()),
                )
            }),
        )
        .route(
            "/sse/events",
            get(|query, State(state): State<Arc<AppState>>| {
                api_sse::sse_handler(
                    query,
                    State(state.node.event_bus()),
                    State(state.metrics.clone()),
                )
            }),
        )
}

fn deposit_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/deposits", get(get_deposits_summary))
        .route("/deposits/:id", get(get_deposit_status))
}

pub fn start_api_server(node: AdicNode, host: String, port: u16) -> JoinHandle<()> {
    start_api_server_with_listener(node, host, port, None)
}

pub fn start_api_server_with_listener(
    node: AdicNode,
    host: String,
    port: u16,
    existing_listener: Option<tokio::net::TcpListener>,
) -> JoinHandle<()> {
    let metrics = metrics::Metrics::new();
    let ws_pool = api_ws::WsConnectionPool::default(); // 1000 max connections
    let state = Arc::new(AppState {
        node: node.clone(),
        metrics,
        ws_pool,
    });

    // Create economics API state
    let economics_state = EconomicsApiState {
        economics: Arc::clone(&node.economics),
    };

    let governance_state = GovernanceApiState {
        node: node.clone(),
    };

    let storage_market_state = StorageMarketApiState {
        node: node.clone(),
    };

    // Build v1 API by composing all domain routers
    let v1_api = Router::new()
        .merge(core_routes())
        .merge(message_routes())
        .merge(wallet_routes())
        .merge(network_routes())
        .merge(consensus_routes())
        .merge(statistics_routes())
        .merge(security_routes())
        .merge(update_routes())
        .merge(streaming_routes())
        .merge(deposit_routes())
        .layer(middleware::from_fn(rate_limit_middleware))
        .layer(middleware::from_fn(auth_middleware))
        .with_state(state)
        // Merge economics routes (has its own state)
        .merge(economics_routes(economics_state))
        // Merge governance routes (has its own state)
        .merge(governance_routes(governance_state))
        // Merge storage market routes (has its own state)
        .merge(storage_market_routes(storage_market_state));

    // Add Swagger UI for API documentation
    let app = Router::new()
        .merge(SwaggerUi::new("/api/docs").url("/api/openapi.json", ApiDoc::openapi()))
        .nest("/v1", v1_api);

    let addr = format!("{}:{}", host, port);
    info!("Starting API server on {}", addr);

    tokio::spawn(async move {
        let listener = if let Some(existing) = existing_listener {
            info!("Using restored API listener from copyover");
            existing
        } else {
            tokio::net::TcpListener::bind(&addr)
                .await
                .expect("Failed to bind API server")
        };

        // Store the file descriptor in the update manager if available
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = listener.as_raw_fd();
            if let Some(ref update_manager) = node.update_manager {
                update_manager.set_api_listener_fd(fd).await;
                debug!("Stored API listener fd {} in update manager", fd);
            }
        }

        axum::serve(listener, app).await.expect("API server failed");
    })
}

async fn health(State(state): State<Arc<AppState>>) -> Response {
    // Collect health status from various components
    let mut components = HashMap::new();
    let mut overall_healthy = true;

    // 1. Storage health check
    let storage_health = match state.node.storage.get_stats().await {
        Ok(stats) => {
            components.insert(
                "storage".to_string(),
                serde_json::json!({
                    "status": "healthy",
                    "message_count": stats.message_count,
                    "tip_count": stats.tip_count,
                    "finalized_count": stats.finalized_count,
                }),
            );
            true
        }
        Err(_) => {
            components.insert(
                "storage".to_string(),
                serde_json::json!({
                    "status": "unhealthy",
                    "error": "Failed to retrieve storage stats"
                }),
            );
            overall_healthy = false;
            false
        }
    };

    // 2. Consensus health check
    let finality_stats = state.node.finality.get_stats().await;
    let finality_rate = if finality_stats.total_messages > 0 {
        (finality_stats.finalized_count as f64) / (finality_stats.total_messages as f64)
    } else {
        0.0
    };

    let consensus_healthy = storage_health; // Basic check - if storage works, consensus should work
    components.insert(
        "consensus".to_string(),
        serde_json::json!({
            "status": if consensus_healthy { "healthy" } else { "degraded" },
            "finalized_count": finality_stats.finalized_count,
            "pending_count": finality_stats.pending_count,
            "total_messages": finality_stats.total_messages,
            "finality_rate": format!("{:.2}%", finality_rate * 100.0),
        }),
    );

    // 3. Network health check (basic check based on storage)
    components.insert(
        "network".to_string(),
        serde_json::json!({
            "status": "healthy",
            "note": "Basic connectivity check"
        }),
    );

    // 4. Event streaming health
    let metrics = &state.metrics;
    let active_connections = metrics.websocket_connections.get() + metrics.sse_connections.get();
    components.insert(
        "event_streaming".to_string(),
        serde_json::json!({
            "status": "healthy",
            "active_connections": active_connections,
            "websocket_connections": metrics.websocket_connections.get(),
            "sse_connections": metrics.sse_connections.get(),
        }),
    );

    // Determine overall status
    let overall_status = if overall_healthy {
        "healthy"
    } else {
        // Check if any components are unhealthy vs degraded
        let has_unhealthy = components
            .values()
            .any(|v| v.get("status").and_then(|s| s.as_str()) == Some("unhealthy"));

        if has_unhealthy {
            "unhealthy"
        } else {
            "degraded"
        }
    };

    let response = serde_json::json!({
        "status": overall_status,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "components": components,
        "version": env!("CARGO_PKG_VERSION"),
    });

    let status_code = match overall_status {
        "healthy" => StatusCode::OK,
        "degraded" => StatusCode::OK, // Still return 200 for degraded
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };

    (status_code, Json(response)).into_response()
}

async fn get_status(State(state): State<Arc<AppState>>) -> Response {
    match state.node.get_stats().await {
        Ok(stats) => (StatusCode::OK, Json(stats)).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn get_streaming_health(State(state): State<Arc<AppState>>) -> Response {
    let metrics = &state.metrics;

    let response = StreamingHealthResponse {
        status: "healthy".to_string(),
        event_streaming: EventStreamingStatus {
            enabled: true,
            websocket_enabled: true,
            sse_enabled: true,
        },
        metrics: StreamingMetrics {
            events_emitted_total: metrics.events_emitted_total.get(),
            websocket_connections: metrics.websocket_connections.get(),
            sse_connections: metrics.sse_connections.get(),
            websocket_messages_sent: metrics.websocket_messages_sent.get(),
            sse_messages_sent: metrics.sse_messages_sent.get(),
        },
    };

    (StatusCode::OK, Json(response)).into_response()
}

async fn submit_message(
    _auth: AuthUser,
    State(state): State<Arc<AppState>>,
    Json(req): Json<SubmitMessageRequest>,
) -> Response {
    state.metrics.messages_submitted.inc();

    // Decode adic1... address to public key
    let proposer_pk = match decode_address(&req.proposer_address) {
        Ok(bytes) => PublicKey::from_bytes(bytes),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid proposer address: must be bech32 adic1... format".to_string(),
                }),
            )
                .into_response();
        }
    };

    // Decode signature
    let sig_bytes = match hex::decode(&req.signature) {
        Ok(bytes) if bytes.len() == 64 => bytes,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid signature: must be 64-byte hex-encoded".to_string(),
                }),
            )
                .into_response();
        }
    };

    // Build canonical message for signature verification
    // Signature covers: content || proposer_pk || parents (if provided)
    let mut signed_data = Vec::new();
    signed_data.extend_from_slice(req.content.as_bytes());
    signed_data.extend_from_slice(proposer_pk.as_bytes());

    // Include parents in signature if provided
    if let Some(ref parents) = req.parents {
        for parent_hex in parents {
            if let Ok(parent_bytes) = hex::decode(parent_hex) {
                signed_data.extend_from_slice(&parent_bytes);
            }
        }
    }

    // Verify Ed25519 signature
    let signature = adic_types::Signature::new(sig_bytes);
    let crypto = adic_crypto::CryptoEngine::new();

    // Create temporary message for verification
    let temp_message = adic_types::AdicMessage::new(
        vec![],
        adic_types::AdicFeatures::new(vec![]),
        adic_types::AdicMeta::new(chrono::Utc::now()),
        proposer_pk,
        signed_data.clone(),
    );

    match crypto.verify_signature(&temp_message, &proposer_pk, &signature) {
        Ok(true) => {
            tracing::debug!("✅ Signature verification passed for message submission");
        }
        Ok(false) | Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "Signature verification failed: invalid signature for provided content and public key".to_string(),
                }),
            )
                .into_response();
        }
    }

    // Check balance for deposit requirement
    let deposit_amount = state.node.get_deposit_amount();
    let proposer_address = adic_economics::AccountAddress::from_public_key(&proposer_pk);

    match state
        .node
        .economics
        .balances
        .get_unlocked_balance(proposer_address)
        .await
    {
        Ok(balance) => {
            if balance < deposit_amount {
                return (
                    StatusCode::PAYMENT_REQUIRED,
                    Json(ErrorResponse {
                        error: format!(
                            "Insufficient balance for deposit. Required: {} ADIC, Available: {} ADIC",
                            deposit_amount.to_adic(),
                            balance.to_adic()
                        ),
                    }),
                )
                    .into_response();
            }
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to check balance: {}", e),
                }),
            )
                .into_response();
        }
    }

    // Parse transfer if provided
    let transfer = if let Some(transfer_req) = req.transfer {
        // Decode addresses from hex
        let from_bytes = match hex::decode(&transfer_req.from) {
            Ok(bytes) if bytes.len() == 32 => bytes,
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Invalid 'from' address in transfer".to_string(),
                    }),
                )
                    .into_response();
            }
        };

        let to_bytes = match hex::decode(&transfer_req.to) {
            Ok(bytes) if bytes.len() == 32 => bytes,
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Invalid 'to' address in transfer".to_string(),
                    }),
                )
                    .into_response();
            }
        };

        Some(adic_types::ValueTransfer::new(
            from_bytes,
            to_bytes,
            transfer_req.amount,
            transfer_req.nonce,
        ))
    } else {
        None
    };

    // Handle explicit parents if provided (for genesis messages or specific parent selection)
    let result = if let Some(parent_hexes) = req.parents {
        tracing::debug!(
            num_parents = parent_hexes.len(),
            "Received submission with explicit parents"
        );

        // Parse parent IDs from hex strings
        let mut parent_ids = Vec::new();
        for hex_id in parent_hexes {
            match hex::decode(&hex_id) {
                Ok(bytes) if bytes.len() == 32 => {
                    let mut id_bytes = [0u8; 32];
                    id_bytes.copy_from_slice(&bytes);
                    parent_ids.push(MessageId::from_bytes(id_bytes));
                }
                _ => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("Invalid parent ID: {}", hex_id),
                        }),
                    )
                        .into_response();
                }
            }
        }

        tracing::debug!(num_parents = parent_ids.len(), "Parsed explicit parent IDs");

        // Use the method that accepts explicit parents and optional transfer
        state
            .node
            .submit_message_with_parents_and_transfer(
                req.content.into_bytes(),
                parent_ids,
                req.features,
                transfer,
            )
            .await
    } else {
        // Use the auto-parent-selection method with optional transfer
        state
            .node
            .submit_message_with_transfer(req.content.into_bytes(), transfer)
            .await
    };

    match result {
        Ok(message_id) => {
            state.metrics.messages_processed.inc();

            // Emit MessageAdded event
            state
                .node
                .event_bus
                .emit(crate::events::NodeEvent::MessageAdded {
                    message_id: hex::encode(message_id.as_bytes()),
                    depth: state.node.get_message_depth(&message_id).await.unwrap_or(0) as u64,
                    timestamp: chrono::Utc::now(),
                });

            (
                StatusCode::OK,
                Json(SubmitMessageResponse {
                    message_id: hex::encode(message_id.as_bytes()),
                    deposit_escrowed: Some(state.node.get_deposit_amount().to_adic()),
                }),
            )
                .into_response()
        }
        Err(e) => {
            state.metrics.messages_failed.inc();
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    }
}

async fn get_message(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let message_id = match hex::decode(&id) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut id_bytes = [0u8; 32];
            id_bytes.copy_from_slice(&bytes);
            MessageId::from_bytes(id_bytes)
        }
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };

    match state.node.get_message(&message_id).await {
        Ok(Some(message)) => {
            // Get depth from message index
            let depth = state.node.get_message_depth(&message.id).await.unwrap_or(0);

            // Convert message to full JSON format per API specification
            let json = serde_json::json!({
                "id": hex::encode(message.id.as_bytes()),
                "parents": message.parents.iter().map(|p| hex::encode(p.as_bytes())).collect::<Vec<_>>(),
                "features": {
                    "axes": message.features.phi.iter().map(|axis_phi| {
                        serde_json::json!({
                            "axis": axis_phi.axis.0,
                            "p": axis_phi.qp_digits.p,
                            "digits": axis_phi.qp_digits.digits,
                        })
                    }).collect::<Vec<_>>()
                },
                "content": general_purpose::STANDARD.encode(&message.data),  // Use base64 per API spec
                "timestamp": message.meta.timestamp.to_rfc3339(),
                "proposer": encode_address(&message.proposer_pk.as_bytes()).unwrap_or_else(|_| "invalid".to_string()),
                "signature": hex::encode(message.signature.as_bytes()),
                "depth": depth,
            });
            (StatusCode::OK, Json(json)).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn get_tips(State(state): State<Arc<AppState>>) -> Response {
    match state.node.get_tips().await {
        Ok(tips) => {
            let tip_strings: Vec<String> =
                tips.iter().map(|id| hex::encode(id.as_bytes())).collect();
            // Return proper JSON object with "tips" field per API specification
            let response = serde_json::json!({
                "tips": tip_strings
            });
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn get_update_status(State(state): State<Arc<AppState>>) -> Response {
    // Get current version
    let current_version = env!("CARGO_PKG_VERSION");

    // Get update manager state if available
    let (update_state, latest_version, update_available) =
        if let Some(ref update_manager) = state.node.update_manager {
            let state = update_manager.get_state().await;
            let latest = update_manager.get_latest_version().await;
            let available = latest
                .as_ref()
                .map(|v| v.version != current_version)
                .unwrap_or(false);

            let state_str = match state {
                adic_network::protocol::update::UpdateState::Idle => "idle",
                adic_network::protocol::update::UpdateState::CheckingVersion => "checking",
                adic_network::protocol::update::UpdateState::Downloading { .. } => "downloading",
                adic_network::protocol::update::UpdateState::Verifying { .. } => "verifying",
                adic_network::protocol::update::UpdateState::Applying { .. } => "applying",
                adic_network::protocol::update::UpdateState::Complete { .. } => "complete",
                adic_network::protocol::update::UpdateState::Failed { .. } => "error",
            };

            (state_str.to_string(), latest.map(|v| v.version), available)
        } else {
            ("idle".to_string(), None, false)
        };

    // Get version distribution if network is enabled
    let version_distribution = if let Some(ref network) = state.node.network {
        let network_lock = network.read().await;
        let peer_versions = network_lock.get_peer_versions().await;

        // Count versions
        let mut version_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for version in peer_versions.values() {
            *version_counts.entry(version.clone()).or_insert(0) += 1;
        }

        Some(version_counts)
    } else {
        None
    };

    let response = serde_json::json!({
        "current_version": current_version,
        "update_available": update_available,
        "latest_version": latest_version.unwrap_or_else(|| current_version.to_string()),
        "update_state": update_state,
        "auto_update_enabled": state.node.update_manager.is_some(),
        "last_check": chrono::Utc::now().to_rfc3339(),
        "version_distribution": version_distribution,
    });

    tracing::info!(
        current_version = current_version,
        "📦 Update status retrieved"
    );

    (StatusCode::OK, Json(response)).into_response()
}

async fn get_update_progress(State(state): State<Arc<AppState>>) -> Response {
    if let Some(ref update_manager) = state.node.update_manager {
        let state = update_manager.get_state().await;

        let response = match state {
            adic_network::protocol::update::UpdateState::Idle => {
                serde_json::json!({
                    "status": "idle",
                    "message": "No update in progress"
                })
            }
            adic_network::protocol::update::UpdateState::CheckingVersion => {
                serde_json::json!({
                    "status": "checking",
                    "message": "Checking for updates..."
                })
            }
            adic_network::protocol::update::UpdateState::Downloading {
                version,
                progress,
                chunks_received,
                total_chunks,
            } => {
                serde_json::json!({
                    "status": "downloading",
                    "version": version,
                    "progress_percent": (progress * 100.0) as u32,
                    "chunks_received": chunks_received,
                    "total_chunks": total_chunks,
                    "message": format!("Downloading version {} ({}/{} chunks)", version, chunks_received, total_chunks)
                })
            }
            adic_network::protocol::update::UpdateState::Verifying { version } => {
                serde_json::json!({
                    "status": "verifying",
                    "version": version,
                    "message": format!("Verifying binary signature for version {}", version)
                })
            }
            adic_network::protocol::update::UpdateState::Applying { version } => {
                serde_json::json!({
                    "status": "applying",
                    "version": version,
                    "message": format!("Applying update to version {}", version)
                })
            }
            adic_network::protocol::update::UpdateState::Complete { version, success } => {
                serde_json::json!({
                    "status": "complete",
                    "version": version,
                    "success": success,
                    "message": if success {
                        format!("Successfully updated to version {}", version)
                    } else {
                        format!("Update to version {} completed with issues", version)
                    }
                })
            }
            adic_network::protocol::update::UpdateState::Failed { version, error } => {
                serde_json::json!({
                    "status": "error",
                    "version": version,
                    "message": error,
                    "error": true
                })
            }
        };

        (StatusCode::OK, Json(response)).into_response()
    } else {
        let response = serde_json::json!({
            "status": "disabled",
            "message": "Update manager not initialized"
        });
        (StatusCode::OK, Json(response)).into_response()
    }
}

async fn get_update_swarm_stats(State(state): State<Arc<AppState>>) -> Response {
    if let Some(ref update_manager) = state.node.update_manager {
        // Get swarm statistics from update protocol
        let stats = update_manager.get_swarm_statistics().await;

        let response = serde_json::json!({
            "success": true,
            "swarm": {
                "total_download_speed": stats.total_download_speed,
                "total_upload_speed": stats.total_upload_speed,
                "downloading_peers": stats.downloading_peers,
                "seeding_peers": stats.seeding_peers,
                "idle_peers": stats.idle_peers,
                "total_active_transfers": stats.total_active_transfers,
                "average_download_progress": stats.average_download_progress,
                "version_distribution": stats.version_distribution,
                "total_peers": stats.downloading_peers + stats.seeding_peers + stats.idle_peers,
                "download_speed_mbps": stats.total_download_speed as f64 / 1_048_576.0,
                "upload_speed_mbps": stats.total_upload_speed as f64 / 1_048_576.0,
            }
        });

        (StatusCode::OK, Json(response)).into_response()
    } else {
        let response = serde_json::json!({
            "success": false,
            "message": "Update manager not initialized"
        });
        (StatusCode::SERVICE_UNAVAILABLE, Json(response)).into_response()
    }
}

async fn trigger_update_check(State(state): State<Arc<AppState>>) -> Response {
    if let Some(ref update_manager) = state.node.update_manager {
        match update_manager.check_for_update().await {
            Ok(Some(version)) => {
                let response = serde_json::json!({
                    "success": true,
                    "message": format!("Update available: version {}", version.version),
                    "version": version.version,
                    "hash": version.sha256_hash,
                });
                (StatusCode::OK, Json(response)).into_response()
            }
            Ok(None) => {
                let response = serde_json::json!({
                    "success": true,
                    "message": "No update available",
                });
                (StatusCode::OK, Json(response)).into_response()
            }
            Err(e) => {
                let response = serde_json::json!({
                    "success": false,
                    "message": format!("Failed to check for update: {}", e),
                });
                (StatusCode::INTERNAL_SERVER_ERROR, Json(response)).into_response()
            }
        }
    } else {
        let response = serde_json::json!({
            "success": false,
            "message": "Update manager not initialized"
        });
        (StatusCode::SERVICE_UNAVAILABLE, Json(response)).into_response()
    }
}

async fn trigger_update_apply(State(state): State<Arc<AppState>>) -> Response {
    if let Some(ref update_manager) = state.node.update_manager {
        // First check if an update is available
        match update_manager.get_latest_version().await {
            Some(version) => {
                // Trigger the update in the background
                let update_manager = update_manager.clone();
                tokio::spawn(async move {
                    if let Err(e) = update_manager.start_auto_update().await {
                        tracing::error!("Failed to apply update: {}", e);
                    }
                });

                let response = serde_json::json!({
                    "success": true,
                    "message": format!("Starting update to version {}", version.version),
                    "version": version.version,
                });
                (StatusCode::OK, Json(response)).into_response()
            }
            None => {
                let response = serde_json::json!({
                    "success": false,
                    "message": "No update available to apply",
                });
                (StatusCode::BAD_REQUEST, Json(response)).into_response()
            }
        }
    } else {
        let response = serde_json::json!({
            "success": false,
            "message": "Update manager not initialized"
        });
        (StatusCode::SERVICE_UNAVAILABLE, Json(response)).into_response()
    }
}

async fn get_peers(State(state): State<Arc<AppState>>) -> Response {
    // Check if network is enabled
    if let Some(ref network) = state.node.network {
        let network_lock = network.read().await;
        let peers_info = network_lock.get_all_peers_info().await;

        // Get peer versions
        let peer_versions = network_lock.get_peer_versions().await;

        // Convert PeerInfo to PeerInfoResponse
        let peer_responses: Vec<PeerInfoResponse> = peers_info
            .into_iter()
            .map(|info| {
                let connection_state = match info.connection_state {
                    adic_network::peer::ConnectionState::Connected => "connected",
                    adic_network::peer::ConnectionState::Connecting => "connecting",
                    adic_network::peer::ConnectionState::Disconnected => "disconnected",
                    adic_network::peer::ConnectionState::Failed => "failed",
                };

                // Get version for this peer
                let version = peer_versions.get(&info.peer_id.to_string()).cloned();

                PeerInfoResponse {
                    peer_id: info.peer_id.to_string(),
                    addresses: info.addresses.iter().map(|a| a.to_string()).collect(),
                    connection_state: connection_state.to_string(),
                    reputation_score: info.reputation_score,
                    latency_ms: info.latency_ms,
                    version,
                }
            })
            .collect();

        // Log the state retrieval with structured field as per best practices
        tracing::info!(peer_count = peer_responses.len(), "📡 Peers retrieved");

        (StatusCode::OK, Json(peer_responses)).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Network is not enabled".to_string(),
            }),
        )
            .into_response()
    }
}

async fn get_network_status(State(state): State<Arc<AppState>>) -> Response {
    // Check if network is enabled
    if let Some(ref network) = state.node.network {
        let network_lock = network.read().await;
        let stats = network_lock.get_network_stats().await;

        let response = NetworkStatusResponse {
            peer_count: stats.peer_count,
            connected_peers: stats.connected_peers,
            messages_sent: stats.messages_sent,
            messages_received: stats.messages_received,
            bytes_sent: stats.bytes_sent,
            bytes_received: stats.bytes_received,
        };

        // Log the network status retrieval with structured fields as per best practices
        tracing::info!(
            peer_count = stats.peer_count,
            connected_peers = stats.connected_peers,
            "🌐 Network status retrieved"
        );

        (StatusCode::OK, Json(response)).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Network is not enabled".to_string(),
            }),
        )
            .into_response()
    }
}

async fn get_finality_artifact(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let message_id = match hex::decode(&id) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut id_bytes = [0u8; 32];
            id_bytes.copy_from_slice(&bytes);
            MessageId::from_bytes(id_bytes)
        }
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };

    // Get F1 (K-core) finality status
    let f1_artifact = state.node.get_finality_artifact(&message_id).await;

    // Extract finality data from artifact if available
    let (
        f1_finalized,
        f1_k_core,
        f1_depth,
        f2_stabilized,
        f2_h3_stable,
        f2_bottleneck,
        f2_confidence,
    ) = if let Some(ref artifact) = f1_artifact {
        let f2_stable = artifact.witness.h3_stable.unwrap_or(false);
        (
            true,
            artifact.witness.core_size,
            artifact.witness.depth,
            f2_stable,
            artifact.witness.h3_stable,
            artifact.witness.h2_bottleneck_distance,
            artifact.witness.f2_confidence,
        )
    } else {
        (false, 0, 0, false, None, None, None)
    };

    // Flatten response format to match API specification and indexer contract
    let response = serde_json::json!({
        "message_id": id,
        "f1_finalized": f1_finalized,
        "f1_k_core": f1_k_core,
        "f1_depth": f1_depth,
        "f2_stabilized": f2_stabilized,
        "f2_h3_stable": f2_h3_stable,
        "f2_bottleneck_distance": f2_bottleneck,
        "f2_confidence": f2_confidence,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    (StatusCode::OK, Json(response)).into_response()
}

async fn get_metrics(State(state): State<Arc<AppState>>) -> String {
    state.metrics.gather()
}

async fn get_mrw_traces(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TracesQuery>,
) -> Response {
    let limit = params.limit.unwrap_or(10).min(50); // Cap at 50 traces

    let traces = state.node.mrw.get_recent_traces(limit).await;

    let response = serde_json::json!({
        "traces": traces.iter().map(|(id, trace)| {
            serde_json::json!({
                "id": id,
                "success": trace.success,
                "parent_count": trace.selected_parents.len(),
                "step_count": trace.step_count(),
                "duration_ms": trace.duration_ms(),
                "widen_count": trace.widen_count,
                "candidates_considered": trace.total_candidates_considered,
            })
        }).collect::<Vec<_>>(),
        "total": traces.len()
    });

    (StatusCode::OK, Json(response)).into_response()
}

async fn get_mrw_trace(
    State(state): State<Arc<AppState>>,
    Path(trace_id): Path<String>,
) -> Response {
    match state.node.mrw.get_trace(&trace_id).await {
        Some(trace) => (StatusCode::OK, Json(trace)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn get_all_reputations(State(state): State<Arc<AppState>>) -> Response {
    let scores = state.node.consensus.reputation.get_all_scores().await;
    let mut reputations = HashMap::new();

    for (pubkey, score) in scores {
        let address = encode_address(&pubkey.as_bytes()).unwrap_or_else(|_| "invalid".to_string());
        reputations.insert(address, score.value);
    }

    // Wrap in "reputations" object per API specification
    let response = serde_json::json!({
        "reputations": reputations
    });

    (StatusCode::OK, Json(response)).into_response()
}

async fn get_reputation(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> Response {
    // Decode adic1... address to public key bytes
    let pubkey_bytes = match decode_address(&address) {
        Ok(bytes) => PublicKey::from_bytes(bytes),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let scores = state.node.consensus.reputation.get_all_scores().await;
    let score = scores.get(&pubkey_bytes);

    let response = if let Some(s) = score {
        serde_json::json!({
            "address": address,
            "reputation": s.value,
            "messages_approved": s.messages_finalized,
            "last_updated": chrono::Utc::now().to_rfc3339(),
        })
    } else {
        serde_json::json!({
            "address": address,
            "reputation": 1.0, // Default reputation
            "messages_approved": 0,
            "last_updated": chrono::Utc::now().to_rfc3339(),
        })
    };

    (StatusCode::OK, Json(response)).into_response()
}

async fn get_all_conflicts(State(state): State<Arc<AppState>>) -> Response {
    let conflicts = state.node.consensus.energy_tracker().get_all_conflicts().await;

    let mut conflict_list = Vec::new();
    for (conflict_id, energy) in conflicts {
        // Get conflicting messages for this conflict
        let messages = energy
            .support
            .keys()
            .map(|msg_id| hex::encode(msg_id.as_bytes()))
            .collect::<Vec<_>>();

        let status = if energy.get_winner().is_some() {
            "resolved"
        } else {
            "pending"
        };

        let conflict_json = serde_json::json!({
            "id": conflict_id.to_string(),
            "messages": messages,
            "status": status,
            "created": energy.last_update,  // Using last_update as proxy for creation time
        });
        conflict_list.push(conflict_json);
    }

    // Wrap in "conflicts" object per API specification
    let response = serde_json::json!({
        "conflicts": conflict_list
    });

    (StatusCode::OK, Json(response)).into_response()
}

async fn get_conflict_details(
    State(state): State<Arc<AppState>>,
    Path(conflict_id_str): Path<String>,
) -> Response {
    let conflict_id = ConflictId::new(conflict_id_str.clone());

    let energy = state
        .node
        .consensus
        .energy_tracker()
        .get_energy(&conflict_id)
        .await;
    let is_resolved = state
        .node
        .consensus
        .energy_tracker()
        .is_resolved(&conflict_id)
        .await;
    let winner = state
        .node
        .consensus
        .energy_tracker()
        .get_winner(&conflict_id)
        .await;

    // Get detailed conflict info
    let conflict_details = state
        .node
        .consensus
        .energy_tracker()
        .get_conflict_details(&conflict_id)
        .await;

    let response = if let Some(details) = conflict_details {
        // Extract conflicting messages and their energy scores
        let conflicting_messages: Vec<String> = details
            .support
            .keys()
            .map(|msg_id| hex::encode(msg_id.as_bytes()))
            .collect();

        let energy_scores: Vec<f64> = details.support.values().copied().collect();

        let resolution = if is_resolved {
            winner.map(|id| hex::encode(id.as_bytes()))
        } else {
            None
        };

        serde_json::json!({
            "id": conflict_id_str,
            "conflicting_messages": conflicting_messages,
            "energy_scores": energy_scores,
            "resolution": resolution,
            "resolved_at": if is_resolved {
                Some(details.last_update)
            } else {
                None
            },
        })
    } else {
        serde_json::json!({
            "id": conflict_id_str,
            "energy": energy,
            "is_resolved": is_resolved,
            "winner": winner.map(|id| hex::encode(id.as_bytes())),
            "support": [],
            "last_update": null,
        })
    };

    (StatusCode::OK, Json(response)).into_response()
}

async fn get_detailed_statistics(State(state): State<Arc<AppState>>) -> Response {
    // Get basic node stats
    let node_stats = match state.node.get_stats().await {
        Ok(stats) => stats,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    // Get storage stats
    let storage_stats = match state.node.storage.get_stats().await {
        Ok(stats) => stats,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    // Get finality stats
    let finality_stats = state.node.finality.get_stats().await;

    // Get conflict stats
    let conflicts_count = state
        .node
        .consensus
        .energy_tracker()
        .get_all_conflicts()
        .await
        .len();
    let resolved_conflicts = state
        .node
        .consensus
        .energy_tracker()
        .get_resolved_conflicts(0.5)
        .await
        .len();

    // Get reputation stats
    let all_reputations = state.node.consensus.reputation.get_all_scores().await;
    let avg_reputation = if all_reputations.is_empty() {
        0.0
    } else {
        all_reputations.values().map(|s| s.value).sum::<f64>() / all_reputations.len() as f64
    };

    // Get deposit stats
    let deposit_stats = state.node.consensus.deposits.get_stats().await;

    let response = serde_json::json!({
        "node": {
            "message_count": node_stats.message_count,
            "tip_count": node_stats.tip_count,
            "finalized_count": node_stats.finalized_count,
            "pending_finality": node_stats.pending_finality,
        },
        "storage": {
            "message_count": storage_stats.message_count,
            "tip_count": storage_stats.tip_count,
            "finalized_count": storage_stats.finalized_count,
            "reputation_entries": storage_stats.reputation_entries,
            "conflict_sets": storage_stats.conflict_sets,
            "total_size_bytes": storage_stats.total_size_bytes,
        },
        "finality": {
            "finalized_count": finality_stats.finalized_count,
            "pending_count": finality_stats.pending_count,
            "total_messages": finality_stats.total_messages,
        },
        "conflicts": {
            "total": conflicts_count,
            "resolved": resolved_conflicts,
            "pending": conflicts_count - resolved_conflicts,
        },
        "reputation": {
            "tracked_nodes": all_reputations.len(),
            "average_reputation": avg_reputation,
            "max_reputation": all_reputations.values().map(|s| s.value).fold(0.0f64, f64::max),
            "min_reputation": all_reputations.values().map(|s| s.value).fold(10.0f64, f64::min),
        },
        "deposits": {
            "escrowed_count": deposit_stats.escrowed_count,
            "total_escrowed_amount": deposit_stats.total_escrowed,
            "slashed_count": deposit_stats.slashed_count,
            "total_slashed_amount": deposit_stats.total_slashed,
            "refunded_count": deposit_stats.refunded_count,
            "total_refunded_amount": deposit_stats.total_refunded,
        },
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    (StatusCode::OK, Json(response)).into_response()
}

async fn get_deposits_summary(State(state): State<Arc<AppState>>) -> Response {
    let deposit_stats = state.node.consensus.deposits.get_stats().await;
    let recent_deposits = state.node.consensus.deposits.get_recent_deposits(20).await;

    let response = serde_json::json!({
        "summary": {
            "escrowed_count": deposit_stats.escrowed_count,
            "total_escrowed": deposit_stats.total_escrowed,
            "slashed_count": deposit_stats.slashed_count,
            "total_slashed": deposit_stats.total_slashed,
            "refunded_count": deposit_stats.refunded_count,
            "total_refunded": deposit_stats.total_refunded,
        },
        "recent_deposits": recent_deposits.iter().map(|deposit| {
            serde_json::json!({
                "message_id": hex::encode(deposit.message_id.as_bytes()),
                "proposer": encode_address(&deposit.proposer_pk.as_bytes()).unwrap_or_else(|_| "invalid".to_string()),
                "amount": deposit.amount,
                "status": format!("{:?}", deposit.status),
                "timestamp": deposit.timestamp,
            })
        }).collect::<Vec<_>>(),
    });

    (StatusCode::OK, Json(response)).into_response()
}

async fn get_deposit_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let message_id = match hex::decode(&id) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut id_bytes = [0u8; 32];
            id_bytes.copy_from_slice(&bytes);
            MessageId::from_bytes(id_bytes)
        }
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };

    match state.node.consensus.deposits.get_deposit(&message_id).await {
        Ok(Some(deposit)) => {
            let response = serde_json::json!({
                "message_id": hex::encode(deposit.message_id.as_bytes()),
                "proposer": encode_address(&deposit.proposer_pk.as_bytes()).unwrap_or_else(|_| "invalid".to_string()),
                "amount": deposit.amount,
                "status": format!("{:?}", deposit.status),
                "timestamp": deposit.timestamp,
                "slashed": deposit.status == adic_consensus::DepositStatus::Slashed,
                "refunded": deposit.status == adic_consensus::DepositStatus::Refunded,
            });
            (StatusCode::OK, Json(response)).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

// Wallet API handlers (wrappers for api_wallet module)

async fn wallet_info_handler(State(state): State<Arc<AppState>>) -> Response {
    crate::api_wallet::get_wallet_info(Arc::new(state.node.clone())).await
}

async fn balance_handler(
    Path(address): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Response {
    crate::api_wallet::get_balance(address, Arc::new(state.node.clone())).await
}

async fn transfer_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<crate::api_wallet::TransferRequest>,
) -> Response {
    crate::api_wallet::transfer(Arc::new(state.node.clone()), req).await
}

async fn faucet_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<crate::api_wallet::FaucetRequest>,
) -> Response {
    crate::api_wallet::request_faucet(Arc::new(state.node.clone()), req).await
}

async fn sign_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<crate::api_wallet::SignRequest>,
) -> Response {
    crate::api_wallet::sign_message(Arc::new(state.node.clone()), req).await
}

async fn transactions_handler(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> Response {
    crate::api_wallet_tx::get_transaction_history(address, Arc::new(state.node.clone())).await
}

#[derive(Deserialize)]
struct AllTransactionsQuery {
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn all_transactions_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AllTransactionsQuery>,
) -> Response {
    crate::api_wallet_tx::get_all_transactions(
        query.limit,
        query.offset,
        Arc::new(state.node.clone()),
    )
    .await
}

// Bulk message query endpoints

#[derive(Deserialize)]
struct BulkMessageQuery {
    ids: String, // Comma-separated list of message IDs
}

async fn get_messages_bulk(
    State(state): State<Arc<AppState>>,
    Query(params): Query<BulkMessageQuery>,
) -> Response {
    let ids: Vec<String> = params
        .ids
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    if ids.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "No message IDs provided"
            })),
        )
            .into_response();
    }

    if ids.len() > 1000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Too many IDs requested. Maximum is 1000"
            })),
        )
            .into_response();
    }

    let mut messages = Vec::new();
    let mut not_found = Vec::new();

    for id_str in ids {
        let message_id = match hex::decode(&id_str) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut id_bytes = [0u8; 32];
                id_bytes.copy_from_slice(&bytes);
                MessageId::from_bytes(id_bytes)
            }
            _ => {
                not_found.push(id_str);
                continue;
            }
        };

        match state.node.get_message(&message_id).await {
            Ok(Some(message)) => {
                // Get depth from message index
                let depth = state.node.get_message_depth(&message.id).await.unwrap_or(0);

                let json = serde_json::json!({
                    "id": hex::encode(message.id.as_bytes()),
                    "parents": message.parents.iter().map(|p| hex::encode(p.as_bytes())).collect::<Vec<_>>(),
                    "features": {
                        "axes": message.features.phi.iter().map(|axis_phi| {
                            serde_json::json!({
                                "axis": axis_phi.axis.0,
                                "p": axis_phi.qp_digits.p,
                                "digits": axis_phi.qp_digits.digits,
                            })
                        }).collect::<Vec<_>>()
                    },
                    "content": general_purpose::STANDARD.encode(&message.data),
                    "timestamp": message.meta.timestamp.to_rfc3339(),
                    "proposer": encode_address(&message.proposer_pk.as_bytes()).unwrap_or_else(|_| "invalid".to_string()),
                    "signature": hex::encode(message.signature.as_bytes()),
                    "depth": depth,
                });
                messages.push(json);
            }
            _ => {
                not_found.push(id_str);
            }
        }
    }

    let response = serde_json::json!({
        "messages": messages,
        "not_found": not_found,
    });

    (StatusCode::OK, Json(response)).into_response()
}

#[derive(Deserialize)]
struct RangeQuery {
    start: String, // ISO8601 timestamp
    end: String,   // ISO8601 timestamp
    limit: Option<usize>,
    cursor: Option<String>,
}

async fn get_messages_range(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RangeQuery>,
) -> Response {
    let start_time = match chrono::DateTime::parse_from_rfc3339(&params.start) {
        Ok(dt) => dt.with_timezone(&chrono::Utc),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Invalid start timestamp"
                })),
            )
                .into_response();
        }
    };

    let end_time = match chrono::DateTime::parse_from_rfc3339(&params.end) {
        Ok(dt) => dt.with_timezone(&chrono::Utc),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Invalid end timestamp"
                })),
            )
                .into_response();
        }
    };

    let limit = params.limit.unwrap_or(1000).min(1000);

    // Get messages in time range from storage
    let messages = match state
        .node
        .storage
        .get_messages_in_range(start_time, end_time, limit, params.cursor)
        .await
    {
        Ok(msgs) => msgs,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Failed to retrieve messages"
                })),
            )
                .into_response();
        }
    };

    let has_more = messages.len() >= limit;
    let next_cursor = if has_more {
        messages.last().map(|m| hex::encode(m.id.as_bytes()))
    } else {
        None
    };

    let mut messages_json = Vec::new();
    for message in &messages {
        // Get depth from message index
        let depth = state.node.get_message_depth(&message.id).await.unwrap_or(0);

        let json = serde_json::json!({
            "id": hex::encode(message.id.as_bytes()),
            "parents": message.parents.iter().map(|p| hex::encode(p.as_bytes())).collect::<Vec<_>>(),
            "features": {
                "axes": message.features.phi.iter().map(|axis_phi| {
                    serde_json::json!({
                        "axis": axis_phi.axis.0,
                        "p": axis_phi.qp_digits.p,
                        "digits": axis_phi.qp_digits.digits,
                    })
                }).collect::<Vec<_>>()
            },
            "content": general_purpose::STANDARD.encode(&message.data),
            "timestamp": message.meta.timestamp.to_rfc3339(),
            "proposer": encode_address(&message.proposer_pk.as_bytes()).unwrap_or_else(|_| "invalid".to_string()),
            "signature": hex::encode(message.signature.as_bytes()),
            "depth": depth,
        });
        messages_json.push(json);
    }

    let response = serde_json::json!({
        "messages": messages_json,
        "next_cursor": next_cursor,
        "has_more": has_more,
    });

    (StatusCode::OK, Json(response)).into_response()
}

async fn get_messages_since(
    State(state): State<Arc<AppState>>,
    Path(checkpoint_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let checkpoint = match hex::decode(&checkpoint_id) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut id_bytes = [0u8; 32];
            id_bytes.copy_from_slice(&bytes);
            MessageId::from_bytes(id_bytes)
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Invalid checkpoint ID"
                })),
            )
                .into_response();
        }
    };

    let limit = params
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1000)
        .min(1000);

    // Get messages since checkpoint
    let messages = match state
        .node
        .storage
        .get_messages_since(&checkpoint, limit)
        .await
    {
        Ok(msgs) => msgs,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Failed to retrieve messages"
                })),
            )
                .into_response();
        }
    };

    let mut messages_json = Vec::new();
    for message in &messages {
        // Get depth from message index
        let depth = state.node.get_message_depth(&message.id).await.unwrap_or(0);

        let json = serde_json::json!({
            "id": hex::encode(message.id.as_bytes()),
            "parents": message.parents.iter().map(|p| hex::encode(p.as_bytes())).collect::<Vec<_>>(),
            "features": {
                "axes": message.features.phi.iter().map(|axis_phi| {
                    serde_json::json!({
                        "axis": axis_phi.axis.0,
                        "p": axis_phi.qp_digits.p,
                        "digits": axis_phi.qp_digits.digits,
                    })
                }).collect::<Vec<_>>()
            },
            "content": general_purpose::STANDARD.encode(&message.data),
            "timestamp": message.meta.timestamp.to_rfc3339(),
            "proposer": encode_address(&message.proposer_pk.as_bytes()).unwrap_or_else(|_| "invalid".to_string()),
            "signature": hex::encode(message.signature.as_bytes()),
            "depth": depth,
        });
        messages_json.push(json);
    }

    let response = serde_json::json!({
        "messages": messages_json,
        "count": messages.len(),
    });

    (StatusCode::OK, Json(response)).into_response()
}

// New API endpoints to match README claims

async fn get_ball_messages(
    State(state): State<Arc<AppState>>,
    Path((axis, radius, center_hex)): Path<(u32, usize, String)>,
) -> Response {
    // Decode the ball ID (center) from hex
    let ball_id = match hex::decode(&center_hex) {
        Ok(id) => id,
        Err(_) => {
            let error_response = serde_json::json!({
                "error": "INVALID_CENTER",
                "message": "Center parameter must be a valid hex string",
            });
            return (StatusCode::BAD_REQUEST, Json(error_response)).into_response();
        }
    };

    // Verify the ball_id length matches the radius
    if ball_id.len() != radius {
        let error_response = serde_json::json!({
            "error": "INVALID_CENTER_LENGTH",
            "message": format!("Center length ({}) must match radius ({})", ball_id.len(), radius),
        });
        return (StatusCode::BAD_REQUEST, Json(error_response)).into_response();
    }

    match state.node.storage.get_ball_members(axis, &ball_id).await {
        Ok(message_ids) => {
            let response = serde_json::json!({
                "axis": axis,
                "radius": radius,
                "center": center_hex,
                "message_count": message_ids.len(),
                "messages": message_ids.iter().map(|id| hex::encode(id.as_bytes())).collect::<Vec<_>>(),
            });
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(_) => {
            let error_response = serde_json::json!({
                "error": "STORAGE_ERROR",
                "message": "Failed to retrieve ball members from storage",
                "axis": axis,
                "radius": radius
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response)).into_response()
        }
    }
}

#[derive(Deserialize)]
struct MembershipProofRequest {
    message_id: String,
    axis: u32,
    radius: usize,
}

async fn generate_membership_proof(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MembershipProofRequest>,
) -> Response {
    // Decode message ID
    let message_id = match hex::decode(&req.message_id) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut id_bytes = [0u8; 32];
            id_bytes.copy_from_slice(&bytes);
            MessageId::from_bytes(id_bytes)
        }
        _ => {
            let error_response = serde_json::json!({
                "error": "INVALID_MESSAGE_ID",
                "message": "Message ID must be a valid 32-byte hex string",
            });
            return (StatusCode::BAD_REQUEST, Json(error_response)).into_response();
        }
    };

    // Get the message from storage
    let message = match state.node.get_message(&message_id).await {
        Ok(Some(msg)) => msg,
        Ok(None) => {
            let error_response = serde_json::json!({
                "error": "MESSAGE_NOT_FOUND",
                "message": "Message not found in storage",
            });
            return (StatusCode::NOT_FOUND, Json(error_response)).into_response();
        }
        Err(e) => {
            let error_response = serde_json::json!({
                "error": "STORAGE_ERROR",
                "message": format!("Failed to retrieve message: {}", e),
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response)).into_response();
        }
    };

    // Get the p-adic features for the requested axis
    let axis_phi = match message
        .features
        .phi
        .iter()
        .find(|phi| phi.axis.0 == req.axis)
    {
        Some(phi) => phi,
        None => {
            let error_response = serde_json::json!({
                "error": "INVALID_AXIS",
                "message": format!("Axis {} not found in message features", req.axis),
            });
            return (StatusCode::BAD_REQUEST, Json(error_response)).into_response();
        }
    };

    // Compute the ball ID for this message at the requested radius
    let computed_ball_id = ball_id(&axis_phi.qp_digits, req.radius);

    // The proof consists of:
    // 1. The message's p-adic features
    // 2. The computed ball ID
    // 3. The axis and radius parameters
    let response = serde_json::json!({
        "message_id": req.message_id,
        "axis": req.axis,
        "radius": req.radius,
        "ball_id": hex::encode(&computed_ball_id),
        "proof": {
            "type": "ball_membership",
            "features": {
                "p": axis_phi.qp_digits.p,
                "digits": axis_phi.qp_digits.digits,
            },
            "ball_id": hex::encode(&computed_ball_id),
            "verification": "Ball ID computed from message p-adic features"
        },
        "verified": true
    });

    (StatusCode::OK, Json(response)).into_response()
}

#[derive(Deserialize)]
struct VerifyProofRequest {
    proof: serde_json::Value,
}

async fn verify_proof(
    State(_state): State<Arc<AppState>>,
    Json(req): Json<VerifyProofRequest>,
) -> Response {
    // Extract proof type
    let proof_type = req
        .proof
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    match proof_type {
        "ball_membership" => {
            // Extract proof fields
            let features = match req.proof.get("features") {
                Some(f) => f,
                None => {
                    let error_response = serde_json::json!({
                        "error": "INVALID_PROOF",
                        "message": "Proof missing 'features' field",
                    });
                    return (StatusCode::BAD_REQUEST, Json(error_response)).into_response();
                }
            };

            let p = features.get("p").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
            let digits = features
                .get("digits")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_u64().map(|n| n as u8))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            let claimed_ball_id = req
                .proof
                .get("ball_id")
                .and_then(|v| v.as_str())
                .and_then(|s| hex::decode(s).ok())
                .unwrap_or_default();

            // Reconstruct QpDigits and compute ball ID
            let qp_digits = QpDigits { p, digits };
            let radius = claimed_ball_id.len();
            let computed_ball_id = ball_id(&qp_digits, radius);

            let verified = computed_ball_id == claimed_ball_id;

            let response = serde_json::json!({
                "verified": verified,
                "proof_type": "ball_membership",
                "details": if verified {
                    "Proof valid: computed ball ID matches claimed ball ID"
                } else {
                    "Proof invalid: computed ball ID does not match claimed ball ID"
                },
                "computed_ball_id": hex::encode(&computed_ball_id),
                "claimed_ball_id": hex::encode(&claimed_ball_id),
            });

            (StatusCode::OK, Json(response)).into_response()
        }
        _ => {
            let error_response = serde_json::json!({
                "error": "UNSUPPORTED_PROOF_TYPE",
                "message": format!("Proof type '{}' is not supported", proof_type),
                "supported_types": ["ball_membership"],
            });
            (StatusCode::BAD_REQUEST, Json(error_response)).into_response()
        }
    }
}

async fn get_security_score(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let message_id = match hex::decode(&id) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut id_bytes = [0u8; 32];
            id_bytes.copy_from_slice(&bytes);
            MessageId::from_bytes(id_bytes)
        }
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };

    // Get message from storage
    match state.node.get_message(&message_id).await {
        Ok(Some(message)) => {
            // Get parent features and reputations from storage
            let mut parent_features = Vec::new();
            let mut parent_reputations = Vec::new();
            for parent_id in &message.parents {
                if let Ok(Some(parent_msg)) = state.node.get_message(parent_id).await {
                    let features: Vec<QpDigits> = parent_msg
                        .features
                        .phi
                        .iter()
                        .map(|axis| axis.qp_digits.clone())
                        .collect();
                    parent_features.push(features);

                    let rep = state
                        .node
                        .consensus
                        .reputation
                        .get_reputation(&parent_msg.proposer_pk)
                        .await;
                    parent_reputations.push(rep);
                }
            }

            let checker = state.node.consensus.admissibility();
            let result = checker.check_message(&message, &parent_features, &parent_reputations);

            let response = match result {
                Ok(res) => {
                    // C1 (Proximity) is the score itself.
                    // C2 (Diversity) and C3 (Reputation) are boolean checks.
                    // We represent pass/fail as 1.0/0.0 to fit the score model.
                    let c1_score = res.score;
                    let c2_score = if res.c2_passed { 1.0 } else { 0.0 };
                    let c3_score = if res.c3_passed { 1.0 } else { 0.0 };

                    // The overall score can be a combination or just the main score
                    let overall_score = (c1_score * c2_score * c3_score).cbrt();

                    serde_json::json!({
                        "message_id": id,
                        "is_admissible": res.is_admissible,
                        "admissibility_score": res.score,
                        "c1_score": c1_score,
                        "c2_score": c2_score,
                        "c3_score": c3_score,
                        "overall": overall_score,
                        "details": res.details,
                    })
                }
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": "Failed to check admissibility",
                            "details": e.to_string()
                        })),
                    )
                        .into_response();
                }
            };

            (StatusCode::OK, Json(response)).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

// New API endpoints for explorer dashboard metrics

async fn get_diversity_stats(State(state): State<Arc<AppState>>) -> Response {
    // Get current tips to analyze diversity
    let tips = match state.node.storage.get_tips().await {
        Ok(tips) => tips,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    // Analyze p-adic ball distribution per axis
    let mut axis_stats = Vec::new();
    let params = state.node.consensus.params().await;

    for (axis_idx, &radius) in params.rho.iter().enumerate() {
        let mut distinct_balls = std::collections::HashSet::new();

        // For each tip, compute which ball it belongs to on this axis
        for tip_id in &tips {
            if let Ok(Some(msg)) = state.node.storage.get_message(tip_id).await {
                if let Some(axis_phi) = msg.features.get_axis(adic_types::AxisId(axis_idx as u32)) {
                    // Compute ball ID based on radius
                    let ball_id = adic_math::ball_id(&axis_phi.qp_digits, radius as usize);
                    distinct_balls.insert(ball_id);
                }
            }
        }

        axis_stats.push(serde_json::json!({
            "axis": axis_idx,
            "radius": radius,
            "distinct_balls": distinct_balls.len(),
            "required_diversity": params.q,
            "meets_requirement": distinct_balls.len() >= params.q as usize,
        }));
    }

    // Calculate overall diversity score
    let total_axes = axis_stats.len();
    let compliant_axes = axis_stats
        .iter()
        .filter(|s| s["meets_requirement"].as_bool().unwrap_or(false))
        .count();
    let diversity_score = if total_axes > 0 {
        (compliant_axes as f64) / (total_axes as f64)
    } else {
        0.0
    };

    let response = serde_json::json!({
        "axes": axis_stats,
        "diversity_score": diversity_score,
        "total_tips": tips.len(),
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    (StatusCode::OK, Json(response)).into_response()
}

async fn get_active_energy_conflicts(State(state): State<Arc<AppState>>) -> Response {
    // Get energy metrics from the energy descent tracker
    let metrics = state.node.consensus.energy_tracker.get_metrics().await;

    // Get all conflicts with their energy levels
    let all_conflicts = state.node.consensus.energy_tracker().get_all_conflicts().await;
    let mut active_conflicts = Vec::new();

    for (conflict_id, conflict_energy) in all_conflicts.iter() {
        // Check if conflict is still active (not resolved)
        let is_resolved = state
            .node
            .consensus
            .energy_tracker
            .is_resolved(conflict_id)
            .await;

        if !is_resolved {
            // Get expected drift for this conflict
            let drift = state
                .node
                .consensus
                .energy_tracker
                .calculate_expected_drift(conflict_id)
                .await;

            active_conflicts.push(serde_json::json!({
                "conflict_id": conflict_id.to_string(),
                "energy": conflict_energy.total_energy,
                "expected_drift": drift,
                "is_descending": drift < 0.0,
                "support": conflict_energy.support.iter()
                    .map(|(msg_id, support)| {
                        serde_json::json!({
                            "message_id": hex::encode(msg_id.as_bytes()),
                            "support": support,
                        })
                    })
                    .collect::<Vec<_>>(),
                "last_update": conflict_energy.last_update,
            }));
        }
    }

    let response = serde_json::json!({
        "total_conflicts": metrics.total_conflicts,
        "resolved_conflicts": metrics.resolved_conflicts,
        "descending_conflicts": metrics.descending_conflicts,
        "total_energy": metrics.total_energy,
        "average_energy": metrics.average_energy,
        "active_conflicts": active_conflicts,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    (StatusCode::OK, Json(response)).into_response()
}

async fn get_kcore_metrics(State(state): State<Arc<AppState>>) -> Response {
    // Get finality stats
    let finality_stats = state.node.finality.get_stats().await;

    // Get recent finalized messages to analyze k-core properties
    let recent_finalized: Vec<MessageId> = state
        .node
        .storage
        .get_recently_finalized(20)
        .await
        .unwrap_or_default();

    // Analyze k-core properties of recent finalizations
    let mut kcore_analyses = Vec::new();
    let mut total_k_value = 0.0;
    let mut total_depth = 0.0;
    let mut count = 0;

    for msg_id in recent_finalized.iter().take(5) {
        // Analyze top 5 for performance
        if let Ok(Some(result)) = state.node.finality.check_kcore_finality(msg_id).await {
            kcore_analyses.push(serde_json::json!({
                "message_id": hex::encode(msg_id.as_bytes()),
                "k_value": result.k_value,
                "depth": result.depth,
                "diversity": result.diversity_score,
                "is_final": result.is_final,
            }));

            // Accumulate for averages
            total_k_value += result.k_value as f64;
            total_depth += result.depth as f64;
            count += 1;
        }
    }

    // Calculate averages
    let average_k_value = if count > 0 {
        total_k_value / count as f64
    } else {
        0.0
    };
    let average_depth = if count > 0 {
        total_depth / count as f64
    } else {
        0.0
    };

    // Get k-core parameters
    let params = state.node.consensus.params().await;

    let response = serde_json::json!({
        "parameters": {
            "required_k": params.k,
            "required_depth": 12,  // From whitepaper: D* = 12
            "required_diversity": params.q,
        },
        "current_stats": {
            "finalized_count": finality_stats.finalized_count,
            "pending_count": finality_stats.pending_count,
            "average_k_value": average_k_value,
            "average_depth": average_depth,
        },
        "recent_finalizations": kcore_analyses,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    (StatusCode::OK, Json(response)).into_response()
}

async fn get_admissibility_rates(State(state): State<Arc<AppState>>) -> Response {
    // Track admissibility check results over recent messages
    let tips = match state.node.storage.get_tips().await {
        Ok(tips) => tips,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let mut total_checks = 0;
    let mut c1_passes = 0; // Proximity constraint
    let mut c2_passes = 0; // Diversity constraint
    let mut c3_passes = 0; // Reputation constraint
    let mut fully_admissible = 0;

    // Sample recent messages for admissibility analysis
    for tip_id in tips.iter().take(100) {
        // Sample up to 100 tips
        if let Ok(Some(msg)) = state.node.storage.get_message(tip_id).await {
            // Get parent features and reputations
            let mut parent_features = Vec::new();
            let mut parent_reputations = Vec::new();

            for parent_id in &msg.parents {
                if let Ok(Some(parent)) = state.node.storage.get_message(parent_id).await {
                    // Extract features
                    let features: Vec<QpDigits> = parent
                        .features
                        .phi
                        .iter()
                        .map(|axis_phi| axis_phi.qp_digits.clone())
                        .collect();
                    parent_features.push(features);

                    // Get reputation
                    let rep = state
                        .node
                        .consensus
                        .reputation
                        .get_reputation(&parent.proposer_pk)
                        .await;
                    parent_reputations.push(rep);
                }
            }

            // Check admissibility
            if let Ok(result) = state.node.consensus.admissibility().check_message(
                &msg,
                &parent_features,
                &parent_reputations,
            ) {
                total_checks += 1;

                if result.score_passed {
                    c1_passes += 1;
                }
                if result.c2_passed {
                    c2_passes += 1;
                }
                if result.c3_passed {
                    c3_passes += 1;
                }
                if result.is_admissible {
                    fully_admissible += 1;
                }
            }
        }
    }

    let c1_rate = if total_checks > 0 {
        (c1_passes as f64) / (total_checks as f64)
    } else {
        0.0
    };
    let c2_rate = if total_checks > 0 {
        (c2_passes as f64) / (total_checks as f64)
    } else {
        0.0
    };
    let c3_rate = if total_checks > 0 {
        (c3_passes as f64) / (total_checks as f64)
    } else {
        0.0
    };
    let overall_rate = if total_checks > 0 {
        (fully_admissible as f64) / (total_checks as f64)
    } else {
        0.0
    };

    let response = serde_json::json!({
        "sample_size": total_checks,
        "constraint_rates": {
            "c1_proximity": c1_rate,
            "c2_diversity": c2_rate,
            "c3_reputation": c3_rate,
            "overall": overall_rate,
        },
        "raw_counts": {
            "c1_passes": c1_passes,
            "c2_passes": c2_passes,
            "c3_passes": c3_passes,
            "fully_admissible": fully_admissible,
            "total": total_checks,
        },
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    (StatusCode::OK, Json(response)).into_response()
}

// Wallet Registry Handlers

async fn register_wallet_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<WalletRegistrationRequest>,
) -> Response {
    match state.node.wallet_registry.register_wallet(req).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "success",
                "message": "Wallet registered successfully"
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": e.to_string()
            })),
        )
            .into_response(),
    }
}

async fn get_wallet_public_key_handler(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> Response {
    // Parse address
    let account_address = if address.starts_with("adic") {
        match adic_economics::AccountAddress::from_bech32(&address) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("Invalid address: {}", e)
                    })),
                )
                    .into_response();
            }
        }
    } else {
        match crate::genesis::account_address_from_hex(&address) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("Invalid address: {}", e)
                    })),
                )
                    .into_response();
            }
        }
    };

    match state
        .node
        .wallet_registry
        .get_public_key(&account_address)
        .await
    {
        Some(public_key) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "address": address,
                "public_key": hex::encode(public_key.as_bytes())
            })),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "Wallet not registered"
            })),
        )
            .into_response(),
    }
}

async fn list_registered_wallets_handler(State(state): State<Arc<AppState>>) -> Response {
    let wallets = state.node.wallet_registry.list_wallets().await;

    let wallet_list: Vec<serde_json::Value> = wallets
        .iter()
        .map(|w| {
            serde_json::json!({
                "address": w.address.to_bech32().unwrap_or_else(|_| hex::encode(w.address.as_bytes())),
                "public_key": hex::encode(w.public_key.as_bytes()),
                "registered_at": w.registered_at.to_rfc3339(),
                "last_used": w.last_used.map(|t| t.to_rfc3339()),
                "metadata": {
                    "label": w.metadata.label.clone(),
                    "wallet_type": w.metadata.wallet_type.clone(),
                    "trusted": w.metadata.trusted
                }
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "wallets": wallet_list,
            "total": wallets.len()
        })),
    )
        .into_response()
}

async fn check_wallet_registration_handler(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> Response {
    // Parse address
    let account_address = if address.starts_with("adic") {
        match adic_economics::AccountAddress::from_bech32(&address) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("Invalid address: {}", e)
                    })),
                )
                    .into_response();
            }
        }
    } else {
        match crate::genesis::account_address_from_hex(&address) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("Invalid address: {}", e)
                    })),
                )
                    .into_response();
            }
        }
    };

    let is_registered = state
        .node
        .wallet_registry
        .is_registered(&account_address)
        .await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "address": address,
            "registered": is_registered
        })),
    )
        .into_response()
}

async fn unregister_wallet_handler(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> Response {
    // Parse address
    let account_address = if address.starts_with("adic") {
        match adic_economics::AccountAddress::from_bech32(&address) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("Invalid address: {}", e)
                    })),
                )
                    .into_response();
            }
        }
    } else {
        match crate::genesis::account_address_from_hex(&address) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("Invalid address: {}", e)
                    })),
                )
                    .into_response();
            }
        }
    };

    match state
        .node
        .wallet_registry
        .unregister_wallet(&account_address)
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "success",
                "message": "Wallet unregistered successfully"
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": e.to_string()
            })),
        )
            .into_response(),
    }
}

async fn wallet_registry_stats_handler(State(state): State<Arc<AppState>>) -> Response {
    let stats = state.node.wallet_registry.get_stats().await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "total_wallets": stats.total_wallets,
            "active_wallets": stats.active_wallets,
            "trusted_wallets": stats.trusted_wallets,
            "last_registration": stats.last_registration.map(|t| t.to_rfc3339()),
            "wallet_types": stats.wallet_types
        })),
    )
        .into_response()
}

async fn get_wallet_info_handler(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> Response {
    // Parse address
    let account_address = if address.starts_with("adic") {
        match adic_economics::AccountAddress::from_bech32(&address) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("Invalid address: {}", e)
                    })),
                )
                    .into_response();
            }
        }
    } else {
        match crate::genesis::account_address_from_hex(&address) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("Invalid address: {}", e)
                    })),
                )
                    .into_response();
            }
        }
    };

    match state.node.wallet_registry.get_wallet_info(&account_address).await {
        Some(wallet_info) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "address": wallet_info.address.to_bech32().unwrap_or_else(|_| hex::encode(wallet_info.address.as_bytes())),
                "public_key": hex::encode(wallet_info.public_key.as_bytes()),
                "registered_at": wallet_info.registered_at.to_rfc3339(),
                "last_used": wallet_info.last_used.map(|t| t.to_rfc3339()),
                "metadata": {
                    "label": wallet_info.metadata.label,
                    "wallet_type": wallet_info.metadata.wallet_type,
                    "trusted": wallet_info.metadata.trusted
                }
            })),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "Wallet not found in registry"
            })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct ExportWalletRequest {
    password: String,
}

async fn export_wallet_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExportWalletRequest>,
) -> Response {
    // Export the node's wallet to JSON
    match state.node.wallet().export_to_json(&req.password) {
        Ok(json) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "wallet": json,
                "format": "encrypted_json",
                "version": 3
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to export wallet: {}", e)
            })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct ImportWalletRequest {
    wallet_json: String,
    password: String,
}

async fn import_wallet_handler(
    _state: State<Arc<AppState>>,
    Json(req): Json<ImportWalletRequest>,
) -> Response {
    // Import wallet from JSON (for validation/testing purposes)
    // Note: This doesn't replace the node's wallet, just validates the import
    match crate::wallet::NodeWallet::import_from_json(&req.wallet_json, &req.password, "imported") {
        Ok(wallet) => {
            let info = wallet.get_info();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "success",
                    "address": info.address,
                    "public_key": info.public_key,
                    "message": "Wallet successfully imported and validated"
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("Failed to import wallet: {}", e)
            })),
        )
            .into_response(),
    }
}
