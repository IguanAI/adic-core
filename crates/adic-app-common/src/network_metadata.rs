use adic_types::PublicKey;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Network metadata for a node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeNetworkInfo {
    /// Autonomous System Number
    pub asn: u32,
    /// Geographic region (ISO 3166-1 alpha-2 code or custom region)
    pub region: String,
    /// Optional additional metadata
    pub metadata: HashMap<String, String>,
}

impl Default for NodeNetworkInfo {
    fn default() -> Self {
        Self {
            asn: 0,
            region: "unknown".to_string(),
            metadata: HashMap::new(),
        }
    }
}

/// Registry for node network metadata
///
/// Provides ASN and region information for nodes, used in:
/// - Committee diversity enforcement
/// - Quorum selection
/// - Geographic distribution analysis
pub struct NetworkMetadataRegistry {
    metadata: Arc<RwLock<HashMap<PublicKey, NodeNetworkInfo>>>,
}

impl NetworkMetadataRegistry {
    pub fn new() -> Self {
        Self {
            metadata: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register or update network metadata for a node
    pub async fn register(&self, public_key: PublicKey, info: NodeNetworkInfo) {
        let mut metadata = self.metadata.write().await;
        metadata.insert(public_key, info);
    }

    /// Get network metadata for a node
    pub async fn get(&self, public_key: &PublicKey) -> Option<NodeNetworkInfo> {
        let metadata = self.metadata.read().await;
        metadata.get(public_key).cloned()
    }

    /// Get network metadata for a node, or return default
    pub async fn get_or_default(&self, public_key: &PublicKey) -> NodeNetworkInfo {
        self.get(public_key).await.unwrap_or_default()
    }

    /// Batch register multiple nodes
    pub async fn register_batch(&self, entries: Vec<(PublicKey, NodeNetworkInfo)>) {
        let mut metadata = self.metadata.write().await;
        for (pk, info) in entries {
            metadata.insert(pk, info);
        }
    }

    /// Get all registered nodes
    pub async fn get_all(&self) -> HashMap<PublicKey, NodeNetworkInfo> {
        let metadata = self.metadata.read().await;
        metadata.clone()
    }

    /// Remove metadata for a node
    pub async fn remove(&self, public_key: &PublicKey) -> Option<NodeNetworkInfo> {
        let mut metadata = self.metadata.write().await;
        metadata.remove(public_key)
    }

    /// Get count of registered nodes
    pub async fn count(&self) -> usize {
        let metadata = self.metadata.read().await;
        metadata.len()
    }

    /// Infer network metadata from IP address (simplified)
    ///
    /// In production, this would use GeoIP databases and ASN lookups.
    /// For now, provides basic heuristics.
    pub fn infer_from_ip(ip_address: &str) -> NodeNetworkInfo {
        // Simplified inference - in production would use MaxMind GeoIP2, IPinfo, etc.
        let (asn, region) = if ip_address.starts_with("10.") || ip_address.starts_with("192.168.") {
            (64512, "local".to_string()) // Private network
        } else if ip_address.starts_with("8.8.") {
            (15169, "us".to_string()) // Google DNS
        } else if ip_address.starts_with("1.1.") {
            (13335, "us".to_string()) // Cloudflare
        } else {
            // Default for unknown IPs
            (0, "unknown".to_string())
        };

        NodeNetworkInfo {
            asn,
            region,
            metadata: HashMap::new(),
        }
    }
}

impl Default for NetworkMetadataRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_and_get() {
        let registry = NetworkMetadataRegistry::new();
        let pk = PublicKey::from_bytes([1; 32]);

        let info = NodeNetworkInfo {
            asn: 15169,
            region: "us-west".to_string(),
            metadata: HashMap::new(),
        };

        registry.register(pk, info.clone()).await;

        let retrieved = registry.get(&pk).await.unwrap();
        assert_eq!(retrieved.asn, 15169);
        assert_eq!(retrieved.region, "us-west");
    }

    #[tokio::test]
    async fn test_get_or_default() {
        let registry = NetworkMetadataRegistry::new();
        let pk = PublicKey::from_bytes([2; 32]);

        let info = registry.get_or_default(&pk).await;
        assert_eq!(info.asn, 0);
        assert_eq!(info.region, "unknown");
    }

    #[tokio::test]
    async fn test_batch_register() {
        let registry = NetworkMetadataRegistry::new();

        let entries = vec![
            (
                PublicKey::from_bytes([1; 32]),
                NodeNetworkInfo {
                    asn: 100,
                    region: "us".to_string(),
                    metadata: HashMap::new(),
                },
            ),
            (
                PublicKey::from_bytes([2; 32]),
                NodeNetworkInfo {
                    asn: 200,
                    region: "eu".to_string(),
                    metadata: HashMap::new(),
                },
            ),
        ];

        registry.register_batch(entries).await;
        assert_eq!(registry.count().await, 2);
    }

    #[test]
    fn test_infer_from_ip() {
        let info = NetworkMetadataRegistry::infer_from_ip("8.8.8.8");
        assert_eq!(info.asn, 15169);
        assert_eq!(info.region, "us");

        let info = NetworkMetadataRegistry::infer_from_ip("192.168.1.1");
        assert_eq!(info.asn, 64512);
        assert_eq!(info.region, "local");
    }
}
