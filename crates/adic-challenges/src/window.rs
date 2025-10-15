use crate::{ChallengeConfig, ChallengeError, ChallengeMetadata, ChallengeStatus, Result};
use adic_types::{MessageId, PublicKey};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Challenge window for a subject (e.g., storage proof, PoUW result)
#[derive(Debug, Clone)]
pub struct ChallengeWindow {
    pub metadata: ChallengeMetadata,
    pub config: ChallengeConfig,
}

impl ChallengeWindow {
    pub fn new(subject_id: MessageId, submitted_at_epoch: u64, config: ChallengeConfig) -> Self {
        let window_expiry_epoch = submitted_at_epoch + config.window_depth;

        Self {
            metadata: ChallengeMetadata {
                id: hex::encode(subject_id.as_bytes()),
                subject_id,
                challenger: PublicKey::from_bytes([0; 32]), // Set when challenged
                submitted_at_epoch,
                window_expiry_epoch,
                status: ChallengeStatus::Active,
            },
            config,
        }
    }

    pub fn is_active(&self, current_epoch: u64) -> bool {
        self.metadata.status == ChallengeStatus::Active
            && current_epoch <= self.metadata.window_expiry_epoch
    }

    pub fn is_expired(&self, current_epoch: u64) -> bool {
        current_epoch > self.metadata.window_expiry_epoch
    }

    pub fn mark_challenged(&mut self, challenger: PublicKey) {
        self.metadata.status = ChallengeStatus::Challenged;
        self.metadata.challenger = challenger;
    }

    pub fn mark_finalized(&mut self) {
        self.metadata.status = ChallengeStatus::Finalized;
    }

    pub fn mark_slashed(&mut self) {
        self.metadata.status = ChallengeStatus::Slashed;
    }
}

/// Manages challenge windows for all subjects
pub struct ChallengeWindowManager {
    windows: Arc<RwLock<HashMap<MessageId, ChallengeWindow>>>,
    config: ChallengeConfig,
    // Metrics
    pub challenge_windows_opened: Option<Arc<prometheus::IntCounter>>,
    pub challenges_submitted: Option<Arc<prometheus::IntCounter>>,
    pub challenge_windows_active: Option<Arc<prometheus::IntGauge>>,
}

impl ChallengeWindowManager {
    pub fn new(config: ChallengeConfig) -> Self {
        Self {
            windows: Arc::new(RwLock::new(HashMap::new())),
            config,
            challenge_windows_opened: None,
            challenges_submitted: None,
            challenge_windows_active: None,
        }
    }

    /// Set metrics for tracking challenge operations
    pub fn set_metrics(
        &mut self,
        challenge_windows_opened: Arc<prometheus::IntCounter>,
        challenges_submitted: Arc<prometheus::IntCounter>,
        challenge_windows_active: Arc<prometheus::IntGauge>,
    ) {
        self.challenge_windows_opened = Some(challenge_windows_opened);
        self.challenges_submitted = Some(challenges_submitted);
        self.challenge_windows_active = Some(challenge_windows_active);
    }

    /// Open a new challenge window
    pub async fn open_window(&self, subject_id: MessageId, submitted_at_epoch: u64) -> Result<()> {
        let window = ChallengeWindow::new(subject_id, submitted_at_epoch, self.config.clone());

        let mut windows = self.windows.write().await;
        windows.insert(subject_id, window);

        // Update metrics
        if let Some(ref counter) = self.challenge_windows_opened {
            counter.inc();
        }
        if let Some(ref gauge) = self.challenge_windows_active {
            gauge.inc();
        }

        info!(
            subject_id = hex::encode(subject_id.as_bytes()),
            submitted_at_epoch,
            window_expiry = submitted_at_epoch + self.config.window_depth,
            window_depth = self.config.window_depth,
            "⏳ Challenge window opened"
        );

        Ok(())
    }

    /// Submit a challenge within the window
    pub async fn submit_challenge(
        &self,
        subject_id: MessageId,
        challenger: PublicKey,
        current_epoch: u64,
    ) -> Result<()> {
        let mut windows = self.windows.write().await;

        let window = windows
            .get_mut(&subject_id)
            .ok_or_else(|| ChallengeError::WindowNotFound(hex::encode(subject_id.as_bytes())))?;

        // Check if window is still active
        if !window.is_active(current_epoch) {
            return Err(ChallengeError::WindowExpired {
                id: window.metadata.id.clone(),
            });
        }

        // Check if already challenged
        if window.metadata.status != ChallengeStatus::Active {
            return Err(ChallengeError::NotActive(window.metadata.id.clone()));
        }

        window.mark_challenged(challenger);

        // Update metrics
        if let Some(ref counter) = self.challenges_submitted {
            counter.inc();
        }

        info!(
            subject_id = hex::encode(subject_id.as_bytes()),
            challenger = hex::encode(challenger.as_bytes()),
            current_epoch,
            window_expiry = window.metadata.window_expiry_epoch,
            "🎯 Challenge submitted"
        );

        Ok(())
    }

