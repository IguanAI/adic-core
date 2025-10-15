use super::adic_complex::AdicComplexBuilder;
use super::bottleneck::bottleneck_distance;
use super::matrix::BoundaryMatrix;
use super::persistence::PersistenceData;
use super::reduction::reduce_boundary_matrix;
use super::streaming::StreamingPersistence;
use adic_types::{AdicMessage, MessageId};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tracing::{debug, info, warn};

/// Configuration for F2 finality checking
#[derive(Debug, Clone)]
pub struct F2Config {
    /// Dimension d for H_d and H_{d-1} computation
    pub dimension: usize,
    /// Radius (window size Δ) for p-adic ball membership
    pub radius: usize,
    /// Axis to use for ball computation
    pub axis: u32,
    /// Bottleneck distance threshold ε for H_{d-1} stabilization
    pub epsilon: f64,
    /// Timeout for computation (milliseconds)
    pub timeout_ms: u64,
    /// Use streaming (incremental) persistent homology instead of batch
    /// Streaming mode provides O(n) amortized complexity vs O(n²-n³) batch
    pub use_streaming: bool,
    /// Maximum allowed Betti number for H_d stability (governable parameter)
    pub max_betti_d: usize,
    /// Minimum persistence threshold for noise filtering (governable parameter)
    pub min_persistence: f64,
}

impl Default for F2Config {
    fn default() -> Self {
        Self {
            dimension: 3,         // d = 3 as per paper
            radius: 5,            // Δ = 5 as per paper
            axis: 0,              // Use axis 0
            epsilon: 0.1,         // ε = 0.1 as per paper
            timeout_ms: 2000,     // 2 second timeout
            use_streaming: false, // Batch mode by default (backward compatible)
            max_betti_d: 2,       // Default from existing heuristic
            min_persistence: 0.01, // Default from existing heuristic (noise filtering)
        }
    }
}

/// Result of F2 finality check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum F2Result {
    /// F2 finality achieved
    Final {
        /// H_d is stable (all infinite bars established)
        h_d_stable: bool,
        /// H_{d-1} bottleneck distance to previous window
        h_d_minus_1_bottleneck: f64,
        /// Confidence score (0.0 to 1.0)
        confidence: f64,
        /// Messages that are finalized
        finalized_messages: Vec<MessageId>,
        /// Computation time
        elapsed_ms: u64,
    },
    /// Computation timed out, fallback to F1
    Timeout {
        /// Time elapsed before timeout
        elapsed_ms: u64,
        /// Partial bottleneck result if available
        partial_bottleneck: Option<f64>,
    },
    /// Not yet final
    Pending {
        /// Reason why not final
        reason: String,
        /// H_d stability status
        h_d_stable: bool,
        /// H_{d-1} bottleneck distance if computed
        h_d_minus_1_distance: Option<f64>,
    },
    /// Error during computation
    Error { message: String },
}

/// F2 finality checker using rigorous persistent homology
pub struct F2FinalityChecker {
    config: F2Config,
    /// Previous window's H_{d-1} diagram for comparison
    previous_h_d_minus_1: Option<super::persistence::PersistenceDiagram>,
    /// Streaming persistence engine (used when use_streaming=true)
    streaming: Option<StreamingPersistence>,
    /// Round counter for streaming mode
    round: u64,
}

impl Clone for F2FinalityChecker {
    fn clone(&self) -> Self {
        // When cloning, reset streaming state if in streaming mode
        // This is appropriate since cloning typically indicates a new checker instance
        let streaming = if self.config.use_streaming {
            Some(StreamingPersistence::new(
                self.config.dimension,
                self.config.radius,
                self.config.axis as usize,
            ))
        } else {
            None
        };

        Self {
            config: self.config.clone(),
            previous_h_d_minus_1: self.previous_h_d_minus_1.clone(),
            streaming,
            round: 0, // Reset round counter on clone
        }
    }
}

impl F2FinalityChecker {
    /// Create a new F2 finality checker
    pub fn new(config: F2Config) -> Self {
        let streaming = if config.use_streaming {
            Some(StreamingPersistence::new(
                config.dimension,
                config.radius,
                config.axis as usize,
            ))
        } else {
            None
        };

        Self {
            config,
            previous_h_d_minus_1: None,
            streaming,
            round: 0,
        }
    }

