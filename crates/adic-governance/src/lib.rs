/*!
# ADIC Governance Module

On-chain governance for the ADIC-DAG protocol implementing:
- Rep-based voting with bounded credits: `w(P) = √min{R(P), Rmax}`
- Constitutional and operational proposal classes
- Full governable parameter vector (19 parameters from PoUW III Table 1)
- Treasury grant execution (atomic/streamed/milestone)
- Axis catalog lifecycle management
- Conflict resolution with priority-based tie-breakers

## Design Reference

This implementation follows the specification in:
- **ADIC PoUW III**: Governance, Parameterization, Treasury Execution, and Conflict Resolution
- **governance-design.md v2.0**: Updated to align with PoUW III (October 7, 2025)

## Core Principles

- **ADIC-Rep Based**: Non-transferable reputation gates proposals and provides voting credits
- **Feeless Governance**: All governance messages are ordinary ADIC messages with refundable deposits
- **Consensus Separation**: Governance does not affect ADIC safety or liveness (Theorem 11.1)
- **Quadratic Caps**: Concave voting function `√Rmax` mitigates plutocracy
- **Dual Finality**: Governance artifacts require F1 (k-core) and F2 (persistent homology) finality
- **Parametric Timelocks**: Enactment delays tied to monitored finality times

## Module Structure

- **types**: Core data structures (Proposal, Vote, Receipt, etc.)
- **voting**: Rep-based voting engine with bounded credits
- **parameters**: Parameter registry with validation
- **error**: Governance-specific errors

## Example Usage

```rust
use adic_governance::{VotingEngine, ParameterRegistry};

// Create voting engine
let voting_engine = VotingEngine::new(
    100_000.0,  // Rmax: maximum reputation cap
    0.1,        // min_quorum: 10% participation required
);

// Compute voting credits for a voter with R=10,000
let credits = voting_engine.compute_voting_credits(10_000.0);
assert_eq!(credits, 100.0);  // √10,000 = 100

// Create parameter registry
let mut registry = ParameterRegistry::new();

// Validate parameter change
registry.validate_change("k", &serde_json::json!(30)).unwrap();

// Apply change
registry.apply_change("k", serde_json::json!(30)).unwrap();
```
*/

pub mod axis_catalog;
pub mod conflict_resolution;
pub mod error;
pub mod lifecycle;
pub mod metrics;
pub mod overlap_penalties;
pub mod parameters;
pub mod treasury;
pub mod types;
pub mod voting;

pub use axis_catalog::{
    AxisCatalogConfig, AxisCatalogManager, AxisEntry, AxisStatus, IndependenceCheck,
    UltrametricValidation,
};
pub use conflict_resolution::{
    ConflictResolver, ConflictSet, ConflictType, ConvergenceMetrics, PriorityScore,
    ResolutionStatus,
};
pub use error::{GovernanceError, Result};
pub use lifecycle::{LifecycleConfig, ProposalLifecycleManager, DST_GOVERNANCE_RECEIPT};
pub use overlap_penalties::OverlapPenaltyTracker;
pub use parameters::{
    GovernanceClass, Parameter, ParameterRegistry, ParameterType, ValidationRule,
};
pub use treasury::{GrantState, GrantStatus, TreasuryExecutor};
pub use types::{
    AxisAction, AxisCatalogChange, Ballot, GovernanceProposal, GovernanceReceipt, GovernanceVote,
    GrantSchedule, Hash, Milestone, MigrationPlan, ProposalClass, ProposalStatus, QuorumStats,
    TreasuryGrant, VerificationScheme, VoteResult,
};
pub use voting::VotingEngine;
