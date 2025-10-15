#![no_main]

use libfuzzer_sys::fuzz_target;
use adic_types::{canonical_hash, verify_canonical_match};
use serde::Serialize;
use std::collections::HashMap;

// Message structure for hash fuzzing
#[derive(Serialize, Clone, Debug)]
struct HashMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Vec<u8>>,

    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    metadata: HashMap<String, String>,
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 8 {
        return;
    }

    // Create message from fuzzer input
    let id_val = if data[0] % 2 == 0 {
        Some(u64::from_le_bytes([
            data[0], data[1], data[2], data[3],
            data[4], data[5], data[6], data[7],
        ]))
    } else {
        None
    };

    let data_val = if data.len() > 10 {
        Some(data[8..std::cmp::min(100, data.len())].to_vec())
    } else {
        None
    };

    let mut metadata = HashMap::new();
    if data.len() > 20 {
        // Add some metadata entries
        let num_entries = std::cmp::min((data[8] % 10) as usize, 5);
        for i in 0..num_entries {
            metadata.insert(
                format!("key_{}", i),
                format!("value_{}", data.get(9 + i).copied().unwrap_or(0)),
            );
        }
    }

    let msg1 = HashMessage {
        id: id_val,
        data: data_val.clone(),
        metadata: metadata.clone(),
    };

    // Clone for comparison
    let msg2 = HashMessage {
        id: id_val,
        data: data_val,
        metadata,
    };

    // Test 1: Hash computation should not panic
    let _ = canonical_hash(&msg1);

    // Test 2: Same data should produce same hash
    if let Ok(hash1) = canonical_hash(&msg1) {
        if let Ok(hash2) = canonical_hash(&msg2) {
            assert_eq!(
                hash1, hash2,
                "Canonical hash must be deterministic for identical data"
            );
        }
    }

    // Test 3: verify_canonical_match should work correctly
    if canonical_hash(&msg1).is_ok() && canonical_hash(&msg2).is_ok() {
        let result = verify_canonical_match(&msg1, &msg2);
        assert!(
            result.is_ok(),
            "verify_canonical_match should succeed for identical data"
        );
        assert!(
            result.unwrap(),
            "verify_canonical_match should return true for identical data"
        );
    }

    // Test 4: Hash should be 32 bytes (it already is [u8; 32])
    if let Ok(hash) = canonical_hash(&msg1) {
        assert_eq!(hash.len(), 32, "Hash must be 32 bytes");
    }
});
