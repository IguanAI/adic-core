# ADIC Core v0.3.0 - Phase 2 Complete

**Release Date:** October 8, 2025
**Release Type:** Major Feature Release
**Status:** Mainnet Candidate (Testnet Recommended)

---

## 🎉 Phase 2 Completion

This release marks a **major milestone** in the ADIC-DAG protocol development with the completion of Phase 2. All advanced protocol features from the whitepaper are now implemented, bringing ADIC to mainnet candidate status.

### What's New

#### 7 New Application-Layer Crates

**Governance System (`adic-governance`)**
- On-chain governance with quadratic voting (√Rmax reputation weighting)
- 19 tunable protocol parameters
- Treasury execution with multi-signature security
- Complete proposal lifecycle (creation → voting → execution)
- Governance metrics and monitoring

**Storage Market (`adic-storage-market`)**
- JITCA (Just-In-Time Computation Attestation) framework
- PoSt (Proof of Space-Time) verification
- Cross-chain settlement support
- Storage deal lifecycle management
- Challenge/dispute resolution system

**Proof of Useful Work (`adic-pouw`)**
- VRF-based deterministic worker selection
- Quorum validation with Byzantine fault tolerance
- Support for multiple task types (ZK proofs, ML inference, data transformation)
- Complete task lifecycle (submission → execution → validation → settlement)
- PoUW receipts with BLS threshold signatures

**VRF Service (`adic-vrf`)**
- Commit-reveal protocol for canonical randomness
- Epoch-based randomness generation
- Prevents manipulation of committee selection

**Quorum Selection (`adic-quorum`)**
- Multi-axis committee selection (ADICPoUW2.pdf §3.2)
- Diversity caps: max per ASN, max per region, max per p-adic ball
- Reputation-weighted eligibility
- Deterministic VRF-based selection

**Challenge Framework (`adic-challenges`)**
- Shared challenge/dispute infrastructure
- Common challenge types for storage and PoUW
- Slashing conditions for failed challenges

**Application Common (`adic-app-common`)**
- Shared primitives for all application-layer features
- Application message integration with DAG consensus
- Common metrics and telemetry

#### Production-Grade Cryptography

**BLS12-381 Threshold Signatures**
- Battle-tested threshold crypto (used by Ethereum 2.0, Zcash, Filecoin)
- 128-bit security level
- Byzantine fault tolerant thresholds (default: t = ⌈2n/3⌉)
- Domain separation for governance, PoUW, and receipts
- Signature aggregation via Lagrange interpolation

**Distributed Key Generation (DKG)**
- Feldman VSS-based trustless key generation
- No trusted dealer required
- Verifiable secret sharing
- Multi-phase ceremony with public verification

**Enhanced Documentation**
- Complete threshold crypto design guide (`docs/THRESHOLD_CRYPTO_DESIGN.md`)
- Governance system specification
- Storage market architecture documentation

---

## 🔐 Security Improvements

### Removed Insecure Code
- **Removed:** Experimental XOR threshold crypto (`UltrametricKeyDerivation::combine_threshold_keys()`)
- **Why:** Provided zero security against collusion
- **Replaced with:** Production BLS12-381 threshold signatures

### New Security Features
- VRF-based unpredictable randomness prevents committee manipulation
- Diversity caps provide Sybil resistance (ASN/region/ball quotas)
- BLS threshold signatures ensure cryptographic security of receipts
- Clear separation of concerns: ultrametric diversity at quorum layer, crypto at BLS layer

---

## 🔄 Breaking Changes

### Removed API
- `UltrametricKeyDerivation::combine_threshold_keys()` removed from `adic-crypto/padic_crypto.rs`

### Migration Path
Use BLS threshold signatures instead:
```rust
use adic_crypto::bls::{generate_threshold_keys, BLSThresholdSigner};

// Old (removed)
// let combined = ukd.combine_threshold_keys(&keys, &positions, threshold)?;

// New (production-ready)
let config = ThresholdConfig::with_bft_threshold(5)?;
let (pk_set, shares) = generate_threshold_keys(5, 3)?;
let signer = BLSThresholdSigner::new(config);
// ... sign and aggregate
```

See `docs/THRESHOLD_CRYPTO_DESIGN.md` for complete migration guide.

---

## 📦 Upgrade Instructions

### For Node Operators

