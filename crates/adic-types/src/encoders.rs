use crate::features::{AxisId, AxisPhi, QpDigits};
use blake3::Hasher;
use chrono::{DateTime, Utc};
use num_bigint::BigUint;

pub trait FeatureEncoder {
    fn encode(&self, value: &[u8], p: u32, precision: usize) -> QpDigits;
    fn axis_id(&self) -> AxisId;
    fn name(&self) -> &str;
}

pub struct TimeEpochEncoder {
    axis: u32,
    bucket_size: u64,
}

impl TimeEpochEncoder {
    pub fn new(axis: u32, bucket_size: u64) -> Self {
        Self { axis, bucket_size }
    }

    pub fn encode_timestamp(&self, timestamp: DateTime<Utc>, p: u32, precision: usize) -> QpDigits {
        let epoch = timestamp.timestamp() as u64;
        let bucket = epoch / self.bucket_size;
        QpDigits::from_u64(bucket, p, precision)
    }
}

impl FeatureEncoder for TimeEpochEncoder {
    fn encode(&self, value: &[u8], p: u32, precision: usize) -> QpDigits {
        let timestamp_str = std::str::from_utf8(value).unwrap_or("0");
        let timestamp = timestamp_str.parse::<i64>().unwrap_or(0);
        let bucket = (timestamp as u64) / self.bucket_size;
        QpDigits::from_u64(bucket, p, precision)
    }

    fn axis_id(&self) -> AxisId {
        AxisId(self.axis)
    }

    fn name(&self) -> &str {
        "time_epoch"
    }
}

pub struct TopicHashEncoder {
    axis: u32,
}

impl TopicHashEncoder {
    pub fn new(axis: u32) -> Self {
        Self { axis }
    }

    pub fn encode_topic(&self, topic: &str, p: u32, precision: usize) -> QpDigits {
        let mut hasher = Hasher::new();
        hasher.update(topic.as_bytes());
        let hash = hasher.finalize();

        let mut value = BigUint::from_bytes_be(hash.as_bytes());
        let p_big = BigUint::from(p);
        let max_value = p_big.pow(precision as u32);
        value %= max_value;

        QpDigits::new(&value, p, precision)
    }
}

impl FeatureEncoder for TopicHashEncoder {
    fn encode(&self, value: &[u8], p: u32, precision: usize) -> QpDigits {
        let mut hasher = Hasher::new();
        hasher.update(value);
        let hash = hasher.finalize();

        let mut hash_value = BigUint::from_bytes_be(hash.as_bytes());
        let p_big = BigUint::from(p);
        let max_value = p_big.pow(precision as u32);
        hash_value %= max_value;

        QpDigits::new(&hash_value, p, precision)
    }

    fn axis_id(&self) -> AxisId {
        AxisId(self.axis)
    }

    fn name(&self) -> &str {
        "topic_hash"
    }
}

pub struct RegionCodeEncoder {
    axis: u32,
}

impl RegionCodeEncoder {
    pub fn new(axis: u32) -> Self {
        Self { axis }
    }

    pub fn encode_region(&self, region_code: &str, p: u32, precision: usize) -> QpDigits {
        let value = match region_code {
            "US" => 1,
            "EU" => 2,
            "ASIA" => 3,
            "AF" => 4,
            "SA" => 5,
            "OC" => 6,
            _ => {
                let mut hasher = Hasher::new();
                hasher.update(region_code.as_bytes());
                let hash = hasher.finalize();
                (hash.as_bytes()[0] as u64) % 100 + 7
            }
        };

        QpDigits::from_u64(value, p, precision)
    }
}

impl FeatureEncoder for RegionCodeEncoder {
    fn encode(&self, value: &[u8], p: u32, precision: usize) -> QpDigits {
        let region_str = std::str::from_utf8(value).unwrap_or("UNKNOWN");
        self.encode_region(region_str, p, precision)
    }

    fn axis_id(&self) -> AxisId {
        AxisId(self.axis)
    }

