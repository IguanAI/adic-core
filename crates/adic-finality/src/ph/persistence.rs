use super::reduction::ReductionResult;
use super::simplex::{SimplicialComplex, Weight};
use std::collections::HashMap;
use tracing::{debug, info};

/// A point in a persistence diagram
#[derive(Debug, Clone, PartialEq)]
pub struct PersistenceInterval {
    /// Birth time (when the feature appears)
    pub birth: Weight,
    /// Death time (when the feature disappears), None for infinite persistence
    pub death: Option<Weight>,
    /// Dimension of the homology class (0 for components, 1 for loops, etc.)
    pub dimension: usize,
}

impl PersistenceInterval {
    /// Create a new finite interval
    pub fn finite(birth: Weight, death: Weight, dimension: usize) -> Self {
        Self {
            birth,
            death: Some(death),
            dimension,
        }
    }

    /// Create a new infinite interval
    pub fn infinite(birth: Weight, dimension: usize) -> Self {
        Self {
            birth,
            death: None,
            dimension,
        }
    }

    /// Get the persistence (lifetime) of this interval
    /// Returns None for infinite intervals
    pub fn persistence(&self) -> Option<Weight> {
        self.death.map(|d| d - self.birth)
    }

    /// Check if this interval is infinite
    pub fn is_infinite(&self) -> bool {
        self.death.is_none()
    }

    /// Get the midpoint of the interval for plotting
    /// For infinite intervals, uses a large value
    pub fn midpoint(&self) -> (Weight, Weight) {
        match self.death {
            Some(d) => (self.birth, d),
            None => (self.birth, self.birth + 1000.0), // Arbitrary large value for visualization
        }
    }
}

/// Persistence diagram for a single dimension
#[derive(Debug, Clone)]
pub struct PersistenceDiagram {
    /// Dimension of this diagram
    pub dimension: usize,
    /// All intervals in this diagram
    pub intervals: Vec<PersistenceInterval>,
}

impl PersistenceDiagram {
    /// Create a new empty persistence diagram
    pub fn new(dimension: usize) -> Self {
        Self {
            dimension,
            intervals: Vec::new(),
        }
    }

    /// Add an interval to this diagram
    pub fn add_interval(&mut self, interval: PersistenceInterval) {
        assert_eq!(interval.dimension, self.dimension);
        self.intervals.push(interval);
    }

    /// Get the number of intervals
    pub fn num_intervals(&self) -> usize {
        self.intervals.len()
    }

    /// Get the number of infinite intervals
    pub fn num_infinite(&self) -> usize {
        self.intervals.iter().filter(|i| i.is_infinite()).count()
    }

    /// Get the number of finite intervals
    pub fn num_finite(&self) -> usize {
        self.intervals.iter().filter(|i| !i.is_infinite()).count()
    }

    /// Get the Betti number (number of infinite intervals)
    /// This represents the rank of the homology group
    pub fn betti_number(&self) -> usize {
        self.num_infinite()
    }

    /// Filter intervals by minimum persistence
    /// Useful for removing noise
    pub fn filter_by_persistence(&self, min_persistence: Weight) -> Vec<PersistenceInterval> {
        self.intervals
            .iter()
            .filter(|i| {
                i.persistence()
                    .map(|p| p >= min_persistence)
                    .unwrap_or(true) // Keep infinite intervals
            })
            .cloned()
            .collect()
    }

    /// Get all finite intervals as (birth, death) pairs
    pub fn finite_points(&self) -> Vec<(Weight, Weight)> {
        self.intervals
            .iter()
            .filter_map(|i| i.death.map(|d| (i.birth, d)))
            .collect()
    }
}

/// Complete persistence information across all dimensions
#[derive(Debug, Clone, Default)]
pub struct PersistenceData {
    /// Diagrams for each dimension
    diagrams: HashMap<usize, PersistenceDiagram>,
    /// Maximum dimension
    max_dimension: usize,
}

impl PersistenceData {
    /// Create an empty PersistenceData
    pub fn new() -> Self {
        Self::default()
    }

