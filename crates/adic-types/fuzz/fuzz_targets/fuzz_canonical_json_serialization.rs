#![no_main]

use libfuzzer_sys::fuzz_target;
use adic_types::to_canonical_json;
use serde::Serialize;
use std::collections::HashMap;

// Arbitrary serializable structure for fuzzing
#[derive(Serialize, Clone, Debug)]
struct FuzzData {
    #[serde(skip_serializing_if = "Option::is_none")]
    str_field: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    int_field: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    float_field: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    bool_field: Option<bool>,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    array_field: Vec<u8>,

    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    map_field: HashMap<String, String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    nested: Option<Box<FuzzData>>,
}

fuzz_target!(|data: &[u8]| {
    // Skip if data is too small
    if data.len() < 10 {
        return;
    }

    // Parse fuzzer input to construct FuzzData
    let str_val = if data[0] % 2 == 0 {
        Some(String::from_utf8_lossy(&data[1..std::cmp::min(20, data.len())]).to_string())
    } else {
        None
    };

    let int_val = if data[0] % 3 == 0 {
        Some(i64::from_le_bytes([
            data.get(1).copied().unwrap_or(0),
            data.get(2).copied().unwrap_or(0),
            data.get(3).copied().unwrap_or(0),
            data.get(4).copied().unwrap_or(0),
            data.get(5).copied().unwrap_or(0),
            data.get(6).copied().unwrap_or(0),
            data.get(7).copied().unwrap_or(0),
            data.get(8).copied().unwrap_or(0),
        ]))
    } else {
        None
    };

    let float_val = if data[0] % 5 == 0 && data.len() > 16 {
        let f = f64::from_le_bytes([
            data[9], data[10], data[11], data[12],
            data[13], data[14], data[15], data[16],
        ]);
        // Only use if finite
        if f.is_finite() {
            Some(f)
        } else {
            None
        }
    } else {
        None
    };

    let bool_val = if data[0] % 7 == 0 {
        Some(data[1] % 2 == 0)
    } else {
        None
    };

    let array_field = if data.len() > 20 {
        data[10..std::cmp::min(30, data.len())].to_vec()
    } else {
        Vec::new()
    };

    let mut map_field = HashMap::new();
    if data.len() > 30 {
        let num_entries = (data[20] % 5) as usize;
        for i in 0..num_entries {
            let key = format!("key_{}", i);
            let val_start = 21 + i * 10;
            if val_start < data.len() {
                let val_end = std::cmp::min(val_start + 10, data.len());
                let value = String::from_utf8_lossy(&data[val_start..val_end]).to_string();
                map_field.insert(key, value);
            }
        }
    }

    let nested = if data[0] % 11 == 0 && data.len() > 50 {
        Some(Box::new(FuzzData {
            str_field: Some("nested".to_string()),
            int_field: Some(42),
            float_field: Some(3.14),
            bool_field: Some(true),
            array_field: vec![1, 2, 3],
            map_field: HashMap::new(),
            nested: None,
        }))
    } else {
        None
    };

    let fuzz_data = FuzzData {
        str_field: str_val,
        int_field: int_val,
        float_field: float_val,
        bool_field: bool_val,
        array_field,
        map_field,
        nested,
    };

    // Test 1: Serialization should not panic
    let _ = to_canonical_json(&fuzz_data);

    // Test 2: Double serialization should produce identical results
    if let Ok(json1) = to_canonical_json(&fuzz_data) {
        if let Ok(json2) = to_canonical_json(&fuzz_data) {
            assert_eq!(json1, json2, "Canonical JSON must be deterministic");
        }
    }

    // Test 3: Serialized JSON should be valid UTF-8 (String is already UTF-8)
    if let Ok(json) = to_canonical_json(&fuzz_data) {
        assert!(!json.is_empty() || fuzz_data.str_field.is_none(), "Canonical JSON should produce output");
    }
});
