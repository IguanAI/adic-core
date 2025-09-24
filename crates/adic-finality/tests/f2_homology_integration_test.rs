//! F2 Homology Finality Integration Tests
//!
//! Comprehensive integration tests for persistent homology-based finality
//! covering complex DAG structures, stability analysis, and F1/F2 coordination.

use adic_consensus::ConsensusEngine;
use adic_crypto::Keypair;
use adic_finality::{homology::HomologyConfig, FinalityConfig, FinalityEngine, HomologyAnalyzer};
use adic_storage::{store::BackendType, StorageConfig, StorageEngine};
use adic_types::{AdicFeatures, AdicMessage, AdicMeta, AdicParams, AxisPhi, MessageId, QpDigits};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::tempdir;

fn create_test_storage() -> Arc<StorageEngine> {
    let temp_dir = tempdir().unwrap();
    let storage_config = StorageConfig {
        backend_type: BackendType::RocksDB {
            path: temp_dir.path().to_str().unwrap().to_string(),
        },
        cache_size: 10,
        flush_interval_ms: 1000,
        max_batch_size: 100,
    };
    Arc::new(StorageEngine::new(storage_config).unwrap())
}
use std::time::Instant;

/// Helper to create test messages with specific features
fn create_message_with_features(
    id: &str,
    parents: Vec<MessageId>,
    features: Vec<u64>,
    keypair: &Keypair,
) -> AdicMessage {
    let axis_features = features
        .into_iter()
        .enumerate()
        .map(|(i, val)| AxisPhi::new(i as u32, QpDigits::from_u64(val, 3, 10)))
        .collect();

    let mut message = AdicMessage::new(
        parents,
        AdicFeatures::new(axis_features),
        AdicMeta::new(Utc::now()),
        *keypair.public_key(),
        format!("Message {}", id).into_bytes(),
    );

    message.signature = keypair.sign(&message.to_bytes());
    message
}

#[tokio::test]
async fn test_f2_with_diamond_dag_structure() {
    let params = AdicParams::default();
    let analyzer = HomologyAnalyzer::new(params.clone());
    let keypair = Keypair::generate();

    // Create diamond DAG structure:
    //     root
    //    /    \
    //   a      b
    //    \    /
    //     join

    let root = create_message_with_features("root", vec![], vec![100, 100, 100], &keypair);
    let a = create_message_with_features("a", vec![root.id], vec![110, 100, 100], &keypair);
    let b = create_message_with_features("b", vec![root.id], vec![100, 110, 100], &keypair);
    let join =
        create_message_with_features("join", vec![a.id, b.id], vec![110, 110, 100], &keypair);

    // Build simplicial complex from diamond
    let messages = vec![
        (root.id, vec![], 2.0),
        (a.id, vec![root.id], 1.8),
        (b.id, vec![root.id], 1.8),
        (join.id, vec![a.id, b.id], 1.5),
    ];

    analyzer
        .build_complex_from_messages(&messages)
        .await
        .unwrap();

    // Compute homology
    let homology = analyzer.compute_persistent_homology().await.unwrap();

    // Diamond should create a 1-dimensional hole (cycle)
    let h1_dims: Vec<_> = homology
        .dimensions
        .iter()
        .filter(|d| d.dimension == 1)
        .collect();

    assert!(
        !h1_dims.is_empty(),
        "Should detect 1-dimensional features in diamond"
    );

    // Check finality
    let finality_result = analyzer.check_finality(&[join.id]).await.unwrap();
    assert!(finality_result.status.contains("PENDING") || finality_result.status.contains("F2"));
}

