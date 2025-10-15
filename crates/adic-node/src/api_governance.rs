//! Governance API endpoints
//!
//! Provides HTTP API for governance operations:
//! - Submit proposals
//! - Cast votes
//! - Query proposals and votes
//! - Get governance parameters

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::node::AdicNode;

/// Governance API state
#[derive(Clone)]
pub struct GovernanceApiState {
    pub node: AdicNode,
}

/// Request to submit a governance proposal
#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitProposalRequest {
    /// Parameter keys to update (e.g., ["k", "delta"])
    pub param_keys: Vec<String>,
    /// New values as JSON object
    pub new_values: serde_json::Value,
    /// Proposal class: "constitutional" or "operational"
    pub class: String,
    /// IPFS CID of rationale document
    pub rationale_cid: String,
    /// Epoch at which to enact (0 = auto-calculate)
    #[serde(default)]
    pub enact_epoch: u64,
}

/// Response from submitting a proposal
#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitProposalResponse {
    /// Proposal ID (hex)
    pub proposal_id: String,
    /// Message ID of the proposal in the DAG
    pub message_id: String,
    /// Current status
    pub status: String,
}

/// Request to cast a vote
#[derive(Debug, Serialize, Deserialize)]
pub struct CastVoteRequest {
    /// Proposal ID (hex)
    pub proposal_id: String,
    /// Ballot: "yes", "no", or "abstain"
    pub ballot: String,
}

/// Response from casting a vote
#[derive(Debug, Serialize, Deserialize)]
pub struct CastVoteResponse {
    /// Message ID of the vote in the DAG
    pub message_id: String,
    /// Voting credits used
    pub credits: f64,
    /// Ballot cast
    pub ballot: String,
}

/// Query parameters for listing proposals
#[derive(Debug, Deserialize)]
pub struct ListProposalsQuery {
    /// Filter by status
    pub status: Option<String>,
    /// Pagination: offset
    #[serde(default)]
    pub offset: usize,
    /// Pagination: limit
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    50
}

/// Proposal list item
#[derive(Debug, Serialize, Deserialize)]
pub struct ProposalListItem {
    pub proposal_id: String,
    pub class: String,
    pub param_keys: Vec<String>,
    pub status: String,
    pub tally_yes: f64,
    pub tally_no: f64,
    pub tally_abstain: f64,
    pub creation_timestamp: String,
    pub voting_end_timestamp: String,
}

/// Detailed proposal information
#[derive(Debug, Serialize, Deserialize)]
pub struct ProposalDetails {
    pub proposal_id: String,
    pub class: String,
    pub proposer_pk: String,
    pub param_keys: Vec<String>,
    pub new_values: serde_json::Value,
    pub enact_epoch: u64,
    pub rationale_cid: String,
    pub status: String,
    pub tally_yes: f64,
    pub tally_no: f64,
    pub tally_abstain: f64,
    pub creation_timestamp: String,
    pub voting_end_timestamp: String,
}

/// Governance parameters response
#[derive(Debug, Serialize, Deserialize)]
pub struct GovernanceParametersResponse {
    pub rmax: f64,
    pub min_quorum: f64,
    pub voting_duration_secs: i64,
    pub min_proposer_reputation: f64,
    pub gamma_f1: f64,
    pub gamma_f2: f64,
    pub min_timelock_secs: f64,
    pub committee_size: usize,
    pub bls_threshold: usize,
}

/// Error response
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Create governance routes
pub fn governance_routes(state: GovernanceApiState) -> Router {
    Router::new()
        .route("/gov/proposals", post(submit_proposal))
        .route("/gov/proposals", get(list_proposals))
        .route("/gov/proposals/:id", get(get_proposal))
        .route("/gov/votes", post(cast_vote))
        .route("/gov/proposals/:id/votes", get(get_proposal_votes))
        .route("/gov/parameters", get(get_parameters))
        .route("/gov/receipts/:id", get(get_receipts))
        .with_state(Arc::new(state))
}

/// Submit a governance proposal
async fn submit_proposal(
    State(state): State<Arc<GovernanceApiState>>,
    Json(req): Json<SubmitProposalRequest>,
) -> impl IntoResponse {
    debug!(
        params = ?req.param_keys,
        class = %req.class,
        "Submitting governance proposal"
    );

    // Check if governance is enabled
    if state.node.governance_manager.is_none() {
        warn!("Governance not enabled on this node");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Governance not enabled on this node".to_string(),
            }),
        )
            .into_response();
    }

    // Parse proposal class
    let class = match req.class.to_lowercase().as_str() {
        "constitutional" => adic_types::ProposalClass::Constitutional,
        "operational" => adic_types::ProposalClass::Operational,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid proposal class: {}. Must be 'constitutional' or 'operational'",
                        req.class
                    ),
                }),
            )
                .into_response();
        }
    };

    // Submit proposal via node
    match state
        .node
        .submit_governance_proposal(
            req.param_keys,
            req.new_values,
            class,
            req.rationale_cid,
            req.enact_epoch,
        )
        .await
    {
        Ok((proposal_id, message_id)) => {
            info!(
                proposal_id = hex::encode(&proposal_id[..8]),
                message_id = ?message_id,
                "✅ Proposal submitted successfully"
            );

            (
                StatusCode::CREATED,
                Json(SubmitProposalResponse {
                    proposal_id: hex::encode(proposal_id),
                    message_id: format!("{:?}", message_id),
                    status: "Voting".to_string(),
                }),
            )
                .into_response()
        }
        Err(e) => {
            warn!(error = %e, "Failed to submit proposal");
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Failed to submit proposal: {}", e),
                }),
            )
                .into_response()
        }
    }
}

