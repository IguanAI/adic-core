use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AxisPhi, ConflictId, MessageId, PublicKey, QpDigits,
    Signature,
};
use chrono::{TimeZone, Utc};

#[test]
fn test_message_id_computation() {
    let payload = b"test content";
    let parents = vec![MessageId::new(b"parent1"), MessageId::new(b"parent2")];
    let features = AdicFeatures::new(vec![]);
    let meta = AdicMeta::new(Utc::now());
    let proposer_pk = PublicKey::from_bytes([1u8; 32]);

    let msg1 = AdicMessage::new(
        parents.clone(),
        features.clone(),
        meta.clone(),
        proposer_pk,
        payload.to_vec(),
    );
    let id1 = msg1.id;

    let msg2 = AdicMessage::new(
        parents.clone(),
        features.clone(),
        meta.clone(),
        proposer_pk,
        payload.to_vec(),
    );
    let id2 = msg2.id;

    // Same inputs should produce same ID
    assert_eq!(id1, id2);

    // Different payload should produce different ID
    let msg3 = AdicMessage::new(
        parents.clone(),
        features.clone(),
        meta.clone(),
        proposer_pk.clone(),
        b"different".to_vec(),
    );
    let id3 = msg3.id;
    assert_ne!(id1, id3);

    // Different parents should produce different ID
    let parents2 = vec![MessageId::new(b"other")];
    let msg4 = AdicMessage::new(parents2, features, meta, proposer_pk, payload.to_vec());
    let id4 = msg4.id;
    assert_ne!(id1, id4);
}

#[test]
fn test_message_id_edge_cases() {
    let features = AdicFeatures::new(vec![]);
    let meta = AdicMeta::new(Utc.timestamp_opt(0, 0).unwrap());
    let proposer_pk = PublicKey::from_bytes([0u8; 32]);

    // Empty payload
    let msg1 = AdicMessage::new(
        vec![],
        features.clone(),
        meta.clone(),
        proposer_pk.clone(),
        b"".to_vec(),
    );
    let id1 = msg1.id;
    assert_ne!(id1.as_bytes(), &[0u8; 32]);

    // Large payload
    let large_payload = vec![0xFFu8; 10000];
    let msg2 = AdicMessage::new(
        vec![],
        features.clone(),
        meta.clone(),
        proposer_pk.clone(),
        large_payload,
    );
    let id2 = msg2.id;
    assert_ne!(id2.as_bytes(), &[0u8; 32]);

    // Large timestamp (not MAX to avoid overflow)
    let meta_large = AdicMeta::new(Utc.timestamp_opt(2_000_000_000, 0).unwrap());
    let msg3 = AdicMessage::new(vec![], features, meta_large, proposer_pk, b"test".to_vec());
    let id3 = msg3.id;
    assert_ne!(id3.as_bytes(), &[0u8; 32]);
}

#[test]
fn test_message_serialization() {
    let meta = AdicMeta::new(Utc.timestamp_opt(1234567890, 0).unwrap());
    let proposer_pk = PublicKey::from_bytes([1u8; 32]);
    let signature = Signature::new(vec![2u8; 64]);
    let mut msg = AdicMessage::new(
        vec![MessageId::new(b"parent1")],
        AdicFeatures::new(vec![]),
        meta,
        proposer_pk,
        b"test content".to_vec(),
    );
    msg.signature = signature;

    // Serialize
    let serialized = serde_json::to_string(&msg).expect("Failed to serialize");
    assert!(!serialized.is_empty());

    // Deserialize
    let deserialized: AdicMessage =
        serde_json::from_str(&serialized).expect("Failed to deserialize");

    assert_eq!(msg.id, deserialized.id);
    assert_eq!(msg.payload, deserialized.payload);
    assert_eq!(msg.parents, deserialized.parents);
    assert_eq!(msg.meta.timestamp, deserialized.meta.timestamp);
}

#[test]
fn test_message_validation() {
    let meta = AdicMeta::new(Utc.timestamp_opt(1000, 0).unwrap());
    let proposer_pk = PublicKey::from_bytes([1u8; 32]);
    let signature = Signature::new(vec![0u8; 64]);

    let mut msg = AdicMessage::new(
        vec![],
        AdicFeatures::new(vec![]),
        meta,
        proposer_pk,
        b"content".to_vec(),
    );
    msg.signature = signature;

    // Should validate with correct ID
    assert!(msg.verify_id());

    // Change payload - should invalidate
    msg.payload = b"different".to_vec();
    assert!(!msg.verify_id());

    // Fix ID by recomputing it
    msg.id = msg.compute_id();
    assert!(msg.verify_id());

    // Mess up timestamp
    msg.meta.timestamp = Utc.timestamp_opt(0, 0).unwrap();
    let new_id = msg.compute_id();
    assert_ne!(msg.id, new_id); // ID should be different with different timestamp
}

