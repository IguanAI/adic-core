# ADIC Storage Market

A decentralized storage marketplace built on the ADIC-DAG protocol as a Phase 2 application layer.

## Overview

The ADIC Storage Market enables **feeless, intent-driven storage deals** that inherit ADIC's multi-axis diversity, dual finality, and reputation-weighted consensus guarantees. It demonstrates how value-bearing applications can be built on top of ADIC's zero-value OCIM (Omnichain Commerce Intent Mempool) foundation.

## Architecture

### Core Components

1. **Intent Layer** (`src/intent.rs`)
   - Zero-value intent discovery via OCIM messages
   - Clients publish `StorageDealIntent` messages
   - Providers respond with `ProviderAcceptance` messages
   - Refundable anti-spam deposits (credited on finality)

2. **JITCA Compiler** (`src/jitca.rs`)
   - Just-in-Time Compiled Agreements
   - Translates finalized intents + acceptances → value-bearing deals
   - Validates semantic compatibility
   - Locks client payment and provider collateral in escrow

3. **Proof Cycle Manager** (`src/proof.rs`)
   - VRF-based deterministic challenge generation (blake3)
   - Merkle tree proof verification
   - Automated payment releases (10% per proof by default)
   - Slashing for missed proofs (10% penalty, 5 epoch grace period)

4. **MRW Integration** (`src/mrw.rs`)
   - Multi-axis Random Walk parent selection
   - 3-axis p-adic feature encoding:
     - Time: 10-minute buckets
     - Topic: Data CID hash
     - Region: ASN/geo region
   - Admissibility constraints (C1: existence, C2: diversity, C3: Sybil resistance)

5. **Market Coordinator** (`src/coordinator.rs`)
   - Orchestrates full deal lifecycle
   - Epoch management
   - Market-wide statistics
   - Deal queries by client/provider

6. **Provider Lifecycle** (`src/provider.rs`)
   - Provider registration with capacity declaration
   - Heartbeat monitoring (100 epoch intervals)
   - Health tracking (Healthy/Degraded/Offline/Exiting)
   - Graceful exit with migration support
   - Automatic offline detection

7. **Retrieval Protocol** (`src/retrieval.rs`)
   - Data retrieval requests with SLA requirements
   - Bandwidth pricing and payment on finality
   - Range request support (partial file retrieval)
   - Timeout handling and performance tracking
   - Off-chain QUIC transfer coordination

8. **Deal Management** (`src/deal_management.rs`)
   - Deal renewal for extended duration
   - Early termination with refund calculations
   - Provider migration with data transfer verification
   - Automated penalty calculations (10% client, 20% provider breach)
   - Six termination reasons with different economic outcomes

9. **Dispute Resolution** (`src/dispute.rs`)
   - DisputeChallenge and DisputeResolution message types
   - Four dispute types (DataCorruption, DataUnavailable, IncorrectProof, PerformanceBreach)
   - Automatic resolution via consensus (provider health checks)
   - VRF-based quorum selection for complex disputes
   - Evidence submission via IPFS/Arweave CIDs
   - Majority voting with configurable threshold (default 66%)

10. **Proof Aggregation** (`src/aggregation.rs`)
   - BatchedStorageProof for multiple deals/epochs in one message
   - CheckpointProof for periodic verification (every 1000 epochs)
   - 99% reduction in proof messages for long-term deals
   - Aggregate Merkle root verification
   - Automatic batching with configurable thresholds

### Deal Lifecycle

```
1. Intent Publication (Client)
   └─> StorageDealIntent (zero-value OCIM message)

2. Provider Acceptance
   └─> ProviderAcceptance (zero-value OCIM message)

3. Finality Achievement
   └─> F1/F2 confirmation via ADIC consensus
   └─> Deposits automatically refunded

4. JITCA Compilation
   └─> StorageDeal created (value-bearing)
   └─> Funds locked in escrow
   └─> Status: PendingActivation

5. Deal Activation (Provider)
   └─> DealActivation with Merkle root
   └─> Status: Active

6. Proof Submissions (Provider)
   └─> StorageProof every N epochs
   └─> Automated payment releases
   └─> Status: Active

7. Completion
   └─> After duration elapsed
   └─> Status: Completed

   OR Slashing (if proofs missed)
   └─> Automatic after grace period
   └─> Status: Failed
```