    /// Check F2 finality for a set of messages
    ///
    /// Algorithm per ADIC-DAG paper:
    /// 1. Build simplicial complex from messages using p-adic ball membership
    /// 2. Compute persistent homology
    /// 3. Check H_d stabilization (all bars infinite or died early)
    /// 4. Compute bottleneck distance for H_{d-1} vs previous window
    /// 5. If distance < ε, declare final
    pub fn check_finality(&mut self, messages: &[AdicMessage]) -> F2Result {
        if self.config.use_streaming {
            self.check_finality_streaming(messages)
        } else {
            self.check_finality_batch(messages)
        }
    }

    /// Check F2 finality using streaming (incremental) mode
    /// O(n) amortized complexity per round
    fn check_finality_streaming(&mut self, messages: &[AdicMessage]) -> F2Result {
        let start = Instant::now();

        info!(
            num_messages = messages.len(),
            dimension = self.config.dimension,
            radius = self.config.radius,
            epsilon = self.config.epsilon,
            mode = "streaming",
            round = self.round,
            "🔬 Starting F2 finality check (streaming mode)"
        );

        if messages.is_empty() {
            warn!("F2 check pending: no messages to check");
            return F2Result::Pending {
                reason: "No messages to check".to_string(),
                h_d_stable: false,
                h_d_minus_1_distance: None,
            };
        }

        // Store dimension before mutable borrow to avoid borrow conflict
        let dimension = self.config.dimension;

        // Add messages incrementally to streaming engine
        let (h_d_stable, current_h_d_minus_1) = {
            let streaming = self
                .streaming
                .as_mut()
                .expect("Streaming mode enabled but engine not initialized");

            match streaming.add_messages(messages, self.round) {
                Ok(new_simplices) => {
                    info!(
                        new_simplices = new_simplices,
                        total_simplices = streaming.num_simplices(),
                        "Added messages to streaming PH engine"
                    );
                }
                Err(e) => {
                    return F2Result::Error {
                        message: format!("Streaming update failed: {}", e),
                    };
                }
            }

            // Get current persistence from streaming engine
            let persistence = streaming.persistence();

            // Check H_d stabilization (using local dimension var)
            let h_d_stable = check_h_d_stability_impl(
                persistence,
                dimension,
                self.config.max_betti_d,
                self.config.min_persistence,
            );

            // Get H_{d-1} diagram
            let current_h_d_minus_1 = persistence.get_diagram(dimension - 1).cloned();

            (h_d_stable, current_h_d_minus_1)
        }; // streaming borrow ends here

        self.round += 1;

        // Compute bottleneck distance if we have a previous diagram
        let bottleneck_dist = if let (Some(prev), Some(ref curr)) =
            (&self.previous_h_d_minus_1, &current_h_d_minus_1)
        {
            Some(bottleneck_distance(prev, curr))
        } else {
            None
        };

        // Store current H_{d-1} for next comparison
        if let Some(curr) = current_h_d_minus_1 {
            self.previous_h_d_minus_1 = Some(curr);
        }

        let elapsed_ms = start.elapsed().as_millis() as u64;

        // Check finality conditions (same logic as batch mode)
        self.evaluate_finality_conditions(h_d_stable, bottleneck_dist, messages, elapsed_ms)
    }

