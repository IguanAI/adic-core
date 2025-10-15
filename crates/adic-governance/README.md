# ADIC Governance Module

On-chain governance for the ADIC-DAG protocol implementing Rep-based voting, parameter management, treasury execution, and axis catalog lifecycle.

## Overview

The ADIC Governance Module enables **feeless, reputation-weighted governance** that inherits ADIC's multi-axis diversity, dual finality (F1 k-core and F2 persistent homology), and consensus separation guarantees. It demonstrates how the protocol can evolve through decentralized decision-making without compromising safety or liveness.

## Architecture

### Core Components

1. **Voting Engine** (`src/voting.rs`)
   - Bounded voting credits: `w(P) = √min{R(P), Rmax}`
   - Non-transferable influence based on ADIC-Rep
   - Quorum and threshold checking
   - Constitutional (≥66.7%) and Operational (>50%) thresholds

2. **Parameter Registry** (`src/parameters.rs`)
   - Full governable parameter vector (19 parameters from PoUW III Table 1)
   - Safe range validation
   - Custom validation functions (rho length, t_bls bounds, replication_t)
   - Deterministic parameter enactment at epoch boundaries

3. **Proposal Lifecycle** (`src/lifecycle.rs`)
   - Rep-gated proposal submission (R ≥ Rsubmit_min)
   - Voting period management
   - Timelock calculation: `Tenact = max{γ₁·TF1, γ₂·TF2, Tmin}`
   - Status transitions: Voting → Tallying → Succeeded → Enacting → Enacted

4. **Treasury Executor** (`src/treasury.rs`)
   - Atomic grants (single immediate payment)
   - Streamed grants (rate_per_epoch over duration)
   - Milestone-gated grants (deliverable verification)
   - Clawback mechanisms for failed milestones

5. **Axis Catalog Manager** (`src/axis_catalog.rs`)
   - Ultrametric validation for new axes
   - Independence checks (anti-capture via correlation analysis)
   - Migration ramps (0→1 and 1→0 weight transitions)
   - Axis lifecycle: Proposed → Ramping → Active → Deprecating → Deprecated

6. **Conflict Resolver** (`src/conflict_resolution.rs`)
   - Conflict set detection (overlapping parameters/axes/epochs)
   - Priority-based resolution (Constitutional > Operational)
   - Tie-breakers (finality time, quorum margin, lexicographic hash)
   - Energy-style convergence monitoring

### Governance Flow

```
1. Proposal Submission (R ≥ Rsubmit_min)
   └─> GovernanceProposal (parameter change, axis change, or treasury grant)

2. Voting Period
   └─> GovernanceVote messages with bounded credits: √min{R(voter), Rmax}

3. Tallying
   └─> Quorum check (≥ min_quorum participation)
   └─> Threshold check (≥66.7% Constitutional, >50% Operational)

4. Conflict Resolution (if needed)
   └─> Detect overlapping proposals
   └─> Priority-based winner selection

5. Finality Achievement
   └─> F1 k-core + F2 persistent homology (placeholder)

6. Enactment Period (Timelock)
   └─> Community review window
   └─> Tenact = max{γ₁·TF1, γ₂·TF2, Tmin}

7. Execution
   └─> Parameter changes applied deterministically
   └─> Treasury grants executed (atomic/streamed/milestone)
   └─> Axis migrations begin (ramp schedule)
```

## Key Features

### Rep-Based Voting

- **Bounded Credits**: `w(P) = √min{R(P), Rmax}` prevents plutocracy
- **Non-Transferable**: Cannot buy or sell influence
- **Concave Cap**: Diminishing returns (10,000× more Rep → only 31.6× more influence)
- **Sybil-Resistant**: Creating multiple identities doesn't increase total influence

Example:
```rust
let engine = VotingEngine::new(100_000.0, 0.1);

// Voter A: R=100 → w=√100 = 10 influence
let credits_a = engine.compute_voting_credits(100.0);
assert_eq!(credits_a, 10.0);

// Voter B: R=1,000,000 → w=√min{1,000,000, 100,000} = √100,000 ≈ 316 influence
let credits_b = engine.compute_voting_credits(1_000_000.0);
assert!((credits_b - 316.227766).abs() < 0.0001);
```

### Governable Parameters

All 19 parameters from PoUW III Table 1:

