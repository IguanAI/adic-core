use super::matrix::{BoundaryMatrix, ColIndex, RowIndex};
use super::simplex::SimplicialComplex;
use std::collections::HashMap;
use tracing::{debug, info};

/// Result of boundary matrix reduction
/// Contains information about which simplices pair up (birth-death pairs)
pub struct ReductionResult {
    /// Reduced boundary matrix
    pub matrix: BoundaryMatrix,
    /// Pairing information: maps a death simplex (column with pivot) to its birth simplex (the pivot row)
    /// Key: column index (death), Value: row index (birth)
    pub pairs: HashMap<ColIndex, RowIndex>,
    /// Unpaired simplices (columns with no pivot) - these represent infinite persistence
    pub unpaired: Vec<ColIndex>,
}

/// Perform standard boundary matrix reduction algorithm
/// This implements the standard algorithm from "Computing Persistent Homology" (Zomorodian & Carlsson)
///
/// Algorithm:
/// For each column j from left to right:
///   While column j has a pivot that matches another column's pivot:
///     XOR column j with that column to eliminate the shared pivot
///   Record the final pivot (if any)
///
/// Complexity: O(n^3) worst case, but often much better in practice
pub fn reduce_boundary_matrix(mut matrix: BoundaryMatrix) -> ReductionResult {
    let n = matrix.num_columns();
    matrix.clear_pivots();

    debug!(num_columns = n, "Starting boundary matrix reduction");

    let mut xor_count = 0;

    for col in 0..n {
        // Reduce this column until it has a unique pivot or becomes zero
        while let Some(pivot) = matrix.lowest_face(col) {
            // Check if another column already has this pivot
            match matrix.find_column_with_pivot(pivot) {
                Some(other_col) => {
                    // Eliminate the pivot by XORing with the other column
                    matrix.xor_columns(col, other_col);
                    xor_count += 1;
                }
                None => {
                    // This pivot is unique, record it and move on
                    matrix.update_pivot(col);
                    break;
                }
            }
        }
    }

    // Extract pairing information
    // pivot_map is HashMap<RowIndex, ColIndex> mapping pivot_row -> column
    // We need to invert it to get death -> birth pairs (column -> row)
    let mut pairs = HashMap::new();
    for (&pivot_row, &column) in matrix.pivot_map() {
        pairs.insert(column, pivot_row);
    }

    // Find unpaired simplices (representing infinite persistence)
    // A simplex is unpaired if it appears in neither death nor birth position
    let paired_deaths: std::collections::HashSet<_> = pairs.keys().copied().collect();
    let paired_births: std::collections::HashSet<_> = pairs.values().copied().collect();
    let unpaired: Vec<ColIndex> = (0..n)
        .filter(|&col| !paired_deaths.contains(&col) && !paired_births.contains(&col))
        .collect();

    info!(
        num_pairs = pairs.len(),
        num_unpaired = unpaired.len(),
        xor_operations = xor_count,
        "⚡ Completed boundary matrix reduction"
    );

    ReductionResult {
        matrix,
        pairs,
        unpaired,
    }
}

