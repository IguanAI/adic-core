use crate::{ChallengeError, FraudEvidence, Result};
use adic_types::{MessageId, PublicKey};
use serde::{Deserialize, Serialize};
use tracing::info;

/// Fraud proof submitted by a challenger
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FraudProof {
    pub id: MessageId,
    pub subject_id: MessageId,
    pub challenger: PublicKey,
    pub evidence: FraudEvidence,
    pub submitted_at_epoch: u64,
}

impl FraudProof {
    pub fn new(
        subject_id: MessageId,
        challenger: PublicKey,
        evidence: FraudEvidence,
        submitted_at_epoch: u64,
    ) -> Self {
        // Compute ID from proof contents
        let mut data = Vec::new();
        data.extend_from_slice(subject_id.as_bytes());
        data.extend_from_slice(challenger.as_bytes());
        data.extend_from_slice(&submitted_at_epoch.to_le_bytes());
        data.extend_from_slice(evidence.evidence_cid.as_bytes());

        let id = MessageId::new(&data);

        Self {
            id,
            subject_id,
            challenger,
            evidence,
            submitted_at_epoch,
        }
    }

    pub fn verify_structure(&self) -> Result<()> {
        // Basic structural validation
        if self.evidence.evidence_cid.is_empty() {
            return Err(ChallengeError::InvalidProof(
                "Evidence CID is empty".to_string(),
            ));
        }

        Ok(())
    }
}

/// Fraud proof verifier
pub struct FraudProofVerifier;

impl FraudProofVerifier {
    /// Verify a fraud proof structurally
    pub fn verify_structure(proof: &FraudProof) -> Result<()> {
        proof.verify_structure()
    }

    /// Verify fraud proof evidence against subject data
    /// This is application-specific and should be implemented by each domain
    pub fn verify_evidence(
        proof: &FraudProof,
        subject_data: &[u8],
        expected_data: &[u8],
    ) -> Result<bool> {
        // Compare provided evidence with actual data
        if subject_data != expected_data {
            info!(
                fraud_proof_id = hex::encode(proof.id.as_bytes()),
                subject_id = hex::encode(proof.subject_id.as_bytes()),
                challenger = hex::encode(proof.challenger.as_bytes()),
                "🚨 Fraud proof verified: data mismatch detected"
            );
            return Ok(true); // Fraud confirmed
        }

        info!(
            fraud_proof_id = hex::encode(proof.id.as_bytes()),
            subject_id = hex::encode(proof.subject_id.as_bytes()),
            "✅ Fraud proof rejected: data matches"
        );
        Ok(false) // No fraud
    }

    /// Compute hash of evidence for verification
    pub fn hash_evidence(data: &[u8]) -> [u8; 32] {
        *blake3::hash(data).as_bytes()
    }

    /// Verify evidence hash matches CID
    pub fn verify_evidence_hash(evidence: &FraudEvidence, actual_data: &[u8]) -> bool {
        let actual_hash = Self::hash_evidence(actual_data);
        let expected_cid = hex::encode(actual_hash);

        evidence.evidence_cid == expected_cid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FraudEvidence, FraudType};

    #[test]
    fn test_fraud_proof_creation() {
        let subject_id = MessageId::new(b"subject");
        let challenger = PublicKey::from_bytes([1; 32]);
        let evidence_data = b"fraud_evidence";

        let evidence = FraudEvidence {
            fraud_type: FraudType::DataCorruption,
            evidence_cid: hex::encode(FraudProofVerifier::hash_evidence(evidence_data)),
            evidence_data: Some(evidence_data.to_vec()),
            metadata: std::collections::HashMap::new(),
        };

        let proof = FraudProof::new(subject_id, challenger, evidence.clone(), 100);

        assert_eq!(proof.subject_id, subject_id);
        assert_eq!(proof.challenger, challenger);
        assert!(proof.verify_structure().is_ok());
    }

    #[test]
    fn test_evidence_verification() {
        let subject_data = b"wrong_data";
        let expected_data = b"correct_data";

        let proof = FraudProof::new(
            MessageId::new(b"subject"),
            PublicKey::from_bytes([1; 32]),
            FraudEvidence {
                fraud_type: FraudType::IncorrectProof,
                evidence_cid: "test".to_string(),
                evidence_data: None,
                metadata: std::collections::HashMap::new(),
            },
            100,
        );

        let is_fraud =
            FraudProofVerifier::verify_evidence(&proof, subject_data, expected_data).unwrap();

        assert!(is_fraud); // Data mismatch = fraud
    }

    #[test]
    fn test_evidence_hash_verification() {
        let data = b"test_data";
        let hash = FraudProofVerifier::hash_evidence(data);

        let evidence = FraudEvidence {
            fraud_type: FraudType::Custom("test".to_string()),
            evidence_cid: hex::encode(hash),
            evidence_data: Some(data.to_vec()),
            metadata: std::collections::HashMap::new(),
        };

        assert!(FraudProofVerifier::verify_evidence_hash(&evidence, data));
        assert!(!FraudProofVerifier::verify_evidence_hash(
            &evidence,
            b"wrong_data"
        ));
    }
}
