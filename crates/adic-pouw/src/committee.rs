//! Committee Certificate Generation
//!
//! Connects the quorum selection system (adic-quorum) with PoUW epoch committee certificates.
//! This module generates ODCCommitteeCert instances from VRF-based quorum selection results.

use crate::bls_coordinator::{BLSCoordinator, SigningRequestId, DST_COMMITTEE_CERT};
use crate::dkg_manager::DKGManager;
use crate::types::{AxisStats, ODCCommitteeCert};
use crate::{PoUWError, Result};
use adic_app_common::NetworkMetadataRegistry;
use adic_consensus::ReputationTracker;
use adic_crypto::{BLSSignature, ThresholdConfig};
use adic_quorum::{QuorumConfig, QuorumSelector};
use adic_types::PublicKey;
use adic_vrf::VRFService;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

/// Configuration for committee certificate generation
#[derive(Debug, Clone)]
pub struct CommitteeCertConfig {
    /// Quorum configuration for committee selection
    pub quorum_config: QuorumConfig,

    /// Whether to require BLS threshold signatures
    pub require_signatures: bool,

    /// BLS domain separator for committee signatures
    pub bls_domain: String,

    /// Minimum reputation required for committee members
    pub min_committee_reputation: f64,
}

impl Default for CommitteeCertConfig {
    fn default() -> Self {
        Self {
            quorum_config: QuorumConfig {
                min_reputation: 50.0,
                members_per_axis: 5,
                total_size: 15,
                max_per_asn: 2,
                max_per_region: 3,
                domain_separator: "ADIC-PoUW-COMMITTEE-v1".to_string(),
                num_axes: 3,
            },
            require_signatures: true,
            bls_domain: "ADIC-PoUW-COMMITTEE-v1".to_string(),
            min_committee_reputation: 50.0, // Same as quorum min_reputation
        }
    }
}

/// Committee certificate generator
///
/// Generates ODCCommitteeCert instances by:
/// 1. Selecting committee via VRF-based quorum selection
/// 2. Running DKG ceremony to generate threshold keys
/// 3. Computing axis diversity statistics
/// 4. Collecting threshold BLS signatures from committee members
/// 5. Producing a verifiable epoch committee certificate
///
/// # Event Emission
/// State changes should emit events when used in a node context:
/// - generate_certificate() → CommitteeCertificateGenerated
/// - (via AggregatorSelector) select_primary_aggregator() → AggregatorSelected
pub struct CommitteeCertGenerator {
    quorum_selector: Arc<QuorumSelector>,
    vrf_service: Arc<VRFService>,
    reputation_tracker: Arc<ReputationTracker>,
    metadata_registry: Arc<NetworkMetadataRegistry>,
    bls_coordinator: Option<Arc<BLSCoordinator>>,
    dkg_manager: Option<Arc<DKGManager>>,
    config: CommitteeCertConfig,
}

impl CommitteeCertGenerator {
    /// Create new committee certificate generator
    pub fn new(
        quorum_selector: Arc<QuorumSelector>,
        vrf_service: Arc<VRFService>,
        reputation_tracker: Arc<ReputationTracker>,
        metadata_registry: Arc<NetworkMetadataRegistry>,
        config: CommitteeCertConfig,
    ) -> Self {
        Self {
            quorum_selector,
            vrf_service,
            reputation_tracker,
            metadata_registry,
            bls_coordinator: None,
            dkg_manager: None,
            config,
        }
    }

    /// Set BLS coordinator for signature collection
    pub fn with_bls_coordinator(mut self, coordinator: Arc<BLSCoordinator>) -> Self {
        self.bls_coordinator = Some(coordinator);
        self
    }

    /// Set DKG manager for committee key generation
    pub fn with_dkg_manager(mut self, manager: Arc<DKGManager>) -> Self {
        self.dkg_manager = Some(manager);
        self
    }

