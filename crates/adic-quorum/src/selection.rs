use crate::{
    diversity::{DiversityCaps, enforce_diversity_caps},
    QuorumConfig, QuorumError, QuorumMember, QuorumResult, Result,
};
use adic_consensus::ReputationTracker;
use adic_types::PublicKey;
use adic_vrf::VRFService;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, info};

/// Node information for quorum selection
#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub public_key: PublicKey,
    pub reputation: f64,
    pub asn: Option<u32>,
    pub region: Option<String>,
    pub axis_balls: Vec<Vec<u8>>, // p-adic ball per axis
}

/// Quorum selector using VRF-based committee selection
/// Algorithm from ADICPoUW2.pdf Section 3.2
pub struct QuorumSelector {
    vrf_service: Arc<VRFService>,
    #[allow(dead_code)] // Reserved for future reputation-based filtering
    reputation_tracker: Arc<ReputationTracker>,
    // Metrics
    pub quorum_selections_total: Option<Arc<prometheus::IntCounter>>,
    pub quorum_selection_duration: Option<Arc<prometheus::Histogram>>,
    pub quorum_committee_size: Option<Arc<prometheus::IntGauge>>,
}

impl QuorumSelector {
    pub fn new(
        vrf_service: Arc<VRFService>,
        reputation_tracker: Arc<ReputationTracker>,
    ) -> Self {
        Self {
            vrf_service,
            reputation_tracker,
            quorum_selections_total: None,
            quorum_selection_duration: None,
            quorum_committee_size: None,
        }
    }

    /// Set metrics for quorum tracking
    pub fn set_metrics(
        &mut self,
        quorum_selections_total: Arc<prometheus::IntCounter>,
        quorum_selection_duration: Arc<prometheus::Histogram>,
        quorum_committee_size: Arc<prometheus::IntGauge>,
    ) {
        self.quorum_selections_total = Some(quorum_selections_total);
        self.quorum_selection_duration = Some(quorum_selection_duration);
        self.quorum_committee_size = Some(quorum_committee_size);
    }

    /// Select quorum committee for an epoch
    pub async fn select_committee(
        &self,
        epoch: u64,
        config: &QuorumConfig,
        eligible_nodes: Vec<NodeInfo>,
    ) -> Result<QuorumResult> {
        let start = std::time::Instant::now();

        // Filter by minimum reputation
        let eligible: Vec<_> = eligible_nodes
            .into_iter()
            .filter(|node| node.reputation >= config.min_reputation)
            .collect();

        if eligible.is_empty() {
            return Err(QuorumError::InsufficientNodes {
                needed: config.total_size,
                available: 0,
            });
        }

        info!(
            epoch,
            eligible_count = eligible.len(),
            min_rep = config.min_reputation,
            "🗳️ Selecting quorum from eligible nodes"
        );

        // Get canonical randomness for epoch
        let _r_k = self
            .vrf_service
            .get_canonical_randomness(epoch)
            .await?;

        // Step 1: Compute VRF scores per axis for each node
        let mut all_members = Vec::new();
        let mut axes_coverage = vec![0; config.num_axes];

        for axis_j in 0..config.num_axes {
            let mut axis_members = Vec::new();

            for node in &eligible {
                let axis_ball = node.axis_balls.get(axis_j).map(|b| b.as_slice()).unwrap_or(&[]);

                // Compute VRF score: y_{u,j} := VRF(R_k || domain || axis_id=j || axis_ball)
                let vrf_score = self
                    .vrf_service
                    .compute_vrf_score_for_epoch(
                        epoch,
                        &config.domain_separator,
                        axis_j,
                        node.public_key.as_bytes(),
                        axis_ball,
                    )
                    .await?;

                axis_members.push(QuorumMember {
                    public_key: node.public_key,
                    reputation: node.reputation,
                    vrf_score,
                    axis_id: axis_j,
                    asn: node.asn,
                    region: node.region.clone(),
                });
            }

            // Step 2: Select m_j lowest scores for this axis
            axis_members.sort_by_key(|m| m.vrf_score);
            let selected_for_axis: Vec<_> = axis_members
                .into_iter()
                .take(config.members_per_axis)
                .collect();

            axes_coverage[axis_j] = selected_for_axis.len();
            all_members.extend(selected_for_axis);

            debug!(
                axis_id = axis_j,
                selected = axes_coverage[axis_j],
                "Selected members for axis"
            );
        }

        // Step 3: Merge and deduplicate
        let mut unique_members = Vec::new();
        let mut seen = HashSet::new();

        for member in all_members {
            if seen.insert(member.public_key) {
                unique_members.push(member);
            }
        }

        // Step 4: Enforce diversity caps (ASN, region)
        let caps = DiversityCaps::new(config.max_per_asn, config.max_per_region);
        let capped_members = enforce_diversity_caps(unique_members, &caps)?;

        // Step 5: Limit to total size
        let final_members: Vec<_> = capped_members
            .into_iter()
            .take(config.total_size)
            .collect();

        if final_members.len() < config.total_size {
            debug!(
                selected = final_members.len(),
                requested = config.total_size,
                "Selected fewer members than requested (after diversity caps)"
            );
        }

        // Update metrics
        if let Some(ref counter) = self.quorum_selections_total {
            counter.inc();
        }
        if let Some(ref histogram) = self.quorum_selection_duration {
            histogram.observe(start.elapsed().as_secs_f64());
        }
        if let Some(ref gauge) = self.quorum_committee_size {
            gauge.set(final_members.len() as i64);
        }

        info!(
            epoch,
            members_count = final_members.len(),
            axes_coverage = ?axes_coverage,
            duration_ms = start.elapsed().as_millis() as u64,
            "🗳️ Quorum committee selected"
        );

        Ok(QuorumResult {
            members: final_members.clone(),
            total_size: final_members.len(),
            axes_coverage,
        })
    }

