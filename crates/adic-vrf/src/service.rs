use crate::{
    canonical::{compute_canonical_randomness, compute_challenge, compute_vrf_score},
    VRFCommit, VRFError, VRFOpen, VRFState, Result,
};
use adic_consensus::ReputationTracker;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// VRF service configuration
#[derive(Debug, Clone)]
pub struct VRFConfig {
    /// Minimum reputation to participate in VRF commit-reveal
    pub min_committer_reputation: f64,

    /// Depth by which VRF must be revealed (in epochs)
    pub reveal_depth: u64,

    /// Root hash from genesis (used for epoch 0)
    pub genesis_root: [u8; 32],
}

impl Default for VRFConfig {
    fn default() -> Self {
        Self {
            min_committer_reputation: 50.0,
            reveal_depth: 10,
            genesis_root: [0u8; 32],
        }
    }
}

/// VRF service manages commit-reveal process and canonical randomness
pub struct VRFService {
    config: VRFConfig,
    reputation_tracker: Arc<ReputationTracker>,

    /// VRF state per epoch
    states: Arc<RwLock<HashMap<u64, VRFState>>>,

    /// Root hashes per epoch (for mixing)
    roots: Arc<RwLock<HashMap<u64, [u8; 32]>>>,

    // Metrics
    pub vrf_commits_total: Option<Arc<prometheus::IntCounter>>,
    pub vrf_reveals_total: Option<Arc<prometheus::IntCounter>>,
    pub vrf_finalizations_total: Option<Arc<prometheus::IntCounter>>,
    pub vrf_commit_duration: Option<Arc<prometheus::Histogram>>,
    pub vrf_reveal_duration: Option<Arc<prometheus::Histogram>>,
    pub vrf_finalize_duration: Option<Arc<prometheus::Histogram>>,
}

impl VRFService {
    pub fn new(config: VRFConfig, reputation_tracker: Arc<ReputationTracker>) -> Self {
        let mut roots = HashMap::new();
        roots.insert(0, config.genesis_root);

        Self {
            config,
            reputation_tracker,
            states: Arc::new(RwLock::new(HashMap::new())),
            roots: Arc::new(RwLock::new(roots)),
            vrf_commits_total: None,
            vrf_reveals_total: None,
            vrf_finalizations_total: None,
            vrf_commit_duration: None,
            vrf_reveal_duration: None,
            vrf_finalize_duration: None,
        }
    }

    /// Set metrics for VRF tracking
    pub fn set_metrics(
        &mut self,
        vrf_commits_total: Arc<prometheus::IntCounter>,
        vrf_reveals_total: Arc<prometheus::IntCounter>,
        vrf_finalizations_total: Arc<prometheus::IntCounter>,
        vrf_commit_duration: Arc<prometheus::Histogram>,
        vrf_reveal_duration: Arc<prometheus::Histogram>,
        vrf_finalize_duration: Arc<prometheus::Histogram>,
    ) {
        self.vrf_commits_total = Some(vrf_commits_total);
        self.vrf_reveals_total = Some(vrf_reveals_total);
        self.vrf_finalizations_total = Some(vrf_finalizations_total);
        self.vrf_commit_duration = Some(vrf_commit_duration);
        self.vrf_reveal_duration = Some(vrf_reveal_duration);
        self.vrf_finalize_duration = Some(vrf_finalize_duration);
    }

    /// Submit a VRF commit for epoch k (during epoch k-1)
    pub async fn submit_commit(&self, commit: VRFCommit) -> Result<()> {
        let start = std::time::Instant::now();

        // Check reputation requirement
        let rep = self
            .reputation_tracker
            .get_reputation(&commit.committer)
            .await;

        if rep < self.config.min_committer_reputation {
            return Err(VRFError::InsufficientReputation {
                required: self.config.min_committer_reputation,
                actual: rep,
            });
        }

        // Get or create state for this epoch
        let mut states = self.states.write().await;
        let state = states
            .entry(commit.target_epoch)
            .or_insert_with(|| VRFState::new(commit.target_epoch));

        // Check for duplicate commits from same committer
        if state
            .commits
            .iter()
            .any(|c| c.committer == commit.committer)
        {
            return Err(VRFError::DuplicateCommit(
                hex::encode(commit.committer.as_bytes()),
                commit.target_epoch,
            ));
        }

        state.add_commit(commit.clone());

        // Update metrics
        if let Some(ref counter) = self.vrf_commits_total {
            counter.inc();
        }
        if let Some(ref histogram) = self.vrf_commit_duration {
            histogram.observe(start.elapsed().as_secs_f64());
        }

        info!(
            epoch = commit.target_epoch,
            committer = hex::encode(&commit.committer.as_bytes()[..8]),
            commit_hash = hex::encode(&commit.commitment[..8]),
            committer_reputation = commit.committer_reputation,
            total_commits = state.commits.len(),
            duration_ms = start.elapsed().as_millis() as u64,
            "🎲 VRF commit submitted"
        );

        Ok(())
    }