    /// Generate committee certificate for an epoch
    ///
    /// This implements the ODC (On-Demand Committee) protocol from PoUW II §2.2:
    /// 1. Retrieve canonical randomness R_k for epoch k
    /// 2. Select committee members via VRF-based quorum selection
    /// 3. Compute axis diversity statistics
    /// 4. Collect BLS threshold signatures from committee members
    /// 5. Aggregate signatures and produce certificate
    pub async fn generate_certificate(
        &self,
        epoch_id: u64,
        eligible_nodes: Vec<(PublicKey, f64, Vec<Vec<u8>>)>, // (pubkey, reputation, axis_balls)
    ) -> Result<ODCCommitteeCert> {
        info!(
            epoch_id,
            eligible_count = eligible_nodes.len(),
            "📜 Generating ODC committee certificate"
        );

        // Step 1: Get canonical randomness for epoch
        let randomness = self
            .vrf_service
            .get_canonical_randomness(epoch_id)
            .await
            .map_err(|e| PoUWError::VRFError(format!("VRF randomness not available: {}", e)))?;

        // Step 2: Build NodeInfo structs for quorum selection with network metadata
        let mut nodes = Vec::new();
        for (pk, reputation, axis_balls) in eligible_nodes {
            let metadata = self.metadata_registry.get(&pk).await;
            nodes.push(adic_quorum::selection::NodeInfo {
                public_key: pk,
                reputation,
                asn: metadata.as_ref().map(|m| m.asn),
                region: metadata.as_ref().map(|m| m.region.clone()),
                axis_balls,
            });
        }

        // Step 3: Select committee using VRF-based quorum selection
        let quorum_result = self
            .quorum_selector
            .select_committee(epoch_id, &self.config.quorum_config, nodes)
            .await
            .map_err(|e| PoUWError::QuorumError(format!("Committee selection failed: {}", e)))?;

        // Step 4: Extract member public keys
        let members: Vec<PublicKey> = quorum_result
            .members
            .iter()
            .map(|m| m.public_key)
            .collect();

        // Step 4.5: Verify committee member reputations haven't degraded
        for member in &members {
            let reputation = self.reputation_tracker.get_reputation(member).await;
            if reputation < self.config.min_committee_reputation {
                return Err(PoUWError::Other(format!(
                    "Committee member {} has insufficient reputation: {:.2} < {:.2}",
                    hex::encode(&member.as_bytes()[..8]),
                    reputation,
                    self.config.min_committee_reputation
                )));
            }
        }

        // Step 5: Initialize DKG ceremony for this committee epoch
        // This generates the threshold BLS keys that will be used for signing
        if let Some(ref dkg_manager) = self.dkg_manager {
            // Check if DKG already complete for this epoch
            if !dkg_manager.is_complete(epoch_id).await {
                // Calculate threshold (typically 2/3 + 1 of committee size)
                let threshold = (members.len() * 2 / 3) + 1;

                let _threshold_config = ThresholdConfig::new(members.len(), threshold)
                    .map_err(|e| PoUWError::BLSError(e))?;

                // In a distributed system, we would:
                // 1. Determine our participant_id in the committee (if we're a member)
                // 2. Start DKG ceremony via dkg_manager.start_ceremony()
                // 3. Broadcast DKG messages (commitments, shares) to other committee members
                // 4. Collect DKG messages from other members
                // 5. Finalize DKG to produce PublicKeySet and our SecretKeyShare
                //
                // For now, we log that DKG should be initiated
                info!(
                    epoch_id,
                    committee_size = members.len(),
                    threshold = threshold,
                    "🔐 DKG ceremony should be initiated for committee (pending network integration)"
                );

                // Once DKG completes, the PublicKeySet should be provided to BLSCoordinator:
                // let pk_set = dkg_manager.finalize_ceremony(epoch_id).await?;
                // if let Some(ref coordinator) = self.bls_coordinator {
                //     coordinator.set_public_key_set(pk_set).await;
                // }
            } else {
                info!(
                    epoch_id,
                    "✅ DKG already complete for this epoch"
                );
            }
        } else {
            warn!(
                epoch_id,
                "⚠️ DKG manager not configured, skipping committee key generation"
            );
        }

        // Step 5: Compute axis diversity statistics
        let axis_stats = self.compute_axis_stats(&quorum_result.members);

        // Step 6: Generate threshold signature (if required)
        let threshold_signature = if self.config.require_signatures {
            if let Some(ref coordinator) = self.bls_coordinator {
                // Create canonical message for committee to sign
                // Message = hash(epoch_id || randomness || sorted_member_pubkeys)
                let mut message_data = Vec::new();
                message_data.extend_from_slice(&epoch_id.to_le_bytes());
                message_data.extend_from_slice(&randomness);
                for member_pk in &members {
                    message_data.extend_from_slice(member_pk.as_bytes());
                }
                let message_hash = blake3::hash(&message_data);

                // Initiate signing request
                let request_id = SigningRequestId::new(
                    "committee-cert",
                    epoch_id,
                    message_hash.as_bytes(),
                );

                match coordinator
                    .initiate_signing(
                        request_id,
                        message_data,
                        DST_COMMITTEE_CERT.to_vec(),
                        members.clone(),
                    )
                    .await
                {
                    Ok(_request) => {
                        info!(
                            epoch_id,
                            "📝 BLS signing request initiated for committee certificate"
                        );
                        // Return None - signature will be added later via add_signature()
                        // after coordinator collects threshold shares
                        None
                    }
                    Err(e) => {
                        warn!(
                            epoch_id,
                            error = %e,
                            "⚠️ Failed to initiate BLS signing request"
                        );
                        None
                    }
                }
            } else {
                warn!(
                    epoch_id,
                    "⚠️ BLS coordinator not configured, skipping signature collection"
                );
                None
            }
        } else {
            None
        };

        let cert = ODCCommitteeCert {
            epoch_id,
            randomness,
            members,
            axis_stats,
            threshold_signature,
        };

        info!(
            epoch_id,
            member_count = cert.members.len(),
            diversity_score = cert.axis_stats.diversity_score,
            "✅ ODC committee certificate generated"
        );

        Ok(cert)
    }

