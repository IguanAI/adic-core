//! Performance benchmarks for canonical JSON serialization and hashing
//!
//! These benchmarks measure the performance of Phase 2 canonical JSON implementation:
//! - Serialization to canonical JSON
//! - Blake3 hashing of canonical JSON
//! - Verification of canonical matches
//!
//! Run with: cargo bench --package adic-types --bench canonical_json_bench

use adic_types::{canonical_hash, to_canonical_json, verify_canonical_match, AdicFeatures, AdicMessage, AdicMeta, MessageId, PublicKey};
use chrono::Utc;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde::Serialize;
use std::collections::HashMap;

// ========== Test Data Generators ==========

/// Create a simple message structure
fn create_simple_message() -> SimpleMessage {
    SimpleMessage {
        id: 42,
        name: "test_message".to_string(),
        value: 100.0,
        flag: true,
    }
}

/// Create a nested message structure
fn create_nested_message() -> NestedMessage {
    NestedMessage {
        metadata: MessageMetadata {
            version: 1,
            author: "test".to_string(),
            timestamp: 1234567890,
        },
        data: vec![1, 2, 3, 4, 5],
        tags: {
            let mut map = HashMap::new();
            map.insert("key1".to_string(), "value1".to_string());
            map.insert("key2".to_string(), "value2".to_string());
            map.insert("key3".to_string(), "value3".to_string());
            map
        },
    }
}

/// Create a complex deeply nested structure
fn create_complex_message() -> ComplexMessage {
    ComplexMessage {
        header: MessageHeader {
            id: [42u8; 32],
            parent_id: [1u8; 32],
            timestamp: 1234567890,
        },
        payload: MessagePayload {
            data: vec![0u8; 1000], // 1KB of data
            checksum: [99u8; 32],
        },
        metadata: {
            let mut map = HashMap::new();
            for i in 0..10 {
                map.insert(format!("key_{}", i), format!("value_{}", i));
            }
            map
        },
        tags: vec!["tag1".to_string(), "tag2".to_string(), "tag3".to_string()],
    }
}

/// Create an ADIC message for realistic benchmarking
fn create_adic_message() -> AdicMessage {
    let parents = vec![
        MessageId::from_bytes([1u8; 32]),
        MessageId::from_bytes([2u8; 32]),
        MessageId::from_bytes([3u8; 32]),
        MessageId::from_bytes([4u8; 32]),
    ];

    let features = AdicFeatures::new(vec![]);
    let meta = AdicMeta::new(Utc::now());
    let proposer_pk = PublicKey::from_bytes([42u8; 32]);
    let data = vec![0u8; 100]; // 100 bytes of data

    AdicMessage::new(parents, features, meta, proposer_pk, data)
}

// ========== Test Message Types ==========

#[derive(Serialize)]
struct SimpleMessage {
    id: u64,
    name: String,
    value: f64,
    flag: bool,
}

#[derive(Serialize)]
struct MessageMetadata {
    version: u32,
    author: String,
    timestamp: u64,
}

#[derive(Serialize)]
struct NestedMessage {
    metadata: MessageMetadata,
    data: Vec<u8>,
    tags: HashMap<String, String>,
}

#[derive(Serialize)]
struct MessageHeader {
    id: [u8; 32],
    parent_id: [u8; 32],
    timestamp: u64,
}

#[derive(Serialize)]
struct MessagePayload {
    data: Vec<u8>,
    checksum: [u8; 32],
}

#[derive(Serialize)]
struct ComplexMessage {
    header: MessageHeader,
    payload: MessagePayload,
    metadata: HashMap<String, String>,
    tags: Vec<String>,
}

// ========== Benchmarks ==========

/// Benchmark canonical JSON serialization for simple messages
fn bench_to_canonical_json_simple(c: &mut Criterion) {
    let msg = create_simple_message();

    c.bench_function("canonical_json_simple", |b| {
        b.iter(|| to_canonical_json(black_box(&msg)).unwrap())
    });
}

/// Benchmark canonical JSON serialization for nested messages
fn bench_to_canonical_json_nested(c: &mut Criterion) {
    let msg = create_nested_message();

    c.bench_function("canonical_json_nested", |b| {
        b.iter(|| to_canonical_json(black_box(&msg)).unwrap())
    });
}

/// Benchmark canonical JSON serialization for complex messages
fn bench_to_canonical_json_complex(c: &mut Criterion) {
    let msg = create_complex_message();

    c.bench_function("canonical_json_complex", |b| {
        b.iter(|| to_canonical_json(black_box(&msg)).unwrap())
    });
}

