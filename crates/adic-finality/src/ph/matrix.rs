use super::simplex::{SimplexId, SimplicialComplex};
use tracing::{debug, info};

/// A sparse column in the boundary matrix (represented as a sorted list of row indices)
/// Working over F₂ (binary field), we don't need coefficients - just presence/absence
pub type Column = Vec<SimplexId>;

/// Index for a row in the boundary matrix
pub type RowIndex = SimplexId;

/// Index for a column in the boundary matrix
pub type ColIndex = usize;

/// Sparse boundary matrix for persistent homology computation
/// Each column represents a simplex, containing the row indices of its faces
/// Working over F₂, so coefficients are implicit (all 1 mod 2)
#[derive(Clone)]
pub struct BoundaryMatrix {
    /// Sparse columns (each column is a sorted list of face indices)
    columns: Vec<Column>,
    /// Map from pivot row to column index for fast pivot lookup during reduction
    /// Key: pivot row (lowest face in a column), Value: column index
    pivot_map: std::collections::HashMap<RowIndex, ColIndex>,
}

impl BoundaryMatrix {
    /// Create a new boundary matrix from a simplicial complex
    pub fn from_complex(complex: &SimplicialComplex) -> Self {
        let simplices = complex.simplices();
        let n = simplices.len();
        let mut columns = Vec::with_capacity(n);

        for simplex in simplices {
            // Get the faces of this simplex
            let mut faces = complex.get_face_ids(simplex.id);
            // Sort faces for consistency (lowest face will be at end)
            faces.sort_unstable();
            columns.push(faces);
        }

        // Compute sparsity statistics
        let total_entries: usize = columns.iter().map(|c| c.len()).sum();
        let sparsity = if n > 0 {
            1.0 - (total_entries as f64 / (n * n) as f64)
        } else {
            0.0
        };

        info!(
            num_columns = n,
            total_nonzero_entries = total_entries,
            sparsity = format!("{:.4}", sparsity),
            "📊 Created boundary matrix from complex"
        );

        Self {
            columns,
            pivot_map: std::collections::HashMap::new(),
        }
    }

    /// Get the number of columns
    pub fn num_columns(&self) -> usize {
        self.columns.len()
    }

    /// Append new columns to the boundary matrix (for streaming updates)
    /// Used when adding new simplices incrementally
    pub fn append_columns(&mut self, new_columns: Vec<Column>) {
        let initial_count = self.columns.len();
        self.columns.extend(new_columns);

        debug!(
            added_columns = self.columns.len() - initial_count,
            total_columns = self.columns.len(),
            "📊 Appended columns to boundary matrix"
        );
    }

    /// Get a column (immutable)
    pub fn get_column(&self, col: ColIndex) -> Option<&Column> {
        self.columns.get(col)
    }

    /// Check if a column is empty
    pub fn is_empty_column(&self, col: ColIndex) -> bool {
        self.columns.get(col).map_or(true, |c| c.is_empty())
    }

    /// Get the lowest (maximum index) face in a column (the pivot)
    /// Returns None if column is empty
    pub fn lowest_face(&self, col: ColIndex) -> Option<RowIndex> {
        self.columns.get(col).and_then(|c| c.last().copied())
    }

    /// Find which column has a specific pivot row
    pub fn find_column_with_pivot(&self, pivot: RowIndex) -> Option<ColIndex> {
        self.pivot_map.get(&pivot).copied()
    }

