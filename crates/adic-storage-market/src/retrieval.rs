//! Retrieval Protocol
//!
//! This module implements Sprint 6 from storage-market-design.md:
//! - RetrievalRequest/RetrievalResponse message types
//! - Bandwidth pricing and payment on finality
//! - Range request support
//! - Performance SLA tracking
//!
//! Note: QUIC-based data transfer is handled off-chain via provider endpoints.
//! This module manages the on-chain coordination and payment settlement.

use crate::error::{Result, StorageMarketError};
use crate::types::{FinalityStatus, Hash, MessageHash};
use adic_economics::types::{AccountAddress, AdicAmount};
use adic_types::Signature;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Retrieval request message (initiates data retrieval)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalRequest {
    // ========== Protocol Fields ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Retrieval Fields ==========
    /// Request ID (hash of request details)
    pub request_id: Hash,
    /// Deal being retrieved
    pub deal_id: u64,
    /// Client requesting retrieval
    pub client: AccountAddress,
    /// Provider serving the data
    pub provider: AccountAddress,
    /// Byte range to retrieve (optional - full file if None)
    pub byte_range: Option<ByteRange>,
    /// Maximum price client willing to pay per GB
    pub max_price_per_gb: AdicAmount,
    /// Required bandwidth (bytes/sec) - SLA requirement
    pub min_bandwidth_bps: u64,
    /// Deadline for retrieval completion (epoch)
    pub deadline_epoch: u64,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
}

/// Byte range for partial retrieval
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64, // Inclusive
}

impl ByteRange {
    pub fn new(start: u64, end: u64) -> Result<Self> {
        if start > end {
            return Err(StorageMarketError::Other(
                "Invalid byte range: start > end".to_string(),
            ));
        }
        Ok(Self { start, end })
    }

    pub fn size(&self) -> u64 {
        self.end - self.start + 1
    }
}

impl RetrievalRequest {
    pub fn new(
        deal_id: u64,
        client: AccountAddress,
        provider: AccountAddress,
        byte_range: Option<ByteRange>,
        max_price_per_gb: AdicAmount,
        min_bandwidth_bps: u64,
        deadline_epoch: u64,
    ) -> Self {
        let now = chrono::Utc::now().timestamp();
        let mut request_data = Vec::new();
        request_data.extend_from_slice(&deal_id.to_le_bytes());
        request_data.extend_from_slice(client.as_bytes());
        request_data.extend_from_slice(&now.to_le_bytes());

        Self {
            approvals: [MessageHash::default(); 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: now,
            request_id: blake3::hash(&request_data).into(),
            deal_id,
            client,
            provider,
            byte_range,
            max_price_per_gb,
            min_bandwidth_bps,
            deadline_epoch,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
        }
    }

    pub fn is_finalized(&self) -> bool {
        self.finality_status.is_finalized()
    }
}

/// Retrieval response message (confirms retrieval completion)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalResponse {
    // ========== Protocol Fields ==========
    pub approvals: [MessageHash; 4],
    pub features: [u64; 3],
    pub signature: Signature,
    pub deposit: AdicAmount,
    pub timestamp: i64,

    // ========== Response Fields ==========
    /// Reference to retrieval request
    pub ref_request: Hash,
    /// Provider serving the data
    pub provider: AccountAddress,
    /// Bytes transferred
    pub bytes_transferred: u64,
    /// Transfer duration (seconds)
    pub transfer_duration_secs: u64,
    /// Actual bandwidth achieved (bytes/sec)
    pub actual_bandwidth_bps: u64,
    /// Price charged per GB
    pub price_per_gb: AdicAmount,
    /// Total payment for retrieval
    pub total_payment: AdicAmount,
    /// Transfer completion status
    pub status: RetrievalStatus,