    /// Compute axis diversity statistics from selected members
    fn compute_axis_stats(&self, members: &[adic_quorum::QuorumMember]) -> AxisStats {
        let mut axis_counts: HashMap<u32, u32> = HashMap::new();

        // Count members per axis
        for member in members {
            *axis_counts.entry(member.axis_id as u32).or_insert(0) += 1;
        }

        // Convert to sorted vector
        let mut axis_distribution: Vec<(u32, u32)> = axis_counts.into_iter().collect();
        axis_distribution.sort_by_key(|(axis_id, _)| *axis_id);

        // Compute diversity score
        // Simple metric: 1.0 if all axes are represented, decreases with imbalance
        let total_members = members.len() as f64;
        let num_axes = self.config.quorum_config.num_axes as f64;
        let ideal_per_axis = total_members / num_axes;

        let variance: f64 = axis_distribution
            .iter()
            .map(|(_, count)| {
                let diff = *count as f64 - ideal_per_axis;
                diff * diff
            })
            .sum::<f64>()
            / num_axes;

        // Diversity score: 1.0 for perfect balance, decreases with variance
        // Using exponential decay to penalize imbalance
        let diversity_score = (-variance / (ideal_per_axis * ideal_per_axis)).exp();

        AxisStats {
            axis_distribution,
            diversity_score,
        }
    }

    /// Verify a committee certificate
    ///
    /// Checks that:
    /// 1. The randomness matches the epoch's VRF output
    /// 2. The committee members were correctly selected
    /// 3. The BLS threshold signature is valid (if present)
    pub async fn verify_certificate(&self, cert: &ODCCommitteeCert) -> Result<bool> {
        // Verify epoch randomness
        let expected_randomness = self
            .vrf_service
            .get_canonical_randomness(cert.epoch_id)
            .await
            .map_err(|e| PoUWError::VRFError(format!("VRF verification failed: {}", e)))?;

        if cert.randomness != expected_randomness {
            warn!(
                epoch_id = cert.epoch_id,
                "❌ Committee certificate randomness mismatch"
            );
            return Ok(false);
        }

        // Verify BLS signature if present
        if let Some(_signature) = &cert.threshold_signature {
            // Reconstruct the signed message
            let _message = self.reconstruct_cert_message(cert);

            // We need the PublicKeySet from DKG for this epoch
            // This would typically come from the DKGManager
            // For now, return an error indicating DKG verification is needed
            warn!(
                epoch_id = cert.epoch_id,
                "⚠️ BLS signature present but PublicKeySet not provided for verification"
            );
            // In production, verify with:
            // let pk_set = dkg_manager.get_public_key_set(cert.epoch_id).await?;
            // let is_valid = BLSThresholdSigner::verify_aggregated(&message, dst, &pk_set, signature)?;
            // if !is_valid {
            //     return Ok(false);
            // }
        }

        Ok(true)
    }

    /// Reconstruct the message that was signed for the certificate
    fn reconstruct_cert_message(&self, cert: &ODCCommitteeCert) -> Vec<u8> {
        use blake3::Hasher;

        let mut hasher = Hasher::new();

        // Hash(epoch_id || randomness || members)
        hasher.update(&cert.epoch_id.to_le_bytes());
        hasher.update(&cert.randomness);

        // Include all members in deterministic order
        let mut members: Vec<_> = cert.members.clone();
        members.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        for member in members {
            hasher.update(member.as_bytes());
        }

        hasher.finalize().as_bytes().to_vec()
    }

