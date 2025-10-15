//! Finality Adapter for Application Layer
//!
//! Provides a simplified interface to the ADIC finality engine (F1 + F2)
//! for use in application modules (PoUW, Storage, Governance).
//!
//! # Finality Policies
//!
//! Per PoUW III §7.1:
//! - Governance artifacts MUST satisfy F1 ∧ F2
//! - Operational signals MAY accept F1 only but remain subject to timelocks
//!
//! # Timelock Formula (PoUW III §7.2)
//!
//! T_enact = max{γ₁·T_F1, γ₂·T_F2, T_min}
//!
//! Where T_F1 and T_F2 are empirical P95 finality times.

use crate::error::{AppError, Result};
use adic_finality::FinalityEngine;
use adic_types::MessageId;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Finality policy for different artifact types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalityPolicy {
    /// Requires both F1 AND F2 (governance-grade, per PoUW III §7.1)
    Strict,
    /// Accepts F1 only (operational signals, still subject to timelock)
    F1Only,
    /// Accepts F2 only (rare, mostly for testing)
    F2Only,
}

impl Default for FinalityPolicy {
    fn default() -> Self {
        Self::Strict // Conservative default: require both F1 and F2
    }
}

/// Complete finality status with timing information
#[derive(Debug, Clone)]
pub struct FinalityStatus {
    /// Message ID being checked
    pub message_id: MessageId,

    /// F1 (k-core) finality achieved
    pub f1_final: bool,

    /// F2 (persistent homology) finality achieved
    pub f2_final: bool,

    /// Time when F1 finality was achieved (if applicable)
    pub f1_time: Option<Instant>,

    /// Time when F2 finality was achieved (if applicable)
    pub f2_time: Option<Instant>,

    /// Confidence score from F2 (0.0 to 1.0)
    pub f2_confidence: Option<f64>,

    /// When this status was last updated
    pub checked_at: Instant,
}

impl FinalityStatus {
    /// Check if finality is achieved according to policy
    pub fn is_final(&self, policy: FinalityPolicy) -> bool {
        match policy {
            FinalityPolicy::Strict => self.f1_final && self.f2_final,
            FinalityPolicy::F1Only => self.f1_final,
            FinalityPolicy::F2Only => self.f2_final,
        }
    }

    /// Get the time to finality in milliseconds
    pub fn time_to_finality_ms(&self, policy: FinalityPolicy) -> Option<u128> {
        match policy {
            FinalityPolicy::Strict => {
                // For strict policy, finality is when BOTH are achieved
                if let (Some(t1), Some(t2)) = (self.f1_time, self.f2_time) {
                    // Take the later of the two times
                    let final_time = t1.max(t2);
                    Some(final_time.elapsed().as_millis())
                } else {
                    None
                }
            }
            FinalityPolicy::F1Only => self.f1_time.map(|t| t.elapsed().as_millis()),
            FinalityPolicy::F2Only => self.f2_time.map(|t| t.elapsed().as_millis()),
        }
    }
}

/// Timelock configuration (PoUW III §7.2)
#[derive(Debug, Clone)]
pub struct TimelockConfig {
    /// Multiplier for F1 time: γ₁ ≥ 1
    pub gamma_f1: f64,

    /// Multiplier for F2 time: γ₂ ≥ 1
    pub gamma_f2: f64,

    /// Minimum timelock duration
    pub t_min: Duration,
}

impl Default for TimelockConfig {
    fn default() -> Self {
        Self {
            gamma_f1: 2.0,                  // 2x safety margin for F1
            gamma_f2: 1.5,                  // 1.5x safety margin for F2
            t_min: Duration::from_secs(30), // Minimum 30 seconds
        }
    }
}

/// Statistics for P95 finality time calculation
#[derive(Debug, Clone)]
struct FinalityStats {
    f1_times: Vec<u128>,
    f2_times: Vec<u128>,
    window_size: usize,
}

impl FinalityStats {
    fn new(window_size: usize) -> Self {
        Self {
            f1_times: Vec::new(),
            f2_times: Vec::new(),
            window_size,
        }
    }

    fn add_f1_time(&mut self, time_ms: u128) {
        self.f1_times.push(time_ms);
        if self.f1_times.len() > self.window_size {
            self.f1_times.remove(0);
        }
    }

    fn add_f2_time(&mut self, time_ms: u128) {
        self.f2_times.push(time_ms);
        if self.f2_times.len() > self.window_size {
            self.f2_times.remove(0);
        }
    }

