use adic_consensus::ConsensusEngine;
use adic_crypto::Keypair;
use adic_math::{ball_id, padic_distance, vp_diff};
use adic_storage::{
    store::BackendType, MemoryBackend, StorageBackend, StorageConfig, StorageEngine,
};
use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AdicParams, AxisPhi, MessageId, QpDigits, DEFAULT_P,
    DEFAULT_PRECISION,
};
use chrono::Utc;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

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

    let meta = AdicMeta::new(Utc::now());

    let mut msg = AdicMessage::new(
        parents,
        features,
        meta,
        *keypair.public_key(),
        format!("data_{}", i).into_bytes(),
    );

    // Sign the message
    let signature = keypair.sign(&msg.to_bytes());
    msg.signature = signature;

    msg
}

struct TestContext {
    engine: ConsensusEngine,
    storage: Arc<StorageEngine>,
    messages: HashMap<MessageId, AdicMessage>,
}

impl TestContext {
    fn new(params: AdicParams) -> Self {
        let storage = Arc::new(
            StorageEngine::new(StorageConfig {
                backend_type: BackendType::Memory,
                ..Default::default()
            })
            .unwrap(),
        );
        Self {
            engine: ConsensusEngine::new(params, storage.clone()),
            storage,
            messages: HashMap::new(),
        }
    }

    async fn add_message(&mut self, msg: AdicMessage) -> Result<(), Box<dyn std::error::Error>> {
        // Store message in storage engine
        self.storage.store_message(&msg).await?;
        self.messages.insert(msg.id, msg);
        Ok(())
    }

    fn get_parent_features(&self, message: &AdicMessage) -> Vec<Vec<QpDigits>> {
        message
            .parents
            .iter()
            .filter_map(|parent_id| {
                self.messages.get(parent_id).map(|parent| {
                    parent
                        .features
                        .phi
                        .iter()
                        .map(|axis| axis.qp_digits.clone())
                        .collect()
                })
            })
            .collect()
    }

    fn get_parent_reputations(&self, message: &AdicMessage) -> Vec<f64> {
        // For benchmarking, return default reputation scores
        // In a real system, these would be async lookups
        message
            .parents
            .iter()
            .map(|_| 10.0) // Default reputation score
            .collect()
    }
}

fn bench_message_processing(c: &mut Criterion) {
    let mut group = c.benchmark_group("message_processing");
    group.measurement_time(Duration::from_secs(5));

    // Create a runtime for async operations
    let runtime = tokio::runtime::Runtime::new().unwrap();

    for size in &[10, 50, 100] {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let params = AdicParams::default();
            let mut context = TestContext::new(params);
            let keypair = Keypair::generate();

            // Bootstrap with messages using async runtime
            let mut prev_ids = vec![];
            runtime.block_on(async {
                for i in 0..size {
                    let parents = if i < 3 {
                        vec![]
                    } else {
                        prev_ids[prev_ids.len() - 3..].to_vec()
                    };

                    let msg = create_test_message(i, parents, &keypair);
                    prev_ids.push(msg.id);
                    context.add_message(msg).await.unwrap();
                }
            });

            b.iter(|| {
                let parents = if prev_ids.len() >= 3 {
                    prev_ids[prev_ids.len() - 3..].to_vec()
                } else {
                    vec![]
                };
                let msg = create_test_message(size + 1, parents, &keypair);

                // Benchmark both validation and storage
                runtime.block_on(async {
                    let validation = context.engine.validator().validate_message(&msg);
                    if validation.is_valid {
                        context.storage.store_message(&msg).await.unwrap();
                    }
                    black_box(validation)
                })
            });
        });
    }
    group.finish();
}

fn bench_admissibility_check(c: &mut Criterion) {
    let mut group = c.benchmark_group("admissibility");

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let params = AdicParams::default();
    let mut context = TestContext::new(params.clone());
    let keypair = Keypair::generate();

    // Setup DAG with proper parents
    let mut all_ids = vec![];
    runtime.block_on(async {
        for i in 0..20 {
            let parents = if i < (params.d + 1) {
                vec![]
            } else {
                // Take last d+1 messages as parents
                let start = all_ids.len().saturating_sub((params.d + 1) as usize);
                all_ids[start..start + (params.d + 1) as usize].to_vec()
            };
            let msg = create_test_message(i, parents, &keypair);
            all_ids.push(msg.id);
            context.add_message(msg).await.unwrap();
        }
    });

    group.bench_function("check_admissibility", |b| {
        // Create a test message with proper number of parents
        let parents = if all_ids.len() >= (params.d + 1) as usize {
            let start = all_ids.len() - (params.d + 1) as usize;
            all_ids[start..].to_vec()
        } else {
            vec![]
        };

        let msg = create_test_message(101, parents, &keypair);
        let parent_features = context.get_parent_features(&msg);
        let parent_reputations = context.get_parent_reputations(&msg);

        b.iter(|| {
            black_box(context.engine.admissibility().check_message(
                &msg,
                &parent_features,
                &parent_reputations,
            ))
        });
    });

    group.finish();
}

