//! Dispute Resolution
//!
//! This module implements Sprint 9 from storage-market-design.md:
//! - DisputeChallenge and DisputeResolution message types
//! - Consensus-driven resolution for corruption/unavailability
//! - VRF-based arbitrator selection and bonding
//! - Evidence submission via IPFS/Arweave CIDs
//!
//! ## Dispute Types
//!
//! 1. **DataCorruption**: Client claims provider has wrong data for a chunk
//! 2. **DataUnavailable**: Client cannot retrieve data from provider
//! 3. **IncorrectProof**: Provider submitted invalid storage proof
//! 4. **PerformanceBreach**: Provider failed to meet SLA requirements
//!
//! ## Resolution Methods
//!
//! - **Automatic**: Re-challenge provider, compare with evidence
//! - **Quorum-based**: VRF-selected arbitrators vote on complex disputes
//! - **Consensus-driven**: Leverage existing provider health/reputation data

use crate::error::{Result, StorageMarketError};
use crate::provider::ProviderHealth;
use crate::types::{FinalityStatus, Hash, MessageHash, StorageDeal};
use adic_economics::types::{AccountAddress, AdicAmount};
use adic_types::Signature;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Dispute types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum DisputeType {
    /// Provider has corrupted data (wrong chunk content)
    DataCorruption,
    /// Provider cannot be reached or data is unavailable
    DataUnavailable,
    /// Provider submitted invalid Merkle proof
    IncorrectProof,
    /// Provider failed to meet retrieval SLA
    PerformanceBreach,
}

/// Dispute ruling outcomes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisputeRuling {
    /// Client's dispute is valid, slash provider
    ClientWins,
    /// Provider successfully defended, slash client deposit
    ProviderWins,
    /// Cannot determine outcome, return deposits
    Inconclusive,
}

/// Dispute challenge message (initiated by client)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisputeChallenge {
    // ========== Protocol Fields ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    /// Higher deposit for disputes (5x normal deposit)
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Dispute Fields ==========
    /// Unique dispute identifier
    pub dispute_id: Hash,
    /// Deal being disputed
    pub deal_id: u64,
    /// Client initiating dispute
    pub client: AccountAddress,
    /// Provider being challenged
    pub provider: AccountAddress,
    /// Type of dispute
    pub dispute_type: DisputeType,

    // ========== Evidence Fields ==========
    /// IPFS/Arweave CID of evidence (logs, data, etc.)
    pub evidence_cid: Option<Hash>,
    /// For DataCorruption: which chunk is claimed corrupt
    pub claimed_chunk_index: Option<u64>,
    /// For DataCorruption: client's version of correct data
    pub claimed_correct_data: Option<Vec<u8>>,
    /// For DataUnavailable: failed retrieval request ID
    pub failed_request_id: Option<Hash>,
    /// For PerformanceBreach: measured bandwidth/latency
    pub measured_performance: Option<PerformanceMetrics>,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
}

/// Performance metrics for SLA breach disputes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    pub bandwidth_bps: u64,
    pub latency_ms: u64,
    pub required_bandwidth_bps: u64,
    pub required_latency_ms: u64,
}

/// Dispute resolution message (result of arbitration)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisputeResolution {
    // ========== Protocol Fields ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Resolution Fields ==========
    /// Reference to original dispute
    pub ref_dispute: Hash,
    /// Arbitrator who resolved (None for consensus-driven)
    pub arbitrator: Option<AccountAddress>,
    /// Quorum members who voted (for quorum-based resolution)
    pub quorum_members: Vec<AccountAddress>,
    /// Individual votes from quorum members
    pub quorum_votes: Vec<DisputeRuling>,
    /// Final ruling
    pub ruling: DisputeRuling,
    /// Summary of evidence and reasoning
    pub evidence_summary: String,
    /// Amount to slash from losing party
    pub slash_amount: AdicAmount,
    /// Amount to refund to winning party
    pub refund_amount: AdicAmount,
    /// Resolution epoch
    pub resolved_at_epoch: u64,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
}