    /// Calculate P95 (95th percentile) finality time
    fn p95(&self, times: &[u128]) -> u128 {
        if times.is_empty() {
            return 0;
        }

        let mut sorted = times.to_vec();
        sorted.sort_unstable();

        let idx = ((times.len() as f64) * 0.95) as usize;
        sorted.get(idx).copied().unwrap_or(sorted[sorted.len() - 1])
    }

    fn f1_p95(&self) -> u128 {
        self.p95(&self.f1_times)
    }

    fn f2_p95(&self) -> u128 {
        self.p95(&self.f2_times)
    }
}

/// Finality adapter for application layer modules
pub struct FinalityAdapter {
    /// Reference to the core finality engine
    engine: Arc<FinalityEngine>,

    /// Cached finality status for recent messages
    cache: Arc<RwLock<HashMap<MessageId, FinalityStatus>>>,

    /// Cache expiration time
    cache_ttl: Duration,

    /// Timelock configuration
    timelock_config: TimelockConfig,

    /// Finality time statistics for P95 calculation
    stats: Arc<RwLock<FinalityStats>>,
}

impl FinalityAdapter {
    /// Create a new finality adapter
    pub fn new(
        engine: Arc<FinalityEngine>,
        cache_ttl: Duration,
        timelock_config: TimelockConfig,
    ) -> Self {
        Self {
            engine,
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl,
            timelock_config,
            stats: Arc::new(RwLock::new(FinalityStats::new(1000))),
        }
    }

    /// Create with default configuration
    pub fn with_defaults(engine: Arc<FinalityEngine>) -> Self {
        Self::new(engine, Duration::from_secs(60), TimelockConfig::default())
    }