#[tokio::test]
async fn test_f2_stability_window_convergence() {
    let config = HomologyConfig {
        window_size: 5,
        bottleneck_epsilon: 0.1,
        ..Default::default()
    };

    let params = AdicParams::default();
    let analyzer = HomologyAnalyzer::with_config(params, config);

    // Build increasingly stable complexes
    for round in 0..10 {
        let mut messages = vec![];

        // Create a consistent structure with small variations
        for i in 0..5 {
            let variation = if round < 5 { round as f64 * 0.1 } else { 0.5 };
            let weight = 1.0 + variation + (i as f64) * 0.1;

            let parents = if i == 0 {
                vec![]
            } else {
                vec![MessageId::new(format!("msg{}", i - 1).as_bytes())]
            };

            messages.push((
                MessageId::new(format!("msg{}", i).as_bytes()),
                parents,
                weight,
            ));
        }

        analyzer
            .build_complex_from_messages(&messages)
            .await
            .unwrap();

        // Check if we've achieved stability
        if round >= 5 {
            let result = analyzer
                .check_finality(&[MessageId::new(b"msg0")])
                .await
                .unwrap();

            // After stabilization, should see consistent results
            if round >= 7 {
                assert!(
                    result.stability_score < 0.5,
                    "Should achieve stability after consistent rounds, got score: {}",
                    result.stability_score
                );
            }
        }
    }
}

#[tokio::test]
async fn test_f2_with_complex_simplicial_structure() {
    let params = AdicParams::default();
    let analyzer = HomologyAnalyzer::new(params);

    // Create a complex with multiple dimensional features
    // This represents a "filled tetrahedron" with internal structure
    let vertices = [
        MessageId::new(b"v0"),
        MessageId::new(b"v1"),
        MessageId::new(b"v2"),
        MessageId::new(b"v3"),
        MessageId::new(b"v4"), // Center vertex
    ];

    // Add edges (1-simplices)
    for i in 0..4 {
        for j in i + 1..4 {
            analyzer
                .add_simplex(vec![vertices[i], vertices[j]], 1.0 + i as f64 * 0.1)
                .await
                .unwrap();
        }
        // Connect to center
        analyzer
            .add_simplex(vec![vertices[i], vertices[4]], 1.5)
            .await
            .unwrap();
    }

    // Add triangles (2-simplices)
    for i in 0..4 {
        for j in i + 1..4 {
            for k in j + 1..4 {
                analyzer
                    .add_simplex(vec![vertices[i], vertices[j], vertices[k]], 2.0)
                    .await
                    .unwrap();
            }
        }
    }

    // Add tetrahedron (3-simplex)
    analyzer
        .add_simplex(
            vec![vertices[0], vertices[1], vertices[2], vertices[3]],
            3.0,
        )
        .await
        .unwrap();

    let homology = analyzer.compute_persistent_homology().await.unwrap();

    // Should have features at multiple dimensions
    assert!(homology.max_dimension >= 3);

    // Check for proper Betti numbers
    for dim_result in &homology.dimensions {
        println!(
            "Dimension {}: {} bars, Betti number: {}",
            dim_result.dimension,
            dim_result.bars.len(),
            dim_result.betti_number
        );

        // Verify consistency
        assert!(dim_result.betti_number <= dim_result.bars.len());
    }
}

#[tokio::test]
async fn test_f1_f2_dual_finality_coordination() {
    let params = AdicParams {
        k: 3, // Lower k-core for testing
        depth_star: 2,
        ..Default::default()
    };

    let storage = create_test_storage();
    let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
    let finality_config = FinalityConfig::from(&params);
    let finality_engine = FinalityEngine::new(finality_config, consensus, storage);

    let keypair = Keypair::generate();

    // Build a structure that should pass F1 but might not pass F2
    let root = create_message_with_features("root", vec![], vec![100, 100, 100], &keypair);

    // Add to both F1 and F2 engines
    let mut ball_ids = HashMap::new();
    for axis_phi in &root.features.phi {
        ball_ids.insert(axis_phi.axis.0, axis_phi.qp_digits.ball_id(3));
    }

    finality_engine
        .add_message(root.id, vec![], 2.0, ball_ids.clone())
        .await
        .unwrap();

    // Create enough messages for k-core
    let mut parent_id = root.id;
    for i in 1..10 {
        let msg = create_message_with_features(
            &format!("msg{}", i),
            vec![parent_id],
            vec![100 + i, 100 + i, 100 + i],
            &keypair,
        );

        let mut ball_ids = HashMap::new();
        for axis_phi in &msg.features.phi {
            ball_ids.insert(axis_phi.axis.0, axis_phi.qp_digits.ball_id(3));
        }

        finality_engine
            .add_message(msg.id, vec![parent_id], 1.5, ball_ids)
            .await
            .unwrap();
        parent_id = msg.id;
    }

    // Check F1 finality
    let f1_finalized = finality_engine.check_finality().await.unwrap();

    // Check F2 finality
    let f2_result = finality_engine
        .check_homology_finality(&[root.id])
        .await
        .unwrap();

    println!("F1 finalized: {} messages", f1_finalized.len());
    println!(
        "F2 status: {}, score: {}",
        f2_result.status, f2_result.stability_score
    );

    // Both should provide consistent finality decisions
    if !f1_finalized.is_empty() {
        assert!(
            f2_result.status.contains("F2") || f2_result.stability_score > 0.0,
            "F2 should show progress when F1 finalizes"
        );
    }
}

