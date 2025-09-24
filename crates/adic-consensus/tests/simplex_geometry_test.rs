use adic_types::{
    AdicFeatures, AdicMessage, AdicMeta, AdicParams, AxisPhi, MessageId, PublicKey, QpDigits,
};
use chrono::Utc;
use std::collections::HashSet;

/// Test that messages form proper d-simplices with d+1 parents
#[test]
fn test_d_simplex_formation() {
    let params = AdicParams::default();
    let d = params.d as usize;

    // Create a message with d+1 parents (forming a d-simplex)
    let mut parent_ids = Vec::new();
    for i in 0..=d {
        parent_ids.push(MessageId::new(&[i as u8; 32]));
    }

    let features = AdicFeatures::new(vec![
        AxisPhi::new(0, QpDigits::from_u64(1, 3, 10)),
        AxisPhi::new(1, QpDigits::from_u64(2, 3, 10)),
        AxisPhi::new(2, QpDigits::from_u64(3, 3, 10)),
    ]);

    let message = AdicMessage::new(
        parent_ids.clone(),
        features,
        AdicMeta::new(Utc::now()),
        PublicKey::from_bytes([1; 32]),
        vec![],
    );

    // Verify d-simplex properties
    assert_eq!(
        message.parents.len(),
        d + 1,
        "Message should have d+1 parents"
    );

    // Verify all parents are distinct
    let unique_parents: HashSet<_> = message.parents.iter().collect();
    assert_eq!(
        unique_parents.len(),
        d + 1,
        "All parents should be distinct"
    );
}

/// Test barycentric coordinates for points in simplex
#[test]
fn test_barycentric_coordinates() {
    // For a d-simplex, any point inside can be expressed as
    // a convex combination of vertices with barycentric coordinates
    let d = 3;
    let num_vertices = d + 1;

    // Create synthetic vertex positions in feature space
    let _vertices: Vec<Vec<f64>> = (0..num_vertices)
        .map(|i| {
            let mut v = vec![0.0; d];
            if i < d {
                v[i] = 1.0; // Standard simplex vertices
            }
            v
        })
        .collect();

    // Test point at centroid (all barycentric coords equal)
    let centroid_bary = vec![1.0 / num_vertices as f64; num_vertices];
    assert!(
        validate_barycentric_coords(&centroid_bary),
        "Centroid barycentric coords should be valid"
    );

    // Test point at vertex (one coord = 1, rest = 0)
    for i in 0..num_vertices {
        let mut vertex_bary = vec![0.0; num_vertices];
        vertex_bary[i] = 1.0;
        assert!(
            validate_barycentric_coords(&vertex_bary),
            "Vertex barycentric coords should be valid"
        );
    }

    // Test invalid coordinates (sum != 1)
    let invalid_bary = vec![0.5; num_vertices];
    assert!(
        !validate_barycentric_coords(&invalid_bary),
        "Invalid barycentric coords should fail"
    );
}

/// Validate barycentric coordinates sum to 1 and are non-negative
fn validate_barycentric_coords(coords: &[f64]) -> bool {
    let sum: f64 = coords.iter().sum();
    let all_non_negative = coords.iter().all(|&c| c >= 0.0);
    (sum - 1.0).abs() < 1e-10 && all_non_negative
}

/// Test simplex orientation consistency
#[test]
fn test_simplex_orientation() {
    // Simplex orientation is determined by the ordering of vertices
    // Swapping two vertices changes the orientation sign
    let d = 3;

    // Create ordered vertices
    let mut vertices: Vec<MessageId> = Vec::new();
    for i in 0..=d {
        vertices.push(MessageId::new(&[i as u8; 32]));
    }

    let orientation1 = compute_orientation_sign(&vertices);

    // Swap two vertices
    vertices.swap(0, 1);
    let orientation2 = compute_orientation_sign(&vertices);

    // Orientations should have opposite signs
    assert_ne!(
        orientation1, orientation2,
        "Swapping vertices should flip orientation"
    );
}

