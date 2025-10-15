# ADIC Testnet Validator Guide

Welcome to the ADIC testnet! This guide will help you set up and run a validator node on the ADIC test network.

## 🚀 Quick Start (Recommended)

Join the ADIC testnet as a validator with a single command:

```bash
curl -sSf https://raw.githubusercontent.com/IguanAI/adic-core/main/scripts/join-testnet-docker.sh | bash
```

This script will:
1. Check for Docker prerequisites
2. Clone and compile the ADIC node from source
3. Configure your node as a validator
4. Start your node with auto-updates enabled
5. Connect to the testnet via DNS seeds

## 📋 Prerequisites

### System Requirements

**Minimum (Testnet)**:
- Docker and Docker Compose installed
- 4GB RAM
- 2 CPU cores
- 20GB available disk space
- Stable internet connection
- Ports 8080, 9000, 9001 available

**Recommended**:
- 8GB RAM
- 4 CPU cores
- 50GB SSD storage
- Dedicated server or VPS
- Public IP address (for better connectivity)

### Required Ports

Make sure these ports are open in your firewall:
- `8080/tcp` - HTTP API
- `9000/tcp` - P2P communication
- `9001/udp` - QUIC protocol

## 🔧 Manual Setup

If you prefer to set up manually:

### Step 1: Clone the Repository

```bash
git clone https://github.com/IguanAI/adic-core.git
cd adic-core
```

### Step 2: Start with Docker Compose

```bash
docker-compose -f docker-compose.testnet.yml up -d
```

### Step 3: Verify Node is Running

```bash
# Check node health
curl http://localhost:8080/v1/health

# View logs
docker-compose -f docker-compose.testnet.yml logs -f
```

## 🎯 What is a Validator?

As a validator on the ADIC testnet, your node will:

- **Participate in Consensus**: Help validate and approve messages in the network
- **Maintain the Tangle**: Store and propagate messages across the network
- **Earn Test Tokens**: Receive test ADIC tokens for validation work (testnet only)
- **Support Network Security**: Contribute to the network's ultrametric security model
- **Auto-Update**: Automatically update to new versions (v0.3.0+)

### Validator Responsibilities

1. **Uptime**: Keep your validator online as much as possible
2. **Resources**: Ensure adequate CPU, memory, and bandwidth
3. **Security**: Keep your server secure and updated
4. **Monitoring**: Watch for issues and respond to alerts

## 📊 Monitoring Your Validator

### Basic Health Checks

```bash
# Check if node is healthy
curl http://localhost:8080/v1/health

# Get node status
curl http://localhost:8080/v1/status | jq

# Check peer connections
curl http://localhost:8080/v1/network/peers | jq '.peers | length'

# View recent messages
curl http://localhost:8080/v1/messages | jq
```

### Docker Commands

```bash
# View logs
docker-compose -f docker-compose.testnet.yml logs -f

# Stop the node
docker-compose -f docker-compose.testnet.yml down

# Restart the node
docker-compose -f docker-compose.testnet.yml restart

# Update and restart
docker-compose -f docker-compose.testnet.yml pull
docker-compose -f docker-compose.testnet.yml up -d
```

### Advanced Monitoring (Optional)

Enable Prometheus and Grafana monitoring:

```bash
# Start with monitoring stack
docker-compose -f docker-compose.testnet.yml --profile monitoring up -d

# Access dashboards
# Prometheus: http://localhost:9090
# Grafana: http://localhost:3000 (user: admin, password: admin)
```

## 🔄 Auto-Updates

Your testnet node is configured with auto-updates enabled by default. The node will:

1. Check for updates via DNS at `_version.adic.network.adicl1.com`
2. Download new versions from peers when available
3. Verify cryptographic signatures
4. Perform safe hot-reload without losing state

To disable auto-updates, edit `testnet-config.toml`:
```toml
[network]
auto_update = false  # Set to false to disable
```

## 🌐 Network Configuration

Your validator connects to the testnet through:

