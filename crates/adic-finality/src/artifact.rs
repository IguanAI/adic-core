use adic_types::{MessageId, PublicKey, Signature};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Type of finality gate used
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinalityGate {
    F1KCore,
    F2PersistentHomology,
    SSF,
}

/// Witness data for finality
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalityWitness {
    pub kcore_root: Option<MessageId>,
    pub depth: u32,
    pub diversity_ok: bool,
    pub reputation_sum: f64,
    pub distinct_balls: HashMap<u32, usize>,
    pub core_size: usize,
}

/// Validator attestation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorAttestation {
    pub public_key: PublicKey,
    pub signature: Signature,
    pub timestamp: i64,
}

/// Finality artifact certifying a message is final
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalityArtifact {
    pub intent_id: MessageId,
    pub gate: FinalityGate,
    pub params: FinalityParams,
    pub witness: FinalityWitness,
    pub validators: Vec<ValidatorAttestation>,
    pub timestamp: i64,
}

/// Parameters used for finality determination
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalityParams {
    pub k: usize,
    pub q: usize,
    pub depth_star: u32,
    pub r_sum_min: f64,
}

impl FinalityArtifact {
    pub fn new(
        intent_id: MessageId,
        gate: FinalityGate,
        params: FinalityParams,
        witness: FinalityWitness,
    ) -> Self {
        Self {
            intent_id,
            gate,
            params,
            witness,
            validators: Vec::new(),
            timestamp: chrono::Utc::now().timestamp(),
        }
    }

    pub fn add_attestation(&mut self, attestation: ValidatorAttestation) {
        self.validators.push(attestation);
    }

    pub fn is_valid(&self) -> bool {
        // Check basic validity
        self.witness.diversity_ok &&
        self.witness.depth >= self.params.depth_star &&
        self.witness.reputation_sum >= self.params.r_sum_min
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn compute_hash(&self) -> MessageId {
        let data = serde_json::to_vec(self).unwrap_or_default();
        MessageId::new(&data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_finality_artifact_creation() {
        let intent_id = MessageId::new(b"test_message");
        let params = FinalityParams {
            k: 20,
            q: 3,
            depth_star: 12,
            r_sum_min: 4.0,
        };
        let witness = FinalityWitness {
            kcore_root: Some(intent_id),
            depth: 15,
            diversity_ok: true,
            reputation_sum: 10.5,
            distinct_balls: [(0, 3), (1, 4), (2, 3)].into_iter().collect(),
            core_size: 25,
        };

        let artifact = FinalityArtifact::new(
            intent_id,
            FinalityGate::F1KCore,
            params,
            witness,
        );

        assert_eq!(artifact.intent_id, intent_id);
        assert_eq!(artifact.gate, FinalityGate::F1KCore);
        assert!(artifact.is_valid());
    }

    #[test]
    fn test_artifact_serialization() {
        let intent_id = MessageId::new(b"test");
        let params = FinalityParams {
            k: 20,
            q: 3,
            depth_star: 12,
            r_sum_min: 4.0,
        };
        let witness = FinalityWitness {
            kcore_root: Some(intent_id),
            depth: 15,
            diversity_ok: true,
            reputation_sum: 10.5,
            distinct_balls: HashMap::new(),
            core_size: 25,
        };

        let artifact = FinalityArtifact::new(
            intent_id,
            FinalityGate::F1KCore,
            params,
            witness,
        );

        let json = artifact.to_json().unwrap();
        let restored = FinalityArtifact::from_json(&json).unwrap();
        
        assert_eq!(artifact.intent_id, restored.intent_id);
        assert_eq!(artifact.gate, restored.gate);
    }
}
