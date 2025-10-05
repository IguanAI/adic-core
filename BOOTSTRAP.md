# ADIC Bootstrap Node Setup Guide

## Overview

A bootstrap node is a special ADIC node that initializes the genesis state for a new ADIC-DAG network. **Only ONE bootstrap node should exist per network** to ensure genesis state consistency.

This guide covers setting up, operating, and securing an ADIC bootstrap node.

## Table of Contents

- [Prerequisites](#prerequisites)
- [Bootstrap Node Responsibilities](#bootstrap-node-responsibilities)
- [Setup Process](#setup-process)
- [Configuration](#configuration)
- [Deployment](#deployment)
- [Verification](#verification)
- [Security Considerations](#security-considerations)
- [Maintenance](#maintenance)
- [Transitioning to Regular Validator](#transitioning-to-regular-validator)

## Prerequisites

### System Requirements

**Minimum:**
- 8GB RAM
- 4 CPU cores
- 100GB SSD storage
- Stable internet connection with public IP
- Open ports: 8080 (API), 9000 (QUIC), 19000 (P2P)

**Recommended:**
- 16GB RAM
- 8 CPU cores
- 500GB NVMe SSD
- 100Mbps+ dedicated bandwidth
- DDoS protection
- Redundant infrastructure

### Software Requirements

- Rust 1.70+ (https://rustup.rs/)
- Git
- System dependencies:
  ```bash
  # Ubuntu/Debian
  sudo apt-get install build-essential pkg-config libssl-dev libclang-dev protobuf-compiler

  # macOS
  brew install cmake pkg-config protobuf

  # Fedora/RHEL
  sudo dnf install gcc gcc-c++ pkgconfig openssl-devel clang-devel protobuf-compiler
  ```

## Bootstrap Node Responsibilities

A bootstrap node has unique responsibilities:

### 1. Genesis Initialization
- **Creates the genesis state** for the entire network
- Applies initial token allocations
- Generates the canonical genesis hash
- Creates and saves `genesis.json` manifest

### 2. Network Anchor
- Serves as the initial connection point for all other nodes
- Provides `genesis.json` to joining nodes
- Acts as the first validator in the network

### 3. Operational Requirements
- **Must be highly available** (99.9%+ uptime recommended)
- Should have reliable connectivity
- Must maintain data integrity
- Should be monitored 24/7

## Setup Process

### Step 1: Clone and Build

```bash
# Clone the repository
git clone https://github.com/IguanAI/adic-core.git
cd adic-core

# Build release binary
cargo build --release

# Verify build
./target/release/adic --version
```

### Step 2: Generate Keys

```bash
# Generate cryptographic keypair for the bootstrap node
./target/release/adic keygen --output node.key

# Secure the key file
chmod 600 node.key
```

### Step 3: Configure Bootstrap Node

The bootstrap node uses the mainnet configuration with bootstrap mode enabled. You can either:

**Option 1: Use environment variable (recommended):**
```bash
# Set bootstrap mode via environment
export BOOTSTRAP=true
```

**Option 2: Create custom config file:**

Create `custom-bootstrap.toml` based on `config/mainnet.toml` with:

```toml
[node]
data_dir = "./data"
validator = true
name = "bootstrap1.adicl1.com"
bootstrap = true  # CRITICAL: Set to true for bootstrap node only

# All other settings inherited from mainnet config
# See config/mainnet.toml for full configuration
```

**Note:** The mainnet configuration (`config/mainnet.toml`) already contains all necessary consensus parameters, genesis allocations (300.4M ADIC), and network settings. Custom configs should only override runtime settings like ports, paths, or the bootstrap flag.

### Step 4: Start Bootstrap Node

```bash
# Start the bootstrap node using mainnet config with bootstrap mode
BOOTSTRAP=true ./target/release/adic start --network mainnet

# Or use custom config file
# ./target/release/adic start --config custom-bootstrap.toml

# For production, use systemd (recommended - see below)
```

### Step 5: Verify Genesis Creation

```bash
# Check that genesis.json was created
ls -lh ./data/genesis.json

# Verify genesis hash
cat ./data/genesis.json | jq '.hash'
# Should output: "e03dffb732c202021e35225771c033b1217b0e6241be360ad88f6d7ac43675f8"

# Check node logs for genesis application
grep "genesis" ./data/node.log
```

## Configuration

### Critical Settings

#### 1. Bootstrap Flag

```toml
[node]
bootstrap = true  # MUST be true for bootstrap node
```

⚠️ **WARNING:** Only ONE node in the network should have this set to `true`.

#### 2. Empty Bootstrap Peers

```toml
[network]
bootstrap_peers = []  # Must be empty for bootstrap node
dns_seeds = []        # Must be empty for bootstrap node
```

Bootstrap nodes don't connect to other bootstrap nodes - they ARE the bootstrap.

#### 3. Genesis Configuration

The `[genesis]` section defines the entire initial state. **Changes to this section after network launch will cause genesis hash mismatches.**

### Optional Settings

#### Network Tuning

```toml
[network]
max_peers = 100  # Higher for bootstrap nodes (they're connection hubs)
```

#### Storage Optimization

```toml
[storage]
backend = "rocksdb"
cache_size = 100000  # Larger cache for bootstrap nodes
```

## Deployment

### Production Deployment

#### Using Systemd (Recommended)

Create `/etc/systemd/system/adic-bootstrap.service`:

```ini
[Unit]
Description=ADIC Bootstrap Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=adic
Group=adic
WorkingDirectory=/home/adic/adic-core
Environment="BOOTSTRAP=true"
ExecStart=/home/adic/adic-core/target/release/adic start --network mainnet
Restart=on-failure
RestartSec=10
StandardOutput=append:/var/log/adic/bootstrap.log
StandardError=append:/var/log/adic/bootstrap-error.log

# Security hardening
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/home/adic/adic-core/data /var/log/adic
ProtectHome=read-only

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable adic-bootstrap
sudo systemctl start adic-bootstrap
sudo systemctl status adic-bootstrap
```

#### Using Docker

```bash
# Build Docker image
docker build -t adic-bootstrap -f Dockerfile .

# Run bootstrap container
docker run -d \
  --name adic-bootstrap \
  -p 8080:8080 \
  -p 9000:9000/udp \
  -p 19000:19000 \
  -v $(pwd)/data:/data \
  -e BOOTSTRAP=true \
  adic-bootstrap start --network mainnet
```

### Network Infrastructure

#### Firewall Configuration

```bash
# Allow API access (restrict to authorized IPs in production)
sudo ufw allow 8080/tcp

# Allow QUIC protocol
sudo ufw allow 9000/udp

# Allow P2P communication
sudo ufw allow 19000/tcp

# Enable firewall
sudo ufw enable
```

#### DNS Setup

Point your domain to the bootstrap node:

```
bootstrap1.adicl1.com    A       <your-server-ip>
_seeds.adicl1.com       TXT      "bootstrap1.adicl1.com:9000"
```

## Verification

### 1. Check Node Status

```bash
# Health check
curl http://localhost:8080/v1/health

# Node status
curl http://localhost:8080/v1/status | jq

# Genesis hash verification
curl http://localhost:8080/v1/economics/genesis | jq '.genesis_amount'
```

### 2. Verify Genesis File

```bash
# Check genesis.json exists
test -f ./data/genesis.json && echo "✓ Genesis file exists"

# Verify canonical hash
HASH=$(jq -r '.hash' ./data/genesis.json)
if [ "$HASH" = "e03dffb732c202021e35225771c033b1217b0e6241be360ad88f6d7ac43675f8" ]; then
  echo "✓ Canonical hash verified"
else
  echo "✗ Hash mismatch: $HASH"
fi

# Verify total supply
TOTAL=$(jq '[.config.allocations[][1]] | add' ./data/genesis.json)
if [ "$TOTAL" = "300400000" ]; then
  echo "✓ Total supply correct (300.4M ADIC)"
else
  echo "✗ Total supply mismatch: $TOTAL"
fi
```

### 3. Monitor Logs

```bash
# Watch logs in real-time
tail -f ./data/node.log | grep -E "genesis|ERROR|WARN"

# Check for successful genesis application
grep "Applying genesis allocations" ./data/node.log
grep "Genesis loaded, verified" ./data/node.log
```

## Security Considerations

### 1. Key Management

```bash
# Secure the node key
chmod 600 node.key
chown adic:adic node.key

# Consider hardware security module (HSM) for production
```

### 2. Network Security

- **Use firewall rules** to restrict API access
- **Enable rate limiting** in API configuration
- **Monitor for DDoS attacks**
- **Use TLS** for API endpoints in production:
  ```toml
  [network]
  use_production_tls = true
  ```

### 3. Access Control

- Restrict SSH access to authorized personnel only
- Use SSH key authentication (disable password auth)
- Implement 2FA for administrative access
- Audit logs regularly

### 4. Data Integrity

```bash
# Regular backups of critical data
tar -czf backup-$(date +%Y%m%d).tar.gz \
  ./data/genesis.json \
  ./node.key \
  ./config

# Store backups securely off-site
```

### 5. Monitoring

Set up monitoring for:
- Node uptime and health
- Network connectivity
- Disk space and I/O
- CPU and memory usage
- Peer connections
- API response times

Example with Prometheus:

```bash
# Scrape metrics
curl http://localhost:8080/metrics

# Configure Prometheus scraping
# See docker-compose.monitoring.yml
```

## Maintenance

### Regular Tasks

#### Daily
- Check node status and logs
- Verify peer connections
- Monitor resource usage

#### Weekly
- Review security logs
- Update system packages
- Backup critical data

#### Monthly
- Review and optimize storage
- Update node software (if available)
- Test disaster recovery procedures

### Software Updates

```bash
# Backup before updating
./backup.sh

# Pull latest code
git pull origin main

# Rebuild
cargo build --release

# Restart node
systemctl restart adic-bootstrap

# Verify node is running
curl http://localhost:8080/v1/health
```

### Storage Management

```bash
# Check storage usage
du -sh ./data/*

# Create snapshot
./scripts/create-snapshot.sh

# Clean old snapshots (keep last 10)
ls -t ./snapshots/*.tar.gz | tail -n +11 | xargs rm -f
```

## Transitioning to Regular Validator

Once the network is established, you may want to transition the bootstrap node to a regular validator:

### Option 1: Keep as Bootstrap

Continue running with `bootstrap = true` - no changes needed. The bootstrap node will function as a regular validator but can re-apply genesis if data is lost.

### Option 2: Convert to Regular Validator

**⚠️ WARNING:** Only do this if you have OTHER validators already running, and you've distributed `genesis.json` to all nodes.

1. Stop the node:
   ```bash
   systemctl stop adic-bootstrap
   ```

2. Update configuration:
   ```toml
   [node]
   bootstrap = false  # Change to false
   ```

3. Ensure `genesis.json` is present in data directory

4. Restart:
   ```bash
   systemctl start adic-bootstrap
   ```

## Troubleshooting

### Bootstrap Node Won't Start

**Check logs:**
```bash
tail -100 ./data/node.log
journalctl -u adic-bootstrap -n 100
```

**Common issues:**
- Port conflicts (8080, 9000, 19000)
- Insufficient permissions on data directory
- Invalid genesis configuration
- Missing dependencies

### Genesis Not Applied

**Symptoms:** Economics engine shows 0 balance for genesis accounts

**Solutions:**
1. Check if genesis was already applied:
   ```bash
   ls ./data/.genesis_applied
   ```

2. For fresh start, delete data directory:
   ```bash
   rm -rf ./data
   mkdir ./data
   ```

3. Restart node

### Peer Connection Issues

**Check network settings:**
```bash
# Verify ports are open
sudo netstat -tulpn | grep -E '8080|9000|19000'

# Test connectivity
curl http://localhost:8080/v1/network/status
```

## Support

For bootstrap node issues:
- Review this guide and [GENESIS.md](./GENESIS.md)
- Check [TESTNET.md](./TESTNET.md) for testnet-specific guidance
- Open an issue: https://github.com/IguanAI/adic-core/issues
- Contact: ADICL1@proton.me

---

**Version:** 0.1.8
**Last Updated:** 2025-09-30