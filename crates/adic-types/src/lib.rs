pub mod canonical_json;
pub mod content;
pub mod encoders;
pub mod error;
pub mod features;
pub mod id;
pub mod keys;
pub mod message;
pub mod transfer;

pub use canonical_json::{
    canonical_hash, to_canonical_json, verify_canonical_match, CanonicalJsonError,
};
pub use content::MessageContent;
pub use encoders::{
    EncoderData, EncoderSet, FeatureEncoder, RegionCodeEncoder, TimeEpochEncoder, TopicHashEncoder,
};
pub use error::{AdicError, Result};
pub use features::{AdicFeatures, AxisId, AxisPhi, QpDigits};
pub use id::MessageId;
pub use keys::{PublicKey, Signature};
pub use message::{dst, AdicMessage, AdicMeta, ConflictId};
pub use transfer::ValueTransfer;

// Re-export governance types when feature is enabled
#[cfg(feature = "governance")]
pub use content::{
    AxisAction, AxisChanges, AxisSpec, Ballot, GovernanceProposalContent, GovernanceReceiptContent,
    GovernanceVoteContent, ProposalClass, QuorumStats, VoteResult,
};

// Re-export PoUW types when feature is enabled
#[cfg(feature = "pouw")]
pub use content::{
    ExecutionMetrics, PoUWReceiptContent, PoUWResultContent, PoUWTaskContent,
};

/// Default precision for p-adic digit representations
/// This determines how many p-adic digits are used to represent values
pub const DEFAULT_PRECISION: usize = 10;

/// Default prime base for the p-adic number system
/// Using p=3 gives us a ternary (base-3) representation
pub const DEFAULT_P: u32 = 3;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AdicParams {
    pub p: u32,
    pub d: u32,
    pub rho: Vec<u32>,
    pub q: u32,
    pub k: u32,
    pub depth_star: u32,
    pub delta: u32,
    pub deposit: f64,
    pub r_min: f64,
    pub r_sum_min: f64,
    pub lambda: f64,
    pub alpha: f64,  // MRW reputation exponent (default 1.0 per PDF spec)
    pub beta: f64,
    pub mu: f64,
    pub gamma: f64,  // Reputation update factor (0 < gamma < 1)
}

impl Default for AdicParams {
    fn default() -> Self {
        Self {
            p: DEFAULT_P,
            d: 3,
            rho: vec![2, 2, 1],
            q: 3,
            k: 20,          // Production value from ADIC-DAG paper Section 1.2
            depth_star: 12, // Production value from ADIC-DAG paper Section 1.2
            delta: 5,
            deposit: 0.1,
            r_min: 1.0,
            r_sum_min: 4.0,
            lambda: 1.0,
            alpha: 1.0,     // MRW reputation exponent (α), default per ADIC-DAG paper Section 1.2
            beta: 1.0,      // MRW age decay exponent (β), default per ADIC-DAG paper Section 1.2
            mu: 1.0,
            gamma: 0.9,     // Default reputation update factor
        }
    }
}

impl AdicParams {
    /// Create test parameters with lowered thresholds for faster testing
    pub fn test() -> Self {
        Self {
            p: DEFAULT_P,
            d: 3,
            rho: vec![2, 2, 1],
            q: 3,
            k: 3,          // Lowered for testing (production: 20)
            depth_star: 2, // Lowered for testing (production: 12)
            delta: 5,
            deposit: 0.1,
            r_min: 1.0,
            r_sum_min: 4.0,
            lambda: 1.0,
            alpha: 1.0,    // MRW reputation exponent (α)
            beta: 1.0,     // MRW age decay exponent (β)
            mu: 1.0,
            gamma: 0.9,
        }
    }
}