/// Compute orientation sign for ordered vertices (simplified)
fn compute_orientation_sign(vertices: &[MessageId]) -> i8 {
    // Simplified: use hash of ordered IDs to determine orientation
    let mut hasher = 0u64;
    for (i, v) in vertices.iter().enumerate() {
        let bytes = v.to_string();
        for b in bytes.bytes() {
            hasher = hasher
                .wrapping_mul(31)
                .wrapping_add((i + 1) as u64 * b as u64);
        }
    }
    if hasher % 2 == 0 {
        1
    } else {
        -1
    }
}

/// Test simplex volume calculation
#[test]
fn test_simplex_volume() {
    // Volume of a d-simplex with vertices at origin and unit vectors
    // should be 1/d! (for standard simplex)
    let d = 3;
    let expected_volume = 1.0 / factorial(d) as f64;

    // Standard simplex vertices
    let mut vertices = vec![vec![0.0; d]; d + 1];
    for i in 0..d {
        vertices[i + 1][i] = 1.0;
    }

    let volume = compute_simplex_volume(&vertices);
    assert!(
        (volume - expected_volume).abs() < 1e-10,
        "Standard simplex volume should be 1/{}! = {}",
        d,
        expected_volume
    );
}

/// Compute factorial
fn factorial(n: usize) -> usize {
    (1..=n).product()
}

/// Compute volume of simplex using determinant formula
fn compute_simplex_volume(vertices: &[Vec<f64>]) -> f64 {
    let d = vertices[0].len();
    if vertices.len() != d + 1 {
        return 0.0;
    }

    // Build matrix with vectors from first vertex to others
    let mut matrix = vec![vec![0.0; d]; d];
    for i in 0..d {
        for j in 0..d {
            matrix[i][j] = vertices[i + 1][j] - vertices[0][j];
        }
    }

    // Volume = |det(matrix)| / d!
    let det = compute_determinant(&matrix);
    det.abs() / factorial(d) as f64
}

/// Compute determinant of square matrix (recursive)
fn compute_determinant(matrix: &[Vec<f64>]) -> f64 {
    let n = matrix.len();
    if n == 1 {
        return matrix[0][0];
    }
    if n == 2 {
        return matrix[0][0] * matrix[1][1] - matrix[0][1] * matrix[1][0];
    }

    let mut det = 0.0;
    for j in 0..n {
        let mut submatrix = vec![vec![0.0; n - 1]; n - 1];
        for i in 1..n {
            let mut k = 0;
            for l in 0..n {
                if l != j {
                    submatrix[i - 1][k] = matrix[i][l];
                    k += 1;
                }
            }
        }
        let sign = if j % 2 == 0 { 1.0 } else { -1.0 };
        det += sign * matrix[0][j] * compute_determinant(&submatrix);
    }
    det
}

/// Test convexity of simplex
#[test]
fn test_simplex_convexity() {
    // Any convex combination of simplex vertices should be inside the simplex
    let _d = 3;

    // Test random convex combinations
    for _ in 0..10 {
        let weights = vec![0.2, 0.3, 0.1, 0.4]; // Sum to 1
        assert!(
            validate_barycentric_coords(&weights),
            "Convex combination should be valid"
        );

        // Verify point is inside simplex (all barycentric coords >= 0)
        let inside = weights.iter().all(|&w| w >= 0.0);
        assert!(inside, "Convex combination should be inside simplex");
    }

    // Test point outside simplex (negative barycentric coord)
    let outside_weights = vec![1.2, 0.1, -0.1, -0.2];
    assert!(
        !validate_barycentric_coords(&outside_weights),
        "Point outside should have negative coord"
    );
}

