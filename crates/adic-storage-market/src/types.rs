use adic_app_common::LifecycleState;
use adic_economics::types::{AccountAddress, AdicAmount};
use adic_finality::FinalityArtifact;
use adic_types::Signature;
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Hash type (32-byte array)
pub type Hash = [u8; 32];
/// Message hash type (32-byte array)
pub type MessageHash = [u8; 32];

/// Finality status for storage messages
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FinalityStatus {
    /// Message pending finality
    Pending,
    /// F1 (k-core) finality complete
    F1Complete {
        k: u32,
        depth: u32,
        rep_weight: f64,
    },
    /// F2 (persistent homology) finality complete
    F2Complete { homology_stable: bool },
}

impl FinalityStatus {
    pub fn is_finalized(&self) -> bool {
        !matches!(self, FinalityStatus::Pending)
    }
}

/// Settlement rail options for deals
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SettlementRail {
    /// Store on ADIC native providers
    AdicNative,
    /// Cross-chain settlement to Filecoin
    Filecoin,
    /// Cross-chain settlement to Arweave
    Arweave,
    /// Fiat settlement via Payment Service Provider
    Fiat(String), // PSP identifier
}

/// Storage deal status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageDealStatus {
    /// Deal message published, waiting for finality
    Published,
    /// Deal finalized, waiting for provider activation
    PendingActivation,
    /// Provider activated, data transfer in progress
    Active,
    /// Deal completed successfully
    Completed,
    /// Deal failed (missed deadline, provider slashed, etc.)
    Failed,
    /// Deal terminated early
    Terminated,
}

impl adic_app_common::LifecycleState for StorageDealStatus {
    fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Terminated)
    }

    fn can_transition_to(&self, next: &Self) -> bool {
        use StorageDealStatus::*;
        match (self, next) {
            // From Published
            (Published, PendingActivation) => true,
            (Published, Failed) => true, // Can fail during finality or validation

            // From PendingActivation
            (PendingActivation, Active) => true,
            (PendingActivation, Failed) => true,    // Provider fails to activate
            (PendingActivation, Terminated) => true, // Client cancels before activation

            // From Active
            (Active, Completed) => true,   // Successful completion after proof cycle
            (Active, Failed) => true,      // Missed proofs, slashing
            (Active, Terminated) => true,  // Early termination

            // Terminal states cannot transition
            (Completed, _) | (Failed, _) | (Terminated, _) => false,

            // All other transitions are invalid
            _ => false,
        }
    }
}

/// Storage deal intent (zero-value OCIM message)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageDealIntent {
    // ========== Protocol Fields (Required) ==========
    /// d+1 parent messages (MRW-selected)
    pub approvals: [MessageHash; 4],
    /// p-adic features: (time_bucket, topic_hash, region_asn)
    pub features: [u64; 3],
    /// Client signature
    pub signature: Signature,
    /// Refundable anti-spam deposit
    pub deposit: AdicAmount,
    /// Timestamp for time axis encoding
    pub timestamp: i64,

    // ========== Intent Fields ==========
    /// Unique intent identifier (content hash)
    pub intent_id: Hash,
    /// Client account
    pub client: AccountAddress,
    /// Content-addressed data hash (SHA-256 or IPFS CID)
    pub data_cid: [u8; 32],
    /// Data size in bytes
    pub data_size: u64,
    /// Requested storage duration in epochs
    pub duration_epochs: u64,
    /// Maximum price per epoch client willing to pay
    pub max_price_per_epoch: AdicAmount,
    /// Number of independent providers required (≥ 1)
    pub required_redundancy: u8,
    /// Intent expiration timestamp
    pub expires_at: i64,

    // ========== Settlement Preferences ==========
    /// Preferred settlement rails (in priority order)
    pub target_rails: Vec<SettlementRail>,
    /// JSON-encoded hints for JITCA compilation
    pub settlement_hints: String,

    // ========== Protocol State ==========
    /// Current finality status
    pub finality_status: FinalityStatus,
    /// Epoch when finality was reached
    pub finalized_at_epoch: Option<u64>,
}

