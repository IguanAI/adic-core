#![no_main]

use libfuzzer_sys::fuzz_target;
use adic_types::canonical_hash;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

// Two structures with different field declaration order but same data
#[derive(Serialize, Clone, Debug)]
struct OrderA {
    z_field: String,
    y_field: u64,
    x_field: bool,
    w_field: Vec<u8>,
    v_field: HashMap<String, String>,
}

#[derive(Serialize, Clone, Debug)]
struct OrderB {
    v_field: HashMap<String, String>,
    w_field: Vec<u8>,
    x_field: bool,
    y_field: u64,
    z_field: String,
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 20 {
        return;
    }

    // Create identical data in different field orders
    let z_val = String::from_utf8_lossy(&data[0..std::cmp::min(10, data.len())]).to_string();
    let y_val = u64::from_le_bytes([
        data[10], data[11], data[12], data[13],
        data[14], data[15], data[16], data[17],
    ]);
    let x_val = data[18] % 2 == 0;
    let w_val = data[10..std::cmp::min(20, data.len())].to_vec();

    let mut v_val = HashMap::new();
    if data.len() > 30 {
        let num_keys = std::cmp::min((data[19] % 10) as usize, 5);
        for i in 0..num_keys {
            // Create keys in pseudo-random order from fuzz data
            let key_order = data.get(20 + i).copied().unwrap_or(0);
            let key = format!("key_{:02x}", key_order);
            let val = format!("val_{}", i);
            v_val.insert(key, val);
        }
    }

    let msg_a = OrderA {
        z_field: z_val.clone(),
        y_field: y_val,
        x_field: x_val,
        w_field: w_val.clone(),
        v_field: v_val.clone(),
    };

    let msg_b = OrderB {
        v_field: v_val,
        w_field: w_val,
        x_field: x_val,
        y_field: y_val,
        z_field: z_val,
    };

    // Test 1: Different struct field orders should produce SAME canonical hash
    if let (Ok(hash_a), Ok(hash_b)) = (canonical_hash(&msg_a), canonical_hash(&msg_b)) {
        assert_eq!(
            hash_a, hash_b,
            "Canonical hash must ignore struct field declaration order"
        );
    }

    // Test 2: Build HashMap with keys in different insertion orders
    let mut map1 = HashMap::new();
    let mut map2 = HashMap::new();

    if data.len() > 40 {
        let num_keys = std::cmp::min((data[30] % 8) as usize, 5);

        // Insert in forward order for map1
        for i in 0..num_keys {
            let key = format!("k{}", i);
            let val = data.get(31 + i).copied().unwrap_or(0).to_string();
            map1.insert(key.clone(), val.clone());
        }

        // Insert in reverse order for map2
        for i in (0..num_keys).rev() {
            let key = format!("k{}", i);
            let val = data.get(31 + i).copied().unwrap_or(0).to_string();
            map2.insert(key.clone(), val.clone());
        }

        // Both maps should hash the same despite different insertion order
        if let (Ok(hash1), Ok(hash2)) = (canonical_hash(&map1), canonical_hash(&map2)) {
            assert_eq!(
                hash1, hash2,
                "Canonical hash must ignore HashMap insertion order"
            );
        }
    }

    // Test 3: Nested structures with different key orders
    #[derive(Serialize)]
    struct Nested {
        inner: HashMap<String, Value>,
    }

    let mut inner1 = HashMap::new();
    let mut inner2 = HashMap::new();

    if data.len() > 50 {
        let keys = vec!["alpha", "beta", "gamma", "delta"];

        // Forward order
        for (i, key) in keys.iter().enumerate() {
            let val = data.get(40 + i).copied().unwrap_or(0);
            inner1.insert(key.to_string(), Value::Number(val.into()));
        }

        // Reverse order
        for (i, key) in keys.iter().rev().enumerate() {
            let val = data.get(40 + (keys.len() - 1 - i)).copied().unwrap_or(0);
            inner2.insert(key.to_string(), Value::Number(val.into()));
        }

        let nested1 = Nested { inner: inner1 };
        let nested2 = Nested { inner: inner2 };

        if let (Ok(h1), Ok(h2)) = (canonical_hash(&nested1), canonical_hash(&nested2)) {
            assert_eq!(
                h1, h2,
                "Canonical hash must handle nested structures with different key orders"
            );
        }
    }
});
