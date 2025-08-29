use adic_economics::{AccountAddress, AdicAmount, EconomicsEngine};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::error;

#[derive(Clone)]
pub struct EconomicsApiState {
    pub economics: Arc<EconomicsEngine>,
}

#[derive(Serialize, Deserialize)]
pub struct SupplyResponse {
    pub total_supply: String,
    pub circulating_supply: String,
    pub treasury_balance: String,
    pub liquidity_balance: String,
    pub genesis_balance: String,
    pub burned_amount: String,
    pub emission_issued: String,
    pub max_supply: String,
    pub genesis_supply: String,
}

#[derive(Serialize, Deserialize)]
pub struct BalanceResponse {
    pub address: String,
    pub balance: String,
    pub locked_balance: String,
    pub unlocked_balance: String,
}

#[derive(Serialize, Deserialize)]
pub struct EmissionResponse {
    pub total_emitted: String,
    pub current_rate: f64,
    pub years_elapsed: f64,
    pub projected_1_year: String,
    pub projected_5_years: String,
    pub projected_10_years: String,
}

#[derive(Serialize, Deserialize)]
pub struct TreasuryResponse {
    pub balance: String,
    pub active_proposals: Vec<ProposalInfo>,
}

#[derive(Serialize, Deserialize)]
pub struct ProposalInfo {
    pub id: String,
    pub recipient: String,
    pub amount: String,
    pub reason: String,
    pub proposer: String,
    pub approvals: u32,
    pub threshold_required: u32,
    pub expires_at: i64,
}

#[derive(Serialize, Deserialize)]
pub struct GenesisResponse {
    pub allocated: bool,
    pub treasury_amount: String,
    pub liquidity_amount: String,
    pub genesis_amount: String,
    pub timestamp: Option<i64>,
}

#[derive(Deserialize)]
pub struct AddressQuery {
    pub address: Option<String>,
}

pub fn economics_routes(state: EconomicsApiState) -> Router {
    Router::new()
        .route("/v1/economics/supply", get(get_supply))
        .route("/v1/economics/balance/:address", get(get_balance))
        .route("/v1/economics/balance", get(get_balance_query))
        .route("/v1/economics/emissions", get(get_emissions))
        .route("/v1/economics/treasury", get(get_treasury))
        .route("/v1/economics/genesis", get(get_genesis))
        .route("/v1/economics/initialize", post(initialize_genesis))
        .with_state(Arc::new(state))
}

async fn get_supply(
    State(state): State<Arc<EconomicsApiState>>,
) -> Result<Json<SupplyResponse>, StatusCode> {
    let metrics = state.economics.supply.get_metrics().await;
    Ok(Json(SupplyResponse {
        total_supply: metrics.total_supply.to_string(),
        circulating_supply: metrics.circulating_supply.to_string(),
        treasury_balance: metrics.treasury_balance.to_string(),
        liquidity_balance: metrics.liquidity_balance.to_string(),
        genesis_balance: metrics.genesis_balance.to_string(),
        burned_amount: metrics.burned_amount.to_string(),
        emission_issued: metrics.emission_issued.to_string(),
        max_supply: AdicAmount::MAX_SUPPLY.to_string(),
        genesis_supply: AdicAmount::GENESIS_SUPPLY.to_string(),
    }))
}

