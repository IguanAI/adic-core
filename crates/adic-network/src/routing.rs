use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};
use tracing::{debug, info};
use bloom::{BloomFilter, ASMS};

use adic_types::{AdicMessage, MessageId};
use libp2p::PeerId;

#[derive(Debug, Clone)]
pub struct AxisRoute {
    pub axis_id: u32,
    pub ball_id: u64,
    pub peers: Vec<PeerId>,
    pub bloom_filter: Vec<u8>, // Serialized bloom filter
}

#[derive(Clone)]
pub struct RoutingTable {
    local_peer_id: PeerId,
    axis_routes: Arc<RwLock<HashMap<u32, Vec<AxisRoute>>>>,
    peer_axes: Arc<RwLock<HashMap<PeerId, HashSet<u32>>>>,
    message_bloom: Arc<RwLock<BloomFilter>>,
    routing_cache: Arc<RwLock<HashMap<MessageId, Vec<PeerId>>>>,
}

impl std::fmt::Debug for RoutingTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoutingTable")
            .field("local_peer_id", &self.local_peer_id)
            .field("axis_routes", &self.axis_routes)
            .field("peer_axes", &self.peer_axes)
            .field("message_bloom", &"<BloomFilter>")
            .field("routing_cache", &self.routing_cache)
            .finish()
    }
}

