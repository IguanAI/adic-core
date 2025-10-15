use adic_types::{MessageId, PublicKey};
use serde::{Deserialize, Serialize};

/// Challenge window configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeConfig {
    /// Window depth in epochs
    pub window_depth: u64,

    /// Fraud proof deposit factor (multiple of base deposit)
    pub fraud_proof_deposit_factor: u64,

    /// Dispute deposit factor
    pub dispute_deposit_factor: u64,

    /// Arbitrator minority penalty (ADIC-Rep adjustment)
    pub arbitrator_minority_penalty: f64,
}

impl Default for ChallengeConfig {
    fn default() -> Self {
        Self {
            window_depth: 100,
            fraud_proof_deposit_factor: 10,
            dispute_deposit_factor: 5,
            arbitrator_minority_penalty: -2.0,
        }
    }
}

/// Status of a challenge window
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChallengeStatus {
    /// Window is open for challenges
    Active,

    /// Fraud proof submitted, under verification
    Challenged,

    /// Window expired, no challenges
    Finalized,

    /// Fraud proven, subject slashed
    Slashed,
}

/// Ruling from dispute adjudication
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisputeRuling {
    /// Challenger wins (subject was guilty)
    ChallengerWins,

    /// Subject wins (fraud proof invalid)
    SubjectWins,

    /// Partial ruling
    Partial,

    /// Inconclusive
    Inconclusive,
}

/// Evidence for a fraud proof
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FraudEvidence {
    /// Type of fraud
    pub fraud_type: FraudType,

    /// Content-addressed evidence CID
    pub evidence_cid: String,

    /// Raw evidence data (optional)
    pub evidence_data: Option<Vec<u8>>,

    /// Metadata
    pub metadata: std::collections::HashMap<String, String>,
}

/// Types of fraud
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FraudType {
    /// Data corruption (storage)
    DataCorruption,

    /// Incorrect proof
    IncorrectProof,

    /// Task abandonment (PoUW)
    TaskAbandonment,

    /// Invalid computation result
    InvalidResult,

    /// Custom fraud type
    Custom(String),
}

/// Challenge metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeMetadata {
    pub id: String,
    pub subject_id: MessageId,
    pub challenger: PublicKey,
    pub submitted_at_epoch: u64,
    pub window_expiry_epoch: u64,
    pub status: ChallengeStatus,
}
