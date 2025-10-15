# ADIC Challenges

Challenge windows, fraud proofs, and dispute adjudication for ADIC-DAG applications.

## Overview

The ADIC Challenges module provides **time-bounded dispute resolution** with fraud proof verification and quorum-based arbitration. It enables trustless challenge of incorrect results, invalid proofs, or malicious behavior with cryptographic evidence and economic stakes.

## Design Principles

Based on **ADIC PoUW II §5**:

- **Challenge Windows**: Time-bounded periods for dispute submission
- **Fraud Proofs**: Cryptographic evidence of incorrect behavior
- **Economic Stakes**: Challengers risk stake; false challenges lose funds
- **Quorum Adjudication**: High-reputation arbitrators resolve complex disputes
- **Deterministic Resolution**: Automatic verification when possible

## Architecture

### Challenge Lifecycle

```
1. Challenge Window Opens
   └─> After event finalization (validation, deal activation, etc.)
   └─> Fixed duration (e.g., 5 epochs or 24 hours)

2. Challenge Submission
   └─> Challenger stakes funds
   └─> Submits fraud proof with evidence
   └─> Challenge recorded on-chain

3. Fraud Proof Verification
   └─> Automatic verification if deterministic
   └─> OR quorum adjudication if complex

4. Resolution
   └─> If valid: Challenger rewarded, malicious party slashed
   └─> If invalid: Challenger loses stake
   └─> Challenge window closes

5. Finalization
   └─> No valid challenges: Event becomes final
   └─> Valid challenge: Event reversed/penalized
```

### Key Components

1. **ChallengeWindow** (`src/window.rs`)
   - Time-bounded challenge periods
   - Window status (Open, Challenging, Closed, Disputed)
   - Challenge collection per window

2. **ChallengeWindowManager** (`src/window.rs`)
   - Open/close windows for events
   - Challenge submission handling
   - Window expiry checks

3. **FraudProof** (`src/fraud_proof.rs`)
   - Evidence of incorrect behavior
   - Proof types (IncorrectComputation, InvalidMerkleProof, etc.)
   - Cryptographic verification

4. **FraudProofVerifier** (`src/fraud_proof.rs`)
   - Automatic fraud proof verification
   - Deterministic result re-computation
   - Evidence validation

5. **DisputeAdjudicator** (`src/adjudication.rs`)
   - Quorum-based complex dispute resolution
   - Arbitrator selection (VRF + high-Rep)
   - Vote collection and threshold checking

## Usage

### Open Challenge Window

```rust
use adic_challenges::{ChallengeWindowManager, ChallengeConfig};

let manager = ChallengeWindowManager::new(ChallengeConfig::default());

// Open window after work result validation
let window_id = manager.open_window(
    "task_123_result",
    current_epoch,
    5,  // Duration: 5 epochs
).await?;

println!("Challenge window open for 5 epochs");
```

### Submit Challenge with Fraud Proof

```rust
use adic_challenges::{FraudProof, FraudType, FraudEvidence};

// Challenger believes worker's result is incorrect
let fraud_proof = FraudProof {
    fraud_type: FraudType::IncorrectComputation,
    target_entity: worker_pubkey,
    evidence: FraudEvidence {
        claim: "Worker output hash doesn't match re-execution".to_string(),
        proof_data: vec![
            // Original input data CID
            // Worker's output CID
            // Challenger's re-executed output CID
            // Merkle proofs, etc.
        ],
        witness_signatures: vec![],
    },
    challenger_stake: AdicAmount::from_adic(10.0),
};

// Submit challenge
manager.submit_challenge(
    window_id,
    fraud_proof,
    challenger_pubkey,
    current_epoch,
).await?;
```

### Verify Fraud Proof (Automatic)

```rust
use adic_challenges::FraudProofVerifier;

let verifier = FraudProofVerifier::new(task_executor);

// Try automatic verification
match verifier.verify_proof(&fraud_proof, &task).await {
    Ok(true) => {
        // Fraud proof valid: challenger wins
        println!("✅ Fraud proven! Worker result was incorrect.");

        // Slash worker, reward challenger
        reward_manager.slash_collateral(worker, SlashReason::Fraud).await?;
        reward_manager.distribute_reward(challenger, challenger_reward).await?;
    }
    Ok(false) => {
        // Fraud proof invalid: challenger loses stake
        println!("❌ Challenge failed. Challenger loses stake.");
    }
    Err(_) => {
        // Cannot automatically verify, needs adjudication
        println!("⚖️ Requires manual arbitration");
    }
}
```