    fn name(&self) -> &str {
        "region_code"
    }
}

/// Encoder for stake/service tier - maps stake amounts to discrete tiers (0-9)
/// Per ADIC-DAG paper Appendix A: Stake/Service Tier axis
pub struct StakeTierEncoder {
    axis: u32,
    /// Stake thresholds for each tier (in ADIC tokens)
    tier_thresholds: Vec<u64>,
}

impl StakeTierEncoder {
    pub fn new(axis: u32) -> Self {
        // Default tier thresholds (logarithmic scale)
        Self {
            axis,
            tier_thresholds: vec![
                0,           // Tier 0: 0-9 ADIC
                10,          // Tier 1: 10-99 ADIC
                100,         // Tier 2: 100-999 ADIC
                1_000,       // Tier 3: 1K-9.9K ADIC
                10_000,      // Tier 4: 10K-99.9K ADIC
                100_000,     // Tier 5: 100K-999K ADIC
                1_000_000,   // Tier 6: 1M-9.9M ADIC
                10_000_000,  // Tier 7: 10M-99.9M ADIC
                100_000_000, // Tier 8: 100M+ ADIC
            ],
        }
    }

    pub fn new_with_thresholds(axis: u32, thresholds: Vec<u64>) -> Self {
        Self {
            axis,
            tier_thresholds: thresholds,
        }
    }

    pub fn encode_stake(&self, stake_amount: u64, p: u32, precision: usize) -> QpDigits {
        // Find the tier for this stake amount
        let tier = self
            .tier_thresholds
            .iter()
            .rev()
            .position(|&threshold| stake_amount >= threshold)
            .map(|pos| self.tier_thresholds.len() - 1 - pos)
            .unwrap_or(0) as u64;

        QpDigits::from_u64(tier, p, precision)
    }
}

impl FeatureEncoder for StakeTierEncoder {
    fn encode(&self, value: &[u8], p: u32, precision: usize) -> QpDigits {
        let stake_str = std::str::from_utf8(value).unwrap_or("0");
        let stake_amount = stake_str.parse::<u64>().unwrap_or(0);
        self.encode_stake(stake_amount, p, precision)
    }

    fn axis_id(&self) -> AxisId {
        AxisId(self.axis)
    }

    fn name(&self) -> &str {
        "stake_tier"
    }
}

/// Encoder for Autonomous System Numbers (ASN) - network topology diversity
/// Per ADIC-DAG paper Appendix A: Region/ASN axis
pub struct AsnEncoder {
    axis: u32,
}

impl AsnEncoder {
    pub fn new(axis: u32) -> Self {
        Self { axis }
    }

    pub fn encode_asn(&self, asn: u32, p: u32, precision: usize) -> QpDigits {
        // Map ASN to p-adic space
        // ASNs are 32-bit, we hash them to fit in our precision
        let asn_value = (asn as u64) % (p.pow(precision as u32) as u64);
        QpDigits::from_u64(asn_value, p, precision)
    }

    pub fn encode_asn_str(&self, asn_str: &str, p: u32, precision: usize) -> QpDigits {
        // Parse ASN from string (e.g., "AS13335" or "13335")
        let asn_num = asn_str
            .trim_start_matches("AS")
            .trim_start_matches("as")
            .parse::<u32>()
            .unwrap_or_else(|_| {
                // If parsing fails, hash the string
                let mut hasher = Hasher::new();
                hasher.update(asn_str.as_bytes());
                let hash = hasher.finalize();
                u32::from_be_bytes([
                    hash.as_bytes()[0],
                    hash.as_bytes()[1],
                    hash.as_bytes()[2],
                    hash.as_bytes()[3],
                ])
            });

        self.encode_asn(asn_num, p, precision)
    }
}

impl FeatureEncoder for AsnEncoder {
    fn encode(&self, value: &[u8], p: u32, precision: usize) -> QpDigits {
        let asn_str = std::str::from_utf8(value).unwrap_or("0");
        self.encode_asn_str(asn_str, p, precision)
    }

