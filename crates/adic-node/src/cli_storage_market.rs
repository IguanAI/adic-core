//! Storage Market CLI command handlers
//!
//! Handles all storage market CLI commands by making HTTP requests
//! to the local node's storage market API.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

// Import the commands enum from main - we need to reference it in the handler signature
// The actual enum is defined in main.rs and passed to this handler
// We'll use a re-export pattern to avoid duplication

/// Re-export of StorageMarketCommands from main module for type signature
/// The actual enum definition is in main.rs
pub enum StorageMarketCommands {
    PublishIntent {
        data_cid: String,
        size: u64,
        duration: u64,
        max_price: f64,
        redundancy: u8,
        rail: String,
        api_url: String,
    },
    ListIntents {
        client: Option<String>,
        status: Option<String>,
        limit: usize,
        offset: usize,
        api_url: String,
    },
    GetIntent {
        id: String,
        api_url: String,
    },
    AcceptIntent {
        intent_id: String,
        price: f64,
        collateral: f64,
        api_url: String,
    },
    ListAcceptances {
        provider: Option<String>,
        intent_id: Option<String>,
        limit: usize,
        api_url: String,
    },
    CompileDeal {
        intent_id: String,
        acceptance_id: String,
        api_url: String,
    },
    ActivateDeal {
        deal_id: u64,
        merkle_root: String,
        chunks: u64,
        api_url: String,
    },
    ListDeals {
        client: Option<String>,
        provider: Option<String>,
        status: Option<String>,
        limit: usize,
        api_url: String,
    },
    GetDeal {
        id: u64,
        api_url: String,
    },
    GetChallenges {
        deal_id: u64,
        epoch: u64,
        api_url: String,
    },
    Stats {
        api_url: String,
    },
    ProviderStats {
        address: String,
        api_url: String,
    },
    ClientStats {
        address: String,
        api_url: String,
    },
}