    // ========== Protocol State ==========
    pub finality_status: FinalityStatus,
    pub finalized_at_epoch: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetrievalStatus {
    /// Transfer completed successfully
    Completed,
    /// Transfer failed (reason in logs)
    Failed,
    /// SLA not met (bandwidth too low)
    SlaViolation,
    /// Timeout - deadline exceeded
    Timeout,
}

impl RetrievalResponse {
    pub fn new(
        ref_request: Hash,
        provider: AccountAddress,
        bytes_transferred: u64,
        transfer_duration_secs: u64,
        price_per_gb: AdicAmount,
        status: RetrievalStatus,
    ) -> Self {
        let actual_bandwidth_bps = if transfer_duration_secs > 0 {
            bytes_transferred / transfer_duration_secs
        } else {
            0
        };

        // Calculate payment based on bytes transferred
        let gb_transferred = bytes_transferred as f64 / (1024.0 * 1024.0 * 1024.0);
        let payment_base = (price_per_gb.to_base_units() as f64 * gb_transferred) as u64;
        let total_payment = AdicAmount::from_base_units(payment_base);

        Self {
            approvals: [MessageHash::default(); 4],
            features: [0; 3],
            signature: Signature::empty(),
            deposit: AdicAmount::ZERO,
            timestamp: chrono::Utc::now().timestamp(),
            ref_request,
            provider,
            bytes_transferred,
            transfer_duration_secs,
            actual_bandwidth_bps,
            price_per_gb,
            total_payment,
            status,
            finality_status: FinalityStatus::Pending,
            finalized_at_epoch: None,
        }
    }

    pub fn is_finalized(&self) -> bool {
        self.finality_status.is_finalized()
    }
}

/// Retrieval session state
#[derive(Debug, Clone)]
pub struct RetrievalSession {
    pub request: RetrievalRequest,
    pub response: Option<RetrievalResponse>,
    pub created_epoch: u64,
    pub completed_epoch: Option<u64>,
}

/// Configuration for retrieval protocol
#[derive(Debug, Clone)]
pub struct RetrievalConfig {
    /// Default timeout for retrieval (epochs)
    pub default_timeout_epochs: u64,
    /// Minimum bandwidth SLA (bytes/sec)
    pub min_bandwidth_sla: u64,
    /// Default price per GB
    pub default_price_per_gb: AdicAmount,
    /// Maximum price per GB
    pub max_price_per_gb: AdicAmount,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            default_timeout_epochs: 100,
            min_bandwidth_sla: 1024 * 1024, // 1 MB/s
            default_price_per_gb: AdicAmount::from_adic(0.01),
            max_price_per_gb: AdicAmount::from_adic(1.0),
        }
    }
}

/// Retrieval protocol manager
pub struct RetrievalManager {
    config: RetrievalConfig,
    sessions: Arc<RwLock<HashMap<Hash, RetrievalSession>>>,
}

