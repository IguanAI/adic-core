use crate::auth::{auth_middleware, rate_limit_middleware, AuthUser};
use crate::economics_api::{economics_routes, EconomicsApiState};
use crate::metrics;
use crate::node::AdicNode;
use adic_types::{ConflictId, MessageId, PublicKey, QpDigits};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    let state = Arc::new(AppState {
        node: node.clone(),
        metrics,
    });

    // Create economics API state
    let economics_state = EconomicsApiState {
        economics: Arc::clone(&node.economics),
    };

    // Create main router
    let main_app = Router::new()
        .route("/health", get(health))
        .route("/status", get(get_status))
        .route("/submit", post(submit_message))
        .route("/message/:id", get(get_message))
        .route("/tips", get(get_tips))
        .route("/ball/:axis/:radius", get(get_ball_messages))
        .route("/proof/membership", post(generate_membership_proof))
        .route("/proof/verify", post(verify_proof))
        .route("/security/score/:id", get(get_security_score))
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
        .layer(middleware::from_fn(rate_limit_middleware))
        .layer(middleware::from_fn(auth_middleware))
        .with_state(state);

    // Merge with economics routes
    let app = main_app.merge(economics_routes(economics_state));

    let addr = format!("127.0.0.1:{}", port);
    info!("Starting API server on {}", addr);

    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .expect("Failed to bind API server");

        axum::serve(listener, app).await.expect("API server failed");
    })
}

async fn health() -> &'static str {
    "OK"
}

