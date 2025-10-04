use super::simplex::{SimplicialComplex, VertexId};
use adic_math::ball_id;
use adic_types::{AdicMessage, MessageId};
use std::collections::HashMap;
use tracing::{debug, info};

/// Build a simplicial complex from ADIC messages based on p-adic ball membership
/// Messages that share a p-adic ball on an axis form simplices together
///
/// This implements the ADIC-DAG paper's construction where:
/// - d-simplices are formed by (d+1) messages sharing a p-adic ball
/// - Weight is based on the maximum timestamp among the vertices
#[derive(Debug)]
pub struct AdicComplexBuilder {
    /// Map from MessageId to vertex index in the complex
    message_to_vertex: HashMap<MessageId, VertexId>,
    /// Reverse map from vertex to MessageId
    vertex_to_message: HashMap<VertexId, MessageId>,
    /// Next available vertex ID
    next_vertex_id: VertexId,
}

impl AdicComplexBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            message_to_vertex: HashMap::new(),
            vertex_to_message: HashMap::new(),
            next_vertex_id: 0,
        }
    }

    /// Get or create a vertex ID for a message
    fn get_or_create_vertex(&mut self, msg_id: MessageId) -> VertexId {
        if let Some(&vertex_id) = self.message_to_vertex.get(&msg_id) {
            return vertex_id;
        }

        let vertex_id = self.next_vertex_id;
        self.next_vertex_id += 1;
        self.message_to_vertex.insert(msg_id, vertex_id);
        self.vertex_to_message.insert(vertex_id, msg_id);
        vertex_id
    }

    /// Build a simplicial complex from messages using p-adic ball membership
    ///
    /// Algorithm:
    /// 1. For each axis and radius, compute ball IDs for all messages
    /// 2. Group messages by ball ID
    /// 3. For each group with k messages, create C(k, d+1) d-simplices
    /// 4. Weight each simplex by the maximum timestamp of its vertices
    pub fn build_from_messages(
        &mut self,
        messages: &[AdicMessage],
        axis: u32,
        radius: usize,
        dimension: usize,
    ) -> SimplicialComplex {
        let mut complex = SimplicialComplex::new();

        if messages.is_empty() {
            debug!("No messages provided for complex construction");
            return complex;
        }

        debug!(
            num_messages = messages.len(),
            axis = axis,
            radius = radius,
            target_dimension = dimension,
            "Building ADIC simplicial complex from messages"
        );

        // Group messages by their ball ID on the given axis
        let mut ball_to_messages: HashMap<Vec<u8>, Vec<(MessageId, f64)>> = HashMap::new();

        for msg in messages {
            // Find the axis features
            if let Some(axis_phi) = msg.features.phi.iter().find(|phi| phi.axis.0 == axis) {
                // Compute ball ID
                let ball = ball_id(&axis_phi.qp_digits, radius);

                // Get timestamp as weight
                let weight = msg.meta.timestamp.timestamp() as f64;

                ball_to_messages
                    .entry(ball)
                    .or_default()
                    .push((msg.id, weight));
            }
        }

        let num_balls = ball_to_messages.len();
        let mut balls_with_simplices = 0;

        // For each ball with enough messages, create simplices
        for (_ball_id, msg_list) in ball_to_messages {
            if msg_list.len() < dimension + 1 {
                continue; // Not enough messages to form a d-simplex
            }

            balls_with_simplices += 1;

            // Create all d-simplices from the messages in this ball
            // For dimension d, we need d+1 vertices
            self.create_simplices_from_ball(&mut complex, &msg_list, dimension);
        }

        // Sort by filtration order
        complex.sort_by_filtration();

        info!(
            num_messages = messages.len(),
            num_balls = num_balls,
            balls_with_d_simplices = balls_with_simplices,
            total_simplices = complex.num_simplices(),
            "🔷 Built ADIC complex via p-adic ball membership"
        );

        complex
    }

    /// Create all d-simplices from messages in the same ball
    fn create_simplices_from_ball(
        &mut self,
        complex: &mut SimplicialComplex,
        messages: &[(MessageId, f64)],
        dimension: usize,
    ) {
        let n = messages.len();
        let d = dimension;

        // We need d+1 vertices for a d-simplex
        if n < d + 1 {
            return;
        }

        // Generate all combinations of (d+1) messages
        let mut indices: Vec<usize> = (0..=d).collect();

        loop {
            // Create a simplex from current combination
            let mut vertices = Vec::new();
            let mut max_weight = 0.0f64;

            for &idx in &indices {
                let (msg_id, weight) = messages[idx];
                let vertex_id = self.get_or_create_vertex(msg_id);
                vertices.push(vertex_id);
                max_weight = max_weight.max(weight);
            }

            // Add the simplex (and its faces automatically)
            complex.add_simplex_with_faces(vertices, max_weight);

            // Generate next combination
            if !next_combination(&mut indices, n) {
                break;
            }
        }
    }

    /// Get the MessageId for a vertex
    pub fn get_message_id(&self, vertex_id: VertexId) -> Option<MessageId> {
        self.vertex_to_message.get(&vertex_id).copied()
    }

    /// Get the vertex ID for a message
    pub fn get_vertex_id(&self, msg_id: &MessageId) -> Option<VertexId> {
        self.message_to_vertex.get(msg_id).copied()
    }
}

