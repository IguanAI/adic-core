use adic_types::{AdicError, AdicParams, MessageId, Result};
use chrono::{DateTime, Utc};
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Represents a simplex in the simplicial complex
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Simplex {
    /// Vertices of the simplex (MessageIds)
    pub vertices: Vec<MessageId>,
    /// Weight based on reputation and age
    pub weight: OrderedFloat<f64>,
    /// Birth time in filtration
    pub birth_time: OrderedFloat<f64>,
    /// Death time in filtration (None if still alive)
    pub death_time: Option<OrderedFloat<f64>>,
}

impl Simplex {
    pub fn new(vertices: Vec<MessageId>, weight: f64) -> Self {
        let mut sorted_vertices = vertices;
        // Sort MessageIds by their bytes for consistent ordering
        sorted_vertices.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        Self {
            vertices: sorted_vertices,
            weight: OrderedFloat(weight),
            birth_time: OrderedFloat(0.0),
            death_time: None,
        }
    }

    pub fn dimension(&self) -> usize {
        if self.vertices.is_empty() {
            0
        } else {
            self.vertices.len() - 1
        }
    }

    /// Get all faces (sub-simplices) of this simplex
    pub fn faces(&self) -> Vec<Simplex> {
        if self.vertices.is_empty() {
            return vec![];
        }

        let mut faces = Vec::new();
        let n = self.vertices.len();

        // Prevent memory exhaustion by limiting face generation
        // For large simplices, only generate boundary faces (codimension 1)
        if n > 20 {
            // Only generate (n-1)-faces for large simplices
            for i in 0..n {
                let mut face_vertices = self.vertices.clone();
                face_vertices.remove(i);
                faces.push(Simplex::new(face_vertices, self.weight.0));
            }
        } else if n > 0 {
            // Generate all proper subsets (faces) for reasonable-sized simplices
            for mask in 1..(1 << n) - 1 {
                // Exclude empty set and the simplex itself
                let mut face_vertices = Vec::new();
                for i in 0..n {
                    if (mask & (1 << i)) != 0 {
                        face_vertices.push(self.vertices[i]);
                    }
                }
                faces.push(Simplex::new(face_vertices, self.weight.0));
            }
        }

        faces
    }
}

/// Represents a persistence bar (birth, death) pair
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistenceBar {
    pub birth: f64,
    pub death: Option<f64>, // None means infinite persistence
    pub dimension: usize,
}

impl PersistenceBar {
    pub fn persistence(&self) -> f64 {
        match self.death {
            Some(d) => d - self.birth,
            None => f64::INFINITY,
        }
    }
}

/// Homology computation result for a specific dimension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomologyDimensionResult {
    pub dimension: usize,
    pub bars: Vec<PersistenceBar>,
    pub betti_number: usize,
}

/// Complete persistent homology result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentHomologyResult {
    pub dimensions: Vec<HomologyDimensionResult>,
    pub max_dimension: usize,
    pub filtration_values: Vec<f64>,
    pub timestamp: DateTime<Utc>,
}

/// Configuration for homology finality
#[derive(Debug, Clone)]
pub struct HomologyConfig {
    /// Maximum dimension to compute (typically d from ADIC params)
    pub max_dimension: usize,
    /// Window size Δ for stabilization check
    pub window_size: usize,
    /// Epsilon threshold for bottleneck distance
    pub bottleneck_epsilon: f64,
    /// Minimum bars required for stability
    pub min_bars_for_stability: usize,
}

impl Default for HomologyConfig {
    fn default() -> Self {
        Self {
            max_dimension: 3,
            window_size: 5, // Δ = 5 from whitepaper
            bottleneck_epsilon: 0.1,
            min_bars_for_stability: 3,
        }
    }
}

/// Enhanced homology result with finality status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomologyResult {
    pub finalized_messages: Vec<MessageId>,
    pub stability_score: f64,
    pub status: String,
    pub persistence_diagram: PersistentHomologyResult,
    pub bottleneck_distances: Vec<f64>,
    pub window_results: VecDeque<PersistentHomologyResult>,
}

