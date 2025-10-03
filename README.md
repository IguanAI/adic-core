# ADIC-DAG Core

<div align="center">

**A Higher-Dimensional p-Adic Ultrametric Tangle with Feeless Consensus**

[![Version](https://img.shields.io/badge/version-0.1.9-blue.svg)](https://github.com/IguanAI/adic-core/releases)
[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Build Status](https://github.com/IguanAI/adic-core/actions/workflows/ci.yml/badge.svg)](https://github.com/IguanAI/adic-core/actions)

*Implementing the ADIC-DAG protocol: feeless, reputation-weighted consensus via p-adic ultrametrics*

⚠️ **IMPLEMENTATION STATUS**: This is a research implementation with simplified cryptography and heuristic algorithms. Not production-ready.

[**Whitepaper**](./docs/references/adic-dag-paper.pdf) • [**Implementation Status**](#implementation-status) • [**Quick Start**](#quick-start) • [**Architecture**](#architecture) • [**Contributing**](#contributing)

</div>

---

## Overview

ADIC-DAG Core is a Rust implementation of the **ADIC-DAG protocol** as specified in the whitepaper. It realizes a feeless, reputation-weighted distributed ledger that organizes messages as a higher-dimensional directed acyclic hypergraph (a Tangle of d-simplices) with ultrametric security guarantees.

### 🚀 Key Innovations

- **P-adic Ultrametric Security**: Multi-axis diversity enforcement via p-adic ball membership
- **Higher-Dimensional Tangle**: Messages form d-simplices with d+1 parent approvals
- **Feeless Consensus**: Refundable anti-spam deposits replace transaction fees
- **Deterministic Finality**: K-core coverage and persistent homology stabilization
- **Reputation-Weighted**: Non-transferable ADIC-Rep scores weight consensus
- **Energy Descent Conflict Resolution**: Mathematically guaranteed convergence
- **Genesis Configuration System**: Canonical genesis state with cryptographic validation (300.4M ADIC supply)
- **Bootstrap Node Infrastructure**: Network initialization with genesis.json manifest creation
- **P2P Update Distribution**: Self-updating network with cryptographic verification
- **Swarm Analytics**: Real-time collective network performance monitoring

### ✨ Protocol Features (Implementation Status)

- **🔐 Ultrametric Cryptography**: ⚠️ **Partially Implemented** - Hash-based ball proofs, experimental p-adic crypto
- **⚡ High Performance**: ⚠️ **Benchmarks Needed** - P-adic operations performance unverified
- **📊 Dual Finality Tests**: ✅ **K-core (F1) Complete** | ⚠️ **F2 Homology Heuristic** - Simplified implementation
- **🌐 Multi-Axis Diversity**: ✅ **C1-C3 Constraints Implemented**
- **🛡️ Attack Resistant**: ⚠️ **Basic Protection** - Sybil, collusion, and censorship resistance implemented
- **📈 Natural Sharding**: ⚠️ **Conceptual** - Emerges from p-adic ball topology (not production-ready)

## Implementation Status

> **📋 For detailed integration status with the Explorer Backend, see [INTEGRATION-STATUS.md](./INTEGRATION-STATUS.md)**

This implementation provides a foundation for the ADIC-DAG protocol but contains several simplified components that require further development:

### ✅ Fully Implemented
- **P-adic Mathematics**: Core valuation, distance, and ball operations
- **Message Structure**: d-dimensional simplices with parent approvals
- **C1-C3 Constraint Validation**: Proximity, diversity, and reputation checking
- **K-core (F1) Finality**: Complete implementation with proper graph algorithms
- **Multi-axis Random Walk**: Tip selection with ultrametric weighting
- **Deposit System**: Anti-spam deposits with refund mechanism
- **Network Gossip**: P2P message propagation (axis-aware overlays planned)
- **Economics Integration**: ✅ Token accounting, balance management, and API endpoints
- **Wallet System**: ✅ Complete wallet with transactions, signing, and faucet
- **Genesis Configuration System**: ✅ Complete genesis state management with 300.4M ADIC supply allocation
- **Bootstrap Node Support**: ✅ Network initialization, genesis.json creation, and validation
- **Genesis Hash Validation**: ✅ Canonical hash verification prevents network splits
- **P2P Update System**: ✅ Distributed binary updates with cryptographic verification
- **Swarm Analytics**: ✅ Network-wide performance monitoring
- **Version Management**: ✅ Automatic version syncing via CARGO_PKG_VERSION

### ⚠️ Partially Implemented / Simplified
- **F2 Homology Finality**: Heuristic approximation, not full persistent homology pipeline
- **Ball Membership Proofs**: Blake3 hash-based verification instead of cryptographic proofs
- **Feature Commitments**: Hash commitments without zero-knowledge properties
- **API Security Endpoints**: Return placeholder values instead of real computations
- **Ultrametric Cryptography**: Experimental XOR-based encryption in p-adic module

### ❌ Not Implemented
- **Threshold Signatures**: Planned but not implemented
- **Zero-Knowledge Proofs**: Hash-based placeholders instead of real ZK systems
- **Proximity Encryption**: Only experimental implementation exists
- **Performance Verification**: Benchmark results not published

### 🔬 Research/Experimental
- **P-adic Cryptography**: Experimental module with XOR-based keystream
- **Energy Descent**: Mathematical framework without cryptographic proofs
- **Natural Sharding**: Conceptual design based on ball topology

## Quick Start

### Prerequisites

#### Required
- Rust 1.70+ recommended (install via [rustup](https://rustup.rs/))
  - **Note**: No MSRV (Minimum Supported Rust Version) is enforced in Cargo.toml
- Git

#### System Dependencies
- **Ubuntu/Debian**: 
  ```bash
  sudo apt-get update
  sudo apt-get install build-essential pkg-config libssl-dev libclang-dev protobuf-compiler
  ```
- **macOS**: 
  ```bash
  brew install cmake pkg-config protobuf
  ```
- **Fedora/RHEL**:
  ```bash
  sudo dnf install gcc gcc-c++ pkgconfig openssl-devel clang-devel protobuf-compiler
  ```

#### Optional
- Docker (for containerized deployment)
- Python 3.8+ (for running simulations)

### Installation

```bash
# Clone the repository
git clone https://github.com/IguanAI/adic-core.git
cd adic-core

# Build the project
cargo build --release

# Run tests to verify installation
cargo test --all
```

### Running Your First Node

#### Check Version
```bash
# Verify installation
./target/release/adic --version
# Output: adic 0.1.8
```

#### Choose Your Node Type

**Bootstrap Node** (for starting a NEW network):
```bash
# Generate keypair
./target/release/adic keygen --output node.key

# Edit bootstrap-config.toml: set [node] bootstrap = true

# Start bootstrap node with config file (creates genesis.json)
./target/release/adic --config bootstrap-config.toml start

# Bootstrap nodes initialize the network genesis state
# See BOOTSTRAP.md for complete setup guide
```

**Validator Node** (for joining an EXISTING network):
```bash
# Generate keypair
./target/release/adic keygen --output node.key

# Obtain genesis.json from bootstrap node or network
# Place it in your data directory

# Edit testnet-config.toml: ensure [node] bootstrap = false (or omit)

# Start validator node with config file
./target/release/adic --config testnet-config.toml start

# Node validates genesis.json against canonical hash
# See TESTNET.md for joining the testnet
```

#### Local Testing
```bash
# Run local test (creates test messages)
./target/release/adic test --count 10

# Check for updates
./target/release/adic update check

# Monitor swarm update statistics
curl http://localhost:8080/update/swarm
```

> **Note**: For testnet participation, see [TESTNET.md](./TESTNET.md). For bootstrap node setup, see [BOOTSTRAP.md](./BOOTSTRAP.md).

### Network & Firewall Configuration

#### Required Ports

ADIC nodes require the following ports to be accessible for proper network operation:

| Port | Protocol | Purpose | Required For |
|------|----------|---------|--------------|
| 8080 | HTTP/WS | REST API, WebSocket & SSE streaming | External access |
| 9000 | TCP | P2P gossip & message sync | Peer communication |
| 9001 | QUIC/UDP | Fast P2P transport | Peer communication |

#### Firewall Configuration

**Ubuntu/Debian (ufw):**
```bash
sudo ufw allow 8080/tcp  # API & WebSocket
sudo ufw allow 9000/tcp  # P2P TCP
sudo ufw allow 9001/udp  # QUIC transport
```

**CentOS/RHEL (firewalld):**
```bash
sudo firewall-cmd --permanent --add-port=8080/tcp
sudo firewall-cmd --permanent --add-port=9000/tcp
sudo firewall-cmd --permanent --add-port=9001/udp
sudo firewall-cmd --reload
```

**Docker:**
Ports are automatically mapped in `docker-compose.yml`

#### Custom Port Configuration

```bash
# Start with custom ports
./target/release/adic start \
  --api-port 8081 \
  --port 9002 \
  --quic-port 9003
```

Or configure in your config file:
```toml
[api]
port = 8081

[network]
p2p_port = 9002
quic_port = 9003
```

### Verify Installation

```bash
# Check node health
curl http://localhost:8080/health

# Get node status (includes finality metrics)
curl http://localhost:8080/status

# Submit a message with features
curl -X POST http://localhost:8080/submit \
  -H "Content-Type: application/json" \
  -d '{
    "content": "Hello, ADIC-DAG!",
    "features": {
      "axes": [
        {"axis": 0, "value": 42},
        {"axis": 1, "value": 100}, 
        {"axis": 2, "value": 7}
      ]
    }
  }'
```

## Genesis & Bootstrap

### What is Genesis?

The **Genesis System** establishes the initial state of the ADIC-DAG network, including:
- **Token Allocations**: Initial distribution of 300,400,000 ADIC tokens
- **Genesis Identities**: Four system identities (g0, g1, g2, g3) for protocol bootstrapping
- **System Parameters**: Core consensus parameters (p=3, d=3, ρ=(2,2,1), q=3, k=20, D*=12)
- **Canonical Hash**: Cryptographic commitment to the entire genesis state

Every ADIC node validates its genesis configuration against the **canonical genesis hash** to ensure network consistency and prevent accidental network splits.

### Canonical Genesis Hash

```
e03dffb732c202021e35225771c033b1217b0e6241be360ad88f6d7ac43675f8
```

This hash is deterministically computed from the genesis configuration and must match on all nodes in the ADIC network.

### Token Allocation (300.4M ADIC)

| Category | Amount | Percentage | Purpose |
|----------|--------|------------|---------|
| **Treasury** | 60,000,000 ADIC | 20% | Protocol development, governance |
| **Liquidity** | 45,000,000 ADIC | 15% | Market liquidity provision |
| **Community R&D** | 45,000,000 ADIC | 15% | Community development, research |
| **Genesis Pool** | 150,000,000 ADIC | 50% | Genesis validator rewards |
| **g0, g1, g2, g3** | 100,000 ADIC each (400,000 total) | 0.13% total | Genesis identities |

### Bootstrap vs Validator Nodes

#### Bootstrap Node
- **Purpose**: Initializes the genesis state for a NEW network
- **Configuration**: `bootstrap = true` in config file
- **Behavior**:
  - Creates `genesis.json` manifest on first start
  - Applies genesis token allocations to economics engine
  - Does not validate against existing genesis (it creates it)
- **Count**: **Only ONE bootstrap node per network**
- **Setup Guide**: See [BOOTSTRAP.md](./BOOTSTRAP.md)

#### Validator Node (Non-Bootstrap)
- **Purpose**: Joins an EXISTING network
- **Configuration**: `bootstrap = false` (or omit, defaults to false)
- **Behavior**:
  - Requires `genesis.json` file in data directory
  - Validates genesis hash against canonical hash
  - Rejects mismatched genesis to prevent network splits
- **Count**: Unlimited validators can join the network
- **Setup Guide**: See [TESTNET.md](./TESTNET.md)

### Genesis Configuration Example

```toml
[node]
bootstrap = false  # true only for bootstrap node
data_dir = "./data"
validator = true

[genesis]
deposit_amount = 0.1
timestamp = "2025-01-01T00:00:00Z"
chain_id = "adic-dag-v1"
genesis_identities = ["g0", "g1", "g2", "g3"]

# Token allocations [address, amount_in_ADIC]
allocations = [
    ["0100000000000000000000000000000000000000000000000000000000000000", 60_000_000],
    # ... (see GENESIS.md for complete allocation list)
]

[genesis.parameters]
p = 3                # Prime for p-adic system
d = 3                # Dimension
rho = [2, 2, 1]      # Axis radii
q = 3                # Diversity threshold
k = 20               # k-core threshold
depth_star = 12      # Minimum depth for F1 finality
homology_window = 5  # Delta for F2
alpha = 1.0          # Reputation exponent
beta = 1.0           # Age discount exponent
```

### Genesis Documentation

For complete genesis system documentation, including:
- Detailed configuration structure
- Token allocation breakdown
- Genesis hash calculation
- Bootstrap node setup procedures
- Troubleshooting common issues

See:
- **[GENESIS.md](./GENESIS.md)** - Complete genesis system guide (300+ lines)
- **[BOOTSTRAP.md](./BOOTSTRAP.md)** - Bootstrap node deployment guide (400+ lines)

## Architecture

ADIC-DAG Core implements the complete protocol specification:

```
adic-core/
├── crates/
│   ├── adic-types/       # Core types (MessageId, AdicFeatures, QpDigits)
│   ├── adic-math/        # P-adic valuation, distance, ball operations
│   ├── adic-crypto/      # ENHANCED: Ultrametric security modules
│   │   ├── ultrametric_security.rs  # C1-C3 enforcement, ball proofs
│   │   ├── feature_crypto.rs        # Feature commitments, ZK proofs
│   │   ├── consensus_crypto.rs      # Reputation-weighted validation
│   │   ├── security_params.rs       # V1 defaults (p=3, d=3, ρ=[2,2,1])
│   │   └── padic_crypto.rs          # Ultrametric key operations
│   ├── adic-consensus/   # Admissibility checker, energy descent
│   ├── adic-mrw/         # Multi-axis random walk tip selection
│   ├── adic-finality/    # K-core (F1) finality ✅; F2 homology ⚠️ (heuristic implementation)
│   ├── adic-storage/     # Persistent hypergraph storage
│   ├── adic-network/     # P2P gossip with axis-aware overlays
│   ├── adic-economics/   # ADIC token and ADIC-Rep reputation
│   └── adic-node/        # Full node with Genesis support
└── docs/                 # Protocol documentation
```

### Core Components Aligned with Whitepaper

| Component | Whitepaper Section | Implementation |
|-----------|-------------------|----------------|
| **P-adic Features** | §3.1 | `QpDigits` with axes Φ(x) ∈ Q_p^d |
| **Admissibility C1-C3** | §3.2 | `UltrametricValidator` enforces constraints |
| **MRW Tip Selection** | §3.3 | Trust-weighted with proximity scoring |
| **Energy Descent** | §4.1 | `ConflictResolver` with support calculation |
| **K-core Finality F1** | §4.2 | Requires k≥20, q≥3 distinct balls, depth≥12 |
| **Homology Finality F2** | §4.2 & App.C | ⚠️ Heuristic stabilization (not full persistent homology) |
| **Feeless + Deposits** | §5.1 | Refundable 0.1 ADIC anti-spam |
| **ADIC Token** | §5.2 | Utility token (not for consensus weight) |
| **ADIC-Rep** | §5.3 | Non-transferable reputation scoring |
| **Genesis** | §6 & App.F | Complete state management: 300.4M ADIC supply, canonical hash validation, bootstrap/validator nodes |

## API Reference

The ADIC node exposes a comprehensive REST API for interacting with the network. See [API.md](./API.md) for complete documentation.

### Quick Reference

#### Basic Operations
- `GET /health` - Health check
- `GET /status` - Node status with finality metrics and capability flags
- `POST /submit` - Submit a message to the DAG
- `GET /message/:id` - Retrieve a specific message
- `GET /tips` - Get current DAG tips

#### Economics & Token Management
- `GET /v1/economics/supply` - Token supply metrics
- `GET /v1/economics/balance/:address` - Account balance
- `GET /v1/economics/emissions` - Emission schedule
- `GET /v1/economics/genesis` - Genesis allocation status

#### Consensus & Finality
- `GET /v1/finality/:id` - Get finality artifact for a message
- `GET /v1/reputation/:pubkey` - Get reputation score
- `GET /v1/conflicts` - View active conflicts

#### Monitoring
- `GET /v1/statistics/detailed` - Detailed node statistics
- `GET /metrics` - Prometheus metrics endpoint

For authentication, rate limiting, and detailed examples, see [API.md](./API.md).

## CLI Commands

The `adic` binary provides a comprehensive command-line interface for node management.

### Top-Level Options
```bash
adic [OPTIONS] <COMMAND>

Options:
  -c, --config <FILE>  Configuration file path
  -v, --verbose...     Verbosity level (can be repeated)
  -h, --help           Print help
  -V, --version        Print version
```

### Commands

#### `adic start` - Start the ADIC node
```bash
adic [--config <FILE>] start [OPTIONS]

Options:
  -d, --data-dir <DATA_DIR>    Data directory [default: ./data]
  -p, --port <PORT>            P2P port [default: 9000]
      --quic-port <QUIC_PORT>  QUIC port [default: 9001]
      --api-port <API_PORT>    HTTP API port [default: 8080]
      --validator              Enable validator mode
```

#### `adic keygen` - Generate a new keypair
```bash
adic keygen [--output <FILE>]
```

#### `adic init` - Initialize a new node configuration
```bash
adic init [OPTIONS]

Options:
  -o, --output <OUTPUT>  Output directory for configuration [default: .]
      --params <PARAMS>  Parameter preset to use (v1, v2, testnet, mainnet)
```

#### `adic test` - Create and submit test messages
```bash
adic test [--count <COUNT>]  # Default: 10 messages
```

#### `adic wallet` - Wallet management
```bash
adic wallet <COMMAND>

Commands:
  export  Export wallet to a file
  import  Import wallet from a file
  info    Show wallet information
```

#### `adic update` - Manage node updates
```bash
adic update <COMMAND>

Commands:
  check     Check for available updates
  download  Download the latest update
  apply     Apply a downloaded update (triggers copyover)
  status    Show current update status
```

### Configuration Files

Node configuration is managed via TOML files. Example configs are provided:
- `bootstrap-config.toml` - Bootstrap node configuration
- `testnet-config.toml` - Testnet validator configuration
- `adic-config.toml` - Mainnet configuration
- `config/node{1,2,3}-config.toml` - Multi-node test configurations

See [Genesis & Bootstrap](#genesis--bootstrap) section for configuration structure.

## Protocol Implementation

### Ultrametric Security (PARTIAL IMPLEMENTATION)

⚠️ **Note**: The cryptography module provides a foundation but uses simplified implementations:

#### C1: Proximity Constraint
```rust
// Each parent must be in correct p-adic ball
vp(φ_j(x) - φ_j(a_j)) ≥ ρ_j  ∀j
```

#### C2: Diversity Constraint  
```rust
// Parents must span q distinct balls per axis
#{B^(p)(φ_j(a_k), ρ_j) : k=0,...,d} ≥ q  ∀j
```

#### C3: Reputation Constraint
```rust
// Adequate combined reputation
Σ R(a_k) ≥ R_min, min_k R(a_k) ≥ r_min
```

### Security Score Formula (§7.1)

```rust
S(x; A) = Σ_j min_a∈A p^{-max{0, ρ_j - vp(φ_j(x) - φ_j(a))}}
```

Requires S(x; A) ≥ d for admissibility.

### Feature Encodings with Cryptographic Binding

| Axis | Encoding | Proof Type |
|------|----------|------------|
| Time | Bucket b = ⌊t/τ⌋ as p-adic | Timestamp proof |
| Topic | LSH hash to Q_p | Commitment proof |
| Stake/Service | Tier quantization | Range proof |

### Consensus Security Features (Implementation Status)

- **Ball Membership Proofs**: ⚠️ **Hash-based implementation** - Blake3 hash verification, not cryptographic proofs
- **Feature Commitments**: ⚠️ **Hash commitments** - Not full zero-knowledge proofs
- **Threshold Signatures**: ❌ **Not implemented** - Planned feature
- **Proximity Encryption**: ⚠️ **Experimental XOR-based** - P-adic crypto module (not production-ready)
- **Energy Verification**: ⚠️ **Basic calculation** - Mathematical verification without cryptographic proofs

## Default Parameters (v1)

From whitepaper Section 1.2:

```toml
[consensus]
p = 3              # Prime for p-adic system
d = 3              # Dimension (4 parents per message)
rho = [2, 2, 1]    # Axis radii for ultrametric balls
q = 3              # Diversity threshold (distinct balls)
k = 20             # K-core degree for finality
depth_star = 12    # Minimum depth for F1
delta = 5          # Homology window for F2
r_min = 1.0        # Minimum individual reputation
r_sum_min = 4.0    # Minimum total reputation

[economics]
deposit = 0.1      # ADIC anti-spam deposit (refundable)
gamma = 0.9        # Reputation decay factor
alpha = 1.0        # Reputation exponent
beta = 0.5         # Age discount exponent

[mrw]
lambda = 1.0       # Proximity weight
mu = 1.0           # Conflict penalty weight
```

## API Endpoints

### Core Operations

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/submit` | POST | Submit message with features and parents |
| `/message/:id` | GET | Retrieve message with ultrametric data |
| `/tips` | GET | Current tips with diversity metrics |
| `/v1/finality/:id` | GET | Check F1 k-core status; F2 homology ⚠️ (heuristic) |

### Ultrametric Security

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/ball/:axis/:radius` | GET | ⚠️ Messages in specific p-adic ball (placeholder implementation) |
| `/proof/membership` | POST | ⚠️ Generate ball membership proof (returns placeholder) |
| `/proof/verify` | POST | ⚠️ Verify ultrametric proof (returns placeholder) |
| `/security/score/:id` | GET | ⚠️ S(x;A) admissibility score (computed with empty features) |

### Consensus & Reputation

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/v1/reputation/:pubkey` | GET | ADIC-Rep score (non-transferable) |
| `/v1/conflict/:id` | GET | Conflict set with energy |

## Testing

```bash
# Run all tests including new ultrametric modules
cargo test --all

# Test specific security components
cargo test -p adic-crypto

# Benchmark consensus operations
cargo bench --bench consensus_bench

# Test network layer
cargo test -p adic-network
```

## Performance

⚠️ **Note**: Performance claims require verification through comprehensive benchmarking.

### Ultrametric Operations (BENCHMARK NEEDED)

| Operation | Status | Notes |
|-----------|---------|-------|
| P-adic Valuation | ⚠️ Unverified | Benchmarks exist but results not published |
| Ball Membership | ⚠️ Unverified | Performance testing required |
| Security Score S(x;A) | ⚠️ Unverified | Full admissibility score calculation |
| Feature Commitment | ⚠️ Hash-based | Not full cryptographic commitment |
| Ball Proof Generation | ⚠️ Placeholder | Needs real cryptographic implementation |
| Proximity Encryption | ⚠️ Experimental | XOR-based implementation only |

## Security Guarantees

### Attack Resistance (§8)

| Attack Type | Defense Mechanism | Implementation |
|-------------|------------------|----------------|
| **Sybil** | Refundable deposits + axis diversity | `UltrametricValidator::verify_c2_diversity()` |
| **Collusion** | Multi-ball coverage requirement | Enforced q≥3 distinct balls |
| **Double-spend** | Energy descent with unique winner | `ConflictResolver` energy calculation |
| **Censorship** | MRW across axes prevents capture | Multi-axis tip selection |
| **Feature Manipulation** | Cryptographic commitments | `FeatureCommitment::commit()` |

### Cryptographic Primitives

- **Signatures**: Ed25519 with reputation weighting
- **Hashing**: Blake3 for commitments and proofs
- **Key Exchange**: X25519 with ultrametric constraints
- **Encryption**: AES-256-GCM with proximity keys

## Roadmap

### Phase 0 (Complete) ✅
- [x] P-adic mathematics (vp, distance, balls)
- [x] Ultrametric security modules
- [x] C1-C3 admissibility enforcement
- [x] K-core (F1) finality
- [x] Homology (F2) finality (heuristic implementation)
- [x] Feature commitments and proofs
- [x] Energy descent conflict resolution
- [x] Genesis manifest and configuration system
- [x] Bootstrap node infrastructure
- [x] P2P update distribution system
- [x] Wallet system with transactions

### Phase 1 (Beta) 🚧
- [ ] Genesis L1 anchoring (Ethereum/Bitcoin)
- [x] Persistent homology streaming (heuristic implementation)
- [ ] ADIC-Rep SBT implementation
- [ ] Axis-aware gossip overlays

### Phase 2 (Mainnet Candidate) 📅
- [ ] PoUW sponsor hooks
- [ ] Storage markets
- [ ] Governance module (quadratic)
- [ ] Parameter optimization

## Documentation

### Core Documentation
- 📄 [**ADIC-DAG Whitepaper**](./docs/references/adic-dag-paper.pdf) - Complete protocol specification
- 📖 [**API Documentation**](./API.md) - Complete REST API reference
- 🧬 [**Genesis System**](./GENESIS.md) - Genesis configuration and validation guide
- 🚀 [**Bootstrap Node Setup**](./BOOTSTRAP.md) - Bootstrap node deployment guide
- 🌐 [**Testnet Guide**](./TESTNET.md) - Join the ADIC testnet as a validator
- 🛡️ [**Security Policy**](./SECURITY.md) - Vulnerability reporting and security

### Technical Documentation
- 💾 [**Update System**](./docs/UPDATE-SYSTEM.md) - P2P update distribution
- 💰 [**Wallet API**](./docs/wallet-api.md) - Wallet integration guide
- 📊 [**Logging Best Practices**](./docs/LOGGING_BEST_PRACTICES.md) - Structured logging guide
- 📈 [**Integration Status**](./INTEGRATION-STATUS.md) - Explorer backend integration

## Genesis Contribution

The Genesis phase begins when contributions are sent to:

- **BTC**: `bc1qnykv3t8fqpar7aguaas3sxtlsqyndxrpa0g7h8`
- **ETH**: `0x7EB0c7ea79D85d2A3Ac45aF6A8CB0F7AC9A125bE`
- **SOL**: `GrUy83AAsibyrcUtpAVA8VgpnQSgyCAb1d8Je8MXNGLJ`

*See whitepaper Appendix F for details.*

## License

This project is licensed under the MIT License - see the [LICENSE](./LICENSE) file.

## Citation

```bibtex
@article{adic2025,
  title={ADIC-DAG: A Higher-Dimensional p-Adic Ultrametric Tangle with Feeless Consensus},
  author={Reid, Samuel},
  journal={arXiv preprint arXiv:submit/6721482},
  year={2025}
}
```

---

<div align="center">

**Built with mathematical rigor and cryptographic security**

*ADIC-DAG: Unifying p-adic ultrametrics, higher-dimensional tangles, and topological finality*

</div>