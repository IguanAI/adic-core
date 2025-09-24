use adic_types::{AdicFeatures, AdicMessage, AdicMeta, MessageId, PublicKey, Signature};
use bincode;
use chrono::{TimeZone, Utc};
use hex;

#[test]
fn test_message_id_creation() {
    // Test basic creation
    let id1 = MessageId::new(b"test");
    let id2 = MessageId::new(b"test");

    // Same input should produce same ID
    assert_eq!(id1, id2);

    // Different input should produce different ID
    let id3 = MessageId::new(b"different");
    assert_ne!(id1, id3);

    // Empty input
    let empty_id = MessageId::new(b"");
    assert_ne!(empty_id.as_bytes(), &[0u8; 32]);

    // Large input
    let large_input = vec![0xABu8; 10000];
    let large_id = MessageId::new(&large_input);
    assert_ne!(large_id.as_bytes(), &[0u8; 32]);
}

#[test]
fn test_message_id_compute() {
    let payload = b"message content";
    let parents = vec![MessageId::new(b"parent1"), MessageId::new(b"parent2")];
    let features = AdicFeatures::new(vec![]);
    let meta = AdicMeta::new(Utc.timestamp_opt(1234567890, 0).unwrap());
    let proposer_pk = PublicKey::from_bytes([1u8; 32]);

    // Compute ID
    let msg1 = AdicMessage::new(
        parents.clone(),
        features.clone(),
        meta.clone(),
        proposer_pk.clone(),
        payload.to_vec(),
    );
    let id1 = msg1.id;

    // Same inputs should produce same ID
    let msg2 = AdicMessage::new(
        parents.clone(),
        features.clone(),
        meta.clone(),
        proposer_pk.clone(),
        payload.to_vec(),
    );
    let id2 = msg2.id;
    assert_eq!(id1, id2);

    // Change payload
    let msg3 = AdicMessage::new(
        parents.clone(),
        features.clone(),
        meta.clone(),
        proposer_pk.clone(),
        b"different content".to_vec(),
    );
    let id3 = msg3.id;
    assert_ne!(id1, id3);

    // Change parents
    let parents2 = vec![MessageId::new(b"other_parent")];
    let msg4 = AdicMessage::new(
        parents2,
        features.clone(),
        meta.clone(),
        proposer_pk.clone(),
        payload.to_vec(),
    );
    let id4 = msg4.id;
    assert_ne!(id1, id4);

    // Change timestamp
    let meta2 = AdicMeta::new(Utc.timestamp_opt(9999999999, 0).unwrap());
    let msg5 = AdicMessage::new(
        parents.clone(),
        features.clone(),
        meta2,
        proposer_pk.clone(),
        payload.to_vec(),
    );
    let id5 = msg5.id;
    assert_ne!(id1, id5);
}

#[test]
fn test_message_id_serialization() {
    let id = MessageId::new(b"test_message");

    // JSON serialization
    let json = serde_json::to_string(&id).unwrap();
    let deserialized: MessageId = serde_json::from_str(&json).unwrap();
    assert_eq!(id, deserialized);

    // Binary serialization
    let binary = bincode::serialize(&id).unwrap();
    let deserialized_bin: MessageId = bincode::deserialize(&binary).unwrap();
    assert_eq!(id, deserialized_bin);

    // Hex representation
    let hex_str = hex::encode(id.as_bytes());
    assert_eq!(hex_str.len(), 64); // 32 bytes = 64 hex chars

    // Parse from hex
    let bytes = hex::decode(&hex_str).unwrap();
    assert_eq!(bytes.len(), 32);
}

#[test]
fn test_message_id_from_bytes() {
    // Create ID from specific bytes
    let bytes = [0xAAu8; 32];
    let id = MessageId::from_bytes(bytes);
    assert_eq!(id.as_bytes(), &bytes);

    // Zero bytes
    let zero_bytes = [0u8; 32];
    let zero_id = MessageId::from_bytes(zero_bytes);
    assert_eq!(zero_id.as_bytes(), &zero_bytes);

    // Max value bytes
    let max_bytes = [0xFFu8; 32];
    let max_id = MessageId::from_bytes(max_bytes);
    assert_eq!(max_id.as_bytes(), &max_bytes);

    // Pattern bytes
    let mut pattern_bytes = [0u8; 32];
    for i in 0..32 {
        pattern_bytes[i] = i as u8;
    }
    let pattern_id = MessageId::from_bytes(pattern_bytes);
    assert_eq!(pattern_id.as_bytes(), &pattern_bytes);
}