#[tokio::test]
async fn test_f2_streaming_complex_updates() {
    let params = AdicParams::default();
    let analyzer = HomologyAnalyzer::new(params);
    let keypair = Keypair::generate();

    // Simulate real-time message arrival
    let mut message_stream = vec![];
    for i in 0..20 {
        let parents = if i == 0 {
            vec![]
        } else {
            // Random parent selection to create complex topology
            let num_parents = (i % 3) + 1;
            let mut parent_ids = vec![];
            for j in 0..num_parents.min(i) {
                parent_ids.push(message_stream[i - j - 1]);
            }
            parent_ids
        };

        let msg = create_message_with_features(
            &format!("stream{}", i),
            parents.clone(),
            vec![i as u64 * 10, i as u64 * 20, i as u64 * 30],
            &keypair,
        );

        message_stream.push(msg.id);

        // Add to complex in real-time
        let mut vertices = vec![msg.id];
        vertices.extend(parents.clone());

        analyzer
            .add_simplex(vertices, 1.0 + i as f64 * 0.1)
            .await
            .unwrap();

        // Periodically check homology
        if i % 5 == 4 {
            let homology = analyzer.compute_persistent_homology().await.unwrap();
            assert!(!homology.filtration_values.is_empty());

            println!(
                "After {} messages: {} filtration values, max dim: {}",
                i + 1,
                homology.filtration_values.len(),
                homology.max_dimension
            );
        }
    }

    // Final homology should be rich
    let final_homology = analyzer.compute_persistent_homology().await.unwrap();
    assert!(final_homology.max_dimension > 0);
    assert!(final_homology.filtration_values.len() >= 10);
}

#[tokio::test]
async fn test_f2_with_cycles_and_voids() {
    let params = AdicParams::default();
    let analyzer = HomologyAnalyzer::new(params);

    // Create a structure with a 2D void (hollow sphere-like)
    // Outer shell vertices
    let outer = [
        MessageId::new(b"o1"),
        MessageId::new(b"o2"),
        MessageId::new(b"o3"),
        MessageId::new(b"o4"),
        MessageId::new(b"o5"),
        MessageId::new(b"o6"),
    ];

    // Create triangulated sphere surface (2-dimensional void)
    for i in 0..6 {
        for j in i + 1..6 {
            for k in j + 1..6 {
                // Add triangle faces to create closed surface
                if (i + j + k) % 2 == 0 {
                    // Only some triangles to maintain structure
                    analyzer
                        .add_simplex(vec![outer[i], outer[j], outer[k]], 2.0 + i as f64 * 0.1)
                        .await
                        .unwrap();
                }
            }
        }
    }

    let homology = analyzer.compute_persistent_homology().await.unwrap();

    // Should detect the 2-dimensional void
    let h2_dims: Vec<_> = homology
        .dimensions
        .iter()
        .filter(|d| d.dimension == 2)
        .collect();

    assert!(!h2_dims.is_empty(), "Should detect 2-dimensional features");

    // Check persistence of the void
    for dim in h2_dims {
        for bar in &dim.bars {
            if bar.death.is_none() {
                println!("Found persistent 2D feature with birth at {}", bar.birth);
            }
        }
    }
}