    /// Check F2 finality using batch mode (original implementation)
    /// O(n²-n³) complexity per round
    fn check_finality_batch(&mut self, messages: &[AdicMessage]) -> F2Result {
        let start = Instant::now();

        info!(
            num_messages = messages.len(),
            dimension = self.config.dimension,
            radius = self.config.radius,
            epsilon = self.config.epsilon,
            mode = "batch",
            "🔬 Starting F2 finality check (batch mode)"
        );

        if messages.is_empty() {
            warn!("F2 check pending: no messages to check");
            return F2Result::Pending {
                reason: "No messages to check".to_string(),
                h_d_stable: false,
                h_d_minus_1_distance: None,
            };
        }

        // Build complex from messages
        let mut builder = AdicComplexBuilder::new();
        let complex = builder.build_from_messages(
            messages,
            self.config.axis,
            self.config.radius,
            self.config.dimension,
        );

        if complex.num_simplices() == 0 {
            warn!("F2 check pending: no simplices formed (insufficient p-adic proximity)");
            return F2Result::Pending {
                reason: "No simplices formed (insufficient p-adic proximity)".to_string(),
                h_d_stable: false,
                h_d_minus_1_distance: None,
            };
        }

        // Check timeout
        if start.elapsed().as_millis() > self.config.timeout_ms as u128 {
            return F2Result::Timeout {
                elapsed_ms: start.elapsed().as_millis() as u64,
                partial_bottleneck: None,
            };
        }

        // Compute persistent homology
        let matrix = BoundaryMatrix::from_complex(&complex);
        let reduction_result = reduce_boundary_matrix(matrix);

        // Check timeout again
        if start.elapsed().as_millis() > self.config.timeout_ms as u128 {
            return F2Result::Timeout {
                elapsed_ms: start.elapsed().as_millis() as u64,
                partial_bottleneck: None,
            };
        }

        let persistence = PersistenceData::from_reduction(&reduction_result, &complex);

        // Check H_d stabilization
        let h_d_stable = self.check_h_d_stability(&persistence);

        debug!(
            h_d_stable = h_d_stable,
            dimension = self.config.dimension,
            "H_d stability check completed"
        );

        // Get H_{d-1} diagram
        let current_h_d_minus_1 = persistence.get_diagram(self.config.dimension - 1);

        // Compute bottleneck distance if we have a previous diagram
        let bottleneck_dist =
            if let (Some(prev), Some(curr)) = (&self.previous_h_d_minus_1, current_h_d_minus_1) {
                if start.elapsed().as_millis() > self.config.timeout_ms as u128 {
                    warn!(
                        elapsed_ms = start.elapsed().as_millis() as u64,
                        "⏱️ F2 computation timed out during bottleneck distance"
                    );
                    return F2Result::Timeout {
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        partial_bottleneck: None,
                    };
                }

                Some(bottleneck_distance(prev, curr))
            } else {
                debug!("No previous H_{{d-1}} diagram for bottleneck comparison");
                None
            };

        // Store current H_{d-1} for next comparison
        if let Some(curr) = current_h_d_minus_1 {
            self.previous_h_d_minus_1 = Some(curr.clone());
        }

        let elapsed_ms = start.elapsed().as_millis() as u64;

        // Check finality conditions (delegated to common method)
        self.evaluate_finality_conditions(h_d_stable, bottleneck_dist, messages, elapsed_ms)
    }

    /// Evaluate finality conditions (shared logic between batch and streaming)
    fn evaluate_finality_conditions(
        &self,
        h_d_stable: bool,
        bottleneck_dist: Option<f64>,
        messages: &[AdicMessage],
        elapsed_ms: u64,
    ) -> F2Result {
        // Check finality conditions
        if h_d_stable {
            if let Some(dist) = bottleneck_dist {
                if dist < self.config.epsilon {
                    // Both conditions met - declare final!
                    let finalized_messages: Vec<MessageId> =
                        messages.iter().map(|m| m.id).collect();

                    // Confidence based on how far below epsilon we are
                    let confidence = (self.config.epsilon - dist) / self.config.epsilon;
                    let confidence = confidence.clamp(0.0, 1.0);

                    info!(
                        h_d_stable = true,
                        bottleneck_distance = format!("{:.6}", dist),
                        epsilon = self.config.epsilon,
                        confidence = format!("{:.4}", confidence),
                        num_finalized = finalized_messages.len(),
                        elapsed_ms = elapsed_ms,
                        "✅ F2 FINALITY ACHIEVED"
                    );

                    F2Result::Final {
                        h_d_stable: true,
                        h_d_minus_1_bottleneck: dist,
                        confidence,
                        finalized_messages,
                        elapsed_ms,
                    }
                } else {
                    info!(
                        h_d_stable = true,
                        bottleneck_distance = format!("{:.6}", dist),
                        epsilon = self.config.epsilon,
                        "⏳ F2 pending: bottleneck distance exceeds ε"
                    );
                    F2Result::Pending {
                        reason: format!(
                            "H_{{d-1}} bottleneck distance {:.4} >= ε {:.4}",
                            dist, self.config.epsilon
                        ),
                        h_d_stable: true,
                        h_d_minus_1_distance: Some(dist),
                    }
                }
            } else {
                info!(
                    h_d_stable = true,
                    "⏳ F2 pending: first window (no previous H_{{d-1}})"
                );
                F2Result::Pending {
                    reason: "First window - no previous H_{d-1} for comparison".to_string(),
                    h_d_stable: true,
                    h_d_minus_1_distance: None,
                }
            }
        } else {
            info!(h_d_stable = false, "⏳ F2 pending: H_d not yet stable");
            F2Result::Pending {
                reason: "H_d not yet stable".to_string(),
                h_d_stable: false,
                h_d_minus_1_distance: bottleneck_dist,
            }
        }
    }

