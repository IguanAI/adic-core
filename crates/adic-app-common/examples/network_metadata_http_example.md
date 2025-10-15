# HTTP Network Metadata Integration

This document demonstrates how to use the HTTP metadata fetcher to automatically obtain ASN and region information for nodes when local data is unavailable.

## Overview

The HTTP metadata system provides:
- **Automatic Fallback**: Fetches metadata from HTTP services when not in local registry
- **Retry Logic**: Configurable retry attempts with exponential backoff
- **Primary/Fallback URLs**: Support for multiple metadata service endpoints
- **Periodic Refresh**: Background task to keep metadata up-to-date
- **Caching**: Automatic caching of fetched metadata in local registry

## Architecture

```
┌─────────────────────┐
│  Get Metadata for   │
│    Public Key       │
└──────────┬──────────┘
           │
           ▼
    ┌──────────────┐
    │  Check Local │
    │   Registry   │
    └──────┬───────┘
           │
    ┌──────▼───────┐
    │  Found?      │
    └──┬─────────┬─┘
       │         │
    Yes│         │No
       │         │
       ▼         ▼
   ┌────────┐  ┌──────────────┐
   │ Return │  │ Fetch from   │
   │  Data  │  │ HTTP Service │
   └────────┘  └──────┬───────┘
                      │
               ┌──────▼───────┐
               │ Primary URL  │
               │   Success?   │
               └──┬─────────┬─┘
                  │         │
               Yes│         │No
                  │         │
                  ▼         ▼
              ┌────────┐  ┌──────────────┐
              │ Cache  │  │ Fallback URL │
              │ & Return│  │   Success?   │
              └────────┘  └──────┬───────┘
                                 │
                          ┌──────▼───────┐
                          │ Yes: Cache   │
                          │ No: Default  │
                          └──────────────┘
```

## Usage

### Basic Setup

Enable the `http-metadata` feature in your `Cargo.toml`:

```toml
[dependencies]
adic-app-common = { path = "../adic-app-common", features = ["http-metadata"] }
```

### Initialize the Fetcher

```rust
use adic_app_common::{NetworkMetadataRegistry, HttpMetadataConfig, HttpMetadataFetcher};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // Create the registry (shared storage)
    let registry = Arc::new(NetworkMetadataRegistry::new());

    // Configure the HTTP fetcher
    let config = HttpMetadataConfig {
        primary_url: "https://metadata.adic.network/v1".to_string(),
        fallback_url: Some("https://backup.adic.network/v1".to_string()),
        timeout_secs: 10,
        max_retries: 3,
        cache_ttl_secs: 3600, // 1 hour
    };

    // Create the fetcher
    let fetcher = Arc::new(HttpMetadataFetcher::new(config, registry.clone()));
}
```

### Fetch Metadata with Automatic Fallback

```rust
use adic_types::PublicKey;

// Fetch metadata for a node
let public_key = PublicKey::from_bytes([1; 32]);
let metadata = fetcher.fetch_or_default(&public_key).await;

println!("ASN: {}", metadata.asn);
println!("Region: {}", metadata.region);

// The result is automatically cached in the registry
// Subsequent calls will use the cached value
let cached = fetcher.fetch_or_default(&public_key).await;
assert_eq!(cached.asn, metadata.asn);
```

### Batch Fetching

```rust
// Fetch metadata for multiple nodes efficiently
let public_keys = vec![
    PublicKey::from_bytes([1; 32]),
    PublicKey::from_bytes([2; 32]),
    PublicKey::from_bytes([3; 32]),
];

let results = fetcher.batch_fetch(&public_keys).await;

for (pk, info) in results {
    println!("Node {:?}: ASN {}, Region {}",
        hex::encode(&pk.as_bytes()[..8]),
        info.asn,
        info.region
    );
}
```

### Periodic Refresh

Start a background task to periodically refresh all known metadata:

```rust
// Start periodic refresh (runs in background)
let refresh_handle = fetcher.clone().start_periodic_refresh();

// The refresh task will:
// 1. Run every cache_ttl_secs (1 hour by default)
// 2. Fetch updated metadata for all registered nodes
// 3. Update the local registry with fresh data
// 4. Continue running until the task is cancelled

// To stop the refresh task later:
// refresh_handle.abort();
```

### Integration with Quorum Selection

When selecting committees, use the HTTP fetcher for nodes without metadata:

```rust
use adic_quorum::QuorumSelector;

async fn select_diverse_committee(
    fetcher: Arc<HttpMetadataFetcher>,
    eligible_nodes: Vec<PublicKey>,
) -> Vec<PublicKey> {
    // Ensure all nodes have metadata
    for pk in &eligible_nodes {
        fetcher.fetch_or_default(pk).await;
    }

    // Now all nodes have metadata in the registry
    // The quorum selector can use it for diversity enforcement
    // ... quorum selection logic ...
}
```

