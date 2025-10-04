use adic_crypto::Keypair;
use adic_storage::{store::BackendType, StorageConfig, StorageEngine};
use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AxisPhi, MessageId, QpDigits, DEFAULT_P, DEFAULT_PRECISION,
};
use chrono::{Duration, Utc};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::Arc;

fn create_test_message(i: u32, parents: Vec<MessageId>, keypair: &Keypair) -> AdicMessage {
    let features = AdicFeatures::new(vec![
        AxisPhi::new(
            0,
            QpDigits::from_u64(i as u64, DEFAULT_P, DEFAULT_PRECISION),
        ),
        AxisPhi::new(
            1,
            QpDigits::from_u64((i % 10) as u64, DEFAULT_P, DEFAULT_PRECISION),
        ),
        AxisPhi::new(
            2,
            QpDigits::from_u64((i % 5) as u64, DEFAULT_P, DEFAULT_PRECISION),
        ),
    ]);

    let timestamp = Utc::now() + Duration::seconds(i as i64);
    let meta = AdicMeta::new(timestamp);

    let mut msg = AdicMessage::new(
        parents,
        features,
        meta,
        *keypair.public_key(),
        format!("data_{}", i).into_bytes(),
    );

    let signature = keypair.sign(&msg.to_bytes());
    msg.signature = signature;

    msg
}

fn bench_storage_writes(c: &mut Criterion) {
    let mut group = c.benchmark_group("storage_writes");
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let backend = BackendType::Memory;
    group.bench_with_input(
        BenchmarkId::from_parameter(format!("{:?}", backend)),
        &backend,
        |b, backend| {
            let storage = Arc::new(
                StorageEngine::new(StorageConfig {
                    backend_type: backend.clone(),
                    ..Default::default()
                })
                .unwrap(),
            );
            let keypair = Keypair::generate();

            b.iter(|| {
                let msg = create_test_message(1, vec![], &keypair);
                runtime.block_on(async {
                    storage.store_message(&msg).await.unwrap();
                    black_box(())
                })
            });
        },
    );
    group.finish();
}

fn bench_storage_reads(c: &mut Criterion) {
    let mut group = c.benchmark_group("storage_reads");
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let backend = BackendType::Memory;
    group.bench_with_input(
        BenchmarkId::from_parameter(format!("{:?}", backend)),
        &backend,
        |b, backend| {
            let storage = Arc::new(
                StorageEngine::new(StorageConfig {
                    backend_type: backend.clone(),
                    ..Default::default()
                })
                .unwrap(),
            );
            let keypair = Keypair::generate();

            // Pre-populate storage
            let msg = create_test_message(1, vec![], &keypair);
            let msg_id = msg.id;
            runtime.block_on(async {
                storage.store_message(&msg).await.unwrap();
            });

            b.iter(|| {
                runtime
                    .block_on(async { black_box(storage.get_message(&msg_id).await.unwrap()) })
            });
        },
    );
    group.finish();
}

fn bench_time_range_queries(c: &mut Criterion) {
    let mut group = c.benchmark_group("time_range_queries");
    let runtime = tokio::runtime::Runtime::new().unwrap();

    for size in &[10, 50, 100, 500] {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let storage = Arc::new(
                StorageEngine::new(StorageConfig {
                    backend_type: BackendType::Memory,
                    ..Default::default()
                })
                .unwrap(),
            );
            let keypair = Keypair::generate();

            // Pre-populate with messages
            let base_time = Utc::now();
            runtime.block_on(async {
                for i in 0..size {
                    let msg = create_test_message(i, vec![], &keypair);
                    storage.store_message(&msg).await.unwrap();
                }
            });

            let start_time = base_time + Duration::seconds(10);
            let end_time = base_time + Duration::seconds(size as i64 - 10);

            b.iter(|| {
                runtime.block_on(async {
                    black_box(
                        storage
                            .get_messages_in_range(start_time, end_time, 100, None)
                            .await
                            .unwrap(),
                    )
                })
            });
        });
    }
    group.finish();
}

fn bench_ball_queries(c: &mut Criterion) {
    let mut group = c.benchmark_group("ball_queries");
    let runtime = tokio::runtime::Runtime::new().unwrap();

    for size in &[10, 50, 100] {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let storage = Arc::new(
                StorageEngine::new(StorageConfig {
                    backend_type: BackendType::Memory,
                    ..Default::default()
                })
                .unwrap(),
            );
            let keypair = Keypair::generate();

            // Pre-populate with messages
            runtime.block_on(async {
                for i in 0..size {
                    let msg = create_test_message(i, vec![], &keypair);
                    storage.store_message(&msg).await.unwrap();
                }
            });

            let ball_id = vec![0u8; 3];

            b.iter(|| {
                runtime.block_on(async {
                    black_box(storage.get_ball_members(0, &ball_id).await.unwrap())
                })
            });
        });
    }
    group.finish();
}

fn bench_bulk_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulk_operations");
    let runtime = tokio::runtime::Runtime::new().unwrap();

    for batch_size in &[10, 50, 100] {
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            batch_size,
            |b, &batch_size| {
                let storage = Arc::new(
                    StorageEngine::new(StorageConfig {
                        backend_type: BackendType::Memory,
                        ..Default::default()
                    })
                    .unwrap(),
                );
                let keypair = Keypair::generate();

                // Pre-populate
                let mut message_ids = Vec::new();
                runtime.block_on(async {
                    for i in 0..batch_size {
                        let msg = create_test_message(i, vec![], &keypair);
                        message_ids.push(msg.id);
                        storage.store_message(&msg).await.unwrap();
                    }
                });

                b.iter(|| {
                    runtime.block_on(async {
                        black_box(storage.get_messages_bulk(&message_ids).await.unwrap())
                    })
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_storage_writes,
    bench_storage_reads,
    bench_time_range_queries,
    bench_ball_queries,
    bench_bulk_operations
);
criterion_main!(benches);
