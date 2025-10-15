//! Storage Market API endpoints
//!
//! Provides HTTP API for storage market operations:
//! - Publish and query storage intents
//! - Submit and query provider acceptances
//! - Compile and manage storage deals
//! - Submit and verify storage proofs
//! - Query market statistics

use crate::node::AdicNode;
use adic_economics::{AccountAddress, AdicAmount};
use adic_storage_market::{
    DealActivation, FinalityStatus, MerkleProof as StorageMerkleProof, ProviderAcceptance,
    SettlementRail, StorageDeal, StorageDealIntent, StorageDealStatus, StorageProof,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use tracing::info;

/// Storage Market API state
#[derive(Clone)]
pub struct StorageMarketApiState {
    pub node: AdicNode,
}

// ============================================================================
// Request/Response Types - Intent Operations
// ============================================================================

/// Request to publish a storage deal intent
#[derive(Debug, Serialize, Deserialize)]
pub struct PublishIntentRequest {
    /// Data CID (hex-encoded, 32 bytes)
    pub data_cid: String,
    /// Data size in bytes
    pub data_size: u64,
    /// Storage duration in epochs
    pub duration_epochs: u64,
    /// Maximum price per epoch (ADIC)
    pub max_price_per_epoch: f64,
    /// Required redundancy (number of providers)
    pub required_redundancy: u8,
    /// Settlement rails (e.g., ["adic", "filecoin"])
    pub settlement_rails: Vec<String>,
}

/// Response from publishing an intent
#[derive(Debug, Serialize, Deserialize)]
pub struct PublishIntentResponse {
    /// Intent ID (hex)
    pub intent_id: String,
    /// Current status
    pub status: String,
    /// Message
    pub message: String,
}

/// Intent list item
#[derive(Debug, Serialize, Deserialize)]
pub struct IntentListItem {
    pub intent_id: String,
    pub client: String,
    pub data_size: u64,
    pub duration_epochs: u64,
    pub max_price_per_epoch: String,
    pub required_redundancy: u8,
    pub status: String,
    pub created_at: i64,
    pub expires_at: i64,
}

/// Detailed intent information
#[derive(Debug, Serialize, Deserialize)]
pub struct IntentDetails {
    pub intent_id: String,
    pub client: String,
    pub data_cid: String,
    pub data_size: u64,
    pub duration_epochs: u64,
    pub max_price_per_epoch: String,
    pub required_redundancy: u8,
    pub settlement_rails: Vec<String>,
    pub status: String,
    pub finalized_at_epoch: Option<u64>,
    pub created_at: i64,
    pub expires_at: i64,
}

// ============================================================================
// Request/Response Types - Acceptance Operations
// ============================================================================

/// Request to submit provider acceptance
#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitAcceptanceRequest {
    /// Intent ID to accept (hex)
    pub intent_id: String,
    /// Price per epoch (ADIC)
    pub price_per_epoch: f64,
    /// Collateral commitment (ADIC)
    pub collateral: f64,
}

/// Response from submitting acceptance
#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitAcceptanceResponse {
    /// Acceptance ID (hex)
    pub acceptance_id: String,
    /// Status
    pub status: String,
    /// Message
    pub message: String,
}

/// Acceptance list item
#[derive(Debug, Serialize, Deserialize)]
pub struct AcceptanceListItem {
    pub acceptance_id: String,
    pub intent_id: String,
    pub provider: String,
    pub price_per_epoch: String,
    pub collateral: String,
    pub status: String,
    pub created_at: i64,
}

// ============================================================================
// Request/Response Types - Deal Operations
// ============================================================================

/// Request to compile a deal
#[derive(Debug, Serialize, Deserialize)]
pub struct CompileDealRequest {
    /// Intent ID (hex)
    pub intent_id: String,
    /// Acceptance ID (hex)
    pub acceptance_id: String,
}

/// Response from compiling a deal
#[derive(Debug, Serialize, Deserialize)]
pub struct CompileDealResponse {
    /// Deal ID
    pub deal_id: u64,
    /// Status
    pub status: String,
    /// Message
    pub message: String,
}

/// Request to activate a deal
#[derive(Debug, Serialize, Deserialize)]
pub struct ActivateDealRequest {
    /// Merkle root of stored data (hex, 32 bytes)
    pub merkle_root: String,
    /// Number of chunks
    pub chunk_count: u64,
}

/// Response from activating a deal
#[derive(Debug, Serialize, Deserialize)]
pub struct ActivateDealResponse {
    /// Deal ID
    pub deal_id: u64,
    /// Status
    pub status: String,
    /// Message
    pub message: String,
}

/// Deal list item
#[derive(Debug, Serialize, Deserialize)]
pub struct DealListItem {
    pub deal_id: u64,
    pub client: String,
    pub provider: String,
    pub data_size: u64,
    pub price_per_epoch: String,
    pub provider_collateral: String,
    pub status: String,
    pub start_epoch: Option<u64>,
    pub duration_epochs: u64,
}