// Re-export types from API for CLI usage
#[derive(Debug, Serialize)]
struct PublishIntentRequest {
    data_cid: String,
    data_size: u64,
    duration_epochs: u64,
    max_price_per_epoch: f64,
    required_redundancy: u8,
    settlement_rails: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PublishIntentResponse {
    intent_id: String,
    status: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct SubmitAcceptanceRequest {
    intent_id: String,
    price_per_epoch: f64,
    collateral: f64,
}

#[derive(Debug, Deserialize)]
struct SubmitAcceptanceResponse {
    acceptance_id: String,
    status: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct CompileDealRequest {
    intent_id: String,
    acceptance_id: String,
}

#[derive(Debug, Deserialize)]
struct CompileDealResponse {
    deal_id: u64,
    status: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct ActivateDealRequest {
    merkle_root: String,
    chunk_count: u64,
}

#[derive(Debug, Deserialize)]
struct ActivateDealResponse {
    deal_id: u64,
    status: String,
    message: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct IntentListItem {
    intent_id: String,
    client: String,
    data_size: u64,
    duration_epochs: u64,
    max_price_per_epoch: String,
    required_redundancy: u8,
    status: String,
    created_at: i64,
    expires_at: i64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AcceptanceListItem {
    acceptance_id: String,
    intent_id: String,
    provider: String,
    price_per_epoch: String,
    collateral: String,
    status: String,
    created_at: i64,
}

#[derive(Debug, Deserialize)]
struct DealListItem {
    deal_id: u64,
    client: String,
    provider: String,
    data_size: u64,
    price_per_epoch: String,
    provider_collateral: String,
    status: String,
    start_epoch: Option<u64>,
    duration_epochs: u64,
}

#[derive(Debug, Deserialize)]
struct DealDetails {
    deal_id: u64,
    client: String,
    provider: String,
    data_cid: String,
    data_size: u64,
    duration_epochs: u64,
    price_per_epoch: String,
    provider_collateral: String,
    client_payment_escrow: String,
    status: String,
    start_epoch: Option<u64>,
    activation_deadline: u64,
    proof_merkle_root: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChallengeResponse {
    deal_id: u64,
    epoch: u64,
    challenge_indices: Vec<u64>,
}

#[derive(Debug, Deserialize)]
struct MarketStatsResponse {
    total_intents: usize,
    finalized_intents: usize,
    total_acceptances: usize,
    compiled_deals: usize,
    active_deals: usize,
    pending_activation: usize,
    completed_deals: usize,
    failed_deals: usize,
    current_epoch: u64,
}

#[derive(Debug, Deserialize)]
struct ProviderStatsResponse {
    provider_address: String,
    total_acceptances: usize,
    active_deals: usize,
    completed_deals: usize,
    failed_deals: usize,
    total_storage_provided: u64,
    total_payments_received: String,
}

#[derive(Debug, Deserialize)]
struct ClientStatsResponse {
    client_address: String,
    total_intents: usize,
    active_deals: usize,
    completed_deals: usize,
    total_storage_used: u64,
    total_payments_made: String,
}

/// Main handler for all storage market CLI commands
pub async fn handle_storage_market_command(command: StorageMarketCommands) -> Result<()> {
    match command {
        StorageMarketCommands::PublishIntent {
            data_cid,
            size,
            duration,
            max_price,
            redundancy,
            rail,
            api_url,
        } => {
            handle_publish_intent(data_cid, size, duration, max_price, redundancy, rail, api_url)
                .await
        }

        StorageMarketCommands::ListIntents {
            client,
            status,
            limit,
            offset,
            api_url,
        } => handle_list_intents(client, status, limit, offset, api_url).await,

        StorageMarketCommands::GetIntent { id, api_url } => {
            handle_get_intent(id, api_url).await
        }

        StorageMarketCommands::AcceptIntent {
            intent_id,
            price,
            collateral,
            api_url,
        } => handle_accept_intent(intent_id, price, collateral, api_url).await,

        StorageMarketCommands::ListAcceptances {
            provider,
            intent_id,
            limit,
            api_url,
        } => handle_list_acceptances(provider, intent_id, limit, api_url).await,

        StorageMarketCommands::CompileDeal {
            intent_id,
            acceptance_id,
            api_url,
        } => handle_compile_deal(intent_id, acceptance_id, api_url).await,

        StorageMarketCommands::ActivateDeal {
            deal_id,
            merkle_root,
            chunks,
            api_url,
        } => handle_activate_deal(deal_id, merkle_root, chunks, api_url).await,

        StorageMarketCommands::ListDeals {
            client,
            provider,
            status,
            limit,
            api_url,
        } => handle_list_deals(client, provider, status, limit, api_url).await,

        StorageMarketCommands::GetDeal { id, api_url } => handle_get_deal(id, api_url).await,

        StorageMarketCommands::GetChallenges {
            deal_id,
            epoch,
            api_url,
        } => handle_get_challenges(deal_id, epoch, api_url).await,

        StorageMarketCommands::Stats { api_url } => handle_stats(api_url).await,

        StorageMarketCommands::ProviderStats { address, api_url } => {
            handle_provider_stats(address, api_url).await
        }

        StorageMarketCommands::ClientStats { address, api_url } => {
            handle_client_stats(address, api_url).await
        }
    }
}

async fn handle_publish_intent(
    data_cid: String,
    size: u64,
    duration: u64,
    max_price: f64,
    redundancy: u8,
    rail: String,
    api_url: String,
) -> Result<()> {
    let client = Client::new();
    let url = format!("{}/v1/storage/intents", api_url);

    let req = PublishIntentRequest {
        data_cid,
        data_size: size,
        duration_epochs: duration,
        max_price_per_epoch: max_price,
        required_redundancy: redundancy,
        settlement_rails: vec![rail],
    };

    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .context("Failed to send request")?
        .json::<PublishIntentResponse>()
        .await
        .context("Failed to parse response")?;

    println!("✅ Intent published successfully!");
    println!("   Intent ID: {}", resp.intent_id);
    println!("   Status: {}", resp.status);
    println!("   {}", resp.message);

    Ok(())
}

async fn handle_list_intents(
    client_addr: Option<String>,
    status: Option<String>,
    limit: usize,
    offset: usize,
    api_url: String,
) -> Result<()> {
    let client = Client::new();
    let mut url = format!("{}/v1/storage/intents?limit={}&offset={}", api_url, limit, offset);

    if let Some(c) = client_addr {
        url.push_str(&format!("&client={}", c));
    }
    if let Some(s) = status {
        url.push_str(&format!("&status={}", s));
    }

    let intents = client
        .get(&url)
        .send()
        .await
        .context("Failed to send request")?
        .json::<Vec<IntentListItem>>()
        .await
        .context("Failed to parse response")?;

    if intents.is_empty() {
        println!("No intents found");
        return Ok(());
    }

    println!("\n📋 Intents ({} total):", intents.len());
    println!("{}", "=".repeat(120));
    println!(
        "{:<20} {:<18} {:<12} {:<12} {:<15} {:<12} {:<12}",
        "Intent ID", "Client", "Size (bytes)", "Duration", "Max Price/Epoch", "Redundancy", "Status"
    );
    println!("{}", "=".repeat(120));

    for intent in intents {
        println!(
            "{:<20} {:<18} {:<12} {:<12} {:<15} {:<12} {:<12}",
            &intent.intent_id[..16],
            &intent.client[..16],
            intent.data_size,
            intent.duration_epochs,
            intent.max_price_per_epoch,
            intent.required_redundancy,
            intent.status
        );
    }

    Ok(())
}

async fn handle_get_intent(id: String, api_url: String) -> Result<()> {
    println!("✅ API endpoint available at: GET {}/v1/storage/intents/{}", api_url, id);
    println!("   Use curl or HTTP client to retrieve intent details");
    Ok(())
}

async fn handle_accept_intent(
    intent_id: String,
    price: f64,
    collateral: f64,
    api_url: String,
) -> Result<()> {
    let client = Client::new();
    let url = format!("{}/v1/storage/acceptances", api_url);

    let req = SubmitAcceptanceRequest {
        intent_id,
        price_per_epoch: price,
        collateral,
    };

    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .context("Failed to send request")?
        .json::<SubmitAcceptanceResponse>()
        .await
        .context("Failed to parse response")?;

    println!("✅ Acceptance submitted successfully!");
    println!("   Acceptance ID: {}", resp.acceptance_id);
    println!("   Status: {}", resp.status);
    println!("   {}", resp.message);

    Ok(())
}

async fn handle_list_acceptances(
    provider: Option<String>,
    intent_id: Option<String>,
    limit: usize,
    api_url: String,
) -> Result<()> {
    let client = Client::new();
    let mut url = format!("{}/v1/storage/acceptances?limit={}", api_url, limit);

    if let Some(p) = provider {
        url.push_str(&format!("&provider={}", p));
    }
    if let Some(i) = intent_id {
        url.push_str(&format!("&intent_id={}", i));
    }

    let acceptances = client
        .get(&url)
        .send()
        .await
        .context("Failed to send request")?
        .json::<Vec<AcceptanceListItem>>()
        .await
        .context("Failed to parse response")?;

    if acceptances.is_empty() {
        println!("No acceptances found");
        return Ok(());
    }

    println!("\n📋 Acceptances ({} total):", acceptances.len());
    println!("{}", "=".repeat(120));
    println!(
        "{:<20} {:<20} {:<18} {:<15} {:<18} {:<12}",
        "Acceptance ID", "Intent ID", "Provider", "Price/Epoch", "Collateral", "Status"
    );
    println!("{}", "=".repeat(120));

    for acceptance in acceptances {
        println!(
            "{:<20} {:<20} {:<18} {:<15} {:<18} {:<12}",
            &acceptance.acceptance_id[..16],
            &acceptance.intent_id[..16],
            &acceptance.provider[..16],
            acceptance.price_per_epoch,
            acceptance.collateral,
            acceptance.status
        );
    }

    Ok(())
}

async fn handle_compile_deal(
    intent_id: String,
    acceptance_id: String,
    api_url: String,
) -> Result<()> {
    let client = Client::new();
    let url = format!("{}/v1/storage/deals/compile", api_url);

    let req = CompileDealRequest {
        intent_id,
        acceptance_id,
    };

    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .context("Failed to send request")?
        .json::<CompileDealResponse>()
        .await
        .context("Failed to parse response")?;

    println!("✅ Deal compiled successfully!");
    println!("   Deal ID: {}", resp.deal_id);
    println!("   Status: {}", resp.status);
    println!("   {}", resp.message);

    Ok(())
}

async fn handle_activate_deal(
    deal_id: u64,
    merkle_root: String,
    chunks: u64,
    api_url: String,
) -> Result<()> {
    let client = Client::new();
    let url = format!("{}/v1/storage/deals/{}/activate", api_url, deal_id);

    let req = ActivateDealRequest {
        merkle_root,
        chunk_count: chunks,
    };

    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .context("Failed to send request")?
        .json::<ActivateDealResponse>()
        .await
        .context("Failed to parse response")?;

    println!("✅ Deal activated successfully!");
    println!("   Deal ID: {}", resp.deal_id);
    println!("   Status: {}", resp.status);
    println!("   {}", resp.message);

    Ok(())
}

async fn handle_list_deals(
    client_addr: Option<String>,
    provider: Option<String>,
    status: Option<String>,
    limit: usize,
    api_url: String,
) -> Result<()> {
    let client = Client::new();
    let mut url = format!("{}/v1/storage/deals?limit={}", api_url, limit);

    if let Some(c) = client_addr {
        url.push_str(&format!("&client={}", c));
    }
    if let Some(p) = provider {
        url.push_str(&format!("&provider={}", p));
    }
    if let Some(s) = status {
        url.push_str(&format!("&status={}", s));
    }

    let deals = client
        .get(&url)
        .send()
        .await
        .context("Failed to send request")?
        .json::<Vec<DealListItem>>()
        .await
        .context("Failed to parse response")?;

    if deals.is_empty() {
        println!("No deals found");
        return Ok(());
    }

    println!("\n📋 Deals ({} total):", deals.len());
    println!("{}", "=".repeat(160));
    println!(
        "{:<8} {:<18} {:<18} {:<12} {:<15} {:<18} {:<12} {:<12} {:<12}",
        "Deal ID", "Client", "Provider", "Size (bytes)", "Price/Epoch", "Collateral", "Status", "Duration", "Start"
    );
    println!("{}", "=".repeat(160));

    for deal in deals {
        println!(
            "{:<8} {:<18} {:<18} {:<12} {:<15} {:<18} {:<12} {:<12} {:<12}",
            deal.deal_id,
            &deal.client[..16],
            &deal.provider[..16],
            deal.data_size,
            deal.price_per_epoch,
            deal.provider_collateral,
            deal.status,
            deal.duration_epochs,
            deal.start_epoch
                .map(|e| e.to_string())
                .unwrap_or_else(|| "-".to_string())
        );
    }

    Ok(())
}

async fn handle_get_deal(id: u64, api_url: String) -> Result<()> {
    let client = Client::new();
    let url = format!("{}/v1/storage/deals/{}", api_url, id);

    let deal = client
        .get(&url)
        .send()
        .await
        .context("Failed to send request")?
        .json::<DealDetails>()
        .await
        .context("Failed to parse response")?;

    println!("\n📄 Deal Details:");
    println!("{}", "=".repeat(80));
    println!("Deal ID:             {}", deal.deal_id);
    println!("Client:              {}", deal.client);
    println!("Provider:            {}", deal.provider);
    println!("Data CID:            {}", deal.data_cid);
    println!("Data Size:           {} bytes", deal.data_size);
    println!("Duration:            {} epochs", deal.duration_epochs);
    println!("Price/Epoch:         {}", deal.price_per_epoch);
    println!("Provider Collateral: {}", deal.provider_collateral);
    println!("Client Escrow:       {}", deal.client_payment_escrow);
    println!("Status:              {}", deal.status);
    println!(
        "Start Epoch:         {}",
        deal.start_epoch
            .map(|e| e.to_string())
            .unwrap_or_else(|| "Not started".to_string())
    );
    println!("Activation Deadline: {}", deal.activation_deadline);
    if let Some(root) = deal.proof_merkle_root {
        println!("Merkle Root:         {}", root);
    }

    Ok(())
}

async fn handle_get_challenges(deal_id: u64, epoch: u64, api_url: String) -> Result<()> {
    let client = Client::new();
    let url = format!("{}/v1/storage/challenges/{}/{}", api_url, deal_id, epoch);

    let challenges = client
        .get(&url)
        .send()
        .await
        .context("Failed to send request")?
        .json::<ChallengeResponse>()
        .await
        .context("Failed to parse response")?;

    println!("\n🎯 Challenges for Deal {} at Epoch {}:", challenges.deal_id, challenges.epoch);
    println!("{}", "=".repeat(60));
    println!("Challenge Indices: {:?}", challenges.challenge_indices);
    println!("Total Challenges:  {}", challenges.challenge_indices.len());

    Ok(())
}

async fn handle_stats(api_url: String) -> Result<()> {
    let client = Client::new();
    let url = format!("{}/v1/storage/stats", api_url);

    let stats = client
        .get(&url)
        .send()
        .await
        .context("Failed to send request")?
        .json::<MarketStatsResponse>()
        .await
        .context("Failed to parse response")?;

    println!("\n📊 Storage Market Statistics:");
    println!("{}", "=".repeat(60));
    println!("Total Intents:        {}", stats.total_intents);
    println!("Finalized Intents:    {}", stats.finalized_intents);
    println!("Total Acceptances:    {}", stats.total_acceptances);
    println!("Compiled Deals:       {}", stats.compiled_deals);
    println!("Active Deals:         {}", stats.active_deals);
    println!("Pending Activation:   {}", stats.pending_activation);
    println!("Completed Deals:      {}", stats.completed_deals);
    println!("Failed Deals:         {}", stats.failed_deals);
    println!("Current Epoch:        {}", stats.current_epoch);

    Ok(())
}

async fn handle_provider_stats(address: String, api_url: String) -> Result<()> {
    let client = Client::new();
    let url = format!("{}/v1/storage/stats/provider/{}", api_url, address);

    let stats = client
        .get(&url)
        .send()
        .await
        .context("Failed to send request")?
        .json::<ProviderStatsResponse>()
        .await
        .context("Failed to parse response")?;

    println!("\n📊 Provider Statistics:");
    println!("{}", "=".repeat(60));
    println!("Provider Address:      {}", stats.provider_address);
    println!("Total Acceptances:     {}", stats.total_acceptances);
    println!("Active Deals:          {}", stats.active_deals);
    println!("Completed Deals:       {}", stats.completed_deals);
    println!("Failed Deals:          {}", stats.failed_deals);
    println!("Total Storage:         {} bytes", stats.total_storage_provided);
    println!("Total Payments:        {}", stats.total_payments_received);

    Ok(())
}

async fn handle_client_stats(address: String, api_url: String) -> Result<()> {
    let client = Client::new();
    let url = format!("{}/v1/storage/stats/client/{}", api_url, address);

    let stats = client
        .get(&url)
        .send()
        .await
        .context("Failed to parse response")?
        .json::<ClientStatsResponse>()
        .await
        .context("Failed to parse response")?;

    println!("\n📊 Client Statistics:");
    println!("{}", "=".repeat(60));
    println!("Client Address:        {}", stats.client_address);
    println!("Total Intents:         {}", stats.total_intents);
    println!("Active Deals:          {}", stats.active_deals);
    println!("Completed Deals:       {}", stats.completed_deals);
    println!("Total Storage Used:    {} bytes", stats.total_storage_used);
    println!("Total Payments Made:   {}", stats.total_payments_made);

    Ok(())
}
