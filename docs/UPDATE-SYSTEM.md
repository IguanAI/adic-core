# ADIC P2P Update System Documentation

## Overview

The ADIC update system provides a fully decentralized, peer-to-peer software update distribution mechanism that enables the network to self-update without relying on centralized servers or CDNs. This system is designed to be resilient, secure, and efficient, leveraging the existing P2P network infrastructure.

## Architecture

### Components

#### 1. Update Manager (`update_manager.rs`)
The central coordinator for update operations on each node:
- Schedules periodic update checks
- Manages update windows for controlled rollouts
- Coordinates download, verification, and application phases
- Handles both automatic and manual update modes

#### 2. Update Protocol (`update_protocol.rs`)
P2P protocol for distributing updates across the network:
- Message types: VersionAnnounce, VersionQuery, BinaryRequest, ChunkResponse
- Rate limiting and concurrent transfer management
- Retry mechanisms with exponential backoff
- Peer reputation tracking for reliable sources

#### 3. Binary Store (`binary_store.rs`)
Efficient storage and management of binary chunks:
- 1MB chunk size for optimal network transfer
- In-memory caching for frequently accessed chunks
- SHA256 verification for each chunk
- Assembly of complete binaries from chunks

#### 4. Swarm Tracker (`swarm_tracker.rs`)
Network-wide performance monitoring:
- Aggregates download/upload speeds from all peers
- Tracks peer states (downloading, seeding, idle)
- Monitors version distribution across the network
- Provides real-time swarm statistics

#### 5. DNS Discovery (`dns_version_discovery.rs`)
Version announcement via DNS TXT records:
- Domain: `_version.adic.network.adicl1.com`
- Format: `v=0.1.7;sha256=<hash>;sig=<signature>;date=<timestamp>`
- Fallback to P2P discovery if DNS unavailable

## Update Flow

### Phase 1: Discovery
1. **Scheduled Check**: Update manager triggers periodic checks (default: hourly)
2. **DNS Query**: Check TXT record for latest version
3. **P2P Fallback**: Query connected peers if DNS fails
4. **Version Comparison**: Determine if update needed

### Phase 2: Download
1. **Peer Selection**: Find peers with the new version
2. **Chunk Request**: Request specific chunks from multiple peers
3. **Parallel Download**: Download chunks concurrently (max 5 by default)
4. **Hash Verification**: Verify SHA256 for each chunk
5. **Progress Tracking**: Update progress display and swarm metrics

### Phase 3: Verification
1. **Binary Assembly**: Combine chunks into complete binary
2. **Full Hash Check**: Verify complete binary SHA256
3. **Signature Verification**: Validate Ed25519 signature
4. **Version Record Check**: Ensure version metadata matches

### Phase 4: Application
1. **Copyover Preparation**: Prepare for graceful restart
2. **Socket Preservation**: Maintain active connections
3. **Binary Replacement**: Replace executable using execv()
4. **State Recovery**: Resume operations with new version

## Configuration

### Environment Variables
```bash
# Update checking interval (seconds)
ADIC_UPDATE_CHECK_INTERVAL=3600

# Update window (HH:MM-HH:MM format)
ADIC_UPDATE_WINDOW="02:00-04:00"

# Auto-update mode
ADIC_AUTO_UPDATE=false

# DNS domain for version discovery
ADIC_VERSION_DNS_DOMAIN="_version.adic.network.adicl1.com"

# Cloudflare API for DNS updates (publishers only)
CLOUDFLARE_API_KEY="your-api-key"
```

### Configuration File (`config.toml`)
```toml
[update]
check_interval = 3600
auto_update = false
update_window_start = "02:00"
update_window_end = "04:00"
max_concurrent_chunks = 5
chunk_timeout = 30
retry_limit = 3

[update.dns]
domain = "_version.adic.network.adicl1.com"
timeout = 5
```

## Security

### Cryptographic Verification
- **SHA256 Hashing**: Every chunk and complete binary verified
- **Ed25519 Signatures**: Binary signed by official release key
- **DNS Security**: DNSSEC validation when available
- **Peer Trust**: Reputation-based peer selection

### Attack Mitigation
- **Sybil Resistance**: Multiple peer verification required
- **Version Pinning**: Nodes can pin specific versions
- **Rollback Protection**: Version downgrade prevention
- **Chunk Poisoning**: Hash verification prevents bad chunks

## CLI Commands

### Check for Updates
```bash
adic update check
```
Output:
```
🔍 Checking for updates...
Current version: 0.1.6
Latest version: 0.1.7
Update available: Yes
SHA256: 3a4f5b6c7d8e9f0a1b2c3d4e5f6g7h8i...
```