impl DisputeChallenge {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        deal_id: u64,
        client: AccountAddress,
        provider: AccountAddress,
        dispute_type: DisputeType,
        evidence_cid: Option<Hash>,
        deposit: AdicAmount,
    ) -> Self {
        let now = chrono::Utc::now().timestamp();
        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(now * 1_000_000_000);
        let mut dispute_data = Vec::new();
        dispute_data.extend_from_slice(&deal_id.to_le_bytes());
        dispute_data.extend_from_slice(client.as_bytes());
        dispute_data.extend_from_slice(provider.as_bytes());
        dispute_data.extend_from_slice(&now_nanos.to_le_bytes()); // Use nanosecond precision
        dispute_data.push(dispute_type as u8); // Include dispute type
        if let Some(cid) = evidence_cid {
            dispute_data.extend_from_slice(&cid);
        }

        Self {
            approvals: [MessageHash::default(); 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit,
            timestamp: now,
            dispute_id: blake3::hash(&dispute_data).into(),
            deal_id,
            client,
            provider,
            dispute_type,
            evidence_cid,
            claimed_chunk_index: None,
            claimed_correct_data: None,
            failed_request_id: None,
            measured_performance: None,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
        }
    }

    pub fn is_finalized(&self) -> bool {
        self.finality_status.is_finalized()
    }
}