/// Detailed deal information
#[derive(Debug, Serialize, Deserialize)]
pub struct DealDetails {
    pub deal_id: u64,
    pub client: String,
    pub provider: String,
    pub data_cid: String,
    pub data_size: u64,
    pub duration_epochs: u64,
    pub price_per_epoch: String,
    pub provider_collateral: String,
    pub client_payment_escrow: String,
    pub status: String,
    pub start_epoch: Option<u64>,
    pub activation_deadline: u64,
    pub proof_merkle_root: Option<String>,
}

// ============================================================================
// Request/Response Types - Proof Operations
// ============================================================================

/// Request to submit a storage proof
#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitProofRequest {
    /// Deal ID
    pub deal_id: u64,
    /// Proof epoch
    pub proof_epoch: u64,
    /// Challenge indices
    pub challenge_indices: Vec<u64>,
    /// Merkle proofs (simplified - in production would be full MerkleProof structs)
    pub merkle_proofs: Vec<MerkleProofData>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MerkleProofData {
    pub chunk_index: u64,
    pub chunk_data: String, // hex-encoded
    pub sibling_hashes: Vec<String>, // hex-encoded
}

/// Response from submitting a proof
#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitProofResponse {
    /// Success
    pub success: bool,
    /// Message
    pub message: String,
}

/// Challenge data for an epoch
#[derive(Debug, Serialize, Deserialize)]
pub struct ChallengeResponse {
    pub deal_id: u64,
    pub epoch: u64,
    pub challenge_indices: Vec<u64>,
}

// ============================================================================
// Request/Response Types - Fraud Proof Operations
// ============================================================================

/// Request to submit a fraud proof challenging a storage proof
#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitFraudProofRequest {
    /// Proof ID (hex-encoded MessageId)
    pub proof_id: String,
    /// Challenger address (hex-encoded, 32 bytes)
    pub challenger: String,
    /// Evidence type ("invalid_merkle_proof", "missing_data", "incorrect_challenge")
    pub evidence_type: String,
    /// Evidence data (hex-encoded)
    pub evidence_data: String,
    /// Submitted at epoch
    pub submitted_at_epoch: u64,
}

/// Response from submitting a fraud proof
#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitFraudProofResponse {
    /// Fraud proof ID (hex-encoded)
    pub fraud_proof_id: String,
    /// Success message
    pub message: String,
}

/// Fraud proof data
#[derive(Debug, Serialize, Deserialize)]
pub struct FraudProofData {
    /// Fraud proof ID (hex-encoded)
    pub id: String,
    /// Target proof ID (hex-encoded)
    pub proof_id: String,
    /// Challenger address (hex-encoded)
    pub challenger: String,
    /// Evidence type
    pub evidence_type: String,
    /// Evidence data (hex-encoded)
    pub evidence_data: String,
    /// Submitted at epoch
    pub submitted_at_epoch: u64,
}

/// Response listing fraud proofs
#[derive(Debug, Serialize, Deserialize)]
pub struct ListFraudProofsResponse {
    pub fraud_proofs: Vec<FraudProofData>,
    pub total_count: usize,
}

// ============================================================================
// Request/Response Types - Statistics
// ============================================================================

/// Market statistics
#[derive(Debug, Serialize, Deserialize)]
pub struct MarketStatsResponse {
    pub total_intents: usize,
    pub finalized_intents: usize,
    pub total_acceptances: usize,
    pub compiled_deals: usize,
    pub active_deals: usize,
    pub pending_activation: usize,
    pub completed_deals: usize,
    pub failed_deals: usize,
    pub current_epoch: u64,
}

/// Provider statistics
#[derive(Debug, Serialize, Deserialize)]
pub struct ProviderStatsResponse {
    pub provider_address: String,
    pub total_acceptances: usize,
    pub active_deals: usize,
    pub completed_deals: usize,
    pub failed_deals: usize,
    pub total_storage_provided: u64,
    pub total_payments_received: String,
}

/// Client statistics
#[derive(Debug, Serialize, Deserialize)]
pub struct ClientStatsResponse {
    pub client_address: String,
    pub total_intents: usize,
    pub active_deals: usize,
    pub completed_deals: usize,
    pub total_storage_used: u64,
    pub total_payments_made: String,
}

// ============================================================================
// Query Parameters
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct IntentListQuery {
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
    pub client: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AcceptanceListQuery {
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
    pub provider: Option<String>,
    pub intent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DealListQuery {
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
    pub client: Option<String>,
    pub provider: Option<String>,
    pub status: Option<String>,
}

fn default_limit() -> usize {
    50
}

// ============================================================================
// Error Response
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::BAD_REQUEST, Json(self)).into_response()
    }
}

// ============================================================================
// API Routes
// ============================================================================

