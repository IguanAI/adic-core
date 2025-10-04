use adic_crypto::Keypair;
use adic_finality::ph::{
    bottleneck_distance, reduce_boundary_matrix, AdicComplexBuilder, BoundaryMatrix, F2Config,
    F2FinalityChecker, PersistenceData,
};
use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AxisPhi, QpDigits, DEFAULT_P, DEFAULT_PRECISION,
};
use chrono::Utc;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

fn create_test_message(value: u64, keypair: &Keypair) -> AdicMessage {
    let features = AdicFeatures::new(vec![AxisPhi::new(
        0,
        QpDigits::from_u64(value, DEFAULT_P, DEFAULT_PRECISION),
    )]);

    let meta = AdicMeta::new(Utc::now());

    let mut msg = AdicMessage::new(
        vec![],
        features,
        meta,
        *keypair.public_key(),
        format!("bench_{}", value).into_bytes(),
    );

    let signature = keypair.sign(&msg.to_bytes());
    msg.signature = signature;

    msg
}

fn bench_complex_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("complex_construction");
    let keypair = Keypair::generate();

    for size in [10, 25, 50, 100].iter() {
        let messages: Vec<_> = (0..*size)
            .map(|i| create_test_message(i, &keypair))
            .collect();

        group.bench_with_input(BenchmarkId::from_parameter(size), &messages, |b, msgs| {
            b.iter(|| {
                let mut builder = AdicComplexBuilder::new();
                black_box(builder.build_from_messages(msgs, 0, 3, 2))
            })
        });
    }
    group.finish();
}

fn bench_boundary_matrix_reduction(c: &mut Criterion) {
    let mut group = c.benchmark_group("boundary_matrix_reduction");
    let keypair = Keypair::generate();

    for size in [10, 25, 50].iter() {
        // Pre-build complex
        let messages: Vec<_> = (0..*size)
            .map(|i| create_test_message(i, &keypair))
            .collect();

        let mut builder = AdicComplexBuilder::new();
        let complex = builder.build_from_messages(&messages, 0, 3, 2);
        let matrix = BoundaryMatrix::from_complex(&complex);

        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &(matrix, complex),
            |b, (_mat, _comp)| {
                b.iter(|| {
                    let matrix_copy = BoundaryMatrix::from_complex(_comp);
                    black_box(reduce_boundary_matrix(matrix_copy))
                })
            },
        );
    }
    group.finish();
}

fn bench_persistence_computation(c: &mut Criterion) {
    let mut group = c.benchmark_group("persistence_computation");
    let keypair = Keypair::generate();

    for size in [10, 25, 50].iter() {
        // Pre-build complex and reduce matrix
        let messages: Vec<_> = (0..*size)
            .map(|i| create_test_message(i, &keypair))
            .collect();

        let mut builder = AdicComplexBuilder::new();
        let complex = builder.build_from_messages(&messages, 0, 3, 2);
        let matrix = BoundaryMatrix::from_complex(&complex);
        let reduction = reduce_boundary_matrix(matrix);

        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &(reduction, complex),
            |b, (red, comp)| b.iter(|| black_box(PersistenceData::from_reduction(red, comp))),
        );
    }
    group.finish();
}

fn bench_bottleneck_distance(c: &mut Criterion) {
    let mut group = c.benchmark_group("bottleneck_distance");
    let keypair = Keypair::generate();

    for size in [10, 20, 30].iter() {
        // Pre-build two persistence diagrams
        let messages1: Vec<_> = (0..*size)
            .map(|i| create_test_message(i, &keypair))
            .collect();
        let messages2: Vec<_> = (0..*size)
            .map(|i| create_test_message(i + 100, &keypair))
            .collect();

        let mut builder1 = AdicComplexBuilder::new();
        let complex1 = builder1.build_from_messages(&messages1, 0, 3, 2);
        let matrix1 = BoundaryMatrix::from_complex(&complex1);
        let reduction1 = reduce_boundary_matrix(matrix1);
        let persistence1 = PersistenceData::from_reduction(&reduction1, &complex1);

        let mut builder2 = AdicComplexBuilder::new();
        let complex2 = builder2.build_from_messages(&messages2, 0, 3, 2);
        let matrix2 = BoundaryMatrix::from_complex(&complex2);
        let reduction2 = reduce_boundary_matrix(matrix2);
        let persistence2 = PersistenceData::from_reduction(&reduction2, &complex2);

        if let (Some(diag1), Some(diag2)) =
            (persistence1.get_diagram(1), persistence2.get_diagram(1))
        {
            group.bench_with_input(
                BenchmarkId::from_parameter(size),
                &(diag1.clone(), diag2.clone()),
                |b, (d1, d2)| b.iter(|| black_box(bottleneck_distance(d1, d2))),
            );
        }
    }
    group.finish();
}