    /// Submit a VRF reveal for epoch k (during early epoch k)
    pub async fn submit_reveal(&self, reveal: VRFOpen, current_epoch: u64) -> Result<()> {
        let start = std::time::Instant::now();

        // Check reveal deadline
        if current_epoch > reveal.target_epoch + self.config.reveal_depth {
            return Err(VRFError::RevealDeadlinePassed(reveal.target_epoch));
        }

        // Get state for this epoch
        let mut states = self.states.write().await;
        let state = states
            .get_mut(&reveal.target_epoch)
            .ok_or(VRFError::CommitNotFound(reveal.target_epoch))?;

        // Find matching commit
        let matching_commit = state
            .commits
            .iter()
            .find(|c| c.id() == reveal.ref_commit)
            .ok_or_else(|| {
                VRFError::RevealVerificationFailed("No matching commit found".to_string())
            })?;

        // Verify commitment
        if !matching_commit.verify_commitment(&reveal.vrf_proof) {
            let expected = hex::encode(matching_commit.commitment);
            let actual = hex::encode(reveal.compute_commitment());
            return Err(VRFError::CommitmentMismatch { expected, actual });
        }

        state.add_reveal(reveal.clone());

        // Update metrics
        if let Some(ref counter) = self.vrf_reveals_total {
            counter.inc();
        }
        if let Some(ref histogram) = self.vrf_reveal_duration {
            histogram.observe(start.elapsed().as_secs_f64());
        }

        info!(
            epoch = reveal.target_epoch,
            revealer = hex::encode(&reveal.public_key.as_bytes()[..8]),
            reveal_id = hex::encode(reveal.message.id.as_bytes()),
            proof_hash = hex::encode(&blake3::hash(&reveal.vrf_proof).as_bytes()[..8]),
            total_reveals = state.reveals.len(),
            duration_ms = start.elapsed().as_millis() as u64,
            "🔓 VRF reveal opened"
        );

        Ok(())
    }

    /// Finalize epoch randomness after reveal phase
    pub async fn finalize_epoch(&self, epoch: u64, _current_epoch: u64) -> Result<[u8; 32]> {
        let start = std::time::Instant::now();
        let mut states = self.states.write().await;
        let state = states
            .get_mut(&epoch)
            .ok_or(VRFError::CommitNotFound(epoch))?;

        // Check if already finalized
        if state.is_finalized {
            return Ok(state.canonical_randomness.unwrap());
        }

        // Check if we have reveals
        if state.reveals.is_empty() {
            return Err(VRFError::NoReveals(epoch));
        }

        // Get previous epoch root
        let roots = self.roots.read().await;
        let prev_root = roots
            .get(&(epoch.saturating_sub(1)))
            .copied()
            .unwrap_or(self.config.genesis_root);

        // Compute canonical randomness
        let canonical = compute_canonical_randomness(epoch, &prev_root, &state.reveals);

        // Finalize state
        state.finalize(canonical.randomness);

        // Store root for next epoch
        drop(roots);
        let mut roots = self.roots.write().await;
        roots.insert(epoch, canonical.randomness);

        // Update metrics
        if let Some(ref counter) = self.vrf_finalizations_total {
            counter.inc();
        }
        if let Some(ref histogram) = self.vrf_finalize_duration {
            histogram.observe(start.elapsed().as_secs_f64());
        }

        info!(
            epoch,
            num_contributors = canonical.num_contributors,
            randomness_hash = hex::encode(&canonical.randomness[..8]),
            prev_root_hash = hex::encode(&prev_root[..8]),
            total_commits = state.commits.len(),
            total_reveals = state.reveals.len(),
            duration_ms = start.elapsed().as_millis() as u64,
            "✨ Canonical randomness finalized"
        );

        Ok(canonical.randomness)
    }

