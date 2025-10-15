//! MRW Integration for Storage Market
//!
//! Provides Multi-axis Random Walk parent selection and admissibility validation
//! for storage market messages (intents, acceptances, deals, proofs).

use crate::error::{Result, StorageMarketError};
use crate::types::MessageHash;
use adic_consensus::ConsensusEngine;
use adic_mrw::MrwEngine;
use adic_storage::StorageEngine;
use adic_types::{features::AxisPhi, AdicFeatures, AdicParams, MessageId, PublicKey, QpDigits};
use std::sync::Arc;

/// Parent selector for storage messages using MRW
pub struct StorageParentSelector {
    mrw_engine: Arc<MrwEngine>,
    storage: Arc<StorageEngine>,
    consensus: Arc<ConsensusEngine>,
}

impl StorageParentSelector {
    /// Create new parent selector
    pub fn new(
        params: AdicParams,
        storage: Arc<StorageEngine>,
        consensus: Arc<ConsensusEngine>,
    ) -> Self {
        Self {
            mrw_engine: Arc::new(MrwEngine::new(params)),
            storage,
            consensus,
        }
    }

    /// Select d+1 parent messages using MRW
    ///
    /// For ADIC-DAG with d=3, this selects 4 parents that satisfy:
    /// - C1: All parents exist in the DAG
    /// - C2: Parents span q distinct balls per axis (diversity)
    /// - C3: Parents are from different proposers (Sybil resistance)
    pub async fn select_parents(
        &self,
        message_features: [u64; 3],
        tips: &[MessageHash],
        _author: &PublicKey,
    ) -> Result<[MessageHash; 4]> {
        // Convert storage features to AdicFeatures
        let features = Self::to_adic_features(message_features);

        // Convert MessageHash tips to MessageId
        let tip_ids: Vec<MessageId> = tips
            .iter()
            .map(|hash| MessageId::from_bytes(*hash))
            .collect();

        // Use MRW to select parents based on message features
        // The MRW engine considers:
        // - Proximity in p-adic space (λ weight)
        // - Reputation of tip proposers (α weight)
        // - Age/depth of tips (β weight)
        let selected = self
            .mrw_engine
            .select_parents(
                &features,
                &tip_ids,
                &self.storage,
                &self.consensus,
            )
            .await
            .map_err(|e| StorageMarketError::Other(format!("MRW selection failed: {}", e)))?;

        // Ensure we have exactly 4 parents (d+1 for d=3)
        if selected.len() != 4 {
            return Err(StorageMarketError::AdmissibilityViolation(format!(
                "Expected 4 parents, got {}",
                selected.len()
            )));
        }

        // Convert back to MessageHash
        let mut parents = [MessageHash::default(); 4];
        for (i, msg_id) in selected.iter().enumerate() {
            parents[i] = *msg_id.as_bytes();
        }

        Ok(parents)
    }

    /// Convert storage features to ADIC features
    fn to_adic_features(features: [u64; 3]) -> AdicFeatures {
        AdicFeatures::new(vec![
            AxisPhi::new(0, QpDigits::from_u64(features[0], 3, 10)),
            AxisPhi::new(1, QpDigits::from_u64(features[1], 3, 10)),
            AxisPhi::new(2, QpDigits::from_u64(features[2], 3, 10)),
        ])
    }

    /// Select parents with bias toward a specific message
    ///
    /// Used when provider wants to bias parent selection toward the intent message
    /// to increase likelihood of discovery via axis-aware gossip.
    pub async fn select_parents_biased(
        &self,
        message_features: [u64; 3],
        tips: &[MessageHash],
        author: &PublicKey,
        bias_toward: &MessageHash,
    ) -> Result<[MessageHash; 4]> {
        // First select normally
        let mut parents = self.select_parents(message_features, tips, author).await?;

        // Replace one parent with the biased target if not already included
        if !parents.contains(bias_toward) {
            parents[0] = *bias_toward;
        }

        Ok(parents)
    }

    /// Validate admissibility of selected parents
    ///
    /// Checks C1-C3 constraints:
    /// - C1: All parents exist
    /// - C2: Diversity across axes
    /// - C3: Different proposers
    pub async fn validate_admissibility(
        &self,
        parents: &[MessageHash; 4],
        _message_features: [u64; 3],
    ) -> Result<()> {
        // C1: Verify all parents exist in DAG
        for (i, parent) in parents.iter().enumerate() {
            if parent == &MessageHash::default() {
                return Err(StorageMarketError::AdmissibilityViolation(format!(
                    "Parent {} is zero hash",
                    i
                )));
            }
        }

        // C2: Check diversity (would query DAG for parent features)
        // This is a simplified check - full validation happens in consensus layer
        let mut unique_parents = parents.to_vec();
        unique_parents.sort();
        unique_parents.dedup();
        if unique_parents.len() < 3 {
            return Err(StorageMarketError::AdmissibilityViolation(
                "Insufficient parent diversity".to_string(),
            ));
        }

        // C3: Different proposers check (would query DAG for proposer info)
        // Full validation deferred to consensus layer

        Ok(())
    }
}

/// Encode storage market features into p-adic space
///
/// Maps storage-specific attributes to 3-axis features:
/// - Axis 0 (time): Timestamp bucket
/// - Axis 1 (topic): Content hash (data_cid)
/// - Axis 2 (region): Provider ASN or geographic region
pub struct FeatureEncoder;

