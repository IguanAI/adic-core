use std::collections::HashMap;
use std::sync::Arc;

use libp2p::PeerId;
use tokio::sync::RwLock;
use tracing::{debug, info};

use adic_types::{AxisId, AxisPhi, QpDigits, Result};

/// Type alias for p-adic ball identifier
/// A ball is identified by truncating the p-adic expansion to a certain precision
pub type BallId = String;

/// Type alias for the peers-by-ball map structure
type PeersByBallMap = Arc<RwLock<HashMap<(AxisId, BallId), Vec<PeerId>>>>;

/// Axis-aware overlay structure for p-adic gossip routing
/// Per ADIC-DAG White Paper §4 and Yellow Paper §4
///
/// Maintains peer assignments to p-adic balls across all axes,
/// enabling q-diverse peer selection for robust gossip propagation.
#[derive(Clone)]
pub struct AxisOverlay {
    /// Map (axis_id, ball_id) -> peers in that ball
    peers_by_ball: PeersByBallMap,

    /// Map peer_id -> their p-adic features (coordinates in each axis)
    peer_features: Arc<RwLock<HashMap<PeerId, Vec<AxisPhi>>>>,

    /// p-adic prime (typically 3 per ADIC-DAG paper)
    p: u32,

    /// Ball radius (precision for ball assignment)
    /// Smaller radius = finer-grained balls, more diverse selection
    ball_radius: usize,
}

impl AxisOverlay {
    /// Create a new axis overlay with specified parameters
    ///
    /// # Arguments
    /// * `p` - The p-adic prime (typically 3)
    /// * `ball_radius` - Precision for ball assignment (typically 2-4)
    pub fn new(p: u32, ball_radius: usize) -> Self {
        Self {
            peers_by_ball: Arc::new(RwLock::new(HashMap::new())),
            peer_features: Arc::new(RwLock::new(HashMap::new())),
            p,
            ball_radius,
        }
    }

    /// Assign a peer to p-adic balls based on their feature coordinates
    ///
    /// Peers are assigned to one ball per axis, determined by truncating
    /// their p-adic coordinate to the ball_radius precision.
    pub async fn assign_peer(&self, peer_id: PeerId, features: Vec<AxisPhi>) -> Result<()> {
        let mut peers_by_ball = self.peers_by_ball.write().await;
        let mut peer_features = self.peer_features.write().await;

        // Remove peer from old balls if they were previously assigned
        if let Some(old_features) = peer_features.get(&peer_id) {
            for axis_phi in old_features {
                let ball_id = self.compute_ball_id(&axis_phi.qp_digits, self.ball_radius);
                let key = (axis_phi.axis, ball_id.clone());

                if let Some(peers) = peers_by_ball.get_mut(&key) {
                    peers.retain(|p| p != &peer_id);
                    if peers.is_empty() {
                        peers_by_ball.remove(&key);
                    }
                }
            }
        }

        // Assign peer to new balls based on their features
        for axis_phi in &features {
            let ball_id = self.compute_ball_id(&axis_phi.qp_digits, self.ball_radius);
            let key = (axis_phi.axis, ball_id.clone());

            peers_by_ball
                .entry(key.clone())
                .or_insert_with(Vec::new)
                .push(peer_id);

            debug!(
                peer_id = %peer_id,
                axis = axis_phi.axis.0,
                ball_id = %ball_id,
                "Assigned peer to p-adic ball"
            );
        }

        // Update peer features mapping
        peer_features.insert(peer_id, features.clone());

        info!(
            peer_id = %peer_id,
            num_axes = features.len(),
            ball_radius = self.ball_radius,
            "Updated peer assignment in axis overlay"
        );

        Ok(())
    }

    /// Remove a peer from the overlay
    pub async fn remove_peer(&self, peer_id: &PeerId) -> Result<()> {
        let mut peers_by_ball = self.peers_by_ball.write().await;
        let mut peer_features = self.peer_features.write().await;

        // Remove from all balls
        if let Some(features) = peer_features.remove(peer_id) {
            for axis_phi in features {
                let ball_id = self.compute_ball_id(&axis_phi.qp_digits, self.ball_radius);
                let key = (axis_phi.axis, ball_id);

                if let Some(peers) = peers_by_ball.get_mut(&key) {
                    peers.retain(|p| p != peer_id);
                    if peers.is_empty() {
                        peers_by_ball.remove(&key);
                    }
                }
            }

            info!(peer_id = %peer_id, "Removed peer from axis overlay");
        }

        Ok(())
    }

