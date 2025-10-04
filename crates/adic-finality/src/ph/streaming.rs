//! Streaming Persistent Homology with Incremental Updates
//!
//! This module implements incremental vineyard-style updates for persistent homology,
//! enabling O(n) amortized complexity per new simplex instead of O(n²-n³) batch recomputation.
//!
//! Key Insight: ADIC messages arrive in monotonic filtration order (newer messages have
//! later timestamps), so we can maintain reduction state and process only NEW simplices.
//!
//! Algorithm:
//! 1. Maintain growing complex + boundary matrix + reduction state
//! 2. When new messages arrive:
//!    a. Append new simplices to complex (maintains sorted order)
//!    b. Append new columns to boundary matrix
//!    c. Reduce ONLY new columns using existing pivot map
//!    d. Update persistence diagrams incrementally
//! 3. Keep bounded history (last Δ snapshots) for bottleneck comparison
//!
//! Complexity: O(k·n) where k = new messages, n = existing simplices
//! Amortized: O(n) per message (vs O(n²-n³) batch)

use super::adic_complex::AdicComplexBuilder;
use super::matrix::BoundaryMatrix;
use super::persistence::{PersistenceData, PersistenceDiagram};
use super::reduction::ReductionResult;
use super::simplex::{SimplexId, SimplicialComplex};
use adic_types::AdicMessage;
use std::collections::{HashMap, VecDeque};
use tracing::{debug, info};

/// Weight type for simplices (filtration values)
pub type Weight = f64;

/// Snapshot of persistence state at a specific round
#[derive(Debug, Clone)]
pub struct PersistenceSnapshot {
    /// Round number when snapshot was taken
    pub round: u64,
    /// Persistence diagrams at this round
    pub persistence: PersistenceData,
    /// Number of simplices in complex at this round
    pub num_simplices: usize,
    /// Timestamp of snapshot
    pub timestamp: i64,
}

/// Streaming persistent homology engine
/// Maintains state between rounds for incremental updates
pub struct StreamingPersistence {
    /// Growing simplicial complex
    complex: SimplicialComplex,

    /// Growing boundary matrix (columns added as simplices added)
    matrix: BoundaryMatrix,

    /// Current reduction result (pairings maintained)
    reduction: ReductionResult,

    /// Current persistence diagrams
    persistence: PersistenceData,

    /// Last simplex index that was reduced
    /// New simplices have indices >= this value
    last_reduced_index: usize,

    /// Rolling window of snapshots for bottleneck distance comparison
    /// Bounded to MAX_SNAPSHOTS to limit memory
    snapshots: VecDeque<PersistenceSnapshot>,

    /// Configuration
    max_snapshots: usize,
    dimension: usize,
    radius: usize,
    axis: usize,

    /// Metrics
    total_incremental_reductions: usize,
    total_xor_operations: usize,
}

impl StreamingPersistence {
    const DEFAULT_MAX_SNAPSHOTS: usize = 10;

    /// Create a new streaming persistence engine
    pub fn new(dimension: usize, radius: usize, axis: usize) -> Self {
        Self {
            complex: SimplicialComplex::new(),
            matrix: BoundaryMatrix::from_complex(&SimplicialComplex::new()),
            reduction: ReductionResult {
                matrix: BoundaryMatrix::from_complex(&SimplicialComplex::new()),
                pairs: HashMap::new(),
                unpaired: Vec::new(),
            },
            persistence: PersistenceData::new(),
            last_reduced_index: 0,
            snapshots: VecDeque::new(),
            max_snapshots: Self::DEFAULT_MAX_SNAPSHOTS,
            dimension,
            radius,
            axis,
            total_incremental_reductions: 0,
            total_xor_operations: 0,
        }
    }

    /// Get current number of simplices
    pub fn num_simplices(&self) -> usize {
        self.complex.num_simplices()
    }

    /// Get current persistence data
    pub fn persistence(&self) -> &PersistenceData {
        &self.persistence
    }

