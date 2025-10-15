//! DKG Manager for Committee Key Generation
//!
//! Manages distributed key generation ceremonies for ODC committees using Feldman VSS.
//! Each committee epoch runs a DKG ceremony to generate threshold keys with verifiable shares.
//!
//! # Feldman VSS Protocol
//! 1. Each participant generates a random polynomial of degree t-1
//! 2. Participants broadcast public key commitments (Feldman commitments)
//! 3. Participants exchange public key shares for verification
//! 4. Each share is verified against the sender's commitment
//! 5. After verification, participants finalize their secret key shares
//!
//! # Security
//! - Shares are cryptographically verified against public commitments
//! - Malicious shares are detected and rejected
//! - Works with up to t-1 dishonest participants

use crate::{PoUWError, Result};
use adic_crypto::{DKGCeremony, DKGCommitment, DKGShare, DKGState, ThresholdConfig};
use std::collections::HashMap;
use std::sync::Arc;
use threshold_crypto::{PublicKeySet, SecretKeyShare};
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Stores DKG results for committee epochs
#[derive(Clone)]
pub struct DKGManager {
    /// PublicKeySet for each epoch
    public_key_sets: Arc<RwLock<HashMap<u64, PublicKeySet>>>,

    /// Active DKG ceremonies (epoch_id -> ceremony)
    active_ceremonies: Arc<RwLock<HashMap<u64, DKGCeremony>>>,

    /// Our secret key shares for each epoch (if we participated)
    secret_shares: Arc<RwLock<HashMap<u64, SecretKeyShare>>>,
}

