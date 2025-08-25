use crate::metrics;
use crate::node::{AdicNode, NodeStats};
use adic_mrw::MrwTrace;
use adic_types::{MessageId, PublicKey, ConflictId};
use adic_consensus::reputation::ReputationScore;
use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::collections::HashMap;
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
        .route("/v1/reputation/all", get(get_all_reputations))
        .route("/v1/reputation/:pubkey", get(get_reputation))
        .route("/v1/conflicts", get(get_all_conflicts))
        .route("/v1/conflict/:id", get(get_conflict_details))
        .route("/v1/statistics/detailed", get(get_detailed_statistics))
        .route("/v1/economics/deposits", get(get_deposits_summary))
        .route("/v1/economics/deposit/:id", get(get_deposit_status))
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

async fn get_all_reputations(
    State(state): State<Arc<AppState>>,
) -> Result<Json<HashMap<String, ReputationScore>>, StatusCode> {
    let scores = state.node.consensus.reputation.get_all_scores().await;
    let mut result = HashMap::new();
    
    for (pubkey, score) in scores {
        let key = hex::encode(pubkey.as_bytes());
        result.insert(key, score);
    }
    
    Ok(Json(result))
}

async fn get_reputation(
    State(state): State<Arc<AppState>>,
    Path(pubkey_hex): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let pubkey_bytes = match hex::decode(&pubkey_hex) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut key_bytes = [0u8; 32];
            key_bytes.copy_from_slice(&bytes);
            PublicKey::from_bytes(key_bytes)
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    
    let reputation = state.node.consensus.reputation.get_reputation(&pubkey_bytes).await;
    let trust_score = state.node.consensus.reputation.get_trust_score(&pubkey_bytes).await;
    
    let response = serde_json::json!({
        "public_key": pubkey_hex,
        "reputation": reputation,
        "trust_score": trust_score,
    });
    
    Ok(Json(response))
}

async fn get_all_conflicts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let conflicts = state.node.consensus.conflicts().get_all_conflicts().await;
    
    let mut result = Vec::new();
    for (conflict_id, energy) in conflicts {
        let conflict_json = serde_json::json!({
            "id": conflict_id.to_string(),
            "energy": energy.energy,
            "last_update": energy.last_update,
            "support_count": energy.support.len(),
            "winner": energy.get_winner().map(|id| hex::encode(id.as_bytes())),
        });
        result.push(conflict_json);
    }
    
    Ok(Json(result))
}

async fn get_conflict_details(
    State(state): State<Arc<AppState>>,
    Path(conflict_id_str): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let conflict_id = ConflictId::new(conflict_id_str.clone());
    
    let energy = state.node.consensus.conflicts().get_energy(&conflict_id).await;
    let is_resolved = state.node.consensus.conflicts().is_resolved(&conflict_id, 0.5).await;
    let winner = state.node.consensus.conflicts().get_winner(&conflict_id).await;
    
    // Get detailed conflict info
    let conflict_details = state.node.consensus.conflicts().get_conflict_details(&conflict_id).await;
    
    let response = if let Some(details) = conflict_details {
        serde_json::json!({
            "id": conflict_id_str,
            "energy": energy,
            "is_resolved": is_resolved,
            "winner": winner.map(|id| hex::encode(id.as_bytes())),
            "support": details.support.iter().map(|(msg_id, support)| {
                serde_json::json!({
                    "message_id": hex::encode(msg_id.as_bytes()),
                    "support_value": support,
                })
            }).collect::<Vec<_>>(),
            "last_update": details.last_update,
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
    
    Ok(Json(response))
}

async fn get_detailed_statistics(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Get basic node stats
    let node_stats = state.node.get_stats().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    // Get storage stats
    let storage_stats = state.node.storage.get_stats().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    // Get finality stats
    let finality_stats = state.node.finality.get_stats().await;
    
    // Get conflict stats
    let conflicts_count = state.node.consensus.conflicts().get_all_conflicts().await.len();
    let resolved_conflicts = state.node.consensus.conflicts().get_resolved_conflicts(0.5).await.len();
    
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
    
    Ok(Json(response))
}

async fn get_deposits_summary(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
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
                "proposer": hex::encode(deposit.proposer_pk.as_bytes()),
                "amount": deposit.amount,
                "status": format!("{:?}", deposit.status),
                "timestamp": deposit.timestamp,
            })
        }).collect::<Vec<_>>(),
    });
    
    Ok(Json(response))
}

async fn get_deposit_status(
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
    
    match state.node.consensus.deposits.get_deposit(&message_id).await {
        Ok(Some(deposit)) => {
            let response = serde_json::json!({
                "message_id": hex::encode(deposit.message_id.as_bytes()),
                "proposer": hex::encode(deposit.proposer_pk.as_bytes()),
                "amount": deposit.amount,
                "status": format!("{:?}", deposit.status),
                "timestamp": deposit.timestamp,
                "slashed": deposit.status == adic_consensus::DepositStatus::Slashed,
                "refunded": deposit.status == adic_consensus::DepositStatus::Refunded,
            });
            Ok(Json(response))
        }
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}