    /// Select q-diverse peers across axes for gossip routing
    ///
    /// This implements diverse peer selection per ADIC-DAG paper §4.3:
    /// - For each axis, select peers from different p-adic balls
    /// - Ensures message propagation across diverse regions of the space
    ///
    /// # Arguments
    /// * `num_peers` - Total number of peers to select
    /// * `target_features` - The message's p-adic features (for proximity selection)
    ///
    /// # Returns
    /// A list of diverse peer IDs for gossip routing
    pub async fn select_diverse_peers(
        &self,
        num_peers: usize,
        target_features: &[AxisPhi],
    ) -> Vec<PeerId> {
        let peers_by_ball = self.peers_by_ball.read().await;
        let mut selected_peers = Vec::new();
        let mut used_peers = std::collections::HashSet::new();

        // Strategy: Round-robin across axes, selecting from different balls
        let num_axes = target_features.len();
        if num_axes == 0 {
            return selected_peers;
        }

        // Future optimization: Could enforce balanced peer selection across axes
        // by limiting to (num_peers + num_axes - 1) / num_axes peers per axis

        for axis_phi in target_features {
            // For this axis, find peers in nearby and distant balls
            let target_ball = self.compute_ball_id(&axis_phi.qp_digits, self.ball_radius);

            // First, try to select from the same ball (local propagation)
            let same_ball_key = (axis_phi.axis, target_ball.clone());
            if let Some(peers) = peers_by_ball.get(&same_ball_key) {
                for peer in peers {
                    if !used_peers.contains(peer) && selected_peers.len() < num_peers {
                        selected_peers.push(*peer);
                        used_peers.insert(*peer);
                    }
                }
            }

            // Then, select from different balls for diversity
            let axis_balls: Vec<_> = peers_by_ball
                .keys()
                .filter(|(axis, ball_id)| *axis == axis_phi.axis && *ball_id != target_ball)
                .collect();

            for (axis, ball_id) in axis_balls {
                if selected_peers.len() >= num_peers {
                    break;
                }

                if let Some(peers) = peers_by_ball.get(&(*axis, ball_id.clone())) {
                    for peer in peers {
                        if !used_peers.contains(peer) && selected_peers.len() < num_peers {
                            selected_peers.push(*peer);
                            used_peers.insert(*peer);
                        }
                    }
                }
            }
        }

        info!(
            selected_count = selected_peers.len(),
            requested_count = num_peers,
            num_axes = num_axes,
            "Selected diverse peers for gossip"
        );

        selected_peers
    }