## HTTP Service API

The metadata service should implement the following API:

### GET /v1/metadata/{public_key_hex}

Returns metadata for a specific public key.

**Request:**
```
GET /v1/metadata/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

**Response (200 OK):**
```json
{
  "public_key": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "asn": 15169,
  "region": "us-west-2",
  "metadata": {
    "provider": "Google Cloud",
    "datacenter": "us-west2-a"
  },
  "timestamp": 1735689600
}
```

**Response (404 Not Found):**
```json
{
  "error": "Node not found"
}
```

## Configuration Options

### HttpMetadataConfig

```rust
pub struct HttpMetadataConfig {
    /// Primary metadata service URL
    /// Example: "https://metadata.adic.network/v1"
    pub primary_url: String,

    /// Fallback metadata service URL (optional)
    /// Example: Some("https://backup.adic.network/v1")
    pub fallback_url: Option<String>,

    /// Request timeout in seconds
    /// Default: 10
    pub timeout_secs: u64,

    /// Retry attempts for failed requests
    /// Default: 3
    pub max_retries: usize,

    /// Cache TTL in seconds (how long to cache fetched metadata)
    /// Default: 3600 (1 hour)
    pub cache_ttl_secs: u64,
}
```

## Error Handling

The HTTP fetcher gracefully handles errors:

```rust
// If HTTP requests fail, returns default metadata
let metadata = fetcher.fetch_or_default(&public_key).await;

// Default metadata:
// {
//   asn: 0,
//   region: "unknown",
//   metadata: {}
// }
```

All errors are logged at the appropriate level:
- **debug**: Individual retry failures
- **warn**: Complete fetch failure (all retries exhausted)
- **info**: Successful fetches with source (primary/fallback)

## Performance Considerations

### Caching Strategy

1. **Local Registry**: First check (O(1) HashMap lookup)
2. **HTTP Fetch**: Only when not in local registry
3. **Automatic Caching**: Fetched results stored in registry
4. **TTL-Based Refresh**: Periodic background refresh

### Rate Limiting

The batch fetcher includes built-in rate limiting:
- 10ms delay between requests
- Prevents overwhelming the metadata service
- Configurable via the batch_fetch implementation

### Retry Backoff

Failed requests use exponential backoff:
- Attempt 1: immediate retry
- Attempt 2: 100ms delay
- Attempt 3: 200ms delay
- etc.

## Testing

### Unit Tests

The module includes comprehensive unit tests:

```bash
# Run tests without HTTP feature
cargo test --package adic-app-common network_metadata

# Run tests with HTTP feature
cargo test --package adic-app-common --features http-metadata network_metadata
```

### Integration Testing

For integration testing with a real metadata service:

```rust
#[tokio::test]
#[ignore] // Requires running metadata service
async fn test_real_metadata_service() {
    let config = HttpMetadataConfig {
        primary_url: "http://localhost:8080/v1".to_string(),
        fallback_url: None,
        timeout_secs: 5,
        max_retries: 2,
        cache_ttl_secs: 60,
    };

    let registry = Arc::new(NetworkMetadataRegistry::new());
    let fetcher = HttpMetadataFetcher::new(config, registry);

    let pk = PublicKey::from_bytes([1; 32]);
    let metadata = fetcher.fetch_or_default(&pk).await;

    assert_ne!(metadata.asn, 0);
    assert_ne!(metadata.region, "unknown");
}
```

## Production Deployment

### Metadata Service Setup

1. **Deploy metadata service** with ASN/GeoIP database
2. **Configure URLs** in node configuration
3. **Enable TLS** for secure communication
4. **Set up monitoring** for metadata service health

### Node Configuration

```toml
[network_metadata]
enabled = true
primary_url = "https://metadata.adic.network/v1"
fallback_url = "https://backup.adic.network/v1"
timeout_secs = 10
max_retries = 3
refresh_interval_secs = 3600
```

### Monitoring

Key metrics to monitor:
- Metadata fetch success rate
- Fallback service usage
- Cache hit rate
- Refresh task health
- HTTP request latency

## Summary

The HTTP metadata integration provides:

✅ **Automatic Fallback**: Seamless metadata retrieval from HTTP services
✅ **Resilient**: Primary/fallback URLs with retry logic
✅ **Efficient**: Local caching with configurable TTL
✅ **Background Refresh**: Keep metadata up-to-date automatically
✅ **Easy Integration**: Drop-in replacement for manual metadata management
✅ **Production-Ready**: Full error handling, logging, and testing

This ensures that quorum selection and committee diversity enforcement always have access to accurate network metadata, even when nodes join dynamically or local data is incomplete.