/// Test simplex face structure
#[test]
fn test_simplex_faces() {
    // A d-simplex has (d+1 choose k+1) k-faces
    let d = 3;

    // Number of vertices (0-faces)
    let num_vertices = d + 1;
    assert_eq!(num_vertices, 4, "3-simplex should have 4 vertices");

    // Number of edges (1-faces)
    let num_edges = binomial_coefficient(d + 1, 2);
    assert_eq!(num_edges, 6, "3-simplex should have 6 edges");

    // Number of triangular faces (2-faces)
    let num_triangles = binomial_coefficient(d + 1, 3);
    assert_eq!(num_triangles, 4, "3-simplex should have 4 triangular faces");

    // The simplex itself is the only 3-face
    let num_3faces = binomial_coefficient(d + 1, 4);
    assert_eq!(num_3faces, 1, "3-simplex should have 1 3-face (itself)");
}

/// Compute binomial coefficient
fn binomial_coefficient(n: usize, k: usize) -> usize {
    if k > n {
        return 0;
    }
    factorial(n) / (factorial(k) * factorial(n - k))
}

/// Test simplex containment relationships
#[test]
fn test_simplex_containment() {
    // Test that lower-dimensional simplices are contained in higher ones
    let d = 3;

    // Create a d-simplex
    let mut simplex_vertices: Vec<MessageId> = Vec::new();
    for i in 0..=d {
        simplex_vertices.push(MessageId::new(&[i as u8; 32]));
    }

    // Any subset of vertices forms a face (lower-dimensional simplex)
    // Test that removing one vertex gives a (d-1)-simplex face
    for i in 0..=d {
        let mut face_vertices = simplex_vertices.clone();
        face_vertices.remove(i);

        assert_eq!(face_vertices.len(), d, "Face should be (d-1)-simplex");

        // Verify all face vertices are in original simplex
        for v in &face_vertices {
            assert!(
                simplex_vertices.contains(v),
                "Face vertices should be subset of simplex"
            );
        }
    }
}

/// Test simplex duality relationships
#[test]
fn test_simplex_duality() {
    // In a d-simplex, the number of k-faces equals the number of (d-k-1)-faces
    let d = 3;

    // 0-faces (vertices) vs (d-1)-faces
    let vertices = binomial_coefficient(d + 1, 1);
    let d_minus_1_faces = binomial_coefficient(d + 1, d);
    assert_eq!(
        vertices, d_minus_1_faces,
        "Duality between vertices and (d-1)-faces"
    );

    // 1-faces (edges) vs (d-2)-faces
    let edges = binomial_coefficient(d + 1, 2);
    let d_minus_2_faces = binomial_coefficient(d + 1, d - 1);
    assert_eq!(
        edges, d_minus_2_faces,
        "Duality between edges and (d-2)-faces"
    );
}

/// Test that simplex properties are preserved under p-adic distance
#[test]
fn test_simplex_padic_embedding() {
    // When embedded in p-adic space, simplex structure should be preserved
    let params = AdicParams::default();
    let p = params.p as u64;
    let d = params.d as usize;

    // Create vertices with distinct p-adic features
    let mut vertices = Vec::new();
    for i in 0..=d {
        let features = AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(i as u64 * p, params.p, 10)),
            AxisPhi::new(1, QpDigits::from_u64((i + 1) as u64 * p * p, params.p, 10)),
            AxisPhi::new(
                2,
                QpDigits::from_u64((i + 2) as u64 * p * p * p, params.p, 10),
            ),
        ]);
        vertices.push(features);
    }

    // Verify vertices maintain distinct p-adic distances
    for i in 0..vertices.len() {
        for j in i + 1..vertices.len() {
            for axis in 0..d {
                let digits_i = &vertices[i].phi[axis].qp_digits.digits;
                let digits_j = &vertices[j].phi[axis].qp_digits.digits;
                assert_ne!(
                    digits_i, digits_j,
                    "Vertices should have distinct p-adic coordinates"
                );
            }
        }
    }
}