/// Create storage market routes
pub fn storage_market_routes(state: StorageMarketApiState) -> Router {
    Router::new()
        // Intent operations
        .route("/storage/intents", post(publish_intent))
        .route("/storage/intents", get(list_intents))
        .route("/storage/intents/:id", get(get_intent))
        // Acceptance operations
        .route("/storage/acceptances", post(submit_acceptance))
        .route("/storage/acceptances", get(list_acceptances))
        .route("/storage/acceptances/:id", get(get_acceptance))
        // Deal operations
        .route("/storage/deals/compile", post(compile_deal))
        .route("/storage/deals/:id/activate", post(activate_deal))
        .route("/storage/deals", get(list_deals))
        .route("/storage/deals/:id", get(get_deal))
        // Proof operations
        .route("/storage/proofs", post(submit_proof))
        .route("/storage/challenges/:deal_id/:epoch", get(get_challenges))
        // Fraud proof operations
        .route("/storage/fraud-proofs", post(submit_fraud_proof))
        .route("/storage/fraud-proofs", get(list_fraud_proofs))
        .route("/storage/fraud-proofs/:id", get(get_fraud_proof))
        // Statistics
        .route("/storage/stats", get(get_market_stats))
        .route("/storage/stats/provider/:address", get(get_provider_stats))
        .route("/storage/stats/client/:address", get(get_client_stats))
        .with_state(state)
}

// ============================================================================
// Intent Endpoint Handlers
// ============================================================================

/// Publish a storage deal intent
async fn publish_intent(
    State(state): State<StorageMarketApiState>,
    Json(req): Json<PublishIntentRequest>,
) -> Result<Json<PublishIntentResponse>, ErrorResponse> {
    info!("Publishing storage intent: {} bytes", req.data_size);

    // Check if storage market is enabled
    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled on this node".to_string(),
        })?;

    // Parse data CID
    let data_cid = hex::decode(&req.data_cid)
        .map_err(|e| ErrorResponse {
            error: format!("Invalid data_cid hex: {}", e),
        })?
        .try_into()
        .map_err(|_| ErrorResponse {
            error: "data_cid must be 32 bytes".to_string(),
        })?;

    // Parse settlement rails
    let rails: Vec<SettlementRail> = req
        .settlement_rails
        .iter()
        .map(|s| match s.as_str() {
            "adic" => Ok(SettlementRail::AdicNative),
            "filecoin" => Ok(SettlementRail::Filecoin),
            "arweave" => Ok(SettlementRail::Arweave),
            other => Ok(SettlementRail::Fiat(other.to_string())),
        })
        .collect::<Result<Vec<_>, ErrorResponse>>()?;

    // Create intent
    let client = state.node.wallet().address();
    let mut intent = StorageDealIntent::new(
        client,
        data_cid,
        req.data_size,
        req.duration_epochs,
        AdicAmount::from_adic(req.max_price_per_epoch),
        req.required_redundancy,
        rails,
    );

    // Set deposit (0.1 ADIC standard anti-spam deposit)
    intent.deposit = AdicAmount::from_adic(0.1);

    // Publish intent
    let intent_id = coordinator
        .publish_intent(intent)
        .await
        .map_err(|e| ErrorResponse {
            error: format!("Failed to publish intent: {}", e),
        })?;

    Ok(Json(PublishIntentResponse {
        intent_id: hex::encode(intent_id),
        status: "published".to_string(),
        message: "Intent published successfully".to_string(),
    }))
}

/// List storage intents
async fn list_intents(
    State(state): State<StorageMarketApiState>,
    Query(query): Query<IntentListQuery>,
) -> Result<Json<Vec<IntentListItem>>, ErrorResponse> {
    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    // Get all intents from coordinator
    let all_intents = coordinator.get_finalized_intents().await;

    // Apply filters
    let mut filtered: Vec<_> = all_intents;

    // Filter by client if specified
    if let Some(client_hex) = &query.client {
        let client = parse_account_address(client_hex)?;
        filtered.retain(|intent| intent.client == client);
    }

    // Filter by status if specified
    if let Some(status_str) = &query.status {
        let is_finalized = status_str.to_lowercase() == "finalized";
        filtered.retain(|intent| intent.finality_status.is_finalized() == is_finalized);
    }

    // Apply pagination
    let items: Vec<IntentListItem> = filtered
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .map(|intent| IntentListItem {
            intent_id: hex::encode(intent.intent_id),
            client: hex::encode(intent.client.as_bytes()),
            data_size: intent.data_size,
            duration_epochs: intent.duration_epochs,
            max_price_per_epoch: intent.max_price_per_epoch.to_string(),
            required_redundancy: intent.required_redundancy,
            status: format!("{:?}", intent.finality_status),
            created_at: intent.timestamp,
            expires_at: intent.expires_at,
        })
        .collect();

    Ok(Json(items))
}