```bash
# 1. Stop your node
systemctl stop adic-node

# 2. Backup your data
tar -czf adic-backup-$(date +%s).tar.gz /data/adic

# 3. Download v0.3.0
wget https://github.com/IguanAI/adic-core/releases/download/v0.3.0/adic-node-linux-amd64.tar.gz
tar -xzf adic-node-linux-amd64.tar.gz
sudo mv adic-node /usr/local/bin/

# 4. Verify version
adic-node --version
# Output: adic 0.3.0

# 5. Restart (no database migration required)
systemctl start adic-node
```

**No configuration changes required** - all existing configs are backward compatible.

### For Developers

```bash
# Update Cargo.toml dependencies
git pull origin main
cargo update
cargo build --release

# All tests should pass
cargo test --workspace
```

---

## ⚠️ Known Limitations

**Testnet Status**
- This release is recommended for **testnet use only**
- Production mainnet requires:
  - Parameter optimization (ρ, q, k, D*, Δ)
  - Comprehensive security audit
  - Extended adversarial testing

**Feature Flags (Development Only)**
Some advanced features have development placeholders (disabled by default):
- `dev-placeholders` - Simplified implementations for testing
- `ml-inference` - Experimental ML task support
- `allow-weak-validation` - Proof-based validation without re-execution (PoUW)
- `allow-deterministic-challenges` - Deterministic fallback when VRF unavailable (storage)

**Do not enable these flags in production.**

---

## 📊 What's Changed

### Version Updates
- All workspace crates: `0.2.1 → 0.3.0`
- Standardized version management across 18 crates
- Updated documentation and deployment guides

### Documentation
- README: Updated Phase 2 status from "📅 Planned" to "✅ Complete"
- CHANGELOG: Comprehensive 0.3.0 release notes
- Added threshold crypto design document
- Updated deployment and operations guides

### Quality
- All code formatted with `cargo fmt`
- Clippy checks passing (warnings only for unused test variables)
- Full workspace builds successfully
- Comprehensive test coverage maintained

---

## 🚀 What's Next

### Phase 3 - Mainnet Preparation
- **Parameter Sweeps:** Empirical optimization of protocol parameters
- **Security Audit:** Professional cryptographic audit of new features
- **Adversarial Testing:** Sybil attacks, collusion scenarios, MEV extraction
- **Performance Optimization:** Benchmarking and optimization of Phase 2 features
- **Explorer Integration:** Full integration with ADIC Explorer for Phase 2 features

---

## 📚 Resources

### Documentation
- [Complete CHANGELOG](./CHANGELOG.md)
- [Threshold Crypto Design](./docs/THRESHOLD_CRYPTO_DESIGN.md)
- [Governance Design](./governance-design.md)
- [Storage Market Design](./storage-market-design.md)
- [Deployment Guide](./docs/operations/DEPLOYMENT_GUIDE.md)
- [Operations Runbook](./docs/operations/RUNBOOK.md)

### Papers
- ADIC-DAG Whitepaper (Section 9.3 - Phase 2)
- ADICPoUW2.pdf (Section 3.2-3.3 - VRF Quorum Selection)
- PoUW III (Section 8-10 - BLS Threshold Signatures)

### Support
- GitHub Issues: https://github.com/IguanAI/adic-core/issues
- Testnet Guide: [TESTNET.md](./TESTNET.md)
- Bootstrap Guide: [BOOTSTRAP.md](./BOOTSTRAP.md)

---

## 🙏 Contributors

This release represents months of development implementing advanced protocol features:
- Governance system with quadratic voting
- Storage market with PoSt verification
- PoUW framework with VRF selection
- Production BLS threshold cryptography
- Complete integration and testing

**ADIC Core Team** - Thank you to everyone who contributed to Phase 2!

---

## 📝 Full Changelog

For a complete list of changes, see [CHANGELOG.md](./CHANGELOG.md#030---2025-10-08).

### Stats
- **New Crates:** 7
- **New Features:** Governance, Storage Market, PoUW, VRF, Quorum, Challenges, BLS/DKG
- **Documentation:** 3 new design documents
- **Security:** Removed insecure XOR crypto, added production BLS
- **Tests:** Comprehensive test coverage across all new modules

---

**Download:** [v0.3.0 Release Assets](https://github.com/IguanAI/adic-core/releases/tag/v0.3.0)

**Verify:** SHA256 checksums provided in release assets

---

*Built with mathematical rigor and cryptographic security*

**ADIC-DAG: Phase 2 Complete** ✅
