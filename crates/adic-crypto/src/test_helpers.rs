//! Test helper utilities for crypto module testing

use adic_types::{AdicFeatures, AdicMessage, AdicMeta, AxisPhi, MessageId, PublicKey, QpDigits};
use chrono::Utc;
use rand::{thread_rng, Rng};

/// Generate a random QpDigits value for testing
pub fn random_qp_digits(p: u32, precision: usize) -> QpDigits {
    let mut rng = thread_rng();
    let value = rng.gen_range(0..1000000);
    QpDigits::from_u64(value, p, precision)
}

/// Generate a test message with specified number of parents
pub fn create_test_message_with_parents(num_parents: usize) -> AdicMessage {
    let parents: Vec<MessageId> = (0..num_parents)
        .map(|i| MessageId::new(&[i as u8; 32]))
        .collect();

    let features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(42, 3, 5)),
        AxisPhi::new(1, QpDigits::from_u64(100, 3, 5)),
        AxisPhi::new(2, QpDigits::from_u64(7, 3, 5)),
    ]);

    AdicMessage::new(
        parents,
        features,
        AdicMeta::new(Utc::now()),
        PublicKey::from_bytes([1; 32]),
        vec![],
    )
}

/// Generate parent features with specified diversity
pub fn create_diverse_parent_features(
    num_parents: usize,
    num_axes: usize,
    p: u32,
    ensure_diverse: bool,
) -> Vec<Vec<QpDigits>> {
    let mut parent_features = Vec::new();

    for i in 0..num_parents {
        let mut features = Vec::new();
        for j in 0..num_axes {
            let value = if ensure_diverse {
                // Ensure diversity by using different values for each parent
                (i * num_axes + j) as u64
            } else {
                // Use same values to violate diversity
                j as u64
            };
            features.push(QpDigits::from_u64(value, p, 5));
        }
        parent_features.push(features);
    }

    parent_features
}

/// Generate corrupted encrypted data for testing error handling
pub fn corrupt_encrypted_data(data: &[u8], corruption_type: CorruptionType) -> Vec<u8> {
    let mut corrupted = data.to_vec();

    match corruption_type {
        CorruptionType::Truncate => {
            if corrupted.len() > 10 {
                corrupted.truncate(corrupted.len() / 2);
            }
        }
        CorruptionType::FlipBits => {
            if !corrupted.is_empty() {
                corrupted[0] ^= 0xFF;
                if corrupted.len() > 1 {
                    let last_idx = corrupted.len() - 1;
                    corrupted[last_idx] ^= 0xFF;
                }
            }
        }
        CorruptionType::Append => {
            corrupted.extend_from_slice(&[0xFF; 10]);
        }
        CorruptionType::Empty => {
            corrupted.clear();
        }
    }

    corrupted
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum CorruptionType {
    Truncate,
    FlipBits,
    Append,
    Empty,
}

/// Generate invalid parameters for testing validation
#[allow(dead_code)]
pub struct InvalidParams {
    pub zero_prime: u32,
    pub composite_prime: u32,
    pub zero_precision: usize,
    pub huge_precision: usize,
    pub empty_rho: Vec<u32>,
    pub zero_radius: Vec<u32>,
    pub mismatched_rho: Vec<u32>,
}

impl InvalidParams {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            zero_prime: 0,
            composite_prime: 15, // 3 * 5
            zero_precision: 0,
            huge_precision: 100000,
            empty_rho: vec![],
            zero_radius: vec![0, 1, 2],
            mismatched_rho: vec![1, 2], // Wrong length for d=3
        }
    }
}

/// Create a deterministic QpDigits for reproducible tests
#[allow(dead_code)]
pub fn deterministic_qp_digits(seed: u64, p: u32, precision: usize) -> QpDigits {
    // Use the seed directly as the value
    QpDigits::from_u64(seed, p, precision)
}

/// Measure execution time for performance testing
#[allow(dead_code)]
pub fn measure_time<F, R>(f: F) -> (R, std::time::Duration)
where
    F: FnOnce() -> R,
{
    let start = std::time::Instant::now();
    let result = f();
    let duration = start.elapsed();
    (result, duration)
}

/// Assert that two QpDigits are approximately equal (for floating point operations)
#[allow(dead_code)]
pub fn assert_qp_approx_equal(a: &QpDigits, b: &QpDigits, tolerance: usize) {
    assert_eq!(a.p, b.p, "Prime mismatch");
    assert_eq!(a.digits.len(), b.digits.len(), "Precision mismatch");

    let mut diff_count = 0;
    for (da, db) in a.digits.iter().zip(b.digits.iter()) {
        if da != db {
            diff_count += 1;
        }
    }

    assert!(
        diff_count <= tolerance,
        "QpDigits differ in {} positions, exceeds tolerance of {}",
        diff_count,
        tolerance
    );
}

/// Generate test vectors for known good values
#[allow(dead_code)]
pub struct TestVectors {
    pub known_good_keys: Vec<QpDigits>,
    pub known_good_ciphertexts: Vec<Vec<u8>>,
    pub known_plaintexts: Vec<Vec<u8>>,
}

impl TestVectors {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            known_good_keys: vec![
                QpDigits::from_u64(42, 3, 10),
                QpDigits::from_u64(123456, 251, 16),
                QpDigits::from_u64(999999, 5, 20),
            ],
            known_good_ciphertexts: vec![
                vec![0x01, 0x23, 0x45, 0x67],
                vec![0x89, 0xAB, 0xCD, 0xEF],
            ],
            known_plaintexts: vec![
                b"Hello".to_vec(),
                b"World".to_vec(),
                b"Test message with special chars: !@#$%^&*()".to_vec(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_helpers_work() {
        let qp = random_qp_digits(3, 10);
        assert_eq!(qp.p, 3);
        assert_eq!(qp.digits.len(), 10);

        let msg = create_test_message_with_parents(4);
        assert_eq!(msg.parents.len(), 4);

        let diverse = create_diverse_parent_features(3, 3, 3, true);
        assert_eq!(diverse.len(), 3);
        assert_eq!(diverse[0].len(), 3);
    }
}
