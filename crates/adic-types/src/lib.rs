pub mod message;
pub mod id;
pub mod features;
pub mod keys;
pub mod error;
pub mod encoders;

pub use message::{AdicMessage, AdicMeta, ConflictId};
pub use id::MessageId;
pub use features::{AdicFeatures, AxisPhi, QpDigits, AxisId};
pub use keys::{PublicKey, Signature};
pub use error::{AdicError, Result};
pub use encoders::{FeatureEncoder, TimeEpochEncoder, TopicHashEncoder, RegionCodeEncoder, EncoderSet, EncoderData};

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
    pub gamma: f64,  // Reputation update factor (0 < gamma < 1)
}

impl Default for AdicParams {
    fn default() -> Self {
        Self {
            p: 3,
            d: 3,
            rho: vec![2, 2, 1],
            q: 3,
            k: 20,
            depth_star: 12,
            delta: 5,
            deposit: 0.1,
            r_min: 1.0,
            r_sum_min: 4.0,
            lambda: 1.0,
            beta: 0.5,
            mu: 1.0,
            gamma: 0.9,  // Default reputation update factor
        }
    }
}