    fn axis_id(&self) -> AxisId {
        AxisId(self.axis)
    }

    fn name(&self) -> &str {
        "asn"
    }
}

pub struct EncoderSet {
    encoders: Vec<Box<dyn FeatureEncoder + Send + Sync>>,
}

impl EncoderSet {
    pub fn default_phase0() -> Self {
        Self {
            encoders: vec![
                Box::new(TimeEpochEncoder::new(0, 60)),
                Box::new(TopicHashEncoder::new(1)),
                Box::new(RegionCodeEncoder::new(2)),
            ],
        }
    }

    /// Create a full 4-axis encoder set (complete per ADIC-DAG paper)
    pub fn default_full() -> Self {
        Self {
            encoders: vec![
                Box::new(TimeEpochEncoder::new(0, 60)),
                Box::new(TopicHashEncoder::new(1)),
                Box::new(RegionCodeEncoder::new(2)),
                Box::new(StakeTierEncoder::new(3)),
            ],
        }
    }

    /// Create encoder set with ASN instead of region
    pub fn with_asn() -> Self {
        Self {
            encoders: vec![
                Box::new(TimeEpochEncoder::new(0, 60)),
                Box::new(TopicHashEncoder::new(1)),
                Box::new(AsnEncoder::new(2)),
                Box::new(StakeTierEncoder::new(3)),
            ],
        }
    }