/// Get intent details
async fn get_intent(
    State(state): State<StorageMarketApiState>,
    Path(id): Path<String>,
) -> Result<Json<IntentDetails>, ErrorResponse> {
    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    let intent_id: [u8; 32] = hex::decode(&id)
        .map_err(|e| ErrorResponse {
            error: format!("Invalid intent_id: {}", e),
        })?
        .try_into()
        .map_err(|_| ErrorResponse {
            error: "intent_id must be 32 bytes".to_string(),
        })?;

    let intent = coordinator
        .get_intent(&intent_id)
        .await
        .ok_or_else(|| ErrorResponse {
            error: format!("Intent {} not found", id),
        })?;

    Ok(Json(IntentDetails {
        intent_id: hex::encode(intent.intent_id),
        client: hex::encode(intent.client.as_bytes()),
        data_cid: hex::encode(intent.data_cid),
        data_size: intent.data_size,
        duration_epochs: intent.duration_epochs,
        max_price_per_epoch: intent.max_price_per_epoch.to_string(),
        required_redundancy: intent.required_redundancy,
        settlement_rails: intent
            .target_rails
            .iter()
            .map(|r| format!("{:?}", r))
            .collect(),
        status: format!("{:?}", intent.finality_status),
        finalized_at_epoch: intent.finalized_at_epoch,
        created_at: intent.timestamp,
        expires_at: intent.expires_at,
    }))
}

// ============================================================================
// Acceptance Endpoint Handlers
// ============================================================================

/// Submit provider acceptance
async fn submit_acceptance(
    State(state): State<StorageMarketApiState>,
    Json(req): Json<SubmitAcceptanceRequest>,
) -> Result<Json<SubmitAcceptanceResponse>, ErrorResponse> {
    info!("Submitting provider acceptance for intent {}", req.intent_id);

    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    // Parse intent ID
    let intent_id: [u8; 32] = hex::decode(&req.intent_id)
        .map_err(|e| ErrorResponse {
            error: format!("Invalid intent_id: {}", e),
        })?
        .try_into()
        .map_err(|_| ErrorResponse {
            error: "intent_id must be 32 bytes".to_string(),
        })?;

    // Create acceptance
    let provider = state.node.wallet().address();

    // Get actual provider reputation
    let provider_pk = state.node.wallet().keypair().public_key();
    let provider_reputation = state
        .node
        .consensus
        .reputation
        .get_reputation(provider_pk)
        .await;

    let mut acceptance = ProviderAcceptance::new(
        intent_id,
        provider,
        AdicAmount::from_adic(req.price_per_epoch),
        AdicAmount::from_adic(req.collateral),
        provider_reputation,
    );
    acceptance.deposit = AdicAmount::from_adic(0.1);

    // Submit acceptance
    let acceptance_id = coordinator
        .submit_acceptance(acceptance)
        .await
        .map_err(|e| ErrorResponse {
            error: format!("Failed to submit acceptance: {}", e),
        })?;

    Ok(Json(SubmitAcceptanceResponse {
        acceptance_id: hex::encode(acceptance_id),
        status: "submitted".to_string(),
        message: "Acceptance submitted successfully".to_string(),
    }))
}

/// List acceptances
async fn list_acceptances(
    State(state): State<StorageMarketApiState>,
    Query(query): Query<AcceptanceListQuery>,
) -> Result<Json<Vec<AcceptanceListItem>>, ErrorResponse> {
    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    // Get acceptances for a specific intent if provided
    let all_acceptances = if let Some(intent_id_hex) = &query.intent_id {
        let intent_id: [u8; 32] = hex::decode(intent_id_hex)
            .map_err(|e| ErrorResponse {
                error: format!("Invalid intent_id: {}", e),
            })?
            .try_into()
            .map_err(|_| ErrorResponse {
                error: "intent_id must be 32 bytes".to_string(),
            })?;

        coordinator
            .get_acceptances_for_intent(&intent_id)
            .await
            .unwrap_or_else(|_| vec![])
    } else {
        // Get all acceptances (need to read directly from storage)
        // For now, return empty - would need to add a get_all_acceptances method
        vec![]
    };

    // Apply filters
    let filtered: Vec<_> = all_acceptances;

    // Apply pagination and filtering
    let items: Vec<AcceptanceListItem> = filtered
        .into_iter()
        .filter(|acceptance| {
            if let Some(provider_hex) = &query.provider {
                if let Ok(provider) = parse_account_address(provider_hex) {
                    acceptance.provider == provider
                } else {
                    false
                }
            } else {
                true
            }
        })
        .skip(query.offset)
        .take(query.limit)
        .map(|acceptance| AcceptanceListItem {
            acceptance_id: hex::encode(acceptance.acceptance_id),
            intent_id: hex::encode(acceptance.ref_intent),
            provider: hex::encode(acceptance.provider.as_bytes()),
            price_per_epoch: acceptance.offered_price_per_epoch.to_string(),
            collateral: acceptance.collateral_commitment.to_string(),
            status: format!("{:?}", acceptance.finality_status),
            created_at: acceptance.timestamp,
        })
        .collect();

    Ok(Json(items))
}

