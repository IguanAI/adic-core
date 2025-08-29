# Contributing to ADIC Core

Thank you for your interest in contributing to ADIC Core! This document provides guidelines and information for contributing to the implementation of the ADIC-DAG protocol.

## Table of Contents

- [Project Overview](#project-overview)
- [Development Setup](#development-setup)
- [Code Organization](#code-organization)
- [How to Contribute](#how-to-contribute)
- [Development Workflow](#development-workflow)
- [Testing Guidelines](#testing-guidelines)
- [Code Style & Conventions](#code-style--conventions)
- [Documentation](#documentation)
- [Security](#security)
- [Community](#community)

## Project Overview

ADIC Core implements the ADIC-DAG protocol, a feeless distributed ledger based on p-adic ultrametric mathematics. Before contributing, we recommend:

1. Reading the [whitepaper](docs/references/adic-dag-paper.pdf) to understand the mathematical foundations
2. Reviewing the [README](README.md) for implementation status
3. Understanding the core concepts:
   - P-adic number theory and ultrametric spaces
   - Higher-dimensional directed acyclic hypergraphs (d-simplices)
   - Reputation-weighted consensus mechanisms
   - K-core and persistent homology finality tests

## Development Setup

### Prerequisites

#### Required Software
- **Rust 1.70+**: Install via [rustup](https://rustup.rs/)
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Git**: Version control system

#### System Dependencies

Different operating systems require different packages for building native dependencies (RocksDB, libp2p, etc.):

**Ubuntu/Debian:**
```bash
sudo apt-get update
sudo apt-get install build-essential pkg-config libssl-dev libclang-dev protobuf-compiler cmake
```

**macOS:**
```bash
# Install Xcode Command Line Tools if not already installed
xcode-select --install

# Install dependencies via Homebrew
brew install cmake pkg-config protobuf
```

**Fedora/RHEL/CentOS:**
```bash
sudo dnf install gcc gcc-c++ pkgconfig openssl-devel clang-devel protobuf-compiler cmake
```

**Arch Linux:**
```bash
sudo pacman -S base-devel pkg-config openssl clang protobuf cmake
```

#### Optional Tools
- **Docker**: For containerized development and testing
- **Python 3.8+**: Required for running network simulations
- **cargo-tarpaulin**: For test coverage reports
  ```bash
  cargo install cargo-tarpaulin
  ```
- **cargo-watch**: For automatic rebuilds during development
  ```bash
  cargo install cargo-watch
  ```

### Building from Source

```bash
# Clone the repository
git clone https://github.com/IguanAI/adic-core.git
cd adic-core

# Build in release mode
cargo build --release

# Run all tests
cargo test --all

# Run the node with verbose output
RUST_LOG=info ./target/release/adic start
```

### Troubleshooting Build Issues

**RocksDB compilation fails:**
- Ensure you have `libclang-dev` installed
- On macOS, make sure Xcode Command Line Tools are up to date

**"error: linker `cc` not found":**
- Install build-essential (Ubuntu) or base-devel (Arch)
- On macOS, install Xcode Command Line Tools

**Protocol buffer errors:**
- Install `protobuf-compiler` package
- Verify with: `protoc --version`

### Using Make Commands

```bash
make build    # Build the project
make test     # Run all tests
make clean    # Clean build artifacts
make docker-build  # Build Docker image
```

## Code Organization

The project uses a Rust workspace with 12 specialized crates:

### Core Mathematics & Types
- `adic-types` - Core type definitions and protocol parameters
- `adic-math` - P-adic arithmetic, valuations, and ultrametric operations
- `adic-crypto` - Cryptographic primitives and experimental p-adic crypto

### Consensus & Validation
- `adic-consensus` - C1-C3 constraint validation and admissibility checking
- `adic-finality` - F1 (K-core) and F2 (homology) finality tests
- `adic-mrw` - Multi-axis random walk for tip selection

### Infrastructure
- `adic-storage` - RocksDB-based persistence layer
- `adic-network` - P2P networking with QUIC/TCP transports
- `adic-economics` - Token accounting and balance management
- `adic-node` - Main node implementation and API server

### Development Tools
- `adic-bench` - Performance benchmarks
- `simulation/` - Python-based network simulator

## How to Contribute

### Types of Contributions

We welcome various types of contributions:

1. **Code Improvements**
   - Bug fixes
   - Performance optimizations
   - New features aligned with the protocol
   - Test coverage improvements

2. **Mathematical Validation**
   - Verification of p-adic calculations
   - Homology computation improvements
   - Energy descent proofs

3. **Documentation**
   - Code documentation
   - Mathematical explanations
   - API documentation
   - Tutorial creation

4. **Research**
   - Cryptographic improvements
   - Network optimization
   - Consensus mechanism enhancements

### Getting Started

1. **Find an Issue**: Check [existing issues](https://github.com/IguanAI/adic-core/issues) or create a new one
2. **Discuss**: For significant changes, open an issue for discussion first
3. **Fork & Branch**: Create a feature branch from `develop`
4. **Implement**: Make your changes following our guidelines
5. **Test**: Ensure all tests pass and add new ones as needed
6. **Submit**: Create a pull request with a clear description

## Development Workflow

### Branch Strategy

- `main` - Stable releases only
- `develop` - Active development branch
- `feature/*` - New features
- `fix/*` - Bug fixes
- `research/*` - Experimental work

### Pull Request Process

1. **Target Branch**: PRs should target `develop` unless fixing a critical bug
2. **Description**: Provide a clear description of changes
3. **Testing**: Include test results and coverage reports
4. **Review**: Address reviewer feedback promptly
5. **Squash**: Consider squashing commits for cleaner history

### Commit Message Format

Follow the [Conventional Commits](https://www.conventionalcommits.org/) specification:

```
type(scope): brief description

Longer explanation if needed

Fixes #issue_number
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`

## Testing Guidelines

### Running Tests

```bash
# Run all tests
cargo test --all

# Run specific crate tests
cargo test --package adic-consensus

# Run with coverage
cargo tarpaulin --all --out Html

# Run integration tests
cargo test --test '*'

# Run benchmarks
cargo bench
```

### Test Categories

1. **Unit Tests**: In-module tests for individual functions
2. **Integration Tests**: Cross-crate functionality (`tests/` directories)
3. **Property-Based Tests**: Using `proptest` for mathematical properties
4. **Network Tests**: Multi-peer scenarios and failure modes
5. **Benchmarks**: Performance regression tests

### Coverage Requirements

- Aim for >70% code coverage
- Critical paths (consensus, finality) should have >90% coverage
- New features must include comprehensive tests

## Code Style & Conventions

### Rust Guidelines

- Follow standard Rust conventions (use `cargo fmt` and `cargo clippy`)
- Prefer explicit error handling over `unwrap()`
- Use `async/await` for asynchronous operations
- Document public APIs with doc comments

### Code Principles

```rust
// GOOD: Clear, explicit error handling
pub async fn validate_message(msg: &AdicMessage) -> Result<ValidationResult> {
    let features = msg.features.clone();
    let validation = self.check_constraints(&features).await?;
    Ok(validation)
}

// AVOID: Unnecessary comments and unwraps
pub async fn validate_message(msg: &AdicMessage) -> ValidationResult {
    // Get the features from the message
    let features = msg.features.clone();
    // Check the constraints
    let validation = self.check_constraints(&features).await.unwrap();
    validation // Return the validation
}
```

### Project-Specific Conventions

- P-adic operations use `QpDigits` type consistently
- Network messages use protobuf serialization
- Async operations use `tokio` runtime
- Storage operations return `Result<T, AdicError>`

## Documentation

### Code Documentation

- Document all public APIs
- Include mathematical formulas where relevant
- Add examples for complex functions

```rust
/// Calculates the p-adic valuation difference between two numbers.
/// 
/// For p-adic numbers x and y, returns v_p(x - y) where v_p is the p-adic valuation.
/// 
/// # Mathematical Background
/// 
/// The p-adic valuation v_p(n) is the highest power of p that divides n.
/// 
/// # Example
/// 
/// ```
/// let x = QpDigits::from_u64(9, 3, 5);
/// let y = QpDigits::from_u64(3, 3, 5);
/// let diff = vp_diff(&x, &y);
/// assert_eq!(diff, 1); // Since 9 - 3 = 6 = 2 * 3^1
/// ```
pub fn vp_diff(x: &QpDigits, y: &QpDigits) -> i32 {
    // Implementation
}
```

### External Documentation

- Update README.md for significant features
- Add entries to CHANGELOG.md following Keep a Changelog format
- Create examples in `examples/` for new functionality

## Security

### Reporting Vulnerabilities

Security issues should be reported privately:

1. **Email**: ADICL1@proton.me with subject "SECURITY: [description]"
2. **GitHub Security Advisories**: Use private vulnerability reporting
3. See [SECURITY.md](SECURITY.md) for detailed guidelines

### Security Considerations

- Never commit private keys or sensitive data
- Be cautious with cryptographic implementations
- Validate all external inputs
- Follow the principle of least privilege
- Consider timing attacks in consensus code

## Community

### Communication Channels

- **Issues**: Bug reports and feature requests
- **Discussions**: GitHub Discussions for general topics
- **Email**: ADICL1@proton.me for private communications

### Code of Conduct

We are committed to providing a welcoming and inclusive environment. Please:

- Be respectful and constructive
- Focus on technical merit
- Help others learn and grow
- Report inappropriate behavior

### Recognition

Contributors will be recognized in:
- Release notes
- CONTRIBUTORS file
- Project documentation

## Getting Help

If you need help:

1. Check existing documentation
2. Search closed issues
3. Ask in GitHub Discussions
4. Contact the maintainers

## License

By contributing, you agree that your contributions will be licensed under the MIT License.

---

Thank you for contributing to ADIC Core! Your efforts help advance the implementation of novel distributed ledger technology based on p-adic mathematics.