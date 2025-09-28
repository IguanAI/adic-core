# ADIC Update System Usage Examples

## Basic Update Check

### CLI
```bash
# Check for updates
adic update check

# Check with verbose output
RUST_LOG=debug adic update check
```

### API
```bash
# Check update status
curl http://localhost:8080/update/status

# Manually trigger update check
curl -X POST http://localhost:8080/update/check
```

## Monitoring Swarm Activity

### Real-time Swarm Statistics
```bash
# Get current swarm statistics
curl http://localhost:8080/update/swarm | jq

# Monitor continuously
watch -n 1 'curl -s http://localhost:8080/update/swarm | jq'
```

### Python Script for Monitoring
```python
import requests
import time
import json

def monitor_swarm(node_url="http://localhost:8080"):
    """Monitor swarm update statistics"""
    while True:
        try:
            response = requests.get(f"{node_url}/update/swarm")
            data = response.json()

            if data['success']:
                swarm = data['swarm']
                print(f"\n--- Swarm Update Statistics ---")
                print(f"Download Speed: {swarm['download_speed_mbps']:.2f} MB/s")
                print(f"Upload Speed: {swarm['upload_speed_mbps']:.2f} MB/s")
                print(f"Downloading Peers: {swarm['downloading_peers']}")
                print(f"Seeding Peers: {swarm['seeding_peers']}")
                print(f"Average Progress: {swarm['average_download_progress']:.1f}%")

                # Show version distribution
                print("\nVersion Distribution:")
                for version, count in swarm.get('version_distribution', {}).items():
                    print(f"  v{version}: {count} peers")

        except Exception as e:
            print(f"Error: {e}")

        time.sleep(5)

if __name__ == "__main__":
    monitor_swarm()
```

## Downloading Updates

### Manual Download
```bash
# Download new version
adic update download

# Download with progress display
RUST_LOG=info adic update download
```

### Monitoring Download Progress
```bash
# Check download progress
curl http://localhost:8080/update/progress | jq

# Monitor progress with a script
#!/bin/bash
while true; do
    clear
    echo "=== ADIC Update Progress ==="
    curl -s http://localhost:8080/update/progress | jq '
        "Version: \(.version)",
        "Progress: \(.progress_percent)%",
        "Chunks: \(.chunks_received)/\(.total_chunks)",
        "Speed: \(.download_speed / 1048576 | floor) MB/s",
        "ETA: \(.eta_seconds) seconds"
    '
    sleep 1
done
```

## Configuring Auto-Update

### Environment Variables
```bash
# Enable auto-update
export ADIC_AUTO_UPDATE=true

# Set update window (2 AM - 4 AM)
export ADIC_UPDATE_WINDOW="02:00-04:00"

# Set check interval to 30 minutes
export ADIC_UPDATE_CHECK_INTERVAL=1800

# Start node with auto-update
adic start
```

### Configuration File
```toml
# config.toml
[update]
auto_update = true
check_interval = 3600
update_window_start = "02:00"
update_window_end = "04:00"
max_concurrent_chunks = 5

[update.dns]
domain = "_version.adic.network.adicl1.com"
```

## Publishing Updates (Admin Only)

### Setting DNS Version Record
```bash
# Requires CLOUDFLARE_API_KEY environment variable
export CLOUDFLARE_API_KEY="your-api-key"

# Publish new version
./scripts/publish-update.sh 0.1.7 /path/to/adic-binary

# This will:
# 1. Calculate SHA256 hash
# 2. Sign with Ed25519 key
# 3. Update DNS TXT record
# 4. Start seeding via P2P
```

### Manual DNS Update
```python
import hashlib
import base64
import requests
import os

def publish_version(version, binary_path, api_key):
    """Publish new version to DNS"""

    # Calculate SHA256
    with open(binary_path, 'rb') as f:
        sha256 = hashlib.sha256(f.read()).hexdigest()

    # Create TXT record value
    txt_value = f"v={version};sha256={sha256};date={int(time.time())}"

    # Update DNS via Cloudflare API
    headers = {
        'Authorization': f'Bearer {api_key}',
        'Content-Type': 'application/json'
    }

    # Update record (example)
    response = requests.patch(
        f"https://api.cloudflare.com/client/v4/zones/{zone_id}/dns_records/{record_id}",
        headers=headers,
        json={'content': txt_value}
    )

    return response.json()
```

## Multi-Node Update Testing

### Docker Compose Setup
```yaml
version: '3.8'

services:
  node1:
    image: adic:0.1.6
    environment:
      - ADIC_AUTO_UPDATE=false
    ports:
      - "8081:8080"

  node2:
    image: adic:0.1.6
    environment:
      - ADIC_AUTO_UPDATE=true
      - ADIC_UPDATE_CHECK_INTERVAL=60
    ports:
      - "8082:8080"

  node3:
    image: adic:0.1.6
    environment:
      - ADIC_AUTO_UPDATE=true
      - ADIC_UPDATE_CHECK_INTERVAL=60
    ports:
      - "8083:8080"
```

