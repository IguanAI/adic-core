pub mod encoders;
pub mod error;
pub mod features;
pub mod id;
pub mod keys;
pub mod message;

pub use encoders::{
    EncoderData, EncoderSet, FeatureEncoder, RegionCodeEncoder, TimeEpochEncoder, TopicHashEncoder,
};
pub use error::{AdicError, Result};
pub use features::{AdicFeatures, AxisId, AxisPhi, QpDigits};
pub use id::MessageId;
pub use keys::{PublicKey, Signature};
pub use message::{AdicMessage, AdicMeta, ConflictId};

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
    pub beta: f64,
    pub mu: f64,
    pub gamma: f64, // Reputation update factor (0 < gamma < 1)
}

impl Default for AdicParams {
    fn default() -> Self {
        Self {
            p: DEFAULT_P,
            d: 3,
            rho: vec![2, 2, 1],
            q: 3,
            k: 3,          // Lowered for testing - was 20
            depth_star: 2, // Lowered for testing - was 12
            delta: 5,
            deposit: 0.1,
            r_min: 1.0,
            r_sum_min: 4.0,
            lambda: 1.0,
            beta: 0.5,
            mu: 1.0,
            gamma: 0.9, // Default reputation update factor
        }
    }
}