    /// XOR two columns (add in F₂)
    /// Computes col_target = col_target XOR col_source
    /// This is the fundamental operation in boundary matrix reduction
    pub fn xor_columns(&mut self, target_col: ColIndex, source_col: ColIndex) {
        if target_col >= self.columns.len() || source_col >= self.columns.len() {
            return;
        }

        let original_len = self.columns[target_col].len();

        // Get source column (we need to clone since we're mutating target)
        let source = self.columns[source_col].clone();
        let target = &mut self.columns[target_col];

        // XOR operation: symmetric difference of two sorted lists
        let mut result = Vec::new();
        let mut i = 0;
        let mut j = 0;

        while i < target.len() && j < source.len() {
            match target[i].cmp(&source[j]) {
                std::cmp::Ordering::Less => {
                    result.push(target[i]);
                    i += 1;
                }
                std::cmp::Ordering::Greater => {
                    result.push(source[j]);
                    j += 1;
                }
                std::cmp::Ordering::Equal => {
                    // Both have this element - cancel out in F₂
                    i += 1;
                    j += 1;
                }
            }
        }

        // Add remaining elements
        result.extend_from_slice(&target[i..]);
        result.extend_from_slice(&source[j..]);

        *target = result;

        debug!(
            target_col = target_col,
            source_col = source_col,
            original_len = original_len,
            result_len = target.len(),
            "XOR columns in boundary matrix"
        );
    }

    /// Update the pivot map after reduction
    /// Should be called after reducing a column
    pub fn update_pivot(&mut self, col: ColIndex) {
        if let Some(pivot) = self.lowest_face(col) {
            self.pivot_map.insert(pivot, col);
        }
    }

    /// Clear the pivot map (used before starting reduction)
    pub fn clear_pivots(&mut self) {
        self.pivot_map.clear();
    }

    /// Get the pivot map (for inspection/debugging)
    pub fn pivot_map(&self) -> &std::collections::HashMap<RowIndex, ColIndex> {
        &self.pivot_map
    }

    /// Get a mutable reference to a column (use with caution)
    pub fn get_column_mut(&mut self, col: ColIndex) -> Option<&mut Column> {
        self.columns.get_mut(col)
    }