    /// Check finality status for a message
    pub async fn check_finality(&self, message_id: &MessageId) -> Result<FinalityStatus> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(status) = cache.get(message_id) {
                // Return cached status if it's recent enough
                if status.checked_at.elapsed() < self.cache_ttl {
                    return Ok(status.clone());
                }
            }
        }

        // Query finality engine
        let is_finalized = self.engine.is_finalized(message_id).await;

        let status = if is_finalized {
            // Get detailed artifact
            let artifact =
                self.engine.get_artifact(message_id).await.ok_or_else(|| {
                    AppError::FinalityCheckFailed("Artifact not found".to_string())
                })?;

            // Determine which finality mechanisms confirmed
            // The artifact contains the finality witness data

            // F1 is confirmed if we have a k-core root
            let f1_final = artifact.witness.kcore_root.is_some();

            // F2 is confirmed if we have H3 stability data
            let f2_final = artifact.witness.h3_stable.is_some();

            let now = Instant::now();

            FinalityStatus {
                message_id: *message_id,
                f1_final,
                f2_final,
                f1_time: if f1_final { Some(now) } else { None },
                f2_time: if f2_final { Some(now) } else { None },
                f2_confidence: artifact.witness.f2_confidence,
                checked_at: now,
            }
        } else {
            // Not finalized yet
            FinalityStatus {
                message_id: *message_id,
                f1_final: false,
                f2_final: false,
                f1_time: None,
                f2_time: None,
                f2_confidence: None,
                checked_at: Instant::now(),
            }
        };

        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(*message_id, status.clone());

            // Prune old entries (simple LRU: keep last 1000)
            if cache.len() > 1000 {
                // Remove oldest entries
                let mut keys: Vec<_> = cache.keys().copied().collect();
                keys.sort_by_key(|k| cache.get(k).map(|s| s.checked_at).unwrap_or(Instant::now()));
                for key in keys.iter().take(cache.len() - 900) {
                    cache.remove(key);
                }
            }
        }

        // Update statistics if finalized
        if status.f1_final || status.f2_final {
            let mut stats = self.stats.write().await;
            if let Some(time_ms) = status.time_to_finality_ms(FinalityPolicy::F1Only) {
                stats.add_f1_time(time_ms);
            }
            if let Some(time_ms) = status.time_to_finality_ms(FinalityPolicy::F2Only) {
                stats.add_f2_time(time_ms);
            }
        }

        Ok(status)
    }

    /// Check if message satisfies finality policy
    pub async fn is_final(&self, message_id: &MessageId, policy: FinalityPolicy) -> Result<bool> {
        let status = self.check_finality(message_id).await?;
        Ok(status.is_final(policy))
    }

    /// Calculate timelock duration per PoUW III §7.2:
    /// T_enact = max{γ₁·T_F1, γ₂·T_F2, T_min}
    pub async fn calculate_timelock(&self) -> Duration {
        // Use real finality metrics from engine
        let f1_p95 = self
            .engine
            .get_p95_f1_finality_time()
            .await
            .unwrap_or(Duration::from_secs(5)); // Fallback if no data
        let f2_p95 = self
            .engine
            .get_p95_f2_finality_time()
            .await
            .unwrap_or(Duration::from_secs(3)); // Fallback if no data

        let timelock_f1 =
            Duration::from_secs_f64(f1_p95.as_secs_f64() * self.timelock_config.gamma_f1);
        let timelock_f2 =
            Duration::from_secs_f64(f2_p95.as_secs_f64() * self.timelock_config.gamma_f2);

        let t_enact = timelock_f1.max(timelock_f2).max(self.timelock_config.t_min);

        debug!(
            t_f1_p95_ms = f1_p95.as_millis(),
            t_f2_p95_ms = f2_p95.as_millis(),
            gamma_f1 = self.timelock_config.gamma_f1,
            gamma_f2 = self.timelock_config.gamma_f2,
            t_min_ms = self.timelock_config.t_min.as_millis(),
            t_enact_ms = t_enact.as_millis(),
            "⏱️  Calculated timelock duration from real finality metrics"
        );

        t_enact
    }

    /// Get P95 finality times for monitoring (PoUW III §16)
    pub async fn get_p95_times(&self) -> (u128, u128) {
        let stats = self.stats.read().await;
        (stats.f1_p95(), stats.f2_p95())
    }

    /// Wait for finality with timeout
    pub async fn wait_for_finality(
        &self,
        message_id: &MessageId,
        policy: FinalityPolicy,
        timeout: Duration,
    ) -> Result<FinalityStatus> {
        let start = Instant::now();

        loop {
            let status = self.check_finality(message_id).await?;

            if status.is_final(policy) {
                info!(
                    message_id = ?message_id,
                    policy = ?policy,
                    elapsed_ms = start.elapsed().as_millis(),
                    f1_final = status.f1_final,
                    f2_final = status.f2_final,
                    "✅ Finality achieved"
                );
                return Ok(status);
            }

            if start.elapsed() >= timeout {
                return Err(AppError::FinalityTimeout(format!(
                    "Finality not achieved within {:?} for policy {:?}",
                    timeout, policy
                )));
            }

            // Poll every 100ms
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Clear cache (useful for testing)
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_finality_policy_strict() {
        let status = FinalityStatus {
            message_id: MessageId::from_bytes([0; 32]),
            f1_final: true,
            f2_final: true,
            f1_time: Some(Instant::now()),
            f2_time: Some(Instant::now()),
            f2_confidence: Some(0.95),
            checked_at: Instant::now(),
        };

        assert!(status.is_final(FinalityPolicy::Strict));
        assert!(status.is_final(FinalityPolicy::F1Only));
        assert!(status.is_final(FinalityPolicy::F2Only));
    }

    #[test]
    fn test_finality_policy_f1_only() {
        let status = FinalityStatus {
            message_id: MessageId::from_bytes([0; 32]),
            f1_final: true,
            f2_final: false,
            f1_time: Some(Instant::now()),
            f2_time: None,
            f2_confidence: None,
            checked_at: Instant::now(),
        };

        assert!(!status.is_final(FinalityPolicy::Strict));
        assert!(status.is_final(FinalityPolicy::F1Only));
        assert!(!status.is_final(FinalityPolicy::F2Only));
    }

    #[test]
    fn test_timelock_calculation() {
        let config = TimelockConfig {
            gamma_f1: 2.0,
            gamma_f2: 1.5,
            t_min: Duration::from_secs(10),
        };

        // Test case 1: F1 time dominates
        let t_f1 = Duration::from_secs(20); // γ₁·T_F1 = 2.0 * 20 = 40s
        let t_f2 = Duration::from_secs(10); // γ₂·T_F2 = 1.5 * 10 = 15s

        let timelock_f1 = Duration::from_millis((t_f1.as_millis() as f64 * config.gamma_f1) as u64);
        let timelock_f2 = Duration::from_millis((t_f2.as_millis() as f64 * config.gamma_f2) as u64);
        let t_enact = timelock_f1.max(timelock_f2).max(config.t_min);

        assert_eq!(t_enact.as_secs(), 40);
    }

    #[test]
    fn test_stats_p95() {
        let mut stats = FinalityStats::new(100);

        // Add 100 samples (0-99 ms)
        for i in 0..100 {
            stats.add_f1_time(i);
        }

        // P95 should be around 95
        let p95 = stats.f1_p95();
        assert!((94..=95).contains(&p95));
    }
}
