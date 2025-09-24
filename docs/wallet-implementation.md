# ADIC Wallet Implementation Documentation

## Overview

The ADIC blockchain now includes a complete wallet system that manages token ownership through cryptographic keys, enforces economic requirements for network participation, and provides comprehensive APIs for wallet management.

## Architecture

### Core Components

1. **NodeWallet** (`crates/adic-node/src/wallet.rs`)
   - Manages keypairs and addresses for nodes
   - Provides persistent storage of wallet data
   - Supports deterministic key derivation from node IDs

2. **Genesis System** (`crates/adic-node/src/genesis.rs`)
   - Implements ADIC-DAG paper specifications
   - Manages initial token distribution
   - Creates genesis hyperedge with d+1 system identities

3. **Transaction API** (`crates/adic-node/src/api_wallet.rs`)
   - RESTful endpoints for wallet operations
   - Balance queries and transfers
   - Faucet for test tokens

4. **Economics Integration**
   - Unified AccountAddress type
   - Balance management with BalanceManager
   - Deposit requirements for messages

## Genesis Configuration

Following the ADIC-DAG paper specifications:

- **Total Supply**: 10^9 ADIC (max)
- **Genesis Mint**: 3×10^8 ADIC
  - Treasury: 60M ADIC (20%)
  - Liquidity/Community: 90M ADIC (30%)
  - Genesis Pool: 150M ADIC (50%)
- **Deposit Requirement**: 0.1 ADIC per message

## API Endpoints

### Wallet Information
```bash
GET /wallet/info
```
Returns the node's wallet address, public key, and node ID.

**Response:**
```json
{
  "address": "71ef6e73551963bfc2c0e0bac722718c92626aa734d2fbcb99fba2e2361a153c",
  "public_key": "71ef6e73551963bfc2c0e0bac722718c92626aa734d2fbcb99fba2e2361a153c",
  "node_id": "71ef6e73551963bf"
}
```

### Balance Query
```bash
GET /wallet/balance/{address}
```
Returns total, available, and locked balances for an address.

**Response:**
```json
{
  "address": "0100000000000000000000000000000000000000000000000000000000000000",
  "total": 60000000.0,
  "available": 60000000.0,
  "locked": 0.0
}
```

### Transfer
```bash
POST /wallet/transfer
```
Transfer ADIC between addresses (currently limited to node's own wallet).

**Request:**
```json
{
  "from": "address_hex",
  "to": "address_hex",
  "amount": 100.0,
  "signature": "signature_hex"
}
```

### Faucet
```bash
POST /wallet/faucet
```
Request test ADIC from the faucet (100 ADIC per request).

**Request:**
```json
{
  "address": "address_hex"
}
```

## Node Startup Process

1. **Wallet Loading/Creation**
   - Check for existing wallet in `{data_dir}/wallet.json`
   - If exists, load the wallet
   - If not, create new wallet with deterministic key from node ID

2. **Genesis Application**
   - Check for genesis marker file `.genesis_applied`
   - If not present, apply genesis allocations
   - Create marker file with manifest hash and timestamp

3. **Balance Initialization**
   - Load balance state from storage
   - Verify treasury and other genesis allocations

## Deposit Mechanism

Messages submitted to the network require a 0.1 ADIC deposit:

1. **Balance Check**: Verify sender has sufficient balance
2. **Deposit Escrow**: Lock 0.1 ADIC in consensus module
3. **Message Validation**: Process message through consensus
4. **Deposit Resolution**:
   - Valid messages: Deposit refunded
   - Invalid messages: Deposit slashed

## Container Deployment

For the 6-node containerized deployment:

### Environment Variables
```bash
ADIC_NODE_NAME=node-1
ADIC_DATA_DIR=/data
ADIC_API_PORT=8080
```

### Genesis Synchronization
All nodes in the container network share the same genesis configuration, ensuring consistent initial state across the network.

### Wallet Persistence
Each node maintains its wallet in the data directory, surviving container restarts:
```
/data/
  wallet.json         # Node wallet
  .genesis_applied    # Genesis marker
```

## Security Considerations

1. **Key Storage**: Currently unencrypted (TODO: Add encryption)
2. **Signature Verification**: Placeholder implementation (TODO: Full verification)
3. **Transfer Restrictions**: Currently limited to node's own wallet
4. **Deterministic Keys**: Based on SHA256 of node ID seed

## Testing

### Local Testing
```bash
# Initialize node
./adic init -o ./test-node

# Start node
./adic start --data-dir ./test-node-data --api-port 8090

# Check wallet
curl http://localhost:8090/wallet/info

# Request from faucet
curl -X POST http://localhost:8090/wallet/faucet \
  -H "Content-Type: application/json" \
  -d '{"address": "your_address"}'

# Submit message (requires deposit)
curl -X POST http://localhost:8090/submit \
  -H "Authorization: Bearer test-token" \
  -H "Content-Type: application/json" \
  -d '{"content": "Test message"}'
```

## Explorer Integration

The wallet system integrates with the ADIC Explorer through:

1. **WalletWidget Component** (`adic-explorer/frontend/src/components/WalletWidget.vue`)
   - Connect/disconnect functionality
   - Balance display
   - Transfer modal
   - Transaction history

2. **Wallet Store** (`adic-explorer/frontend/src/stores/wallet.ts`)
   - State management with Pinia
   - API integration
   - Balance polling
   - Transaction tracking

## Future Enhancements

1. **Wallet Encryption**: Add password-based encryption for stored keys
2. **Hardware Wallet Support**: Integration with Ledger/Trezor
3. **Multi-signature Wallets**: Support for multi-sig transactions
4. **Full Signature Verification**: Complete Ed25519 signature verification
5. **Transaction History API**: Comprehensive transaction query endpoints
6. **Staking Mechanisms**: Lock ADIC for validator participation
7. **Governance Integration**: Use ADIC for voting weight

## Troubleshooting

### Common Issues

1. **Genesis Not Applied**
   - Check for `.genesis_applied` marker
   - Verify treasury balance
   - Review logs for genesis errors

2. **Insufficient Balance for Deposit**
   - Request ADIC from faucet
   - Check balance with `/wallet/balance/{address}`

3. **Wallet Not Loading**
   - Verify wallet.json exists in data directory
   - Check file permissions
   - Review wallet data structure

4. **Transfer Failures**
   - Ensure sufficient balance
   - Verify address format (64 hex chars)
   - Check signature (currently placeholder)