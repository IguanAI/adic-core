use crate::metrics;
use crate::node::{AdicNode, NodeStats};
use adic_mrw::MrwTrace;
use adic_types::MessageId;
use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::info;

#[derive(Clone)]
struct AppState {
    node: AdicNode,
    metrics: metrics::Metrics,
}

#[derive(Serialize, Deserialize)]
struct SubmitMessageRequest {
    content: String,
}

#[derive(Serialize, Deserialize)]
struct SubmitMessageResponse {
    message_id: String,
}

#[derive(Serialize, Deserialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Deserialize)]
struct TracesQuery {
    limit: Option<usize>,
}

pub fn start_api_server(node: AdicNode, port: u16) -> JoinHandle<()> {
    let metrics = metrics::Metrics::new();
    let state = AppState { node, metrics };
    
    let app = Router::new()
        .route("/health", get(health))
        .route("/status", get(get_status))
        .route("/submit", post(submit_message))
        .route("/message/:id", get(get_message))
        .route("/tips", get(get_tips))
        .route("/v1/finality/:id", get(get_finality_artifact))
        .route("/v1/mrw/traces", get(get_mrw_traces))
        .route("/v1/mrw/trace/:id", get(get_mrw_trace))
        .route("/metrics", get(get_metrics))
        .with_state(Arc::new(state));
    
    let addr = format!("127.0.0.1:{}", port);
    info!("Starting API server on {}", addr);
    
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .expect("Failed to bind API server");
            
        axum::serve(listener, app)
            .await
            .expect("API server failed");
    })
}

async fn health() -> &'static str {
    "OK"
}

async fn get_status(State(state): State<Arc<AppState>>) -> Result<Json<NodeStats>, StatusCode> {
    match state.node.get_stats().await {
        Ok(stats) => Ok(Json(stats)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn submit_message(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SubmitMessageRequest>,
) -> Result<Json<SubmitMessageResponse>, (StatusCode, Json<ErrorResponse>)> {
    state.metrics.messages_submitted.inc();
    
    match state.node.submit_message(req.content.into_bytes()).await {
        Ok(message_id) => {
            state.metrics.messages_processed.inc();
            Ok(Json(SubmitMessageResponse {
                message_id: hex::encode(message_id.as_bytes()),
            }))
        }
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )),
    }
}

async fn get_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let message_id = match hex::decode(&id) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut id_bytes = [0u8; 32];
            id_bytes.copy_from_slice(&bytes);
            MessageId::from_bytes(id_bytes)
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    
    match state.node.get_message(&message_id).await {
        Ok(Some(message)) => {
            // Convert message to JSON (simplified)
            let json = serde_json::json!({
                "id": hex::encode(message.id.as_bytes()),
                "parents": message.parents.iter().map(|p| hex::encode(p.as_bytes())).collect::<Vec<_>>(),
                "timestamp": message.meta.timestamp.to_rfc3339(),
                "content": String::from_utf8_lossy(&message.payload),
            });
            Ok(Json(json))
        }
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_tips(State(state): State<Arc<AppState>>) -> Result<Json<Vec<String>>, StatusCode> {
    match state.node.get_tips().await {
        Ok(tips) => {
            let tip_strings: Vec<String> = tips
                .iter()
                .map(|id| hex::encode(id.as_bytes()))
                .collect();
            Ok(Json(tip_strings))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_finality_artifact(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let message_id = match hex::decode(&id) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut id_bytes = [0u8; 32];
            id_bytes.copy_from_slice(&bytes);
            MessageId::from_bytes(id_bytes)
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    
    match state.node.get_finality_artifact(&message_id).await {
        Some(artifact) => {
            match serde_json::to_value(&artifact) {
                Ok(json) => Ok(Json(json)),
                Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
            }
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn get_metrics(State(state): State<Arc<AppState>>) -> Result<String, StatusCode> {
    Ok(state.metrics.gather())
}

async fn get_mrw_traces(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TracesQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
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
    
    Ok(Json(response))
}

async fn get_mrw_trace(
    State(state): State<Arc<AppState>>,
    Path(trace_id): Path<String>,
) -> Result<Json<MrwTrace>, StatusCode> {
    match state.node.mrw.get_trace(&trace_id).await {
        Some(trace) => Ok(Json(trace)),
        None => Err(StatusCode::NOT_FOUND),
    }
}