## Key Features

### Feeless Core
- Zero-value intent discovery via OCIM
- Refundable anti-spam deposits (0.1 ADIC default)
- Only value-bearing deals lock significant funds

### Verifiable Storage
- Cryptographic Merkle tree proofs
- Deterministic challenge generation (blake3-based)
- VRF integration for fairness

### Automated Economics
- 10% payment release per successful proof
- 10% collateral slashing for missed proofs
- 5 epoch grace period before slashing
- Configurable via `ProofCycleConfig`

### Finality Integration
- All messages respect F1/F2 guarantees
- Dual finality tracking (k-core + persistent homology)

### Cross-Chain Settlement
Supports multiple settlement rails:
- **ADIC Native**: Direct on-chain settlement
- **Filecoin**: FVM integration for storage deals
- **Arweave**: Permanent storage coordination
- **Fiat**: Traditional payment integration

### Sybil Resistance
- Multi-axis p-adic diversity in parent selection
- Reputation-weighted consensus integration
- Prevents provider collusion and centralization

## Usage

### Publishing a Storage Intent

```rust
use adic_storage_market::*;
use adic_economics::AdicAmount;

// Create coordinator
let coordinator = StorageMarketCoordinator::new(
    MarketConfig::default(),
    intent_manager,
    jitca_compiler,
    proof_manager,
    balance_manager,
);

// Publish intent
let mut intent = StorageDealIntent::new(
    client_address,
    data_cid,
    1024 * 1024 * 1024,  // 1 GB
    2880,  // ~30 days at 10min/epoch
    AdicAmount::from_adic(1.0),  // Max price per epoch
    3,  // 3x redundancy
    vec![SettlementRail::AdicNative],
);
intent.deposit = AdicAmount::from_adic(0.1);

let intent_id = coordinator.publish_intent(intent).await?;
```

### Provider Acceptance

```rust
// Provider submits acceptance (after intent is finalized)
let acceptance = ProviderAcceptance::new(
    intent_id,
    provider_address,
    AdicAmount::from_adic(0.5),  // Price per epoch
    AdicAmount::from_adic(50.0),  // Collateral
    85.0,  // Reputation score
);

let acceptance_id = coordinator.submit_acceptance(acceptance).await?;
```

### Compiling a Deal

```rust
// After both intent and acceptance are finalized
let deal_id = coordinator.compile_deal(&intent_id, &acceptance_id).await?;
```

### Activating a Deal

```rust
// Provider activates after receiving data
let activation = DealActivation {
    approvals: parent_hashes,
    features: encode_features(timestamp, data_cid, region),
    signature: provider_signature,
    deposit: AdicAmount::ZERO,
    timestamp: chrono::Utc::now().timestamp(),
    ref_deal: deal_id,
    provider: provider_address,
    data_merkle_root: compute_merkle_root(&data),
    chunk_count: total_chunks,
    activated_at_epoch: current_epoch,
    finality_status: FinalityStatus::Pending,
    finalized_at_epoch: None,
};

coordinator.activate_deal(deal_id, activation).await?;
```

### Submitting Proofs

```rust
// Provider generates challenges
let deal = coordinator.get_deal(deal_id).await.unwrap();
let challenges = coordinator.proof_manager
    .generate_challenge(&deal, current_epoch)
    .await?;

// Provider creates Merkle proofs for challenged chunks
let merkle_proofs: Vec<MerkleProof> = challenges.iter()
    .map(|&chunk_index| {
        create_merkle_proof_for_chunk(chunk_index, &data, &merkle_tree)
    })
    .collect();

// Submit proof
let proof = StorageProof {
    approvals: parent_hashes,
    features: encode_features(timestamp, data_cid, region),
    signature: provider_signature,
    deposit: AdicAmount::ZERO,
    timestamp: chrono::Utc::now().timestamp(),
    deal_id,
    provider: provider_address,
    proof_epoch: current_epoch,
    challenge_indices: challenges,
    merkle_proofs,
    finality_status: FinalityStatus::Pending,
    finalized_at_epoch: None,
};

coordinator.submit_proof(proof).await?;
```

## Configuration

### Market Configuration