/// Get acceptance details
async fn get_acceptance(
    State(state): State<StorageMarketApiState>,
    Path(id): Path<String>,
) -> Result<Json<ProviderAcceptance>, ErrorResponse> {
    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    let acceptance_id: [u8; 32] = hex::decode(&id)
        .map_err(|e| ErrorResponse {
            error: format!("Invalid acceptance_id: {}", e),
        })?
        .try_into()
        .map_err(|_| ErrorResponse {
            error: "acceptance_id must be 32 bytes".to_string(),
        })?;

    let acceptance = coordinator
        .get_acceptance(&acceptance_id)
        .await
        .ok_or_else(|| ErrorResponse {
            error: format!("Acceptance {} not found", id),
        })?;

    Ok(Json(acceptance))
}

// ============================================================================
// Deal Endpoint Handlers
// ============================================================================

/// Compile a deal from intent and acceptance
async fn compile_deal(
    State(state): State<StorageMarketApiState>,
    Json(req): Json<CompileDealRequest>,
) -> Result<Json<CompileDealResponse>, ErrorResponse> {
    info!(
        "Compiling deal from intent {} and acceptance {}",
        req.intent_id, req.acceptance_id
    );

    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    let intent_id: [u8; 32] = hex::decode(&req.intent_id)
        .map_err(|e| ErrorResponse {
            error: format!("Invalid intent_id: {}", e),
        })?
        .try_into()
        .map_err(|_| ErrorResponse {
            error: "intent_id must be 32 bytes".to_string(),
        })?;

    let acceptance_id: [u8; 32] = hex::decode(&req.acceptance_id)
        .map_err(|e| ErrorResponse {
            error: format!("Invalid acceptance_id: {}", e),
        })?
        .try_into()
        .map_err(|_| ErrorResponse {
            error: "acceptance_id must be 32 bytes".to_string(),
        })?;

    let deal_id = coordinator
        .compile_deal(&intent_id, &acceptance_id)
        .await
        .map_err(|e| ErrorResponse {
            error: format!("Failed to compile deal: {}", e),
        })?;

    Ok(Json(CompileDealResponse {
        deal_id,
        status: "compiled".to_string(),
        message: "Deal compiled successfully, pending activation".to_string(),
    }))
}

/// Activate a deal
async fn activate_deal(
    State(state): State<StorageMarketApiState>,
    Path(deal_id): Path<u64>,
    Json(req): Json<ActivateDealRequest>,
) -> Result<Json<ActivateDealResponse>, ErrorResponse> {
    info!("Activating deal {}", deal_id);

    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    let merkle_root: [u8; 32] = hex::decode(&req.merkle_root)
        .map_err(|e| ErrorResponse {
            error: format!("Invalid merkle_root: {}", e),
        })?
        .try_into()
        .map_err(|_| ErrorResponse {
            error: "merkle_root must be 32 bytes".to_string(),
        })?;

    let provider = state.node.wallet().address();
    let activation = DealActivation {
        approvals: [[0u8; 32]; 4],
        features: [0; 3],
        signature: adic_types::Signature::empty(),
        deposit: AdicAmount::ZERO,
        timestamp: chrono::Utc::now().timestamp(),
        ref_deal: deal_id,
        provider,
        data_merkle_root: merkle_root,
        chunk_count: req.chunk_count,
        activated_at_epoch: coordinator.get_current_epoch().await,
        finality_status: FinalityStatus::Pending,
        finalized_at_epoch: None,
    };

    coordinator
        .activate_deal(deal_id, activation)
        .await
        .map_err(|e| ErrorResponse {
            error: format!("Failed to activate deal: {}", e),
        })?;

    Ok(Json(ActivateDealResponse {
        deal_id,
        status: "active".to_string(),
        message: "Deal activated successfully".to_string(),
    }))
}

/// List deals
async fn list_deals(
    State(state): State<StorageMarketApiState>,
    Query(query): Query<DealListQuery>,
) -> Result<Json<Vec<DealListItem>>, ErrorResponse> {
    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    // Get deals based on filters
    let deals: Vec<StorageDeal> = if let Some(client_hex) = &query.client {
        let client = parse_account_address(client_hex)?;
        coordinator.get_client_deals(client).await
    } else if let Some(provider_hex) = &query.provider {
        let provider = parse_account_address(provider_hex)?;
        coordinator.get_provider_deals(provider).await
    } else {
        // Return all deals (would need a new method on coordinator)
        vec![]
    };

    // Filter by status if specified
    let filtered: Vec<_> = if let Some(status_str) = &query.status {
        let status = parse_deal_status(status_str)?;
        deals.into_iter().filter(|d| d.status == status).collect()
    } else {
        deals
    };

    // Apply pagination
    let items: Vec<DealListItem> = filtered
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .map(|deal| DealListItem {
            deal_id: deal.deal_id,
            client: hex::encode(deal.client.as_bytes()),
            provider: hex::encode(deal.provider.as_bytes()),
            data_size: deal.data_size,
            price_per_epoch: deal.price_per_epoch.to_string(),
            provider_collateral: deal.provider_collateral.to_string(),
            status: format!("{:?}", deal.status),
            start_epoch: deal.start_epoch,
            duration_epochs: deal.deal_duration_epochs,
        })
        .collect();

    Ok(Json(items))
}