    /// Get canonical randomness for an epoch (with fallback)
    pub async fn get_canonical_randomness(&self, epoch: u64) -> Result<[u8; 32]> {
        let states = self.states.read().await;

        if let Some(state) = states.get(&epoch) {
            if state.is_finalized {
                return Ok(state.canonical_randomness.unwrap());
            }
        }

        // Fallback: use previous epoch's randomness
        if epoch > 0 {
            if let Some(prev_state) = states.get(&(epoch - 1)) {
                if prev_state.is_finalized {
                    warn!(
                        epoch,
                        "Using fallback randomness from epoch {}",
                        epoch - 1
                    );
                    return Ok(prev_state.canonical_randomness.unwrap());
                }
            }
        }

        Err(VRFError::RandomnessNotFinalized(epoch))
    }

    /// Compute a challenge from epoch randomness
    pub async fn compute_challenge_for_epoch(
        &self,
        epoch: u64,
        domain: &str,
        params: &[&[u8]],
    ) -> Result<[u8; 32]> {
        let randomness = self.get_canonical_randomness(epoch).await?;
        Ok(compute_challenge(&randomness, domain, params))
    }

    /// Compute VRF score for committee selection
    pub async fn compute_vrf_score_for_epoch(
        &self,
        epoch: u64,
        domain: &str,
        axis_id: usize,
        node_id: &[u8],
        axis_ball: &[u8],
    ) -> Result<[u8; 32]> {
        let randomness = self.get_canonical_randomness(epoch).await?;
        Ok(compute_vrf_score(
            &randomness,
            domain,
            axis_id,
            node_id,
            axis_ball,
        ))
    }

    /// Get VRF state for an epoch
    pub async fn get_state(&self, epoch: u64) -> Option<VRFState> {
        let states = self.states.read().await;
        states.get(&epoch).cloned()
    }

    /// Get number of commits for an epoch
    pub async fn get_commit_count(&self, epoch: u64) -> usize {
        let states = self.states.read().await;
        states
            .get(&epoch)
            .map(|s| s.commits.len())
            .unwrap_or(0)
    }

    /// Get number of reveals for an epoch
    pub async fn get_reveal_count(&self, epoch: u64) -> usize {
        let states = self.states.read().await;
        states
            .get(&epoch)
            .map(|s| s.reveals.len())
            .unwrap_or(0)
    }

    /// Set finalized randomness directly (test helper only - not for production use)
    pub async fn set_test_randomness(&self, epoch: u64, randomness: [u8; 32]) {
        let mut states = self.states.write().await;
        let mut state = VRFState::new(epoch);
        state.is_finalized = true;
        state.canonical_randomness = Some(randomness);
        states.insert(epoch, state);

        let mut roots = self.roots.write().await;
        roots.insert(epoch, randomness);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::{AdicFeatures, AdicMessage, AdicMeta, PublicKey};
    use chrono::Utc;

    #[tokio::test]
    async fn test_vrf_service_lifecycle() {
        let rep_tracker = Arc::new(ReputationTracker::new(0.9));
        let config = VRFConfig::default();
        let service = VRFService::new(config, rep_tracker.clone());

        let committer = PublicKey::from_bytes([1; 32]);

        // Set up reputation
        rep_tracker.set_reputation(&committer, 60.0).await;

        // Create commit
        let msg = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![]),
            AdicMeta::new(Utc::now()),
            committer,
            vec![],
        );

        let vrf_proof = b"test_vrf_proof";
        let commitment = *blake3::hash(vrf_proof).as_bytes();

        let commit = VRFCommit::new(msg.clone(), 100, commitment, committer, 60.0);

        // Submit commit
        service.submit_commit(commit.clone()).await.unwrap();

        // Create reveal
        let reveal = VRFOpen::new(
            msg.clone(),
            100,
            commit.id(),
            vrf_proof.to_vec(),
            committer,
        );

        // Submit reveal
        service.submit_reveal(reveal, 100).await.unwrap();

        // Finalize
        let randomness = service.finalize_epoch(100, 100).await.unwrap();

        assert_ne!(randomness, [0u8; 32]);

        // Get canonical randomness
        let r = service.get_canonical_randomness(100).await.unwrap();
        assert_eq!(r, randomness);
    }
}