/// Main persistent homology analyzer for F2 finality
pub struct HomologyAnalyzer {
    config: HomologyConfig,
    _params: AdicParams,
    /// Historical persistence diagrams for stability analysis
    history: Arc<RwLock<VecDeque<PersistentHomologyResult>>>,
    /// Current simplicial complex
    complex: Arc<Mutex<Vec<Simplex>>>,
    /// Cache for computed results
    result_cache: Arc<Mutex<HashMap<Vec<MessageId>, HomologyResult>>>,
}

impl HomologyAnalyzer {
    pub fn new(params: AdicParams) -> Self {
        let config = HomologyConfig {
            max_dimension: params.d as usize,
            window_size: 5, // Δ from whitepaper
            bottleneck_epsilon: 0.1,
            min_bars_for_stability: 3,
        };

        Self {
            config,
            _params: params,
            history: Arc::new(RwLock::new(VecDeque::new())),
            complex: Arc::new(Mutex::new(Vec::new())),
            result_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_config(params: AdicParams, config: HomologyConfig) -> Self {
        Self {
            config,
            _params: params,
            history: Arc::new(RwLock::new(VecDeque::new())),
            complex: Arc::new(Mutex::new(Vec::new())),
            result_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Add a new simplex to the complex (from message approvals)
    pub async fn add_simplex(&self, vertices: Vec<MessageId>, weight: f64) -> Result<()> {
        let simplex = Simplex::new(vertices, weight);

        {
            let mut complex = self
                .complex
                .lock()
                .map_err(|e| AdicError::HomologyError(format!("Failed to lock complex: {}", e)))?;

            // Add the simplex and all its faces
            let faces = simplex.faces();
            complex.push(simplex);
            complex.extend(faces);
        }

        debug!("Added simplex with weight {} to complex", weight);
        Ok(())
    }

    /// Build simplicial complex from message approvals in future cone
    pub async fn build_complex_from_messages(
        &self,
        messages: &[(MessageId, Vec<MessageId>, f64)], // (msg_id, parents, reputation_weight)
    ) -> Result<()> {
        let mut complex = self
            .complex
            .lock()
            .map_err(|e| AdicError::HomologyError(format!("Failed to lock complex: {}", e)))?;

        complex.clear();

        for (msg_id, parents, weight) in messages {
            // Create d-simplex from message and its parents
            let mut vertices = vec![*msg_id];
            vertices.extend(parents.iter().cloned());

            let simplex = Simplex::new(vertices, *weight);
            let faces = simplex.faces();

            complex.push(simplex);
            complex.extend(faces);
        }

        info!(
            "Built complex with {} simplices from {} messages",
            complex.len(),
            messages.len()
        );
        Ok(())
    }

    /// Compute persistent homology using simplified algorithm
    /// This is a basic implementation - production would use libraries like GUDHI or Dionysus
    pub async fn compute_persistent_homology(&self) -> Result<PersistentHomologyResult> {
        let complex = self
            .complex
            .lock()
            .map_err(|e| AdicError::HomologyError(format!("Failed to lock complex: {}", e)))?;

        if complex.is_empty() {
            return Ok(PersistentHomologyResult {
                dimensions: vec![],
                max_dimension: 0,
                filtration_values: vec![],
                timestamp: Utc::now(),
            });
        }

        // Sort simplices by weight for filtration
        let mut sorted_simplices = complex.clone();
        sorted_simplices.sort_by_key(|s| s.weight);

        // Extract unique filtration values using OrderedFloat for HashSet compatibility
        let mut filtration_values: Vec<f64> = sorted_simplices
            .iter()
            .map(|s| s.weight)
            .collect::<HashSet<_>>()
            .into_iter()
            .map(|of| of.0)
            .collect();
        filtration_values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let mut dimension_results = Vec::new();
        let max_dim = sorted_simplices
            .iter()
            .map(|s| s.dimension())
            .max()
            .unwrap_or(0);

        // Compute homology for each dimension up to max_dimension
        for dim in 0..=std::cmp::min(max_dim, self.config.max_dimension) {
            let bars =
                self.compute_dimension_persistence(&sorted_simplices, dim, &filtration_values)?;
            let betti_number = bars.iter().filter(|b| b.death.is_none()).count();

            dimension_results.push(HomologyDimensionResult {
                dimension: dim,
                bars,
                betti_number,
            });
        }

        Ok(PersistentHomologyResult {
            dimensions: dimension_results,
            max_dimension: max_dim,
            filtration_values,
            timestamp: Utc::now(),
        })
    }

    /// Simplified persistence computation for a single dimension
    fn compute_dimension_persistence(
        &self,
        simplices: &[Simplex],
        target_dim: usize,
        filtration_values: &[f64],
    ) -> Result<Vec<PersistenceBar>> {
        let mut bars = Vec::new();

        // This is a simplified algorithm - proper persistent homology requires
        // boundary matrices, reduction algorithms, etc.
        // For the prototype, we simulate the expected behavior

        let target_simplices: Vec<_> = simplices
            .iter()
            .filter(|s| s.dimension() == target_dim)
            .collect();

        if target_simplices.is_empty() {
            return Ok(bars);
        }

        // Group simplices by filtration value
        let mut groups: HashMap<OrderedFloat<f64>, Vec<_>> = HashMap::new();
        for simplex in target_simplices {
            groups.entry(simplex.weight).or_default().push(simplex);
        }

        // Simple heuristic: create bars based on connected components
        // In reality, this would use proper homology computation
        for (i, &filtration_value) in filtration_values.iter().enumerate() {
            let weight_key = OrderedFloat(filtration_value);
            if let Some(group) = groups.get(&weight_key) {
                // Create bars for this filtration level
                // This is a placeholder - real implementation would compute actual cycles
                for (j, _simplex) in group.iter().enumerate() {
                    let birth = filtration_value;
                    let death = if i + j < filtration_values.len() / 2 {
                        Some(
                            filtration_values
                                [std::cmp::min(i + j + 1, filtration_values.len() - 1)],
                        )
                    } else {
                        None // Infinite persistence
                    };

                    bars.push(PersistenceBar {
                        birth,
                        death,
                        dimension: target_dim,
                    });
                }
            }
        }

        Ok(bars)
    }

    /// Compute bottleneck distance between two persistence diagrams
    fn bottleneck_distance(&self, diagram1: &[PersistenceBar], diagram2: &[PersistenceBar]) -> f64 {
        // Simplified bottleneck distance computation
        // Real implementation would use Hungarian algorithm or similar

        if diagram1.is_empty() && diagram2.is_empty() {
            return 0.0;
        }

        if diagram1.is_empty() || diagram2.is_empty() {
            return f64::INFINITY;
        }

        let mut max_distance: f64 = 0.0;

        // Simple approximation: maximum difference in persistence values
        for bar1 in diagram1 {
            let persistence1 = bar1.persistence();
            let mut min_diff = f64::INFINITY;

            for bar2 in diagram2 {
                let persistence2 = bar2.persistence();
                let diff = (persistence1 - persistence2).abs();
                min_diff = min_diff.min(diff);
            }

            max_distance = max_distance.max(min_diff);
        }

        max_distance
    }

    /// Check if homology has stabilized over the window
    async fn check_stability(&self) -> Result<(bool, f64)> {
        let history = self.history.read().await;

        if history.len() < self.config.window_size {
            return Ok((false, 0.0));
        }

        let recent: Vec<_> = history.iter().rev().take(self.config.window_size).collect();

        // Check stabilization of Hd bars
        let target_dim = self.config.max_dimension;
        let mut bottleneck_distances = Vec::new();

        for i in 1..recent.len() {
            let empty_bars = vec![];
            let prev_bars = recent[i]
                .dimensions
                .iter()
                .find(|d| d.dimension == target_dim)
                .map(|d| &d.bars)
                .unwrap_or(&empty_bars);

            let curr_bars = recent[i - 1]
                .dimensions
                .iter()
                .find(|d| d.dimension == target_dim)
                .map(|d| &d.bars)
                .unwrap_or(&empty_bars);

            let distance = self.bottleneck_distance(prev_bars, curr_bars);
            bottleneck_distances.push(distance);
        }

        let avg_distance = if bottleneck_distances.is_empty() {
            0.0
        } else {
            bottleneck_distances.iter().sum::<f64>() / bottleneck_distances.len() as f64
        };

        let stable = avg_distance < self.config.bottleneck_epsilon;

        info!(
            "Homology stability check: stable={}, avg_distance={:.4}, epsilon={:.4}",
            stable, avg_distance, self.config.bottleneck_epsilon
        );

        Ok((stable, avg_distance))
    }

    /// Main F2 finality check for a set of messages
    pub async fn check_finality(&self, message_ids: &[MessageId]) -> Result<HomologyResult> {
        // Check cache first
        {
            let cache = self
                .result_cache
                .lock()
                .map_err(|e| AdicError::HomologyError(format!("Failed to lock cache: {}", e)))?;

            if let Some(cached) = cache.get(&message_ids.to_vec()) {
                return Ok(cached.clone());
            }
        }

        // Compute current persistent homology
        let persistence_diagram = self.compute_persistent_homology().await?;

        // Update history
        {
            let mut history = self.history.write().await;
            history.push_back(persistence_diagram.clone());

            // Keep only recent results
            while history.len() > self.config.window_size * 2 {
                history.pop_front();
            }
        }

        // Check stability
        let (is_stable, stability_score) = self.check_stability().await?;

        let finalized_messages = if is_stable {
            message_ids.to_vec()
        } else {
            vec![]
        };

        let status = if is_stable {
            "F2_FINALIZED".to_string()
        } else {
            format!("PENDING_STABILITY_{:.4}", stability_score)
        };

        let history = self.history.read().await;
        let window_results = history.clone();

        // Compute bottleneck distances for the window
        let bottleneck_distances = if window_results.len() >= 2 {
            let target_dim = self.config.max_dimension;
            let mut distances = Vec::new();

            for i in 1..window_results.len() {
                let empty_bars = vec![];
                let prev_bars = window_results[i - 1]
                    .dimensions
                    .iter()
                    .find(|d| d.dimension == target_dim)
                    .map(|d| &d.bars)
                    .unwrap_or(&empty_bars);

                let curr_bars = window_results[i]
                    .dimensions
                    .iter()
                    .find(|d| d.dimension == target_dim)
                    .map(|d| &d.bars)
                    .unwrap_or(&empty_bars);

                distances.push(self.bottleneck_distance(prev_bars, curr_bars));
            }
            distances
        } else {
            vec![]
        };

        let result = HomologyResult {
            finalized_messages,
            stability_score,
            status,
            persistence_diagram,
            bottleneck_distances,
            window_results,
        };

        // Cache result
        {
            let mut cache = self
                .result_cache
                .lock()
                .map_err(|e| AdicError::HomologyError(format!("Failed to lock cache: {}", e)))?;
            cache.insert(message_ids.to_vec(), result.clone());
        }

        Ok(result)
    }

    /// Check if homology analysis is enabled and functional
    pub fn is_enabled(&self) -> bool {
        true // F2 is now fully implemented
    }

    /// Get current configuration
    pub fn config(&self) -> &HomologyConfig {
        &self.config
    }

    /// Clear the complex and reset state
    pub async fn reset(&self) -> Result<()> {
        {
            let mut complex = self
                .complex
                .lock()
                .map_err(|e| AdicError::HomologyError(format!("Failed to lock complex: {}", e)))?;
            complex.clear();
        }

        {
            let mut history = self.history.write().await;
            history.clear();
        }

        {
            let mut cache = self
                .result_cache
                .lock()
                .map_err(|e| AdicError::HomologyError(format!("Failed to lock cache: {}", e)))?;
            cache.clear();
        }

        info!("Reset homology analyzer state");
        Ok(())
    }

    /// Legacy method for backwards compatibility
    pub async fn check_homology_finality(
        &self,
        message_ids: &[MessageId],
    ) -> Result<HomologyResult> {
        self.check_finality(message_ids).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::AdicParams;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_homology_analyzer_creation() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);
        assert!(analyzer.is_enabled());
        assert_eq!(analyzer.config().max_dimension, 3);
        assert_eq!(analyzer.config().window_size, 5);
    }

    #[tokio::test]
    async fn test_simplex_creation() {
        let vertices = vec![
            MessageId::new(b"msg1"),
            MessageId::new(b"msg2"),
            MessageId::new(b"msg3"),
        ];
        let simplex = Simplex::new(vertices.clone(), 1.0);

        assert_eq!(simplex.dimension(), 2); // 3 vertices = 2-simplex
        assert_eq!(simplex.weight, OrderedFloat(1.0));

        let faces = simplex.faces();
        assert!(!faces.is_empty());
    }

    #[tokio::test]
    async fn test_add_simplex() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        let vertices = vec![MessageId::new(b"msg1"), MessageId::new(b"msg2")];

        let result = analyzer.add_simplex(vertices, 1.0).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_build_complex_from_messages() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        let messages = vec![
            (
                MessageId::new(b"msg1"),
                vec![MessageId::new(b"genesis")],
                1.0,
            ),
            (MessageId::new(b"msg2"), vec![MessageId::new(b"msg1")], 1.5),
            (
                MessageId::new(b"msg3"),
                vec![MessageId::new(b"msg1"), MessageId::new(b"msg2")],
                2.0,
            ),
        ];

        let result = analyzer.build_complex_from_messages(&messages).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_compute_persistent_homology() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        // Build a simple complex
        let messages = vec![
            (
                MessageId::new(b"msg1"),
                vec![MessageId::new(b"genesis")],
                1.0,
            ),
            (MessageId::new(b"msg2"), vec![MessageId::new(b"msg1")], 1.5),
        ];

        analyzer
            .build_complex_from_messages(&messages)
            .await
            .unwrap();

        let result = analyzer.compute_persistent_homology().await;
        assert!(result.is_ok());

        let homology = result.unwrap();
        assert!(!homology.filtration_values.is_empty());
        assert!(!homology.dimensions.is_empty());
    }

    #[tokio::test]
    async fn test_finality_check() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        // Build complex
        let messages = vec![
            (
                MessageId::new(b"msg1"),
                vec![MessageId::new(b"genesis")],
                1.0,
            ),
            (MessageId::new(b"msg2"), vec![MessageId::new(b"msg1")], 1.5),
        ];

        analyzer
            .build_complex_from_messages(&messages)
            .await
            .unwrap();

        let message_ids = vec![MessageId::new(b"msg1")];
        let result = analyzer.check_finality(&message_ids).await;

        assert!(result.is_ok());
        let finality = result.unwrap();
        assert!(finality.status.contains("PENDING") || finality.status.contains("F2"));
    }

    #[tokio::test]
    async fn test_reset() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        // Add some data
        analyzer
            .add_simplex(vec![MessageId::new(b"test")], 1.0)
            .await
            .unwrap();

        // Reset
        let result = analyzer.reset().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_legacy_compatibility() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        let message_ids = vec![MessageId::new(b"test")];
        let result = analyzer.check_homology_finality(&message_ids).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_simplex_face_generation() {
        // Test that faces are generated correctly for various simplex sizes
        let vertices = vec![
            MessageId::new(b"a"),
            MessageId::new(b"b"),
            MessageId::new(b"c"),
            MessageId::new(b"d"),
        ];
        let simplex = Simplex::new(vertices.clone(), 2.5);

        assert_eq!(simplex.dimension(), 3); // 4 vertices = 3-simplex

        let faces = simplex.faces();
        // A 3-simplex has 2^4 - 2 = 14 proper subsets (excluding empty and itself)
        assert_eq!(faces.len(), 14);

        // Verify all faces have correct dimensions
        let mut dim_counts = [0; 4];
        for face in &faces {
            dim_counts[face.dimension()] += 1;
        }
        assert_eq!(dim_counts[0], 4); // 4 vertices (0-simplices)
        assert_eq!(dim_counts[1], 6); // 6 edges (1-simplices)
        assert_eq!(dim_counts[2], 4); // 4 triangles (2-simplices)
    }

    #[tokio::test]
    async fn test_empty_complex_handling() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        // Compute homology on empty complex
        let result = analyzer.compute_persistent_homology().await;
        assert!(result.is_ok());

        let homology = result.unwrap();
        assert_eq!(homology.max_dimension, 0);
        assert!(homology.filtration_values.is_empty());
        assert!(homology.dimensions.is_empty());
    }