#[tokio::test]
async fn test_f2_performance_with_large_complex() {
    let params = AdicParams::default();
    let analyzer = HomologyAnalyzer::new(params);

    const NUM_MESSAGES: usize = 1000;
    let start = Instant::now();

    // Build a large complex
    let mut vertices = Vec::new();
    for i in 0..NUM_MESSAGES {
        vertices.push(MessageId::new(format!("large{}", i).as_bytes()));
    }

    // Add simplices with structured pattern
    for i in 0..NUM_MESSAGES {
        // Add edges to nearby vertices
        for j in 1..=3 {
            if i + j < NUM_MESSAGES {
                analyzer
                    .add_simplex(vec![vertices[i], vertices[i + j]], 1.0 + (i as f64) * 0.001)
                    .await
                    .unwrap();
            }
        }

        // Add some triangles
        if i + 2 < NUM_MESSAGES {
            analyzer
                .add_simplex(
                    vec![vertices[i], vertices[i + 1], vertices[i + 2]],
                    2.0 + (i as f64) * 0.001,
                )
                .await
                .unwrap();
        }
    }

    let build_time = start.elapsed();

    // Compute homology
    let homology_start = Instant::now();
    let homology = analyzer.compute_persistent_homology().await.unwrap();
    let homology_time = homology_start.elapsed();

    println!("Large complex performance:");
    println!("  Build time: {:?}", build_time);
    println!("  Homology computation: {:?}", homology_time);
    println!("  Filtration values: {}", homology.filtration_values.len());
    println!("  Max dimension: {}", homology.max_dimension);

    // Performance assertions
    assert!(
        build_time.as_secs() < 5,
        "Building should complete within 5 seconds"
    );
    assert!(
        homology_time.as_secs() < 2,
        "Homology should compute within 2 seconds"
    );
}

#[tokio::test]
async fn test_f2_memory_efficiency() {
    let config = HomologyConfig {
        window_size: 5, // Small window to test memory management
        ..Default::default()
    };

    let params = AdicParams::default();
    let analyzer = HomologyAnalyzer::with_config(params, config);

    // Add many complexes to test memory bounds
    for batch in 0..20 {
        let mut messages = vec![];
        for i in 0..100 {
            let msg_id = MessageId::new(format!("batch{}msg{}", batch, i).as_bytes());
            let parents = if i == 0 {
                vec![]
            } else {
                vec![MessageId::new(b"parent")]
            };
            messages.push((msg_id, parents, 1.0 + i as f64 * 0.01));
        }

        analyzer
            .build_complex_from_messages(&messages)
            .await
            .unwrap();
        analyzer
            .check_finality(&[MessageId::new(b"test")])
            .await
            .unwrap();
    }

    // Verify memory is bounded by window
    // Note: history is private, so we verify indirectly through finality results
    let result = analyzer
        .check_finality(&[MessageId::new(b"test")])
        .await
        .unwrap();
    assert!(
        result.window_results.len() <= 10, // window_size * 2
        "History should be bounded, got {} entries",
        result.window_results.len()
    );
}

#[tokio::test]
async fn test_f2_bottleneck_distance_accuracy() {
    let params = AdicParams::default();
    let analyzer = HomologyAnalyzer::new(params);

    // Create two similar complexes with known bottleneck distance
    let complex1 = vec![
        (MessageId::new(b"c1_1"), vec![], 1.0),
        (MessageId::new(b"c1_2"), vec![MessageId::new(b"c1_1")], 2.0),
        (MessageId::new(b"c1_3"), vec![MessageId::new(b"c1_2")], 3.0),
    ];

    analyzer
        .build_complex_from_messages(&complex1)
        .await
        .unwrap();
    let h1 = analyzer.compute_persistent_homology().await.unwrap();

    // Slightly perturbed version
    let complex2 = vec![
        (MessageId::new(b"c2_1"), vec![], 1.1), // Shifted by 0.1
        (MessageId::new(b"c2_2"), vec![MessageId::new(b"c2_1")], 2.1),
        (MessageId::new(b"c2_3"), vec![MessageId::new(b"c2_2")], 3.1),
    ];

    analyzer
        .build_complex_from_messages(&complex2)
        .await
        .unwrap();
    let h2 = analyzer.compute_persistent_homology().await.unwrap();

    // Compare persistence diagrams
    for dim in 0..=h1.max_dimension.min(h2.max_dimension) {
        let bars1 = h1
            .dimensions
            .iter()
            .find(|d| d.dimension == dim)
            .map(|d| &d.bars[..])
            .unwrap_or(&[]);

        let bars2 = h2
            .dimensions
            .iter()
            .find(|d| d.dimension == dim)
            .map(|d| &d.bars[..])
            .unwrap_or(&[]);

        // Compare persistence manually since bottleneck_distance is private
        // We expect similar structure but slightly different filtration values
        if !bars1.is_empty() && !bars2.is_empty() {
            // Check that we have similar number of bars
            let count_diff = (bars1.len() as i32 - bars2.len() as i32).abs();
            assert!(
                count_diff <= 1,
                "Should have similar number of persistence bars"
            );

            // For matching bars, check birth/death are close
            let min_len = bars1.len().min(bars2.len());
            for i in 0..min_len {
                let birth_diff = (bars1[i].birth - bars2[i].birth).abs();
                let death_diff = match (bars1[i].death, bars2[i].death) {
                    (Some(d1), Some(d2)) => (d1 - d2).abs(),
                    (None, None) => 0.0,
                    _ => continue, // Skip if one is infinite and the other isn't
                };

                // We shifted weights by 0.1, so differences should be around that
                assert!(birth_diff < 0.5, "Birth times should be close");
                assert!(death_diff < 0.5, "Death times should be close");
            }

            println!("Dimension {} successfully compared {} bars", dim, min_len);
        }
    }
}