    /// Get a specific persistence diagram
    pub fn get_diagram(&self, dim: usize) -> Option<&PersistenceDiagram> {
        self.persistence.get_diagram(dim)
    }

    /// Get snapshot history
    pub fn snapshots(&self) -> &VecDeque<PersistenceSnapshot> {
        &self.snapshots
    }

    /// Add new messages incrementally
    /// Returns Ok(num_new_simplices) on success
    pub fn add_messages(&mut self, messages: &[AdicMessage], round: u64) -> Result<usize, String> {
        if messages.is_empty() {
            debug!("No new messages to add");
            return Ok(0);
        }

        let start_time = std::time::Instant::now();
        let initial_simplices = self.complex.num_simplices();

        debug!(
            num_messages = messages.len(),
            current_simplices = initial_simplices,
            round = round,
            "🔄 Adding messages to streaming persistence"
        );

        // 1. Build new simplices from messages
        let mut builder = AdicComplexBuilder::new();
        let new_complex =
            builder.build_from_messages(messages, self.axis as u32, self.radius, self.dimension);

        let new_simplices_count = new_complex.num_simplices();
        if new_simplices_count == 0 {
            debug!("No simplices generated from new messages");
            return Ok(0);
        }

        // 2. Append new simplices to existing complex
        self.complex.append_simplices(new_complex)?;

        // 3. Extend boundary matrix with new columns
        let new_columns_start = initial_simplices;
        self.extend_boundary_matrix(new_columns_start)?;

        // 4. Incrementally reduce new columns
        self.incremental_reduce(new_columns_start)?;

        // 5. Update persistence diagrams
        self.update_persistence()?;

        // 6. Take snapshot for this round
        self.take_snapshot(round);

        let elapsed = start_time.elapsed();
        info!(
            round = round,
            new_simplices = new_simplices_count,
            total_simplices = self.complex.num_simplices(),
            elapsed_us = elapsed.as_micros(),
            total_reductions = self.total_incremental_reductions,
            "✅ Streaming update complete"
        );

        Ok(new_simplices_count)
    }

    /// Extend boundary matrix with columns for new simplices
    fn extend_boundary_matrix(&mut self, start_index: usize) -> Result<(), String> {
        let simplices = self.complex.simplices();
        let mut new_columns = Vec::new();

        for simplex in simplices.iter().skip(start_index) {
            let mut faces = self.complex.get_face_ids(simplex.id);
            faces.sort_unstable(); // Maintain sorted order for XOR operations
            new_columns.push(faces);
        }

        debug!(
            num_new_columns = new_columns.len(),
            "📊 Extending boundary matrix"
        );

        self.matrix.append_columns(new_columns);
        Ok(())
    }

    /// Incrementally reduce new columns [start_col..]
    /// Leverages existing pivot_map, only processes new columns
    fn incremental_reduce(&mut self, start_col: usize) -> Result<(), String> {
        let num_columns = self.matrix.num_columns();
        if start_col >= num_columns {
            return Ok(());
        }

        debug!(
            start_col = start_col,
            num_new_cols = num_columns - start_col,
            "⚡ Starting incremental reduction"
        );

        let mut local_xor_count = 0;

        // Reduce only NEW columns
        for col in start_col..num_columns {
            // Standard reduction loop, but pivot_map already contains existing pairs
            while let Some(pivot) = self.matrix.lowest_face(col) {
                match self.matrix.find_column_with_pivot(pivot) {
                    Some(other_col) => {
                        // Eliminate pivot by XORing
                        self.matrix.xor_columns(col, other_col);
                        local_xor_count += 1;
                        self.total_xor_operations += 1;
                    }
                    None => {
                        // Unique pivot - record and done with this column
                        self.matrix.update_pivot(col);
                        self.reduction.pairs.insert(col, pivot);
                        break;
                    }
                }
            }

            // If column became empty after reduction, it's unpaired (infinite persistence)
            if self.matrix.is_empty_column(col) {
                // Check if it's not a death in an existing pair
                if !self.reduction.pairs.contains_key(&col) {
                    self.reduction.unpaired.push(col);
                }
            }

            self.total_incremental_reductions += 1;
        }

        debug!(
            new_pairs = self.reduction.pairs.len(),
            new_unpaired = self.reduction.unpaired.len(),
            xor_ops = local_xor_count,
            "⚡ Incremental reduction complete"
        );

        // Update last reduced index
        self.last_reduced_index = num_columns;

        Ok(())
    }