- **DNS Seeds**: Automatic peer discovery via `_seeds.adicl1.com`
- **Bootstrap Peers**: Initial nodes to connect to (discovered via DNS)
- **P2P Network**: Gossip protocol for message propagation
- **QUIC Transport**: Fast, reliable communication protocol

### Configuration File

The main configuration is in `testnet-config.toml`:

```toml
[node]
validator = true  # Run as validator
data_dir = "./data/testnet"
bootstrap = false  # Regular validator (not bootstrap node)

[network]
dns_seeds = ["_seeds.adicl1.com"]
auto_update = true
max_peers = 50

[consensus]
# Testnet parameters
r_sum_min = 2.0  # Lower threshold for testnet
r_min = 0.5      # Lower threshold for testnet
deposit = 0.1    # Test ADIC deposit amount

[genesis]
chain_id = "adic-testnet"
timestamp = "2025-01-01T00:00:00Z"
deposit_amount = 0.1
genesis_identities = ["g0", "g1", "g2", "g3"]

# Testnet allocations (23,000 ADIC total)
allocations = [
    ["0100000000000000000000000000000000000000000000000000000000000000", 10_000],  # Treasury
    ["c1403f4763367340178077be2ab3144af2b9065901232335f960a9910bb9ab1b", 1_000],   # node-1
    ["2f89601b32149388d38652ac432307bf183eb97de87b5599cb76d256fd7a7f89", 1_000],   # node-2
    ["98831caf9b0861ec6eba3072275efc0de1557062043d317ba5f218361e028441", 1_000],   # node-3
    ["52ba18a771da5f8ebfb7e0eb88a229b748637c8041e0ddf06271b0511e67a5d4", 10_000],  # faucet
]

[genesis.parameters]
p = 3
d = 3
rho = [2, 2, 1]
q = 3
k = 20
depth_star = 12
homology_window = 5
alpha = 1.0
beta = 1.0
```

## 🔐 Genesis Configuration

ADIC uses a genesis system to initialize the network state. Understanding genesis is important for validators.

### What is Genesis?

The genesis configuration establishes:
- **Initial token allocations** - Distribution of test ADIC tokens
- **Network parameters** - Consensus thresholds and protocol settings
- **Genesis hash** - Cryptographic commitment to the initial state

### Bootstrap vs Validator Nodes

**Bootstrap Node** (`bootstrap = true`):
- Creates the genesis.json manifest
- Initializes the network state
- **Only ONE** bootstrap node should exist per network
- Operated by the network administrators

**Validator Node** (`bootstrap = false`):
- Validates against existing genesis.json
- Joins an existing network
- **This is what you run** as a testnet participant

### Genesis.json File

Your validator node requires a `genesis.json` file to start. This file:
- Contains the canonical genesis configuration
- Must match the network's genesis hash
- Is automatically downloaded from bootstrap peers

**Location**: `./data/testnet/genesis.json`

The Docker setup automatically obtains this file from the testnet bootstrap nodes. If you're running manually, the node will fetch it from connected peers during the initial sync.

### Testnet Token Allocation

The testnet has a total supply of **23,000 test ADIC tokens**:

| Account | Allocation | Purpose |
|---------|------------|---------|
| Treasury | 10,000 | Protocol development |
| Node-1 | 1,000 | Test validator |
| Node-2 | 1,000 | Test validator |
| Node-3 | 1,000 | Test validator |
| Faucet | 10,000 | Distribution to participants |

### Getting Test Tokens

Request test tokens from the faucet:

```bash
curl -X POST http://localhost:8080/wallet/faucet \
  -H "Content-Type: application/json" \
  -d '{
    "address": "YOUR_ADDRESS",
    "amount": 100.0
  }'
```

### Genesis Validation

On startup, your node will:
1. Check for `genesis.json` in the data directory
2. Compute the genesis hash from the configuration
3. Verify it matches the network's canonical hash
4. Reject connection if there's a mismatch (prevents network splits)

**Testnet Genesis Hash**: Check your logs for the genesis hash verification message.

### For More Details