/// Get deal details
async fn get_deal(
    State(state): State<StorageMarketApiState>,
    Path(deal_id): Path<u64>,
) -> Result<Json<DealDetails>, ErrorResponse> {
    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    let deal = coordinator
        .get_deal(deal_id)
        .await
        .ok_or_else(|| ErrorResponse {
            error: format!("Deal {} not found", deal_id),
        })?;

    Ok(Json(DealDetails {
        deal_id: deal.deal_id,
        client: hex::encode(deal.client.as_bytes()),
        provider: hex::encode(deal.provider.as_bytes()),
        data_cid: hex::encode(deal.data_cid),
        data_size: deal.data_size,
        duration_epochs: deal.deal_duration_epochs,
        price_per_epoch: deal.price_per_epoch.to_string(),
        provider_collateral: deal.provider_collateral.to_string(),
        client_payment_escrow: deal.client_payment_escrow.to_string(),
        status: format!("{:?}", deal.status),
        start_epoch: deal.start_epoch,
        activation_deadline: deal.activation_deadline,
        proof_merkle_root: deal.proof_merkle_root.map(hex::encode),
    }))
}

// ============================================================================
// Proof Endpoint Handlers
// ============================================================================

/// Submit storage proof
async fn submit_proof(
    State(state): State<StorageMarketApiState>,
    Json(req): Json<SubmitProofRequest>,
) -> Result<Json<SubmitProofResponse>, ErrorResponse> {
    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    // Convert MerkleProofData to StorageMerkleProof
    let merkle_proofs: Result<Vec<StorageMerkleProof>, ErrorResponse> = req
        .merkle_proofs
        .iter()
        .map(|proof_data| {
            let chunk_data = hex::decode(&proof_data.chunk_data).map_err(|e| ErrorResponse {
                error: format!("Invalid chunk_data hex: {}", e),
            })?;

            let sibling_hashes: Result<Vec<[u8; 32]>, ErrorResponse> = proof_data
                .sibling_hashes
                .iter()
                .map(|hash_hex| {
                    let bytes = hex::decode(hash_hex).map_err(|e| ErrorResponse {
                        error: format!("Invalid sibling hash hex: {}", e),
                    })?;
                    bytes.try_into().map_err(|_| ErrorResponse {
                        error: "Sibling hash must be 32 bytes".to_string(),
                    })
                })
                .collect();

            Ok(StorageMerkleProof {
                chunk_index: proof_data.chunk_index,
                chunk_data,
                sibling_hashes: sibling_hashes?,
            })
        })
        .collect();

    let merkle_proofs = merkle_proofs?;

    // Create StorageProof
    let provider = state.node.wallet().address();
    let proof = StorageProof {
        approvals: [[0u8; 32]; 4],
        features: [0; 3],
        signature: adic_types::Signature::empty(),
        deposit: AdicAmount::ZERO,
        timestamp: chrono::Utc::now().timestamp(),
        deal_id: req.deal_id,
        provider,
        proof_epoch: req.proof_epoch,
        challenge_indices: req.challenge_indices,
        merkle_proofs,
        finality_status: FinalityStatus::Pending,
        finalized_at_epoch: None,
    };

    // Submit proof
    coordinator
        .submit_proof(proof)
        .await
        .map_err(|e| ErrorResponse {
            error: format!("Failed to submit proof: {}", e),
        })?;

    Ok(Json(SubmitProofResponse {
        success: true,
        message: "Proof submitted and verified successfully".to_string(),
    }))
}

/// Get challenges for a deal and epoch
async fn get_challenges(
    State(state): State<StorageMarketApiState>,
    Path((deal_id, epoch)): Path<(u64, u64)>,
) -> Result<Json<ChallengeResponse>, ErrorResponse> {
    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    let deal = coordinator
        .get_deal(deal_id)
        .await
        .ok_or_else(|| ErrorResponse {
            error: format!("Deal {} not found", deal_id),
        })?;

    // Get current epoch from storage market coordinator
    let current_epoch = coordinator.get_current_epoch().await;

    let challenges = coordinator
        .proof_manager
        .generate_challenge(&deal, epoch, current_epoch)
        .await
        .map_err(|e| ErrorResponse {
            error: format!("Failed to generate challenges: {}", e),
        })?;

    Ok(Json(ChallengeResponse {
        deal_id,
        epoch,
        challenge_indices: challenges,
    }))
}