```rust
let config = MarketConfig {
    intent_config: IntentConfig {
        min_deposit: AdicAmount::from_adic(0.1),
        min_client_reputation: 10.0,
        max_intent_duration: 86400,  // 24 hours
        max_intents_per_client: 10,
    },
    jitca_config: JitcaConfig {
        min_deal_duration: 100,  // epochs
        max_deal_duration: 100000,  // epochs
        activation_grace_period: 50,  // epochs
        min_collateral_ratio: 1.0,  // 100% of payment
        finality_requirement: FinalityLevel::F1Complete,
        compilation_window: 1000,  // epochs
    },
    proof_config: ProofCycleConfig {
        chunks_per_challenge: 10,
        proof_interval_epochs: 10,
        proof_grace_period: 5,
        slash_percentage: 0.1,  // 10%
        payment_per_proof: 0.1,  // 10%
    },
};
```

## Design Principles

### 1. Feeless Discovery
Intent publication and acceptance submission are zero-value OCIM messages. Only compiled deals lock significant value, reducing spam incentives.

### 2. Economic Security
- Provider collateral ≥ client payment (default 100%)
- Automated slashing for missed proofs
- Incremental payment releases prevent provider rug-pulls

### 3. Cryptographic Verification
- Deterministic challenges prevent selective disclosure
- Merkle proofs ensure data possession
- blake3 hashing for performance

### 4. Multi-Finality Awareness
All state transitions respect:
- **F1 (k-core)**: Structural consensus
- **F2 (persistent homology)**: Topological confirmation

### 5. Censorship Resistance
- MRW tip selection prevents capture
- Multi-axis diversity requirements
- Reputation-weighted parent selection

## Testing

### Unit Tests (54 tests)
```bash
cargo test -p adic-storage-market --lib
```

Coverage:
- Intent lifecycle and finality
- JITCA compilation and validation
- Proof generation and verification
- MRW parent selection
- Provider lifecycle management
- Retrieval protocol and bandwidth SLAs
- Deal management (renewal, termination, migration)
- Dispute resolution (challenges, automatic & quorum-based resolution)
- Proof aggregation (batched proofs, checkpoints)
- Type conversions and utilities

### Integration Tests (6 tests)
```bash
cargo test -p adic-storage-market --test integration_test
```

Coverage:
- Full deal lifecycle (intent → completion)
- Concurrent deals management
- Slashing scenarios
- Error handling
- Market statistics
- Deal queries

### All Tests
```bash
cargo test -p adic-storage-market
```

**Result: 60/60 tests passing ✅**

## Design Documents

- **[storage-market-design.md](../../storage-market-design.md)**: Complete specification (3900+ lines)

**Note:** PoUW integration patterns referenced in the design are implemented in:
- **PoUW Integration**: See `../adic-pouw/` crate
- **VRF & Determinism**: See `../adic-vrf/` and `../adic-quorum/` crates
- **Governance & Disputes**: See `../adic-governance/` and `../adic-challenges/` crates

## Performance Characteristics

- **Challenge Generation**: O(log n) with blake3
- **Merkle Verification**: O(log n) tree depth
- **Parent Selection**: O(k·d) for k parents, d dimensions
- **Proof Submission**: Sub-second for 10 chunks (4KB each)

## Future Enhancements

1. **Dispute Resolution**: Governance-based arbitration (see `../adic-governance/`)
2. **Quorum Verification**: Multi-provider proof validation
3. **Cross-Chain Bridges**: HTLC/adaptor signatures for atomic settlement
4. **Data Repair**: Automatic redundancy restoration
5. **Dynamic Pricing**: Market-driven price discovery
6. **Retrieval Market**: Separate layer for data access

## License

Same as parent ADIC project (check repository root).

## References

### Design Documents
- [Storage Market Design](../../storage-market-design.md)

### Published Papers
- [ADIC-DAG Paper](../../docs/references/adic-dag-paper.pdf)
- [ADIC Yellow Paper](../../docs/references/ADICYellowPaper.pdf)
- [ADIC Applications Paper](../../docs/references/ADICApplications.pdf)

### Related Crates
- [ADIC Economics](../adic-economics/)
- [ADIC Consensus](../adic-consensus/)
- [ADIC PoUW](../adic-pouw/)