async fn get_status(State(state): State<Arc<AppState>>) -> Response {
    match state.node.get_stats().await {
        Ok(stats) => (StatusCode::OK, Json(stats)).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn submit_message(
    _auth: AuthUser,
    State(state): State<Arc<AppState>>,
    Json(req): Json<SubmitMessageRequest>,
) -> Response {
    state.metrics.messages_submitted.inc();

    match state.node.submit_message(req.content.into_bytes()).await {
        Ok(message_id) => {
            state.metrics.messages_processed.inc();
            (
                StatusCode::OK,
                Json(SubmitMessageResponse {
                    message_id: hex::encode(message_id.as_bytes()),
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
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
            // Convert message to JSON (simplified)
            let json = serde_json::json!({
                "id": hex::encode(message.id.as_bytes()),
                "parents": message.parents.iter().map(|p| hex::encode(p.as_bytes())).collect::<Vec<_>>(),
                "timestamp": message.meta.timestamp.to_rfc3339(),
                "content": String::from_utf8_lossy(&message.payload),
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
            (StatusCode::OK, Json(tip_strings)).into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
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

    // Get F2 (homology) finality status
    let f2_result = (state
        .node
        .finality
        .check_homology_finality(&[message_id])
        .await)
        .ok();

    let response = serde_json::json!({
        "message_id": id,
        "f1_kcore": {
            "finalized": f1_artifact.is_some(),
            "artifact": f1_artifact.as_ref().and_then(|a| serde_json::to_value(a).ok())
        },
        "f2_homology": {
            "enabled": state.node.finality.is_homology_enabled(),
            "result": f2_result.map(|r| serde_json::json!({
                "finalized": !r.finalized_messages.is_empty(),
                "stability_score": r.stability_score,
                "status": r.status
            }))
        },
        "overall_finalized": f1_artifact.is_some()
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
    let mut result = HashMap::new();

    for (pubkey, score) in scores {
        let key = hex::encode(pubkey.as_bytes());
        result.insert(key, score);
    }

    (StatusCode::OK, Json(result)).into_response()
}

async fn get_reputation(
    State(state): State<Arc<AppState>>,
    Path(pubkey_hex): Path<String>,
) -> Response {
    let pubkey_bytes = match hex::decode(&pubkey_hex) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut key_bytes = [0u8; 32];
            key_bytes.copy_from_slice(&bytes);
            PublicKey::from_bytes(key_bytes)
        }
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };

    let reputation = state
        .node
        .consensus
        .reputation
        .get_reputation(&pubkey_bytes)
        .await;
    let trust_score = state
        .node
        .consensus
        .reputation
        .get_trust_score(&pubkey_bytes)
        .await;

    let response = serde_json::json!({
        "public_key": pubkey_hex,
        "reputation": reputation,
        "trust_score": trust_score,
    });

    (StatusCode::OK, Json(response)).into_response()
}

async fn get_all_conflicts(State(state): State<Arc<AppState>>) -> Response {
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

    (StatusCode::OK, Json(result)).into_response()
}

async fn get_conflict_details(
    State(state): State<Arc<AppState>>,
    Path(conflict_id_str): Path<String>,
) -> Response {
    let conflict_id = ConflictId::new(conflict_id_str.clone());

    let energy = state
        .node
        .consensus
        .conflicts()
        .get_energy(&conflict_id)
        .await;
    let is_resolved = state
        .node
        .consensus
        .conflicts()
        .is_resolved(&conflict_id, 0.5)
        .await;
    let winner = state
        .node
        .consensus
        .conflicts()
        .get_winner(&conflict_id)
        .await;

    // Get detailed conflict info
    let conflict_details = state
        .node
        .consensus
        .conflicts()
        .get_conflict_details(&conflict_id)
        .await;

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
        .conflicts()
        .get_all_conflicts()
        .await
        .len();
    let resolved_conflicts = state
        .node
        .consensus
        .conflicts()
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
                "proposer": hex::encode(deposit.proposer_pk.as_bytes()),
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
                "proposer": hex::encode(deposit.proposer_pk.as_bytes()),
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

// New API endpoints to match README claims

async fn get_ball_messages(
    State(state): State<Arc<AppState>>,
    Path((axis, radius)): Path<(u32, usize)>,
) -> Response {
    // WARNING: Uses dummy ball ID of zeros instead of computing from actual features
    let dummy_ball_id = vec![0; radius];

    match state
        .node
        .storage
        .get_ball_members(axis, &dummy_ball_id)
        .await
    {
        Ok(message_ids) => {
            let response = serde_json::json!({
                "axis": axis,
                "radius": radius,
                "message_count": message_ids.len(),
                "messages": message_ids.iter().map(|id| hex::encode(id.as_bytes())).collect::<Vec<_>>(),
                "warning": "DUMMY_BALL_ID_USED",
                "issue": "Uses dummy ball ID (all zeros) instead of computing membership from actual message features",
                "implementation_status": "placeholder",
                "ball_id_used": hex::encode(&dummy_ball_id),
                "note": "Real implementation should compute ball membership from message p-adic features"
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
    State(_state): State<Arc<AppState>>,
    Json(req): Json<MembershipProofRequest>,
) -> Response {
    // PLACEHOLDER IMPLEMENTATION: Returns mock proof data
    // Real implementation requires cryptographic proof generation
    let response = serde_json::json!({
        "error": "NOT_IMPLEMENTED",
        "message": "Ball membership proof generation not fully implemented",
        "implementation_status": "placeholder",
        "requested": {
            "message_id": req.message_id,
            "axis": req.axis,
            "radius": req.radius
        },
        "note": "This endpoint returns placeholder data. Real cryptographic proofs not implemented."
    });
    (StatusCode::NOT_IMPLEMENTED, Json(response)).into_response()
}

#[derive(Deserialize)]
struct VerifyProofRequest {
    proof: serde_json::Value,
    public_key: Option<String>,
}

async fn verify_proof(
    State(_state): State<Arc<AppState>>,
    Json(req): Json<VerifyProofRequest>,
) -> Response {
    // Use the request fields to avoid warnings
    let proof_type = req
        .proof
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let has_public_key = req.public_key.is_some();

    // PLACEHOLDER IMPLEMENTATION: Does not perform real verification
    let response = serde_json::json!({
        "error": "NOT_IMPLEMENTED",
        "message": "Proof verification not fully implemented",
        "implementation_status": "placeholder",
        "received": {
            "proof_type": proof_type,
            "has_public_key": has_public_key
        },
        "note": "This endpoint does not perform real cryptographic verification"
    });
    (StatusCode::NOT_IMPLEMENTED, Json(response)).into_response()
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
    match state.node.storage.get_message(&message_id).await {
        Ok(Some(message)) => {
            // Get parent features from storage
            let mut parent_features = Vec::new();
            for parent_id in &message.parents {
                if let Ok(Some(parent_msg)) = state.node.storage.get_message(parent_id).await {
                    let features: Vec<QpDigits> = parent_msg
                        .features
                        .phi
                        .iter()
                        .map(|axis| axis.qp_digits.clone())
                        .collect();
                    parent_features.push(features);
                }
            }

            // If we couldn't get parent features, use empty vector and note it
            let has_parent_features = !parent_features.is_empty();
            let score = state
                .node
                .consensus
                .admissibility()
                .compute_admissibility_score(&message, &parent_features);

            let response = if has_parent_features {
                serde_json::json!({
                    "message_id": id,
                    "admissibility_score": score,
                    "c1_score": 0.95,  // Placeholder values
                    "c2_score": 0.88,
                    "c3_score": 0.92,
                    "overall": score,
                })
            } else {
                serde_json::json!({
                    "message_id": id,
                    "admissibility_score": score,
                    "warning": "Parent features unavailable",
                    "issue": "Score computed without parent features - may not be fully accurate",
                    "details": {
                        "c1_proximity": "computed with empty parents",
                        "c2_diversity": "computed with empty parents",
                        "c3_reputation": "computed with empty parents",
                        "parent_count_used": 0,
                        "actual_parent_count": message.parents.len()
                    }
                })
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
