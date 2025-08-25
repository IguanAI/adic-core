use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use adic_consensus::{ConsensusEngine, ConsensusConfig};
use adic_types::{AdicMessage, MessageId, QpDigits, Features};
use adic_crypto::Keypair;
use std::time::Duration;

fn create_test_message(i: u32, parents: Vec<MessageId>) -> AdicMessage {
    let id = MessageId::new(&format!("msg_{}", i).into_bytes());
    let features = Features {
        time: QpDigits::from_u64(3, i as u64),
        topic: QpDigits::from_u64(3, (i % 10) as u64),
        region: QpDigits::from_u64(3, (i % 5) as u64),
    };
    
    AdicMessage {
        id,
        parents,
        features,
        author: format!("author_{}", i % 3).into_bytes(),
        signature: vec![0; 64],
        payload: format!("data_{}", i).into_bytes(),
        timestamp: i as u64,
        deposit: 100,
    }
}

fn bench_message_processing(c: &mut Criterion) {
    let mut group = c.benchmark_group("message_processing");
    group.measurement_time(Duration::from_secs(10));
    
    for size in &[10, 100, 1000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            size,
            |b, &size| {
                let config = ConsensusConfig {
                    k: 10,
                    c: 0.5,
                    alpha: 2.0,
                    tau: 0.1,
                    max_parents: 8,
                    p: 3,
                    delta: 0.05,
                };
                
                let mut engine = ConsensusEngine::new(config);
                
                // Bootstrap with messages
                let mut prev_ids = vec![];
                for i in 0..size {
                    let parents = if i < 3 {
                        vec![]
                    } else {
                        prev_ids[prev_ids.len()-3..].to_vec()
                    };
                    
                    let msg = create_test_message(i, parents);
                    prev_ids.push(msg.id.clone());
                    let _ = engine.process_message(msg);
                }
                
                b.iter(|| {
                    let parents = prev_ids[prev_ids.len()-3..].to_vec();
                    let msg = create_test_message(size + 1, parents);
                    black_box(engine.process_message(msg))
                });
            }
        );
    }
    group.finish();
}

fn bench_admissibility_check(c: &mut Criterion) {
    let mut group = c.benchmark_group("admissibility");
    
    let config = ConsensusConfig {
        k: 10,
        c: 0.5,
        alpha: 2.0,
        tau: 0.1,
        max_parents: 8,
        p: 3,
        delta: 0.05,
    };
    
    let mut engine = ConsensusEngine::new(config);
    
    // Setup DAG
    for i in 0..100 {
        let parents = if i < 3 { vec![] } else { vec![] };
        let msg = create_test_message(i, parents);
        let _ = engine.process_message(msg);
    }
    
    group.bench_function("check_single", |b| {
        let msg = create_test_message(101, vec![]);
        b.iter(|| {
            black_box(engine.check_message(&msg))
        });
    });
    
    group.finish();
}

fn bench_finality_computation(c: &mut Criterion) {
    let mut group = c.benchmark_group("finality");
    group.measurement_time(Duration::from_secs(10));
    
    for dag_size in &[100, 500, 1000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(dag_size),
            dag_size,
            |b, &size| {
                let config = ConsensusConfig {
                    k: 10,
                    c: 0.5,
                    alpha: 2.0,
                    tau: 0.1,
                    max_parents: 8,
                    p: 3,
                    delta: 0.05,
                };
                
                let mut engine = ConsensusEngine::new(config);
                
                // Build DAG
                let mut all_ids = vec![];
                for i in 0..size {
                    let parents = if i < 3 {
                        vec![]
                    } else {
                        // Select random parents
                        let start = (i as usize).saturating_sub(10);
                        all_ids[start..i as usize].iter()
                            .step_by(3)
                            .take(3)
                            .cloned()
                            .collect()
                    };
                    
                    let msg = create_test_message(i, parents);
                    all_ids.push(msg.id.clone());
                    let _ = engine.process_message(msg);
                }
                
                b.iter(|| {
                    black_box(engine.compute_finality())
                });
            }
        );
    }
    group.finish();
}

fn bench_mrw_selection(c: &mut Criterion) {
    use adic_mrw::MultiAxisRandomWalk;
    use adic_types::DAG;
    use std::collections::HashMap;
    
    let mut group = c.benchmark_group("mrw_selection");
    
    for num_tips in &[10, 50, 100] {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_tips),
            num_tips,
            |b, &num_tips| {
                let mrw = MultiAxisRandomWalk::new(3, 2.0);
                let mut dag = DAG::new();
                let mut tips = vec![];
                
                // Create tips
                for i in 0..num_tips {
                    let msg = create_test_message(i, vec![]);
                    tips.push(msg.id.clone());
                    dag.messages.insert(msg.id.clone(), msg);
                }
                
                // Create reputation map
                let reputation: HashMap<Vec<u8>, f64> = (0..3)
                    .map(|i| (format!("author_{}", i).into_bytes(), 1.0))
                    .collect();
                
                b.iter(|| {
                    black_box(mrw.select_parents(&dag, &tips, 8, &reputation))
                });
            }
        );
    }
    group.finish();
}

fn bench_padic_operations(c: &mut Criterion) {
    use adic_math::PadicOperations;
    
    let mut group = c.benchmark_group("padic_ops");
    
    group.bench_function("valuation", |b| {
        let x = QpDigits::from_u64(3, 243);
        let y = QpDigits::from_u64(3, 81);
        b.iter(|| {
            black_box(PadicOperations::vp_diff(&x, &y))
        });
    });
    
    group.bench_function("distance", |b| {
        let x = QpDigits::from_u64(3, 100);
        let y = QpDigits::from_u64(3, 200);
        b.iter(|| {
            black_box(PadicOperations::distance(&x, &y))
        });
    });
    
    group.bench_function("ball_id", |b| {
        let x = QpDigits::from_u64(3, 123456);
        b.iter(|| {
            black_box(PadicOperations::ball_id(&x, 5))
        });
    });
    
    group.finish();
}

criterion_group!(
    benches,
    bench_message_processing,
    bench_admissibility_check,
    bench_finality_computation,
    bench_mrw_selection,
    bench_padic_operations
);
criterion_main!(benches);