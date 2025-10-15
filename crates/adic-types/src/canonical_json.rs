//! Canonical JSON Serialization
//!
//! Provides deterministic JSON serialization for ADIC messages to ensure
//! consistent hashing across all nodes in the network.
//!
//! # Canonical Format
//!
//! 1. **Key Ordering**: Object keys sorted lexicographically (UTF-8 byte order)
//! 2. **No Whitespace**: Compact representation, no spaces or newlines
//! 3. **Number Format**: IEEE 754 double precision, no trailing zeros
//! 4. **Unicode**: Minimal escaping (only required characters)
//! 5. **No Null Values**: Omit fields with null values
//!
//! # Example
//!
//! ```text
//! use adic_types::canonical_json::{to_canonical_json, canonical_hash};
//! use serde::Serialize;
//!
//! #[derive(Serialize)]
//! struct Message {
//!     sender: String,
//!     nonce: u64,
//!     data: Vec<u8>,
//! }
//!
//! let msg = Message {
//!     sender: "alice".to_string(),
//!     nonce: 42,
//!     data: vec![1, 2, 3],
//! };
//!
//! // Canonical JSON string
//! let json = to_canonical_json(&msg)?;
//! // Result: {"data":[1,2,3],"nonce":42,"sender":"alice"}
//! //         ^ keys are alphabetically sorted, no whitespace
//!
//! // Deterministic hash
//! let hash = canonical_hash(&msg)?;
//! ```

use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CanonicalJsonError {
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Invalid JSON structure: {0}")]
    InvalidStructure(String),
}

pub type Result<T> = std::result::Result<T, CanonicalJsonError>;

/// Serialize value to canonical JSON string
///
/// Keys are sorted lexicographically, no whitespace, consistent formatting
pub fn to_canonical_json<T: Serialize>(value: &T) -> Result<String> {
    // First serialize to serde_json::Value
    let json_value = serde_json::to_value(value)?;

    // Canonicalize the value
    let canonical = canonicalize_value(json_value);

    // Serialize to string with no whitespace
    let canonical_string = serde_json::to_string(&canonical)?;

    Ok(canonical_string)
}

/// Compute deterministic hash of canonical JSON representation
///
/// Uses Blake3 for fast, cryptographically secure hashing
pub fn canonical_hash<T: Serialize>(value: &T) -> Result<[u8; 32]> {
    let canonical_json = to_canonical_json(value)?;
    let hash = blake3::hash(canonical_json.as_bytes());
    Ok(*hash.as_bytes())
}

/// Canonicalize a JSON value recursively
fn canonicalize_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            // Convert to BTreeMap for sorted keys
            let mut sorted: BTreeMap<String, Value> = BTreeMap::new();
            for (k, v) in map {
                // Skip null values (don't include in canonical form)
                if !v.is_null() {
                    sorted.insert(k, canonicalize_value(v));
                }
            }

            // Convert back to Map (preserves insertion order, which is now sorted)
            let mut canonical_map = Map::new();
            for (k, v) in sorted {
                canonical_map.insert(k, v);
            }

            Value::Object(canonical_map)
        }
        Value::Array(arr) => {
            // Recursively canonicalize array elements
            Value::Array(arr.into_iter().map(canonicalize_value).collect())
        }
        Value::Number(n) => {
            // Ensure consistent number representation
            // Convert to f64 and back to eliminate trailing zeros
            if let Some(f) = n.as_f64() {
                // Special handling for integers
                if f.fract() == 0.0 && f.abs() < (1u64 << 53) as f64 {
                    // Represent as integer if possible
                    Value::Number(serde_json::Number::from(f as i64))
                } else {
                    // Keep as float
                    Value::Number(serde_json::Number::from_f64(f).unwrap_or(n))
                }
            } else {
                Value::Number(n)
            }
        }
        Value::String(s) => Value::String(s),
        Value::Bool(b) => Value::Bool(b),
        Value::Null => Value::Null,
    }
}

/// Verify that two values have the same canonical hash
pub fn verify_canonical_match<T1: Serialize, T2: Serialize>(a: &T1, b: &T2) -> Result<bool> {
    let hash_a = canonical_hash(a)?;
    let hash_b = canonical_hash(b)?;
    Ok(hash_a == hash_b)
}