/// Benchmark canonical hashing for simple messages
fn bench_canonical_hash_simple(c: &mut Criterion) {
    let msg = create_simple_message();

    c.bench_function("canonical_hash_simple", |b| {
        b.iter(|| canonical_hash(black_box(&msg)).unwrap())
    });
}

/// Benchmark canonical hashing for nested messages
fn bench_canonical_hash_nested(c: &mut Criterion) {
    let msg = create_nested_message();

    c.bench_function("canonical_hash_nested", |b| {
        b.iter(|| canonical_hash(black_box(&msg)).unwrap())
    });
}

/// Benchmark canonical hashing for complex messages
fn bench_canonical_hash_complex(c: &mut Criterion) {
    let msg = create_complex_message();

    c.bench_function("canonical_hash_complex", |b| {
        b.iter(|| canonical_hash(black_box(&msg)).unwrap())
    });
}

/// Benchmark canonical hash verification
fn bench_verify_canonical_match(c: &mut Criterion) {
    let msg1 = create_simple_message();
    let msg2 = create_simple_message();

    c.bench_function("verify_canonical_match", |b| {
        b.iter(|| verify_canonical_match(black_box(&msg1), black_box(&msg2)).unwrap())
    });
}

/// Benchmark ADIC message ID computation (uses canonical hash)
fn bench_adic_message_compute_id(c: &mut Criterion) {
    let msg = create_adic_message();

    c.bench_function("adic_message_compute_id", |b| {
        b.iter(|| black_box(&msg).compute_id())
    });
}

/// Benchmark scaling with message size
fn bench_canonical_hash_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("canonical_hash_scaling");

    for size in [100, 500, 1000, 5000, 10000].iter() {
        let data = vec![0u8; *size];

        #[derive(Serialize)]
        struct Message {
            data: Vec<u8>,
        }

        let msg = Message { data };

        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| canonical_hash(black_box(&msg)).unwrap());
        });
    }
    group.finish();
}

/// Benchmark throughput: operations per second
fn bench_canonical_hash_throughput(c: &mut Criterion) {
    let msg = create_simple_message();

    let mut group = c.benchmark_group("canonical_hash_throughput");
    group.sample_size(1000);

    group.bench_function("throughput", |b| {
        b.iter(|| {
            // Simulate batch processing
            for _ in 0..100 {
                let _ = canonical_hash(black_box(&msg)).unwrap();
            }
        });
    });

    group.finish();
}

/// Benchmark key ordering impact
fn bench_key_ordering(c: &mut Criterion) {
    #[derive(Serialize)]
    struct OrderTest1 {
        a: u32,
        b: u32,
        c: u32,
        d: u32,
        e: u32,
    }

    #[derive(Serialize)]
    struct OrderTest2 {
        e: u32,
        d: u32,
        c: u32,
        b: u32,
        a: u32,
    }

    let msg1 = OrderTest1 {
        a: 1,
        b: 2,
        c: 3,
        d: 4,
        e: 5,
    };

    let msg2 = OrderTest2 {
        e: 5,
        d: 4,
        c: 3,
        b: 2,
        a: 1,
    };

    c.bench_function("canonical_hash_order1", |b| {
        b.iter(|| canonical_hash(black_box(&msg1)).unwrap())
    });

    c.bench_function("canonical_hash_order2", |b| {
        b.iter(|| canonical_hash(black_box(&msg2)).unwrap())
    });
}

/// Benchmark null value omission
fn bench_null_omission(c: &mut Criterion) {
    #[derive(Serialize)]
    struct WithNulls {
        a: Option<u32>,
        b: Option<u32>,
        c: Option<u32>,
        d: u32,
        e: u32,
    }

    let msg = WithNulls {
        a: None,
        b: Some(2),
        c: None,
        d: 4,
        e: 5,
    };

    c.bench_function("canonical_json_with_nulls", |b| {
        b.iter(|| to_canonical_json(black_box(&msg)).unwrap())
    });
}

// ========== Benchmark Groups ==========

criterion_group!(
    benches,
    bench_to_canonical_json_simple,
    bench_to_canonical_json_nested,
    bench_to_canonical_json_complex,
    bench_canonical_hash_simple,
    bench_canonical_hash_nested,
    bench_canonical_hash_complex,
    bench_verify_canonical_match,
    bench_adic_message_compute_id,
    bench_canonical_hash_scaling,
    bench_canonical_hash_throughput,
    bench_key_ordering,
    bench_null_omission,
);

criterion_main!(benches);