async fn get_balance(
    Path(address): Path<String>,
    State(state): State<Arc<EconomicsApiState>>,
) -> Result<Json<BalanceResponse>, StatusCode> {
    let address = parse_address(&address).map_err(|_| StatusCode::BAD_REQUEST)?;

    let balance = state
        .economics
        .balances
        .get_balance(address)
        .await
        .map_err(|e| {
            error!("Failed to get balance: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let locked = state
        .economics
        .balances
        .get_locked_balance(address)
        .await
        .map_err(|e| {
            error!("Failed to get locked balance: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let unlocked = balance.saturating_sub(locked);

    Ok(Json(BalanceResponse {
        address: format!("{}", address),
        balance: balance.to_string(),
        locked_balance: locked.to_string(),
        unlocked_balance: unlocked.to_string(),
    }))
}

async fn get_balance_query(
    Query(query): Query<AddressQuery>,
    State(state): State<Arc<EconomicsApiState>>,
) -> Result<Json<BalanceResponse>, StatusCode> {
    let address_str = query.address.ok_or(StatusCode::BAD_REQUEST)?;
    get_balance(Path(address_str), State(state)).await
}

async fn get_emissions(
    State(state): State<Arc<EconomicsApiState>>,
) -> Result<Json<EmissionResponse>, StatusCode> {
    let metrics = state.economics.emission.get_metrics().await;
    let current_rate = state.economics.emission.get_current_emission_rate().await;

    let projected_1 = state.economics.emission.get_projected_emission(1.0).await;
    let projected_5 = state.economics.emission.get_projected_emission(5.0).await;
    let projected_10 = state.economics.emission.get_projected_emission(10.0).await;

    Ok(Json(EmissionResponse {
        total_emitted: metrics.total_emitted.to_string(),
        current_rate,
        years_elapsed: metrics.years_elapsed,
        projected_1_year: projected_1.to_string(),
        projected_5_years: projected_5.to_string(),
        projected_10_years: projected_10.to_string(),
    }))
}

async fn get_treasury(
    State(state): State<Arc<EconomicsApiState>>,
) -> Result<Json<TreasuryResponse>, StatusCode> {
    let balance = state
        .economics
        .treasury
        .get_treasury_balance()
        .await
        .map_err(|e| {
            error!("Failed to get treasury balance: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let proposals = state.economics.treasury.get_active_proposals().await;

    let proposal_infos: Vec<ProposalInfo> = proposals
        .into_iter()
        .map(|p| {
            ProposalInfo {
                id: hex::encode(&p.id[..8]),
                recipient: format!("{}", p.recipient),
                amount: p.amount.to_string(),
                reason: p.reason,
                proposer: format!("{}", p.proposer),
                approvals: p.approvals.len() as u32,
                threshold_required: 2, // This should come from config
                expires_at: p.expires_at,
            }
        })
        .collect();

    Ok(Json(TreasuryResponse {
        balance: balance.to_string(),
        active_proposals: proposal_infos,
    }))
}

async fn get_genesis(
    State(state): State<Arc<EconomicsApiState>>,
) -> Result<Json<GenesisResponse>, StatusCode> {
    if let Some(allocation) = state.economics.genesis.get_allocation().await {
        Ok(Json(GenesisResponse {
            allocated: true,
            treasury_amount: allocation.treasury_amount.to_string(),
            liquidity_amount: allocation.liquidity_amount.to_string(),
            genesis_amount: allocation.genesis_amount.to_string(),
            timestamp: Some(allocation.timestamp),
        }))
    } else {
        Ok(Json(GenesisResponse {
            allocated: false,
            treasury_amount: "0".to_string(),
            liquidity_amount: "0".to_string(),
            genesis_amount: "0".to_string(),
            timestamp: None,
        }))
    }
}

async fn initialize_genesis(
    State(state): State<Arc<EconomicsApiState>>,
) -> Result<Json<GenesisResponse>, StatusCode> {
    state.economics.initialize_genesis().await.map_err(|e| {
        error!("Failed to initialize genesis: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    get_genesis(State(state)).await
}

fn parse_address(s: &str) -> Result<AccountAddress, String> {
    // Handle special addresses
    match s.to_lowercase().as_str() {
        "treasury" => return Ok(AccountAddress::treasury()),
        "liquidity" => return Ok(AccountAddress::liquidity_pool()),
        "community" => return Ok(AccountAddress::community_grants()),
        "genesis" => return Ok(AccountAddress::genesis_pool()),
        _ => {}
    }

    // Try to parse as hex
    if let Some(hex_str) = s.strip_prefix("0x") {
        let bytes = hex::decode(hex_str).map_err(|e| e.to_string())?;
        if bytes.len() != 32 {
            return Err("Address must be 32 bytes".to_string());
        }
        let mut addr_bytes = [0u8; 32];
        addr_bytes.copy_from_slice(&bytes);
        Ok(AccountAddress::from_bytes(addr_bytes))
    } else {
        Err("Invalid address format".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_address() {
        // Test special addresses
        assert_eq!(
            parse_address("treasury").unwrap(),
            AccountAddress::treasury()
        );
        assert_eq!(
            parse_address("liquidity").unwrap(),
            AccountAddress::liquidity_pool()
        );

        // Test hex address
        let hex_addr = "0x".to_string() + &"00".repeat(32);
        assert!(parse_address(&hex_addr).is_ok());

        // Test invalid
        assert!(parse_address("invalid").is_err());
        assert!(parse_address("0x123").is_err());
    }
}
