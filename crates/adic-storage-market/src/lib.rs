//! # ADIC Storage Market
//!
//! A decentralized storage market built on the ADIC-DAG protocol as a Phase 2 application layer.
//!
//! ## Overview
//!
//! The storage market enables feeless, intent-driven storage deals that inherit ADIC's
//! multi-axis diversity, dual finality, and reputation-weighted consensus guarantees.
//!
//! ## Architecture
//!
//! - **Intent Layer**: Zero-value OCIM messages for deal discovery
//! - **JITCA Compilation**: Just-in-Time Compiled Agreements translate intents to deals
//! - **Finality Integration**: All operations respect F1 (k-core) and F2 (persistent homology) finality guarantees
//! - **Cross-Chain Settlement**: Supports ADIC native, Filecoin, Arweave, and fiat rails
//! - **PoUW Integration**: VRF-based challenges, quorum verification, dispute resolution
//!
//! ## Key Features
//!
//! - **Verifiable Storage**: Cryptographic proofs via Merkle trees
//! - **Feeless Core**: Refundable anti-spam deposits
//! - **Sybil Resistant**: Provider diversity across p-adic neighborhoods
//! - **Censorship Resistant**: MRW tip selection prevents capture
//! - **Atomic Settlement**: HTLC/adaptor signatures for cross-chain deals
//!
//! ## Message Types
//!
//! 1. **StorageDealIntent**: Client publishes storage needs (zero-value)
//! 2. **ProviderAcceptance**: Provider signals willingness (zero-value)
//! 3. **StorageDeal**: Compiled value-bearing deal (locks funds)
//! 4. **DealActivation**: Provider confirms data receipt and storage
//! 5. **StorageProof**: Periodic proofs of continued data possession
//! 6. **SettlementProof**: Cross-chain settlement verification
//!
//! ## Design Documents
//!
//! - **storage-market-design.md**: Complete specification (3891 lines)
//! - **ADICPoUW.pdf**: PoUW integration patterns
//! - **ADICPoUW2.pdf**: VRF commit-reveal and determinism
//! - **ADICPoUW3.pdf**: Governance and dispute resolution

pub mod aggregation;
pub mod coordinator;
pub mod deal_management;
pub mod dispute;
pub mod error;
pub mod intent;
pub mod jitca;
pub mod mrw;
pub mod proof;
pub mod provider;
pub mod retrieval;
pub mod types;

pub use aggregation::{
    AggregationConfig, AggregationManager, AggregationStats, BatchedStorageProof,
    CheckpointProof, IndividualProof,
};
pub use coordinator::{MarketConfig, MarketStats, StorageMarketCoordinator};
pub use deal_management::{
    DealManagementConfig, DealManagementCoordinator, DealManagementStats, DealMigration,
    DealRenewal, DealTermination, MigrationProof, TerminationReason,
};
pub use dispute::{
    DisputeChallenge, DisputeConfig, DisputeManager, DisputeResolution, DisputeRuling,
    DisputeStats, DisputeStatus, DisputeType, PerformanceMetrics,
};
pub use error::{Result, StorageMarketError};
pub use intent::{IntentConfig, IntentManager, IntentStats};
pub use jitca::{JitcaCompiler, JitcaConfig, JitcaStats};
pub use mrw::{AdmissibilityValidator, FeatureEncoder, StorageParentSelector};
pub use proof::{ProofCycleConfig, ProofCycleManager, ProofStats};
pub use provider::{
    ExitReason, ProviderConfig, ProviderExit, ProviderHealth, ProviderHeartbeat,
    ProviderManager, ProviderRegistration, ProviderState, ProviderStats,
};
pub use retrieval::{
    ByteRange, RetrievalConfig, RetrievalManager, RetrievalRequest, RetrievalResponse,
    RetrievalSession, RetrievalStats, RetrievalStatus,
};
pub use types::{
    DealActivation, FinalityStatus, MerkleProof, ProviderAcceptance, SettlementProof,
    SettlementRail, StorageDeal, StorageDealIntent, StorageDealStatus, StorageProof,
};