### Download Update
```bash
adic update download
```
Shows progress:
```
Downloading ADIC v0.1.7...

▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░  45.2%

2.3 MB / 5.1 MB    •    1.2 MB/s    •    3s remaining
Swarm: ↓ 45.2 MB/s / ↑ 127.8 MB/s (42 peers distributing)
```

### Apply Update
```bash
adic update apply
```
Performs graceful restart with new binary.

### View Update Status
```bash
adic update status
```
Output:
```
Update Status:
  Current version: 0.1.6
  Update system: Ready
  Last check: 2024-12-28 12:00:00
  Auto-update: Disabled
```

## API Endpoints

### GET /update/swarm
Returns real-time swarm statistics including collective download/upload speeds, peer distribution, and version adoption.

### GET /update/status
Returns current node update status, available versions, and configuration.

### POST /update/check
Manually triggers an update check.

### GET /update/progress
Returns detailed download progress including chunk status and ETA.

## Swarm Metrics

### SwarmMetrics Message
Peers broadcast their metrics every 5 seconds:
```rust
SwarmMetrics {
    download_speed: u64,
    upload_speed: u64,
    active_transfers: u32,
    seeding_versions: Vec<String>,
    downloading_version: Option<String>,
    download_progress: Option<f32>,
    peer_count: usize,
    timestamp: u64,
}
```

### Aggregated Statistics
- Total network bandwidth usage
- Version adoption rate
- Average download completion time
- Peer contribution ratios

## Performance Optimizations

### Chunk Caching
- LRU cache for recently accessed chunks
- Configurable cache size (default: 100MB)
- Automatic cache cleanup

### Parallel Transfers
- Concurrent chunk downloads from multiple peers
- Load balancing across available seeders
- Automatic peer switching on failure

### Rate Limiting
- Upload bandwidth limiting to prevent saturation
- Fair queuing for multiple requesters
- Priority given to rare chunks

## Monitoring

### Logs
Structured logging with performance metrics:
```
INFO peer_id=12D3KooW... version=0.1.7 chunks=51 "📦 Version announced"
DEBUG chunk_index=23 size_bytes=1048576 "Chunk received"
INFO download_speed_mbps=2.3 upload_speed_mbps=5.1 "Swarm statistics updated"
```

### Metrics
Prometheus-compatible metrics:
- `adic_update_chunks_downloaded_total`
- `adic_update_bytes_transferred_total`
- `adic_update_swarm_speed_bytes_per_second`
- `adic_update_peers_by_version`

## Troubleshooting

### Common Issues

#### Update Check Fails
- Check DNS connectivity: `dig TXT _version.adic.network.adicl1.com`
- Verify network connectivity to peers
- Check logs for DNS timeout errors

#### Slow Downloads
- Check peer count: need seeders with new version
- Verify network bandwidth availability
- Check swarm statistics for overall network speed

#### Verification Failures
- Ensure system time is synchronized
- Check for corrupted chunks in storage
- Verify DNS record integrity

#### Apply Fails
- Check file permissions on binary
- Ensure sufficient disk space
- Verify no security software blocking execv()

### Debug Mode
Enable detailed update logging:
```bash
RUST_LOG=adic_node::update_manager=debug,adic_network::protocol::update_protocol=debug adic start
```

## Future Enhancements

### Planned Features
- Delta updates for bandwidth efficiency
- Torrent-like piece selection algorithms
- Geographic peer prioritization
- Update staging and testing phases
- Rollback mechanisms with state preservation
- Multi-signature requirement for critical updates

### Protocol Extensions
- Merkle tree verification for chunks
- Zero-knowledge proofs for version authenticity
- Homomorphic encryption for private networks
- IPFS integration for persistent storage

## Contributing

The update system is a critical component of ADIC's infrastructure. When contributing:

1. **Test Thoroughly**: Updates affect the entire network
2. **Maintain Compatibility**: Ensure backward compatibility
3. **Document Changes**: Update this documentation
4. **Security First**: All changes must maintain security guarantees
5. **Performance**: Monitor impact on network performance

For implementation details, see:
- `/crates/adic-node/src/update_manager.rs`
- `/crates/adic-network/src/protocol/update_protocol.rs`
- `/crates/adic-network/src/protocol/binary_store.rs`
- `/crates/adic-network/src/protocol/swarm_tracker.rs`

## References

- [Copyover Technique](https://en.wikipedia.org/wiki/Exec_(system_call)#C_language_prototypes)
- [BitTorrent Protocol](https://www.bittorrent.org/beps/bep_0003.html)
- [DNS TXT Records](https://datatracker.ietf.org/doc/html/rfc1035#section-3.3.14)
- [Ed25519 Signatures](https://ed25519.cr.yp.to/)