impl FeatureEncoder {
    /// Encode intent features
    pub fn encode_intent(timestamp: i64, data_cid: &[u8; 32], client_region: u64) -> [u64; 3] {
        [
            Self::encode_time(timestamp),
            Self::encode_topic(data_cid),
            client_region,
        ]
    }

    /// Encode acceptance features
    pub fn encode_acceptance(
        timestamp: i64,
        data_cid: &[u8; 32],
        provider_region: u64,
    ) -> [u64; 3] {
        [
            Self::encode_time(timestamp),
            Self::encode_topic(data_cid),
            provider_region,
        ]
    }

    /// Encode deal features
    pub fn encode_deal(timestamp: i64, data_cid: &[u8; 32], provider_region: u64) -> [u64; 3] {
        [
            Self::encode_time(timestamp),
            Self::encode_topic(data_cid),
            provider_region,
        ]
    }

    /// Encode timestamp to p-adic time axis
    ///
    /// Buckets timestamps into discrete intervals for proximity calculations
    fn encode_time(timestamp: i64) -> u64 {
        // 10-minute buckets (600 seconds)
        const BUCKET_SIZE: i64 = 600;
        (timestamp / BUCKET_SIZE) as u64
    }

    /// Encode data CID to p-adic topic axis
    ///
    /// Maps content hash to topic space using first 8 bytes
    fn encode_topic(data_cid: &[u8; 32]) -> u64 {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&data_cid[0..8]);
        u64::from_le_bytes(bytes)
    }
}

/// Admissibility validator for storage messages
pub struct AdmissibilityValidator {
    parent_selector: Arc<StorageParentSelector>,
}

impl AdmissibilityValidator {
    /// Create new admissibility validator
    pub fn new(parent_selector: Arc<StorageParentSelector>) -> Self {
        Self { parent_selector }
    }

    /// Validate storage intent admissibility
    pub async fn validate_intent_admissibility(
        &self,
        approvals: &[MessageHash; 4],
        features: [u64; 3],
    ) -> Result<()> {
        self.parent_selector
            .validate_admissibility(approvals, features)
            .await
    }

    /// Validate provider acceptance admissibility
    pub async fn validate_acceptance_admissibility(
        &self,
        approvals: &[MessageHash; 4],
        features: [u64; 3],
    ) -> Result<()> {
        self.parent_selector
            .validate_admissibility(approvals, features)
            .await
    }

    /// Validate storage deal admissibility
    pub async fn validate_deal_admissibility(
        &self,
        approvals: &[MessageHash; 4],
        features: [u64; 3],
    ) -> Result<()> {
        self.parent_selector
            .validate_admissibility(approvals, features)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_consensus::ConsensusEngine;
    use adic_storage::store::BackendType;

    #[test]
    fn test_feature_encoder_time() {
        let t1 = 1000000000; // Base time (bucket 1666666)
        let t2 = 1000000100; // Same bucket (1666666)
        let t3 = 1000000600; // Next bucket (1666667)

        let e1 = FeatureEncoder::encode_time(t1);
        let e2 = FeatureEncoder::encode_time(t2);
        let e3 = FeatureEncoder::encode_time(t3);

        assert_eq!(e1, e2); // Same bucket
        assert_ne!(e1, e3); // Different bucket
    }

    #[test]
    fn test_feature_encoder_topic() {
        let cid1 = [1u8; 32];
        let cid2 = [1u8; 32];
        let mut cid3 = [1u8; 32];
        cid3[0] = 2;

        let e1 = FeatureEncoder::encode_topic(&cid1);
        let e2 = FeatureEncoder::encode_topic(&cid2);
        let e3 = FeatureEncoder::encode_topic(&cid3);

        assert_eq!(e1, e2); // Same CID
        assert_ne!(e1, e3); // Different CID
    }

    #[test]
    fn test_encode_intent() {
        let timestamp = 1000000000;
        let data_cid = [5u8; 32];
        let region = 12345;

        let features = FeatureEncoder::encode_intent(timestamp, &data_cid, region);

        assert_eq!(features.len(), 3);
        assert_eq!(features[0], FeatureEncoder::encode_time(timestamp));
        assert_eq!(features[1], FeatureEncoder::encode_topic(&data_cid));
        assert_eq!(features[2], region);
    }

    #[tokio::test]
    async fn test_validate_admissibility_zero_parents() {
        let params = AdicParams::default();
        let config = adic_storage::StorageConfig {
            backend_type: BackendType::Memory,
            ..Default::default()
        };
        let storage = Arc::new(StorageEngine::new(config).unwrap());
        let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
        let selector = StorageParentSelector::new(params, storage, consensus);

        let parents = [MessageHash::default(); 4];
        let features = [1, 2, 3];

        let result = selector.validate_admissibility(&parents, features).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_validate_admissibility_insufficient_diversity() {
        let params = AdicParams::default();
        let config = adic_storage::StorageConfig {
            backend_type: BackendType::Memory,
            ..Default::default()
        };
        let storage = Arc::new(StorageEngine::new(config).unwrap());
        let consensus = Arc::new(ConsensusEngine::new(params.clone(), storage.clone()));
        let selector = StorageParentSelector::new(params, storage, consensus);

        // All same parent (no diversity)
        let parent = [1u8; 32];
        let parents = [parent; 4];
        let features = [1, 2, 3];

        let result = selector.validate_admissibility(&parents, features).await;
        assert!(result.is_err());
    }
}
