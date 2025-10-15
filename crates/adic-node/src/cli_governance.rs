//! CLI handlers for governance commands
//!
//! Provides command-line interface functions for:
//! - Submitting proposals
//! - Casting votes
//! - Listing proposals
//! - Viewing proposal details
//! - Displaying governance parameters

use anyhow::{anyhow, Result};
use adic_types::{Ballot, ProposalClass};
use std::path::PathBuf;
use tracing::info;

/// Handle propose command
pub async fn handle_propose(
    params: String,
    values: String,
    class: String,
    rationale: String,
    enact_epoch: u64,
    _network: String,
    config_path: Option<PathBuf>,
) -> Result<()> {
    info!("📝 Submitting governance proposal");

    // Parse parameter keys (comma-separated)
    let param_keys: Vec<String> = params
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    // Parse values JSON
    let new_values: serde_json::Value = serde_json::from_str(&values)
        .map_err(|e| anyhow!("Invalid JSON for values: {}", e))?;

    // Parse proposal class
    let proposal_class = match class.to_lowercase().as_str() {
        "constitutional" => ProposalClass::Constitutional,
        "operational" => ProposalClass::Operational,
        _ => return Err(anyhow!("Invalid class: {}. Must be 'constitutional' or 'operational'", class)),
    };

    println!("📝 Governance Proposal");
    println!("  Parameters: {}", params);
    println!("  Values: {}", values);
    println!("  Class: {}", class);
    println!("  Rationale: {}", rationale);
    println!("  Enact Epoch: {}", enact_epoch);
    println!();

    // Load node
    let node = load_node(config_path).await?;

    // Submit proposal
    let (proposal_id, message_id) = node
        .submit_governance_proposal(
            param_keys,
            new_values,
            proposal_class,
            rationale,
            enact_epoch,
        )
        .await?;

    println!("✅ Proposal submitted successfully!");
    println!("  Proposal ID: {}", hex::encode(proposal_id));
    println!("  Message ID: {:?}", message_id);
    println!("  Status: Voting");
    println!();
    println!("Track your proposal:");
    println!("  adic governance show --proposal-id {}", hex::encode(proposal_id));

    Ok(())
}

/// Handle vote command
pub async fn handle_vote(
    proposal_id: String,
    ballot: String,
    _network: String,
    config_path: Option<PathBuf>,
) -> Result<()> {
    info!("🗳️  Casting vote");

    // Parse proposal ID (hex)
    let proposal_id_bytes = hex::decode(&proposal_id)
        .map_err(|e| anyhow!("Invalid proposal ID hex: {}", e))?;

    if proposal_id_bytes.len() != 32 {
        return Err(anyhow!("Proposal ID must be 32 bytes (64 hex chars)"));
    }

    let mut proposal_id_array = [0u8; 32];
    proposal_id_array.copy_from_slice(&proposal_id_bytes);

    // Parse ballot
    let vote_ballot = match ballot.to_lowercase().as_str() {
        "yes" => Ballot::Yes,
        "no" => Ballot::No,
        "abstain" => Ballot::Abstain,
        _ => return Err(anyhow!("Invalid ballot: {}. Must be 'yes', 'no', or 'abstain'", ballot)),
    };

    println!("🗳️  Vote");
    println!("  Proposal: {}", proposal_id);
    println!("  Ballot: {}", ballot);
    println!();

    // Load node
    let node = load_node(config_path).await?;

    // Cast vote
    let message_id = node.vote_on_proposal(proposal_id_array, vote_ballot).await?;

    println!("✅ Vote cast successfully!");
    println!("  Message ID: {:?}", message_id);
    println!();
    println!("View proposal status:");
    println!("  adic governance show --proposal-id {}", proposal_id);

    Ok(())
}

/// Handle list command
pub async fn handle_list(
    status: Option<String>,
    _network: String,
    config_path: Option<PathBuf>,
) -> Result<()> {
    info!("📋 Listing proposals");

    println!("📋 Governance Proposals");
    if let Some(ref s) = status {
        println!("  Filter: status={}", s);
    }
    println!();

    // Load node
    let node = load_node(config_path).await?;

    // Get governance manager
    let gov_manager = node
        .governance_manager
        .as_ref()
        .ok_or_else(|| anyhow!("Governance not enabled on this node"))?;

    // List proposals
    let proposals = gov_manager.get_all_proposals().await;

    if proposals.is_empty() {
        println!("No proposals found.");
        return Ok(());
    }

    // Filter by status if requested
    let filtered: Vec<_> = if let Some(ref filter) = status {
        proposals
            .into_iter()
            .filter(|p| format!("{:?}", p.status).to_lowercase() == filter.to_lowercase())
            .collect()
    } else {
        proposals
    };

    if filtered.is_empty() {
        println!("No proposals found matching filter.");
        return Ok(());
    }

    // Print table header
    println!("{:<20} {:<16} {:<12} {:>10} {:>10}",
        "Proposal ID", "Status", "Class", "Yes", "No");
    println!("{}", "=".repeat(70));

    // Get count before consuming
    let count = filtered.len();

    // Print proposals
    for proposal in filtered {
        let id_short = hex::encode(&proposal.proposal_id[..8]);
        println!(
            "{:<20} {:<16} {:<12} {:>10.1} {:>10.1}",
            id_short,
            format!("{:?}", proposal.status),
            format!("{:?}", proposal.class),
            proposal.tally_yes,
            proposal.tally_no,
        );
    }

    println!();
    println!("Total: {} proposals", count);

    Ok(())
}

