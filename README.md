# ADIC-DAG Core

<div align="center">

**A Higher-Dimensional p-Adic Ultrametric Tangle with Feeless Consensus**

[![Version](https://img.shields.io/badge/version-0.3.0-blue.svg)](https://github.com/IguanAI/adic-core/releases)
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
- **Streaming Persistent Homology**: Incremental F2 finality with O(n) amortized complexity
- **Reputation-Weighted**: Non-transferable ADIC-Rep scores weight consensus
- **Energy Descent Conflict Resolution**: Mathematically guaranteed convergence
- **Genesis Configuration System**: Canonical genesis state with cryptographic validation (300.4M ADIC supply)
- **Bootstrap Node Infrastructure**: Network initialization with genesis.json manifest creation
- **P2P Update Distribution**: Self-updating network with cryptographic verification
- **Swarm Analytics**: Real-time collective network performance monitoring

### ✨ Protocol Features (Implementation Status)

- **🔐 Privacy & Security**: ✅ **Bech32 Addresses** - User-facing adic1... format (raw public keys never exposed), Wallet v4 with XSalsa20-Poly1305 + Argon2id
- **🔐 Ultrametric Cryptography**: ⚠️ **Partially Implemented** - Hash-based ball proofs, experimental p-adic crypto
- **⚡ High Performance**: ⚠️ **Benchmarks Needed** - P-adic operations performance unverified
- **📊 Dual Finality Tests**: ✅ **K-core (F1) Complete** | ✅ **F2 Persistent Homology Complete** - Full implementation with GUDHI validation
- **🌐 Multi-Axis Diversity**: ✅ **C1-C3 Constraints Implemented**
- **🛡️ Attack Resistant**: ⚠️ **Basic Protection** - Sybil, collusion, and censorship resistance implemented
- **📈 Natural Sharding**: ⚠️ **Conceptual** - Emerges from p-adic ball topology (not production-ready)

## Implementation Status

This implementation provides a foundation for the ADIC-DAG protocol but contains several simplified components that require further development:

### ✅ Fully Implemented
- **P-adic Mathematics**: Core valuation, distance, and ball operations
- **Message Structure**: d-dimensional simplices with parent approvals
- **C1-C3 Constraint Validation**: Proximity, diversity, and reputation checking
- **K-core (F1) Finality**: Complete implementation with proper graph algorithms
- **F2 Persistent Homology**: Complete implementation with streaming support (3,603 lines)
- **GUDHI Validation Suite**: Gold-standard TDA library validation (5 topological test cases)
- **Streaming Finality Updates**: O(n) amortized incremental persistence computation
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
- **Governance System**: ✅ Quadratic voting (√Rmax), 19 parameters, treasury execution
- **Storage Market**: ✅ JITCA compilation, PoSt verification, cross-chain settlement
- **PoUW Framework**: ✅ VRF worker selection, quorum validation, dispute resolution
- **BLS Threshold Crypto**: ✅ DKG + threshold signatures for governance/PoUW receipts
- **VRF Randomness**: ✅ Commit-reveal protocol for unpredictable worker selection
- **Bech32 Addresses**: ✅ Privacy-preserving adic1... addresses (no raw public keys exposed)
- **Wallet v4**: ✅ XSalsa20-Poly1305 AEAD + Argon2id KDF (production-grade encryption)

### ⚠️ Partially Implemented / Simplified
- **Ball Membership Proofs**: Blake3 hash-based verification instead of cryptographic proofs
- **Feature Commitments**: Hash commitments without zero-knowledge properties
- **Ultrametric Cryptography**: Experimental XOR-based encryption in p-adic module

### ❌ Not Implemented
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
# Output: adic 0.3.0
```

#### Quick Start: Using Network Presets

The simplest way to start a node is using the `--network` flag with built-in network configurations:

```bash
# Start a mainnet node (production configuration)
./target/release/adic start --network mainnet

# Start a testnet node (public testing network)
./target/release/adic start --network testnet

# Start a devnet node (local development with fast finality)
./target/release/adic start --network devnet
```

**Network Configurations:**
- **`mainnet`**: Production parameters (k=20, D*=12, deposit=0.1 ADIC)
- **`testnet`**: Same consensus params as mainnet, lower reputation thresholds for testing
- **`devnet`**: Minimal thresholds (k=2, D*=2) for rapid local development

Network configurations are defined in `config/*.toml` files and include all consensus parameters, economic settings, and network topology. Runtime settings (ports, paths) can be overridden via environment variables (see `.env.example`).

**⚠️ Important:** Consensus parameters (p, d, k, rho, etc.) are immutable and defined in the network config files. Never override them via environment variables.

#### Advanced: Custom Configurations

For custom network configurations, you can use the `--config` flag:

#### Choose Your Node Type

**Bootstrap Node** (for starting a NEW network):
```bash
# Generate keypair
./target/release/adic keygen --output node.key

# Start bootstrap node for mainnet (creates genesis.json)
./target/release/adic start --network mainnet

# Or create custom bootstrap config with [node] bootstrap = true
# ./target/release/adic --config custom-bootstrap.toml start

# Bootstrap nodes initialize the network genesis state
# See BOOTSTRAP.md for complete setup guide
```

**Validator Node** (for joining an EXISTING network):
```bash
# Generate keypair
./target/release/adic keygen --output node.key

# Obtain genesis.json from bootstrap node or network
# Place it in your data directory

# Start validator node for testnet
./target/release/adic start --network testnet

# Or use mainnet
./target/release/adic start --network mainnet

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
curl http://localhost:8080/v1/health

# Get node status (includes finality metrics)
curl http://localhost:8080/v1/status

# Submit a message with features
curl -X POST http://localhost:8080/v1/messages \
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
│   ├── adic-finality/    # K-core (F1) ✅; F2 persistent homology ✅ (3,566 lines, streaming support)
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
| **Homology Finality F2** | §4.2 & App.C | ✅ Full persistent homology with GUDHI validation, streaming O(n) updates |
| **Feeless + Deposits** | §5.1 | Refundable 0.1 ADIC anti-spam |
| **ADIC Token** | §5.2 | Utility token (not for consensus weight) |
| **ADIC-Rep** | §5.3 | Non-transferable reputation scoring |
| **Genesis** | §6 & App.F | Complete state management: 300.4M ADIC supply, canonical hash validation, bootstrap/validator nodes |

## API Reference

The ADIC node exposes a comprehensive REST API for interacting with the network. See [API.md](./API.md) for complete documentation.

### Quick Reference

#### Basic Operations
- `GET /v1/health` - Health check
- `GET /v1/status` - Node status with finality metrics and capability flags
- `POST /v1/messages` - Submit a message to the DAG
- `GET /v1/messages/:id` - Retrieve a specific message
- `GET /v1/tips` - Get current DAG tips

#### Economics & Token Management
- `GET /v1/economics/supply` - Token supply metrics
- `GET /v1/economics/balance/:address` - Account balance
- `GET /v1/economics/emissions` - Emission schedule
- `GET /v1/economics/genesis` - Genesis allocation status

#### Consensus & Finality
- `GET /v1/finality/:id` - Get finality artifact for a message
- `GET /v1/reputation/:address` - Get reputation score (bech32 adic1... format)
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

Node configuration is managed via TOML files using the `--network` flag:
- `config/mainnet.toml` - Production network configuration (k=20, D*=12, 300.4M ADIC supply)
- `config/testnet.toml` - Public testing network (same consensus as mainnet, lower thresholds)
- `config/devnet.toml` - Local development (k=2, D*=2, fast finality)

For custom configurations, create your own TOML file and use `--config custom.toml`.

See [Genesis & Bootstrap](#genesis--bootstrap) section for configuration structure.

## F2 Persistent Homology Implementation

### Overview
Version 0.2.0 introduces a complete F2 topological finality implementation based on persistent homology over the field F₂. This replaces the previous heuristic approximation with a mathematically rigorous computation aligned with the ADIC-DAG paper (Appendix C).

### Module Architecture (3,566 lines)
- **simplex.rs** (401 lines): Simplicial complex data structures with filtration
- **streaming.rs** (485 lines): Incremental persistence with O(n) amortized complexity
- **f2_finality.rs** (797 lines): Dual-mode finality checker (batch vs streaming)
- **persistence.rs** (431 lines): Persistence diagram computation
- **reduction.rs** (336 lines): Boundary matrix reduction over F₂
- **bottleneck.rs** (374 lines): L∞ Wasserstein distance for diagram comparison
- **matrix.rs** (347 lines): Sparse boundary matrix operations
- **adic_complex.rs** (306 lines): ADIC message → simplicial complex conversion

### GUDHI Validation
The implementation is validated against GUDHI (Geometric Understanding in Higher Dimensions), the gold-standard TDA library:
- **5 test cases**: Simple triangle, tetrahedron, sphere S², torus, Klein bottle
- **Accuracy criteria**: Barcode accuracy ε = 10⁻⁶, Betti number exactness, bottleneck distance ε = 10⁻⁴
- **Generate references**: `cd crates/adic-finality/validation && poetry run python generate_references.py`

### Streaming vs Batch Mode
**Batch Mode** (default, backward compatible):
- Recomputes full persistent homology each round
- Complexity: O(n²-n³) per round
- Use case: Small message sets, maximum accuracy

**Streaming Mode** (v0.2.0):
- Incremental updates using vineyard-style algorithm
- Complexity: O(n) amortized per new message
- 10-1000x speedup potential for large message sets
- Bounded memory: maintains last Δ snapshots

Configuration:
```rust
F2Config {
    use_streaming: true,  // Enable streaming mode
    dimension: 3,          // Compute H₀, H₁, H₂, H₃
    radius: 5,             // Δ = 5 rounds for bottleneck comparison
    epsilon: 0.1,          // Stabilization threshold
    ...
}
```

### Finality Criteria
**F2 finality achieved when**:
1. H₃ (3-dimensional holes) is stable for Δ rounds
2. H₂ bottleneck distance < ε between consecutive rounds
3. Confidence score exceeds threshold

### Performance
- **101 tests passing** (96 core + 5 GUDHI validation)
- **Benchmarks**: `cargo bench --bench f2_finality_bench`
- **Memory**: O(n) simplices + bounded snapshots (10 by default)

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
| `/v1/messages` | POST | Submit message with features and parents |
| `/v1/messages/:id` | GET | Retrieve message with ultrametric data |
| `/v1/tips` | GET | Current tips with diversity metrics |
| `/v1/finality/:id` | GET | Check F1 k-core status; F2 persistent homology ✅ (complete) |

### Ultrametric Security

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/ball/:axis/:radius/:center` | GET | ✅ Query messages in specific p-adic ball |
| `/proofs/membership` | POST | ✅ Generate ball membership proof with p-adic features |
| `/proofs/verify` | POST | ✅ Verify ultrametric ball membership proofs |
| `/security/score/:id` | GET | ✅ Compute S(x;A) admissibility score (C1+C2+C3) |

### Consensus & Reputation

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/v1/reputation/:address` | GET | ADIC-Rep score (accepts bech32 adic1... addresses) |
| `/v1/conflict/:id` | GET | Conflict set with energy |

## Testing

```bash
# Run all tests including new ultrametric modules
cargo test --all

# Test specific security components
cargo test -p adic-crypto

# Run F2 persistent homology tests
cargo test -p adic-finality ph::

# Run GUDHI validation suite
cargo test validation_against_gudhi

# Benchmark F2 finality
cargo bench --bench f2_finality_bench

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

- **Signatures**: Ed25519 (messages), BLS threshold signatures (governance/PoUW receipts)
- **Hashing**: Blake3 for commitments and proofs
- **Key Derivation**: Argon2id (wallet v4)
- **Encryption**: XSalsa20-Poly1305 AEAD (wallet v4), experimental p-adic XOR
- **Randomness**: VRF commit-reveal protocol for unpredictable worker selection
- **DKG**: Distributed key generation for BLS threshold schemes

## Roadmap

### Phase 0: Prototype (Complete) ✅
**Goal**: Core consensus and basic functionality ([Paper §9.1](./docs/references/adic-dag-paper.pdf))

- [x] Message format, feature encoders, and p-adic ultrametric
- [x] MRW (multi-axis random walk) tip selection
- [x] C1-C3 admissibility enforcement
- [x] K-core (F1) finality with diversity thresholds
- [x] Energy descent conflict resolution
- [x] Deposits and refunds (anti-spam, feeless base)
- [x] Genesis manifest and configuration system
- [x] Bootstrap node infrastructure
- [x] Explorer and basic monitoring

### Phase 1: Beta (Nearly Complete) ✅🚧
**Goal**: Production-grade finality and reputation system ([Paper §9.2](./docs/references/adic-dag-paper.pdf))

- [x] **Persistent homology-based finality (F2)** - Streaming implementation complete (3,603 LOC in `adic-finality/src/ph/`)
- [x] **ADIC-Rep SBT** - Non-transferable reputation scoring (692 LOC, `is_transferable: false`, soul-bound to keypair)
- [x] **Axis-aware gossip overlays** - Multi-axis p-adic ball routing (`adic-network/src/protocol/axis_overlay.rs`)
- [x] **Anti-entropy checkpoints** - Merkle-root checkpoints with F1/F2 metadata (`adic-finality/src/checkpoint.rs`)
- [ ] **Optional L1 anchors** - Genesis timestamping to Ethereum/Bitcoin (hash function exists, no L1 integration yet)

**Status**: 4/5 Phase 1 deliverables complete. L1 anchoring requires ethers-rs/bitcoin-rs integration.

### Phase 2: Mainnet Candidate ✅
**Goal**: Advanced features and production hardening ([Paper §9.3](./docs/references/adic-dag-paper.pdf))

- [x] **Governance module** - Quadratic voting, 19 parameters, treasury execution
- [x] **Storage markets** - JITCA compilation, PoSt verification
- [x] **PoUW sponsor hooks** - VRF worker selection, quorum validation
- [x] **BLS threshold signatures** - DKG for governance/PoUW receipts
- [ ] **Parameter sweeps** - Empirical optimization of (ρ, q, k, D, Δ)
- [ ] **Adversarial testing** - Security audit and attack simulations

**Status**: Phase 2 core features complete (v0.3.0). Parameter optimization and security audits remain for full mainnet readiness.

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