    /// Get peers in a specific p-adic ball
    pub async fn peers_in_ball(&self, axis: AxisId, ball_id: &str) -> Vec<PeerId> {
        let peers_by_ball = self.peers_by_ball.read().await;
        peers_by_ball
            .get(&(axis, ball_id.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    /// Get the p-adic features for a peer
    pub async fn peer_features(&self, peer_id: &PeerId) -> Option<Vec<AxisPhi>> {
        let peer_features = self.peer_features.read().await;
        peer_features.get(peer_id).cloned()
    }

    /// Get all peers in the overlay
    pub async fn all_peers(&self) -> Vec<PeerId> {
        let peer_features = self.peer_features.read().await;
        peer_features.keys().copied().collect()
    }

    /// Compute ball ID by truncating p-adic expansion to specified precision
    ///
    /// The ball ID is a string representation of the first `radius` p-adic digits
    fn compute_ball_id(&self, phi: &QpDigits, radius: usize) -> BallId {
        let truncated_digits: Vec<u8> = phi.digits.iter().take(radius).copied().collect();

        // Format as string: "d0_d1_d2_..." for easy comparison
        truncated_digits
            .iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join("_")
    }

    /// Get statistics about the overlay for debugging
    pub async fn stats(&self) -> OverlayStats {
        let peers_by_ball = self.peers_by_ball.read().await;
        let peer_features = self.peer_features.read().await;

        let total_peers = peer_features.len();
        let total_balls = peers_by_ball.len();

        let balls_per_axis: HashMap<u32, usize> =
            peers_by_ball
                .keys()
                .fold(HashMap::new(), |mut acc, (axis, _)| {
                    *acc.entry(axis.0).or_insert(0) += 1;
                    acc
                });

        OverlayStats {
            total_peers,
            total_balls,
            balls_per_axis,
            p: self.p,
            ball_radius: self.ball_radius,
        }
    }
}

/// Statistics about the axis overlay
#[derive(Debug, Clone)]
pub struct OverlayStats {
    pub total_peers: usize,
    pub total_balls: usize,
    pub balls_per_axis: HashMap<u32, usize>,
    pub p: u32,
    pub ball_radius: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_peer_id(_byte: u8) -> PeerId {
        let keypair = libp2p::identity::Keypair::generate_ed25519();
        keypair.public().to_peer_id()
    }

    #[tokio::test]
    async fn test_axis_overlay_creation() {
        let overlay = AxisOverlay::new(3, 3);
        let stats = overlay.stats().await;

        assert_eq!(stats.total_peers, 0);
        assert_eq!(stats.total_balls, 0);
        assert_eq!(stats.p, 3);
        assert_eq!(stats.ball_radius, 3);
    }

    #[tokio::test]
    async fn test_peer_assignment() {
        let overlay = AxisOverlay::new(3, 3);
        let peer_id = create_test_peer_id(1);

        // Create features for 2 axes
        let features = vec![
            AxisPhi::new(0, QpDigits::from_u64(42, 3, 10)),
            AxisPhi::new(1, QpDigits::from_u64(100, 3, 10)),
        ];

        overlay
            .assign_peer(peer_id, features.clone())
            .await
            .unwrap();

        // Verify peer was assigned
        let all_peers = overlay.all_peers().await;
        assert_eq!(all_peers.len(), 1);
        assert!(all_peers.contains(&peer_id));

        // Verify features were stored
        let stored_features = overlay.peer_features(&peer_id).await.unwrap();
        assert_eq!(stored_features.len(), 2);
    }

    #[tokio::test]
    async fn test_peer_removal() {
        let overlay = AxisOverlay::new(3, 3);
        let peer_id = create_test_peer_id(1);

        let features = vec![AxisPhi::new(0, QpDigits::from_u64(42, 3, 10))];

        overlay.assign_peer(peer_id, features).await.unwrap();
        assert_eq!(overlay.all_peers().await.len(), 1);

        overlay.remove_peer(&peer_id).await.unwrap();
        assert_eq!(overlay.all_peers().await.len(), 0);
    }

    #[tokio::test]
    async fn test_diverse_peer_selection() {
        let overlay = AxisOverlay::new(3, 2);

        // Add peers with different features
        for i in 0..10 {
            let peer_id = create_test_peer_id(i);
            let features = vec![
                AxisPhi::new(0, QpDigits::from_u64(i as u64 * 10, 3, 10)),
                AxisPhi::new(1, QpDigits::from_u64(i as u64 * 20, 3, 10)),
            ];
            overlay.assign_peer(peer_id, features).await.unwrap();
        }

        // Select diverse peers
        let target_features = vec![
            AxisPhi::new(0, QpDigits::from_u64(50, 3, 10)),
            AxisPhi::new(1, QpDigits::from_u64(100, 3, 10)),
        ];

        let selected = overlay.select_diverse_peers(5, &target_features).await;

        // Should select some peers (exact count depends on ball distribution)
        assert!(!selected.is_empty());
        assert!(selected.len() <= 5);
    }

    #[tokio::test]
    async fn test_ball_id_computation() {
        let overlay = AxisOverlay::new(3, 3);

        let phi1 = QpDigits::from_u64(42, 3, 10);
        let phi2 = QpDigits::from_u64(42, 3, 10);
        let phi3 = QpDigits::from_u64(43, 3, 10);

        let ball1 = overlay.compute_ball_id(&phi1, 3);
        let ball2 = overlay.compute_ball_id(&phi2, 3);
        let ball3 = overlay.compute_ball_id(&phi3, 3);

        // Same values should have same ball ID
        assert_eq!(ball1, ball2);

        // Different values might have different ball IDs (depends on p-adic expansion)
        // At minimum, they should be valid ball IDs
        assert!(!ball1.is_empty());
        assert!(!ball3.is_empty());
    }

    #[tokio::test]
    async fn test_overlay_stats() {
        let overlay = AxisOverlay::new(3, 2);

        // Add peers across multiple axes
        for i in 0..5 {
            let peer_id = create_test_peer_id(i);
            let features = vec![
                AxisPhi::new(0, QpDigits::from_u64(i as u64 * 10, 3, 10)),
                AxisPhi::new(1, QpDigits::from_u64(i as u64 * 20, 3, 10)),
                AxisPhi::new(2, QpDigits::from_u64(i as u64 * 30, 3, 10)),
            ];
            overlay.assign_peer(peer_id, features).await.unwrap();
        }

        let stats = overlay.stats().await;
        assert_eq!(stats.total_peers, 5);
        assert!(stats.total_balls > 0);
    }
}
