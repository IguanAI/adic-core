# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.2] - 2025-08-25

### Added
- **Performance Benchmarks**: Comprehensive benchmark suite for consensus operations
  - Message processing, validation, and admissibility checks
  - P-adic mathematics operations (valuation, distance, ball ID)
  - Reputation system operations
  - Real-world performance measurements with Criterion.rs
  - Benchmarks now use actual storage operations for accurate metrics

- **Code Quality Improvements**
  - Added `DEFAULT_PRECISION` and `DEFAULT_P` constants to eliminate magic numbers
  - Proper constant usage throughout the codebase for p-adic operations
  - Improved code maintainability and clarity

### Changed
- **Configuration Precedence**: Corrected order to be more intuitive
  - CLI arguments now have highest priority (explicit user intent)
  - Environment variables have medium priority (deployment-specific)
  - Config file has lowest priority (project defaults)
  - This matches standard CLI tool behavior (Docker, Kubernetes, etc.)

### Fixed
- **Environment Variable Loading**: Fixed bug where env vars weren't properly overriding config
  - Environment variables from `.env` file now correctly load
  - System environment variables properly override file settings
  - Fixed precedence order in `main.rs` configuration loading

- **Benchmark Compilation**: Fixed all benchmark compilation errors
  - Updated to use correct API calls for consensus engine
  - Fixed async operations in benchmark code
  - Corrected field names (e.g., `phi` vs `axes`, `qp_digits` vs `phi`)
  - Added proper trait imports for storage operations

### Documentation
- **Performance Metrics**: Updated README with actual measured performance
  - Message processing: ~16,000 msg/s (61.5 µs latency)
  - Admissibility checks: ~980,000 ops/s (1.02 µs latency)
  - P-adic operations: 118-244M ops/s (4-8 ns latency)
  - Replaced hypothetical numbers with real benchmark data

### Testing
- All README functionality verified and working:
  - Project builds successfully with `cargo build --release`
  - All 300+ tests pass with `cargo test --all`
  - Key generation, node initialization, and startup work correctly
  - Environment variable configuration works as documented
  - HTTP API endpoints respond correctly
  - CLI test command creates test messages successfully

## [0.1.1] - 2025-01-25

### Added
- **Environment Variable Support**: Full integration of `.env` configuration with precedence system
  - Added `dotenv` dependency for automatic `.env` file loading
  - Implemented `apply_env_overrides()` method for runtime configuration
  - Docker Compose now properly uses environment variables from `.env`
  - Added comprehensive test coverage for environment variable overrides
  
- **Security Infrastructure**
  - Created comprehensive `SECURITY.md` with vulnerability reporting guidelines
  - Added `.env.example` template with secure defaults and documentation
  - Secured Docker configurations with environment-based passwords for Grafana
  - Added security warnings for Jupyter notebook deployment

- **Core Implementations**
  - **Consensus Engine**: Full implementation replacing all TODO macros
    - Complete conflict detection and resolution with energy calculations
    - Enhanced deposit manager with balance integration and slashing
    - Full reputation system with decay and updates
    - Message validation with comprehensive checks
  
  - **Finality Engine**: Complete K-core algorithm implementation
    - Artifact scheduling and processing
    - Proper K-core computation with finalization logic
    - Integration with storage backend
  
  - **Network Module**: Extensive P2P networking capabilities
    - Complete deposit verifier with Proof-of-Work challenges
    - Message codec with compression support
    - Peer management and reputation tracking
    - Protocol handlers for sync and messaging
    - Network resilience and security features
  
  - **Parent Selection**: Enhanced Multi-axis Random Walk (MRW)
    - Bootstrap mode for genesis conditions
    - Diversity requirements with fallback mechanisms
    - Trust-weighted selection with reputation integration
    - Comprehensive trace logging for debugging

- **Storage Improvements**
  - Enhanced memory storage with complete snapshot support
  - Proper tip management synchronization
  - Snapshot cleanup with configurable retention
  - Thread-safe operations with RwLock

- **API Enhancements**
  - New endpoints for economics, conflicts, and reputation
  - Parent selection trace endpoint for debugging
  - Message query with feature filtering
  - Comprehensive node statistics

- **Mathematics Module**
  - Complete p-adic valuation implementation (`vp_diff` function)
  - Fixed proximity score calculations for edge cases
  - Enhanced ball membership tests
  - Proper ultrametric distance calculations

### Changed
- **Repository Migration**
  - Updated all URLs from `github.com/adic-core/adic-core` to `github.com/IguanAI/adic-core`
  - Changed contact email to `ADICL1@proton.me`
  - Updated social links to `X.com/ADICL1Tangle`
  - Fixed citation year to 2025

- **Code Consolidation**
  - Removed all duplicate implementations (`_v2`, `_enhanced` variants)
  - Consolidated deposit managers into single implementation
  - Replaced all mock implementations with real ones
  - Unified message serialization methods

- **Configuration System**
  - Command-line arguments now have highest priority
  - Environment variables override config files
  - Added clear configuration hierarchy documentation
  - Docker services properly inherit environment settings

### Fixed
- **Test Failures**
  - Fixed p-adic mathematics test logic errors
  - Corrected parent selection algorithm for bootstrap scenarios
  - Fixed TipManager synchronization issues
  - Resolved signature verification mismatches
  - Fixed snapshot cleanup sorting logic

- **Compilation Issues**
  - Removed unused imports across all crates
  - Fixed missing trait implementations
  - Resolved circular dependency issues
  - Added proper error handling throughout

- **Node Processing**
  - Implemented complete processing loop with periodic tasks
  - Fixed finality checking logic
  - Added proper tip synchronization from storage
  - Corrected reputation decay timing

### Documentation
- **README.md**: Complete alignment with actual codebase
  - Updated architecture diagram with new crates
  - Fixed CLI commands to match implementation
  - Added economics section
  - Documented configuration options
  - Added environment variable usage examples

- **Security Documentation**
  - Comprehensive security policy with reporting process
  - Docker deployment best practices
  - Environment configuration guidelines
  - Production deployment recommendations

### Testing
- Added integration tests for node operations
- Created genesis bootstrap tests
- Added environment variable override tests
- Enhanced parent selection test coverage

### Infrastructure
- Docker Compose improvements for multi-node testing
- Monitoring setup with Prometheus and Grafana
- Python simulator Docker configuration
- Debian packaging structure (`libadic`)

## [0.1.0] - 2025-01-24

### Initial Release
- Core ADIC protocol implementation
- Basic node functionality
- P-adic mathematics foundation
- Initial storage abstraction
- Basic consensus mechanism
- CLI interface

---

*Note: This changelog summarizes the extensive development work completed since the last commit (c8d66bb "code dump"). The codebase has been substantially enhanced with full implementations replacing all placeholder TODOs, comprehensive testing, and production-ready security measures.*