impl StorageDealIntent {
    /// Create new storage deal intent
    pub fn new(
        client: AccountAddress,
        data_cid: [u8; 32],
        data_size: u64,
        duration_epochs: u64,
        max_price_per_epoch: AdicAmount,
        required_redundancy: u8,
        target_rails: Vec<SettlementRail>,
    ) -> Self {
        let now = Utc::now().timestamp();
        let mut intent_data = Vec::new();
        intent_data.extend_from_slice(&data_cid);
        intent_data.extend_from_slice(&data_size.to_le_bytes());
        intent_data.extend_from_slice(&duration_epochs.to_le_bytes());
        intent_data.extend_from_slice(&now.to_le_bytes());

        Self {
            approvals: [MessageHash::default(); 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: now,
            intent_id: blake3::hash(&intent_data).into(),
            client,
            data_cid,
            data_size,
            duration_epochs,
            max_price_per_epoch,
            required_redundancy,
            expires_at: now + 86400, // 24 hours default
            target_rails,
            settlement_hints: String::new(),
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
        }
    }

    /// Check if intent is expired
    pub fn is_expired(&self) -> bool {
        Utc::now().timestamp() > self.expires_at
    }

    /// Check if intent is finalized
    pub fn is_finalized(&self) -> bool {
        self.finality_status.is_finalized()
    }
}

/// Provider acceptance of a storage intent (zero-value OCIM message)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAcceptance {
    // ========== Protocol Fields (Required) ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Acceptance Fields ==========
    /// Unique acceptance identifier
    pub acceptance_id: Hash,
    /// References finalized StorageDealIntent
    pub ref_intent: Hash,
    /// Provider account
    pub provider: AccountAddress,
    /// Offered price per epoch
    pub offered_price_per_epoch: AdicAmount,
    /// Provider collateral commitment
    pub collateral_commitment: AdicAmount,
    /// Provider's ADIC-Rep at time of acceptance
    pub provider_reputation: f64,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
}

impl ProviderAcceptance {
    /// Create new provider acceptance
    pub fn new(
        ref_intent: Hash,
        provider: AccountAddress,
        offered_price_per_epoch: AdicAmount,
        collateral_commitment: AdicAmount,
        provider_reputation: f64,
    ) -> Self {
        let now = Utc::now().timestamp();
        let mut acceptance_data = Vec::new();
        acceptance_data.extend_from_slice(&ref_intent);
        acceptance_data.extend_from_slice(&now.to_le_bytes());

        Self {
            approvals: [MessageHash::default(); 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: now,
            acceptance_id: blake3::hash(&acceptance_data).into(),
            ref_intent,
            provider,
            offered_price_per_epoch,
            collateral_commitment,
            provider_reputation,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
        }
    }

    /// Check if acceptance is finalized
    pub fn is_finalized(&self) -> bool {
        self.finality_status.is_finalized()
    }
}

/// Compiled storage deal (value-bearing JITCA output)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageDeal {
    // ========== Protocol Fields (Required) ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    /// Multi-sig or sequential signatures from client+provider
    pub signature: Signature,
    /// Combined client+provider deposit
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Deal Fields ==========
    /// Unique deal identifier
    pub deal_id: u64,
    /// Finalized intent reference
    pub ref_intent: Hash,
    /// Finalized acceptance reference
    pub ref_acceptance: Hash,
    /// Client account
    pub client: AccountAddress,
    /// Provider account
    pub provider: AccountAddress,
    /// Data CID
    pub data_cid: [u8; 32],
    /// Data size in bytes
    pub data_size: u64,
    /// Deal duration in epochs
    pub deal_duration_epochs: u64,
    /// Agreed price per epoch
    pub price_per_epoch: AdicAmount,

    // ========== Economic Fields ==========
    /// Provider collateral locked
    pub provider_collateral: AdicAmount,
    /// Client payment escrowed (price_per_epoch * duration)
    pub client_payment_escrow: AdicAmount,

    // ========== Proof Fields ==========
    /// Merkle root of data (set after activation)
    pub proof_merkle_root: Option<[u8; 32]>,
    /// Epoch when deal started (activation)
    pub start_epoch: Option<u64>,
    /// Deadline for activation (current_epoch + grace_period)
    pub activation_deadline: u64,

    // ========== Protocol State ==========
    pub status: StorageDealStatus,
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
}