impl RetrievalManager {
    pub fn new(config: RetrievalConfig) -> Self {
        Self {
            config,
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Submit a retrieval request
    pub async fn submit_request(
        &self,
        request: RetrievalRequest,
        current_epoch: u64,
    ) -> Result<Hash> {
        // Validate price
        if request.max_price_per_gb > self.config.max_price_per_gb {
            return Err(StorageMarketError::Other(format!(
                "Price exceeds maximum: {} > {}",
                request.max_price_per_gb, self.config.max_price_per_gb
            )));
        }

        // Validate bandwidth SLA
        if request.min_bandwidth_bps < self.config.min_bandwidth_sla {
            return Err(StorageMarketError::Other(format!(
                "Bandwidth SLA below minimum: {} < {}",
                request.min_bandwidth_bps, self.config.min_bandwidth_sla
            )));
        }

        // Validate deadline
        if request.deadline_epoch <= current_epoch {
            return Err(StorageMarketError::Other(
                "Deadline must be in the future".to_string(),
            ));
        }

        // Create session
        let session = RetrievalSession {
            request: request.clone(),
            response: None,
            created_epoch: current_epoch,
            completed_epoch: None,
        };

        // Store session
        self.sessions
            .write()
            .await
            .insert(request.request_id, session);

        info!(
            request_id = hex::encode(&request.request_id),
            deal_id = request.deal_id,
            client = hex::encode(request.client.as_bytes()),
            provider = hex::encode(request.provider.as_bytes()),
            "📥 Retrieval request submitted"
        );

        Ok(request.request_id)
    }

    /// Submit a retrieval response (provider confirms completion)
    pub async fn submit_response(
        &self,
        response: RetrievalResponse,
        current_epoch: u64,
    ) -> Result<()> {
        // Get session
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&response.ref_request)
            .ok_or_else(|| StorageMarketError::Other("Retrieval request not found".to_string()))?;

        // Validate provider
        if session.request.provider != response.provider {
            return Err(StorageMarketError::Other(
                "Response from unauthorized provider".to_string(),
            ));
        }

        // Check deadline
        if current_epoch > session.request.deadline_epoch {
            warn!(
                request_id = hex::encode(&response.ref_request),
                "⚠️  Retrieval response after deadline"
            );
        }

        // Validate SLA
        if response.actual_bandwidth_bps < session.request.min_bandwidth_bps {
            warn!(
                request_id = hex::encode(&response.ref_request),
                actual_bps = response.actual_bandwidth_bps,
                required_bps = session.request.min_bandwidth_bps,
                "⚠️  Bandwidth SLA violated"
            );
        }

        // Update session
        session.response = Some(response.clone());
        session.completed_epoch = Some(current_epoch);

        info!(
            request_id = hex::encode(&response.ref_request),
            bytes = response.bytes_transferred,
            duration_secs = response.transfer_duration_secs,
            bandwidth_mbps = response.actual_bandwidth_bps / (1024 * 1024),
            payment = %response.total_payment,
            status = ?response.status,
            "✅ Retrieval completed"
        );

        Ok(())
    }

    /// Get retrieval session
    pub async fn get_session(&self, request_id: &Hash) -> Option<RetrievalSession> {
        self.sessions.read().await.get(request_id).cloned()
    }

    /// Get all active retrieval sessions
    pub async fn get_active_sessions(&self) -> Vec<RetrievalSession> {
        self.sessions
            .read()
            .await
            .values()
            .filter(|s| s.response.is_none())
            .cloned()
            .collect()
    }

    /// Check for timed out retrievals
    pub async fn check_timeouts(&self, current_epoch: u64) -> Result<Vec<Hash>> {
        let mut timed_out = Vec::new();
        let mut sessions = self.sessions.write().await;

        for (request_id, session) in sessions.iter_mut() {
            if session.response.is_some() {
                continue; // Already completed
            }

            if current_epoch > session.request.deadline_epoch {
                // Create timeout response
                let timeout_response = RetrievalResponse {
                    approvals: [MessageHash::default(); 4],
                    features: [0; 3],
                    signature: Signature::empty(),
                    deposit: AdicAmount::ZERO,
                    timestamp: chrono::Utc::now().timestamp(),
                    ref_request: *request_id,
                    provider: session.request.provider,
                    bytes_transferred: 0,
                    transfer_duration_secs: 0,
                    actual_bandwidth_bps: 0,
                    price_per_gb: AdicAmount::ZERO,
                    total_payment: AdicAmount::ZERO,
                    status: RetrievalStatus::Timeout,
                    finality_status: FinalityStatus::Pending,
                    finalized_at_epoch: None,
                };

                session.response = Some(timeout_response);
                session.completed_epoch = Some(current_epoch);
                timed_out.push(*request_id);

                warn!(
                    request_id = hex::encode(request_id),
                    deadline = session.request.deadline_epoch,
                    "⏰ Retrieval timed out"
                );
            }
        }

        Ok(timed_out)
    }

    /// Get retrieval statistics
    pub async fn get_stats(&self) -> RetrievalStats {
        let sessions = self.sessions.read().await;

        let total_requests = sessions.len();
        let completed = sessions
            .values()
            .filter(|s| {
                s.response
                    .as_ref()
                    .map(|r| r.status == RetrievalStatus::Completed)
                    .unwrap_or(false)
            })
            .count();
        let failed = sessions
            .values()
            .filter(|s| {
                s.response
                    .as_ref()
                    .map(|r| r.status == RetrievalStatus::Failed)
                    .unwrap_or(false)
            })
            .count();
        let sla_violations = sessions
            .values()
            .filter(|s| {
                s.response
                    .as_ref()
                    .map(|r| r.status == RetrievalStatus::SlaViolation)
                    .unwrap_or(false)
            })
            .count();
        let timeouts = sessions
            .values()
            .filter(|s| {
                s.response
                    .as_ref()
                    .map(|r| r.status == RetrievalStatus::Timeout)
                    .unwrap_or(false)
            })
            .count();
        let pending = sessions.values().filter(|s| s.response.is_none()).count();

        let total_bytes: u64 = sessions
            .values()
            .filter_map(|s| s.response.as_ref())
            .map(|r| r.bytes_transferred)
            .sum();

        let total_payments: u64 = sessions
            .values()
            .filter_map(|s| s.response.as_ref())
            .map(|r| r.total_payment.to_base_units())
            .sum();

        // Calculate average bandwidth for completed retrievals
        let bandwidth_samples: Vec<u64> = sessions
            .values()
            .filter_map(|s| s.response.as_ref())
            .filter(|r| r.status == RetrievalStatus::Completed)
            .map(|r| r.actual_bandwidth_bps)
            .collect();

        let avg_bandwidth_bps = if !bandwidth_samples.is_empty() {
            bandwidth_samples.iter().sum::<u64>() / bandwidth_samples.len() as u64
        } else {
            0
        };

        RetrievalStats {
            total_requests,
            completed_retrievals: completed,
            failed_retrievals: failed,
            sla_violations,
            timeouts,
            pending_retrievals: pending,
            total_bytes_transferred: total_bytes,
            total_payments: AdicAmount::from_base_units(total_payments),
            avg_bandwidth_bps,
        }
    }
}

/// Retrieval statistics
#[derive(Debug, Clone)]
pub struct RetrievalStats {
    pub total_requests: usize,
    pub completed_retrievals: usize,
    pub failed_retrievals: usize,
    pub sla_violations: usize,
    pub timeouts: usize,
    pub pending_retrievals: usize,
    pub total_bytes_transferred: u64,
    pub total_payments: AdicAmount,
    pub avg_bandwidth_bps: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_byte_range() {
        let range = ByteRange::new(0, 1023).unwrap();
        assert_eq!(range.size(), 1024);

        let invalid = ByteRange::new(100, 50);
        assert!(invalid.is_err());
    }