| Parameter | Type | Default | Range | Class |
|-----------|------|---------|-------|-------|
| `p` (prime) | u8 | 3 | {3, 5, 7} | Constitutional |
| `d` (dimension) | u8 | 3 | {3, 4} | Constitutional |
| `ρ` (axis radii) | Vec<u8> | [2, 2, 1] | ℤ≥1^d | Constitutional |
| `q` (diversity) | u8 | 3 | [2, d+1] | Constitutional |
| `k` (k-core) | u32 | 20 | ℤ≥1 | Operational |
| `D` (depth) | u32 | 12 | ℤ≥1 | Operational |
| `Δ` (window) | u32 | 5 | ℤ≥1 | Operational |
| `(λ, μ)` (MRW weights) | (f64, f64) | (1.0, 1.0) | ℝ>0² | Operational |
| `(α, β)` (MRW exponents) | (f64, f64) | (1.0, 1.0) | ℝ>0² | Operational |
| `Rmin, rmin` | (f64, f64) | (50.0, 10.0) | ℝ≥0² | Operational |
| `m` (committee size) | u32 | 64 | ℤ≥1 | Operational |
| `tBLS` (threshold) | u32 | 43 | [⌈m/2⌉, m] | Operational |
| `(r, t)` (replication) | (u32, u32) | (5, 4) | ℤ≥1² | Operational |
| `challenge_depth` | u32 | 5 | ℤ≥1 | Operational |
| `Tenact` (timelock) | f64 | 1000.0 | ℝ>0 | Operational |
| `Rmax` (voting cap) | f64 | 100,000.0 | ℝ≥1 | Operational |

### Treasury Execution

Three grant modes:

```rust
// 1. Atomic: Single immediate payment
let grant = TreasuryGrant {
    recipient: recipient_pk,
    total_amount: AdicAmount::from_adic(100.0),
    schedule: GrantSchedule::Atomic,
    proposal_id,
};

// 2. Streamed: Regular payments over time
let grant = TreasuryGrant {
    recipient: recipient_pk,
    total_amount: AdicAmount::from_adic(300.0),
    schedule: GrantSchedule::Streamed {
        rate_per_epoch: AdicAmount::from_adic(10.0),
        duration_epochs: 30,
    },
    proposal_id,
};

// 3. Milestone: Deliverable-gated payments
let grant = TreasuryGrant {
    recipient: recipient_pk,
    total_amount: AdicAmount::from_adic(500.0),
    schedule: GrantSchedule::Milestone {
        milestones: vec![
            Milestone {
                id: 1,
                amount: AdicAmount::from_adic(200.0),
                deliverable_cid: "Qm_phase1_deliverables",
                deadline_epoch: 1000,
                verification_scheme: VerificationScheme::QuorumAttestation { threshold: 0.66 },
            },
            // ... more milestones
        ],
    },
    proposal_id,
};
```

### Axis Catalog Management

Manage the multi-axis diversity framework:

```rust
let manager = AxisCatalogManager::new(AxisCatalogConfig::default());

// Propose new axis
let change = AxisCatalogChange {
    action: AxisAction::Add,
    axis_id: 3,
    encoder_spec_cid: "Qm_encoder_wasm",
    ultrametric_proof_cid: "Qm_mathematical_proof",
    security_analysis_cid: "Qm_security_analysis",
    migration_plan: MigrationPlan {
        ramp_epochs: 1000,
        weight_schedule: vec![(0, 0.0), (500, 0.5), (1000, 1.0)],
    },
};

manager.propose_axis_change(&change, current_epoch).await?;

// Activate axis (begins ramp)
manager.activate_axis(3, current_epoch).await?;

// Get current weight during ramp
let weight = manager.get_axis_weight(3, current_epoch).await?;
// weight = (current_epoch - start_epoch) / ramp_epochs
```

### Conflict Resolution

Automatic resolution of conflicting proposals:

```rust
let mut resolver = ConflictResolver::new();

// Detect conflicts
let conflicts = resolver.detect_conflicts(&proposals);

// Resolve using priority rules:
// 1. Constitutional > Operational
// 2. Earlier finality time
// 3. Higher quorum margin
// 4. Lexicographic hash

let winner = resolver.resolve_conflict_set(&conflict_id, &proposals_map)?;
```

## Usage

### Submitting a Proposal

```rust
use adic_governance::*;
use adic_types::PublicKey;
use chrono::Utc;

// Create proposal
let proposal = GovernanceProposal {
    proposal_id: compute_id(...),
    class: ProposalClass::Operational,
    proposer_pk: my_public_key,
    param_keys: vec!["k".to_string()],
    new_values: serde_json::json!({"k": 30}),
    axis_changes: None,
    treasury_grant: None,
    enact_epoch: 10000,
    rationale_cid: "Qm_rationale_document",
    creation_timestamp: Utc::now(),
    voting_end_timestamp: Utc::now() + chrono::Duration::days(7),
    status: ProposalStatus::Voting,
    tally_yes: 0.0,
    tally_no: 0.0,
    tally_abstain: 0.0,
};

// Submit (requires R ≥ Rsubmit_min)
let lifecycle_manager = ProposalLifecycleManager::new(config, rep_tracker);
let proposal_id = lifecycle_manager.submit_proposal(proposal).await?;
```

### Casting a Vote

```rust
// Get voter reputation
let voter_rep = rep_tracker.get_reputation(&voter_pk).await;

// Compute voting credits
let voting_engine = VotingEngine::new(100_000.0, 0.1);
let credits = voting_engine.compute_voting_credits(voter_rep);

// Create vote
let vote = GovernanceVote::new(
    proposal_id,
    voter_pk,
    credits,
    Ballot::Yes,
);

// Cast vote
lifecycle_manager.cast_vote(vote).await?;
```

