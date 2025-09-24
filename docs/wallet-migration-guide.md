# Wallet System Migration Guide

## For Container Deployment

This guide helps integrate the new wallet system into the existing 6-node containerized ADIC deployment.

## Changes Required

### 1. Docker Image Update

Rebuild the Docker image to include wallet support:

```dockerfile
# Ensure wallet module is included
COPY crates/adic-node/src/wallet.rs crates/adic-node/src/
COPY crates/adic-node/src/genesis.rs crates/adic-node/src/
COPY crates/adic-node/src/api_wallet.rs crates/adic-node/src/

# Add sha2 dependency if not present
RUN cargo add sha2 --package adic-node
```

### 2. Configuration Updates

Each node's configuration should include:

```toml
[node]
name = "node-1"  # Unique for each node
data_dir = "/data"

[consensus]
deposit_amount = 0.1  # ADIC per message

[api]
wallet_endpoints = true
```

### 3. Genesis Initialization

On first startup, the lead node should initialize genesis:

```bash
# Inside container
adic init --params mainnet -o /data

# This creates:
# - /data/adic-config.toml
# - /data/node.key (if not using wallet)
```

### 4. Volume Mounts

Ensure persistent storage for wallets:

```yaml
volumes:
  - ./node-1-data:/data
```

Each node needs:
- `/data/wallet.json` - Wallet file
- `/data/.genesis_applied` - Genesis marker

### 5. Environment Variables

```yaml
environment:
  - ADIC_NODE_NAME=node-1
  - ADIC_DATA_DIR=/data
  - ADIC_API_PORT=8080
  - ADIC_WALLET_ENABLED=true
```

## Startup Sequence

### First-Time Setup

1. **Node 1 (Genesis Node)**:
   ```bash
   # Starts first, creates wallet, applies genesis
   adic start --data-dir /data --api-port 8080
   ```

2. **Nodes 2-6**:
   ```bash
   # Start after Node 1, create wallets, skip genesis
   adic start --data-dir /data --api-port 808X
   ```

### Subsequent Starts

All nodes load existing wallets and skip genesis application.

## API Testing

### Verify Wallet Creation
```bash
# From host
for i in {1..6}; do
  echo "Node $i wallet:"
  curl -s http://localhost:808$i/wallet/info | jq .address
done
```

### Check Genesis Allocation
```bash
# Treasury balance (should be 60M ADIC)
curl http://localhost:8081/wallet/balance/0100000000000000000000000000000000000000000000000000000000000000
```

### Test Faucet
```bash
# Get node 1's address
NODE1_ADDR=$(curl -s http://localhost:8081/wallet/info | jq -r .address)

# Request from faucet
curl -X POST http://localhost:8081/wallet/faucet \
  -H "Content-Type: application/json" \
  -d "{\"address\": \"$NODE1_ADDR\"}"
```

## Docker Compose Example

```yaml
version: '3.8'

services:
  node1:
    image: adic:latest
    volumes:
      - ./node1-data:/data
    environment:
      - ADIC_NODE_NAME=node-1
      - ADIC_API_PORT=8080
    ports:
      - "8081:8080"
    command: ["start", "--data-dir", "/data"]

  node2:
    image: adic:latest
    volumes:
      - ./node2-data:/data
    environment:
      - ADIC_NODE_NAME=node-2
      - ADIC_API_PORT=8080
    ports:
      - "8082:8080"
    command: ["start", "--data-dir", "/data"]
    depends_on:
      - node1

  # ... nodes 3-6 similar
```

## Monitoring

### Check Wallet Balances
```bash
#!/bin/bash
for i in {1..6}; do
  ADDR=$(curl -s http://localhost:808$i/wallet/info | jq -r .address)
  BALANCE=$(curl -s http://localhost:808$i/wallet/balance/$ADDR | jq .total)
  echo "Node $i: $BALANCE ADIC"
done
```

### Verify Deposits
```bash
# Submit message and check deposit
RESPONSE=$(curl -X POST http://localhost:8081/submit \
  -H "Authorization: Bearer test-token" \
  -H "Content-Type: application/json" \
  -d '{"content": "Test"}')

echo $RESPONSE | jq .deposit_escrowed
# Should show: 0.1
```

## Rollback Plan

If issues arise:

1. **Preserve Data**: Backup `/data` directories
2. **Revert Code**: Use previous image without wallet
3. **Clear Genesis**: Remove `.genesis_applied` markers
4. **Restart**: Bring up old version

## Troubleshooting

### Issue: Genesis Not Applied
```bash
# Check marker
ls -la /data/.genesis_applied

# Check treasury
curl http://localhost:8081/wallet/balance/0100000000000000000000000000000000000000000000000000000000000000
```

### Issue: Wallet Not Created
```bash
# Check wallet file
ls -la /data/wallet.json

# Check logs
docker logs adic-node-1 | grep wallet
```

### Issue: Insufficient Balance
```bash
# Use faucet
curl -X POST http://localhost:8081/wallet/faucet \
  -H "Content-Type: application/json" \
  -d '{"address": "node_address"}'
```

## Performance Impact

- **Memory**: +~50MB per node for wallet and balance tracking
- **Disk**: +~1KB per wallet file
- **CPU**: Minimal impact (key generation on first start)
- **Network**: Additional API endpoints, minimal overhead

## Security Notes

1. **Wallet Files**: Currently unencrypted, rely on filesystem permissions
2. **API Access**: Wallet endpoints should be behind authentication
3. **Transfer Limits**: Currently restricted to node's own wallet
4. **Faucet Rate Limiting**: Implement in production

## Next Steps

After successful integration:

1. Test message submission with deposits
2. Verify balance deductions
3. Test faucet distribution
4. Monitor genesis allocations
5. Set up wallet backup procedures