#[tokio::test]
async fn test_f2_handles_disconnected_components() {
    let params = AdicParams::default();
    let analyzer = HomologyAnalyzer::new(params);

    // Create multiple disconnected components
    for component in 0..3 {
        let base = MessageId::new(format!("comp{}_base", component).as_bytes());
        let child1 = MessageId::new(format!("comp{}_c1", component).as_bytes());
        let child2 = MessageId::new(format!("comp{}_c2", component).as_bytes());

        // Each component is isolated
        analyzer
            .add_simplex(vec![base, child1], 1.0 + component as f64)
            .await
            .unwrap();
        analyzer
            .add_simplex(vec![base, child2], 1.5 + component as f64)
            .await
            .unwrap();
        analyzer
            .add_simplex(vec![child1, child2], 2.0 + component as f64)
            .await
            .unwrap();
    }

    let homology = analyzer.compute_persistent_homology().await.unwrap();

    // H0 (connected components) should show 3 components
    let h0 = homology
        .dimensions
        .iter()
        .find(|d| d.dimension == 0)
        .expect("Should have H0");

    // Count persistent features (components that never die)
    let persistent_components = h0.bars.iter().filter(|bar| bar.death.is_none()).count();

    println!(
        "Found {} persistent connected components",
        persistent_components
    );
    assert!(
        persistent_components >= 3,
        "Should detect disconnected components"
    );
}

#[tokio::test]
async fn test_f2_finality_under_attack_conditions() {
    let config = HomologyConfig {
        window_size: 5,
        bottleneck_epsilon: 0.1,
        ..Default::default()
    };

    let params = AdicParams::default();
    let analyzer = HomologyAnalyzer::with_config(params, config);

    // Simulate an attacker trying to manipulate finality
    // by creating artificial stability

    // First, build legitimate stable complexes
    for _ in 0..3 {
        let messages = vec![
            (MessageId::new(b"legit1"), vec![], 1.0),
            (
                MessageId::new(b"legit2"),
                vec![MessageId::new(b"legit1")],
                1.5,
            ),
        ];
        analyzer
            .build_complex_from_messages(&messages)
            .await
            .unwrap();
        analyzer
            .check_finality(&[MessageId::new(b"legit1")])
            .await
            .unwrap();
    }

    // Now attacker injects wildly different complex
    let attack_messages = vec![
        (MessageId::new(b"attack1"), vec![], 100.0), // Very different weight
        (MessageId::new(b"attack2"), vec![], 200.0),
        (MessageId::new(b"attack3"), vec![], 300.0),
    ];

    analyzer
        .build_complex_from_messages(&attack_messages)
        .await
        .unwrap();

    // Check if stability is broken
    let result = analyzer
        .check_finality(&[MessageId::new(b"target")])
        .await
        .unwrap();

    // Should detect the disruption
    assert!(
        result.stability_score > 1.0 || result.status.contains("PENDING"),
        "Should detect stability disruption from attack"
    );
}