#[test]
fn test_conflict_id() {
    // Test creation
    let conflict1 = ConflictId::new("double-spend-1".to_string());
    let conflict2 = ConflictId::new("double-spend-1".to_string());
    let conflict3 = ConflictId::new("double-spend-2".to_string());

    // Same string should produce same ID
    assert_eq!(conflict1, conflict2);

    // Different string should produce different ID
    assert_ne!(conflict1, conflict3);

    // Test serialization
    let serialized = serde_json::to_string(&conflict1).unwrap();
    let deserialized: ConflictId = serde_json::from_str(&serialized).unwrap();
    assert_eq!(conflict1, deserialized);

    // Test edge cases
    let empty = ConflictId::new(String::new());
    let long = ConflictId::new("a".repeat(10000));
    assert_ne!(empty, long);
}

#[test]
fn test_message_with_complex_features() {
    // Create features with axis phi values
    let features = AdicFeatures::new(vec![
        AxisPhi::new(
            0,
            QpDigits {
                p: 3,
                digits: vec![1, 2, 0, 1, 2, 1, 0, 2, 1, 2],
            },
        ),
        AxisPhi::new(
            1,
            QpDigits {
                p: 3,
                digits: vec![2, 1, 1, 0, 2, 2, 1, 0, 1, 2],
            },
        ),
    ]);

    let meta = AdicMeta::new(Utc.timestamp_opt(999999, 0).unwrap());
    let proposer_pk = PublicKey::from_bytes([0xAAu8; 32]);
    let signature = Signature::new(vec![0xBBu8; 64]);

    let mut msg = AdicMessage::new(
        vec![
            MessageId::new(b"p1"),
            MessageId::new(b"p2"),
            MessageId::new(b"p3"),
        ],
        features.clone(),
        meta,
        proposer_pk,
        b"complex message".to_vec(),
    );
    msg.signature = signature;

    // Serialize with complex features
    let serialized = serde_json::to_string(&msg).unwrap();
    let deserialized: AdicMessage = serde_json::from_str(&serialized).unwrap();

    assert_eq!(msg.features.phi.len(), deserialized.features.phi.len());
    assert_eq!(msg.features.phi[0].axis, deserialized.features.phi[0].axis);
    assert_eq!(
        msg.features.phi[0].qp_digits.digits,
        deserialized.features.phi[0].qp_digits.digits
    );
    assert_eq!(msg.parents.len(), deserialized.parents.len());
}

#[test]
fn test_message_parent_limits() {
    let features = AdicFeatures::new(vec![]);
    let meta = AdicMeta::new(Utc.timestamp_opt(1000, 0).unwrap());
    let proposer_pk = PublicKey::from_bytes([1u8; 32]);

    // Test with no parents (genesis-like)
    let msg_no_parents = AdicMessage::new(
        vec![],
        features.clone(),
        meta.clone(),
        proposer_pk.clone(),
        b"genesis".to_vec(),
    );
    let id_no_parents = msg_no_parents.id;
    assert_ne!(id_no_parents.as_bytes(), &[0u8; 32]);

    // Test with many parents
    let many_parents: Vec<MessageId> = (0..100)
        .map(|i| MessageId::new(format!("parent_{}", i).as_bytes()))
        .collect();
    let msg_many_parents = AdicMessage::new(
        many_parents,
        features.clone(),
        meta.clone(),
        proposer_pk.clone(),
        b"child".to_vec(),
    );
    let id_many_parents = msg_many_parents.id;
    assert_ne!(id_many_parents, id_no_parents);

    // Order of parents should matter
    let parents_ordered = vec![MessageId::new(b"a"), MessageId::new(b"b")];
    let parents_reversed = vec![MessageId::new(b"b"), MessageId::new(b"a")];
    let msg_ordered = AdicMessage::new(
        parents_ordered,
        features.clone(),
        meta.clone(),
        proposer_pk.clone(),
        b"test".to_vec(),
    );
    let id_ordered = msg_ordered.id;
    let msg_reversed = AdicMessage::new(
        parents_reversed,
        features,
        meta,
        proposer_pk,
        b"test".to_vec(),
    );
    let id_reversed = msg_reversed.id;
    assert_ne!(id_ordered, id_reversed);
}

#[test]
fn test_message_clone_and_equality() {
    let meta = AdicMeta::new(Utc.timestamp_opt(5000, 0).unwrap());
    let proposer_pk = PublicKey::from_bytes([5u8; 32]);
    let signature = Signature::new(vec![6u8; 64]);
    let mut msg1 = AdicMessage::new(
        vec![MessageId::new(b"parent")],
        AdicFeatures::new(vec![]),
        meta,
        proposer_pk.clone(),
        b"content".to_vec(),
    );
    msg1.signature = signature;

    // Clone should be equal
    let msg2 = msg1.clone();
    assert_eq!(msg1.id, msg2.id);
    assert_eq!(msg1.payload, msg2.payload);
    assert_eq!(msg1.parents, msg2.parents);
    assert_eq!(msg1.meta.timestamp, msg2.meta.timestamp);

    // Different message should not be equal
    let meta3 = AdicMeta::new(Utc.timestamp_opt(6000, 0).unwrap());
    let proposer_pk3 = PublicKey::from_bytes([7u8; 32]);
    let signature3 = Signature::new(vec![8u8; 64]);
    let mut msg3 = AdicMessage::new(
        vec![],
        AdicFeatures::new(vec![]),
        meta3,
        proposer_pk3,
        b"different".to_vec(),
    );
    msg3.signature = signature3;

    assert_ne!(msg1.id, msg3.id);
    assert_ne!(msg1.payload, msg3.payload);
}