    /// Finalize window (no challenges)
    pub async fn finalize_window(&self, subject_id: MessageId, current_epoch: u64) -> Result<()> {
        let mut windows = self.windows.write().await;

        let window = windows
            .get_mut(&subject_id)
            .ok_or_else(|| ChallengeError::WindowNotFound(hex::encode(subject_id.as_bytes())))?;

        // Check if window expired
        if !window.is_expired(current_epoch) {
            return Err(ChallengeError::NotActive(
                "Window has not expired yet".to_string(),
            ));
        }

        // Check if there are challenges
        if window.metadata.status == ChallengeStatus::Challenged {
            return Err(ChallengeError::NotActive(
                "Cannot finalize: window has active challenge".to_string(),
            ));
        }

        window.mark_finalized();

        // Update metrics
        if let Some(ref gauge) = self.challenge_windows_active {
            gauge.dec();
        }

        info!(
            subject_id = hex::encode(subject_id.as_bytes()),
            finalized_at_epoch = current_epoch,
            window_expiry_epoch = window.metadata.window_expiry_epoch,
            "✅ Challenge window finalized (no challenges)"
        );

        Ok(())
    }

    /// Mark window as slashed (after fraud proof verified)
    pub async fn mark_slashed(&self, subject_id: MessageId) -> Result<()> {
        let mut windows = self.windows.write().await;

        let window = windows
            .get_mut(&subject_id)
            .ok_or_else(|| ChallengeError::WindowNotFound(hex::encode(subject_id.as_bytes())))?;

        window.mark_slashed();

        // Update metrics
        if let Some(ref gauge) = self.challenge_windows_active {
            gauge.dec();
        }

        info!(
            subject_id = hex::encode(subject_id.as_bytes()),
            challenger = hex::encode(window.metadata.challenger.as_bytes()),
            "⚡ Subject slashed after fraud proof"
        );

        Ok(())
    }

    /// Get window status
    pub async fn get_window(&self, subject_id: &MessageId) -> Option<ChallengeWindow> {
        let windows = self.windows.read().await;
        windows.get(subject_id).cloned()
    }

    /// Get all active windows
    pub async fn get_active_windows(&self, current_epoch: u64) -> Vec<ChallengeWindow> {
        let windows = self.windows.read().await;
        windows
            .values()
            .filter(|w| w.is_active(current_epoch))
            .cloned()
            .collect()
    }

    /// Get all expired windows that need finalization
    pub async fn get_expired_windows(&self, current_epoch: u64) -> Vec<ChallengeWindow> {
        let windows = self.windows.read().await;
        windows
            .values()
            .filter(|w| w.is_expired(current_epoch) && w.metadata.status == ChallengeStatus::Active)
            .cloned()
            .collect()
    }

    /// Clean up old finalized windows
    pub async fn cleanup_finalized(&self) -> usize {
        let mut windows = self.windows.write().await;
        let before_count = windows.len();

        windows.retain(|_, w| {
            !matches!(
                w.metadata.status,
                ChallengeStatus::Finalized | ChallengeStatus::Slashed
            )
        });

        let removed = before_count - windows.len();

        if removed > 0 {
            debug!(removed, "Cleaned up finalized challenge windows");
        }

        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_challenge_window_lifecycle() {
        let config = ChallengeConfig::default();
        let manager = ChallengeWindowManager::new(config.clone());

        let subject_id = MessageId::new(b"test_subject");
        let challenger = PublicKey::from_bytes([1; 32]);

        // Open window
        manager.open_window(subject_id, 100).await.unwrap();

        // Verify window is active
        let window = manager.get_window(&subject_id).await.unwrap();
        assert!(window.is_active(100));
        assert!(window.is_active(100 + config.window_depth - 1));

        // Submit challenge
        manager
            .submit_challenge(subject_id, challenger, 100)
            .await
            .unwrap();

        // Verify challenged status
        let window = manager.get_window(&subject_id).await.unwrap();
        assert_eq!(window.metadata.status, ChallengeStatus::Challenged);

        // Mark as slashed
        manager.mark_slashed(subject_id).await.unwrap();

        let window = manager.get_window(&subject_id).await.unwrap();
        assert_eq!(window.metadata.status, ChallengeStatus::Slashed);
    }

    #[tokio::test]
    async fn test_window_expiry() {
        let config = ChallengeConfig {
            window_depth: 10,
            ..Default::default()
        };
        let manager = ChallengeWindowManager::new(config);

        let subject_id = MessageId::new(b"test");

        manager.open_window(subject_id, 100).await.unwrap();

        // Active at epoch 100
        let window = manager.get_window(&subject_id).await.unwrap();
        assert!(window.is_active(100));

        // Active at epoch 109
        assert!(window.is_active(109));

        // Expired at epoch 111
        assert!(window.is_expired(111));
        assert!(!window.is_active(111));
    }
}
