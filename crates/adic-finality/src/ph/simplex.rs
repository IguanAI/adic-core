use std::collections::HashMap;
use tracing::{debug, info};

/// A simplex index in the complex
pub type SimplexId = usize;

/// Weight associated with a simplex (filtration value)
pub type Weight = f64;

/// A vertex in the simplicial complex
pub type VertexId = usize;

/// Represents a simplex in a simplicial complex
#[derive(Debug, Clone, PartialEq)]
pub struct Simplex {
    /// Unique identifier for this simplex
    pub id: SimplexId,
    /// Vertices that make up this simplex (sorted)
    pub vertices: Vec<VertexId>,
    /// Dimension of the simplex (0 for vertex, 1 for edge, 2 for triangle, etc.)
    pub dimension: usize,
    /// Weight/filtration value (when this simplex appears in the filtration)
    pub weight: Weight,
}

impl Simplex {
    /// Create a new simplex from vertices and weight
    pub fn new(id: SimplexId, mut vertices: Vec<VertexId>, weight: Weight) -> Self {
        vertices.sort_unstable();
        let dimension = if vertices.is_empty() {
            0
        } else {
            vertices.len() - 1
        };

        Self {
            id,
            vertices,
            dimension,
            weight,
        }
    }

    /// Generate all faces (d-1 dimensional boundaries) of this simplex
    /// For example, faces of a triangle [0,1,2] are edges [0,1], [0,2], [1,2]
    pub fn faces(&self) -> Vec<Vec<VertexId>> {
        if self.dimension == 0 {
            return vec![]; // vertices have no faces
        }

        let mut faces = Vec::with_capacity(self.vertices.len());
        for i in 0..self.vertices.len() {
            let mut face = self.vertices.clone();
            face.remove(i);
            faces.push(face);
        }
        faces
    }

    /// Check if this simplex contains another simplex as a face
    pub fn contains_face(&self, face_vertices: &[VertexId]) -> bool {
        face_vertices.iter().all(|v| self.vertices.contains(v))
    }
}

/// A simplicial complex with filtration
pub struct SimplicialComplex {
    /// All simplices in the complex, indexed by SimplexId
    simplices: Vec<Simplex>,
    /// Map from vertex set to simplex ID for fast lookup
    vertex_to_simplex: HashMap<Vec<VertexId>, SimplexId>,
    /// Next available simplex ID
    next_id: SimplexId,
}

impl SimplicialComplex {
    /// Create a new empty simplicial complex
    pub fn new() -> Self {
        Self {
            simplices: Vec::new(),
            vertex_to_simplex: HashMap::new(),
            next_id: 0,
        }
    }

    /// Add a simplex to the complex
    /// Returns the simplex ID (either newly created or existing)
    pub fn add_simplex(&mut self, mut vertices: Vec<VertexId>, weight: Weight) -> SimplexId {
        vertices.sort_unstable();

        // Check if this simplex already exists
        if let Some(&id) = self.vertex_to_simplex.get(&vertices) {
            return id;
        }

        // Create new simplex
        let id = self.next_id;
        self.next_id += 1;

        let simplex = Simplex::new(id, vertices.clone(), weight);
        self.vertex_to_simplex.insert(vertices, id);
        self.simplices.push(simplex);

        id
    }

    /// Add a simplex and all its faces recursively
    /// This ensures the complex is actually a valid simplicial complex
    /// Faces are added with weight 0.0 if not already present (ensuring proper filtration order)
    pub fn add_simplex_with_faces(&mut self, vertices: Vec<VertexId>, weight: Weight) -> SimplexId {
        // First add all faces recursively
        // Faces get weight 0.0 to ensure they appear before the simplex in filtration
        let temp_simplex = Simplex::new(0, vertices.clone(), weight);
        for face_vertices in temp_simplex.faces() {
            self.add_simplex_with_faces(face_vertices, 0.0);
        }

        // Then add the simplex itself
        let simplex_id = self.add_simplex(vertices.clone(), weight);

        debug!(
            simplex_id = simplex_id,
            dimension = temp_simplex.dimension,
            num_vertices = vertices.len(),
            weight = weight,
            "Added {}-simplex to complex",
            temp_simplex.dimension
        );

        simplex_id
    }

    /// Get a simplex by ID
    pub fn get_simplex(&self, id: SimplexId) -> Option<&Simplex> {
        self.simplices.get(id)
    }

