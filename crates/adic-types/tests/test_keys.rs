use adic_types::{PublicKey, Signature};

#[test]
fn test_public_key_creation() {
    // Valid 32-byte key
    let key_bytes = [1u8; 32];
    let pubkey = PublicKey::from_bytes(key_bytes);
    assert_eq!(pubkey.as_bytes(), &key_bytes);

    // Zero key
    let zero_key = PublicKey::from_bytes([0u8; 32]);
    assert_eq!(zero_key.as_bytes(), &[0u8; 32]);

    // Max value key
    let max_key = PublicKey::from_bytes([0xFFu8; 32]);
    assert_eq!(max_key.as_bytes(), &[0xFFu8; 32]);
}

#[test]
fn test_public_key_serialization() {
    let key_bytes = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32,
        0x10, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
        0xFF, 0x00,
    ];
    let pubkey = PublicKey::from_bytes(key_bytes);

    // Serialize
    let serialized = serde_json::to_string(&pubkey).unwrap();
    assert!(!serialized.is_empty());

    // Deserialize
    let deserialized: PublicKey = serde_json::from_str(&serialized).unwrap();
    assert_eq!(pubkey.as_bytes(), deserialized.as_bytes());

    // Binary serialization
    let bin_serialized = bincode::serialize(&pubkey).unwrap();
    let bin_deserialized: PublicKey = bincode::deserialize(&bin_serialized).unwrap();
    assert_eq!(pubkey.as_bytes(), bin_deserialized.as_bytes());
}

#[test]
fn test_public_key_equality() {
    let key1 = PublicKey::from_bytes([1u8; 32]);
    let key2 = PublicKey::from_bytes([1u8; 32]);
    let key3 = PublicKey::from_bytes([2u8; 32]);

    // Same bytes should be equal
    assert_eq!(key1, key2);

    // Different bytes should not be equal
    assert_ne!(key1, key3);

    // Clone should be equal
    let key1_clone = key1;
    assert_eq!(key1, key1_clone);
}

#[test]
fn test_signature_creation() {
    // Valid 64-byte signature
    let sig_bytes = vec![2u8; 64];
    let signature = Signature::new(sig_bytes.clone());
    assert_eq!(signature.as_bytes(), &sig_bytes[..]);

    // Zero signature
    let zero_sig = Signature::new(vec![0u8; 64]);
    assert_eq!(zero_sig.as_bytes(), &[0u8; 64]);

    // Max value signature
    let max_sig = Signature::new(vec![0xFFu8; 64]);
    assert_eq!(max_sig.as_bytes(), &[0xFFu8; 64]);

    // Empty signature
    let empty = Signature::empty();
    assert!(empty.is_empty());
    assert_eq!(empty.as_bytes().len(), 0);
}

#[test]
fn test_signature_various_sizes() {
    // Signatures can be various sizes in this implementation
    let small = Signature::new(vec![1u8; 32]);
    assert_eq!(small.as_bytes().len(), 32);

    let medium = Signature::new(vec![2u8; 64]);
    assert_eq!(medium.as_bytes().len(), 64);

    let large = Signature::new(vec![3u8; 128]);
    assert_eq!(large.as_bytes().len(), 128);

    let empty = Signature::empty();
    assert_eq!(empty.as_bytes().len(), 0);
}

#[test]
fn test_signature_serialization() {
    let mut sig_bytes = vec![0u8; 64];
    for (i, byte) in sig_bytes.iter_mut().enumerate().take(64) {
        *byte = i as u8;
    }
    let signature = Signature::new(sig_bytes.clone());

    // JSON serialization
    let serialized = serde_json::to_string(&signature).unwrap();
    let deserialized: Signature = serde_json::from_str(&serialized).unwrap();
    assert_eq!(signature.as_bytes(), deserialized.as_bytes());

    // Binary serialization
    let bin_serialized = bincode::serialize(&signature).unwrap();
    let bin_deserialized: Signature = bincode::deserialize(&bin_serialized).unwrap();
    assert_eq!(signature.as_bytes(), bin_deserialized.as_bytes());
}