    /// Update persistence diagrams from current reduction state
    fn update_persistence(&mut self) -> Result<(), String> {
        self.persistence = PersistenceData::from_reduction(&self.reduction, &self.complex);
        Ok(())
    }

    /// Take a snapshot of current state
    fn take_snapshot(&mut self, round: u64) {
        let snapshot = PersistenceSnapshot {
            round,
            persistence: self.persistence.clone(),
            num_simplices: self.complex.num_simplices(),
            timestamp: chrono::Utc::now().timestamp(),
        };

        self.snapshots.push_back(snapshot);

        // Prune old snapshots to maintain bounded memory
        while self.snapshots.len() > self.max_snapshots {
            self.snapshots.pop_front();
        }

        debug!(
            round = round,
            snapshots_stored = self.snapshots.len(),
            "📸 Snapshot taken"
        );
    }

    /// Get snapshot from delta rounds ago
    pub fn get_snapshot_delta_rounds_ago(&self, delta: usize) -> Option<&PersistenceSnapshot> {
        if self.snapshots.len() < delta + 1 {
            return None;
        }

        self.snapshots.get(self.snapshots.len() - delta - 1)
    }

    /// Get metrics about streaming performance
    pub fn metrics(&self) -> StreamingMetrics {
        StreamingMetrics {
            total_simplices: self.complex.num_simplices(),
            total_snapshots: self.snapshots.len(),
            total_incremental_reductions: self.total_incremental_reductions,
            total_xor_operations: self.total_xor_operations,
            memory_estimate_mb: self.estimate_memory_mb(),
        }
    }

    /// Estimate memory usage in MB
    fn estimate_memory_mb(&self) -> f64 {
        let simplex_size = std::mem::size_of::<super::simplex::Simplex>();
        let snapshot_size = std::mem::size_of::<PersistenceSnapshot>();

        let simplex_mem = self.complex.num_simplices() * simplex_size;
        let snapshot_mem = self.snapshots.len() * snapshot_size;
        let matrix_mem = self.matrix.num_columns() * std::mem::size_of::<Vec<SimplexId>>();

        ((simplex_mem + snapshot_mem + matrix_mem) as f64) / (1024.0 * 1024.0)
    }

    /// Reset state (useful for testing or after finality achieved)
    pub fn reset(&mut self) {
        self.complex = SimplicialComplex::new();
        self.matrix = BoundaryMatrix::from_complex(&self.complex);
        self.reduction = ReductionResult {
            matrix: self.matrix.clone(),
            pairs: HashMap::new(),
            unpaired: Vec::new(),
        };
        self.persistence = PersistenceData::new();
        self.last_reduced_index = 0;
        self.snapshots.clear();
        self.total_incremental_reductions = 0;
        self.total_xor_operations = 0;

        info!("🔄 Streaming persistence reset");
    }
}

/// Metrics about streaming performance
#[derive(Debug, Clone)]
pub struct StreamingMetrics {
    pub total_simplices: usize,
    pub total_snapshots: usize,
    pub total_incremental_reductions: usize,
    pub total_xor_operations: usize,
    pub memory_estimate_mb: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_crypto::Keypair;
    use adic_types::{AdicFeatures, AdicMeta, AxisPhi, QpDigits, DEFAULT_P, DEFAULT_PRECISION};

