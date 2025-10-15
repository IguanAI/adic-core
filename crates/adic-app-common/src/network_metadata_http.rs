//! HTTP-based network metadata fetching
//!
//! Provides fallback mechanisms to fetch ASN and region metadata from external services
//! when local data is unavailable.

use crate::network_metadata::{NetworkMetadataRegistry, NodeNetworkInfo};
use crate::security::{
    CertificatePinning, RequestSignature, SecurityAuditEvent, SecurityAuditLogger, SecuritySeverity,
};
use adic_crypto::Keypair;
use adic_types::PublicKey;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Configuration for HTTP metadata fetcher
#[derive(Clone)]
pub struct HttpMetadataConfig {
    /// Primary metadata service URL
    pub primary_url: String,
    /// Fallback metadata service URL
    pub fallback_url: Option<String>,
    /// Request timeout in seconds
    pub timeout_secs: u64,
    /// Retry attempts for failed requests
    pub max_retries: usize,
    /// Cache TTL in seconds (how long to cache fetched metadata)
    pub cache_ttl_secs: u64,
    /// Certificate pinning configuration
    pub cert_pinning: Option<CertificatePinning>,
    /// Whether to require TLS for all requests
    pub require_tls: bool,
    /// Node keypair for request signing (optional)
    pub keypair: Option<Arc<Keypair>>,
}

impl Default for HttpMetadataConfig {
    fn default() -> Self {
        Self {
            primary_url: "https://metadata.adic.network/v1".to_string(),
            fallback_url: Some("https://backup-metadata.adic.network/v1".to_string()),
            timeout_secs: 10,
            max_retries: 3,
            cache_ttl_secs: 3600, // 1 hour
            cert_pinning: None,
            require_tls: true,
            keypair: None,
        }
    }
}

/// Response format from metadata service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataResponse {
    pub public_key: String,
    pub asn: u32,
    pub region: String,
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub timestamp: i64,
}

/// HTTP-based metadata fetcher with automatic fallback
pub struct HttpMetadataFetcher {
    config: HttpMetadataConfig,
    registry: Arc<NetworkMetadataRegistry>,
    audit_logger: Arc<SecurityAuditLogger>,
    #[cfg(feature = "http-metadata")]
    client: reqwest::Client,
}

impl HttpMetadataFetcher {
    /// Create new HTTP metadata fetcher
    pub fn new(config: HttpMetadataConfig, registry: Arc<NetworkMetadataRegistry>) -> Self {
        // Validate TLS requirements
        if config.require_tls {
            if !config.primary_url.starts_with("https://") {
                panic!(
                    "TLS required but primary URL is not HTTPS: {}",
                    config.primary_url
                );
            }
            if let Some(ref fallback) = config.fallback_url {
                if !fallback.starts_with("https://") {
                    panic!("TLS required but fallback URL is not HTTPS: {}", fallback);
                }
            }
        }

        #[cfg(feature = "http-metadata")]
        let client = if let Some(ref cert_pinning) = config.cert_pinning {
            // Create client with certificate pinning
            crate::cert_pinning::create_pinned_client(
                cert_pinning.clone(),
                Duration::from_secs(config.timeout_secs),
            )
            .expect("Failed to create pinned HTTP client")
        } else {
            // Create standard client without pinning
            reqwest::Client::builder()
                .timeout(Duration::from_secs(config.timeout_secs))
                .min_tls_version(reqwest::tls::Version::TLS_1_2)
                .https_only(config.require_tls)
                .build()
                .expect("Failed to create HTTP client")
        };

        let audit_logger = Arc::new(SecurityAuditLogger::new(1000));

        Self {
            config,
            registry,
            audit_logger,
            #[cfg(feature = "http-metadata")]
            client,
        }
    }

    /// Get the security audit logger
    pub fn audit_logger(&self) -> &Arc<SecurityAuditLogger> {
        &self.audit_logger
    }