### Quorum Adjudication (Complex Disputes)

```rust
use adic_challenges::{DisputeAdjudicator, ArbitratorVote, VoteDecision};

let adjudicator = DisputeAdjudicator::new(
    quorum_selector,
    reputation_tracker,
    AdjudicationConfig::default(),
);

// Select arbitration committee
let committee = adjudicator.select_arbitrators(
    &fraud_proof,
    15,  // Committee size
    current_epoch,
).await?;

// Collect arbitrator votes
let mut votes = Vec::new();
for arbitrator in committee {
    let decision = arbitrator.review_evidence(&fraud_proof).await?;
    votes.push(ArbitratorVote {
        arbitrator: arbitrator.public_key,
        decision,  // ChallengerWins, DefendantWins, Inconclusive
        rationale: "...".to_string(),
        signature: arbitrator.sign(&fraud_proof.id()),
    });
}

// Adjudicate with threshold (e.g., 66% majority)
let result = adjudicator.adjudicate(&fraud_proof, votes).await?;

match result.ruling {
    DisputeRuling::ChallengerWins => {
        println!("🏆 Committee rules in favor of challenger");
    }
    DisputeRuling::DefendantWins => {
        println!("🛡️ Committee rules in favor of defendant");
    }
    DisputeRuling::Inconclusive => {
        println!("🤷 Committee cannot reach consensus");
    }
}
```

### Check Window Status

```rust
// Check if challenge window is still open
if manager.is_window_open(window_id, current_epoch).await? {
    println!("Window open, can submit challenges");
} else {
    println!("Window closed, event is final");
}

// Get all challenges for a window
let challenges = manager.get_challenges(window_id).await?;
println!("Received {} challenges", challenges.len());
```

## Fraud Types

```rust
pub enum FraudType {
    /// Worker's computation result is provably incorrect
    IncorrectComputation,

    /// Merkle proof doesn't verify against claimed root
    InvalidMerkleProof,

    /// Data stored/retrieved doesn't match claimed CID
    DataMismatch,

    /// Multiple conflicting signatures/commitments from same entity
    DoubleSign,

    /// False claim about resource usage or performance
    FalseAttestation,

    /// Generic fraud with custom evidence
    Custom(String),
}
```

## Challenge Status

```rust
pub enum ChallengeStatus {
    /// Challenge submitted, awaiting verification
    Pending,

    /// Under automatic verification
    Verifying,

    /// Requires arbitration committee
    Arbitrating,

    /// Challenge was valid, defendant slashed
    Upheld,

    /// Challenge was invalid, challenger penalized
    Rejected,

    /// Could not determine outcome
    Inconclusive,
}
```

## Configuration

```rust
pub struct ChallengeConfig {
    /// Default challenge window duration in epochs
    pub default_window_epochs: u64,

    /// Minimum stake to submit challenge
    pub min_challenger_stake: AdicAmount,

    /// Percentage of slashed funds given to challenger
    pub challenger_reward_share: f64,

    /// Minimum reputation to be arbitrator
    pub min_arbitrator_reputation: f64,

    /// Arbitration committee size
    pub arbitration_committee_size: usize,

    /// Arbitration threshold (e.g., 0.66 for 66% majority)
    pub arbitration_threshold: f64,

    /// Max challenges per window (anti-spam)
    pub max_challenges_per_window: usize,
}

impl Default for ChallengeConfig {
    fn default() -> Self {
        Self {
            default_window_epochs: 5,
            min_challenger_stake: AdicAmount::from_adic(10.0),
            challenger_reward_share: 0.5,  // 50% of slashed amount
            min_arbitrator_reputation: 5000.0,
            arbitration_committee_size: 15,
            arbitration_threshold: 0.66,
            max_challenges_per_window: 100,
        }
    }
}
```

## Evidence Types

```rust
pub struct FraudEvidence {
    /// Textual description of fraud claim
    pub claim: String,

    /// Proof data (CIDs, hashes, signatures, etc.)
    pub proof_data: Vec<Vec<u8>>,

    /// Optional witness signatures
    pub witness_signatures: Vec<Signature>,
}
```

### Evidence Examples

**Incorrect Computation**:
```rust
FraudEvidence {
    claim: "Worker output doesn't match re-execution".to_string(),
    proof_data: vec![
        task_input_cid.as_bytes().to_vec(),
        worker_output_cid.as_bytes().to_vec(),
        correct_output_cid.as_bytes().to_vec(),
        re_execution_proof.as_bytes().to_vec(),
    ],
    witness_signatures: vec![],
}
```