#[test]
fn test_signature_equality() {
    let sig1 = Signature::new(vec![1u8; 64]);
    let sig2 = Signature::new(vec![1u8; 64]);
    let sig3 = Signature::new(vec![2u8; 64]);

    // Same bytes should be equal
    assert_eq!(sig1, sig2);

    // Different bytes should not be equal
    assert_ne!(sig1, sig3);

    // Clone should be equal
    let sig1_clone = sig1.clone();
    assert_eq!(sig1, sig1_clone);
}

#[test]
fn test_key_signature_interaction() {
    // Create a key and signature pair
    let key = PublicKey::from_bytes([0xAAu8; 32]);
    let sig = Signature::new(vec![0xBBu8; 64]);

    // They should serialize independently
    let key_json = serde_json::to_string(&key).unwrap();
    let sig_json = serde_json::to_string(&sig).unwrap();

    assert!(!key_json.is_empty());
    assert!(!sig_json.is_empty());
    assert_ne!(key_json, sig_json);

    // Create a structure containing both
    #[derive(serde::Serialize, serde::Deserialize)]
    struct KeySigPair {
        public_key: PublicKey,
        signature: Signature,
    }

    let pair = KeySigPair {
        public_key: key,
        signature: sig.clone(),
    };

    let pair_json = serde_json::to_string(&pair).unwrap();
    let pair_deserialized: KeySigPair = serde_json::from_str(&pair_json).unwrap();

    assert_eq!(key.as_bytes(), pair_deserialized.public_key.as_bytes());
    assert_eq!(sig.as_bytes(), pair_deserialized.signature.as_bytes());
}

#[test]
fn test_key_hex_representation() {
    let key_bytes: [u8; 32] = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32,
        0x10, 0x00, 0xFF, 0xAA, 0x55, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0, 0x11, 0x22,
        0x33, 0x44,
    ];

    let key = PublicKey::from_bytes(key_bytes);

    // Convert to hex
    let hex_str = key.to_hex();
    assert_eq!(hex_str.len(), 64); // 32 bytes = 64 hex characters

    // Parse from hex
    let key_from_hex = PublicKey::from_hex(&hex_str).unwrap();
    assert_eq!(key.as_bytes(), key_from_hex.as_bytes());
}

#[test]
fn test_key_hex_errors() {
    // Too short
    let short = "01234567";
    let result = PublicKey::from_hex(short);
    assert!(result.is_err());

    // Too long
    let long = "0".repeat(66);
    let result = PublicKey::from_hex(&long);
    assert!(result.is_err());

    // Invalid hex
    let invalid = "gg".repeat(32);
    let result = PublicKey::from_hex(&invalid);
    assert!(result.is_err());
}

#[test]
fn test_signature_hex_representation() {
    let mut sig_bytes = vec![0u8; 64];
    for (i, byte) in sig_bytes.iter_mut().enumerate().take(64) {
        *byte = ((i * 3) % 256) as u8;
    }

    let sig = Signature::new(sig_bytes.clone());

    // Convert to hex
    let hex_str = sig.to_hex();
    assert_eq!(hex_str.len(), 128); // 64 bytes = 128 hex characters

    // Parse from hex
    let sig_from_hex = Signature::from_hex(&hex_str).unwrap();
    assert_eq!(sig.as_bytes(), sig_from_hex.as_bytes());
}

#[test]
fn test_signature_hex_various_sizes() {
    // Test different signature sizes
    let sig32 = Signature::new(vec![0xABu8; 32]);
    let hex32 = sig32.to_hex();
    assert_eq!(hex32.len(), 64);

    let sig_from_hex32 = Signature::from_hex(&hex32).unwrap();
    assert_eq!(sig32.as_bytes(), sig_from_hex32.as_bytes());

    // Empty signature
    let empty = Signature::empty();
    let hex_empty = empty.to_hex();
    assert_eq!(hex_empty.len(), 0);

    let empty_from_hex = Signature::from_hex(&hex_empty).unwrap();
    assert!(empty_from_hex.is_empty());
}

// Add bincode dependency for binary serialization tests