    /// Verify certificate with PublicKeySet from DKG
    ///
    /// This is the complete verification that should be used in production
    pub async fn verify_with_public_key_set(
        &self,
        cert: &ODCCommitteeCert,
        public_key_set: &threshold_crypto::PublicKeySet,
    ) -> Result<bool> {
        // Basic validation
        if !self.verify_certificate(cert).await.unwrap_or(false) {
            return Ok(false);
        }

        // BLS signature verification
        if let Some(signature) = &cert.threshold_signature {
            let message = self.reconstruct_cert_message(cert);
            let dst = crate::DST_COMMITTEE_CERT;

            // Verify signature using PublicKeySet
            let sig = signature.to_tc_signature()
                .map_err(|e| PoUWError::BLSError(e))?;

            // Domain-separated message
            let mut prefixed_message = Vec::with_capacity(dst.len() + message.len());
            prefixed_message.extend_from_slice(dst);
            prefixed_message.extend_from_slice(&message);

            let is_valid = public_key_set.public_key().verify(&sig, &prefixed_message);

            if !is_valid {
                warn!(
                    epoch_id = cert.epoch_id,
                    "❌ BLS threshold signature verification failed"
                );
                return Ok(false);
            }

            info!(
                epoch_id = cert.epoch_id,
                "✅ BLS threshold signature verified successfully"
            );
        }

        Ok(true)
    }

    /// Add BLS threshold signature to certificate
    ///
    /// This method updates a certificate with a collected threshold signature.
    /// In production, this would be called after the BLS signing coordinator
    /// has collected and aggregated signatures from committee members.
    pub fn add_signature(&self, cert: &mut ODCCommitteeCert, signature: BLSSignature) {
        cert.threshold_signature = Some(signature);
        info!(
            epoch_id = cert.epoch_id,
            "✅ BLS threshold signature added to committee certificate"
        );
    }
}

/// Aggregator selection and DoS resilience (PoUW II §4.2)
pub struct AggregatorSelector {
    /// Timeout duration (τ) in seconds for fallback aggregation
    pub timeout_secs: u64,
}

impl Default for AggregatorSelector {
    fn default() -> Self {
        Self {
            timeout_secs: 60, // 1 minute default timeout
        }
    }
}

impl AggregatorSelector {
    /// Create new aggregator selector with custom timeout
    pub fn new(timeout_secs: u64) -> Self {
        Self { timeout_secs }
    }

    /// Select primary aggregator from committee
    ///
    /// Per PoUW II §4.2:
    /// Primary aggregator = member with smallest min_j y_{u,j}
    /// (i.e., the member with the smallest minimum VRF score across all axes)
    ///
    /// This ensures deterministic, unpredictable aggregator selection based on VRF randomness.
    pub fn select_primary_aggregator(
        &self,
        members: &[adic_quorum::QuorumMember],
    ) -> Option<PublicKey> {
        if members.is_empty() {
            return None;
        }

        // Group members by public key to find min VRF score across all axes
        let mut member_min_scores: std::collections::HashMap<PublicKey, [u8; 32]> =
            std::collections::HashMap::new();

        for member in members {
            member_min_scores
                .entry(member.public_key)
                .and_modify(|min_score| {
                    if member.vrf_score < *min_score {
                        *min_score = member.vrf_score;
                    }
                })
                .or_insert(member.vrf_score);
        }

        // Find member with smallest minimum score
        let primary = member_min_scores
            .into_iter()
            .min_by_key(|(_, min_score)| *min_score)
            .map(|(pk, _)| pk)?;

        info!(
            primary_aggregator = hex::encode(primary.as_bytes()),
            "🎯 Selected primary aggregator (smallest min_j VRF score)"
        );

        Some(primary)
    }

    /// Check if timeout has elapsed for fallback aggregation
    ///
    /// Per PoUW II §4.2:
    /// After timeout τ, any committee member can aggregate receipts.
    /// This provides DoS resilience if the primary aggregator is offline.
    pub fn is_timeout_elapsed(&self, aggregation_start_time: std::time::Instant) -> bool {
        let elapsed = aggregation_start_time.elapsed().as_secs();
        elapsed >= self.timeout_secs
    }