impl RoutingTable {
    pub fn new(local_peer_id: PeerId) -> Self {
        let bloom = BloomFilter::with_rate(0.01, 100000); // 1% false positive rate, 100k expected items
        
        Self {
            local_peer_id,
            axis_routes: Arc::new(RwLock::new(HashMap::new())),
            peer_axes: Arc::new(RwLock::new(HashMap::new())),
            message_bloom: Arc::new(RwLock::new(bloom)),
            routing_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_axis_route(&self, axis_id: u32, ball_id: u64, peers: Vec<PeerId>) {
        let mut axis_routes = self.axis_routes.write().await;
        let routes = axis_routes.entry(axis_id).or_insert_with(Vec::new);
        
        // Check if route already exists
        let exists = routes.iter().any(|r| r.ball_id == ball_id);
        if !exists {
            // Create an empty bloom filter representation
            // In a real implementation, this would be properly serialized
            let bloom_bytes = Vec::new();
            
            routes.push(AxisRoute {
                axis_id,
                ball_id,
                peers: peers.clone(),
                bloom_filter: bloom_bytes,
            });
            
            // Update peer-axis mapping
            let mut peer_axes = self.peer_axes.write().await;
            for peer in &peers {
                peer_axes.entry(*peer).or_insert_with(HashSet::new).insert(axis_id);
            }
            
            debug!("Added route for axis {} ball {} with {} peers", axis_id, ball_id, peers.len());
        }
    }

    pub async fn find_route_for_message(&self, message: &AdicMessage) -> Vec<PeerId> {
        // Check cache first
        {
            let cache = self.routing_cache.read().await;
            if let Some(cached_peers) = cache.get(&message.id) {
                return cached_peers.clone();
            }
        }
        
        let mut selected_peers = HashSet::new();
        let axis_routes = self.axis_routes.read().await;
        
        // Route based on p-adic features
        for axis_phi in &message.features.phi {
            if let Some(routes) = axis_routes.get(&axis_phi.axis.0) {
                // Find closest ball using p-adic distance
                // Use first few digits to determine target ball
                let target_ball = axis_phi.qp_digits.digits.get(0).copied().unwrap_or(0) as u64;
                
                for route in routes {
                    let distance = (route.ball_id as i64 - target_ball as i64).abs();
                    if distance < 10 { // Within proximity threshold
                        selected_peers.extend(route.peers.iter().cloned());
                    }
                }
            }
        }
        
        // If no axis-specific routes, use all known peers for the axes
        if selected_peers.is_empty() {
            let peer_axes = self.peer_axes.read().await;
            for axis_phi in &message.features.phi {
                for (peer, axes) in peer_axes.iter() {
                    if axes.contains(&axis_phi.axis.0) {
                        selected_peers.insert(*peer);
                    }
                }
            }
        }
        
        let result: Vec<PeerId> = selected_peers.into_iter().collect();
        
        // Cache the result
        {
            let mut cache = self.routing_cache.write().await;
            cache.insert(message.id, result.clone());
            
            // Limit cache size
            if cache.len() > 10000 {
                cache.clear();
            }
        }
        
        result
    }

    pub async fn check_duplicate(&self, message_id: &MessageId) -> bool {
        let mut bloom = self.message_bloom.write().await;
        let id_bytes = message_id.as_bytes();
        
        if bloom.contains(id_bytes) {
            true
        } else {
            bloom.insert(id_bytes);
            false
        }
    }

    pub async fn get_axis_peers(&self, axis_id: u32) -> Vec<PeerId> {
        let axis_routes = self.axis_routes.read().await;
        
        if let Some(routes) = axis_routes.get(&axis_id) {
            let mut peers = HashSet::new();
            for route in routes {
                peers.extend(route.peers.iter().cloned());
            }
            peers.into_iter().collect()
        } else {
            Vec::new()
        }
    }

    pub async fn get_peer_axes(&self, peer_id: &PeerId) -> Vec<u32> {
        let peer_axes = self.peer_axes.read().await;
        
        peer_axes.get(peer_id)
            .map(|axes| axes.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub async fn remove_peer(&self, peer_id: &PeerId) {
        // Remove from axis routes
        let mut axis_routes = self.axis_routes.write().await;
        for routes in axis_routes.values_mut() {
            for route in routes {
                route.peers.retain(|p| p != peer_id);
            }
        }
        
        // Remove from peer-axis mapping
        let mut peer_axes = self.peer_axes.write().await;
        peer_axes.remove(peer_id);
        
        // Clear routing cache entries that include this peer
        let mut cache = self.routing_cache.write().await;
        cache.retain(|_, peers| !peers.contains(peer_id));
        
        info!("Removed peer {} from routing table", peer_id);
    }

    pub async fn clear_cache(&self) {
        let mut cache = self.routing_cache.write().await;
        cache.clear();
        
        let mut bloom = self.message_bloom.write().await;
        *bloom = BloomFilter::with_rate(0.01, 100000);
        
        debug!("Routing cache cleared");
    }

    pub async fn get_route_stats(&self) -> RouteStats {
        let axis_routes = self.axis_routes.read().await;
        let peer_axes = self.peer_axes.read().await;
        let cache = self.routing_cache.read().await;
        
        let total_axes = axis_routes.len();
        let total_routes = axis_routes.values().map(|v| v.len()).sum();
        let total_peers = peer_axes.len();
        let cache_size = cache.len();
        
        RouteStats {
            total_axes,
            total_routes,
            total_peers,
            cache_size,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteStats {
    pub total_axes: usize,
    pub total_routes: usize,
    pub total_peers: usize,
    pub cache_size: usize,
}

pub struct HypertangleRouter {
    routing_table: Arc<RoutingTable>,
    axis_channels: Arc<RwLock<HashMap<u32, Vec<PeerId>>>>,
    conflict_routes: Arc<RwLock<HashMap<String, Vec<PeerId>>>>,
}

impl HypertangleRouter {
    pub fn new(local_peer_id: PeerId) -> Self {
        Self {
            routing_table: Arc::new(RoutingTable::new(local_peer_id)),
            axis_channels: Arc::new(RwLock::new(HashMap::new())),
            conflict_routes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn route_message(&self, message: &AdicMessage) -> Vec<PeerId> {
        // Check for duplicate
        if self.routing_table.check_duplicate(&message.id).await {
            debug!("Duplicate message {} detected, not routing", message.id);
            return Vec::new();
        }
        
        // Get peers for routing
        let mut peers = self.routing_table.find_route_for_message(message).await;
        
        // If it's a conflict message, add conflict-specific routes
        if !message.meta.conflict.is_none() {
            let conflict_routes = self.conflict_routes.read().await;
            if let Some(conflict_peers) = conflict_routes.get(&message.meta.conflict.0) {
                peers.extend(conflict_peers.iter().cloned());
            }
        }
        
        // Deduplicate
        let unique_peers: HashSet<PeerId> = peers.into_iter().collect();
        unique_peers.into_iter().collect()
    }

    pub async fn setup_axis_channel(&self, axis_id: u32, peers: Vec<PeerId>) {
        let peer_count = peers.len();
        let mut channels = self.axis_channels.write().await;
        channels.insert(axis_id, peers.clone());
        
        // Also update routing table
        self.routing_table.add_axis_route(axis_id, 0, peers).await;
        
        info!("Setup axis channel {} with {} peers", axis_id, peer_count);
    }

    pub async fn setup_conflict_route(&self, conflict_id: String, peers: Vec<PeerId>) {
        let peer_count = peers.len();
        let mut routes = self.conflict_routes.write().await;
        routes.insert(conflict_id.clone(), peers);
        
        info!("Setup conflict route {} with {} peers", conflict_id, peer_count);
    }

    pub async fn broadcast_to_axis(&self, axis_id: u32, _message: &AdicMessage) -> Vec<PeerId> {
        let channels = self.axis_channels.read().await;
        
        if let Some(peers) = channels.get(&axis_id) {
            peers.clone()
        } else {
            self.routing_table.get_axis_peers(axis_id).await
        }
    }

    pub async fn optimize_routes(&self) {
        // Analyze routing patterns and optimize
        let stats = self.routing_table.get_route_stats().await;
        
        if stats.cache_size > 5000 {
            self.routing_table.clear_cache().await;
            debug!("Cleared routing cache due to size");
        }
        
        // In a real implementation, perform more sophisticated optimization
        // such as merging similar routes, removing inactive peers, etc.
    }

    pub fn routing_table(&self) -> &RoutingTable {
        &self.routing_table
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::features::{AdicFeatures, AxisPhi, QpDigits};
    use adic_types::{AdicMeta, PublicKey};

    #[tokio::test]
    async fn test_routing_table_creation() {
        let peer_id = PeerId::random();
        let routing_table = RoutingTable::new(peer_id);
        
        let stats = routing_table.get_route_stats().await;
        assert_eq!(stats.total_axes, 0);
        assert_eq!(stats.total_peers, 0);
    }

    #[tokio::test]
    async fn test_axis_routing() {
        let local_peer = PeerId::random();
        let routing_table = RoutingTable::new(local_peer);
        
        let peers = vec![PeerId::random(), PeerId::random()];
        routing_table.add_axis_route(0, 100, peers.clone()).await;
        
        let axis_peers = routing_table.get_axis_peers(0).await;
        assert_eq!(axis_peers.len(), 2);
    }

    #[tokio::test]
    async fn test_duplicate_detection() {
        let peer_id = PeerId::random();
        let routing_table = RoutingTable::new(peer_id);
        
        let message_id = MessageId::new(b"test");
        
        assert!(!routing_table.check_duplicate(&message_id).await);
        assert!(routing_table.check_duplicate(&message_id).await);
    }

    #[tokio::test]
    async fn test_hypertangle_router() {
        let local_peer = PeerId::random();
        let router = HypertangleRouter::new(local_peer);
        
        let peers = vec![PeerId::random(), PeerId::random()];
        router.setup_axis_channel(0, peers.clone()).await;
        
        let message = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(42, 3, 5))]),
            AdicMeta::new(chrono::Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![],
        );
        
        let routed_peers = router.route_message(&message).await;
        assert_eq!(routed_peers.len(), 2);
    }
}