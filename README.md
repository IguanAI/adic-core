# ADIC Core

<div align="center">

**A Production-Ready P-adic DAG Consensus Protocol**

[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Build Status](https://img.shields.io/github/workflow/status/IguanAI/adic-core/CI)](https://github.com/IguanAI/adic-core/actions)
[![Documentation](https://docs.rs/adic-core/badge.svg)](https://docs.rs/adic-core)

*Revolutionizing distributed consensus through p-adic ultrametric mathematics*

[**Documentation**](#documentation) • [**Quick Start**](#quick-start) • [**Architecture**](#architecture) • [**Contributing**](#contributing)

</div>

---

## Overview

ADIC Core is a groundbreaking Rust implementation of the **Adaptive Distributed Information Consensus (ADIC)** protocol. It leverages p-adic number theory to create a novel ultrametric space for distributed ledger consensus, offering unprecedented scalability and mathematical rigor.

### 🚀 Key Innovations

- **P-adic Mathematics**: First production use of p-adic numbers in distributed systems
- **Ultrametric Consensus**: Natural sharding through hierarchical ball structures  
- **Multi-axis Random Walk**: Advanced parent selection with trust-based weighting
- **K-core Finality**: Mathematically robust finality guarantees
- **Feeless Design**: Refundable deposit system eliminates transaction fees
- **Adaptive Topology**: Self-organizing network structure

### ✨ Features

- **🔐 Cryptographically Secure**: Ed25519 signatures with Blake3 hashing
- **⚡ High Performance**: Optimized Rust implementation with async architecture
- **📊 Observable**: Comprehensive metrics and monitoring with Prometheus
- **🌐 Network Agnostic**: Pluggable networking with libp2p and QUIC support
- **🛡️ Byzantine Fault Tolerant**: Handles malicious actors through reputation systems
- **📈 Scalable**: Natural sharding through p-adic ball topology

## Quick Start

### Prerequisites

- Rust 1.70+ 
- Git

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
# Generate cryptographic keys (saves to keypair.json)
./target/release/adic keygen

# Initialize node configuration  
./target/release/adic init

# Start the node (with command-line arguments)
./target/release/adic start --data-dir ./data --api-port 8080

# Or configure via environment variables (copy .env.example to .env first)
cp .env.example .env
# Edit .env with your settings, then:
./target/release/adic start

# In another terminal, run a local test (creates test messages)
./target/release/adic test --count 10
```

#### Configuration Options

The node can be configured via:
1. Command-line arguments (highest priority)
2. Environment variables (from `.env` file or system)
3. Configuration file (`adic-config.toml`)
4. Default values

See `.env.example` for all available environment variables.

### Verify Installation

```bash
# Check node health
curl http://localhost:8080/health

# Get node status
curl http://localhost:8080/status

# Submit a test message
curl -X POST http://localhost:8080/submit \
  -H "Content-Type: application/json" \
  -d '{"content": "Hello, ADIC!"}'
```

## Architecture

ADIC Core is built as a modular, production-ready system:

```
adic-core/
├── crates/
│   ├── adic-types/       # Core data structures and type definitions
│   ├── adic-math/        # P-adic mathematics and ultrametric calculations
│   ├── adic-crypto/      # Ed25519 signatures and Blake3 hashing
│   ├── adic-consensus/   # Consensus engine with C1-C3 constraints
│   ├── adic-mrw/         # Multi-axis Random Walk parent selection
│   ├── adic-finality/    # K-core finality algorithm implementation
│   ├── adic-storage/     # Storage abstraction with RocksDB backend
│   ├── adic-network/     # P2P networking, protocols, and resilience
│   ├── adic-economics/   # Tokenomics and economic model implementation
│   ├── adic-bench/       # Performance benchmarking suite
│   └── adic-node/        # Full node implementation with CLI and API
└── docs/                 # Technical documentation
```

### Core Components

| Component | Purpose | Technology |
|-----------|---------|------------|
| **adic-node** | Full node binary with CLI and HTTP API | Rust + Axum |
| **adic-consensus** | Consensus engine with admissibility checks | Rust + async |
| **adic-math** | P-adic mathematics and ball calculations | Rust |
| **adic-storage** | Persistent storage with snapshots | RocksDB + In-memory |
| **adic-network** | P2P networking and protocol handling | Rust + libp2p |
| **adic-economics** | Tokenomics, balances, and emissions | Rust |
| **adic-mrw** | Multi-axis random walk parent selection | Rust |
| **adic-finality** | K-core finality algorithm | Rust |

## Protocol Concepts

### P-adic Ultrametric Space

The protocol organizes messages in a p-adic ultrametric space where:

- **Distance** satisfies the strong triangle inequality: `d(x,z) ≤ max(d(x,y), d(y,z))`
- **Messages** are grouped into hierarchical balls based on proximity
- **Sharding** emerges naturally from the mathematical structure

### Consensus Mechanism

#### Admissibility Constraints

Every message must satisfy three mathematical constraints:

1. **C1 (Proximity)**: Parents must be sufficiently close in p-adic distance
2. **C2 (Diversity)**: Parents must span distinct balls across multiple axes  
3. **C3 (Reputation)**: Parents must have adequate combined reputation scores

#### Multi-axis Random Walk (MRW)

Parent selection uses a sophisticated trust-weighted random walk:

```rust
// Trust function with logarithmic scaling
trust(node) = log(1 + reputation(node))

// Weight calculation per axis
weight = exp(λ × proximity + β × trust - μ × conflict_penalty)
```

#### K-core Finality

Messages achieve irreversible finality when they form a k-core subgraph:

- Each node has degree ≥ k within the finalized set
- Sufficient depth from current tips
- Meets all reputation and diversity requirements

### Economics Model

ADIC implements a comprehensive tokenomics system with:

#### Supply Management
- **Genesis Supply**: 1 billion ADIC tokens
- **Max Supply**: 2 billion ADIC tokens (reached asymptotically)
- **Emission Schedule**: Logarithmic decay over time
- **Treasury**: Protocol-controlled reserves for ecosystem development
- **Liquidity Pool**: Automated market making reserves

#### Token Distribution
- **Genesis Allocation**: Initial distribution to early participants
- **Emissions**: Block rewards following decay curve
- **Treasury**: 10% of genesis for protocol development
- **Liquidity**: 5% of genesis for market stability

#### Deposit System
- **Anti-spam Deposits**: Refundable deposits for message submission
- **Slashing**: Malicious actors lose deposits
- **Reputation Integration**: Higher reputation reduces deposit requirements

## CLI Reference

### Node Operations

```bash
# Start a validator node
adic start --validator --data-dir ./validator-data --port 9000 --api-port 8080

# Start with custom configuration
adic start --config ./custom-adic.toml

# Enable debug logging
RUST_LOG=debug adic start

# Enable trace logging for maximum verbosity
adic start -vv  # Or use RUST_LOG=trace
```

### Key Management

```bash
# Generate new keypair (outputs to stdout or file)
adic keygen
adic keygen --output ./keys/node.key
```

### Testing & Development

```bash
# Run local test (creates test messages locally)
adic test --count 100

# Benchmark message processing
cargo bench --bench consensus_bench

# Run all tests
cargo test --workspace
```

## HTTP API

### Core Endpoints

| Endpoint | Method | Description |
|----------|---------|-------------|
| `/health` | GET | Node health status |
| `/status` | GET | Detailed node statistics |
| `/submit` | POST | Submit new message |
| `/message/:id` | GET | Retrieve message by ID |
| `/tips` | GET | Current DAG tips |
| `/v1/finality/:id` | GET | Check finality status |
| `/v1/reputation/all` | GET | All reputation scores |
| `/v1/reputation/:pubkey` | GET | Specific reputation score |
| `/v1/conflicts` | GET | List all conflicts |
| `/v1/conflict/:id` | GET | Conflict details |
| `/v1/mrw/traces` | GET | MRW selection traces |
| `/v1/mrw/trace/:id` | GET | Specific MRW trace |
| `/v1/economics/deposits` | GET | Deposits summary |
| `/v1/economics/deposit/:id` | GET | Deposit status |
| `/v1/statistics/detailed` | GET | Detailed statistics |
| `/metrics` | GET | Prometheus metrics |

### Example API Usage

```bash
# Submit a message
curl -X POST http://localhost:8080/submit \
  -H "Content-Type: application/json" \
  -d '{
    "content": "Transaction data"
  }'

# Response
{
  "message_id": "a1b2c3d4..."
}

# Get message details
curl http://localhost:8080/message/a1b2c3d4...

# Check node status
curl http://localhost:8080/status
```

## Configuration

### Default Parameters

The protocol uses carefully calibrated parameters for Phase-0:

```toml
[consensus]
p = 3              # Prime base for p-adic system
d = 3              # Degree (number of parents - 1)  
rho = [2, 2, 1]    # Proximity thresholds per axis
q = 3              # Minimum distinct balls per axis
k = 20             # K-core degree requirement
depth_star = 12    # Minimum depth for finality
r_min = 1.0        # Minimum individual reputation
r_sum_min = 4.0    # Minimum total reputation
deposit = 0.1      # Anti-spam deposit amount

[mrw]
lambda = 1.0       # Proximity weight
beta = 0.5         # Trust weight  
mu = 1.0           # Conflict penalty weight
```

### Network Configuration

```toml
[network]
p2p_port = 9000
max_peers = 50
bootstrap_peers = [
  "/ip4/192.168.1.10/tcp/9000/p2p/QmBootstrap1...",
  "/ip4/192.168.1.11/tcp/9000/p2p/QmBootstrap2..."
]

[api]
host = "127.0.0.1"
port = 8080
cors_origins = ["http://localhost:3000"]
```

## Development

### Building from Source

```bash
# Debug build with all features
cargo build --all-features

# Optimized release build
cargo build --release --all-features

# Build specific component
cargo build -p adic-consensus
```

### Testing

```bash
# Run all tests
cargo test --all

# Run with code coverage
cargo tarpaulin --all

# Run property-based tests
cargo test --test property_tests

# Benchmark performance
cargo criterion
```

### Contributing

We welcome contributions! Please see our [Contributing Guidelines](CONTRIBUTING.md).

#### Development Workflow

1. **Fork** the repository
2. **Create** a feature branch
3. **Write** tests for new functionality
4. **Ensure** all tests pass
5. **Submit** a pull request

#### Code Standards

- Follow Rust naming conventions
- Add comprehensive documentation
- Maintain test coverage above 80%
- Use `rustfmt` and `clippy` for code quality

## Monitoring & Observability

### Metrics

ADIC Core exposes Prometheus metrics:

```bash
# View metrics
curl http://localhost:8080/metrics

# Key metrics include:
# - adic_messages_total
# - adic_finality_latency  
# - adic_consensus_failures
# - adic_reputation_scores
```

### Logging

Structured JSON logging with configurable levels:

```bash
# Enable debug logging
export RUST_LOG=adic_core=debug

# Log to file
export RUST_LOG=info
adic start 2>&1 | tee node.log
```

## Performance

### Benchmarks

| Operation | Latency (median) | Throughput | Description |
|-----------|------------------|------------|-------------|
| Message Processing | 61.5 µs | ~16,000 msg/s | Full validation + storage |
| Message Validation | 41.0 µs | ~24,000 ops/s | Signature & structure validation |
| Admissibility Check | 1.02 µs | ~980,000 ops/s | Parent relationship validation |
| P-adic Valuation | 4.1 ns | ~244M ops/s | Core math operation |
| P-adic Distance | 5.5 ns | ~182M ops/s | Ultrametric distance calculation |
| Ball ID Computation | 8.5 ns | ~118M ops/s | Hierarchical clustering |

*Benchmarked using Criterion.rs on Ubuntu Linux. Run `cargo bench` for detailed results.*

## Production Deployment

### Docker

```bash
# Build image
docker build -t adic-core:latest .

# Run container
docker run -d \
  --name adic-node \
  -p 8080:8080 \
  -p 9000:9000 \
  -v ./data:/app/data \
  adic-core:latest
```

### Kubernetes

```yaml
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: adic-node
spec:
  serviceName: adic-node
  replicas: 3
  template:
    spec:
      containers:
      - name: adic-node
        image: adic-core:latest
        ports:
        - containerPort: 8080
        - containerPort: 9000
        volumeMounts:
        - name: data
          mountPath: /app/data
```

## Security

### Cryptographic Security

- **Ed25519** signatures for message authentication
- **Blake3** hashing for integrity verification  
- **Secure random** number generation
- **Key rotation** support

### Network Security

- **Transport encryption** via libp2p Noise protocol
- **Peer authentication** and reputation tracking
- **Rate limiting** and DoS protection
- **Objective fault** detection and slashing

## Documentation

- 📖 [**Protocol Specification**](./DESIGN.md) - Complete Phase-0 design
- 🛠️ [**API Documentation**](https://docs.rs/adic-core) - Rust API docs
- 🎯 [**Integration Guide**](./docs/INTEGRATION.md) - Building on ADIC
- 📊 [**Benchmarks**](./crates/adic-bench/README.md) - Performance analysis

## Community & Support

- 💬 **Discussions**: [GitHub Discussions](https://github.com/IguanAI/adic-core/discussions)
- 🐛 **Issues**: [Bug Reports](https://github.com/IguanAI/adic-core/issues)
- 📧 **Email**: ADICL1@proton.me
- 🐦 **X (Twitter)**: [@ADICL1Tangle](https://x.com/ADICL1Tangle)

## Roadmap

### Phase 0 (Current) ✅
- [x] Core p-adic mathematics implementation
- [x] Basic consensus with C1-C3 constraints  
- [x] K-core finality mechanism
- [x] Multi-axis random walk
- [x] HTTP API and CLI

### Phase 1 (Q2 2025)
- [ ] Full P2P networking with libp2p
- [ ] Advanced finality gates (F2, SSF)
- [ ] Web-based explorer interface
- [ ] Enhanced monitoring and alerting

### Phase 2 (Q3 2025)  
- [ ] Smart contract execution layer
- [ ] Cross-chain bridges
- [ ] Governance mechanisms
- [ ] Mobile wallet support

## License

This project is licensed under the MIT License - see the [LICENSE](./LICENSE) file for details.

## Citation

If you use ADIC Core in your research, please cite:

```bibtex
@misc{adic2025,
  title={ADIC: Adaptive Distributed Information Consensus via P-adic Ultrametrics},
  author={ADIC Core Team},
  year={2025},
  howpublished={\url{https://github.com/IguanAI/adic-core}}
}
```

---

<div align="center">

**Built with ❤️ by the ADIC Core Team**

*Advancing the frontiers of distributed consensus through mathematical innovation*

</div>
