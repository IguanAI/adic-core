# ADIC-DAG Core

<div align="center">

**A Higher-Dimensional p-Adic Ultrametric Tangle with Feeless Consensus**

[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Build Status](https://img.shields.io/github/workflow/status/IguanAI/adic-core/CI)](https://github.com/IguanAI/adic-core/actions)
[![Documentation](https://docs.rs/adic-core/badge.svg)](https://docs.rs/adic-core)

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
- **Network Gossip**: P2P message propagation with axis-aware overlays
- **Economics Integration**: ✅ **Fully Implemented** - Token accounting, balance management, and API endpoints
- **Wallet System**: ✅ **Fully Implemented** - Complete wallet with transactions, signing, and faucet

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
- Rust 1.70+ (install via [rustup](https://rustup.rs/))
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

```bash
# Generate cryptographic keys (saves to node.key by default)
./target/release/adic keygen --output node.key

# Initialize node with v1 parameters (p=3, d=3)
./target/release/adic init --params v1

# Start the node 
./target/release/adic start --data-dir ./data --api-port 8080

# In another terminal, run local test (creates test messages)
./target/release/adic test --count 10
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
| **Genesis** | §6 & App.F | d+1 system identities, anchored manifest |

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

#### Consensus & Finality
- `GET /v1/finality/:id` - Get finality artifact for a message
- `GET /v1/reputation/:pubkey` - Get reputation score
- `GET /v1/conflicts` - View active conflicts

#### Monitoring
- `GET /v1/statistics/detailed` - Detailed node statistics
- `GET /metrics` - Prometheus metrics endpoint

For authentication, rate limiting, and detailed examples, see [API.md](./API.md).

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
r_sum_min = 10.0   # Minimum total reputation

[economics]
deposit = 0.1      # ADIC anti-spam deposit (refundable)
gamma = 0.9        # Reputation decay factor
alpha = 1.0        # Reputation exponent
beta = 1.0         # Age discount exponent

[mrw]
lambda = 1.0       # Proximity weight
mu = 0.5           # Conflict penalty weight
```

## API Endpoints

### Core Operations

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/submit` | POST | Submit message with features and parents |
| `/message/:id` | GET | Retrieve message with ultrametric data |
| `/tips` | GET | Current tips with diversity metrics |
| `/finality/:id` | GET | Check F1 k-core status; F2 homology ⚠️ (heuristic) |

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
| `/reputation/:pubkey` | GET | ADIC-Rep score (non-transferable) |
| `/conflict/:id` | GET | Conflict set with energy |
| `/certificate/:id` | GET | Finality certificate with attestations |

## Testing

```bash
# Run all tests including new ultrametric modules
cargo test --all

# Test specific security components
cargo test -p adic-crypto

# Benchmark p-adic operations
cargo bench --bench math_bench

# Test network with ultrametric validation
cargo test -p adic-network test_gossip_with_diversity
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
| **Double-spend** | Energy descent with unique winner | `ConflictResolver::calculate_energy()` |
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
- [x] Homology (F2) finality
- [x] Feature commitments and proofs
- [x] Energy descent conflict resolution

### Phase 1 (Beta) 🚧
- [ ] Genesis manifest with L1 anchors
- [x] Persistent homology streaming
- [ ] ADIC-Rep SBT implementation
- [ ] Axis-aware gossip overlays

### Phase 2 (Mainnet Candidate) 📅
- [ ] PoUW sponsor hooks
- [ ] Storage markets
- [ ] Governance module (quadratic)
- [ ] Parameter optimization

## Documentation

- 📄 [**ADIC-DAG Whitepaper**](./view10ADICDAG.pdf) - Complete protocol specification
- 📖 [**API Documentation**](https://docs.rs/adic-core) - Rust API reference
- 🔬 [**Mathematical Foundations**](./docs/MATH.md) - P-adic theory deep dive
- 🛡️ [**Security Analysis**](./docs/SECURITY.md) - Attack vectors and defenses

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