    /// Check matrix invariants (for testing)
    #[cfg(test)]
    pub fn verify_invariants(&self) -> bool {
        // Check that all columns are sorted
        for col in &self.columns {
            if !col.windows(2).all(|w| w[0] < w[1]) {
                return false;
            }
        }

        // Check that pivot map is consistent
        for (&pivot, &col) in &self.pivot_map {
            if self.lowest_face(col) != Some(pivot) {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ph::simplex::SimplicialComplex;
    use std::collections::HashSet;

    #[test]
    fn test_matrix_creation() {
        let mut complex = SimplicialComplex::new();
        complex.add_simplex_with_faces(vec![0, 1], 1.0);
        complex.sort_by_filtration();

        let matrix = BoundaryMatrix::from_complex(&complex);

        // Should have 3 columns (2 vertices + 1 edge)
        assert_eq!(matrix.num_columns(), 3);

        // Vertex columns should be empty (no faces)
        assert!(matrix.is_empty_column(0));
        assert!(matrix.is_empty_column(1));

        // Edge column should have 2 faces (the vertices)
        let edge_col = matrix.get_column(2).unwrap();
        assert_eq!(edge_col.len(), 2);
    }

    #[test]
    fn test_lowest_face() {
        let mut complex = SimplicialComplex::new();
        complex.add_simplex_with_faces(vec![0, 1, 2], 1.0);
        complex.sort_by_filtration();

        let matrix = BoundaryMatrix::from_complex(&complex);

        // Vertices have no faces
        assert_eq!(matrix.lowest_face(0), None);
        assert_eq!(matrix.lowest_face(1), None);
        assert_eq!(matrix.lowest_face(2), None);

        // Edges have vertices as faces - lowest should be the highest vertex ID
        let edge_01_id = complex.get_simplex_id(&[0, 1]).unwrap();
        assert!(matrix.lowest_face(edge_01_id).is_some());

        // Triangle has 3 edges as faces
        let triangle_id = complex.get_simplex_id(&[0, 1, 2]).unwrap();
        assert!(matrix.lowest_face(triangle_id).is_some());
    }

    #[test]
    fn test_xor_columns() {
        let mut matrix = BoundaryMatrix {
            columns: vec![
                vec![0, 2, 4], // Column 0
                vec![1, 2, 3], // Column 1
                vec![],        // Column 2 (empty)
            ],
            pivot_map: std::collections::HashMap::new(),
        };

        // XOR columns 0 and 1: {0,2,4} XOR {1,2,3} = {0,1,3,4}
        matrix.xor_columns(0, 1);
        assert_eq!(matrix.get_column(0).unwrap(), &vec![0, 1, 3, 4]);

        // XOR with empty column shouldn't change anything
        matrix.xor_columns(0, 2);
        assert_eq!(matrix.get_column(0).unwrap(), &vec![0, 1, 3, 4]);

        // XOR column with itself should make it empty
        matrix.xor_columns(1, 1);
        assert!(matrix.is_empty_column(1));
    }

    #[test]
    fn test_xor_cancellation() {
        let mut matrix = BoundaryMatrix {
            columns: vec![vec![0, 1, 2], vec![1, 2, 3]],
            pivot_map: std::collections::HashMap::new(),
        };

        // {0,1,2} XOR {1,2,3} = {0,3} (middle elements cancel)
        matrix.xor_columns(0, 1);
        assert_eq!(matrix.get_column(0).unwrap(), &vec![0, 3]);
    }

    #[test]
    fn test_pivot_map() {
        let mut matrix = BoundaryMatrix {
            columns: vec![vec![0, 2], vec![1, 3], vec![2, 4]],
            pivot_map: std::collections::HashMap::new(),
        };

        // Update pivots
        matrix.update_pivot(0);
        matrix.update_pivot(1);
        matrix.update_pivot(2);

        // Check that pivots are tracked correctly
        assert_eq!(matrix.find_column_with_pivot(2), Some(0));
        assert_eq!(matrix.find_column_with_pivot(3), Some(1));
        assert_eq!(matrix.find_column_with_pivot(4), Some(2));
        assert_eq!(matrix.find_column_with_pivot(0), None);
    }

    #[test]
    fn test_triangle_boundary() {
        let mut complex = SimplicialComplex::new();

        // Build a triangle manually
        let v0 = complex.add_simplex(vec![0], 0.0);
        let v1 = complex.add_simplex(vec![1], 0.0);
        let _v2 = complex.add_simplex(vec![2], 0.0);
        let e01 = complex.add_simplex(vec![0, 1], 1.0);
        let e02 = complex.add_simplex(vec![0, 2], 1.0);
        let e12 = complex.add_simplex(vec![1, 2], 1.0);
        let triangle = complex.add_simplex(vec![0, 1, 2], 2.0);

        complex.sort_by_filtration();

        let matrix = BoundaryMatrix::from_complex(&complex);

        // Triangle's boundary should contain its 3 edges
        let triangle_col = matrix.get_column(triangle).unwrap();
        assert_eq!(triangle_col.len(), 3);

        // The edges should be in the column
        let edge_ids: HashSet<_> = [e01, e02, e12].iter().copied().collect();
        let col_ids: HashSet<_> = triangle_col.iter().copied().collect();
        assert_eq!(edge_ids, col_ids);

        // Each edge's boundary should contain 2 vertices
        let e01_col = matrix.get_column(e01).unwrap();
        assert_eq!(e01_col.len(), 2);
        assert!(e01_col.contains(&v0));
        assert!(e01_col.contains(&v1));
    }

    #[test]
    fn test_matrix_invariants() {
        let mut complex = SimplicialComplex::new();
        complex.add_simplex_with_faces(vec![0, 1, 2], 1.0);
        complex.sort_by_filtration();

        let mut matrix = BoundaryMatrix::from_complex(&complex);
        assert!(matrix.verify_invariants());

        // Update some pivots
        matrix.update_pivot(3);
        matrix.update_pivot(4);
        assert!(matrix.verify_invariants());
    }
}