**Invalid Merkle Proof**:
```rust
FraudEvidence {
    claim: "Storage proof doesn't verify against claimed root".to_string(),
    proof_data: vec![
        merkle_root.to_vec(),
        chunk_index.to_le_bytes().to_vec(),
        claimed_chunk_data.to_vec(),
        claimed_sibling_hashes.concat(),
    ],
    witness_signatures: vec![],
}
```

## Automatic Verification

Deterministic fraud proofs can be automatically verified:

```rust
impl FraudProofVerifier {
    pub async fn verify_proof(&self, proof: &FraudProof, context: &Context) -> Result<bool> {
        match proof.fraud_type {
            FraudType::IncorrectComputation => {
                // Re-execute task
                let expected = self.executor.execute(&context.task, &context.input).await?;
                let claimed = &proof.evidence.proof_data[1];
                Ok(expected.output_cid != claimed)
            }

            FraudType::InvalidMerkleProof => {
                // Verify Merkle proof
                let root = &proof.evidence.proof_data[0];
                let chunk_data = &proof.evidence.proof_data[2];
                let siblings = &proof.evidence.proof_data[3];

                let computed_root = compute_merkle_root(chunk_data, siblings);
                Ok(computed_root != root)
            }

            FraudType::DataMismatch => {
                // Fetch data and compare CID
                let claimed_cid = &proof.evidence.proof_data[0];
                let data = ipfs_fetch(claimed_cid).await?;
                let actual_cid = compute_cid(&data);
                Ok(actual_cid != claimed_cid)
            }

            _ => Err(ChallengeError::RequiresArbitration),
        }
    }
}
```

## Adjudication Process

For complex disputes that cannot be automatically verified:

```rust
pub struct AdjudicationResult {
    pub fraud_proof_id: Hash,
    pub ruling: DisputeRuling,
    pub votes: Vec<ArbitratorVote>,
    pub vote_counts: (usize, usize, usize),  // (challenger, defendant, inconclusive)
    pub committee_signature: BlsSignature,
}

impl DisputeAdjudicator {
    pub async fn adjudicate(
        &self,
        fraud_proof: &FraudProof,
        votes: Vec<ArbitratorVote>,
    ) -> Result<AdjudicationResult> {
        // Tally votes
        let challenger_wins = votes.iter().filter(|v| v.decision == VoteDecision::ChallengerWins).count();
        let defendant_wins = votes.iter().filter(|v| v.decision == VoteDecision::DefendantWins).count();
        let inconclusive = votes.iter().filter(|v| v.decision == VoteDecision::Inconclusive).count();

        // Check threshold
        let total = votes.len();
        let threshold_votes = (total as f64 * self.config.arbitration_threshold) as usize;

        let ruling = if challenger_wins >= threshold_votes {
            DisputeRuling::ChallengerWins
        } else if defendant_wins >= threshold_votes {
            DisputeRuling::DefendantWins
        } else {
            DisputeRuling::Inconclusive
        };

        // Aggregate BLS signatures
        let signatures: Vec<_> = votes.iter().map(|v| v.signature).collect();
        let committee_sig = bls_aggregate(&signatures)?;

        Ok(AdjudicationResult {
            fraud_proof_id: fraud_proof.id(),
            ruling,
            votes,
            vote_counts: (challenger_wins, defendant_wins, inconclusive),
            committee_signature: committee_sig,
        })
    }
}
```

## Economic Model

### Challenger Economics

**Submit Challenge**:
- Stake required: `min_challenger_stake` (e.g., 10 ADIC)
- Risk: Lose stake if challenge invalid
- Reward: `challenger_reward_share` × slashed_amount (e.g., 50% of slashed collateral)

**Example**:
```
Worker collateral: 50 ADIC
Challenger stake: 10 ADIC

If challenge valid:
  - Worker slashed: 50 ADIC (100% for fraud)
  - Challenger reward: 10 ADIC (stake) + 25 ADIC (50% of slash) = 35 ADIC
  - Net profit: 25 ADIC

If challenge invalid:
  - Challenger loses: 10 ADIC stake
  - Worker keeps: 50 ADIC collateral
```

### Arbitrator Economics

**Voting Correctly**:
- Reputation increase: +1% for agreeing with majority
- Reputation decrease: -1% for disagreeing with majority
- Long-term: Accurate arbitrators build Rep, get selected more often

## Integration Points

### PoUW Task Validation