/// Handle show command
pub async fn handle_show(
    proposal_id: String,
    _network: String,
    config_path: Option<PathBuf>,
) -> Result<()> {
    info!("🔍 Showing proposal: {}", proposal_id);

    // Parse proposal ID (hex)
    let proposal_id_bytes = hex::decode(&proposal_id)
        .map_err(|e| anyhow!("Invalid proposal ID hex: {}", e))?;

    if proposal_id_bytes.len() != 32 {
        return Err(anyhow!("Proposal ID must be 32 bytes (64 hex chars)"));
    }

    let mut proposal_id_array = [0u8; 32];
    proposal_id_array.copy_from_slice(&proposal_id_bytes);

    // Load node
    let node = load_node(config_path).await?;

    // Get governance manager
    let gov_manager = node
        .governance_manager
        .as_ref()
        .ok_or_else(|| anyhow!("Governance not enabled on this node"))?;

    // Get proposal
    let proposal = gov_manager
        .get_proposal(&proposal_id_array)
        .await
        .ok_or_else(|| anyhow!("Proposal not found"))?;

    // Print detailed information
    println!("🔍 Proposal Details");
    println!();
    println!("Proposal ID: {}", hex::encode(proposal.proposal_id));
    println!("Class: {:?}", proposal.class);
    println!("Status: {:?}", proposal.status);
    println!();
    println!("Proposer: {}", hex::encode(proposal.proposer_pk.as_bytes()));
    println!("Created: {}", proposal.creation_timestamp);
    println!("Voting Ends: {}", proposal.voting_end_timestamp);
    println!();
    println!("Parameters to Update:");
    for (i, key) in proposal.param_keys.iter().enumerate() {
        if let Some(value) = proposal.new_values.get(key) {
            println!("  {}: {} = {}", i + 1, key, value);
        } else {
            println!("  {}: {}", i + 1, key);
        }
    }
    println!();
    println!("Rationale: {}", proposal.rationale_cid);
    println!("Enact Epoch: {}", proposal.enact_epoch);
    println!();
    println!("Tally:");
    println!("  Yes:     {:>10.1} credits", proposal.tally_yes);
    println!("  No:      {:>10.1} credits", proposal.tally_no);
    println!("  Abstain: {:>10.1} credits", proposal.tally_abstain);

    let total = proposal.tally_yes + proposal.tally_no + proposal.tally_abstain;
    if total > 0.0 {
        println!("  Total:   {:>10.1} credits", total);
        println!();
        println!("Percentages:");
        println!("  Yes:     {:>6.1}%", (proposal.tally_yes / total) * 100.0);
        println!("  No:      {:>6.1}%", (proposal.tally_no / total) * 100.0);
        println!("  Abstain: {:>6.1}%", (proposal.tally_abstain / total) * 100.0);
    }

    Ok(())
}

/// Handle parameters command
pub async fn handle_parameters(
    _network: String,
    config_path: Option<PathBuf>,
) -> Result<()> {
    info!("⚙️  Showing governance parameters");

    println!("⚙️  Current Governance Parameters");
    println!();

    // Load node
    let node = load_node(config_path).await?;

    // Get ADIC parameters from consensus
    let params = node.consensus.params().await;

    // Print parameters
    println!("ADIC-DAG Parameters:");
    println!("  p (prime base):        {}", params.p);
    println!("  d (dimension):         {}", params.d);
    println!("  ρ (diversity radii):   {:?}", params.rho);
    println!("  q (quorum factor):     {}", params.q);
    println!("  k (k-core threshold):  {}", params.k);
    println!("  D* (depth threshold):  {}", params.depth_star);
    println!("  Δ (tip set size):      {}", params.delta);
    println!();
    println!("Economic Parameters:");
    println!("  Deposit:               {}", params.deposit);
    println!("  r_min:                 {}", params.r_min);
    println!("  r_sum_min:             {}", params.r_sum_min);
    println!();
    println!("MRW Parameters:");
    println!("  λ (proximity weight):  {}", params.lambda);
    println!("  α (reputation exp):    {}", params.alpha);
    println!("  β (age decay exp):     {}", params.beta);
    println!("  μ (conflict penalty):  {}", params.mu);
    println!("  γ (reputation update): {}", params.gamma);

    println!();
    println!("For more details, see the ADIC-DAG yellow paper.");

    Ok(())
}

/// Load node from config
async fn load_node(config_path: Option<PathBuf>) -> Result<std::sync::Arc<crate::node::AdicNode>> {
    use crate::config::NodeConfig;
    use crate::node::AdicNode;
    use std::sync::Arc;

    // Load configuration
    let config_file = config_path.unwrap_or_else(|| PathBuf::from("config.toml"));

    let config = NodeConfig::from_file(&config_file)?;

    // Create node
    let node = AdicNode::new(config).await?;

    Ok(Arc::new(node))
}