#[test]
fn test_message_id_equality() {
    let id1 = MessageId::new(b"test");
    let id2 = MessageId::new(b"test");
    let id3 = MessageId::new(b"other");

    // Same content should be equal
    assert_eq!(id1, id2);

    // Different content should not be equal
    assert_ne!(id1, id3);

    // Clone should be equal
    let id1_clone = id1.clone();
    assert_eq!(id1, id1_clone);

    // From same bytes should be equal
    let bytes = [0x42u8; 32];
    let from_bytes1 = MessageId::from_bytes(bytes);
    let from_bytes2 = MessageId::from_bytes(bytes);
    assert_eq!(from_bytes1, from_bytes2);
}

#[test]
fn test_message_id_ordering() {
    // MessageIds should have consistent ordering for use in collections
    let id1 = MessageId::from_bytes([0x00u8; 32]);
    let id2 = MessageId::from_bytes([0x01u8; 32]);
    let id3 = MessageId::from_bytes([0xFFu8; 32]);

    assert!(id1 < id2);
    assert!(id2 < id3);
    assert!(id1 < id3);

    // Test with more complex patterns
    let mut bytes1 = [0u8; 32];
    let mut bytes2 = [0u8; 32];
    bytes1[15] = 0x01;
    bytes2[15] = 0x02;

    let id_a = MessageId::from_bytes(bytes1);
    let id_b = MessageId::from_bytes(bytes2);
    assert!(id_a < id_b);
}

#[test]
fn test_message_id_hash() {
    use std::collections::HashSet;

    // MessageIds should be hashable for use in HashMaps/HashSets
    let mut set = HashSet::new();

    let id1 = MessageId::new(b"first");
    let id2 = MessageId::new(b"second");
    let id3 = MessageId::new(b"first"); // Duplicate of id1

    set.insert(id1.clone());
    set.insert(id2.clone());
    set.insert(id3.clone()); // Should not increase size

    assert_eq!(set.len(), 2);
    assert!(set.contains(&id1));
    assert!(set.contains(&id2));
}

#[test]
fn test_message_id_display() {
    let id = MessageId::new(b"display_test");

    // Should have a display representation
    let display = format!("{:?}", id);
    assert!(!display.is_empty());

    // Should be able to convert to string
    let hex_str = hex::encode(id.as_bytes());
    assert_eq!(hex_str.len(), 64);
}

#[test]
fn test_message_id_determinism() {
    // Test that ID generation is deterministic
    let ids: Vec<MessageId> = (0..10).map(|_| MessageId::new(b"deterministic")).collect();

    // All should be equal
    for i in 1..ids.len() {
        assert_eq!(ids[0], ids[i]);
    }

    // Test compute determinism
    let payload = b"content";
    let parents = vec![MessageId::new(b"parent")];
    let features = AdicFeatures::new(vec![]);
    let meta = AdicMeta::new(Utc.timestamp_opt(1000, 0).unwrap());
    let proposer_pk = PublicKey::from_bytes([5u8; 32]);

    let computed_ids: Vec<MessageId> = (0..10)
        .map(|_| {
            let msg = AdicMessage::new(
                parents.clone(),
                features.clone(),
                meta.clone(),
                proposer_pk.clone(),
                payload.to_vec(),
            );
            msg.id
        })
        .collect();

    // All should be equal
    for i in 1..computed_ids.len() {
        assert_eq!(computed_ids[0], computed_ids[i]);
    }
}

#[test]
fn test_message_id_collision_resistance() {
    // Test that similar inputs produce different IDs
    let id1 = MessageId::new(b"test1");
    let id2 = MessageId::new(b"test2");
    let id3 = MessageId::new(b"test11");
    let id4 = MessageId::new(b"1test");

    // All should be different
    assert_ne!(id1, id2);
    assert_ne!(id1, id3);
    assert_ne!(id1, id4);
    assert_ne!(id2, id3);
    assert_ne!(id2, id4);
    assert_ne!(id3, id4);

    // Even single bit differences should produce different IDs
    let bytes1 = b"00000000000000000000000000000000";
    let bytes2 = b"00000000000000000000000000000001";
    let id_a = MessageId::new(bytes1);
    let id_b = MessageId::new(bytes2);
    assert_ne!(id_a, id_b);
}
