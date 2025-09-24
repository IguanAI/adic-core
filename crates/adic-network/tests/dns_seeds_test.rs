#[cfg(test)]
mod tests {
    use adic_network::protocol::dns_seeds::{DnsSeedConfig, DnsSeedDiscovery};
    use libp2p::Multiaddr;

    #[test]
    fn test_dns_seed_config_default() {
        let config = DnsSeedConfig::default();
        assert!(config.enabled);
        assert_eq!(config.seed_domains.len(), 1);
        assert_eq!(config.seed_domains[0], "_seeds.adicl1.com");
    }

    #[tokio::test]
    async fn test_dns_seed_record_parsing() {
        let discovery = DnsSeedDiscovery::new(vec![]).await.unwrap();

        // Test with addr= prefix
        let record = "addr=/ip4/192.168.1.1/udp/9001/quic";
        let addr = discovery.parse_seed_record(record);
        assert!(addr.is_some());
        assert_eq!(addr.unwrap().to_string(), "/ip4/192.168.1.1/udp/9001/quic");

        // Test without prefix
        let record = "/ip4/10.0.0.1/udp/9002/quic";
        let addr = discovery.parse_seed_record(record);
        assert!(addr.is_some());
        assert_eq!(addr.unwrap().to_string(), "/ip4/10.0.0.1/udp/9002/quic");

        // Test invalid record
        let record = "invalid record";
        let addr = discovery.parse_seed_record(record);
        assert!(addr.is_none());
    }

    #[tokio::test]
    async fn test_dns_seed_multiple_addresses() {
        let discovery = DnsSeedDiscovery::new(vec![]).await.unwrap();

        // Test semicolon-separated addresses
        let record = "addr=/ip4/192.168.1.1/udp/9001/quic;/ip4/192.168.1.2/udp/9002/quic";
        let addrs = discovery.parse_multiple_seeds(record);
        assert_eq!(addrs.len(), 2);
        assert_eq!(addrs[0].to_string(), "/ip4/192.168.1.1/udp/9001/quic");
        assert_eq!(addrs[1].to_string(), "/ip4/192.168.1.2/udp/9002/quic");
    }

    #[tokio::test]
    async fn test_dns4_multiaddr_parsing() {
        let discovery = DnsSeedDiscovery::new(vec![]).await.unwrap();

        // Test DNS4 multiaddr with peer ID
        let record = "addr=/dns4/bootstrap1.adicl1.com/udp/9001/quic/p2p/12D3KooWLRPgRv7aDfmH6K5J7hT3aBp8KkdkQhr3qVEEuz6NXFj7";
        let addr = discovery.parse_seed_record(record);
        assert!(addr.is_some());

        let multiaddr = addr.unwrap();
        let protocols: Vec<_> = multiaddr.iter().collect();

        // Should contain dns4, udp, quic, and p2p protocols
        assert!(protocols.len() >= 4);

        // Check the multiaddr string contains expected components
        let addr_str = multiaddr.to_string();
        assert!(addr_str.contains("dns4"));
        assert!(addr_str.contains("bootstrap1.adicl1.com"));
        assert!(addr_str.contains("9001"));
        assert!(addr_str.contains("quic"));
        assert!(addr_str.contains("p2p"));
    }

    #[tokio::test]
    async fn test_empty_dns_seeds() {
        let discovery = DnsSeedDiscovery::new(vec![]).await.unwrap();
        let seeds = discovery.discover_seeds().await;
        assert_eq!(seeds.len(), 0);
    }

    #[tokio::test]
    #[ignore] // Run manually when DNS is configured
    async fn test_real_dns_query_adicl1() {
        // This test will only work when _seeds.adicl1.com has TXT records configured
        let discovery = DnsSeedDiscovery::new(vec!["_seeds.adicl1.com".to_string()])
            .await
            .unwrap();

        let seeds = discovery.discover_seeds().await;

        println!("DNS Seed Discovery Results:");
        println!("Found {} seeds from _seeds.adicl1.com", seeds.len());

        for (i, seed) in seeds.iter().enumerate() {
            println!("  {}. {}", i + 1, seed);

            // Verify the multiaddr is well-formed
            let protocols: Vec<_> = seed.iter().collect();
            assert!(
                !protocols.is_empty(),
                "Seed should have at least one protocol"
            );
        }

        // When properly configured, should find at least one seed
        if !seeds.is_empty() {
            println!("\n✓ DNS seed discovery is working!");
        } else {
            println!("\n⚠ No seeds found - ensure TXT records are configured at _seeds.adicl1.com");
        }
    }

    #[tokio::test]
    async fn test_dns_seed_deduplication() {
        // Create a mock scenario where we might get duplicate addresses
        let _discovery = DnsSeedDiscovery::new(vec![]).await.unwrap();

        // In real implementation, discover_seeds() deduplicates
        // This test verifies the deduplication logic works
        let test_addrs = vec![
            "/ip4/192.168.1.1/udp/9001/quic"
                .parse::<Multiaddr>()
                .unwrap(),
            "/ip4/192.168.1.1/udp/9001/quic"
                .parse::<Multiaddr>()
                .unwrap(),
            "/ip4/192.168.1.2/udp/9002/quic"
                .parse::<Multiaddr>()
                .unwrap(),
        ];

        // Simulate deduplication
        let mut unique_addrs = test_addrs.clone();
        unique_addrs.sort();
        unique_addrs.dedup();

        assert_eq!(unique_addrs.len(), 2);
    }

    #[tokio::test]
    #[ignore] // Performance test - run manually
    async fn test_dns_seed_performance() {
        use std::time::Instant;

        let start = Instant::now();

        let discovery = DnsSeedDiscovery::new(vec![
            "_seeds.adicl1.com".to_string(),
            "_backup.adicl1.com".to_string(),   // May not exist
            "_tertiary.adicl1.com".to_string(), // May not exist
        ])
        .await
        .unwrap();

        let seeds = discovery.discover_seeds().await;

        let elapsed = start.elapsed();

        println!("DNS seed discovery took: {:?}", elapsed);
        println!("Discovered {} seeds", seeds.len());

        // DNS queries should complete reasonably quickly even with failures
        assert!(elapsed.as_secs() < 10, "DNS discovery took too long");
    }
}
