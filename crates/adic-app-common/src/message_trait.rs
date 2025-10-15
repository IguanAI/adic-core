use adic_finality::FinalityGate;
use adic_types::{AdicMessage, MessageId, PublicKey};
use serde::{Deserialize, Serialize};

/// Reputation requirements for message submission
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepRequirements {
    pub min_reputation: f64,
    pub required_for: RepCheckType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RepCheckType {
    Submission,    // e.g., governance proposal submission
    Participation, // e.g., voting
    Verification,  // e.g., quorum member
}

/// Application-level message types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppMessageType {
    Storage(StorageMessageType),
    PoUW(PoUWMessageType),
    Governance(GovernanceMessageType),
    VRF(VRFMessageType),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageMessageType {
    DealIntent,
    ProviderAcceptance,
    StorageDeal,
    DealActivation,
    StorageProof,
    RetrievalRequest,
    RetrievalResponse,
    DisputeChallenge,
    DisputeResolution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoUWMessageType {
    HookMessage,
    WorkResult,
    MilestoneReceipt,
    FraudProof,
    QuorumReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GovernanceMessageType {
    Proposal,
    Vote,
    Receipt,
    TreasuryGrant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VRFMessageType {
    Commit,
    Reveal,
}

/// Core trait that all application messages must implement
/// This extends the base AdicMessage with application-specific requirements
pub trait AppMessage: Send + Sync + Clone {
    /// The lifecycle state type for this message
    type Lifecycle: LifecycleState;

    /// Get the underlying ADIC message
    fn as_adic_message(&self) -> &AdicMessage;

    /// Get the message ID
    fn id(&self) -> MessageId {
        self.as_adic_message().id
    }

    /// Get the proposer public key
    fn proposer(&self) -> PublicKey {
        self.as_adic_message().proposer_pk
    }

    /// Get the application message type
    fn app_type(&self) -> AppMessageType;

    /// Get the current lifecycle state
    fn lifecycle_state(&self) -> Self::Lifecycle;

    /// Set the lifecycle state (returns new instance)
    fn with_lifecycle_state(&self, state: Self::Lifecycle) -> Self;

    /// Get the required finality gate for this message
    fn required_finality(&self) -> FinalityGate;

    /// Get the reputation requirements for this message
    fn rep_requirements(&self) -> Option<RepRequirements>;

    /// Serialize to bytes
    fn to_bytes(&self) -> Vec<u8>;

    /// Deserialize from bytes
    fn from_bytes(bytes: &[u8]) -> crate::Result<Self>
    where
        Self: Sized;
}

/// Trait for lifecycle states
pub trait LifecycleState: Send + Sync + Clone + std::fmt::Debug {
    /// Check if this is a terminal state
    fn is_terminal(&self) -> bool;

    /// Check if transition to another state is valid
    fn can_transition_to(&self, next: &Self) -> bool;
}