impl StorageDeal {
    /// Create new storage deal from intent and acceptance
    pub fn from_intent_and_acceptance(
        intent: &StorageDealIntent,
        acceptance: &ProviderAcceptance,
        deal_id: u64,
        current_epoch: u64,
        activation_grace_epochs: u64,
    ) -> Self {
        // Calculate total payment: price_per_epoch * duration_epochs
        let price_base = acceptance.offered_price_per_epoch.to_base_units();
        let total_base = price_base * intent.duration_epochs;
        let total_payment = AdicAmount::from_base_units(total_base);

        Self {
            approvals: [MessageHash::default(); 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: Utc::now().timestamp(),
            deal_id,
            ref_intent: intent.intent_id,
            ref_acceptance: acceptance.acceptance_id,
            client: intent.client,
            provider: acceptance.provider,
            data_cid: intent.data_cid,
            data_size: intent.data_size,
            deal_duration_epochs: intent.duration_epochs,
            price_per_epoch: acceptance.offered_price_per_epoch,
            provider_collateral: acceptance.collateral_commitment,
            client_payment_escrow: total_payment,
            proof_merkle_root: None,
            start_epoch: None,
            activation_deadline: current_epoch + activation_grace_epochs,
            status: StorageDealStatus::Published,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
        }
    }

    /// Check if activation deadline has passed
    pub fn is_activation_deadline_exceeded(&self, current_epoch: u64) -> bool {
        current_epoch > self.activation_deadline && self.start_epoch.is_none()
    }

    /// Check if deal is active
    pub fn is_active(&self) -> bool {
        matches!(self.status, StorageDealStatus::Active)
    }

    /// Check if deal is finalized
    pub fn is_finalized(&self) -> bool {
        self.finality_status.is_finalized()
    }

    /// Transition deal to new status with FSM validation
    ///
    /// This method enforces the state machine rules defined in `StorageDealStatus::can_transition_to()`.
    /// Use this instead of direct status assignment to ensure valid state transitions.
    pub fn transition_to(&mut self, new_status: StorageDealStatus) -> crate::error::Result<()> {
        if !self.status.can_transition_to(&new_status) {
            return Err(crate::error::StorageMarketError::InvalidStateTransition {
                from: format!("{:?}", self.status),
                to: format!("{:?}", new_status),
            });
        }

        tracing::debug!(
            deal_id = self.deal_id,
            from = ?self.status,
            to = ?new_status,
            "Storage deal state transition"
        );

        self.status = new_status;
        Ok(())
    }
}

/// Deal activation message (provider signals data received and stored)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DealActivation {
    // ========== Protocol Fields ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Activation Fields ==========
    /// References the finalized StorageDeal
    pub ref_deal: u64,
    /// Provider account (must match deal.provider)
    pub provider: AccountAddress,
    /// Merkle root of stored data
    pub data_merkle_root: [u8; 32],
    /// Number of data chunks (leaves in Merkle tree)
    pub chunk_count: u64,
    /// Activation epoch
    pub activated_at_epoch: u64,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
}

/// Storage proof message (provider proves continued data possession)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageProof {
    // ========== Protocol Fields ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Proof Fields ==========
    /// Deal identifier
    pub deal_id: u64,
    /// Provider account
    pub provider: AccountAddress,
    /// Epoch this proof covers
    pub proof_epoch: u64,
    /// Deterministically-generated challenge indices
    /// Generated from blake3(deal_id || epoch || data_cid)
    pub challenge_indices: Vec<u64>,
    /// Merkle proofs for challenged chunks
    pub merkle_proofs: Vec<MerkleProof>,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
}

/// Merkle proof for a single chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleProof {
    /// Chunk index (leaf position)
    pub chunk_index: u64,
    /// Chunk data (4096 bytes)
    pub chunk_data: Vec<u8>,
    /// Sibling hashes from leaf to root
    pub sibling_hashes: Vec<[u8; 32]>,
}

impl MerkleProof {
    /// Verify Merkle proof against root
    pub fn verify(&self, root: &[u8; 32]) -> bool {
        let mut current_hash: [u8; 32] = *blake3::hash(&self.chunk_data).as_bytes();
        let mut index = self.chunk_index;

        for sibling in &self.sibling_hashes {
            let combined = if index % 2 == 0 {
                // Current is left child
                let mut data = Vec::new();
                data.extend_from_slice(&current_hash);
                data.extend_from_slice(sibling);
                *blake3::hash(&data).as_bytes()
            } else {
                // Current is right child
                let mut data = Vec::new();
                data.extend_from_slice(sibling);
                data.extend_from_slice(&current_hash);
                *blake3::hash(&data).as_bytes()
            };
            current_hash = combined;
            index /= 2;
        }

        &current_hash == root
    }
}

