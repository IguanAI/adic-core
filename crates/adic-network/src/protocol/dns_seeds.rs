use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::TokioAsyncResolver;
use libp2p::Multiaddr;
use std::sync::Arc;
use tracing::{debug, info, warn};

use adic_types::{AdicError, Result};

/// DNS-based seed discovery for finding bootstrap nodes
pub struct DnsSeedDiscovery {
    resolver: Arc<TokioAsyncResolver>,
    seed_domains: Vec<String>,
}

impl DnsSeedDiscovery {
    /// Create a new DNS seed discovery instance
    pub async fn new(seed_domains: Vec<String>) -> Result<Self> {
        let resolver =
            TokioAsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default());

        Ok(Self {
            resolver: Arc::new(resolver),
            seed_domains,
        })
    }

    /// Discover bootstrap nodes from all configured DNS seed domains
    pub async fn discover_seeds(&self) -> Vec<Multiaddr> {
        let mut all_seeds = Vec::new();

        for domain in &self.seed_domains {
            match self.query_txt_records(domain).await {
                Ok(seeds) => {
                    info!(
                        "Discovered {} bootstrap nodes from DNS seed: {}",
                        seeds.len(),
                        domain
                    );
                    all_seeds.extend(seeds);
                }
                Err(e) => {
                    warn!("Failed to query DNS seeds from {}: {}", domain, e);
                }
            }
        }

        // Deduplicate addresses
        all_seeds.sort();
        all_seeds.dedup();

        info!(
            "Total {} unique bootstrap nodes discovered from DNS",
            all_seeds.len()
        );
        all_seeds
    }

    /// Query TXT records from a specific domain
    async fn query_txt_records(&self, domain: &str) -> Result<Vec<Multiaddr>> {
        debug!("Querying DNS TXT records for: {}", domain);

        let txt_lookup =
            self.resolver.txt_lookup(domain).await.map_err(|e| {
                AdicError::Network(format!("DNS lookup failed for {}: {}", domain, e))
            })?;

        let mut seeds = Vec::new();

        for record in txt_lookup.iter() {
            for txt_data in record.txt_data() {
                // TXT records contain binary data, convert to string
                let text = String::from_utf8_lossy(txt_data);
                debug!("Processing TXT record: {}", text);

                if let Some(addr) = self.parse_seed_record(&text) {
                    seeds.push(addr);
                }
            }
        }

        Ok(seeds)
    }

    /// Parse a seed record from DNS TXT data
    ///
    /// Expected formats:
    /// - "addr=/dns4/bootstrap1.adicl1.com/udp/9001/quic/p2p/12D3KooW..."
    /// - "/dns4/bootstrap1.adicl1.com/udp/9001/quic/p2p/12D3KooW..." (without prefix)
    /// - Multiple addresses separated by semicolon: "addr=/dns4/...;/dns4/..."
    pub fn parse_seed_record(&self, record: &str) -> Option<Multiaddr> {
        // Handle multiple formats for flexibility
        let addr_str = if record.starts_with("addr=") {
            record.strip_prefix("addr=")?
        } else if record.starts_with('/') {
            // Direct multiaddr format
            record
        } else {
            return None;
        };

        // Parse as multiaddr
        match addr_str.parse::<Multiaddr>() {
            Ok(addr) => {
                debug!(multiaddr = %addr, "Parsed valid multiaddr");
                Some(addr)
            }
            Err(e) => {
                warn!(
                    addr_str = %addr_str,
                    error = %e,
                    "Failed to parse multiaddr"
                );
                None
            }
        }
    }

    /// Parse multiple seed records from a single TXT record (semicolon-separated)
    pub fn parse_multiple_seeds(&self, record: &str) -> Vec<Multiaddr> {
        let mut seeds = Vec::new();

        // Remove "addr=" prefix if present
        let addresses = if record.starts_with("addr=") {
            record.strip_prefix("addr=").unwrap_or(record)
        } else {
            record
        };

        // Split by semicolon and parse each address
        for addr_str in addresses.split(';') {
            let trimmed = addr_str.trim();
            if !trimmed.is_empty() {
                if let Ok(addr) = trimmed.parse::<Multiaddr>() {
                    seeds.push(addr);
                }
            }
        }

        seeds
    }
}

/// Configuration for DNS seed discovery
#[derive(Clone, Debug)]
pub struct DnsSeedConfig {
    /// DNS domains to query for seed nodes
    pub seed_domains: Vec<String>,
    /// Whether to use DNS seeds (can be disabled for testing)
    pub enabled: bool,
}

impl Default for DnsSeedConfig {
    fn default() -> Self {
        Self {
            seed_domains: vec!["_seeds.adicl1.com".to_string()],
            enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_seed_record_with_prefix() {
        let discovery = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(DnsSeedDiscovery::new(vec![]))
            .unwrap();

        let record = "addr=/ip4/127.0.0.1/udp/9001/quic";
        let addr = discovery.parse_seed_record(record);
        assert!(addr.is_some());
    }

    #[test]
    fn test_parse_seed_record_without_prefix() {
        let discovery = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(DnsSeedDiscovery::new(vec![]))
            .unwrap();

        let record = "/ip4/127.0.0.1/udp/9001/quic";
        let addr = discovery.parse_seed_record(record);
        assert!(addr.is_some());
    }

    #[test]
    fn test_parse_multiple_seeds() {
        let discovery = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(DnsSeedDiscovery::new(vec![]))
            .unwrap();

        let record = "addr=/ip4/127.0.0.1/udp/9001/quic;/ip4/192.168.1.1/udp/9002/quic";
        let addrs = discovery.parse_multiple_seeds(record);
        assert_eq!(addrs.len(), 2);
    }

    #[test]
    fn test_parse_dns4_multiaddr() {
        let discovery = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(DnsSeedDiscovery::new(vec![]))
            .unwrap();

        let record = "addr=/dns4/bootstrap1.adicl1.com/udp/9001/quic/p2p/12D3KooWLRPgRv7aDfmH6K5J7hT3aBp8KkdkQhr3qVEEuz6NXFj7";
        let addr = discovery.parse_seed_record(record);
        assert!(addr.is_some());

        // Verify the multiaddr contains expected protocols
        if let Some(ma) = addr {
            let protocols: Vec<_> = ma.iter().collect();
            assert!(protocols.len() >= 3); // Should have dns4, udp, quic at minimum
        }
    }

    #[tokio::test]
    #[ignore] // Run manually when DNS is configured
    async fn test_real_dns_query() {
        let discovery = DnsSeedDiscovery::new(vec!["_seeds.adicl1.com".to_string()])
            .await
            .unwrap();

        let seeds = discovery.discover_seeds().await;
        debug!(seed_count = seeds.len(), "Discovered seeds from DNS");

        // When properly configured, should find at least one seed
        if !seeds.is_empty() {
            for seed in &seeds {
                debug!(seed = %seed, "Found seed");
            }
        }
    }
}
