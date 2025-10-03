//! Value transfer types for ADIC messages
//!
//! This module defines the types used for transferring value (ADIC tokens)
//! within messages on the tangle/DAG. Transfers are now part of the consensus
//! system rather than a separate transaction layer.

use serde::{Deserialize, Serialize};

/// Represents a value transfer within an ADIC message
///
/// When a message includes a `ValueTransfer`, it represents an atomic transfer
/// of ADIC tokens from one account to another. The transfer is validated by
/// consensus and included in the DAG, ensuring all value movements benefit from
/// the security properties of the tangle structure.
///
/// # Security
///
/// - The `nonce` field prevents replay attacks
/// - Signature must be from the `from` address (validated separately)
/// - Balance checks are performed before accepting the message
/// - Double-spends create conflict sets resolved by energy descent
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ValueTransfer {
    /// Source account for the transfer
    /// Must have signature from this address
    pub from: Vec<u8>, // Will use AccountAddress after imports resolved

    /// Destination account for the transfer
    pub to: Vec<u8>,

    /// Amount to transfer in base units
    pub amount: u64,

    /// Nonce to prevent replay attacks
    /// Must be unique per sender to prevent duplicate transfers
    pub nonce: u64,
}

impl ValueTransfer {
    /// Create a new value transfer
    pub fn new(from: Vec<u8>, to: Vec<u8>, amount: u64, nonce: u64) -> Self {
        Self {
            from,
            to,
            amount,
            nonce,
        }
    }

    /// Check if this is a valid transfer (non-zero amount, different addresses)
    pub fn is_valid(&self) -> bool {
        self.amount > 0 && self.from != self.to && self.from.len() == 32 && self.to.len() == 32
    }

    /// Get a stable byte representation for hashing
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.from);
        bytes.extend_from_slice(&self.to);
        bytes.extend_from_slice(&self.amount.to_le_bytes());
        bytes.extend_from_slice(&self.nonce.to_le_bytes());
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_transfer_creation() {
        let from = vec![1u8; 32];
        let to = vec![2u8; 32];
        let transfer = ValueTransfer::new(from.clone(), to.clone(), 1000, 1);

        assert_eq!(transfer.from, from);
        assert_eq!(transfer.to, to);
        assert_eq!(transfer.amount, 1000);
        assert_eq!(transfer.nonce, 1);
    }

    #[test]
    fn test_value_transfer_validation() {
        let from = vec![1u8; 32];
        let to = vec![2u8; 32];

        // Valid transfer
        let valid = ValueTransfer::new(from.clone(), to.clone(), 1000, 1);
        assert!(valid.is_valid());

        // Zero amount - invalid
        let zero_amount = ValueTransfer::new(from.clone(), to.clone(), 0, 1);
        assert!(!zero_amount.is_valid());

        // Same address - invalid
        let same_addr = ValueTransfer::new(from.clone(), from.clone(), 1000, 1);
        assert!(!same_addr.is_valid());

        // Wrong address length - invalid
        let wrong_len = ValueTransfer::new(vec![1u8; 16], to.clone(), 1000, 1);
        assert!(!wrong_len.is_valid());
    }

    #[test]
    fn test_value_transfer_serialization() {
        let from = vec![1u8; 32];
        let to = vec![2u8; 32];
        let transfer = ValueTransfer::new(from, to, 1000, 1);

        // Serialize
        let serialized = serde_json::to_string(&transfer).unwrap();
        assert!(!serialized.is_empty());

        // Deserialize
        let deserialized: ValueTransfer = serde_json::from_str(&serialized).unwrap();
        assert_eq!(transfer, deserialized);
    }

    #[test]
    fn test_value_transfer_to_bytes() {
        let from = vec![1u8; 32];
        let to = vec![2u8; 32];
        let transfer = ValueTransfer::new(from.clone(), to.clone(), 1000, 1);

        let bytes = transfer.to_bytes();

        // Should be: 32 (from) + 32 (to) + 8 (amount) + 8 (nonce) = 80 bytes
        assert_eq!(bytes.len(), 80);

        // Verify components
        assert_eq!(&bytes[0..32], &from[..]);
        assert_eq!(&bytes[32..64], &to[..]);
    }

    #[test]
    fn test_value_transfer_equality() {
        let from = vec![1u8; 32];
        let to = vec![2u8; 32];

        let transfer1 = ValueTransfer::new(from.clone(), to.clone(), 1000, 1);
        let transfer2 = ValueTransfer::new(from.clone(), to.clone(), 1000, 1);
        let transfer3 = ValueTransfer::new(from.clone(), to.clone(), 1000, 2); // Different nonce

        assert_eq!(transfer1, transfer2);
        assert_ne!(transfer1, transfer3);
    }
}