/// Settlement proof (proof that cross-chain settlement occurred)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettlementProof {
    // ========== Protocol Fields ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Settlement Fields ==========
    /// Deal identifier
    pub deal_id: u64,
    /// Settlement rail used
    pub settlement_rail: SettlementRail,
    /// External transaction/proof identifier
    pub external_tx_id: String,
    /// Settlement amount
    pub settlement_amount: AdicAmount,
    /// Finality artifact from ADIC
    pub finality_artifact: FinalityArtifact,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_deal_intent_creation() {
        let client = AccountAddress::from_bytes([1u8; 32]);
        let data_cid = [2u8; 32];

        let intent = StorageDealIntent::new(
            client,
            data_cid,
            1024 * 1024, // 1 MB
            100, // 100 epochs
            AdicAmount::from_adic(10.0),
            3, // 3x redundancy
            vec![SettlementRail::AdicNative],
        );

        assert_eq!(intent.client, client);
        assert_eq!(intent.data_cid, data_cid);
        assert_eq!(intent.required_redundancy, 3);
        assert!(!intent.is_expired());
        assert!(!intent.is_finalized());
    }

    #[test]
    fn test_provider_acceptance_creation() {
        let intent_id = blake3::hash(b"test_intent").into();
        let provider = AccountAddress::from_bytes([3u8; 32]);

        let acceptance = ProviderAcceptance::new(
            intent_id,
            provider,
            AdicAmount::from_adic(8.0),
            AdicAmount::from_adic(50.0),
            100.0,
        );

        assert_eq!(acceptance.ref_intent, intent_id);
        assert_eq!(acceptance.provider, provider);
        assert_eq!(acceptance.provider_reputation, 100.0);
        assert!(!acceptance.is_finalized());
    }

    #[test]
    fn test_storage_deal_from_intent_and_acceptance() {
        let client = AccountAddress::from_bytes([1u8; 32]);
        let provider = AccountAddress::from_bytes([2u8; 32]);

        let intent = StorageDealIntent::new(
            client,
            [0u8; 32],
            1000,
            10,
            AdicAmount::from_adic(5.0),
            1,
            vec![SettlementRail::AdicNative],
        );

        let acceptance = ProviderAcceptance::new(
            intent.intent_id,
            provider,
            AdicAmount::from_adic(4.0),
            AdicAmount::from_adic(50.0),
            100.0,
        );

        let deal = StorageDeal::from_intent_and_acceptance(
            &intent,
            &acceptance,
            12345,
            100,
            10, // 10 epoch grace period
        );

        assert_eq!(deal.deal_id, 12345);
        assert_eq!(deal.client, client);
        assert_eq!(deal.provider, provider);
        assert_eq!(deal.activation_deadline, 110);
        assert!(!deal.is_active());
        assert!(!deal.is_activation_deadline_exceeded(100));
        assert!(deal.is_activation_deadline_exceeded(111));
    }

    #[test]
    fn test_merkle_proof_verification() {
        let chunk_data = vec![0u8; 4096];
        let chunk_hash: [u8; 32] = blake3::hash(&chunk_data).into();

        // Simple proof: just the leaf hash equals root (tree with one leaf)
        let proof = MerkleProof {
            chunk_index: 0,
            chunk_data: chunk_data.clone(),
            sibling_hashes: vec![],
        };

        assert!(proof.verify(&chunk_hash));
    }

    #[test]
    fn test_finality_status() {
        assert!(!FinalityStatus::Pending.is_finalized());
        assert!(FinalityStatus::F1Complete {
            k: 20,
            depth: 12,
            rep_weight: 150.0
        }
        .is_finalized());
        assert!(FinalityStatus::F2Complete {
            homology_stable: true
        }
        .is_finalized());
    }

    #[test]
    fn test_storage_deal_status_fsm_valid_transitions() {
        use StorageDealStatus::*;

        // Valid transitions from Published
        assert!(Published.can_transition_to(&PendingActivation));
        assert!(Published.can_transition_to(&Failed));

        // Valid transitions from PendingActivation
        assert!(PendingActivation.can_transition_to(&Active));
        assert!(PendingActivation.can_transition_to(&Failed));
        assert!(PendingActivation.can_transition_to(&Terminated));

        // Valid transitions from Active
        assert!(Active.can_transition_to(&Completed));
        assert!(Active.can_transition_to(&Failed));
        assert!(Active.can_transition_to(&Terminated));
    }

    #[test]
    fn test_storage_deal_status_fsm_invalid_transitions() {
        use StorageDealStatus::*;

        // Cannot skip states
        assert!(!Published.can_transition_to(&Active));
        assert!(!Published.can_transition_to(&Completed));

        // Cannot go backwards
        assert!(!Active.can_transition_to(&PendingActivation));
        assert!(!Active.can_transition_to(&Published));

        // Terminal states cannot transition
        assert!(!Completed.can_transition_to(&Active));
        assert!(!Completed.can_transition_to(&Failed));
        assert!(!Failed.can_transition_to(&Active));
        assert!(!Failed.can_transition_to(&Completed));
        assert!(!Terminated.can_transition_to(&Active));
    }

    #[test]
    fn test_storage_deal_status_terminal_states() {
        use StorageDealStatus::*;

        assert!(!Published.is_terminal());
        assert!(!PendingActivation.is_terminal());
        assert!(!Active.is_terminal());
        assert!(Completed.is_terminal());
        assert!(Failed.is_terminal());
        assert!(Terminated.is_terminal());
    }

    #[test]
    fn test_storage_deal_transition_to_valid() {
        let client = AccountAddress::from_bytes([1u8; 32]);
        let provider = AccountAddress::from_bytes([2u8; 32]);

        let intent = StorageDealIntent::new(
            client,
            [0u8; 32],
            1000,
            10,
            AdicAmount::from_adic(5.0),
            1,
            vec![SettlementRail::AdicNative],
        );

        let acceptance = ProviderAcceptance::new(
            intent.intent_id,
            provider,
            AdicAmount::from_adic(4.0),
            AdicAmount::from_adic(50.0),
            100.0,
        );

        let mut deal = StorageDeal::from_intent_and_acceptance(&intent, &acceptance, 1, 100, 10);

        // Initial state
        assert_eq!(deal.status, StorageDealStatus::Published);

        // Valid transition: Published -> PendingActivation
        assert!(deal.transition_to(StorageDealStatus::PendingActivation).is_ok());
        assert_eq!(deal.status, StorageDealStatus::PendingActivation);

        // Valid transition: PendingActivation -> Active
        assert!(deal.transition_to(StorageDealStatus::Active).is_ok());
        assert_eq!(deal.status, StorageDealStatus::Active);

        // Valid transition: Active -> Completed
        assert!(deal.transition_to(StorageDealStatus::Completed).is_ok());
        assert_eq!(deal.status, StorageDealStatus::Completed);
    }

    #[test]
    fn test_storage_deal_transition_to_invalid() {
        let client = AccountAddress::from_bytes([1u8; 32]);
        let provider = AccountAddress::from_bytes([2u8; 32]);

        let intent = StorageDealIntent::new(
            client,
            [0u8; 32],
            1000,
            10,
            AdicAmount::from_adic(5.0),
            1,
            vec![SettlementRail::AdicNative],
        );

        let acceptance = ProviderAcceptance::new(
            intent.intent_id,
            provider,
            AdicAmount::from_adic(4.0),
            AdicAmount::from_adic(50.0),
            100.0,
        );

        let mut deal = StorageDeal::from_intent_and_acceptance(&intent, &acceptance, 1, 100, 10);

        // Invalid transition: Published -> Active (must go through PendingActivation)
        let result = deal.transition_to(StorageDealStatus::Active);
        assert!(result.is_err());
        assert_eq!(deal.status, StorageDealStatus::Published); // State unchanged

        // Move to terminal state
        deal.transition_to(StorageDealStatus::PendingActivation).unwrap();
        deal.transition_to(StorageDealStatus::Active).unwrap();
        deal.transition_to(StorageDealStatus::Completed).unwrap();

        // Invalid transition: Completed -> Active (terminal state)
        let result = deal.transition_to(StorageDealStatus::Active);
        assert!(result.is_err());
        assert_eq!(deal.status, StorageDealStatus::Completed); // State unchanged
    }
}