### Test Script
```bash
#!/bin/bash
# test-swarm-update.sh

echo "Starting update test with 3 nodes..."

# Start nodes
docker-compose up -d

# Wait for network formation
sleep 10

# Trigger update on node1 (seeder)
echo "Publishing update on node1..."
docker exec node1 adic update publish 0.1.7

# Monitor swarm
echo "Monitoring swarm update distribution..."
while true; do
    clear
    echo "=== Node Update Status ==="
    for port in 8081 8082 8083; do
        echo -n "Node $port: "
        curl -s http://localhost:$port/update/status | jq -r '.current_version'
    done

    echo -e "\n=== Swarm Statistics ==="
    curl -s http://localhost:8081/update/swarm | jq '.swarm |
        "Download: \(.download_speed_mbps) MB/s",
        "Upload: \(.upload_speed_mbps) MB/s",
        "Downloading: \(.downloading_peers)",
        "Seeding: \(.seeding_peers)"'

    sleep 2
done
```

## Troubleshooting

### Check Logs
```bash
# View update-related logs
journalctl -u adic | grep -i update

# Debug mode for update system
RUST_LOG=adic_node::update_manager=debug,adic_network::protocol::update_protocol=trace adic start
```

### Verify DNS Records
```bash
# Check version DNS record
dig TXT _version.adic.network.adicl1.com +short

# Parse version info
dig TXT _version.adic.network.adicl1.com +short | sed 's/"//g' | tr ';' '\n'
```

### Reset Update State
```bash
# Clear downloaded chunks
rm -rf ~/.adic/data/binaries/chunks/

# Reset update state
rm ~/.adic/data/update_state.json

# Force re-check
adic update check --force
```

## Security Verification

### Verify Binary Signature
```bash
# Download public key
curl -O https://adic.network/release.pub

# Verify signature (manual)
openssl dgst -sha256 -verify release.pub -signature update.sig adic-binary

# Via API
curl http://localhost:8080/update/verify
```

### Check Binary Hash
```bash
# Calculate hash
sha256sum adic-binary

# Compare with DNS record
dig TXT _version.adic.network.adicl1.com +short | grep -o 'sha256=[^;]*'
```

## Performance Tuning

### Optimize Chunk Downloads
```toml
[update]
# Increase concurrent chunks for fast networks
max_concurrent_chunks = 10

# Reduce timeout for responsive peers
chunk_timeout = 15

# Increase retry limit for unreliable networks
retry_limit = 5
```

### Network Bandwidth Control
```bash
# Limit upload bandwidth (Linux)
tc qdisc add dev eth0 root tbf rate 1mbit burst 32kbit latency 100ms

# Monitor bandwidth usage
iftop -i eth0 -f "port 9000"
```

## Integration with CI/CD

### GitHub Actions Example
```yaml
name: Deploy Update

on:
  release:
    types: [published]

jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2

      - name: Build Binary
        run: cargo build --release

      - name: Calculate Hash
        run: sha256sum target/release/adic > hash.txt

      - name: Sign Binary
        run: |
          echo "${{ secrets.SIGNING_KEY }}" | base64 -d > key.pem
          openssl dgst -sha256 -sign key.pem -out signature.bin target/release/adic

      - name: Update DNS
        env:
          CF_API_KEY: ${{ secrets.CLOUDFLARE_API_KEY }}
        run: |
          ./scripts/update-dns.sh ${{ github.event.release.tag_name }}

      - name: Seed to Network
        run: |
          ./target/release/adic update seed --version ${{ github.event.release.tag_name }}
```

## Monitoring Dashboard

### Grafana Query Examples
```sql
-- Update adoption rate
SELECT
  time,
  COUNT(DISTINCT peer_id) FILTER (WHERE version = '0.1.7') as v017_nodes,
  COUNT(DISTINCT peer_id) as total_nodes,
  (COUNT(DISTINCT peer_id) FILTER (WHERE version = '0.1.7')::float /
   COUNT(DISTINCT peer_id)) * 100 as adoption_percent
FROM node_metrics
WHERE time > NOW() - INTERVAL '1 hour'
GROUP BY time
ORDER BY time;

-- Swarm transfer speeds
SELECT
  time,
  SUM(download_speed) / 1048576 as total_download_mbps,
  SUM(upload_speed) / 1048576 as total_upload_mbps,
  COUNT(*) as peer_count
FROM swarm_metrics
WHERE time > NOW() - INTERVAL '10 minutes'
GROUP BY time
ORDER BY time;
```

## Related Documentation

- [Update System Architecture](../docs/UPDATE-SYSTEM.md)
- [API Documentation](../API.md#update-system-endpoints)
- [Security Considerations](../SECURITY.md#update-verification)
- [Logging Best Practices](../docs/LOGGING_BEST_PRACTICES.md)