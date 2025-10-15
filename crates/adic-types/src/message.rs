use crate::{canonical_hash, AdicFeatures, MessageId, PublicKey, Signature, ValueTransfer};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Domain Separation Tags for cryptographic signatures (PoUW III §10.3)
///
/// These tags ensure that signatures for different message types cannot be confused
/// or replayed across different contexts. Each message type uses a unique DST prefix.
pub mod dst {
    /// Governance proposal signature tag
    pub const GOVERNANCE_PROPOSAL: &[u8] = b"ADIC-GOV-PROP-v1";

    /// Governance vote signature tag
    pub const GOVERNANCE_VOTE: &[u8] = b"ADIC-GOV-VOTE-v1";

    /// Governance receipt signature tag (already defined in adic-governance, included here for completeness)
    pub const GOVERNANCE_RECEIPT: &[u8] = b"ADIC-GOV-R-v1";

    /// Governance milestone attestation signature tag
    pub const GOVERNANCE_MILESTONE: &[u8] = b"ADIC-GOV-MILESTONE-v1";
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConflictId(pub String);

impl ConflictId {
    pub fn new(id: String) -> Self {
        Self(id)
    }

    pub fn none() -> Self {
        Self(String::new())
    }

    pub fn is_none(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Display for ConflictId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdicMeta {
    pub timestamp: DateTime<Utc>,
    pub axes: HashMap<String, String>,
    pub conflict: ConflictId,
}

impl AdicMeta {
    pub fn new(timestamp: DateTime<Utc>) -> Self {
        Self {
            timestamp,
            axes: HashMap::new(),
            conflict: ConflictId::none(),
        }
    }

    pub fn with_conflict(mut self, conflict: ConflictId) -> Self {
        self.conflict = conflict;
        self
    }

    pub fn add_axis_tag(mut self, key: String, value: String) -> Self {
        self.axes.insert(key, value);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdicMessage {
    pub id: MessageId,
    pub parents: Vec<MessageId>,
    pub features: AdicFeatures,
    pub meta: AdicMeta,
    pub proposer_pk: PublicKey,
    pub signature: Signature,

    /// Optional value transfer - when present, this message transfers ADIC tokens
    pub transfer: Option<ValueTransfer>,

    /// Arbitrary data payload (renamed from payload for clarity)
    pub data: Vec<u8>,
}

impl AdicMessage {
    pub fn new(
        parents: Vec<MessageId>,
        features: AdicFeatures,
        meta: AdicMeta,
        proposer_pk: PublicKey,
        data: Vec<u8>,
    ) -> Self {
        let mut msg = Self {
            id: MessageId::from_bytes([0; 32]),
            parents,
            features,
            meta,
            proposer_pk,
            signature: Signature::empty(),
            transfer: None,
            data,
        };

        msg.id = msg.compute_id();
        msg
    }

    /// Create a new message with a value transfer
    pub fn new_with_transfer(
        parents: Vec<MessageId>,
        features: AdicFeatures,
        meta: AdicMeta,
        proposer_pk: PublicKey,
        transfer: ValueTransfer,
        data: Vec<u8>,
    ) -> Self {
        let mut msg = Self {
            id: MessageId::from_bytes([0; 32]),
            parents,
            features,
            meta,
            proposer_pk,
            signature: Signature::empty(),
            transfer: Some(transfer),
            data,
        };

        msg.id = msg.compute_id();
        msg
    }

    /// Check if this message includes a value transfer
    pub fn has_value_transfer(&self) -> bool {
        self.transfer.is_some()
    }

    /// Get the transfer if present
    pub fn get_transfer(&self) -> Option<&ValueTransfer> {
        self.transfer.as_ref()
    }

    pub fn compute_id(&self) -> MessageId {
        // Create canonical representation (excludes id and signature)
        #[derive(Serialize)]
        struct CanonicalMessage<'a> {
            parents: &'a Vec<MessageId>,
            features: &'a AdicFeatures,
            meta: &'a AdicMeta,
            proposer_pk: &'a PublicKey,
            transfer: &'a Option<ValueTransfer>,
            data: &'a Vec<u8>,
        }

        let canonical = CanonicalMessage {
            parents: &self.parents,
            features: &self.features,
            meta: &self.meta,
            proposer_pk: &self.proposer_pk,
            transfer: &self.transfer,
            data: &self.data,
        };

        // Use canonical JSON hashing for deterministic ID
        let hash = canonical_hash(&canonical).expect("Failed to compute canonical hash");
        MessageId::from_bytes(hash)
    }

    pub fn verify_id(&self) -> bool {
        self.id == self.compute_id()
    }

    pub fn parent_count(&self) -> usize {
        self.parents.len()
    }

    pub fn is_genesis(&self) -> bool {
        self.parents.is_empty()
    }

    /// Get the message bytes for signing (excludes the signature field)
    ///
    /// Uses canonical JSON for deterministic serialization
    pub fn to_bytes(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct SignableMessage<'a> {
            id: &'a MessageId,
            parents: &'a Vec<MessageId>,
            features: &'a AdicFeatures,
            meta: &'a AdicMeta,
            proposer_pk: &'a PublicKey,
            transfer: &'a Option<ValueTransfer>,
            data: &'a Vec<u8>,
        }

        let signable = SignableMessage {
            id: &self.id,
            parents: &self.parents,
            features: &self.features,
            meta: &self.meta,
            proposer_pk: &self.proposer_pk,
            transfer: &self.transfer,
            data: &self.data,
        };

        // Use canonical JSON for deterministic serialization
        use crate::to_canonical_json;
        to_canonical_json(&signable)
            .expect("Failed to serialize message")
            .into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::{AxisPhi, QpDigits};

    #[test]
    fn test_message_creation() {
        let parents = vec![MessageId::new(b"parent1"), MessageId::new(b"parent2")];
        let phi = AxisPhi::new(0, QpDigits::from_u64(42, 3, 5));
        let features = AdicFeatures::new(vec![phi]);
        let meta = AdicMeta::new(Utc::now());
        let pk = PublicKey::from_bytes([0; 32]);
        let data = b"test payload".to_vec();

        let msg = AdicMessage::new(parents.clone(), features, meta, pk, data);

        assert_eq!(msg.parent_count(), 2);
        assert!(!msg.is_genesis());
        assert!(msg.verify_id());
        assert!(!msg.has_value_transfer());
    }

    #[test]
    fn test_conflict_id() {
        let conflict = ConflictId::new("conflict-123".to_string());
        assert!(!conflict.is_none());

        let none = ConflictId::none();
        assert!(none.is_none());
    }
}