See [GENESIS.md](./GENESIS.md) and [BOOTSTRAP.md](./BOOTSTRAP.md) for comprehensive documentation on the genesis system.

## 🛠️ Troubleshooting

### Node Won't Start

1. Check Docker is running:
   ```bash
   docker ps
   ```

2. Check logs for errors:
   ```bash
   docker-compose -f docker-compose.testnet.yml logs --tail 50
   ```

3. Ensure ports are available:
   ```bash
   netstat -tulpn | grep -E '8080|9000|9001'
   ```

### No Peers Connected

1. Check DNS resolution:
   ```bash
   dig TXT _seeds.adicl1.com
   ```

2. Check firewall allows outbound connections

3. Verify network configuration:
   ```bash
   curl http://localhost:8080/v1/network/status
   ```

### High Memory Usage

The RocksDB backend may use available memory for caching. To limit usage, adjust in `testnet-config.toml`:

```toml
[storage]
cache_size = 67108864  # 64MB cache (reduce if needed)
```

### Build Failures

If Docker build fails:

1. Ensure you have enough disk space
2. Try with more memory: `docker-compose --compatibility up`
3. Clean and rebuild:
   ```bash
   docker-compose -f docker-compose.testnet.yml down -v
   docker system prune -a
   docker-compose -f docker-compose.testnet.yml up -d --build
   ```

### Genesis-Related Issues

#### Error: "Non-bootstrap node requires genesis.json"

**Cause**: Your node cannot find the genesis.json file.

**Solution**:
1. Ensure you're connected to bootstrap peers (check DNS seeds)
2. Wait for the node to download genesis.json from peers
3. If manual setup, obtain genesis.json from a trusted source and place it in `./data/testnet/genesis.json`
4. Restart the node

#### Error: "Genesis hash mismatch"

**Cause**: Your genesis configuration doesn't match the network's canonical genesis.

**Solution**:
1. Delete your data directory: `rm -rf ./data/testnet`
2. Ensure your `testnet-config.toml` has the correct genesis configuration
3. Let the node download a fresh genesis.json from bootstrap peers
4. If the problem persists, you may have modified the genesis configuration - restore from the repository

#### Genesis Not Applied

**Symptom**: Node starts but economics endpoints show 0 balance for all accounts.

**Solution**:
1. Check logs for "Genesis loaded, verified" or "Applying genesis allocations" messages
2. Verify genesis.json exists: `ls -lh ./data/testnet/genesis.json`
3. For a fresh start: `rm -rf ./data/testnet && restart node`
4. Check genesis initialization endpoint:
   ```bash
   curl http://localhost:8080/v1/economics/genesis | jq '.'
   ```

## 📚 Additional Resources

- **GitHub Repository**: https://github.com/IguanAI/adic-core
- **Main Website**: https://adicl1.com
- **Explorer**: https://adicl1.com/explorer
- **API Documentation**: [API.md](./API.md)
- **Architecture Guide**: [DESIGN.md](./DESIGN.md)

## 🤝 Getting Help

If you encounter issues:

1. Check this troubleshooting guide
2. Search existing issues on [GitHub](https://github.com/IguanAI/adic-core/issues)
3. Join our community channels (coming soon)
4. Open a new issue with:
   - Your setup details (OS, Docker version)
   - Error messages from logs
   - Steps to reproduce the issue

## 🔐 Security Considerations

While this is a testnet:

1. **Don't use production keys**: This is a test network
2. **Keep Docker updated**: Regular security updates are important
3. **Monitor your node**: Watch for unusual behavior
4. **Report issues**: Help us identify and fix problems

## 🎉 Next Steps

Once your validator is running:

1. **Monitor Performance**: Use the API endpoints to track your node's performance
2. **Stay Updated**: Watch for announcements about testnet events
3. **Provide Feedback**: Report bugs and suggest improvements
4. **Experiment**: Try sending messages, exploring the API, and testing features

Thank you for participating in the ADIC testnet! Your contribution helps us build a more robust and decentralized network.

---

*For mainnet participation (when available), additional requirements and staking will apply.*