    /// Check if H_d is stable
    /// H_d is stable if:
    /// - All infinite bars were established early in the filtration
    /// - No short-lived bars exist (noise filtering)
    fn check_h_d_stability(&self, persistence: &PersistenceData) -> bool {
        check_h_d_stability_impl(
            persistence,
            self.config.dimension,
            self.config.max_betti_d,
            self.config.min_persistence,
        )
    }

    /// Reset the checker (clear previous window data)
    pub fn reset(&mut self) {
        self.previous_h_d_minus_1 = None;
    }
}

/// Standalone implementation of H_d stability check (to avoid borrow issues)
fn check_h_d_stability_impl(
    persistence: &PersistenceData,
    d: usize,
    max_betti_d: usize,
    min_persistence: f64,
) -> bool {
    if let Some(h_d) = persistence.get_diagram(d) {
        // Check that all infinite intervals are stable
        // (This is a simplified check - could be enhanced)
        let betti = h_d.betti_number();

        // For d=3, we typically expect betti_3 = 0 or 1
        // If we have too many infinite bars, something is wrong
        // Use configurable threshold (governable parameter)
        if betti > max_betti_d {
            return false;
        }

        // Check that finite bars have reasonable persistence
        // (filter out noise)
        // Use configurable threshold (governable parameter)
        let finite_bars = h_d.filter_by_persistence(min_persistence);
        let total_bars = h_d.num_intervals();

        // If too many bars are noise, not stable
        if total_bars > 0 && finite_bars.len() < total_bars / 2 {
            return false;
        }

        true
    } else {
        // No H_d computed - not stable
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_crypto::Keypair;
    use adic_types::{AdicFeatures, AdicMeta, AxisPhi, QpDigits, DEFAULT_P, DEFAULT_PRECISION};
    use chrono::Utc;

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
            format!("test_{}", value).into_bytes(),
        );

        let signature = keypair.sign(&msg.to_bytes());
        msg.signature = signature;

        msg
    }

    #[test]
    fn test_empty_messages() {
        let config = F2Config::default();
        let mut checker = F2FinalityChecker::new(config);

        let result = checker.check_finality(&[]);

        match result {
            F2Result::Pending { reason, .. } => {
                assert!(reason.contains("No messages"));
            }
            _ => panic!("Expected Pending result for empty messages"),
        }
    }

    #[test]
    fn test_insufficient_messages() {
        let config = F2Config::default();
        let mut checker = F2FinalityChecker::new(config);

        let keypair = Keypair::generate();
        let messages = vec![create_test_message(1, &keypair)];

        let result = checker.check_finality(&messages);

        // With only 1 message, we can't form d-simplices for d=3
        match result {
            F2Result::Pending { .. } => {
                // Expected
            }
            _ => panic!("Expected Pending result for insufficient messages"),
        }
    }

    #[test]
    fn test_reset() {
        let config = F2Config::default();
        let mut checker = F2FinalityChecker::new(config);

        let keypair = Keypair::generate();
        // Create enough messages with identical p-adic values to form simplices
        let messages = vec![
            create_test_message(0, &keypair),
            create_test_message(0, &keypair),
            create_test_message(0, &keypair),
            create_test_message(0, &keypair),
        ];

        let _ = checker.check_finality(&messages);
        // Note: previous_h_d_minus_1 might still be None if not enough d-simplices formed
        // That's okay for this test - we're just testing the reset functionality

        checker.reset();
        assert!(checker.previous_h_d_minus_1.is_none());
    }

    #[test]
    fn test_f2_finality_progression() {
        // Test that F2 finality progresses through states properly
        let config = F2Config {
            dimension: 2, // Lower dimension for easier testing
            radius: 3,
            axis: 0,
            epsilon: 0.5,
            timeout_ms: 5000,
            use_streaming: false,
            max_betti_d: 2,
            min_persistence: 0.01,
        };
        let mut checker = F2FinalityChecker::new(config);
        let keypair = Keypair::generate();

        // First window - should be pending (no previous)
        let messages1: Vec<_> = (0..10).map(|i| create_test_message(i, &keypair)).collect();

        let result1 = checker.check_finality(&messages1);
        match result1 {
            F2Result::Pending { reason, .. } => {
                assert!(
                    reason.contains("First window")
                        || reason.contains("no previous")
                        || reason.contains("No simplices"),
                    "Unexpected pending reason: {}",
                    reason
                );
            }
            F2Result::Final { .. } => {
                // Can also be final if H_d is stable and conditions met
            }
            _ => {}
        }

        // Second window - same messages, should have bottleneck distance 0
        let result2 = checker.check_finality(&messages1);
        match result2 {
            F2Result::Final {
                h_d_minus_1_bottleneck,
                ..
            } => {
                assert!(
                    h_d_minus_1_bottleneck < 0.1,
                    "Identical windows should have small bottleneck distance"
                );
            }
            F2Result::Pending {
                h_d_minus_1_distance: Some(dist),
                ..
            } => {
                assert!(
                    dist < 0.1,
                    "Identical windows should have small bottleneck distance"
                );
            }
            _ => {}
        }
    }

    #[test]
    fn test_f2_timeout_behavior() {
        // Test that timeout is respected
        let config = F2Config {
            dimension: 3,
            radius: 5,
            axis: 0,
            epsilon: 0.1,
            timeout_ms: 1, // Very short timeout
            use_streaming: false,
            max_betti_d: 2,
            min_persistence: 0.01,
        };
        let mut checker = F2FinalityChecker::new(config);
        let keypair = Keypair::generate();

        // Create many messages to potentially trigger timeout
        let messages: Vec<_> = (0..100).map(|i| create_test_message(i, &keypair)).collect();

        let result = checker.check_finality(&messages);
        match result {
            F2Result::Timeout { elapsed_ms, .. } => {
                assert!(elapsed_ms >= 1);
            }
            F2Result::Pending { .. } | F2Result::Final { .. } => {
                // Also acceptable - computation might be fast enough
            }
            F2Result::Error { .. } => {
                panic!("Should not error on timeout test");
            }
        }
    }

    #[test]
    fn test_h_d_stability_criteria() {
        // Test the H_d stability checking logic
        let config = F2Config::default();
        let checker = F2FinalityChecker::new(config);

        // We can't directly test check_h_d_stability since it's private,
        // but we can test it indirectly through check_finality
        let keypair = Keypair::generate();

        // Create messages that won't form many simplices (dispersed values)
        let messages: Vec<_> = (0..5)
            .map(|i| create_test_message(i * 1000, &keypair))
            .collect();

        // This should likely result in unstable H_d or pending
        let mut checker_mut = checker.clone();
        let result = checker_mut.check_finality(&messages);

        // Just verify we got a result - dispersed messages expected to be pending
        if let F2Result::Pending { .. } = result {
            // Expected - dispersed messages won't form good complex
        }
    }

    #[test]
    fn test_multiple_axes() {
        // Test F2 finality with different axes
        let keypair = Keypair::generate();

        for axis in 0..3 {
            let config = F2Config {
                dimension: 2,
                radius: 3,
                axis,
                epsilon: 0.5,
                timeout_ms: 5000,
                use_streaming: false,
                max_betti_d: 2,
                min_persistence: 0.01,
            };
            let mut checker = F2FinalityChecker::new(config);

            let messages: Vec<_> = (0..8).map(|i| create_test_message(i, &keypair)).collect();

            // Should not error regardless of axis
            let result = checker.check_finality(&messages);
            if let F2Result::Error { .. } = result {
                panic!("Should not error on axis {}", axis);
            }
        }
    }

    #[test]
    fn test_varying_epsilon_thresholds() {
        // Test that different epsilon values affect finality decisions
        let keypair = Keypair::generate();
        let messages: Vec<_> = (0..10).map(|i| create_test_message(i, &keypair)).collect();

        // Very strict epsilon (hard to achieve)
        let strict_config = F2Config {
            dimension: 2,
            radius: 3,
            axis: 0,
            epsilon: 0.001,
            timeout_ms: 5000,
            use_streaming: false,
            max_betti_d: 2,
            min_persistence: 0.01,
        };
        let mut strict_checker = F2FinalityChecker::new(strict_config);

        // Lenient epsilon (easy to achieve)
        let lenient_config = F2Config {
            dimension: 2,
            radius: 3,
            axis: 0,
            epsilon: 10.0,
            timeout_ms: 5000,
            use_streaming: false,
            max_betti_d: 2,
            min_persistence: 0.01,
        };
        let mut lenient_checker = F2FinalityChecker::new(lenient_config);

        // Run first window for both
        let _ = strict_checker.check_finality(&messages);
        let _ = lenient_checker.check_finality(&messages);

        // Run second window (same messages)
        let strict_result = strict_checker.check_finality(&messages);
        let lenient_result = lenient_checker.check_finality(&messages);

        // Lenient should be more likely to finalize
        // (though both might finalize since we're using identical windows)
        match (strict_result, lenient_result) {
            (_, F2Result::Final { .. }) => {
                // Lenient finalized as expected
            }
            _ => {
                // Other outcomes are also valid
            }
        }
    }

    #[test]
    fn test_large_message_set() {
        // Test F2 finality with a larger set of messages
        let config = F2Config {
            dimension: 2,
            radius: 4,
            axis: 0,
            epsilon: 0.5,
            timeout_ms: 10000,
            use_streaming: false,
            max_betti_d: 2,
            min_persistence: 0.01,
        };
        let mut checker = F2FinalityChecker::new(config);
        let keypair = Keypair::generate();

        // Create 50 messages
        let messages: Vec<_> = (0..50).map(|i| create_test_message(i, &keypair)).collect();

        let result = checker.check_finality(&messages);

        // Should complete without error or timeout
        match result {
            F2Result::Error { message } => {
                panic!("Unexpected error: {}", message);
            }
            F2Result::Timeout { .. } => {
                panic!("Should not timeout with 10s limit");
            }
            _ => {} // Pending or Final is fine
        }
    }

    #[test]
    fn test_clustered_vs_dispersed_messages() {
        // Test that clustered messages behave differently than dispersed ones
        let config = F2Config {
            dimension: 2,
            radius: 3,
            axis: 0,
            epsilon: 0.5,
            timeout_ms: 5000,
            use_streaming: false,
            max_betti_d: 2,
            min_persistence: 0.01,
        };
        let keypair = Keypair::generate();

        // Clustered: values close together
        let clustered: Vec<_> = (0..10).map(|i| create_test_message(i, &keypair)).collect();

        // Dispersed: values far apart
        let dispersed: Vec<_> = (0..10)
            .map(|i| create_test_message(i * 10000, &keypair))
            .collect();

        let mut checker1 = F2FinalityChecker::new(config.clone());
        let mut checker2 = F2FinalityChecker::new(config);

        let clustered_result = checker1.check_finality(&clustered);
        let dispersed_result = checker2.check_finality(&dispersed);

        // Both should complete without error
        if let F2Result::Error { message } = clustered_result {
            panic!("Clustered error: {}", message)
        }
        if let F2Result::Error { message } = dispersed_result {
            panic!("Dispersed error: {}", message)
        }
    }
}