// ============================================================================
// Fraud Proof Endpoint Handlers
// ============================================================================

/// Submit a fraud proof challenging a storage proof
async fn submit_fraud_proof(
    State(state): State<StorageMarketApiState>,
    Json(req): Json<SubmitFraudProofRequest>,
) -> Result<Json<SubmitFraudProofResponse>, ErrorResponse> {
    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    // Parse proof_id
    let proof_id_bytes = hex::decode(&req.proof_id).map_err(|e| ErrorResponse {
        error: format!("Invalid proof_id hex: {}", e),
    })?;
    let proof_id = adic_types::MessageId::from_bytes(
        proof_id_bytes
            .try_into()
            .map_err(|_| ErrorResponse {
                error: "proof_id must be 32 bytes".to_string(),
            })?,
    );

    // Parse challenger address
    let challenger_bytes = hex::decode(&req.challenger).map_err(|e| ErrorResponse {
        error: format!("Invalid challenger hex: {}", e),
    })?;
    let challenger = adic_types::PublicKey::from_bytes(
        challenger_bytes
            .try_into()
            .map_err(|_| ErrorResponse {
                error: "challenger must be 32 bytes".to_string(),
            })?,
    );

    // Parse evidence data
    let evidence_data = hex::decode(&req.evidence_data).map_err(|e| ErrorResponse {
        error: format!("Invalid evidence_data hex: {}", e),
    })?;

    // Create fraud proof
    use adic_challenges::{FraudEvidence, FraudProof, FraudType};

    let fraud_type = match req.evidence_type.as_str() {
        "data_corruption" => FraudType::DataCorruption,
        "incorrect_proof" => FraudType::IncorrectProof,
        "task_abandonment" => FraudType::TaskAbandonment,
        "invalid_result" => FraudType::InvalidResult,
        _ => FraudType::Custom(req.evidence_type.clone()),
    };

    let evidence = FraudEvidence {
        fraud_type,
        evidence_cid: format!("fraud-{}", hex::encode(&proof_id.as_bytes()[..8])),
        evidence_data: Some(evidence_data),
        metadata: std::collections::HashMap::new(),
    };

    let fraud_proof = FraudProof::new(proof_id, challenger, evidence, req.submitted_at_epoch);
    let fraud_proof_id = fraud_proof.id;

    // Submit fraud proof
    coordinator
        .proof_manager
        .submit_fraud_proof(proof_id, fraud_proof.clone())
        .await
        .map_err(|e| ErrorResponse {
            error: format!("Failed to submit fraud proof: {}", e),
        })?;

    info!(
        proof_id = hex::encode(proof_id.as_bytes()),
        fraud_proof_id = hex::encode(fraud_proof_id.as_bytes()),
        challenger = hex::encode(challenger.as_bytes()),
        "🚨 Fraud proof submitted via API"
    );

    Ok(Json(SubmitFraudProofResponse {
        fraud_proof_id: hex::encode(fraud_proof_id.as_bytes()),
        message: "Fraud proof submitted successfully".to_string(),
    }))
}

/// List all fraud proofs
async fn list_fraud_proofs(
    State(state): State<StorageMarketApiState>,
) -> Result<Json<ListFraudProofsResponse>, ErrorResponse> {
    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    let fraud_proofs = coordinator.proof_manager.get_all_fraud_proofs().await;

    let fraud_proof_data: Vec<FraudProofData> = fraud_proofs
        .iter()
        .map(|fp| FraudProofData {
            id: hex::encode(fp.id.as_bytes()),
            proof_id: hex::encode(fp.subject_id.as_bytes()),
            challenger: hex::encode(fp.challenger.as_bytes()),
            evidence_type: format!("{:?}", fp.evidence.fraud_type),
            evidence_data: fp
                .evidence
                .evidence_data
                .as_ref()
                .map(|d| hex::encode(d))
                .unwrap_or_default(),
            submitted_at_epoch: fp.submitted_at_epoch,
        })
        .collect();

    let total_count = fraud_proof_data.len();

    Ok(Json(ListFraudProofsResponse {
        fraud_proofs: fraud_proof_data,
        total_count,
    }))
}