/// Optimized reduction with clearing optimization
/// After reducing columns of dimension d, we mark their pivot rows and skip those rows
/// This dramatically reduces operations for higher-dimensional features
pub fn reduce_with_clearing(
    mut matrix: BoundaryMatrix,
    _complex: &SimplicialComplex,
) -> ReductionResult {
    let n = matrix.num_columns();
    matrix.clear_pivots();

    debug!(
        num_columns = n,
        "Starting boundary matrix reduction with clearing optimization"
    );

    // Track which rows have been used as pivots (these can be "cleared")
    let mut cleared_rows = vec![false; n];
    let mut cleared_count = 0;

    for col in 0..n {
        // Check if this column's faces have been cleared
        // If so, we can skip some operations
        if let Some(column_data) = matrix.get_column(col) {
            // Remove cleared rows from the column
            let filtered: Vec<_> = column_data
                .iter()
                .copied()
                .filter(|&row| !cleared_rows[row])
                .collect();

            if filtered.len() != column_data.len() {
                cleared_count += column_data.len() - filtered.len();
                *matrix.get_column_mut(col).unwrap() = filtered;
            }
        }

        // Reduce this column
        while let Some(pivot) = matrix.lowest_face(col) {
            match matrix.find_column_with_pivot(pivot) {
                Some(other_col) => {
                    matrix.xor_columns(col, other_col);
                }
                None => {
                    matrix.update_pivot(col);
                    cleared_rows[pivot] = true; // Mark this row as cleared
                    break;
                }
            }
        }
    }

    // Extract pairing information
    let mut pairs = HashMap::new();
    for (&pivot_row, &column) in matrix.pivot_map() {
        pairs.insert(column, pivot_row);
    }
    let paired_deaths: std::collections::HashSet<_> = pairs.keys().copied().collect();
    let paired_births: std::collections::HashSet<_> = pairs.values().copied().collect();
    let unpaired: Vec<ColIndex> = (0..n)
        .filter(|&col| !paired_deaths.contains(&col) && !paired_births.contains(&col))
        .collect();

    info!(
        num_pairs = pairs.len(),
        num_unpaired = unpaired.len(),
        cleared_entries = cleared_count,
        "⚡ Completed reduction with clearing optimization"
    );

    ReductionResult {
        matrix,
        pairs,
        unpaired,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ph::simplex::SimplicialComplex;

    #[test]
    fn test_reduction_single_point() {
        // Single vertex - should have one unpaired 0-simplex (infinite H_0)
        let mut complex = SimplicialComplex::new();
        complex.add_simplex(vec![0], 0.0);
        complex.sort_by_filtration();

        let matrix = BoundaryMatrix::from_complex(&complex);
        let result = reduce_boundary_matrix(matrix);

        // Single point has no pairs (it's unpaired)
        assert_eq!(result.pairs.len(), 0);
        assert_eq!(result.unpaired.len(), 1);
    }

    #[test]
    fn test_reduction_two_points() {
        // Two disconnected vertices - should have two unpaired 0-simplices
        let mut complex = SimplicialComplex::new();
        complex.add_simplex(vec![0], 0.0);
        complex.add_simplex(vec![1], 0.0);
        complex.sort_by_filtration();

        let matrix = BoundaryMatrix::from_complex(&complex);
        let result = reduce_boundary_matrix(matrix);

        // Two points, no edges - both unpaired
        assert_eq!(result.pairs.len(), 0);
        assert_eq!(result.unpaired.len(), 2);
    }

    #[test]
    fn test_reduction_edge() {
        // Two vertices connected by an edge
        let mut complex = SimplicialComplex::new();
        complex.add_simplex_with_faces(vec![0, 1], 1.0);
        complex.sort_by_filtration();

        let matrix = BoundaryMatrix::from_complex(&complex);
        let result = reduce_boundary_matrix(matrix);

        // Edge should pair with one vertex (killing one H_0 component)
        // One vertex remains unpaired (the surviving H_0 component)
        assert_eq!(result.pairs.len(), 1);
        assert_eq!(result.unpaired.len(), 1);

        // The unpaired one should be a 0-simplex
        let unpaired_id = result.unpaired[0];
        let unpaired_simplex = complex.get_simplex(unpaired_id).unwrap();
        assert_eq!(unpaired_simplex.dimension, 0);
    }

    #[test]
    fn test_reduction_triangle_hollow() {
        // Triangle without the face (3 vertices, 3 edges, no 2-simplex)
        // This has a 1-dimensional hole
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

        // 3 vertices, 3 edges
        // Two edges pair with vertices (reduce to 1 connected component)
        // One edge remains unpaired (the 1D loop)
        // One vertex remains unpaired (the H_0 component)
        assert_eq!(result.unpaired.len(), 2);

        // Check dimensions of unpaired simplices
        let mut unpaired_dims: Vec<_> = result
            .unpaired
            .iter()
            .map(|&id| complex.get_simplex(id).unwrap().dimension)
            .collect();
        unpaired_dims.sort();

        // Should have one 0-simplex (H_0) and one 1-simplex (H_1)
        assert_eq!(unpaired_dims, vec![0, 1]);
    }

    #[test]
    fn test_reduction_triangle_filled() {
        // Filled triangle (3 vertices, 3 edges, 1 face)
        // No holes - only one connected component
        let mut complex = SimplicialComplex::new();
        complex.add_simplex_with_faces(vec![0, 1, 2], 2.0);
        complex.sort_by_filtration();

        let matrix = BoundaryMatrix::from_complex(&complex);
        let result = reduce_boundary_matrix(matrix);

        // The 2-simplex (triangle face) should pair with one edge (killing the H_1 loop)
        // Two edges pair with vertices (reducing H_0 to 1 component)
        // One vertex remains unpaired (the H_0 component)
        assert_eq!(result.unpaired.len(), 1);

        let unpaired_id = result.unpaired[0];
        let unpaired_simplex = complex.get_simplex(unpaired_id).unwrap();

        // The unpaired simplex should be a vertex (H_0 generator)
        assert_eq!(unpaired_simplex.dimension, 0);
    }

    #[test]
    fn test_reduction_tetrahedron() {
        // Filled tetrahedron (4 vertices, 6 edges, 4 faces, 1 volume)
        // Topologically a 3-ball - no holes
        let mut complex = SimplicialComplex::new();
        complex.add_simplex_with_faces(vec![0, 1, 2, 3], 3.0);
        complex.sort_by_filtration();

        let matrix = BoundaryMatrix::from_complex(&complex);
        let result = reduce_boundary_matrix(matrix);

        // Everything pairs up except one vertex (the H_0 generator)
        assert_eq!(result.unpaired.len(), 1);

        let unpaired_id = result.unpaired[0];
        let unpaired_simplex = complex.get_simplex(unpaired_id).unwrap();
        assert_eq!(unpaired_simplex.dimension, 0);
    }

    #[test]
    fn test_reduction_with_clearing() {
        // Test that clearing optimization produces same result
        let mut complex = SimplicialComplex::new();
        complex.add_simplex_with_faces(vec![0, 1, 2], 2.0);
        complex.sort_by_filtration();

        let matrix1 = BoundaryMatrix::from_complex(&complex);
        let result1 = reduce_boundary_matrix(matrix1);

        let matrix2 = BoundaryMatrix::from_complex(&complex);
        let result2 = reduce_with_clearing(matrix2, &complex);

        // Results should be identical
        assert_eq!(result1.pairs.len(), result2.pairs.len());
        assert_eq!(result1.unpaired.len(), result2.unpaired.len());
    }

    #[test]
    fn test_pairing_structure() {
        // Verify that pairs map death -> birth correctly
        let mut complex = SimplicialComplex::new();
        complex.add_simplex_with_faces(vec![0, 1], 1.0);
        complex.sort_by_filtration();

        let matrix = BoundaryMatrix::from_complex(&complex);
        let result = reduce_boundary_matrix(matrix);

        // The edge (col 2) should pair with one of the vertices (row 0 or 1)
        assert_eq!(result.pairs.len(), 1);

        for (&death_col, &birth_row) in &result.pairs {
            let death_simplex = complex.get_simplex(death_col).unwrap();
            let birth_simplex = complex.get_simplex(birth_row).unwrap();

            // Death should be higher dimension than birth
            assert!(death_simplex.dimension > birth_simplex.dimension);

            // Death should appear later in filtration
            assert!(death_simplex.weight >= birth_simplex.weight);
        }
    }
}