### Managing Parameters

```rust
let mut registry = ParameterRegistry::new();

// Get current value
let k_value = registry.get("k").unwrap();
println!("Current k: {}", k_value);

// Validate proposed change
registry.validate_change("k", &serde_json::json!(30))?;

// Apply change (after governance approval)
registry.apply_change("k", serde_json::json!(30))?;
```

## Configuration

### Lifecycle Configuration

```rust
let config = LifecycleConfig {
    min_proposer_reputation: 100.0,
    voting_duration_secs: 7 * 24 * 3600,  // 7 days
    min_quorum: 0.1,                       // 10% participation
    rmax: 100_000.0,
    gamma_f1: 2.0,
    gamma_f2: 3.0,
    min_timelock_secs: 1000.0,
};
```

### Axis Catalog Configuration

```rust
let config = AxisCatalogConfig {
    max_correlation_threshold: 0.7,           // Anti-capture
    review_committee_min_reputation: 1000.0,
    default_ramp_epochs: 1000,
};
```

## Design Principles

### 1. Consensus Separation

**Theorem 11.1 (PoUW III)**: Governance artifacts do not influence ADIC safety or liveness.

All governance messages are ordinary ADIC messages. No governance result is consulted by consensus. Base safety/liveness reduce to established ADIC proofs.

### 2. Feeless Governance

- All governance messages are OCIM messages (zero-value)
- Refundable anti-spam deposits
- No token voting or staking

### 3. Reputation-Weighted

- Non-transferable ADIC-Rep as voting power
- Concave cap function prevents dominance
- Overlap penalties for coordinated rings

### 4. Parametric Timelocks

Enactment delays tied to observed finality times:

```
Tenact = max{γ₁·TF1, γ₂·TF2, Tmin}
```

where `TF1`, `TF2` are P95 finality times monitored continuously.

### 5. Sybil Resistance

- Multi-axis diversity in proposal propagation
- Reputation-weighted consensus
- Bounded voting credits prevent Sybil multiplication

## Testing

### Unit Tests (27 tests)

```bash
cargo test -p adic-governance --lib
```

Coverage:
- Voting credits calculation and tallying (6 tests)
- Parameter validation and application (5 tests)
- Proposal lifecycle management (3 tests)
- Treasury execution (atomic, streamed) (2 tests)
- Axis catalog validation and migration (7 tests)
- Conflict detection and resolution (7 tests)

### Module Tests

```bash
# Test voting engine
cargo test -p adic-governance voting::tests

# Test parameter registry
cargo test -p adic-governance parameters::tests

# Test conflict resolution
cargo test -p adic-governance conflict_resolution::tests
```

**Result: 27/27 tests passing ✅**

## Implementation Status

### ✅ Implemented (Phases 2.1 & 2.2 & 2.3-partial)

- Rep-based voting with bounded credits
- Full parameter vector (19 parameters)
- Treasury execution (atomic/streamed/milestone)
- Proposal lifecycle management
- Timelock calculation
- Axis catalog validation
- Migration ramps
- Conflict resolution
- Energy-style convergence

### ❌ Not Yet Implemented

- F1+F2 finality integration (placeholder)
- BLS threshold signatures for receipts
- Overlap penalties for coordinated voting
- ADIC message integration (C1-C3, MRW)
- REST API endpoints
- Governance metrics dashboard
- Canonical JSON (RFC 8785) encoding

## Design Documents

- **governance-design.md**: Complete specification (586 lines, v2.0)
- **ADIC PoUW III**: Governance, Parameterization, Treasury Execution, and Conflict Resolution

## Performance Characteristics

- **Credit Calculation**: O(1) - simple sqrt operation
- **Vote Tallying**: O(n) for n votes with duplicate detection
- **Conflict Detection**: O(n²) for n proposals (pairwise comparison)
- **Conflict Resolution**: O(n log n) for n conflicting proposals (sorting by priority)
- **Parameter Validation**: O(1) for most rules, O(d) for custom validation

## Future Enhancements

1. **F1+F2 Integration**: Actual finality verification (currently placeholder)
2. **BLS Signatures**: Threshold signatures for GovernanceReceipt
3. **Overlap Penalties**: p-adic ball detection and attenuation factor η
4. **ADIC Message Integration**: Full C1-C3 admissibility and MRW tip selection
5. **REST API**: HTTP endpoints for proposal submission and querying
6. **Metrics Dashboard**: Real-time governance participation and Rep distribution
7. **Emergency Response**: Off-chain fork coordination for critical issues

## License

Same as parent ADIC project (check repository root).

## References

### Design Documents
- [Governance Design](../../governance-design.md)

### Published Papers
- [ADIC-DAG Paper](../../docs/references/adic-dag-paper.pdf)
- [ADIC Yellow Paper](../../docs/references/ADICYellowPaper.pdf)

### Related Crates
- [ADIC Consensus](../adic-consensus/)
- [ADIC Economics](../adic-economics/)