fn bench_f2_finality_check(c: &mut Criterion) {
    let mut group = c.benchmark_group("f2_finality_check");
    let keypair = Keypair::generate();

    for size in [10, 25, 50].iter() {
        let messages: Vec<_> = (0..*size)
            .map(|i| create_test_message(i, &keypair))
            .collect();

        let config = F2Config {
            dimension: 2,
            radius: 3,
            axis: 0,
            epsilon: 0.5,
            timeout_ms: 10000, // Long timeout for benchmarking
            use_streaming: false,
        };

        group.bench_with_input(BenchmarkId::from_parameter(size), &messages, |b, msgs| {
            b.iter(|| {
                let mut checker = F2FinalityChecker::new(config.clone());
                black_box(checker.check_finality(msgs))
            })
        });
    }
    group.finish();
}

fn bench_f2_full_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("f2_full_pipeline");
    group.sample_size(20); // Fewer samples for longer benchmark
    let keypair = Keypair::generate();

    for size in [10, 25, 50, 100].iter() {
        let messages: Vec<_> = (0..*size)
            .map(|i| create_test_message(i, &keypair))
            .collect();

        group.bench_with_input(BenchmarkId::from_parameter(size), &messages, |b, msgs| {
            b.iter(|| {
                // Full F2 pipeline: complex → matrix → reduction → persistence → bottleneck
                let mut builder = AdicComplexBuilder::new();
                let complex = builder.build_from_messages(msgs, 0, 3, 2);

                if complex.num_simplices() > 0 {
                    let matrix = BoundaryMatrix::from_complex(&complex);
                    let reduction = reduce_boundary_matrix(matrix);
                    let persistence = PersistenceData::from_reduction(&reduction, &complex);

                    // Simulate bottleneck distance check if we have diagrams
                    if let Some(diag1) = persistence.get_diagram(1) {
                        if let Some(diag2) = persistence.get_diagram(1) {
                            black_box(bottleneck_distance(diag1, diag2));
                        }
                    }
                }
            })
        });
    }
    group.finish();
}

fn bench_varying_dimensions(c: &mut Criterion) {
    let mut group = c.benchmark_group("varying_dimensions");
    let keypair = Keypair::generate();

    let messages: Vec<_> = (0..30).map(|i| create_test_message(i, &keypair)).collect();

    for dimension in [1, 2, 3].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(dimension),
            dimension,
            |b, &dim| {
                b.iter(|| {
                    let mut builder = AdicComplexBuilder::new();
                    let complex = builder.build_from_messages(&messages, 0, 3, dim);

                    if complex.num_simplices() > 0 {
                        let matrix = BoundaryMatrix::from_complex(&complex);
                        black_box(reduce_boundary_matrix(matrix));
                    }
                })
            },
        );
    }
    group.finish();
}

fn bench_varying_radius(c: &mut Criterion) {
    let mut group = c.benchmark_group("varying_radius");
    let keypair = Keypair::generate();

    let messages: Vec<_> = (0..30).map(|i| create_test_message(i, &keypair)).collect();

    for radius in [2, 3, 4, 5].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(radius), radius, |b, &rad| {
            b.iter(|| {
                let mut builder = AdicComplexBuilder::new();
                black_box(builder.build_from_messages(&messages, 0, rad, 2))
            })
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_complex_construction,
    bench_boundary_matrix_reduction,
    bench_persistence_computation,
    bench_bottleneck_distance,
    bench_f2_finality_check,
    bench_f2_full_pipeline,
    bench_varying_dimensions,
    bench_varying_radius,
);
criterion_main!(benches);