    /// Fetch metadata for a public key with automatic fallback
    ///
    /// 1. Check local registry
    /// 2. If missing, try primary HTTP service
    /// 3. If primary fails, try fallback service
    /// 4. If all fail, return default metadata
    pub async fn fetch_or_default(&self, public_key: &PublicKey) -> NodeNetworkInfo {
        // Step 1: Check local registry
        if let Some(info) = self.registry.get(public_key).await {
            debug!(
                pubkey = hex::encode(&public_key.as_bytes()[..8]),
                "Metadata found in local registry"
            );
            return info;
        }

        // Step 2 & 3: Try HTTP services
        #[cfg(feature = "http-metadata")]
        {
            if let Some(info) = self.fetch_from_http(public_key).await {
                // Cache the result
                self.registry.register(*public_key, info.clone()).await;
                return info;
            }
        }

        // Step 4: Return default if all else fails
        warn!(
            pubkey = hex::encode(&public_key.as_bytes()[..8]),
            "Could not fetch metadata, using default"
        );
        NodeNetworkInfo::default()
    }

    /// Fetch metadata from HTTP service with retry logic
    #[cfg(feature = "http-metadata")]
    async fn fetch_from_http(&self, public_key: &PublicKey) -> Option<NodeNetworkInfo> {
        let pubkey_hex = hex::encode(public_key.as_bytes());

        // Try primary service
        for attempt in 0..self.config.max_retries {
            match self
                .fetch_from_url(&self.config.primary_url, &pubkey_hex)
                .await
            {
                Ok(info) => {
                    info!(
                        pubkey = &pubkey_hex[..16],
                        asn = info.asn,
                        region = &info.region,
                        source = "primary",
                        attempt = attempt + 1,
                        "Fetched metadata from HTTP"
                    );
                    return Some(info);
                }
                Err(e) => {
                    debug!(
                        pubkey = &pubkey_hex[..16],
                        attempt = attempt + 1,
                        max = self.config.max_retries,
                        error = %e,
                        "Primary metadata fetch failed"
                    );
                }
            }

            if attempt < self.config.max_retries - 1 {
                tokio::time::sleep(Duration::from_millis(100 * (attempt as u64 + 1))).await;
            }
        }

        // Try fallback service if available
        if let Some(ref fallback_url) = self.config.fallback_url {
            for attempt in 0..self.config.max_retries {
                match self.fetch_from_url(fallback_url, &pubkey_hex).await {
                    Ok(info) => {
                        info!(
                            pubkey = &pubkey_hex[..16],
                            asn = info.asn,
                            region = &info.region,
                            source = "fallback",
                            attempt = attempt + 1,
                            "Fetched metadata from fallback HTTP"
                        );
                        return Some(info);
                    }
                    Err(e) => {
                        debug!(
                            pubkey = &pubkey_hex[..16],
                            attempt = attempt + 1,
                            max = self.config.max_retries,
                            error = %e,
                            "Fallback metadata fetch failed"
                        );
                    }
                }

                if attempt < self.config.max_retries - 1 {
                    tokio::time::sleep(Duration::from_millis(100 * (attempt as u64 + 1))).await;
                }
            }
        }

        None
    }

    /// Fetch metadata from a specific URL
    #[cfg(feature = "http-metadata")]
    async fn fetch_from_url(
        &self,
        base_url: &str,
        pubkey_hex: &str,
    ) -> Result<NodeNetworkInfo, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/metadata/{}", base_url, pubkey_hex);

        // Build request
        let mut request_builder = self.client.get(&url);

        // Add request signature if keypair is configured
        if let Some(ref keypair) = self.config.keypair {
            let timestamp = chrono::Utc::now().timestamp();
            let signature = RequestSignature::new(keypair, "GET", &url, b"", timestamp);

            for (key, value) in signature.add_to_headers() {
                request_builder = request_builder.header(key, value);
            }

            // Log security event
            self.audit_logger
                .log(SecurityAuditEvent::new(
                    "request_signing",
                    SecuritySeverity::Info,
                    base_url,
                    format!("Signed request to {}", url),
                    true,
                ))
                .await;
        }

        // Send request
        let response = request_builder.send().await?;

        // Log certificate pinning status
        if self.config.require_tls && self.config.cert_pinning.is_some() {
            // Certificate pinning is enforced at TLS layer during handshake
            // If we got here, the certificate was validated successfully
            self.audit_logger
                .log(SecurityAuditEvent::new(
                    "certificate_pinning",
                    SecuritySeverity::Info,
                    base_url,
                    "Certificate pinning validation passed",
                    true,
                ))
                .await;
        }