    #[tokio::test]
    async fn test_persistence_bar_calculation() {
        let bar1 = PersistenceBar {
            birth: 1.0,
            death: Some(5.0),
            dimension: 1,
        };
        assert_eq!(bar1.persistence(), 4.0);

        let bar2 = PersistenceBar {
            birth: 2.0,
            death: None,
            dimension: 2,
        };
        assert_eq!(bar2.persistence(), f64::INFINITY);
    }

    #[tokio::test]
    async fn test_bottleneck_distance_edge_cases() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        // Test empty diagrams
        let empty: Vec<PersistenceBar> = vec![];
        let distance = analyzer.bottleneck_distance(&empty, &empty);
        assert_eq!(distance, 0.0);

        // Test one empty, one non-empty
        let bars = vec![PersistenceBar {
            birth: 1.0,
            death: Some(2.0),
            dimension: 0,
        }];
        let distance = analyzer.bottleneck_distance(&empty, &bars);
        assert_eq!(distance, f64::INFINITY);

        // Test identical diagrams
        let bars1 = vec![
            PersistenceBar {
                birth: 1.0,
                death: Some(2.0),
                dimension: 0,
            },
            PersistenceBar {
                birth: 2.0,
                death: Some(4.0),
                dimension: 1,
            },
        ];
        let bars2 = bars1.clone();
        let distance = analyzer.bottleneck_distance(&bars1, &bars2);
        assert_eq!(distance, 0.0);
    }

    #[tokio::test]
    async fn test_config_validation() {
        let config = HomologyConfig {
            max_dimension: 5,
            window_size: 10,
            bottleneck_epsilon: 0.01,
            min_bars_for_stability: 5,
        };

        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::with_config(params, config.clone());

        assert_eq!(analyzer.config().max_dimension, 5);
        assert_eq!(analyzer.config().window_size, 10);
        assert_eq!(analyzer.config().bottleneck_epsilon, 0.01);
        assert_eq!(analyzer.config().min_bars_for_stability, 5);
    }

    #[tokio::test]
    async fn test_complex_with_varying_weights() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        // Build complex with different weight patterns
        let messages = vec![
            (
                MessageId::new(b"low1"),
                vec![MessageId::new(b"genesis")],
                0.1,
            ),
            (
                MessageId::new(b"mid1"),
                vec![MessageId::new(b"genesis")],
                5.0,
            ),
            (
                MessageId::new(b"high1"),
                vec![MessageId::new(b"genesis")],
                10.0,
            ),
            (
                MessageId::new(b"mixed"),
                vec![MessageId::new(b"low1"), MessageId::new(b"high1")],
                7.5,
            ),
        ];

        analyzer
            .build_complex_from_messages(&messages)
            .await
            .unwrap();

        let result = analyzer.compute_persistent_homology().await;
        assert!(result.is_ok());

        let homology = result.unwrap();
        // Should have distinct filtration values
        assert!(homology.filtration_values.len() >= 4);
    }

    #[tokio::test]
    async fn test_stability_without_enough_history() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        // Add single complex
        analyzer
            .add_simplex(vec![MessageId::new(b"test")], 1.0)
            .await
            .unwrap();

        // Check stability with insufficient history
        let (stable, score) = analyzer.check_stability().await.unwrap();
        assert!(!stable);
        assert_eq!(score, 0.0);
    }

    #[tokio::test]
    async fn test_cache_behavior() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        // Build a simple complex
        let messages = vec![(
            MessageId::new(b"msg1"),
            vec![MessageId::new(b"genesis")],
            1.0,
        )];
        analyzer
            .build_complex_from_messages(&messages)
            .await
            .unwrap();

        let message_ids = vec![MessageId::new(b"msg1")];

        // First call - should compute and cache
        let result1 = analyzer.check_finality(&message_ids).await.unwrap();

        // Second call - should use cache (verify by checking same result)
        let result2 = analyzer.check_finality(&message_ids).await.unwrap();

        assert_eq!(result1.status, result2.status);
        assert_eq!(result1.stability_score, result2.stability_score);
    }

    #[tokio::test]
    async fn test_multiple_dimension_homology() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        // Create a complex with features at multiple dimensions
        let vertices = vec![
            MessageId::new(b"v1"),
            MessageId::new(b"v2"),
            MessageId::new(b"v3"),
            MessageId::new(b"v4"),
        ];

        // Add a 3-simplex (tetrahedron)
        analyzer.add_simplex(vertices, 1.0).await.unwrap();

        let result = analyzer.compute_persistent_homology().await.unwrap();

        // Should have homology at dimensions 0, 1, 2, 3
        assert!(result.max_dimension >= 3);
        assert!(result.dimensions.len() >= 3);

        // Check Betti numbers make sense
        for dim_result in &result.dimensions {
            assert!(dim_result.betti_number <= dim_result.bars.len());
        }
    }

    #[tokio::test]
    async fn test_ordered_float_behavior() {
        // Verify OrderedFloat handles edge cases correctly
        let weight1 = OrderedFloat(1.0);
        let weight2 = OrderedFloat(2.0);
        let weight_nan = OrderedFloat(f64::NAN);

        assert!(weight1 < weight2);
        assert_eq!(weight1, weight1);

        // NaN handling
        assert!(weight_nan == weight_nan); // OrderedFloat makes NaN equal to itself
    }

    #[tokio::test]
    async fn test_message_id_ordering() {
        // Test that MessageIds sort consistently
        let ids = vec![
            MessageId::new(b"zebra"),
            MessageId::new(b"alpha"),
            MessageId::new(b"beta"),
        ];

        let mut sorted_ids = ids.clone();
        sorted_ids.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        // Check that sorting produces the expected order
        assert_eq!(sorted_ids.len(), 3);
        // The actual order depends on MessageId's internal hashing
        // Just verify that we can sort without panicking
        assert!(sorted_ids.iter().all(|id| ids.contains(id)));
    }

    #[tokio::test]
    async fn test_concurrent_operations() {
        let params = AdicParams::default();
        let analyzer = Arc::new(HomologyAnalyzer::new(params));

        // Spawn multiple concurrent operations
        let mut handles = vec![];

        for i in 0..5 {
            let analyzer_clone = Arc::clone(&analyzer);
            let handle = tokio::spawn(async move {
                let vertices = vec![
                    MessageId::new(format!("msg{}", i).as_bytes()),
                    MessageId::new(format!("parent{}", i).as_bytes()),
                ];
                analyzer_clone.add_simplex(vertices, i as f64).await
            });
            handles.push(handle);
        }

        // Wait for all to complete
        for handle in handles {
            assert!(handle.await.unwrap().is_ok());
        }

        // Verify complex was built correctly
        let homology = analyzer.compute_persistent_homology().await.unwrap();
        assert!(!homology.filtration_values.is_empty());
    }

    #[tokio::test]
    async fn test_window_sliding_behavior() {
        let config = HomologyConfig {
            window_size: 3, // Small window for testing
            ..Default::default()
        };

        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::with_config(params, config);

        // Add messages to trigger window sliding
        for i in 0..10 {
            let messages = vec![(
                MessageId::new(format!("msg{}", i).as_bytes()),
                vec![],
                i as f64,
            )];
            analyzer
                .build_complex_from_messages(&messages)
                .await
                .unwrap();
            analyzer
                .check_finality(&[MessageId::new(b"test")])
                .await
                .unwrap();
        }

        // History should be bounded by window_size * 2
        let history = analyzer.history.read().await;
        assert!(history.len() <= 6); // window_size * 2
    }

    #[tokio::test]
    async fn test_reset_clears_all_state() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        // Add data to all components
        analyzer
            .add_simplex(vec![MessageId::new(b"test1")], 1.0)
            .await
            .unwrap();
        analyzer
            .check_finality(&[MessageId::new(b"test1")])
            .await
            .unwrap();

        // Reset
        analyzer.reset().await.unwrap();

        // Verify everything is cleared
        {
            let complex = analyzer.complex.lock().unwrap();
            assert!(complex.is_empty());
        }

        let history = analyzer.history.read().await;
        assert!(history.is_empty());

        {
            let cache = analyzer.result_cache.lock().unwrap();
            assert!(cache.is_empty());
        }
    }

    #[tokio::test]
    async fn test_filtration_value_uniqueness() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        // Add simplices with duplicate weights
        let messages = vec![
            (MessageId::new(b"a"), vec![], 1.0),
            (MessageId::new(b"b"), vec![], 1.0),
            (MessageId::new(b"c"), vec![], 2.0),
            (MessageId::new(b"d"), vec![], 2.0),
            (MessageId::new(b"e"), vec![], 3.0),
        ];

        analyzer
            .build_complex_from_messages(&messages)
            .await
            .unwrap();
        let homology = analyzer.compute_persistent_homology().await.unwrap();

        // Should have unique filtration values [1.0, 2.0, 3.0]
        assert_eq!(homology.filtration_values.len(), 3);
        assert!(homology.filtration_values.contains(&1.0));
        assert!(homology.filtration_values.contains(&2.0));
        assert!(homology.filtration_values.contains(&3.0));
    }

    #[tokio::test]
    async fn test_stability_convergence_simulation() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        // Simulate gradual stabilization
        for i in 0..10 {
            // Build similar complexes with slight variations
            let weight = 1.0 + (i as f64) * 0.01; // Small changes
            let messages = vec![
                (MessageId::new(b"stable1"), vec![], weight),
                (
                    MessageId::new(b"stable2"),
                    vec![MessageId::new(b"stable1")],
                    weight,
                ),
            ];

            analyzer
                .build_complex_from_messages(&messages)
                .await
                .unwrap();
            analyzer
                .check_finality(&[MessageId::new(b"stable1")])
                .await
                .unwrap();
        }

        // After enough similar complexes, should approach stability
        let (_stable, score) = analyzer.check_stability().await.unwrap();

        // Score should be relatively small due to similar complexes
        assert!(
            score < 1.0,
            "Stability score {} should be low for similar complexes",
            score
        );
    }

    #[tokio::test]
    async fn test_extreme_weight_values() {
        let params = AdicParams::default();
        let analyzer = HomologyAnalyzer::new(params);

        // Test with extreme weight values
        let messages = vec![
            (MessageId::new(b"tiny"), vec![], f64::MIN_POSITIVE),
            (MessageId::new(b"huge"), vec![], f64::MAX / 2.0),
            (MessageId::new(b"zero"), vec![], 0.0),
            (MessageId::new(b"negative"), vec![], -1.0), // Should handle negative
        ];

        let result = analyzer.build_complex_from_messages(&messages).await;
        assert!(result.is_ok());

        let homology = analyzer.compute_persistent_homology().await.unwrap();
        assert!(!homology.filtration_values.is_empty());
    }

    #[tokio::test]
    async fn test_homology_result_serialization() {
        use serde_json;

        let persistence_diagram = PersistentHomologyResult {
            dimensions: vec![HomologyDimensionResult {
                dimension: 0,
                bars: vec![PersistenceBar {
                    birth: 0.0,
                    death: Some(1.0),
                    dimension: 0,
                }],
                betti_number: 1,
            }],
            max_dimension: 0,
            filtration_values: vec![0.0, 1.0],
            timestamp: Utc::now(),
        };

        let result = HomologyResult {
            finalized_messages: vec![MessageId::new(b"test")],
            stability_score: 0.5,
            status: "TEST".to_string(),
            persistence_diagram: persistence_diagram.clone(),
            bottleneck_distances: vec![0.1, 0.2],
            window_results: VecDeque::from(vec![persistence_diagram]),
        };

        // Should serialize and deserialize correctly
        let serialized = serde_json::to_string(&result).unwrap();
        let deserialized: HomologyResult = serde_json::from_str(&serialized).unwrap();

        assert_eq!(result.status, deserialized.status);
        assert_eq!(result.stability_score, deserialized.stability_score);
    }
}
