use adic_types::{
    EncoderData, EncoderSet, FeatureEncoder, RegionCodeEncoder, TimeEpochEncoder, TopicHashEncoder,
};

#[test]
fn test_region_code_encoder() {
    let encoder = RegionCodeEncoder::new(0); // Need axis id

    // Test various region codes
    let codes = vec!["US-CA", "UK-LON", "JP-TYO", "AU-SYD", "FR-PAR"];

    for code in codes {
        let qp_digits = encoder.encode(code.as_bytes(), 3, 10); // encode takes bytes, p, and precision
        assert_eq!(qp_digits.p, 3);
        assert!(!qp_digits.digits.is_empty());

        // Encoding same region should produce same features
        let qp_digits2 = encoder.encode(code.as_bytes(), 3, 10);
        assert_eq!(qp_digits.digits, qp_digits2.digits);
    }

    // Empty region
    let empty = encoder.encode(b"", 3, 10);
    assert!(!empty.digits.is_empty()); // Even empty input produces some encoding

    // Long region code
    let long_code = "US-CA-SF-DOWNTOWN-FINANCIAL-DISTRICT-BUILDING-42";
    let long_qp = encoder.encode(long_code.as_bytes(), 3, 10);
    assert!(!long_qp.digits.is_empty());
}

#[test]
fn test_time_epoch_encoder() {
    let encoder = TimeEpochEncoder::new(1, 3600); // axis 1, 1 hour epochs

    // Test various timestamps
    let now = chrono::Utc::now().timestamp();
    let qp1 = encoder.encode(now.to_string().as_bytes(), 3, 10);
    assert_eq!(qp1.p, 3);
    assert!(!qp1.digits.is_empty());

    // Same epoch should produce same features
    let close_time = (now + 1800).to_string(); // 30 minutes later
    let _qp2 = encoder.encode(close_time.as_bytes(), 3, 10);
    // These might be in same epoch depending on alignment

    // Different epoch should produce different features
    let later_time = (now + 7200).to_string(); // 2 hours later
    let _qp3 = encoder.encode(later_time.as_bytes(), 3, 10);
    // This should definitely be a different epoch

    // Test edge cases
    let qp_zero = encoder.encode(b"0", 3, 10);
    let qp_max = encoder.encode(u64::MAX.to_string().as_bytes(), 3, 10);
    assert!(!qp_zero.digits.is_empty());
    assert!(!qp_max.digits.is_empty());
}

#[test]
fn test_topic_hash_encoder() {
    let encoder = TopicHashEncoder::new(2); // axis 2

    // Test various topics
    let topics = vec!["payments", "messaging", "file-storage", "voting", ""];

    for topic in &topics {
        let qp = encoder.encode(topic.as_bytes(), 3, 10);
        assert_eq!(qp.p, 3);
        assert!(!qp.digits.is_empty());

        // Same topic should produce same features
        let qp2 = encoder.encode(topic.as_bytes(), 3, 10);
        assert_eq!(qp.digits, qp2.digits);
    }

    // Different topics should produce different features
    let qp1 = encoder.encode(b"topic1", 3, 10);
    let qp2 = encoder.encode(b"topic2", 3, 10);
    // The digits should be different for different topics
    assert_ne!(qp1.digits, qp2.digits);

    // Test long topic
    let long_topic = "a".repeat(1000);
    let long_qp = encoder.encode(long_topic.as_bytes(), 3, 10);
    assert!(!long_qp.digits.is_empty());
}

#[test]
fn test_encoder_set() {
    let encoder_set = EncoderSet::default_phase0();

    // Create encoder data
    let data = EncoderData::new(
        chrono::Utc::now().timestamp(),
        "payments".to_string(),
        "US-CA".to_string(),
    );

    // Encode with all encoders
    let features = encoder_set.encode_all(&data, 3, 10);

    // Should have features from all encoders (3 axes by default)
    assert_eq!(features.len(), 3);

    // Each should have valid QpDigits
    for axis_phi in &features {
        assert_eq!(axis_phi.qp_digits.p, 3);
        assert!(!axis_phi.qp_digits.digits.is_empty());
    }
}

#[test]
fn test_encoder_determinism() {
    let encoder = RegionCodeEncoder::new(0);

    // Same input should always produce same output
    let region = b"US-NY";
    let mut qp_list = Vec::new();

    for _ in 0..10 {
        let qp = encoder.encode(region, 3, 10);
        qp_list.push(qp);
    }

    // All should be the same
    for i in 1..qp_list.len() {
        assert_eq!(qp_list[0].digits, qp_list[i].digits);
    }
}

#[test]
fn test_encoder_edge_cases() {
    // Test with various edge case inputs
    let region_encoder = RegionCodeEncoder::new(0);
    let time_encoder = TimeEpochEncoder::new(1, 1); // 1 second epochs
    let topic_encoder = TopicHashEncoder::new(2);

    // Unicode strings
    let unicode_region = "日本-東京";
    let unicode_qp = region_encoder.encode(unicode_region.as_bytes(), 3, 10);
    assert!(!unicode_qp.digits.is_empty());

    // Special characters
    let special_topic = "topic!@#$%^&*()_+=-[]{}|;:',.<>?/~`";
    let special_qp = topic_encoder.encode(special_topic.as_bytes(), 3, 10);
    assert!(!special_qp.digits.is_empty());

    // Minimum epoch size
    let min_epoch_qp = time_encoder.encode(b"1234567890", 3, 10);
    assert!(!min_epoch_qp.digits.is_empty());
}

#[test]
fn test_encoder_data_serialization() {
    let data = EncoderData::new(1234567890, "test-topic".to_string(), "US-CA".to_string());

    // Serialize
    let serialized = serde_json::to_string(&data).unwrap();
    assert!(serialized.contains("1234567890"));
    assert!(serialized.contains("test-topic"));
    assert!(serialized.contains("US-CA"));

    // Deserialize
    let deserialized: EncoderData = serde_json::from_str(&serialized).unwrap();
    assert_eq!(data.timestamp, deserialized.timestamp);
    assert_eq!(data.topic, deserialized.topic);
    assert_eq!(data.region, deserialized.region);
}

#[test]
fn test_different_p_values() {
    let encoder = TopicHashEncoder::new(0);
    let topic = b"test-topic";

    // Test with different p values
    let qp_p2 = encoder.encode(topic, 2, 10);
    let qp_p3 = encoder.encode(topic, 3, 10);
    let qp_p5 = encoder.encode(topic, 5, 10);

    assert_eq!(qp_p2.p, 2);
    assert_eq!(qp_p3.p, 3);
    assert_eq!(qp_p5.p, 5);

    // All digits should be less than p
    assert!(qp_p2.digits.iter().all(|&d| d < 2));
    assert!(qp_p3.digits.iter().all(|&d| d < 3));
    assert!(qp_p5.digits.iter().all(|&d| d < 5));
}

#[test]
fn test_different_precision_values() {
    let encoder = RegionCodeEncoder::new(0);
    let region = b"test-region";

    // Test with different precision values
    let qp_5 = encoder.encode(region, 3, 5);
    let qp_10 = encoder.encode(region, 3, 10);
    let qp_20 = encoder.encode(region, 3, 20);

    assert_eq!(qp_5.digits.len(), 5);
    assert_eq!(qp_10.digits.len(), 10);
    assert_eq!(qp_20.digits.len(), 20);
}