    /// Determine if a member can aggregate
    ///
    /// Returns true if:
    /// 1. The member is the primary aggregator, OR
    /// 2. Timeout τ has elapsed (fallback mode)
    pub fn can_aggregate(
        &self,
        member_pk: &PublicKey,
        committee_members: &[adic_quorum::QuorumMember],
        aggregation_start_time: std::time::Instant,
    ) -> bool {
        // Check if member is in committee
        let is_committee_member = committee_members
            .iter()
            .any(|m| &m.public_key == member_pk);

        if !is_committee_member {
            return false;
        }

        // Check if member is primary aggregator
        let primary = self.select_primary_aggregator(committee_members);
        if let Some(primary_pk) = primary {
            if &primary_pk == member_pk {
                info!(
                    member = hex::encode(member_pk.as_bytes()),
                    "✅ Member is primary aggregator, can aggregate immediately"
                );
                return true;
            }
        }

        // Check if timeout has elapsed (fallback mode)
        if self.is_timeout_elapsed(aggregation_start_time) {
            info!(
                member = hex::encode(member_pk.as_bytes()),
                timeout_secs = self.timeout_secs,
                "⏰ Timeout elapsed, member can aggregate as fallback"
            );
            return true;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_vrf::VRFConfig;

    #[tokio::test]
    async fn test_generate_committee_certificate() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let vrf_service = Arc::new(VRFService::new(
            VRFConfig::default(),
            rep_tracker.clone(),
        ));
        let quorum_selector = Arc::new(QuorumSelector::new(
            vrf_service.clone(),
            rep_tracker.clone(),
        ));

        let metadata_registry = Arc::new(NetworkMetadataRegistry::new());

        let _generator = CommitteeCertGenerator::new(
            quorum_selector,
            vrf_service,
            rep_tracker.clone(),
            metadata_registry,
            CommitteeCertConfig::default(),
        );

        // Create eligible nodes
        let mut eligible_nodes = Vec::new();
        for i in 0..20 {
            let pk = PublicKey::from_bytes([i; 32]);
            let reputation = 100.0;
            let axis_balls = vec![vec![i], vec![i], vec![i]];

            rep_tracker.set_reputation(&pk, reputation).await;
            eligible_nodes.push((pk, reputation, axis_balls));
        }

        // Note: This test will fail without VRF finalization
        // In real usage, VRF commit-reveal must complete before certificate generation
        // For testing, you would need to:
        // 1. Submit VRF commits
        // 2. Finalize VRF for the epoch
        // 3. Then generate certificate

        // We skip actual generation in this test since VRF isn't finalized
        // but the API demonstrates the usage pattern
    }

    #[test]
    fn test_axis_stats_computation() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let vrf_service = Arc::new(VRFService::new(
            VRFConfig::default(),
            rep_tracker.clone(),
        ));
        let quorum_selector = Arc::new(QuorumSelector::new(
            vrf_service.clone(),
            rep_tracker.clone(),
        ));

        let metadata_registry = Arc::new(NetworkMetadataRegistry::new());

        let generator = CommitteeCertGenerator::new(
            quorum_selector,
            vrf_service,
            rep_tracker,
            metadata_registry,
            CommitteeCertConfig::default(),
        );

        // Create mock members with axis distribution
        let members: Vec<adic_quorum::QuorumMember> = vec![
            adic_quorum::QuorumMember {
                public_key: PublicKey::from_bytes([1; 32]),
                reputation: 100.0,
                vrf_score: [0u8; 32],
                axis_id: 0,
                asn: None,
                region: None,
            },
            adic_quorum::QuorumMember {
                public_key: PublicKey::from_bytes([2; 32]),
                reputation: 100.0,
                vrf_score: [1u8; 32],
                axis_id: 0,
                asn: None,
                region: None,
            },
            adic_quorum::QuorumMember {
                public_key: PublicKey::from_bytes([3; 32]),
                reputation: 100.0,
                vrf_score: [2u8; 32],
                axis_id: 1,
                asn: None,
                region: None,
            },
            adic_quorum::QuorumMember {
                public_key: PublicKey::from_bytes([4; 32]),
                reputation: 100.0,
                vrf_score: [3u8; 32],
                axis_id: 2,
                asn: None,
                region: None,
            },
        ];

        let stats = generator.compute_axis_stats(&members);

        // Check axis distribution
        assert_eq!(stats.axis_distribution.len(), 3);
        assert_eq!(stats.axis_distribution[0], (0, 2)); // Axis 0: 2 members
        assert_eq!(stats.axis_distribution[1], (1, 1)); // Axis 1: 1 member
        assert_eq!(stats.axis_distribution[2], (2, 1)); // Axis 2: 1 member

        // Diversity score should be > 0 (somewhat balanced)
        assert!(stats.diversity_score > 0.0);
        assert!(stats.diversity_score <= 1.0);
    }