    pub fn encode_all(&self, data: &EncoderData, p: u32, precision: usize) -> Vec<AxisPhi> {
        self.encoders
            .iter()
            .map(|encoder| {
                let value = match encoder.name() {
                    "time_epoch" => data.timestamp.to_string().into_bytes(),
                    "topic_hash" => data.topic.as_bytes().to_vec(),
                    "region_code" => data.region.as_bytes().to_vec(),
                    "stake_tier" => data.stake_amount.to_string().into_bytes(),
                    "asn" => data.asn.as_bytes().to_vec(),
                    _ => vec![],
                };

                AxisPhi::new(encoder.axis_id().0, encoder.encode(&value, p, precision))
            })
            .collect()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EncoderData {
    pub timestamp: i64,
    pub topic: String,
    pub region: String,
    pub stake_amount: u64,
    pub asn: String,
}

impl EncoderData {
    pub fn new(timestamp: i64, topic: String, region: String) -> Self {
        Self {
            timestamp,
            topic,
            region,
            stake_amount: 0,
            asn: "0".to_string(),
        }
    }

    pub fn with_stake(mut self, stake_amount: u64) -> Self {
        self.stake_amount = stake_amount;
        self
    }

    pub fn with_asn(mut self, asn: String) -> Self {
        self.asn = asn;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_encoder() {
        let encoder = TimeEpochEncoder::new(0, 60);
        let timestamp = Utc::now();
        let encoded = encoder.encode_timestamp(timestamp, 3, 5);
        assert_eq!(encoded.p, 3);
        assert_eq!(encoded.digits.len(), 5);
    }

    #[test]
    fn test_topic_encoder() {
        let encoder = TopicHashEncoder::new(1);
        let topic1 = encoder.encode_topic("test_topic", 3, 5);
        let topic2 = encoder.encode_topic("test_topic", 3, 5);
        assert_eq!(topic1, topic2);

        let topic3 = encoder.encode_topic("different", 3, 5);
        assert_ne!(topic1, topic3);
    }

    #[test]
    fn test_region_encoder() {
        let encoder = RegionCodeEncoder::new(2);
        let us = encoder.encode_region("US", 3, 5);
        let eu = encoder.encode_region("EU", 3, 5);
        assert_ne!(us, eu);

        let unknown = encoder.encode_region("UNKNOWN", 3, 5);
        assert_ne!(unknown, us);
    }

    #[test]
    fn test_encoder_set() {
        let encoders = EncoderSet::default_phase0();
        let data = EncoderData::new(1234567890, "test".to_string(), "US".to_string());

        let encoded = encoders.encode_all(&data, 3, 5);
        assert_eq!(encoded.len(), 3);

        for phi in encoded {
            assert_eq!(phi.qp_digits.p, 3);
            assert_eq!(phi.qp_digits.digits.len(), 5);
        }
    }

    #[test]
    fn test_stake_tier_encoder() {
        let encoder = StakeTierEncoder::new(3);

        // Test tier boundaries
        let tier0 = encoder.encode_stake(5, 3, 5);
        let tier1 = encoder.encode_stake(50, 3, 5);
        let tier2 = encoder.encode_stake(500, 3, 5);
        let tier3 = encoder.encode_stake(5_000, 3, 5);

        // Tiers should be different
        assert_ne!(tier0, tier1);
        assert_ne!(tier1, tier2);
        assert_ne!(tier2, tier3);

        // Same tier should encode the same
        let tier1_a = encoder.encode_stake(10, 3, 5);
        let tier1_b = encoder.encode_stake(99, 3, 5);
        assert_eq!(tier1_a, tier1_b);

        // Test via trait
        let encoded = encoder.encode(b"100000", 3, 5);
        assert_eq!(encoded.p, 3);
        assert_eq!(encoded.digits.len(), 5);
    }

    #[test]
    fn test_asn_encoder() {
        let encoder = AsnEncoder::new(2);

        // Test ASN encoding
        let asn1 = encoder.encode_asn(13335, 3, 5); // Cloudflare
        let asn2 = encoder.encode_asn(15169, 3, 5); // Google
        assert_ne!(asn1, asn2);

        // Test string parsing
        let asn_with_prefix = encoder.encode_asn_str("AS13335", 3, 5);
        let asn_without_prefix = encoder.encode_asn_str("13335", 3, 5);
        assert_eq!(asn_with_prefix, asn_without_prefix);

        // Test same ASN encodes the same
        let asn_a = encoder.encode_asn(13335, 3, 5);
        let asn_b = encoder.encode_asn(13335, 3, 5);
        assert_eq!(asn_a, asn_b);

        // Test via trait
        let encoded = encoder.encode(b"AS15169", 3, 5);
        assert_eq!(encoded.p, 3);
        assert_eq!(encoded.digits.len(), 5);
    }

    #[test]
    fn test_full_encoder_set() {
        let encoders = EncoderSet::default_full();
        let data =
            EncoderData::new(1234567890, "test".to_string(), "US".to_string()).with_stake(50_000);

        let encoded = encoders.encode_all(&data, 3, 5);
        assert_eq!(encoded.len(), 4); // All 4 axes

        for phi in encoded {
            assert_eq!(phi.qp_digits.p, 3);
            assert_eq!(phi.qp_digits.digits.len(), 5);
        }
    }

    #[test]
    fn test_asn_encoder_set() {
        let encoders = EncoderSet::with_asn();
        let data = EncoderData::new(1234567890, "test".to_string(), "US".to_string())
            .with_asn("AS13335".to_string())
            .with_stake(1_000_000);

        let encoded = encoders.encode_all(&data, 3, 5);
        assert_eq!(encoded.len(), 4); // All 4 axes with ASN instead of region

        for phi in encoded {
            assert_eq!(phi.qp_digits.p, 3);
            assert_eq!(phi.qp_digits.digits.len(), 5);
        }
    }

    #[test]
    fn test_stake_tier_boundaries() {
        let encoder = StakeTierEncoder::new(3);

        // Test exact tier boundaries
        assert_eq!(
            encoder.encode_stake(0, 3, 5),
            encoder.encode_stake(9, 3, 5),
            "Should be in tier 0"
        );
        assert_ne!(
            encoder.encode_stake(9, 3, 5),
            encoder.encode_stake(10, 3, 5),
            "Should cross tier boundary"
        );
        assert_eq!(
            encoder.encode_stake(10, 3, 5),
            encoder.encode_stake(99, 3, 5),
            "Should be in tier 1"
        );
    }
}
