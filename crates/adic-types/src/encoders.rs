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

    pub fn encode_all(&self, data: &EncoderData, p: u32, precision: usize) -> Vec<AxisPhi> {
        self.encoders
            .iter()
            .map(|encoder| {
                let value = match encoder.name() {
                    "time_epoch" => data.timestamp.to_string().into_bytes(),
                    "topic_hash" => data.topic.as_bytes().to_vec(),
                    "region_code" => data.region.as_bytes().to_vec(),
                    _ => vec![],
                };

                AxisPhi::new(encoder.axis_id().0, encoder.encode(&value, p, precision))
            })
            .collect()
    }
}

pub struct EncoderData {
    pub timestamp: i64,
    pub topic: String,
    pub region: String,
}

impl EncoderData {
    pub fn new(timestamp: i64, topic: String, region: String) -> Self {
        Self {
            timestamp,
            topic,
            region,
        }
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
}