/// Parse canonical JSON and verify hash
pub fn parse_and_verify_hash(json: &str, expected_hash: &[u8; 32]) -> Result<Value> {
    let value: Value = serde_json::from_str(json)?;
    let computed_hash = canonical_hash(&value)?;

    if computed_hash != *expected_hash {
        return Err(CanonicalJsonError::InvalidStructure(
            "Hash mismatch".to_string()
        ));
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct TestMessage {
        sender: String,
        nonce: u64,
        data: Vec<u8>,
    }

    #[test]
    fn test_key_ordering() {
        let msg = TestMessage {
            sender: "alice".to_string(),
            nonce: 42,
            data: vec![1, 2, 3],
        };

        let json = to_canonical_json(&msg).unwrap();

        // Keys should be alphabetically sorted: data, nonce, sender
        assert!(json.starts_with(r#"{"data":"#));
        assert!(json.contains(r#""nonce":42"#));
        assert!(json.contains(r#""sender":"alice""#));
    }

    #[test]
    fn test_no_whitespace() {
        let msg = TestMessage {
            sender: "bob".to_string(),
            nonce: 100,
            data: vec![],
        };

        let json = to_canonical_json(&msg).unwrap();

        // No spaces, tabs, or newlines
        assert!(!json.contains(' '));
        assert!(!json.contains('\t'));
        assert!(!json.contains('\n'));
    }

    #[test]
    fn test_deterministic_hash() {
        let msg1 = TestMessage {
            sender: "charlie".to_string(),
            nonce: 999,
            data: vec![0xAB, 0xCD],
        };

        let msg2 = TestMessage {
            sender: "charlie".to_string(),
            nonce: 999,
            data: vec![0xAB, 0xCD],
        };

        let hash1 = canonical_hash(&msg1).unwrap();
        let hash2 = canonical_hash(&msg2).unwrap();

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_different_messages_different_hashes() {
        let msg1 = TestMessage {
            sender: "alice".to_string(),
            nonce: 1,
            data: vec![],
        };

        let msg2 = TestMessage {
            sender: "alice".to_string(),
            nonce: 2, // Different nonce
            data: vec![],
        };

        let hash1 = canonical_hash(&msg1).unwrap();
        let hash2 = canonical_hash(&msg2).unwrap();

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_nested_objects() {
        use serde_json::json;

        let value = json!({
            "z_field": "last",
            "a_field": "first",
            "nested": {
                "z_inner": 2,
                "a_inner": 1,
            }
        });

        let canonical = to_canonical_json(&value).unwrap();

        // Outer keys sorted
        assert!(canonical.starts_with(r#"{"a_field":"#));

        // Inner keys also sorted
        assert!(canonical.contains(r#"{"a_inner":1,"z_inner":2}"#));
    }

    #[test]
    fn test_null_values_omitted() {
        use serde_json::json;

        let value = json!({
            "present": "value",
            "missing": null,
        });

        let canonical = to_canonical_json(&value).unwrap();

        // Null field should be omitted
        assert!(!canonical.contains("missing"));
        assert!(canonical.contains("present"));
    }

    #[test]
    fn test_number_formatting() {
        use serde_json::json;

        let value = json!({
            "integer": 42,
            "float": 3.14159,
            "zero": 0,
        });

        let canonical = to_canonical_json(&value).unwrap();

        // Integers should remain integers
        assert!(canonical.contains(r#""integer":42"#));
        assert!(canonical.contains(r#""zero":0"#));

        // Floats should maintain precision
        assert!(canonical.contains(r#""float":3.14159"#));
    }

    #[test]
    fn test_array_order_preserved() {
        use serde_json::json;

        let value = json!({
            "items": [3, 1, 4, 1, 5, 9],
        });

        let canonical = to_canonical_json(&value).unwrap();

        // Array order should NOT be sorted (preserve original order)
        assert!(canonical.contains(r#"[3,1,4,1,5,9]"#));
    }

    #[test]
    fn test_unicode_handling() {
        use serde_json::json;

        let value = json!({
            "emoji": "🚀",
            "chinese": "你好",
            "unicode": "\u{0041}", // 'A'
        });

        let canonical = to_canonical_json(&value).unwrap();

        // Should handle unicode correctly
        assert!(canonical.contains("🚀"));
        assert!(canonical.contains("你好"));
    }

    #[test]
    fn test_verify_canonical_match() {
        let msg1 = TestMessage {
            sender: "dave".to_string(),
            nonce: 777,
            data: vec![0xFF],
        };

        let msg2 = TestMessage {
            sender: "dave".to_string(),
            nonce: 777,
            data: vec![0xFF],
        };

        assert!(verify_canonical_match(&msg1, &msg2).unwrap());
    }

    #[test]
    fn test_parse_and_verify() {
        let msg = TestMessage {
            sender: "eve".to_string(),
            nonce: 123,
            data: vec![1, 2, 3],
        };

        let json = to_canonical_json(&msg).unwrap();
        let hash = canonical_hash(&msg).unwrap();

        // Should successfully parse and verify
        let parsed = parse_and_verify_hash(&json, &hash);
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_parse_and_verify_fail() {
        let msg = TestMessage {
            sender: "frank".to_string(),
            nonce: 456,
            data: vec![],
        };

        let json = to_canonical_json(&msg).unwrap();
        let wrong_hash = [0u8; 32]; // Wrong hash

        // Should fail verification
        let result = parse_and_verify_hash(&json, &wrong_hash);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_object() {
        use serde_json::json;

        let value = json!({});
        let canonical = to_canonical_json(&value).unwrap();

        assert_eq!(canonical, "{}");
    }

    #[test]
    fn test_empty_array() {
        use serde_json::json;

        let value = json!([]);
        let canonical = to_canonical_json(&value).unwrap();

        assert_eq!(canonical, "[]");
    }

    #[test]
    fn test_deeply_nested() {
        use serde_json::json;

        let value = json!({
            "level1": {
                "level2": {
                    "level3": {
                        "z": "last",
                        "a": "first",
                    }
                }
            }
        });

        let canonical = to_canonical_json(&value).unwrap();

        // Even deeply nested keys should be sorted
        assert!(canonical.contains(r#"{"a":"first","z":"last"}"#));
    }
}