/// Get a specific fraud proof by ID
async fn get_fraud_proof(
    State(state): State<StorageMarketApiState>,
    Path(id): Path<String>,
) -> Result<Json<FraudProofData>, ErrorResponse> {
    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    // Parse fraud proof ID
    let fraud_proof_id_bytes = hex::decode(&id).map_err(|e| ErrorResponse {
        error: format!("Invalid fraud proof ID hex: {}", e),
    })?;
    let fraud_proof_id = adic_types::MessageId::from_bytes(
        fraud_proof_id_bytes
            .try_into()
            .map_err(|_| ErrorResponse {
                error: "Fraud proof ID must be 32 bytes".to_string(),
            })?,
    );

    let fraud_proof = coordinator
        .proof_manager
        .get_fraud_proof(&fraud_proof_id)
        .await
        .ok_or_else(|| ErrorResponse {
            error: format!("Fraud proof {} not found", id),
        })?;

    Ok(Json(FraudProofData {
        id: hex::encode(fraud_proof.id.as_bytes()),
        proof_id: hex::encode(fraud_proof.subject_id.as_bytes()),
        challenger: hex::encode(fraud_proof.challenger.as_bytes()),
        evidence_type: format!("{:?}", fraud_proof.evidence.fraud_type),
        evidence_data: fraud_proof
            .evidence
            .evidence_data
            .as_ref()
            .map(|d| hex::encode(d))
            .unwrap_or_default(),
        submitted_at_epoch: fraud_proof.submitted_at_epoch,
    }))
}

// ============================================================================
// Statistics Endpoint Handlers
// ============================================================================

/// Get market statistics
async fn get_market_stats(
    State(state): State<StorageMarketApiState>,
) -> Result<Json<MarketStatsResponse>, ErrorResponse> {
    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    let stats = coordinator.get_market_stats().await;

    Ok(Json(MarketStatsResponse {
        total_intents: stats.total_intents,
        finalized_intents: stats.finalized_intents,
        total_acceptances: stats.total_acceptances,
        compiled_deals: stats.compiled_deals,
        active_deals: stats.active_deals,
        pending_activation: stats.pending_activation,
        completed_deals: stats.completed_deals,
        failed_deals: stats.failed_deals,
        current_epoch: stats.current_epoch,
    }))
}

/// Get provider statistics
async fn get_provider_stats(
    State(state): State<StorageMarketApiState>,
    Path(address): Path<String>,
) -> Result<Json<ProviderStatsResponse>, ErrorResponse> {
    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    let provider = parse_account_address(&address)?;
    let deals = coordinator.get_provider_deals(provider).await;
    let total_acceptances = coordinator.get_provider_acceptance_count(provider).await;

    let active = deals.iter().filter(|d| d.status == StorageDealStatus::Active).count();
    let completed = deals.iter().filter(|d| d.status == StorageDealStatus::Completed).count();
    let failed = deals.iter().filter(|d| d.status == StorageDealStatus::Failed).count();
    let total_storage: u64 = deals.iter().map(|d| d.data_size).sum();

    Ok(Json(ProviderStatsResponse {
        provider_address: address,
        total_acceptances,
        active_deals: active,
        completed_deals: completed,
        failed_deals: failed,
        total_storage_provided: total_storage,
        total_payments_received: "0".to_string(), // TODO: Track payments
    }))
}

/// Get client statistics
async fn get_client_stats(
    State(state): State<StorageMarketApiState>,
    Path(address): Path<String>,
) -> Result<Json<ClientStatsResponse>, ErrorResponse> {
    let coordinator = state
        .node
        .storage_market
        .as_ref()
        .ok_or_else(|| ErrorResponse {
            error: "Storage market not enabled".to_string(),
        })?;

    let client = parse_account_address(&address)?;
    let deals = coordinator.get_client_deals(client).await;
    let total_intents = coordinator.get_client_intent_count(client).await;

    let active = deals.iter().filter(|d| d.status == StorageDealStatus::Active).count();
    let completed = deals.iter().filter(|d| d.status == StorageDealStatus::Completed).count();
    let total_storage: u64 = deals.iter().map(|d| d.data_size).sum();

    Ok(Json(ClientStatsResponse {
        client_address: address,
        total_intents,
        active_deals: active,
        completed_deals: completed,
        total_storage_used: total_storage,
        total_payments_made: "0".to_string(), // TODO: Track payments
    }))
}

// ============================================================================
// Helper Functions
// ============================================================================

fn parse_account_address(hex: &str) -> Result<AccountAddress, ErrorResponse> {
    let bytes = hex::decode(hex).map_err(|e| ErrorResponse {
        error: format!("Invalid address hex: {}", e),
    })?;

    let arr: [u8; 32] = bytes.try_into().map_err(|_| ErrorResponse {
        error: "Address must be 32 bytes".to_string(),
    })?;

    Ok(AccountAddress::from_bytes(arr))
}

fn parse_deal_status(s: &str) -> Result<StorageDealStatus, ErrorResponse> {
    match s.to_lowercase().as_str() {
        "published" => Ok(StorageDealStatus::Published),
        "pendingactivation" | "pending_activation" => Ok(StorageDealStatus::PendingActivation),
        "active" => Ok(StorageDealStatus::Active),
        "completed" => Ok(StorageDealStatus::Completed),
        "failed" => Ok(StorageDealStatus::Failed),
        "terminated" => Ok(StorageDealStatus::Terminated),
        _ => Err(ErrorResponse {
            error: format!("Invalid status: {}", s),
        }),
    }
}
