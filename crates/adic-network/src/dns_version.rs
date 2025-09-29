use anyhow::{anyhow, Result};
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::TokioAsyncResolver;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info};

/// DNS-based version discovery for ADIC network
pub struct DnsVersionDiscovery {
    resolver: TokioAsyncResolver,
    cache: Arc<RwLock<Option<CachedVersion>>>,
    domain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionRecord {
    pub version: String,
    pub sha256_hash: String,
    pub signature: String,
    pub release_date: Option<String>,
    pub min_compatible: Option<String>,
}

#[derive(Debug, Clone)]
struct CachedVersion {
    record: VersionRecord,
    cached_at: Instant,
}

impl DnsVersionDiscovery {
    /// Create a new DNS version discovery instance
    pub fn new(domain: String) -> Result<Self> {
        let resolver =
            TokioAsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default());

        Ok(Self {
            resolver,
            cache: Arc::new(RwLock::new(None)),
            domain,
        })
    }

    /// Query for the latest version information
    pub async fn get_latest_version(&self) -> Result<VersionRecord> {
        // Check cache first (5 minute TTL)
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.as_ref() {
                if cached.cached_at.elapsed() < Duration::from_secs(300) {
                    debug!(
                        version = %cached.record.version,
                        "🔍 Using cached version info"
                    );
                    return Ok(cached.record.clone());
                }
            }
        }

        // Query DNS TXT record
        let txt_name = format!("_version.{}", self.domain);
        info!(
            domain = %txt_name,
            "🌐 Querying DNS for version info"
        );

        let response = self
            .resolver
            .txt_lookup(txt_name.clone())
            .await
            .map_err(|e| anyhow!("DNS lookup failed: {}", e))?;

        // Parse TXT records
        for record in response.iter() {
            let data = record.to_string();
            if let Ok(version_record) = self.parse_txt_record(&data) {
                info!(
                    version = %version_record.version,
                    hash = %&version_record.sha256_hash[..8],
                    "📦 Found version info via DNS"
                );

                // Update cache
                let mut cache = self.cache.write().await;
                *cache = Some(CachedVersion {
                    record: version_record.clone(),
                    cached_at: Instant::now(),
                });

                return Ok(version_record);
            }
        }

        Err(anyhow!("No valid version record found in DNS"))
    }

    /// Parse a TXT record into a VersionRecord
    /// Format: "v=0.2.0;sha256=abc123...;sig=def456...;date=2024-01-15;min=0.1.5"
    fn parse_txt_record(&self, txt: &str) -> Result<VersionRecord> {
        let mut version = String::new();
        let mut sha256_hash = String::new();
        let mut signature = String::new();
        let mut release_date = None;
        let mut min_compatible = None;

        for part in txt.split(';') {
            let kv: Vec<&str> = part.splitn(2, '=').collect();
            if kv.len() != 2 {
                continue;
            }

            match kv[0].trim() {
                "v" | "version" => version = kv[1].trim().to_string(),
                "sha256" | "hash" => sha256_hash = kv[1].trim().to_string(),
                "sig" | "signature" => signature = kv[1].trim().to_string(),
                "date" | "release_date" => release_date = Some(kv[1].trim().to_string()),
                "min" | "min_compatible" => min_compatible = Some(kv[1].trim().to_string()),
                _ => {}
            }
        }

        if version.is_empty() || sha256_hash.is_empty() || signature.is_empty() {
            return Err(anyhow!("Incomplete version record"));
        }

        Ok(VersionRecord {
            version,
            sha256_hash,
            signature,
            release_date,
            min_compatible,
        })
    }

    /// Check if an update is available
    pub async fn check_for_update(&self, current_version: &str) -> Result<Option<VersionRecord>> {
        let latest = self.get_latest_version().await?;

        if self.is_newer_version(&latest.version, current_version) {
            info!(
                current = %current_version,
                available = %latest.version,
                "🆕 Update available"
            );
            Ok(Some(latest))
        } else {
            debug!(
                current = %current_version,
                latest = %latest.version,
                "✅ Already on latest version"
            );
            Ok(None)
        }
    }

    /// Compare version strings (simple semver comparison)
    fn is_newer_version(&self, candidate: &str, current: &str) -> bool {
        let parse_version = |v: &str| -> (u32, u32, u32) {
            let parts: Vec<&str> = v.trim_start_matches('v').split('.').collect();
            let major = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
            let minor = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let patch = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            (major, minor, patch)
        };

        let (c_major, c_minor, c_patch) = parse_version(candidate);
        let (cur_major, cur_minor, cur_patch) = parse_version(current);

        if c_major > cur_major {
            return true;
        }
        if c_major == cur_major && c_minor > cur_minor {
            return true;
        }
        if c_major == cur_major && c_minor == cur_minor && c_patch > cur_patch {
            return true;
        }
        false
    }

    /// Clear the cache
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        *cache = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_txt_record() {
        let discovery = DnsVersionDiscovery::new("adic.network".to_string()).unwrap();

        let txt = "v=0.2.0;sha256=abcdef123456;sig=xyz789;date=2024-01-15;min=0.1.5";
        let record = discovery.parse_txt_record(txt).unwrap();

        assert_eq!(record.version, "0.2.0");
        assert_eq!(record.sha256_hash, "abcdef123456");
        assert_eq!(record.signature, "xyz789");
        assert_eq!(record.release_date, Some("2024-01-15".to_string()));
        assert_eq!(record.min_compatible, Some("0.1.5".to_string()));
    }

    #[test]
    fn test_version_comparison() {
        let discovery = DnsVersionDiscovery::new("adic.network".to_string()).unwrap();

        assert!(discovery.is_newer_version("0.2.0", "0.1.9"));
        assert!(discovery.is_newer_version("1.0.0", "0.9.9"));
        assert!(discovery.is_newer_version("0.1.10", "0.1.9"));
        assert!(!discovery.is_newer_version("0.1.9", "0.2.0"));
        assert!(!discovery.is_newer_version("0.2.0", "0.2.0"));
    }
}