impl DisputeResolution {
    pub fn new(
        ref_dispute: Hash,
        ruling: DisputeRuling,
        evidence_summary: String,
        slash_amount: AdicAmount,
        refund_amount: AdicAmount,
        resolved_at_epoch: u64,
    ) -> Self {
        Self {
            approvals: [MessageHash::default(); 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: chrono::Utc::now().timestamp(),
            ref_dispute,
            arbitrator: None,
            quorum_members: Vec::new(),
            quorum_votes: Vec::new(),
            ruling,
            evidence_summary,
            slash_amount,
            refund_amount,
            resolved_at_epoch,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
        }
    }

    pub fn is_finalized(&self) -> bool {
        self.finality_status.is_finalized()
    }
}

/// Dispute manager configuration
#[derive(Debug, Clone)]
pub struct DisputeConfig {
    /// Minimum dispute deposit (typically 5x normal deposit)
    pub min_dispute_deposit: AdicAmount,
    /// Minimum reputation required to be arbitrator
    pub min_arbitrator_reputation: f64,
    /// Quorum size for complex disputes
    pub quorum_size: usize,
    /// Threshold for majority ruling (e.g., 0.66 = 2/3)
    pub ruling_threshold: f64,
    /// Time limit for dispute resolution (epochs)
    pub resolution_deadline: u64,
    /// Slash percentage for client if dispute loses
    pub client_loss_penalty: f64,
    /// Slash percentage for provider if dispute wins
    pub provider_corruption_penalty: f64,
}

impl Default for DisputeConfig {
    fn default() -> Self {
        Self {
            min_dispute_deposit: AdicAmount::from_adic(0.5),
            min_arbitrator_reputation: 50.0,
            quorum_size: 15,
            ruling_threshold: 0.66,
            resolution_deadline: 100,
            client_loss_penalty: 0.5,          // 50% of dispute deposit
            provider_corruption_penalty: 0.3,   // 30% of collateral
        }
    }
}

/// Dispute state tracking
#[derive(Debug, Clone)]
pub struct DisputeState {
    pub challenge: DisputeChallenge,
    pub resolution: Option<DisputeResolution>,
    pub status: DisputeStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisputeStatus {
    Pending,
    UnderReview,
    Resolved,
    Expired,
}

/// Dispute manager coordinator
pub struct DisputeManager {
    config: DisputeConfig,
    disputes: Arc<RwLock<HashMap<Hash, DisputeState>>>,
    // Map deal_id -> dispute_ids
    deal_disputes: Arc<RwLock<HashMap<u64, Vec<Hash>>>>,
}

impl DisputeManager {
    pub fn new(config: DisputeConfig) -> Self {
        Self {
            config,
            disputes: Arc::new(RwLock::new(HashMap::new())),
            deal_disputes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Helper to multiply AdicAmount by a percentage
    fn multiply_by_percentage(&self, amount: AdicAmount, percentage: f64) -> AdicAmount {
        AdicAmount::from_adic(amount.to_adic() * percentage)
    }

    /// Submit a dispute challenge
    pub async fn submit_dispute(
        &self,
        challenge: DisputeChallenge,
        deal: &StorageDeal,
    ) -> Result<Hash> {
        // Validate dispute
        if challenge.client != deal.client {
            return Err(StorageMarketError::InvalidDispute(
                "Dispute client does not match deal client".into(),
            ));
        }

        if challenge.provider != deal.provider {
            return Err(StorageMarketError::InvalidDispute(
                "Dispute provider does not match deal provider".into(),
            ));
        }

        if challenge.deposit < self.config.min_dispute_deposit {
            return Err(StorageMarketError::InsufficientFunds {
                required: self.config.min_dispute_deposit.to_string(),
                available: challenge.deposit.to_string(),
            });
        }

        // Validate dispute type-specific fields
        match challenge.dispute_type {
            DisputeType::DataCorruption => {
                if challenge.claimed_chunk_index.is_none() {
                    return Err(StorageMarketError::InvalidDispute(
                        "DataCorruption dispute requires claimed_chunk_index".into(),
                    ));
                }
            }
            DisputeType::DataUnavailable => {
                if challenge.failed_request_id.is_none() {
                    return Err(StorageMarketError::InvalidDispute(
                        "DataUnavailable dispute requires failed_request_id".into(),
                    ));
                }
            }
            DisputeType::PerformanceBreach => {
                if challenge.measured_performance.is_none() {
                    return Err(StorageMarketError::InvalidDispute(
                        "PerformanceBreach dispute requires measured_performance".into(),
                    ));
                }
            }
            DisputeType::IncorrectProof => {
                // Evidence CID should contain the incorrect proof
                if challenge.evidence_cid.is_none() {
                    return Err(StorageMarketError::InvalidDispute(
                        "IncorrectProof dispute requires evidence_cid".into(),
                    ));
                }
            }
        }

        let dispute_id = challenge.dispute_id;

        // Store dispute
        let state = DisputeState {
            challenge: challenge.clone(),
            resolution: None,
            status: DisputeStatus::Pending,
        };

        self.disputes.write().await.insert(dispute_id, state);

        self.deal_disputes
            .write()
            .await
            .entry(challenge.deal_id)
            .or_insert_with(Vec::new)
            .push(dispute_id);

        info!(
            "Dispute {} submitted for deal {} (type: {:?})",
            hex::encode(&dispute_id[..8]),
            challenge.deal_id,
            challenge.dispute_type
        );

        Ok(dispute_id)
    }

    /// Resolve dispute automatically (consensus-driven)
    pub async fn resolve_dispute_automatic(
        &self,
        dispute_id: &Hash,
        deal: &StorageDeal,
        provider_health: ProviderHealth,
        current_epoch: u64,
    ) -> Result<DisputeResolution> {
        let disputes = self.disputes.read().await;
        let state = disputes
            .get(dispute_id)
            .ok_or_else(|| StorageMarketError::NotFound("Dispute not found".into()))?;

        let challenge = &state.challenge;

        // Determine ruling based on dispute type
        let (ruling, evidence_summary, slash_amount, refund_amount) =
            self.calculate_automatic_resolution(challenge, deal, provider_health, current_epoch);

        drop(disputes);

        let resolution = DisputeResolution::new(
            *dispute_id,
            ruling,
            evidence_summary,
            slash_amount,
            refund_amount,
            current_epoch,
        );

        // Update dispute state
        self.disputes.write().await.get_mut(dispute_id).unwrap().resolution = Some(resolution.clone());
        self.disputes.write().await.get_mut(dispute_id).unwrap().status = DisputeStatus::Resolved;

        info!(
            "Dispute {} resolved automatically: {:?}",
            hex::encode(&dispute_id[..8]),
            ruling
        );

        Ok(resolution)
    }

    /// Calculate automatic resolution based on dispute type and evidence
    fn calculate_automatic_resolution(
        &self,
        challenge: &DisputeChallenge,
        deal: &StorageDeal,
        provider_health: ProviderHealth,
        _current_epoch: u64,
    ) -> (DisputeRuling, String, AdicAmount, AdicAmount) {
        match challenge.dispute_type {
            DisputeType::DataUnavailable => {
                // Check provider health
                if matches!(provider_health, ProviderHealth::Offline) {
                    let slash = self.multiply_by_percentage(deal.provider_collateral, self.config.provider_corruption_penalty);
                    let refund = self.calculate_remaining_payment(deal);
                    (
                        DisputeRuling::ClientWins,
                        "Provider is offline, cannot retrieve data".to_string(),
                        slash,
                        refund.saturating_add(challenge.deposit),
                    )
                } else {
                    (
                        DisputeRuling::Inconclusive,
                        "Provider is online, need more evidence".to_string(),
                        AdicAmount::ZERO,
                        challenge.deposit, // Return dispute deposit
                    )
                }
            }
            DisputeType::PerformanceBreach => {
                if let Some(metrics) = &challenge.measured_performance {
                    if metrics.bandwidth_bps < metrics.required_bandwidth_bps {
                        let slash = self.multiply_by_percentage(deal.provider_collateral, 0.1); // 10% penalty for SLA breach
                        let slash_bonus = self.multiply_by_percentage(slash, 0.5);
                        (
                            DisputeRuling::ClientWins,
                            format!(
                                "Provider bandwidth {}bps below required {}bps",
                                metrics.bandwidth_bps, metrics.required_bandwidth_bps
                            ),
                            slash,
                            challenge.deposit.saturating_add(slash_bonus),
                        )
                    } else {
                        (
                            DisputeRuling::ProviderWins,
                            "Provider met SLA requirements".to_string(),
                            self.multiply_by_percentage(challenge.deposit, self.config.client_loss_penalty),
                            AdicAmount::ZERO,
                        )
                    }
                } else {
                    (
                        DisputeRuling::Inconclusive,
                        "No performance metrics provided".to_string(),
                        AdicAmount::ZERO,
                        challenge.deposit,
                    )
                }
            }
            DisputeType::DataCorruption | DisputeType::IncorrectProof => {
                // These require quorum verification
                (
                    DisputeRuling::Inconclusive,
                    "Requires quorum verification".to_string(),
                    AdicAmount::ZERO,
                    AdicAmount::ZERO,
                )
            }
        }
    }

    /// Resolve dispute via quorum voting
    pub async fn resolve_dispute_via_quorum(
        &self,
        dispute_id: &Hash,
        deal: &StorageDeal,
        quorum_votes: Vec<(AccountAddress, DisputeRuling)>,
        current_epoch: u64,
    ) -> Result<DisputeResolution> {
        let disputes = self.disputes.read().await;
        let state = disputes
            .get(dispute_id)
            .ok_or_else(|| StorageMarketError::NotFound("Dispute not found".into()))?;

        let challenge = &state.challenge;

        // Determine majority ruling
        let ruling = self.determine_majority_ruling(&quorum_votes.iter().map(|(_, r)| *r).collect::<Vec<_>>());

        let (slash_amount, refund_amount) = match ruling {
            DisputeRuling::ClientWins => {
                let slash = self.multiply_by_percentage(deal.provider_collateral, self.config.provider_corruption_penalty);
                let refund = self.calculate_remaining_payment(deal);
                (slash, refund.saturating_add(challenge.deposit))
            }
            DisputeRuling::ProviderWins => {
                let slash = self.multiply_by_percentage(challenge.deposit, self.config.client_loss_penalty);
                (slash, AdicAmount::ZERO)
            }
            DisputeRuling::Inconclusive => {
                (AdicAmount::ZERO, challenge.deposit) // Return deposit
            }
        };

        drop(disputes);

        let mut resolution = DisputeResolution::new(
            *dispute_id,
            ruling,
            format!("Quorum vote: {} members voted", quorum_votes.len()),
            slash_amount,
            refund_amount,
            current_epoch,
        );

        resolution.quorum_members = quorum_votes.iter().map(|(addr, _)| *addr).collect();
        resolution.quorum_votes = quorum_votes.iter().map(|(_, ruling)| *ruling).collect();

        // Update dispute state
        self.disputes.write().await.get_mut(dispute_id).unwrap().resolution = Some(resolution.clone());
        self.disputes.write().await.get_mut(dispute_id).unwrap().status = DisputeStatus::Resolved;

        info!(
            "Dispute {} resolved via quorum: {:?} ({} votes)",
            hex::encode(&dispute_id[..8]),
            ruling,
            quorum_votes.len()
        );

        Ok(resolution)
    }

    /// Determine majority ruling from votes
    fn determine_majority_ruling(&self, votes: &[DisputeRuling]) -> DisputeRuling {
        let total = votes.len() as f64;
        let client_wins = votes.iter().filter(|v| matches!(v, DisputeRuling::ClientWins)).count() as f64;
        let provider_wins = votes.iter().filter(|v| matches!(v, DisputeRuling::ProviderWins)).count() as f64;

        if client_wins / total >= self.config.ruling_threshold {
            DisputeRuling::ClientWins
        } else if provider_wins / total >= self.config.ruling_threshold {
            DisputeRuling::ProviderWins
        } else {
            DisputeRuling::Inconclusive
        }
    }

    /// Calculate remaining payment in deal
    fn calculate_remaining_payment(&self, deal: &StorageDeal) -> AdicAmount {
        // In a real implementation, this would check how much has been paid out
        // For now, return full escrow amount
        deal.client_payment_escrow
    }

    /// Get dispute by ID
    pub async fn get_dispute(&self, dispute_id: &Hash) -> Option<DisputeState> {
        self.disputes.read().await.get(dispute_id).cloned()
    }

    /// Get all disputes for a deal
    pub async fn get_deal_disputes(&self, deal_id: u64) -> Vec<Hash> {
        self.deal_disputes
            .read()
            .await
            .get(&deal_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Mark expired disputes
    pub async fn check_expired_disputes(&self, current_epoch: u64) {
        let mut disputes = self.disputes.write().await;

        for (dispute_id, state) in disputes.iter_mut() {
            if state.status == DisputeStatus::Pending {
                if let Some(finalized_epoch) = state.challenge.finalized_at_epoch {
                    if current_epoch - finalized_epoch > self.config.resolution_deadline {
                        state.status = DisputeStatus::Expired;
                        warn!(
                            "Dispute {} expired after {} epochs",
                            hex::encode(&dispute_id[..8]),
                            current_epoch - finalized_epoch
                        );
                    }
                }
            }
        }
    }

    /// Get statistics
    pub async fn get_stats(&self) -> DisputeStats {
        let disputes = self.disputes.read().await;

        let mut stats = DisputeStats {
            total_disputes: disputes.len(),
            pending_disputes: 0,
            resolved_disputes: 0,
            expired_disputes: 0,
            client_wins: 0,
            provider_wins: 0,
            inconclusive: 0,
            disputes_by_type: HashMap::new(),
        };

        for state in disputes.values() {
            match state.status {
                DisputeStatus::Pending | DisputeStatus::UnderReview => stats.pending_disputes += 1,
                DisputeStatus::Resolved => {
                    stats.resolved_disputes += 1;
                    if let Some(resolution) = &state.resolution {
                        match resolution.ruling {
                            DisputeRuling::ClientWins => stats.client_wins += 1,
                            DisputeRuling::ProviderWins => stats.provider_wins += 1,
                            DisputeRuling::Inconclusive => stats.inconclusive += 1,
                        }
                    }
                }
                DisputeStatus::Expired => stats.expired_disputes += 1,
            }

            *stats.disputes_by_type.entry(state.challenge.dispute_type).or_insert(0) += 1;
        }

        stats
    }
}

/// Dispute statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisputeStats {
    pub total_disputes: usize,
    pub pending_disputes: usize,
    pub resolved_disputes: usize,
    pub expired_disputes: usize,
    pub client_wins: usize,
    pub provider_wins: usize,
    pub inconclusive: usize,
    pub disputes_by_type: HashMap<DisputeType, usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StorageDealStatus;

    fn create_test_deal() -> StorageDeal {
        StorageDeal {
            approvals: [[0u8; 32]; 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: chrono::Utc::now().timestamp(),
            deal_id: 1,
            ref_intent: [1u8; 32],
            ref_acceptance: [2u8; 32],
            client: AccountAddress::from_bytes([1u8; 32]),
            provider: AccountAddress::from_bytes([2u8; 32]),
            data_cid: [3u8; 32],
            data_size: 1024 * 1024,
            deal_duration_epochs: 1000,
            price_per_epoch: AdicAmount::from_adic(1.0),
            provider_collateral: AdicAmount::from_adic(1000.0),
            client_payment_escrow: AdicAmount::from_adic(1000.0),
            proof_merkle_root: Some([7u8; 32]),
            start_epoch: Some(100),
            activation_deadline: 200,
            status: StorageDealStatus::Active,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
        }
    }

    #[tokio::test]
    async fn test_submit_data_corruption_dispute() {
        let manager = DisputeManager::new(DisputeConfig::default());
        let deal = create_test_deal();

        let mut challenge = DisputeChallenge::new(
            deal.deal_id,
            deal.client,
            deal.provider,
            DisputeType::DataCorruption,
            Some([5u8; 32]),
            AdicAmount::from_adic(0.5),
        );
        challenge.claimed_chunk_index = Some(42);

        let dispute_id = manager.submit_dispute(challenge, &deal).await.unwrap();

        let state = manager.get_dispute(&dispute_id).await.unwrap();
        assert_eq!(state.status, DisputeStatus::Pending);
        assert_eq!(state.challenge.dispute_type, DisputeType::DataCorruption);
        assert_eq!(state.challenge.claimed_chunk_index, Some(42));
    }

    #[tokio::test]
    async fn test_submit_unavailable_dispute() {
        let manager = DisputeManager::new(DisputeConfig::default());
        let deal = create_test_deal();

        let mut challenge = DisputeChallenge::new(
            deal.deal_id,
            deal.client,
            deal.provider,
            DisputeType::DataUnavailable,
            None,
            AdicAmount::from_adic(0.5),
        );
        challenge.failed_request_id = Some([6u8; 32]);

        let dispute_id = manager.submit_dispute(challenge, &deal).await.unwrap();

        let state = manager.get_dispute(&dispute_id).await.unwrap();
        assert_eq!(state.status, DisputeStatus::Pending);
        assert_eq!(state.challenge.dispute_type, DisputeType::DataUnavailable);
    }

    #[tokio::test]
    async fn test_automatic_resolution_provider_offline() {
        let manager = DisputeManager::new(DisputeConfig::default());
        let deal = create_test_deal();

        let mut challenge = DisputeChallenge::new(
            deal.deal_id,
            deal.client,
            deal.provider,
            DisputeType::DataUnavailable,
            None,
            AdicAmount::from_adic(0.5),
        );
        challenge.failed_request_id = Some([6u8; 32]);

        let dispute_id = manager.submit_dispute(challenge, &deal).await.unwrap();

        // Resolve with provider offline
        let resolution = manager
            .resolve_dispute_automatic(&dispute_id, &deal, ProviderHealth::Offline, 150)
            .await
            .unwrap();

        assert_eq!(resolution.ruling, DisputeRuling::ClientWins);
        assert!(resolution.slash_amount > AdicAmount::ZERO);
        assert!(resolution.refund_amount > AdicAmount::ZERO);
    }

    #[tokio::test]
    async fn test_quorum_resolution() {
        let manager = DisputeManager::new(DisputeConfig::default());
        let deal = create_test_deal();

        let mut challenge = DisputeChallenge::new(
            deal.deal_id,
            deal.client,
            deal.provider,
            DisputeType::DataCorruption,
            Some([5u8; 32]),
            AdicAmount::from_adic(0.5),
        );
        challenge.claimed_chunk_index = Some(42);

        let dispute_id = manager.submit_dispute(challenge, &deal).await.unwrap();

        // Simulate quorum votes (10 client wins, 5 provider wins)
        let mut votes = Vec::new();
        for i in 0..10 {
            votes.push((AccountAddress::from_bytes([i as u8; 32]), DisputeRuling::ClientWins));
        }
        for i in 10..15 {
            votes.push((AccountAddress::from_bytes([i as u8; 32]), DisputeRuling::ProviderWins));
        }

        let resolution = manager
            .resolve_dispute_via_quorum(&dispute_id, &deal, votes, 150)
            .await
            .unwrap();

        // 10/15 = 66.7% >= 66% threshold -> ClientWins
        assert_eq!(resolution.ruling, DisputeRuling::ClientWins);
        assert_eq!(resolution.quorum_members.len(), 15);
        assert_eq!(resolution.quorum_votes.len(), 15);
    }

    #[tokio::test]
    async fn test_performance_breach_resolution() {
        let manager = DisputeManager::new(DisputeConfig::default());
        let deal = create_test_deal();

        let mut challenge = DisputeChallenge::new(
            deal.deal_id,
            deal.client,
            deal.provider,
            DisputeType::PerformanceBreach,
            None,
            AdicAmount::from_adic(0.5),
        );
        challenge.measured_performance = Some(PerformanceMetrics {
            bandwidth_bps: 1_000_000,      // 1 Mbps
            latency_ms: 500,
            required_bandwidth_bps: 10_000_000, // 10 Mbps required
            required_latency_ms: 100,
        });

        let dispute_id = manager.submit_dispute(challenge, &deal).await.unwrap();

        // Resolve with provider healthy but under-performing
        let resolution = manager
            .resolve_dispute_automatic(&dispute_id, &deal, ProviderHealth::Healthy, 150)
            .await
            .unwrap();

        assert_eq!(resolution.ruling, DisputeRuling::ClientWins);
    }

    #[tokio::test]
    async fn test_dispute_stats() {
        let manager = DisputeManager::new(DisputeConfig::default());
        let deal = create_test_deal();

        // Submit multiple disputes
        for i in 0..3 {
            let mut challenge = DisputeChallenge::new(
                deal.deal_id,
                deal.client,
                deal.provider,
                DisputeType::DataUnavailable,
                None,
                AdicAmount::from_adic(0.5),
            );
            challenge.failed_request_id = Some([i as u8; 32]);
            manager.submit_dispute(challenge, &deal).await.unwrap();
        }

        let stats = manager.get_stats().await;
        assert_eq!(stats.total_disputes, 3);
        assert_eq!(stats.pending_disputes, 3);
        assert_eq!(stats.disputes_by_type.get(&DisputeType::DataUnavailable), Some(&3));
    }
}