    /// Get simplex ID from vertices
    pub fn get_simplex_id(&self, vertices: &[VertexId]) -> Option<SimplexId> {
        self.vertex_to_simplex.get(vertices).copied()
    }

    /// Get all simplices
    pub fn simplices(&self) -> &[Simplex] {
        &self.simplices
    }

    /// Get number of simplices
    pub fn num_simplices(&self) -> usize {
        self.simplices.len()
    }

    /// Append simplices from another complex (for streaming updates)
    /// Assumes new_complex simplices have weights >= existing max weight (monotonic filtration)
    /// Returns Ok(()) if successful, Err if monotonicity violated
    pub fn append_simplices(&mut self, new_complex: SimplicialComplex) -> Result<(), String> {
        // Verify monotonicity: new simplices should have weight >= max existing NON-ZERO weight
        // We allow 0-weight simplices (structural vertices from add_simplex_with_faces) to be added
        let max_existing_weight = self
            .simplices
            .iter()
            .map(|s| s.weight)
            .filter(|w| *w > 0.0)
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(0.0);

        let min_new_nonzero_weight = new_complex
            .simplices
            .iter()
            .map(|s| s.weight)
            .filter(|w| *w > 0.0)
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(f64::INFINITY);

        if min_new_nonzero_weight < max_existing_weight {
            return Err(format!(
                "Monotonicity violated: new simplex weight {} < existing max {}",
                min_new_nonzero_weight, max_existing_weight
            ));
        }

        let initial_count = self.simplices.len();

        // Append new simplices, updating IDs and maps
        for mut simplex in new_complex.simplices {
            // Reassign ID to avoid conflicts
            let new_id = self.next_id;
            self.next_id += 1;
            simplex.id = new_id;

            // Update vertex map
            self.vertex_to_simplex
                .insert(simplex.vertices.clone(), new_id);

            // Append to list
            self.simplices.push(simplex);
        }

        let added = self.simplices.len() - initial_count;
        debug!(
            added_simplices = added,
            total_simplices = self.simplices.len(),
            "📥 Appended simplices to complex"
        );

        Ok(())
    }

    /// Sort simplices by filtration order (weight, then dimension)
    /// This is required for persistent homology computation
    pub fn sort_by_filtration(&mut self) {
        // Compute dimension statistics before sorting
        let mut dim_counts: HashMap<usize, usize> = HashMap::new();
        for simplex in &self.simplices {
            *dim_counts.entry(simplex.dimension).or_insert(0) += 1;
        }

        // Sort by weight first, then by dimension
        self.simplices.sort_by(|a, b| {
            a.weight
                .partial_cmp(&b.weight)
                .unwrap()
                .then(a.dimension.cmp(&b.dimension))
        });

        // Rebuild the vertex_to_simplex map and reassign IDs
        self.vertex_to_simplex.clear();
        for (new_id, simplex) in self.simplices.iter_mut().enumerate() {
            simplex.id = new_id;
            self.vertex_to_simplex
                .insert(simplex.vertices.clone(), new_id);
        }
        self.next_id = self.simplices.len();

        info!(
            total_simplices = self.simplices.len(),
            vertices = dim_counts.get(&0).unwrap_or(&0),
            edges = dim_counts.get(&1).unwrap_or(&0),
            triangles = dim_counts.get(&2).unwrap_or(&0),
            tetrahedra = dim_counts.get(&3).unwrap_or(&0),
            "🔺 Sorted simplicial complex by filtration"
        );
    }

    /// Get faces of a simplex as SimplexIds
    pub fn get_face_ids(&self, simplex_id: SimplexId) -> Vec<SimplexId> {
        if let Some(simplex) = self.get_simplex(simplex_id) {
            let face_vertices = simplex.faces();
            face_vertices
                .iter()
                .filter_map(|fv| self.get_simplex_id(fv))
                .collect()
        } else {
            vec![]
        }
    }

    /// Get all simplices of a specific dimension
    pub fn simplices_by_dimension(&self, dimension: usize) -> Vec<SimplexId> {
        self.simplices
            .iter()
            .filter(|s| s.dimension == dimension)
            .map(|s| s.id)
            .collect()
    }
}