        let response = response.error_for_status()?;
        let metadata_response: MetadataResponse = response.json().await?;

        Ok(NodeNetworkInfo {
            asn: metadata_response.asn,
            region: metadata_response.region,
            metadata: metadata_response.metadata,
        })
    }

    /// Batch fetch metadata for multiple public keys
    pub async fn batch_fetch(
        &self,
        public_keys: &[PublicKey],
    ) -> Vec<(PublicKey, NodeNetworkInfo)> {
        let mut results = Vec::with_capacity(public_keys.len());

        for pk in public_keys {
            let info = self.fetch_or_default(pk).await;
            results.push((*pk, info));
        }

        results
    }

    /// Start periodic refresh of all known metadata
    ///
    /// This spawns a background task that refreshes metadata for all registered nodes
    /// at the configured TTL interval.
    pub fn start_periodic_refresh(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let refresh_interval = Duration::from_secs(self.config.cache_ttl_secs);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(refresh_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                interval.tick().await;

                info!("Starting periodic metadata refresh");

                let all_nodes = self.registry.get_all().await;
                let public_keys: Vec<PublicKey> = all_nodes.keys().cloned().collect();

                if public_keys.is_empty() {
                    debug!("No nodes to refresh");
                    continue;
                }

                #[cfg(feature = "http-metadata")]
                {
                    let mut refreshed = 0;
                    let mut failed = 0;

                    for pk in &public_keys {
                        if let Some(info) = self.fetch_from_http(pk).await {
                            self.registry.register(*pk, info).await;
                            refreshed += 1;
                        } else {
                            failed += 1;
                        }

                        // Rate limiting: small delay between requests
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }

                    info!(
                        total = public_keys.len(),
                        refreshed, failed, "Periodic metadata refresh completed"
                    );
                }

                #[cfg(not(feature = "http-metadata"))]
                {
                    debug!("HTTP metadata feature not enabled, skipping refresh");
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fetch_or_default_with_local_cache() {
        let registry = Arc::new(NetworkMetadataRegistry::new());
        let config = HttpMetadataConfig::default();
        let fetcher = HttpMetadataFetcher::new(config, registry.clone());

        // Pre-populate registry
        let pk = PublicKey::from_bytes([1; 32]);
        let info = NodeNetworkInfo {
            asn: 12345,
            region: "us-east".to_string(),
            metadata: std::collections::HashMap::new(),
        };
        registry.register(pk, info.clone()).await;

        // Should return from cache
        let result = fetcher.fetch_or_default(&pk).await;
        assert_eq!(result.asn, 12345);
        assert_eq!(result.region, "us-east");
    }

    #[tokio::test]
    async fn test_fetch_or_default_fallback_to_default() {
        let registry = Arc::new(NetworkMetadataRegistry::new());
        let config = HttpMetadataConfig {
            primary_url: "http://localhost:9999".to_string(), // Non-existent service
            fallback_url: None,
            timeout_secs: 1,
            max_retries: 1,
            cache_ttl_secs: 3600,
            cert_pinning: None,
            require_tls: false,
            keypair: None,
        };
        let fetcher = HttpMetadataFetcher::new(config, registry);

        let pk = PublicKey::from_bytes([2; 32]);

        // Should fallback to default
        let result = fetcher.fetch_or_default(&pk).await;
        assert_eq!(result.asn, 0);
        assert_eq!(result.region, "unknown");
    }

    #[tokio::test]
    async fn test_batch_fetch() {
        let registry = Arc::new(NetworkMetadataRegistry::new());
        let config = HttpMetadataConfig::default();
        let fetcher = HttpMetadataFetcher::new(config, registry.clone());

        // Pre-populate some data
        let pk1 = PublicKey::from_bytes([1; 32]);
        let pk2 = PublicKey::from_bytes([2; 32]);

        registry
            .register(
                pk1,
                NodeNetworkInfo {
                    asn: 100,
                    region: "us".to_string(),
                    metadata: std::collections::HashMap::new(),
                },
            )
            .await;

        let results = fetcher.batch_fetch(&[pk1, pk2]).await;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1.asn, 100);
        assert_eq!(results[0].1.region, "us");
        assert_eq!(results[1].1.asn, 0); // Default for pk2
    }
}