    /// Compute persistence diagrams from reduction result
    pub fn from_reduction(result: &ReductionResult, complex: &SimplicialComplex) -> Self {
        let mut diagrams: HashMap<usize, PersistenceDiagram> = HashMap::new();
        let mut max_dimension = 0;

        debug!(
            num_pairs = result.pairs.len(),
            num_unpaired = result.unpaired.len(),
            "Computing persistence diagrams from reduction"
        );

        // Process paired simplices (finite intervals)
        for (&death_col, &birth_row) in &result.pairs {
            let death_simplex = complex.get_simplex(death_col).unwrap();
            let birth_simplex = complex.get_simplex(birth_row).unwrap();

            // The dimension of the homology class is birth_simplex.dimension
            // (the feature that was born)
            let dimension = birth_simplex.dimension;
            max_dimension = max_dimension.max(dimension);

            let interval =
                PersistenceInterval::finite(birth_simplex.weight, death_simplex.weight, dimension);

            diagrams
                .entry(dimension)
                .or_insert_with(|| PersistenceDiagram::new(dimension))
                .add_interval(interval);
        }

        // Process unpaired simplices (infinite intervals)
        for &col in &result.unpaired {
            let simplex = complex.get_simplex(col).unwrap();
            let dimension = simplex.dimension;
            max_dimension = max_dimension.max(dimension);

            let interval = PersistenceInterval::infinite(simplex.weight, dimension);

            diagrams
                .entry(dimension)
                .or_insert_with(|| PersistenceDiagram::new(dimension))
                .add_interval(interval);
        }

        let persistence_data = Self {
            diagrams,
            max_dimension,
        };

        // Log Betti numbers
        let betti_numbers = persistence_data.betti_numbers();
        let mut betti_log = String::new();
        for (dim, betti) in betti_numbers.iter().enumerate() {
            if *betti > 0 {
                if !betti_log.is_empty() {
                    betti_log.push_str(", ");
                }
                betti_log.push_str(&format!("β_{} = {}", dim, betti));
            }
        }

        info!(
            max_dimension = max_dimension,
            betti_numbers = betti_log,
            "📈 Computed persistence diagrams"
        );

        persistence_data
    }

    /// Get the diagram for a specific dimension
    pub fn get_diagram(&self, dimension: usize) -> Option<&PersistenceDiagram> {
        self.diagrams.get(&dimension)
    }

    /// Get Betti numbers across all dimensions
    pub fn betti_numbers(&self) -> Vec<usize> {
        (0..=self.max_dimension)
            .map(|d| {
                self.diagrams
                    .get(&d)
                    .map(|diag| diag.betti_number())
                    .unwrap_or(0)
            })
            .collect()
    }

    /// Get the maximum dimension with non-trivial homology
    pub fn max_dimension(&self) -> usize {
        self.max_dimension
    }

    /// Check if homology is stable at a given dimension
    /// A dimension is stable if all finite intervals have died by the given time
    /// and infinite intervals were born before the given time
    pub fn is_stable_at(&self, dimension: usize, time: Weight) -> bool {
        if let Some(diagram) = self.get_diagram(dimension) {
            // Check that all finite intervals are done
            let finite_stable = diagram
                .intervals
                .iter()
                .filter(|i| !i.is_infinite())
                .all(|i| i.death.unwrap() <= time);

            // Check that all infinite intervals were born before this time
            let infinite_stable = diagram
                .intervals
                .iter()
                .filter(|i| i.is_infinite())
                .all(|i| i.birth <= time);

            finite_stable && infinite_stable
        } else {
            true // No homology at this dimension = stable
        }
    }