impl Default for SimplicialComplex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simplex_creation() {
        let s = Simplex::new(0, vec![0, 1, 2], 1.0);
        assert_eq!(s.dimension, 2);
        assert_eq!(s.vertices, vec![0, 1, 2]);
        assert_eq!(s.weight, 1.0);
    }

    #[test]
    fn test_simplex_faces() {
        // Triangle has 3 edge faces
        let triangle = Simplex::new(0, vec![0, 1, 2], 1.0);
        let faces = triangle.faces();
        assert_eq!(faces.len(), 3);
        assert!(faces.contains(&vec![0, 1]));
        assert!(faces.contains(&vec![0, 2]));
        assert!(faces.contains(&vec![1, 2]));

        // Edge has 2 vertex faces
        let edge = Simplex::new(1, vec![0, 1], 0.5);
        let faces = edge.faces();
        assert_eq!(faces.len(), 2);
        assert!(faces.contains(&vec![0]));
        assert!(faces.contains(&vec![1]));

        // Vertex has no faces
        let vertex = Simplex::new(2, vec![0], 0.0);
        let faces = vertex.faces();
        assert_eq!(faces.len(), 0);
    }

    #[test]
    fn test_complex_add_simplex() {
        let mut complex = SimplicialComplex::new();

        let id1 = complex.add_simplex(vec![0], 0.0);
        let id2 = complex.add_simplex(vec![1], 0.0);
        let id3 = complex.add_simplex(vec![0, 1], 1.0);

        assert_eq!(complex.num_simplices(), 3);
        assert_ne!(id1, id2);
        assert_ne!(id1, id3);

        // Adding duplicate should return existing ID
        let id_dup = complex.add_simplex(vec![0, 1], 1.0);
        assert_eq!(id_dup, id3);
        assert_eq!(complex.num_simplices(), 3);
    }

    #[test]
    fn test_complex_with_faces() {
        let mut complex = SimplicialComplex::new();

        // Add a triangle - should automatically add edges and vertices
        complex.add_simplex_with_faces(vec![0, 1, 2], 1.0);

        // Should have: 3 vertices, 3 edges, 1 triangle = 7 simplices
        assert_eq!(complex.num_simplices(), 7);

        // Check vertices exist
        assert!(complex.get_simplex_id(&[0]).is_some());
        assert!(complex.get_simplex_id(&[1]).is_some());
        assert!(complex.get_simplex_id(&[2]).is_some());

        // Check edges exist
        assert!(complex.get_simplex_id(&[0, 1]).is_some());
        assert!(complex.get_simplex_id(&[0, 2]).is_some());
        assert!(complex.get_simplex_id(&[1, 2]).is_some());

        // Check triangle exists
        assert!(complex.get_simplex_id(&[0, 1, 2]).is_some());
    }

    #[test]
    fn test_complex_sorting() {
        let mut complex = SimplicialComplex::new();

        // Add simplices in non-filtration order
        complex.add_simplex(vec![0, 1], 2.0); // edge, weight 2.0
        complex.add_simplex(vec![0], 0.0); // vertex, weight 0.0
        complex.add_simplex(vec![1], 0.0); // vertex, weight 0.0
        complex.add_simplex(vec![0, 1, 2], 3.0); // triangle, weight 3.0
        complex.add_simplex(vec![2], 1.0); // vertex, weight 1.0

        complex.sort_by_filtration();

        let simplices = complex.simplices();

        // Check that they're sorted by weight then dimension
        for i in 0..simplices.len() - 1 {
            let curr = &simplices[i];
            let next = &simplices[i + 1];

            assert!(
                curr.weight < next.weight
                    || (curr.weight == next.weight && curr.dimension <= next.dimension)
            );
        }

        // IDs should be reassigned to match sorted order
        for (i, simplex) in simplices.iter().enumerate() {
            assert_eq!(simplex.id, i);
        }
    }

    #[test]
    fn test_face_ids() {
        let mut complex = SimplicialComplex::new();
        complex.add_simplex_with_faces(vec![0, 1, 2], 1.0);
        complex.sort_by_filtration();

        let triangle_id = complex.get_simplex_id(&[0, 1, 2]).unwrap();
        let face_ids = complex.get_face_ids(triangle_id);

        // Triangle should have 3 faces (the edges)
        assert_eq!(face_ids.len(), 3);

        // Check that the faces are actually edges
        for face_id in face_ids {
            let face = complex.get_simplex(face_id).unwrap();
            assert_eq!(face.dimension, 1); // edges are dimension 1
        }
    }
}