    #[tokio::test]
    async fn test_retrieval_request_submission() {
        let manager = RetrievalManager::new(RetrievalConfig::default());
        let client = AccountAddress::from_bytes([1u8; 32]);
        let provider = AccountAddress::from_bytes([2u8; 32]);

        let request = RetrievalRequest::new(
            1,
            client,
            provider,
            None,
            AdicAmount::from_adic(0.1),
            2 * 1024 * 1024, // 2 MB/s
            200,
        );

        let request_id = manager.submit_request(request, 100).await.unwrap();
        assert_ne!(request_id, [0u8; 32]);

        let session = manager.get_session(&request_id).await.unwrap();
        assert_eq!(session.created_epoch, 100);
        assert!(session.response.is_none());
    }

    #[tokio::test]
    async fn test_retrieval_response() {
        let manager = RetrievalManager::new(RetrievalConfig::default());
        let client = AccountAddress::from_bytes([1u8; 32]);
        let provider = AccountAddress::from_bytes([2u8; 32]);

        let request = RetrievalRequest::new(
            1,
            client,
            provider,
            None,
            AdicAmount::from_adic(0.1),
            1024 * 1024,
            200,
        );

        let request_id = manager.submit_request(request.clone(), 100).await.unwrap();

        // Submit response
        let response = RetrievalResponse::new(
            request_id,
            provider,
            1024 * 1024 * 1024, // 1 GB
            60,                 // 60 seconds
            AdicAmount::from_adic(0.05),
            RetrievalStatus::Completed,
        );

        manager.submit_response(response.clone(), 120).await.unwrap();

        let session = manager.get_session(&request_id).await.unwrap();
        assert!(session.response.is_some());
        assert_eq!(session.completed_epoch, Some(120));

        let resp = session.response.unwrap();
        assert_eq!(resp.status, RetrievalStatus::Completed);
        assert_eq!(resp.bytes_transferred, 1024 * 1024 * 1024);
    }

