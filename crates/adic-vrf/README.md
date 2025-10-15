# ADIC VRF (Verifiable Random Function)

Normative randomness generation for ADIC-DAG using commit-reveal VRF scheme.

## Overview

The ADIC VRF module provides **deterministic, unpredictable, and verifiable randomness** for various protocol components including quorum selection, worker lottery, and challenge generation. It implements a two-phase commit-reveal scheme to ensure randomness cannot be manipulated by any single participant.

## Design Principles

Based on **ADIC PoUW II §2-3**:

- **Normative Randomness**: Network-wide canonical randomness per epoch
- **Commit-Reveal**: Two-phase protocol prevents last-actor manipulation
- **VRF-Based**: Each participant contributes verifiable random output
- **Weighted Aggregation**: Reputation-weighted XOR of individual VRF outputs
- **Non-Interactive**: No coordination required beyond message posting

## Architecture

### Two-Phase Protocol

**Phase 1: Commit (Epoch E_{k-1})**
```
Participants post VRFCommit messages:
  - commitment = H(VRF_proof)
  - Hides actual VRF output until Phase 2
  - Posted in preceding epoch
```

**Phase 2: Reveal (Early Epoch E_k)**
```
Participants post VRFOpen messages:
  - vrf_proof = actual VRF evaluation
  - Must match commitment from Phase 1
  - Reveals randomness for aggregation
```

**Aggregation**
```
Canonical randomness R_k = XOR of all valid VRF outputs
  - Weight by committer reputation: repeat VRF output ⌊R(P)⌋ times
  - Deterministic: all nodes compute same R_k
  - Unpredictable: requires collusion of all participants
```

### Key Components

1. **VRFCommit** (`src/messages.rs`)
   - Commitment to VRF proof: `commitment = H(π)`
   - Posted in epoch E_{k-1} for epoch k
   - Includes committer reputation for weighting

2. **VRFOpen** (`src/messages.rs`)
   - Reveals VRF proof π
   - References prior VRFCommit message
   - Posted in early epoch E_k

3. **VRFService** (`src/service.rs`)
   - Generates VRF proofs
   - Manages commit-reveal lifecycle
   - Computes canonical randomness

4. **CanonicalRandomness** (`src/canonical.rs`)
   - Aggregates VRF outputs from all participants
   - Reputation-weighted XOR
   - Caches computed randomness per epoch

## Usage

### Commit Phase (Epoch k-1)

```rust
use adic_vrf::{VRFService, VRFConfig};

let vrf_service = VRFService::new(
    my_vrf_keypair,
    reputation_tracker,
    VRFConfig::default(),
);

// Generate VRF proof for next epoch
let target_epoch = current_epoch + 1;
let seed = format!("ADIC-VRF-EPOCH-{}", target_epoch);
let vrf_proof = vrf_service.generate_vrf_proof(&seed)?;

// Create commitment
let commitment = blake3::hash(&vrf_proof);

// Post VRFCommit message
let commit_msg = VRFCommit::new(
    adic_message,
    target_epoch,
    *commitment.as_bytes(),
    my_pubkey,
    my_reputation,
);

// Submit to ADIC-DAG
dag.submit_message(commit_msg).await?;
```

### Reveal Phase (Early Epoch k)

```rust
// Post VRFOpen message with actual proof
let open_msg = VRFOpen::new(
    adic_message,
    target_epoch,
    commit_msg.id(),
    vrf_proof.to_vec(),
    my_vrf_pubkey,
);

dag.submit_message(open_msg).await?;
```

### Computing Canonical Randomness

```rust
use adic_vrf::compute_canonical_randomness;

// Collect all finalized VRFOpen messages for epoch
let opens: Vec<VRFOpen> = dag.get_vrf_opens(epoch).await?;

// Compute canonical randomness
let r_k = compute_canonical_randomness(
    &opens,
    epoch,
    reputation_tracker,
).await?;

// Use for quorum selection, worker lottery, etc.
let selected_workers = select_via_vrf(task_id, r_k, workers)?;
```

## VRF State Management

```rust
pub enum VRFState {
    /// No commit for epoch yet
    NoCommit,

    /// Committed but not yet revealed
    Committed {
        commitment: [u8; 32],
        commit_message_id: MessageId,
    },

    /// Both commit and reveal posted
    Revealed {
        commitment: [u8; 32],
        vrf_proof: Vec<u8>,
        canonical_randomness: Option<CanonicalRandomness>,
    },

    /// Commit without reveal (slashable offense)
    Forfeited,
}
```

## Configuration

```rust
let config = VRFConfig {
    /// Commit deadline: epochs before target epoch
    commit_deadline_epochs: 1,

    /// Reveal window: early portion of target epoch
    reveal_window_epochs: 0.1,  // First 10% of epoch

    /// Minimum reputation to participate in VRF
    min_committer_reputation: 100.0,

    /// Weight cap for reputation (prevents dominance)
    reputation_weight_cap: 1000.0,

    /// Cache size for computed randomness
    randomness_cache_size: 100,
};
```

## Security Properties

### Unpredictability

- **Last-Actor Problem Solved**: Commit-reveal prevents last participant from choosing favorable randomness
- **Collusion Required**: All participants must collude to manipulate randomness
- **VRF Non-Malleability**: Cannot derive related outputs from observed VRF proofs

### Verifiability