/// List all proposals
async fn list_proposals(
    State(state): State<Arc<GovernanceApiState>>,
    Query(query): Query<ListProposalsQuery>,
) -> impl IntoResponse {
    debug!(
        status = ?query.status,
        offset = query.offset,
        limit = query.limit,
        "Listing proposals"
    );

    if state.node.governance_manager.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Governance not enabled on this node".to_string(),
            }),
        )
            .into_response();
    }

    let gov_manager = state.node.governance_manager.as_ref().unwrap();

    // Get proposals (filtered by status if provided)
    let all_proposals = if let Some(status_str) = &query.status {
        // Parse status filter
        use adic_governance::types::ProposalStatus;
        let status = match status_str.to_lowercase().as_str() {
            "voting" => ProposalStatus::Voting,
            "tallying" => ProposalStatus::Tallying,
            "succeeded" => ProposalStatus::Succeeded,
            "enacting" => ProposalStatus::Enacting,
            "rejected" => ProposalStatus::Rejected,
            "enacted" => ProposalStatus::Enacted,
            "failed" => ProposalStatus::Failed,
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid status: {}", status_str),
                    }),
                )
                    .into_response();
            }
        };
        gov_manager.get_proposals_by_status(status).await
    } else {
        gov_manager.get_all_proposals().await
    };

    // Apply pagination
    let total = all_proposals.len();
    let proposals: Vec<ProposalListItem> = all_proposals
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .map(|p| ProposalListItem {
            proposal_id: hex::encode(p.proposal_id),
            class: format!("{:?}", p.class),
            param_keys: p.param_keys,
            status: format!("{:?}", p.status),
            tally_yes: p.tally_yes,
            tally_no: p.tally_no,
            tally_abstain: p.tally_abstain,
            creation_timestamp: p.creation_timestamp.to_rfc3339(),
            voting_end_timestamp: p.voting_end_timestamp.to_rfc3339(),
        })
        .collect();

    debug!(
        count = proposals.len(),
        total = total,
        "Retrieved proposals"
    );

    (StatusCode::OK, Json(proposals)).into_response()
}

/// Get proposal details
async fn get_proposal(
    State(state): State<Arc<GovernanceApiState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    debug!(proposal_id = %id, "Getting proposal details");

    if state.node.governance_manager.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Governance not enabled on this node".to_string(),
            }),
        )
            .into_response();
    }

    // Parse proposal ID from hex
    let proposal_id_bytes = match hex::decode(&id) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            arr
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid proposal ID format (must be 64-char hex)".to_string(),
                }),
            )
                .into_response();
        }
    };

    let gov_manager = state.node.governance_manager.as_ref().unwrap();

    // Get proposal
    match gov_manager.get_proposal(&proposal_id_bytes).await {
        Some(proposal) => {
            let details = ProposalDetails {
                proposal_id: hex::encode(proposal.proposal_id),
                class: format!("{:?}", proposal.class),
                proposer_pk: hex::encode(proposal.proposer_pk.as_bytes()),
                param_keys: proposal.param_keys,
                new_values: proposal.new_values,
                enact_epoch: proposal.enact_epoch,
                rationale_cid: proposal.rationale_cid,
                status: format!("{:?}", proposal.status),
                tally_yes: proposal.tally_yes,
                tally_no: proposal.tally_no,
                tally_abstain: proposal.tally_abstain,
                creation_timestamp: proposal.creation_timestamp.to_rfc3339(),
                voting_end_timestamp: proposal.voting_end_timestamp.to_rfc3339(),
            };

            (StatusCode::OK, Json(details)).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Proposal {} not found", id),
            }),
        )
            .into_response(),
    }
}

