use adic_types::{MessageId, PublicKey};
use serde::{Deserialize, Serialize};

/// Unique identifier for an escrow lock
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LockId(pub String);

impl LockId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn storage_collateral(deal_id: u64, provider: &PublicKey) -> Self {
        Self(format!(
            "storage_collateral_{}_{}",
            deal_id,
            hex::encode(&provider.as_bytes()[..8])
        ))
    }

    pub fn storage_payment(deal_id: u64, client: &PublicKey) -> Self {
        Self(format!(
            "storage_payment_{}_{}",
            deal_id,
            hex::encode(&client.as_bytes()[..8])
        ))
    }

    pub fn pouw_deposit(hook_id: MessageId, worker: &PublicKey) -> Self {
        Self(format!(
            "pouw_deposit_{}_{}",
            hex::encode(hook_id.as_bytes()),
            hex::encode(&worker.as_bytes()[..8])
        ))
    }

    pub fn treasury_grant(proposal_id: MessageId, recipient: &PublicKey) -> Self {
        Self(format!(
            "treasury_grant_{}_{}",
            hex::encode(proposal_id.as_bytes()),
            hex::encode(&recipient.as_bytes()[..8])
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for LockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// State event that triggers a lifecycle transition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StateEvent {
    /// Message achieved finality
    FinalityAchieved,

    /// Message was accepted/approved
    Accepted,

    /// Message was rejected
    Rejected,

    /// Timeout occurred
    Timeout,

    /// External condition met
    ConditionMet(String),

    /// Explicit cancellation
    Cancelled,

    /// Custom event
    Custom(String),
}

/// Quorum configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuorumConfig {
    pub min_reputation: f64,
    pub members_per_axis: usize,
    pub total_size: usize,
    pub max_per_asn: usize,
    pub max_per_region: usize,
    pub domain_separator: String,
}

impl Default for QuorumConfig {
    fn default() -> Self {
        Self {
            min_reputation: 50.0,
            members_per_axis: 5,
            total_size: 15,
            max_per_asn: 2,
            max_per_region: 3,
            domain_separator: "default".to_string(),
        }
    }
}

/// Result of a quorum vote
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuorumResult {
    pub committee: Vec<PublicKey>,
    pub votes_for: usize,
    pub votes_against: usize,
    pub passed: bool,
    pub threshold_met: bool,
}