impl DKGManager {
    /// Create new DKG manager
    pub fn new() -> Self {
        Self {
            public_key_sets: Arc::new(RwLock::new(HashMap::new())),
            active_ceremonies: Arc::new(RwLock::new(HashMap::new())),
            secret_shares: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start a new DKG ceremony for an epoch
    ///
    /// # Arguments
    /// * `epoch_id` - Committee epoch identifier
    /// * `participant_id` - Our participant index in the committee
    /// * `config` - Threshold configuration (t, n)
    pub async fn start_ceremony(
        &self,
        epoch_id: u64,
        participant_id: usize,
        config: ThresholdConfig,
    ) -> Result<()> {
        let mut ceremonies = self.active_ceremonies.write().await;

        if ceremonies.contains_key(&epoch_id) {
            return Err(PoUWError::Other(format!(
                "DKG ceremony already active for epoch {}",
                epoch_id
            )));
        }

        let mut ceremony = DKGCeremony::new(config, participant_id);
        ceremony.start();

        info!(
            epoch_id = epoch_id,
            participant_id = participant_id,
            threshold = config.threshold,
            total = config.total_participants,
            "🔐 Started DKG ceremony"
        );

        ceremonies.insert(epoch_id, ceremony);

        Ok(())
    }

    /// Generate our commitment for a ceremony (Feldman VSS)
    ///
    /// This generates the polynomial and public key commitment.
    /// Must be called once per ceremony to initialize our polynomial.
    pub async fn generate_commitment(&self, epoch_id: u64) -> Result<DKGCommitment> {
        let mut ceremonies = self.active_ceremonies.write().await;
        let ceremony = ceremonies
            .get_mut(&epoch_id)
            .ok_or_else(|| PoUWError::Other(format!("No active ceremony for epoch {}", epoch_id)))?;

        let commitment = ceremony
            .generate_commitment()
            .map_err(|e| PoUWError::BLSError(e))?;

        info!(
            epoch_id = epoch_id,
            "Generated Feldman VSS commitment"
        );

        Ok(commitment)
    }

    /// Add a commitment from another participant
    pub async fn add_commitment(&self, epoch_id: u64, commitment: DKGCommitment) -> Result<()> {
        let mut ceremonies = self.active_ceremonies.write().await;
        let ceremony = ceremonies
            .get_mut(&epoch_id)
            .ok_or_else(|| PoUWError::Other(format!("No active ceremony for epoch {}", epoch_id)))?;

        ceremony
            .add_commitment(commitment)
            .map_err(|e| PoUWError::BLSError(e))?;

        debug!(
            epoch_id = epoch_id,
            "Received commitment"
        );

        Ok(())
    }

    /// Generate shares to send to other participants
    pub async fn generate_shares(&self, epoch_id: u64) -> Result<Vec<DKGShare>> {
        let ceremonies = self.active_ceremonies.read().await;
        let ceremony = ceremonies
            .get(&epoch_id)
            .ok_or_else(|| PoUWError::Other(format!("No active ceremony for epoch {}", epoch_id)))?;

        ceremony
            .generate_shares()
            .map_err(|e| PoUWError::BLSError(e))
    }

    /// Add and verify a received share from another participant (Feldman VSS)
    ///
    /// The share is cryptographically verified against the sender's public commitment.
    /// Invalid shares are rejected before being stored.
    pub async fn add_share(&self, epoch_id: u64, share: DKGShare) -> Result<()> {
        let mut ceremonies = self.active_ceremonies.write().await;
        let ceremony = ceremonies
            .get_mut(&epoch_id)
            .ok_or_else(|| PoUWError::Other(format!("No active ceremony for epoch {}", epoch_id)))?;

        // This performs Feldman VSS verification
        ceremony.add_share(share.clone()).map_err(|e| PoUWError::BLSError(e))?;

        debug!(
            epoch_id = epoch_id,
            from = share.from,
            to = share.to,
            "✅ Share verified via Feldman VSS"
        );

        Ok(())
    }

    /// Finalize the ceremony and store the results
    pub async fn finalize_ceremony(&self, epoch_id: u64) -> Result<PublicKeySet> {
        let mut ceremonies = self.active_ceremonies.write().await;
        let ceremony = ceremonies
            .get_mut(&epoch_id)
            .ok_or_else(|| PoUWError::Other(format!("No active ceremony for epoch {}", epoch_id)))?;

        let result = ceremony.finalize().map_err(|e| PoUWError::BLSError(e))?;

        // Store the results
        let mut pk_sets = self.public_key_sets.write().await;
        pk_sets.insert(epoch_id, result.public_key_set.clone());

        let mut shares = self.secret_shares.write().await;
        shares.insert(epoch_id, result.secret_key_share.clone());

        info!(
            epoch_id = epoch_id,
            participant_id = result.participant_id,
            "✅ DKG ceremony finalized and keys stored"
        );

        // Remove the ceremony
        ceremonies.remove(&epoch_id);

        Ok(result.public_key_set)
    }

    /// Get the public key set for an epoch
    pub async fn get_public_key_set(&self, epoch_id: u64) -> Result<PublicKeySet> {
        let pk_sets = self.public_key_sets.read().await;
        pk_sets
            .get(&epoch_id)
            .cloned()
            .ok_or_else(|| {
                PoUWError::Other(format!("No PublicKeySet for epoch {}", epoch_id))
            })
    }

    /// Get our secret key share for an epoch
    pub async fn get_secret_share(&self, epoch_id: u64) -> Result<SecretKeyShare> {
        let shares = self.secret_shares.read().await;
        shares
            .get(&epoch_id)
            .cloned()
            .ok_or_else(|| {
                PoUWError::Other(format!("No secret share for epoch {}", epoch_id))
            })
    }

    /// Check if DKG is complete for an epoch
    pub async fn is_complete(&self, epoch_id: u64) -> bool {
        self.public_key_sets.read().await.contains_key(&epoch_id)
    }

    /// Get the state of an active ceremony
    pub async fn get_ceremony_state(&self, epoch_id: u64) -> Result<DKGState> {
        let ceremonies = self.active_ceremonies.read().await;
        let ceremony = ceremonies
            .get(&epoch_id)
            .ok_or_else(|| PoUWError::Other(format!("No active ceremony for epoch {}", epoch_id)))?;

        Ok(ceremony.state().clone())
    }
}

impl Default for DKGManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dkg_manager_lifecycle() {
        let manager = DKGManager::new();
        let epoch_id = 1;
        let participant_id = 0;
        let config = ThresholdConfig::new(3, 2).unwrap();

        // Start ceremony
        manager
            .start_ceremony(epoch_id, participant_id, config)
            .await
            .unwrap();

        // Generate commitment (Feldman VSS)
        let commitment = manager.generate_commitment(epoch_id).await.unwrap();
        assert_eq!(commitment.participant_id, participant_id);

        // Check state
        let state = manager.get_ceremony_state(epoch_id).await.unwrap();
        assert!(matches!(state, DKGState::CollectingCommitments));
    }

    #[tokio::test]
    async fn test_dkg_manager_multi_epoch() {
        let manager = DKGManager::new();
        let config = ThresholdConfig::new(3, 2).unwrap();

        // Start ceremonies for different epochs
        manager.start_ceremony(1, 0, config).await.unwrap();
        manager.start_ceremony(2, 0, config).await.unwrap();

        // Both should be active
        assert!(manager.get_ceremony_state(1).await.is_ok());
        assert!(manager.get_ceremony_state(2).await.is_ok());
    }
}