/// Cast a vote on a proposal
async fn cast_vote(
    State(state): State<Arc<GovernanceApiState>>,
    Json(req): Json<CastVoteRequest>,
) -> impl IntoResponse {
    debug!(
        proposal_id = %req.proposal_id,
        ballot = %req.ballot,
        "Casting vote"
    );

    if state.node.governance_manager.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Governance not enabled on this node".to_string(),
            }),
        )
            .into_response();
    }

    // Parse proposal ID from hex
    let proposal_id_bytes = match hex::decode(&req.proposal_id) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            arr
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid proposal ID format (must be 64-char hex)".to_string(),
                }),
            )
                .into_response();
        }
    };

    // Parse ballot
    let ballot = match req.ballot.to_lowercase().as_str() {
        "yes" => adic_types::Ballot::Yes,
        "no" => adic_types::Ballot::No,
        "abstain" => adic_types::Ballot::Abstain,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid ballot: {}. Must be 'yes', 'no', or 'abstain'",
                        req.ballot
                    ),
                }),
            )
                .into_response();
        }
    };

    // Cast vote via node
    match state
        .node
        .vote_on_proposal(proposal_id_bytes, ballot)
        .await
    {
        Ok(message_id) => {
            // Get voter reputation to compute credits
            let reputation = state
                .node
                .consensus
                .reputation
                .get_reputation(&state.node.public_key())
                .await;
            let rmax = 100_000.0; // Should come from config
            let credits = (reputation.min(rmax)).sqrt();

            info!(
                proposal_id = &req.proposal_id[..16],
                ballot = %req.ballot,
                credits = credits,
                "✅ Vote cast successfully"
            );

            (
                StatusCode::CREATED,
                Json(CastVoteResponse {
                    message_id: format!("{:?}", message_id),
                    credits,
                    ballot: req.ballot,
                }),
            )
                .into_response()
        }
        Err(e) => {
            warn!(error = %e, "Failed to cast vote");
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Failed to cast vote: {}", e),
                }),
            )
                .into_response()
        }
    }
}

/// Get votes for a proposal
async fn get_proposal_votes(
    State(state): State<Arc<GovernanceApiState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    debug!(proposal_id = %id, "Getting proposal votes");

    if state.node.governance_manager.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Governance not enabled on this node".to_string(),
            }),
        )
            .into_response();
    }

    // Parse proposal ID from hex
    let proposal_id_bytes = match hex::decode(&id) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            arr
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid proposal ID format (must be 64-char hex)".to_string(),
                }),
            )
                .into_response();
        }
    };

    let gov_manager = state.node.governance_manager.as_ref().unwrap();

    // Get votes for this proposal
    let votes = gov_manager.get_votes(&proposal_id_bytes).await;

    let vote_list: Vec<serde_json::Value> = votes
        .into_iter()
        .map(|v| {
            serde_json::json!({
                "voter_pk": hex::encode(v.voter_pk.as_bytes()),
                "ballot": format!("{:?}", v.ballot),
                "credits": v.credits,
                "timestamp": v.timestamp.to_rfc3339(),
            })
        })
        .collect();

    (StatusCode::OK, Json(vote_list)).into_response()
}

/// Get governance parameters
async fn get_parameters(
    State(state): State<Arc<GovernanceApiState>>,
) -> impl IntoResponse {
    debug!("Getting governance parameters");

    if state.node.governance_manager.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Governance not enabled on this node".to_string(),
            }),
        )
            .into_response();
    }

    let gov_manager = state.node.governance_manager.as_ref().unwrap();
    let config = gov_manager.config();

    let response = GovernanceParametersResponse {
        rmax: config.rmax,
        min_quorum: config.min_quorum,
        voting_duration_secs: config.voting_duration_secs,
        min_proposer_reputation: config.min_proposer_reputation,
        gamma_f1: config.gamma_f1,
        gamma_f2: config.gamma_f2,
        min_timelock_secs: config.min_timelock_secs,
        committee_size: config.committee_size,
        bls_threshold: config.bls_threshold,
    };

    (StatusCode::OK, Json(response)).into_response()
}

/// Get governance receipts for a proposal
async fn get_receipts(
    State(state): State<Arc<GovernanceApiState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    debug!(proposal_id = %id, "Getting governance receipts");

    if state.node.governance_manager.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Governance not enabled on this node".to_string(),
            }),
        )
            .into_response();
    }

    // Parse proposal ID from hex
    let proposal_id_bytes = match hex::decode(&id) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            arr
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid proposal ID format (must be 64-char hex)".to_string(),
                }),
            )
                .into_response();
        }
    };

    let gov_manager = state.node.governance_manager.as_ref().unwrap();

    // Get receipts for this proposal
    let receipts = gov_manager.get_receipts(&proposal_id_bytes).await;

    let receipt_list: Vec<serde_json::Value> = receipts
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "proposal_id": hex::encode(r.proposal_id),
                "result": format!("{:?}", r.result),
                "receipt_seq": r.receipt_seq,
                "quorum_stats": {
                    "yes": r.quorum_stats.yes,
                    "no": r.quorum_stats.no,
                    "abstain": r.quorum_stats.abstain,
                    "total_participation": r.quorum_stats.total_participation,
                },
                "committee_members": r.committee_members.iter().map(|pk| hex::encode(pk.as_bytes())).collect::<Vec<_>>(),
                "timestamp": r.timestamp.to_rfc3339(),
            })
        })
        .collect();

    (StatusCode::OK, Json(receipt_list)).into_response()
}