    /// Get all diagrams
    pub fn all_diagrams(&self) -> &HashMap<usize, PersistenceDiagram> {
        &self.diagrams
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ph::matrix::BoundaryMatrix;
    use crate::ph::reduction::reduce_boundary_matrix;
    use crate::ph::simplex::SimplicialComplex;

    #[test]
    fn test_interval_creation() {
        let finite = PersistenceInterval::finite(1.0, 3.0, 1);
        assert_eq!(finite.persistence(), Some(2.0));
        assert!(!finite.is_infinite());

        let infinite = PersistenceInterval::infinite(1.0, 0);
        assert_eq!(infinite.persistence(), None);
        assert!(infinite.is_infinite());
    }

    #[test]
    fn test_single_point() {
        let mut complex = SimplicialComplex::new();
        complex.add_simplex(vec![0], 0.0);
        complex.sort_by_filtration();

        let matrix = BoundaryMatrix::from_complex(&complex);
        let result = reduce_boundary_matrix(matrix);
        let persistence = PersistenceData::from_reduction(&result, &complex);

        // Should have one infinite H_0 interval
        let h0 = persistence.get_diagram(0).unwrap();
        assert_eq!(h0.num_infinite(), 1);
        assert_eq!(h0.num_finite(), 0);

        let betti = persistence.betti_numbers();
        assert_eq!(betti[0], 1); // One connected component
    }

    #[test]
    fn test_two_points() {
        let mut complex = SimplicialComplex::new();
        complex.add_simplex(vec![0], 0.0);
        complex.add_simplex(vec![1], 0.0);
        complex.sort_by_filtration();

        let matrix = BoundaryMatrix::from_complex(&complex);
        let result = reduce_boundary_matrix(matrix);
        let persistence = PersistenceData::from_reduction(&result, &complex);

        // Should have two infinite H_0 intervals (two components)
        let h0 = persistence.get_diagram(0).unwrap();
        assert_eq!(h0.num_infinite(), 2);
        assert_eq!(h0.betti_number(), 2);
    }

    #[test]
    fn test_edge() {
        let mut complex = SimplicialComplex::new();
        complex.add_simplex_with_faces(vec![0, 1], 1.0);
        complex.sort_by_filtration();

        let matrix = BoundaryMatrix::from_complex(&complex);
        let result = reduce_boundary_matrix(matrix);
        let persistence = PersistenceData::from_reduction(&result, &complex);

        let h0 = persistence.get_diagram(0).unwrap();

        // Should have one infinite interval (the connected component)
        // and one finite interval (the component that merged)
        assert_eq!(h0.num_infinite(), 1);
        assert_eq!(h0.num_finite(), 1);

        // The finite interval should be born at 0.0 and die at 1.0
        let finite = h0.intervals.iter().find(|i| !i.is_infinite()).unwrap();
        assert_eq!(finite.birth, 0.0);
        assert_eq!(finite.death, Some(1.0));
        assert_eq!(finite.persistence(), Some(1.0));

        let betti = persistence.betti_numbers();
        assert_eq!(betti[0], 1); // One connected component
    }

    #[test]
    fn test_triangle_hollow() {
        // Triangle without face - has a 1D hole
        let mut complex = SimplicialComplex::new();
        complex.add_simplex(vec![0], 0.0);
        complex.add_simplex(vec![1], 0.0);
        complex.add_simplex(vec![2], 0.0);
        complex.add_simplex(vec![0, 1], 1.0);
        complex.add_simplex(vec![0, 2], 1.0);
        complex.add_simplex(vec![1, 2], 1.0);
        complex.sort_by_filtration();

        let matrix = BoundaryMatrix::from_complex(&complex);
        let result = reduce_boundary_matrix(matrix);
        let persistence = PersistenceData::from_reduction(&result, &complex);

        // H_0: one infinite (connected component) + 2 finite (merges)
        let h0 = persistence.get_diagram(0).unwrap();
        assert_eq!(h0.betti_number(), 1);

        // H_1: one infinite (the loop)
        let h1 = persistence.get_diagram(1).unwrap();
        assert_eq!(h1.betti_number(), 1);

        let betti = persistence.betti_numbers();
        assert_eq!(betti[0], 1); // One component
        assert_eq!(betti[1], 1); // One loop
    }

    #[test]
    fn test_triangle_filled() {
        // Filled triangle - no holes
        let mut complex = SimplicialComplex::new();
        complex.add_simplex_with_faces(vec![0, 1, 2], 2.0);
        complex.sort_by_filtration();

        let matrix = BoundaryMatrix::from_complex(&complex);
        let result = reduce_boundary_matrix(matrix);
        let persistence = PersistenceData::from_reduction(&result, &complex);

        // H_0: one infinite component
        let h0 = persistence.get_diagram(0).unwrap();
        assert_eq!(h0.betti_number(), 1);

        // H_1: should have one finite interval (loop born and killed)
        let h1 = persistence.get_diagram(1).unwrap();
        assert_eq!(h1.betti_number(), 0); // No infinite loops
        assert_eq!(h1.num_finite(), 1); // One loop that gets filled

        let betti = persistence.betti_numbers();
        assert_eq!(betti[0], 1); // One component
        assert_eq!(betti[1], 0); // No loops (filled)
    }

    #[test]
    fn test_persistence_filtering() {
        let mut diagram = PersistenceDiagram::new(1);
        diagram.add_interval(PersistenceInterval::finite(0.0, 0.1, 1)); // Short-lived
        diagram.add_interval(PersistenceInterval::finite(0.0, 2.0, 1)); // Long-lived
        diagram.add_interval(PersistenceInterval::infinite(0.5, 1)); // Infinite

        // Filter by persistence >= 0.5
        let filtered = diagram.filter_by_persistence(0.5);
        assert_eq!(filtered.len(), 2); // Long-lived + infinite

        // Filter by persistence >= 3.0
        let filtered = diagram.filter_by_persistence(3.0);
        assert_eq!(filtered.len(), 1); // Only infinite
    }

    #[test]
    fn test_stability() {
        let mut complex = SimplicialComplex::new();
        complex.add_simplex_with_faces(vec![0, 1, 2], 2.0);
        complex.sort_by_filtration();

        let matrix = BoundaryMatrix::from_complex(&complex);
        let result = reduce_boundary_matrix(matrix);
        let persistence = PersistenceData::from_reduction(&result, &complex);

        // At time 1.0, vertices are born but triangle not filled yet
        assert!(!persistence.is_stable_at(1, 1.0)); // H_1 not stable (loop exists but not filled)

        // At time 3.0, everything is done
        assert!(persistence.is_stable_at(0, 3.0));
        assert!(persistence.is_stable_at(1, 3.0));
    }
}