    #[tokio::test]
    async fn test_bandwidth_calculation() {
        let provider = AccountAddress::from_bytes([2u8; 32]);
        let request_id = [1u8; 32];

        let response = RetrievalResponse::new(
            request_id,
            provider,
            10 * 1024 * 1024, // 10 MB
            10,               // 10 seconds
            AdicAmount::from_adic(0.05),
            RetrievalStatus::Completed,
        );

        // Bandwidth = 10 MB / 10 sec = 1 MB/s = 1,048,576 bytes/sec
        assert_eq!(response.actual_bandwidth_bps, 1024 * 1024);
    }

    #[tokio::test]
    async fn test_retrieval_timeout() {
        let manager = RetrievalManager::new(RetrievalConfig::default());
        let client = AccountAddress::from_bytes([1u8; 32]);
        let provider = AccountAddress::from_bytes([2u8; 32]);

        let request = RetrievalRequest::new(
            1,
            client,
            provider,
            None,
            AdicAmount::from_adic(0.1),
            1024 * 1024,
            200, // Deadline at epoch 200
        );

        let request_id = manager.submit_request(request, 100).await.unwrap();

        // Check for timeouts at epoch 201
        let timed_out = manager.check_timeouts(201).await.unwrap();
        assert_eq!(timed_out.len(), 1);
        assert_eq!(timed_out[0], request_id);

        let session = manager.get_session(&request_id).await.unwrap();
        assert!(session.response.is_some());
        assert_eq!(
            session.response.unwrap().status,
            RetrievalStatus::Timeout
        );
    }

    #[tokio::test]
    async fn test_retrieval_stats() {
        let manager = RetrievalManager::new(RetrievalConfig::default());
        let client = AccountAddress::from_bytes([1u8; 32]);
        let provider = AccountAddress::from_bytes([2u8; 32]);

        // Submit 3 requests
        for i in 0..3 {
            let request = RetrievalRequest::new(
                i,
                client,
                provider,
                None,
                AdicAmount::from_adic(0.1),
                1024 * 1024,
                200,
            );
            manager.submit_request(request, 100).await.unwrap();
        }

        let stats = manager.get_stats().await;
        assert_eq!(stats.total_requests, 3);
        assert_eq!(stats.pending_retrievals, 3);
        assert_eq!(stats.completed_retrievals, 0);
    }

    #[tokio::test]
    async fn test_range_request() {
        let client = AccountAddress::from_bytes([1u8; 32]);
        let provider = AccountAddress::from_bytes([2u8; 32]);

        // Request bytes 1000-1999 (1KB)
        let range = ByteRange::new(1000, 1999).unwrap();
        let request = RetrievalRequest::new(
            1,
            client,
            provider,
            Some(range),
            AdicAmount::from_adic(0.1),
            1024 * 1024,
            200,
        );

        assert!(request.byte_range.is_some());
        assert_eq!(request.byte_range.unwrap().size(), 1000);
    }
}
