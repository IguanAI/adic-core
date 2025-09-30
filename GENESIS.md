# ADIC Genesis System

## Overview

The ADIC Genesis System establishes the initial state of the ADIC-DAG network, including token allocations, genesis identities, and system parameters. Every ADIC node validates its genesis configuration against a canonical hash to ensure network consistency.

## Table of Contents

- [Genesis Configuration](#genesis-configuration)
- [Canonical Genesis Hash](#canonical-genesis-hash)
- [Token Allocations](#token-allocations)
- [Bootstrap vs Non-Bootstrap Nodes](#bootstrap-vs-non-bootstrap-nodes)
- [Genesis Validation](#genesis-validation)
- [Configuration Examples](#configuration-examples)
- [Troubleshooting](#troubleshooting)

## Genesis Configuration

The genesis configuration is defined in the `[genesis]` section of your node's configuration file (e.g., `bootstrap-config.toml`).

### Genesis Structure

```toml
[genesis]
deposit_amount = 0.1              # Anti-spam deposit amount
timestamp = "2025-01-01T00:00:00Z"  # Genesis timestamp
chain_id = "adic-dag-v1"          # Network chain identifier

# Genesis identities (g0, g1, g2, g3 for d=3)
genesis_identities = ["g0", "g1", "g2", "g3"]

# Token allocations [address, amount_in_ADIC]
allocations = [
    ["0100000000000000000000000000000000000000000000000000000000000000", 60_000_000],    # Treasury (20%)
    ["c1403f4763367340178077be2ab3144af2b9065901232335f960a9910bb9ab1b", 45_000_000],    # Liquidity (15%)
    ["2f89601b32149388d38652ac432307bf183eb97de87b5599cb76d256fd7a7f89", 45_000_000],    # Community (15%)
    ["98831caf9b0861ec6eba3072275efc0de1557062043d317ba5f218361e028441", 150_000_000],   # Genesis pool (50%)
    ["52ba18a771da5f8ebfb7e0eb88a229b748637c8041e0ddf06271b0511e67a5d4", 100_000],       # g0
    ["39abcb02b715f742149a698dcfd534884b8696d07d3d40afd83a0ebd5dcfa3e8", 100_000],       # g1
    ["fd2401601d44dd03cfe585782b48bf2f5681ac3264975ab43a4fea2d0089a543", 100_000],       # g2
    ["d5828a126c990608a0d385cb31dcc818fcef5bdf64a2315481f7f656d42e53af", 100_000],       # g3
]

# System parameters (aligned with ADIC-DAG paper)
[genesis.parameters]
p = 3                # Prime p
d = 3                # Dimension
rho = [2, 2, 1]      # Axis radii
q = 3                # Diversity threshold
k = 20               # k-core threshold
depth_star = 12      # Depth D*
homology_window = 5  # Δ (Delta)
alpha = 1.0          # Reputation exponent α
beta = 1.0           # Reputation exponent β
```

## Canonical Genesis Hash

**The canonical genesis hash for the ADIC mainnet is:**

```
e03dffb732c202021e35225771c033b1217b0e6241be360ad88f6d7ac43675f8
```

This hash is deterministically computed from the genesis configuration and ensures that all nodes on the network agree on the initial state. **Any node with a different genesis hash will be unable to join the ADIC network.**

### How the Hash is Calculated

The genesis hash is computed using SHA-256 over the following components:
1. Chain ID
2. Timestamp
3. All token allocations (address and amount pairs)
4. Genesis identities (g0-g3)
5. System parameters

This creates a cryptographic commitment to the entire genesis state.

## Token Allocations

The ADIC genesis mint creates a total supply of **300,400,000 ADIC tokens** (300.4M), allocated as follows:

### Mainnet Allocation Breakdown

| Category | Amount (ADIC) | Percentage | Purpose |
|----------|--------------|------------|---------|
| **Treasury** | 60,000,000 | 20% | Protocol development, governance |
| **Liquidity** | 45,000,000 | 15% | Market liquidity provision |
| **Community R&D** | 45,000,000 | 15% | Community development, research |
| **Genesis Pool** | 150,000,000 | 50% | Genesis validator rewards |
| **g0** | 100,000 | 0.033% | Genesis identity 0 |
| **g1** | 100,000 | 0.033% | Genesis identity 1 |
| **g2** | 100,000 | 0.033% | Genesis identity 2 |
| **g3** | 100,000 | 0.033% | Genesis identity 3 |
| **TOTAL** | **300,400,000** | **100%** | |

### Genesis Identities

The four genesis identities (g0, g1, g2, g3) are special addresses that correspond to the `d+1 = 4` required genesis validators for the ADIC-DAG protocol (where d=3 dimensions). These identities bootstrap the network's reputation system.

### Testnet Allocations

For testnet deployments, smaller allocations are used for testing purposes:

```toml
allocations = [
    ["0100000000000000000000000000000000000000000000000000000000000000", 10_000],   # Treasury
    ["c1403f4763367340178077be2ab3144af2b9065901232335f960a9910bb9ab1b", 1_000],    # node-1
    ["2f89601b32149388d38652ac432307bf183eb97de87b5599cb76d256fd7a7f89", 1_000],    # node-2
    ["98831caf9b0861ec6eba3072275efc0de1557062043d317ba5f218361e028441", 1_000],    # node-3
    ["52ba18a771da5f8ebfb7e0eb88a229b748637c8041e0ddf06271b0511e67a5d4", 10_000],   # faucet
]
```

**Total testnet supply: 23,000 ADIC**

## Bootstrap vs Non-Bootstrap Nodes

### Bootstrap Nodes

Bootstrap nodes are special nodes that **initialize the genesis state** for the network. They:

- Create the `genesis.json` manifest file
- Initialize token allocations in the economics engine
- Set the canonical genesis hash
- Do NOT validate against an existing genesis

**Configuration:**
```toml
[node]
bootstrap = true
```

**There should typically be only ONE bootstrap node per network** to prevent genesis state conflicts.

### Non-Bootstrap Nodes

Non-bootstrap nodes are regular validator or full nodes that:

- **Require** a `genesis.json` file to start
- Validate their genesis configuration against the canonical hash
- Reject genesis mismatches to prevent network splits
- Join an existing network

**Configuration:**
```toml
[node]
bootstrap = false  # or omit this field (defaults to false)
```

Non-bootstrap nodes must obtain the `genesis.json` file from:
1. The bootstrap node
2. Other network peers
3. A trusted source (e.g., official repository)

## Genesis Validation

### Bootstrap Node Initialization

When a bootstrap node starts:

```
1. Load genesis config from node config file
2. Apply genesis allocations to economics engine
3. Compute genesis hash
4. Create genesis.json manifest
5. Save genesis.json to data directory
6. Continue normal node operation
```

### Non-Bootstrap Node Validation

When a non-bootstrap node starts:

```
1. Check for genesis.json in data directory
   ├─ If missing → ERROR: "Non-bootstrap node requires genesis.json"
   └─ If present → Continue
2. Load genesis.json manifest
3. Verify genesis hash matches canonical hash
   ├─ If mismatch → ERROR: "Genesis hash mismatch!"
   └─ If match → Continue
4. Load genesis config
5. Continue normal node operation
```

### Genesis Manifest Structure

The `genesis.json` file contains:

```json
{
  "config": {
    "deposit_amount": 0.1,
    "timestamp": "2025-01-01T00:00:00Z",
    "chain_id": "adic-dag-v1",
    "genesis_identities": ["g0", "g1", "g2", "g3"],
    "allocations": [[address, amount], ...],
    "parameters": {
      "p": 3,
      "d": 3,
      "rho": [2, 2, 1],
      "q": 3,
      "k": 20,
      "depth_star": 12,
      "homology_window": 5,
      "alpha": 1.0,
      "beta": 1.0
    }
  },
  "hash": "e03dffb732c202021e35225771c033b1217b0e6241be360ad88f6d7ac43675f8",
  "version": "1.0.0"
}
```

## Configuration Examples

### Mainnet Bootstrap Node

```toml
[node]
data_dir = "./data"
validator = true
name = "bootstrap1.adicl1.com"
bootstrap = true  # Bootstrap node

[network]
enabled = true
p2p_port = 19000
quic_port = 9000
max_peers = 50

[genesis]
deposit_amount = 0.1
timestamp = "2025-01-01T00:00:00Z"
chain_id = "adic-dag-v1"
genesis_identities = ["g0", "g1", "g2", "g3"]

allocations = [
    ["0100000000000000000000000000000000000000000000000000000000000000", 60_000_000],
    ["c1403f4763367340178077be2ab3144af2b9065901232335f960a9910bb9ab1b", 45_000_000],
    # ... (full allocation list)
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

### Mainnet Validator Node (Non-Bootstrap)

```toml
[node]
data_dir = "./data"
validator = true
name = "validator1.adic.network"
# bootstrap = false  # Default, can be omitted

[network]
enabled = true
p2p_port = 19001
quic_port = 9001
bootstrap_peers = ["bootstrap1.adicl1.com:9000"]
dns_seeds = ["_seeds.adicl1.com"]
max_peers = 50

# Genesis section can be omitted for non-bootstrap nodes
# They will validate against genesis.json file
```

### Testnet Configuration

```toml
[node]
data_dir = "./test_data"
validator = true
name = "testnet-node-1"
bootstrap = false

[consensus]
# Relaxed parameters for testnet
q = 1
r_sum_min = 2.0
r_min = 0.5
k = 5

[network]
enabled = true
bootstrap_peers = ["testnet-bootstrap.adicl1.com:9000"]

[genesis]
chain_id = "adic-testnet"
allocations = [
    ["0100000000000000000000000000000000000000000000000000000000000000", 10_000],
    # ... (testnet allocations)
]
```

## Troubleshooting

### Error: "Non-bootstrap node requires genesis.json"

**Cause:** Your node is configured as a non-bootstrap node but cannot find the `genesis.json` file.

**Solution:**
1. Obtain `genesis.json` from the bootstrap node or trusted source
2. Place it in your data directory (e.g., `./data/genesis.json`)
3. Verify the file is valid JSON
4. Restart your node

Alternatively, if you're starting a new network, set `bootstrap = true` in your config.

### Error: "Genesis hash mismatch"

**Cause:** Your genesis configuration produces a different hash than the canonical network genesis.

**Solution:**
1. Verify you're using the correct `genesis.json` file
2. Check the canonical hash: `e03dffb732c202021e35225771c033b1217b0e6241be360ad88f6d7ac43675f8`
3. Ensure allocations match exactly (order matters)
4. Verify timestamps and chain ID are correct
5. If joining an existing network, obtain a fresh `genesis.json` from a trusted source

### Error: "Invalid genesis config"

**Cause:** The genesis configuration failed validation checks.

**Solution:**
1. Check that allocations are not empty
2. Verify all addresses are valid hex strings (64 characters)
3. Ensure amounts are positive integers
4. Verify genesis identities array has exactly 4 entries
5. Check that system parameters are valid (e.g., `p = 3`, `d = 3`)

### Genesis Not Applied

**Symptom:** Node starts but genesis allocations are not visible in economics engine.

**Solution:**
1. Check node logs for "Applying genesis allocations" message
2. Verify the genesis flag wasn't already applied (stored in data directory)
3. For a fresh start, delete the data directory and restart
4. Ensure economics engine is properly initialized

### Multiple Bootstrap Nodes

**Issue:** Multiple nodes configured with `bootstrap = true` create conflicting genesis states.

**Solution:**
1. **Only one node should be a bootstrap node**
2. All other nodes should be non-bootstrap
3. If you already have multiple bootstrap nodes:
   - Choose ONE canonical bootstrap node
   - Stop all other nodes
   - Delete their data directories
   - Copy `genesis.json` from canonical bootstrap
   - Configure them as non-bootstrap (`bootstrap = false`)
   - Restart nodes

## Further Reading

- [BOOTSTRAP.md](./BOOTSTRAP.md) - Guide for setting up bootstrap nodes
- [TESTNET.md](./TESTNET.md) - Testnet validator guide
- [API.md](./API.md) - Economics and genesis API endpoints
- [ADIC-DAG Paper](./docs/references/adic-dag-paper.pdf) - Section 6.2: Economics & Genesis

## Support

For genesis-related issues:
1. Check this troubleshooting guide
2. Review node logs for error messages
3. Verify genesis.json format and hash
4. Open an issue: https://github.com/IguanAI/adic-core/issues

---

**Version:** 0.1.8
**Last Updated:** 2025-09-30