impl Default for AdicComplexBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate the next k-combination in lexicographic order
/// Returns false if we've exhausted all combinations
fn next_combination(indices: &mut [usize], n: usize) -> bool {
    let k = indices.len();
    if k == 0 || n == 0 {
        return false;
    }

    // Find the rightmost index that can be incremented
    let mut i = k;
    loop {
        if i == 0 {
            return false; // All combinations exhausted
        }
        i -= 1;

        if indices[i] < n - k + i {
            break;
        }
    }

    // Increment and propagate
    indices[i] += 1;
    for j in i + 1..k {
        indices[j] = indices[j - 1] + 1;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_crypto::Keypair;
    use adic_types::{AdicFeatures, AdicMeta, AxisPhi, QpDigits, DEFAULT_P, DEFAULT_PRECISION};
    use chrono::Utc;

    fn create_test_message(vertices: Vec<u64>, keypair: &Keypair) -> AdicMessage {
        // Create features with axis 0 based on first vertex value
        let features = AdicFeatures::new(vec![AxisPhi::new(
            0,
            QpDigits::from_u64(vertices[0], DEFAULT_P, DEFAULT_PRECISION),
        )]);

        let meta = AdicMeta::new(Utc::now());

        let mut msg = AdicMessage::new(
            vec![],
            features,
            meta,
            *keypair.public_key(),
            format!("test_{:?}", vertices).into_bytes(),
        );

        let signature = keypair.sign(&msg.to_bytes());
        msg.signature = signature;

        msg
    }

    #[test]
    fn test_builder_vertex_mapping() {
        let mut builder = AdicComplexBuilder::new();
        let keypair = Keypair::generate();
        let msg1 = create_test_message(vec![1], &keypair);
        let msg2 = create_test_message(vec![2], &keypair);

        let v1 = builder.get_or_create_vertex(msg1.id);
        let v2 = builder.get_or_create_vertex(msg2.id);

        assert_ne!(v1, v2);
        assert_eq!(builder.get_vertex_id(&msg1.id), Some(v1));
        assert_eq!(builder.get_message_id(v1), Some(msg1.id));

        // Getting same message again should return same vertex
        let v1_again = builder.get_or_create_vertex(msg1.id);
        assert_eq!(v1, v1_again);
    }

    #[test]
    fn test_build_simple_complex() {
        let mut builder = AdicComplexBuilder::new();
        let keypair = Keypair::generate();

        // Create 4 messages with IDENTICAL p-adic values (guaranteed to be in same ball)
        // Using value 0 for all, they will definitely share ball at any radius
        let messages = vec![
            create_test_message(vec![0], &keypair),
            create_test_message(vec![0], &keypair),
            create_test_message(vec![0], &keypair),
            create_test_message(vec![0], &keypair),
        ];

        // For dimension 2, we need 3 vertices per simplex
        // With 4 messages in same ball, we should get C(4,3) = 4 triangles
        let complex = builder.build_from_messages(&messages, 0, 3, 2);

        // Should have created simplices (vertices + edges + triangles + faces)
        assert!(complex.num_simplices() > 0);
        eprintln!("Created {} simplices", complex.num_simplices());
    }

    #[test]
    fn test_next_combination() {
        let mut indices = vec![0, 1, 2];
        let n = 5;

        assert!(next_combination(&mut indices, n));
        assert_eq!(indices, vec![0, 1, 3]);

        assert!(next_combination(&mut indices, n));
        assert_eq!(indices, vec![0, 1, 4]);

        assert!(next_combination(&mut indices, n));
        assert_eq!(indices, vec![0, 2, 3]);
    }

    #[test]
    fn test_empty_messages() {
        let mut builder = AdicComplexBuilder::new();
        let messages: Vec<AdicMessage> = vec![];

        let complex = builder.build_from_messages(&messages, 0, 3, 2);
        assert_eq!(complex.num_simplices(), 0);
    }
}