fn bench_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("validation");

    let params = AdicParams::default();
    let context = TestContext::new(params.clone());
    let keypair = Keypair::generate();

    group.bench_function("validate_message", |b| {
        let msg = create_test_message(1, vec![], &keypair);
        b.iter(|| black_box(context.engine.validator().validate_message(&msg)));
    });

    group.bench_function("validate_with_parents", |b| {
        let parent1 = create_test_message(1, vec![], &keypair);
        let parent2 = create_test_message(2, vec![], &keypair);
        let parent3 = create_test_message(3, vec![], &keypair);
        let parent4 = create_test_message(4, vec![], &keypair);

        let parents = vec![parent1.id, parent2.id, parent3.id, parent4.id];
        let msg = create_test_message(5, parents, &keypair);

        b.iter(|| black_box(context.engine.validator().validate_message(&msg)));
    });

    group.finish();
}

fn bench_padic_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("padic_ops");

    group.bench_function("valuation", |b| {
        let x = QpDigits::from_u64(243, DEFAULT_P, DEFAULT_PRECISION);
        let y = QpDigits::from_u64(81, DEFAULT_P, DEFAULT_PRECISION);
        b.iter(|| black_box(vp_diff(&x, &y)));
    });

    group.bench_function("distance", |b| {
        let x = QpDigits::from_u64(100, DEFAULT_P, DEFAULT_PRECISION);
        let y = QpDigits::from_u64(200, DEFAULT_P, DEFAULT_PRECISION);
        b.iter(|| black_box(padic_distance(&x, &y)));
    });

    group.bench_function("ball_id", |b| {
        let x = QpDigits::from_u64(123456, DEFAULT_P, DEFAULT_PRECISION);
        b.iter(|| black_box(ball_id(&x, 5)));
    });

    group.bench_function("ball_distance", |b| {
        let x = QpDigits::from_u64(100, DEFAULT_P, DEFAULT_PRECISION);
        let y = QpDigits::from_u64(200, DEFAULT_P, DEFAULT_PRECISION);
        let z = QpDigits::from_u64(300, DEFAULT_P, DEFAULT_PRECISION);

        b.iter(|| {
            let d1 = padic_distance(&x, &y);
            let d2 = padic_distance(&y, &z);
            let d3 = padic_distance(&x, &z);
            black_box((d1, d2, d3))
        });
    });

    group.finish();
}

fn bench_reputation(c: &mut Criterion) {
    let mut group = c.benchmark_group("reputation");

    let params = AdicParams::default();
    let storage = Arc::new(
        StorageEngine::new(StorageConfig {
            backend_type: BackendType::Memory,
            ..Default::default()
        })
        .unwrap(),
    );
    let engine = ConsensusEngine::new(params, storage);
    let keypair = Keypair::generate();
    let pubkey = *keypair.public_key();

    group.bench_function("good_update", |b| {
        b.iter(|| {
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                // good_update(pubkey, diversity, depth)
                let _: () = engine.reputation.good_update(&pubkey, 0.8, 10).await;
                black_box(())
            })
        });
    });

    group.bench_function("bad_update", |b| {
        b.iter(|| {
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                let _: () = engine.reputation.bad_update(&pubkey, 1.0).await;
                black_box(())
            })
        });
    });

    group.bench_function("get_trust_score", |b| {
        // Pre-populate some scores
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            engine.reputation.good_update(&pubkey, 0.8, 10).await;
        });

        b.iter(|| {
            runtime.block_on(async { black_box(engine.reputation.get_trust_score(&pubkey).await) })
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_message_processing,
    bench_admissibility_check,
    bench_validation,
    bench_padic_operations,
    bench_reputation
);
criterion_main!(benches);