    fn create_test_message(value: u64, keypair: &Keypair) -> AdicMessage {
        let features = AdicFeatures::new(vec![AxisPhi::new(
            0,
            QpDigits::from_u64(value, DEFAULT_P, DEFAULT_PRECISION),
        )]);

        let meta = AdicMeta::new(chrono::Utc::now());

        let mut msg = AdicMessage::new(
            vec![],
            features,
            meta,
            *keypair.public_key(),
            format!("stream_{}", value).into_bytes(),
        );

        let signature = keypair.sign(&msg.to_bytes());
        msg.signature = signature;
        msg
    }

    #[test]
    fn test_streaming_creation() {
        let streaming = StreamingPersistence::new(2, 5, 0);
        assert_eq!(streaming.num_simplices(), 0);
        assert_eq!(streaming.snapshots().len(), 0);
    }

    #[test]
    fn test_incremental_additions() {
        // Use smaller radius to ensure simplices are created
        let mut streaming = StreamingPersistence::new(2, 5, 0);
        let keypair = Keypair::generate();

        // Add first batch - use identical values to ensure they're in same p-adic ball
        // This guarantees simplex creation
        let messages1: Vec<_> = (0..10).map(|_| create_test_message(0, &keypair)).collect();
        let result1 = streaming.add_messages(&messages1, 1);
        assert!(result1.is_ok(), "First batch should succeed");

        let simplices1 = streaming.num_simplices();
        assert!(
            simplices1 > 0,
            "Should create simplices from messages in same ball"
        );

        // Sleep briefly to ensure second batch has later timestamps
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Add second batch - also identical values in same ball
        let messages2: Vec<_> = (0..10).map(|_| create_test_message(0, &keypair)).collect();
        let result2 = streaming.add_messages(&messages2, 2);
        if let Err(e) = &result2 {
            eprintln!("Second batch error: {}", e);
        }
        assert!(
            result2.is_ok(),
            "Second batch should succeed: {:?}",
            result2
        );

        let simplices2 = streaming.num_simplices();
        // Second batch should have more simplices than first
        assert!(simplices2 > simplices1, "Simplices should increase");

        // Should have 2 snapshots
        assert_eq!(streaming.snapshots().len(), 2);
    }

    #[test]
    fn test_snapshot_management() {
        let mut streaming = StreamingPersistence::new(2, 5, 0);
        let keypair = Keypair::generate();

        // Add messages for multiple rounds - use identical values for same ball
        for round in 0..15 {
            // Sleep briefly to ensure monotonic timestamps
            if round > 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }

            let messages: Vec<_> = (0..5).map(|_| create_test_message(0, &keypair)).collect();
            let result = streaming.add_messages(&messages, round);
            assert!(result.is_ok(), "Round {} should succeed", round);
        }

        // Should maintain max 10 snapshots
        assert_eq!(
            streaming.snapshots().len(),
            StreamingPersistence::DEFAULT_MAX_SNAPSHOTS
        );
    }

    #[test]
    fn test_metrics() {
        let mut streaming = StreamingPersistence::new(2, 5, 0);
        let keypair = Keypair::generate();

        // Use identical values to ensure simplices are created
        let messages: Vec<_> = (0..20).map(|_| create_test_message(0, &keypair)).collect();
        let result = streaming.add_messages(&messages, 1);
        assert!(result.is_ok(), "Adding messages should succeed");

        let metrics = streaming.metrics();
        assert_eq!(metrics.total_snapshots, 1);
        assert!(metrics.memory_estimate_mb >= 0.0);
        assert!(metrics.total_simplices > 0, "Should have created simplices");
    }

    #[test]
    fn test_reset() {
        let mut streaming = StreamingPersistence::new(2, 5, 0);
        let keypair = Keypair::generate();

        // Use identical values to ensure simplices are created
        let messages: Vec<_> = (0..10).map(|_| create_test_message(0, &keypair)).collect();
        let result = streaming.add_messages(&messages, 1);
        assert!(result.is_ok(), "Adding messages should succeed");

        let simplices_before = streaming.num_simplices();
        assert!(simplices_before > 0, "Should have simplices before reset");

        streaming.reset();
        assert_eq!(streaming.num_simplices(), 0);
        assert_eq!(streaming.snapshots().len(), 0);
    }
}