```rust
// After quorum validates result
let validation_report = validator.validate_result(&result).await?;

// Open challenge window
let window_id = challenge_manager.open_window(
    &result.result_id,
    current_epoch,
    5,  // 5 epoch window
).await?;

// Wait for challenge window
tokio::time::sleep(Duration::from_secs(5 * 600)).await;  // 5 epochs

// Check for valid challenges
let challenges = challenge_manager.get_challenges(window_id).await?;
let has_valid_challenge = challenges.iter().any(|c| c.status == ChallengeStatus::Upheld);

if !has_valid_challenge {
    // Finalize task, distribute rewards
    task_manager.finalize_task(task_id, current_epoch).await?;
}
```

### Storage Market Proofs

```rust
// After storage proof submission
let proof = StorageProof { ... };

// Open challenge window
let window_id = challenge_manager.open_window(
    &proof.proof_id,
    current_epoch,
    5,
).await?;

// Anyone can challenge if proof invalid
let fraud_proof = FraudProof {
    fraud_type: FraudType::InvalidMerkleProof,
    target_entity: provider,
    evidence: FraudEvidence {
        claim: "Merkle proof doesn't verify".to_string(),
        proof_data: vec![
            proof.data_merkle_root.to_vec(),
            proof.merkle_proofs[0].chunk_index.to_le_bytes().to_vec(),
            proof.merkle_proofs[0].chunk_data.clone(),
            proof.merkle_proofs[0].sibling_hashes.concat(),
        ],
        witness_signatures: vec![],
    },
    challenger_stake: AdicAmount::from_adic(10.0),
};

challenge_manager.submit_challenge(window_id, fraud_proof, challenger, current_epoch).await?;
```

### Governance Proposals

```rust
// After proposal enacts
let proposal = governance.get_proposal(proposal_id).await?;

// Open challenge window for parameter safety
let window_id = challenge_manager.open_window(
    &proposal.proposal_id,
    current_epoch,
    10,  // Longer window for governance
).await?;

// Community can challenge if parameter values are unsafe
// (Though governance should catch this earlier)
```

## Testing

### Unit Tests (8 tests)

```bash
cargo test -p adic-challenges --lib
```

Coverage:
- Challenge window lifecycle (open, close, expiry)
- Challenge submission with stake validation
- Fraud proof verification (automatic)
- Adjudication committee selection
- Vote tallying and threshold checking
- Challenger reward calculation
- Window status transitions
- Evidence validation

**Result: 8/8 tests passing ✅**

## Performance Characteristics

- **Window Management**: O(1) to open/close window
- **Challenge Submission**: O(1) to record challenge
- **Automatic Verification**: O(T) where T is task execution time
- **Adjudication**: O(c) for c committee members
- **Memory**: ~500 bytes per challenge, ~200 bytes per window

## Security Properties

### False Challenge Prevention

- **Economic Stake**: Challengers risk funds
- **Reputation Penalty**: Failed challenges decrease Rep
- **Rate Limiting**: Max challenges per window prevents spam

### Fraud Detection Incentives

- **Challenger Rewards**: 50% of slashed amount incentivizes monitoring
- **Public Verification**: Anyone can challenge (permissionless)
- **Evidence-Based**: Requires cryptographic proof, not just accusation

### Arbitrator Alignment

- **Reputation at Stake**: Incorrect votes decrease Rep
- **VRF Selection**: Cannot predict if selected, prevents collusion
- **Threshold**: Only need 66% majority, tolerates some bad actors

## Design Rationale

### Why Time Windows?

- **Finality**: Events must eventually become final
- **Liveness**: Cannot wait indefinitely for challenges
- **Predictability**: Fixed window duration allows planning

### Why Economic Stakes?

- **Anti-Spam**: Free challenges would flood system
- **Skin in the Game**: Challengers must be confident
- **Reward Mechanism**: Successful challengers profit, incentivizes monitoring

### Why Automatic + Quorum?

- **Efficiency**: Automatic verification when deterministic
- **Flexibility**: Quorum handles complex/subjective disputes
- **Cost**: Automatic is free, quorum is expensive (use sparingly)

## Future Enhancements

1. **Escalation**: Appeal to larger committee if inconclusive
2. **Batch Verification**: Verify multiple proofs together
3. **ZK Fraud Proofs**: Privacy-preserving evidence
4. **Slashing Gradation**: Partial slashing based on severity
5. **Challenge Bonding**: Lock stake for duration, not just submission

## References

- **ADIC PoUW II** §5: Challenge Mechanisms and Fraud Proofs
- **ADIC PoUW III** §10: Dispute Resolution

## License

Same as parent ADIC project (check repository root).