    /// Check if a node is in the selected quorum
    pub fn is_in_quorum(&self, node: &PublicKey, quorum: &QuorumResult) -> bool {
        quorum.members.iter().any(|m| &m.public_key == node)
    }

    /// Get quorum member by public key
    pub fn get_member<'a>(
        &self,
        node: &PublicKey,
        quorum: &'a QuorumResult,
    ) -> Option<&'a QuorumMember> {
        quorum.members.iter().find(|m| &m.public_key == node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::{AdicFeatures, AdicMessage, AdicMeta};
    use adic_vrf::{VRFCommit, VRFConfig, VRFOpen};
    use chrono::Utc;

    #[tokio::test]
    async fn test_quorum_selection() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let vrf_service = Arc::new(VRFService::new(
            VRFConfig::default(),
            rep_tracker.clone(),
        ));
        let selector = QuorumSelector::new(vrf_service.clone(), rep_tracker.clone());

        // Create eligible nodes
        let mut nodes = Vec::new();
        for i in 0..20 {
            let pk = PublicKey::from_bytes([i; 32]);
            rep_tracker.set_reputation(&pk, 60.0).await;

            nodes.push(NodeInfo {
                public_key: pk,
                reputation: 60.0,
                asn: Some((i % 3) as u32 + 100), // 3 different ASNs
                region: Some(format!("region_{}", i % 2)), // 2 regions
                axis_balls: vec![vec![i], vec![i], vec![i]],
            });
        }

        let config = QuorumConfig {
            min_reputation: 50.0,
            members_per_axis: 5,
            total_size: 15,
            max_per_asn: 6,
            max_per_region: 10,
            domain_separator: "test".to_string(),
            num_axes: 3,
        };

        // Finalize VRF randomness for epoch
        let epoch = 1u64;
        let committer = PublicKey::from_bytes([99u8; 32]);
        rep_tracker.set_reputation(&committer, 100.0).await;
        let msg = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![]),
            AdicMeta::new(Utc::now()),
            committer,
            vec![],
        );
        let vrf_proof = b"test_vrf_proof".to_vec();
        let commitment = *blake3::hash(&vrf_proof).as_bytes();
        let commit = VRFCommit::new(msg.clone(), epoch, commitment, committer, 100.0);
        vrf_service.submit_commit(commit.clone()).await.unwrap();
        let reveal = VRFOpen::new(msg, epoch, commit.id(), vrf_proof, committer);
        vrf_service.submit_reveal(reveal, epoch).await.unwrap();
        vrf_service.finalize_epoch(epoch, epoch).await.unwrap();

        // Select committee
        let result = selector
            .select_committee(epoch, &config, nodes.clone())
            .await
            .expect("quorum selection should succeed");

        // Basic invariants
        assert!(result.members.len() > 0);
        assert!(result.members.len() <= config.total_size);
        assert_eq!(result.axes_coverage.len(), config.num_axes);

        // Uniqueness of members
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        assert!(result
            .members
            .iter()
            .all(|m| seen.insert(m.public_key)), "Members must be unique");

        // Diversity caps: count by ASN and region
        let mut asn_counts = std::collections::HashMap::new();
        let mut region_counts = std::collections::HashMap::new();
        for m in &result.members {
            if let Some(asn) = m.asn {
                *asn_counts.entry(asn).or_insert(0usize) += 1;
            }
            if let Some(region) = &m.region {
                *region_counts.entry(region.clone()).or_insert(0usize) += 1;
            }
            assert!(m.reputation >= config.min_reputation);
        }
        assert!(asn_counts.values().all(|&c| c <= config.max_per_asn));
        assert!(region_counts.values().all(|&c| c <= config.max_per_region));
    }
}