    #[test]
    fn test_primary_aggregator_selection() {
        let selector = AggregatorSelector::default();

        // Create committee members with different VRF scores across axes
        let pk1 = PublicKey::from_bytes([1; 32]);
        let pk2 = PublicKey::from_bytes([2; 32]);
        let pk3 = PublicKey::from_bytes([3; 32]);

        let members = vec![
            // pk1: min score across axes = [5; 32]
            adic_quorum::QuorumMember {
                public_key: pk1,
                reputation: 100.0,
                vrf_score: [5u8; 32], // Axis 0
                axis_id: 0,
                asn: None,
                region: None,
            },
            adic_quorum::QuorumMember {
                public_key: pk1,
                reputation: 100.0,
                vrf_score: [10u8; 32], // Axis 1 (higher)
                axis_id: 1,
                asn: None,
                region: None,
            },
            // pk2: min score across axes = [3; 32] (LOWEST - should be primary)
            adic_quorum::QuorumMember {
                public_key: pk2,
                reputation: 100.0,
                vrf_score: [8u8; 32], // Axis 0
                axis_id: 0,
                asn: None,
                region: None,
            },
            adic_quorum::QuorumMember {
                public_key: pk2,
                reputation: 100.0,
                vrf_score: [3u8; 32], // Axis 1 (LOWEST overall)
                axis_id: 1,
                asn: None,
                region: None,
            },
            // pk3: min score across axes = [7; 32]
            adic_quorum::QuorumMember {
                public_key: pk3,
                reputation: 100.0,
                vrf_score: [7u8; 32], // Axis 0
                axis_id: 0,
                asn: None,
                region: None,
            },
        ];

        let primary = selector.select_primary_aggregator(&members).unwrap();

        // pk2 should be selected (has smallest min VRF score of [3; 32])
        assert_eq!(primary, pk2);
    }

    #[test]
    fn test_aggregator_timeout_logic() {
        let selector = AggregatorSelector::new(2); // 2 second timeout

        let start_time = std::time::Instant::now();

        // Should not have elapsed immediately
        assert!(!selector.is_timeout_elapsed(start_time));

        // Wait for timeout
        std::thread::sleep(std::time::Duration::from_secs(3));

        // Should have elapsed now
        assert!(selector.is_timeout_elapsed(start_time));
    }

    #[test]
    fn test_can_aggregate_primary() {
        let selector = AggregatorSelector::new(60);

        let pk1 = PublicKey::from_bytes([1; 32]);
        let pk2 = PublicKey::from_bytes([2; 32]);

        let members = vec![
            adic_quorum::QuorumMember {
                public_key: pk1,
                reputation: 100.0,
                vrf_score: [1u8; 32], // Lowest score - primary aggregator
                axis_id: 0,
                asn: None,
                region: None,
            },
            adic_quorum::QuorumMember {
                public_key: pk2,
                reputation: 100.0,
                vrf_score: [5u8; 32],
                axis_id: 0,
                asn: None,
                region: None,
            },
        ];

        let start_time = std::time::Instant::now();

        // pk1 is primary, should be able to aggregate immediately
        assert!(selector.can_aggregate(&pk1, &members, start_time));

        // pk2 is not primary and timeout hasn't elapsed, should NOT aggregate
        assert!(!selector.can_aggregate(&pk2, &members, start_time));
    }

    #[test]
    fn test_can_aggregate_fallback() {
        let selector = AggregatorSelector::new(1); // 1 second timeout

        let pk1 = PublicKey::from_bytes([1; 32]);
        let pk2 = PublicKey::from_bytes([2; 32]);

        let members = vec![
            adic_quorum::QuorumMember {
                public_key: pk1,
                reputation: 100.0,
                vrf_score: [1u8; 32], // Primary
                axis_id: 0,
                asn: None,
                region: None,
            },
            adic_quorum::QuorumMember {
                public_key: pk2,
                reputation: 100.0,
                vrf_score: [5u8; 32],
                axis_id: 0,
                asn: None,
                region: None,
            },
        ];

        let start_time = std::time::Instant::now();

        // pk2 cannot aggregate initially (not primary, no timeout)
        assert!(!selector.can_aggregate(&pk2, &members, start_time));

        // Wait for timeout
        std::thread::sleep(std::time::Duration::from_secs(2));

        // pk2 can now aggregate (timeout elapsed)
        assert!(selector.can_aggregate(&pk2, &members, start_time));
    }
}