- **Commitment Binding**: Cannot change VRF proof after commitment
- **VRF Proof Verification**: Anyone can verify VRF proof matches commitment and public key
- **Deterministic Aggregation**: All nodes compute same canonical randomness

### Liveness

- **Forfeit Mechanism**: Participants who commit but don't reveal forfeit reputation/stake
- **Sufficient Participation**: Only need threshold of honest participants
- **Fallback**: If too few reveals, use fallback randomness from previous epoch

## API Reference

### VRFService

```rust
impl VRFService {
    /// Create new VRF service
    pub fn new(
        vrf_keypair: VRFKeypair,
        reputation_tracker: Arc<ReputationTracker>,
        config: VRFConfig,
    ) -> Self;

    /// Generate VRF proof for seed
    pub fn generate_vrf_proof(&self, seed: &str) -> Result<VRFProof>;

    /// Create commitment for next epoch
    pub async fn create_commitment(&self, target_epoch: u64) -> Result<VRFCommit>;

    /// Reveal VRF proof for epoch
    pub async fn reveal_proof(&self, target_epoch: u64) -> Result<VRFOpen>;

    /// Get canonical randomness for epoch
    pub async fn get_canonical_randomness(&self, epoch: u64) -> Result<CanonicalRandomness>;

    /// Check if commit/reveal deadlines passed
    pub fn check_deadlines(&self, current_epoch: u64, target_epoch: u64) -> DeadlineStatus;
}
```

### Canonical Randomness Computation

```rust
/// Compute canonical randomness from VRF opens
pub async fn compute_canonical_randomness(
    opens: &[VRFOpen],
    epoch: u64,
    reputation_tracker: Arc<ReputationTracker>,
) -> Result<CanonicalRandomness> {
    let mut aggregated = [0u8; 32];

    for open in opens {
        // Verify VRF proof
        if !open.verify()? {
            continue;
        }

        // Get committer reputation
        let rep = reputation_tracker.get_reputation(&open.public_key).await;
        let weight = rep.min(reputation_weight_cap).floor() as usize;

        // XOR weighted by reputation
        for _ in 0..weight {
            for i in 0..32 {
                aggregated[i] ^= open.vrf_output()[i];
            }
        }
    }

    Ok(CanonicalRandomness {
        epoch,
        value: aggregated,
        participant_count: opens.len(),
        computed_at: Utc::now(),
    })
}
```

## Integration Points

### PoUW Worker Selection

```rust
use adic_vrf::VRFService;

let vrf_service = VRFService::new(...);
let r_k = vrf_service.get_canonical_randomness(epoch).await?;

// Use as seed for worker lottery
let task_seed = blake3::hash(&[&task_id[..], &r_k.value[..]].concat());
let selected_workers = select_workers_via_vrf(task_seed, workers)?;
```

### Quorum Selection

```rust
use adic_quorum::QuorumSelector;

let selector = QuorumSelector::new(vrf_service, reputation_tracker);
let quorum = selector.select_committee(epoch, config, nodes).await?;
```

### Challenge Generation

```rust
use adic_challenges::ChallengeWindowManager;

let r_k = vrf_service.get_canonical_randomness(epoch).await?;
let challenge_seed = blake3::hash(&[&deal_id[..], &r_k.value[..]].concat());
let challenged_chunks = generate_challenge_indices(challenge_seed, num_chunks)?;
```

## Design Rationale

### Why Commit-Reveal?

Without commit-reveal, the last participant to submit VRF output can:
1. Compute canonical randomness for each possible output
2. Choose output that favors them (e.g., selects them for quorum)
3. Submit that output

Commit-reveal prevents this by forcing participants to commit before seeing others' outputs.

### Why Reputation-Weighted?

Equal weighting allows Sybil attacks (create many identities to dominate randomness). Reputation weighting:
- Requires substantial Rep accumulation to influence randomness
- Aligns with ADIC's reputation-weighted consensus model
- Caps prevent single high-Rep participant from dominating

### Why XOR Aggregation?

XOR is:
- Associative and commutative (order doesn't matter)
- Efficient to compute
- Provides uniform distribution if any input is uniform
- Simple to verify deterministically

## Testing

### Unit Tests (6 tests)

```bash
cargo test -p adic-vrf --lib
```

Coverage:
- VRF proof generation and verification
- Commitment creation and validation
- Canonical randomness computation
- Reputation weighting
- State transitions (NoCommit → Committed → Revealed)
- Deadline checking

**Result: 6/6 tests passing ✅**

## Performance Characteristics

- **VRF Proof Generation**: O(1) - single elliptic curve operation
- **Commitment**: O(1) - single hash
- **Reveal Verification**: O(1) - VRF verification
- **Canonical Aggregation**: O(n × w) for n participants, avg weight w
- **Memory**: ~100 bytes per commit, ~300 bytes per reveal

## Future Enhancements

1. **Threshold VRF**: Multiple parties generate single VRF output
2. **Optimistic Reveals**: Skip commit phase when trust is high
3. **Slashing**: Automatic penalties for non-reveal
4. **Batched Commits**: Commit to multiple future epochs at once
5. **VRF Pooling**: Aggregate multiple VRF outputs per participant

## References

- **ADIC PoUW II** §2: Normative Randomness Generation
- **ADIC PoUW II** §3: VRF-based Quorum Selection
- [VRF RFC](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-vrf)

## License

Same as parent ADIC project (check repository root).
