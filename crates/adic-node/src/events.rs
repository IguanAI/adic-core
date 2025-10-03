//! Event system for node state changes
//!
//! This module provides an event bus for notifying clients (WebSocket, SSE)
//! about state changes in the ADIC node without requiring polling.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::debug;

/// Maximum number of events buffered per channel before old events are dropped
const HIGH_PRIORITY_BUFFER: usize = 1000;
const MEDIUM_PRIORITY_BUFFER: usize = 500;
const LOW_PRIORITY_BUFFER: usize = 100;

/// Types of events that can be emitted by the node
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum NodeEvent {
    /// DAG tips have changed
    TipsUpdated {
        tips: Vec<String>,
        count: usize,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// A message has achieved finality
    MessageFinalized {
        message_id: String,
        finality_type: FinalityType,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// P-adic diversity metrics updated
    DiversityUpdated {
        diversity_score: f64,
        axes: Vec<AxisData>,
        total_tips: usize,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// Energy conflict state changed
    EnergyUpdated {
        total_conflicts: u32,
        resolved_conflicts: u32,
        active_conflicts: u32,
        total_energy: f64,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// K-core finality metrics updated
    KCoreUpdated {
        finalized_count: u32,
        pending_count: u32,
        current_k_value: Option<u32>,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// Admissibility compliance rates updated
    AdmissibilityUpdated {
        c1_rate: f64,
        c2_rate: f64,
        c3_rate: f64,
        overall_rate: f64,
        sample_size: usize,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// Economics/supply data updated
    EconomicsUpdated {
        total_supply: String,
        circulating_supply: String,
        treasury_balance: String,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// New message added to DAG
    MessageAdded {
        message_id: String,
        depth: u64,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// Message rejected during validation or admissibility check
    MessageRejected {
        message_id: String,
        reason: String,
        rejection_type: RejectionType,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// Deposit escrowed for message submission
    DepositEscrowed {
        message_id: String,
        validator_address: String,
        amount: String,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// Deposit slashed due to rule violation
    DepositSlashed {
        message_id: String,
        validator_address: String,
        amount: String,
        reason: String,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// Deposit released back to validator
    DepositReleased {
        message_id: String,
        validator_address: String,
        amount: String,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// Balance changed (credit, debit, or transfer)
    BalanceChanged {
        address: String,
        balance_before: String,
        balance_after: String,
        change_amount: String,
        change_type: BalanceChangeType,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// Transfer recorded (from -> to)
    TransferRecorded {
        from_address: String,
        to_address: String,
        amount: String,
        reason: String,
        tx_hash: Option<String>,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// Peer connected to network
    PeerConnected {
        peer_id: String,
        address: String,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// Peer disconnected from network
    PeerDisconnected {
        peer_id: String,
        reason: Option<String>,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// State sync started
    SyncStarted {
        peer_id: String,
        target_height: u64,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// State sync progress update
    SyncProgress {
        peer_id: String,
        synced_messages: usize,
        progress_percent: f64,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// State sync completed
    SyncCompleted {
        peer_id: String,
        synced_messages: usize,
        duration_ms: u64,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// Validator reputation changed
    ReputationChanged {
        validator_address: String,
        old_reputation: f64,
        new_reputation: f64,
        reason: String,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// Node started and ready
    NodeStarted {
        node_id: String,
        version: String,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// Node stopped
    NodeStopped {
        node_id: String,
        reason: String,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },

    /// Genesis state loaded
    GenesisLoaded {
        chain_id: String,
        genesis_hash: String,
        total_supply: String,
        #[serde(with = "chrono::serde::ts_seconds")]
        timestamp: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FinalityType {
    #[serde(rename = "kcore")]
    KCore,
    #[serde(rename = "homology")]
    Homology,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RejectionType {
    #[serde(rename = "admissibility_failed")]
    AdmissibilityFailed,
    #[serde(rename = "validation_failed")]
    ValidationFailed,
    #[serde(rename = "insufficient_deposit")]
    InsufficientDeposit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BalanceChangeType {
    #[serde(rename = "credit")]
    Credit,
    #[serde(rename = "debit")]
    Debit,
    #[serde(rename = "transfer_in")]
    TransferIn,
    #[serde(rename = "transfer_out")]
    TransferOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxisData {
    pub axis: u32,
    pub radius: u32,
    pub distinct_balls: usize,
    pub required_diversity: usize,
    pub meets_requirement: bool,
}

impl NodeEvent {
    /// Get the event type as a string (for SSE event names)
    pub fn event_type(&self) -> &'static str {
        match self {
            NodeEvent::TipsUpdated { .. } => "tips.update",
            NodeEvent::MessageFinalized { .. } => "finality.update",
            NodeEvent::DiversityUpdated { .. } => "diversity.update",
            NodeEvent::EnergyUpdated { .. } => "energy.update",
            NodeEvent::KCoreUpdated { .. } => "kcore.update",
            NodeEvent::AdmissibilityUpdated { .. } => "admissibility.update",
            NodeEvent::EconomicsUpdated { .. } => "economics.update",
            NodeEvent::MessageAdded { .. } => "message.new",
            NodeEvent::MessageRejected { .. } => "message.rejected",
            NodeEvent::DepositEscrowed { .. } => "deposit.escrowed",
            NodeEvent::DepositSlashed { .. } => "deposit.slashed",
            NodeEvent::DepositReleased { .. } => "deposit.released",
            NodeEvent::BalanceChanged { .. } => "balance.changed",
            NodeEvent::TransferRecorded { .. } => "transfer.recorded",
            NodeEvent::PeerConnected { .. } => "peer.connected",
            NodeEvent::PeerDisconnected { .. } => "peer.disconnected",
            NodeEvent::SyncStarted { .. } => "sync.started",
            NodeEvent::SyncProgress { .. } => "sync.progress",
            NodeEvent::SyncCompleted { .. } => "sync.completed",
            NodeEvent::ReputationChanged { .. } => "reputation.changed",
            NodeEvent::NodeStarted { .. } => "node.started",
            NodeEvent::NodeStopped { .. } => "node.stopped",
            NodeEvent::GenesisLoaded { .. } => "genesis.loaded",
        }
    }

    /// Get the event priority level
    pub fn priority(&self) -> EventPriority {
        match self {
            // High priority: Real-time consensus events
            NodeEvent::TipsUpdated { .. } => EventPriority::High,
            NodeEvent::MessageFinalized { .. } => EventPriority::High,
            NodeEvent::MessageAdded { .. } => EventPriority::High,
            NodeEvent::MessageRejected { .. } => EventPriority::High,
            NodeEvent::DepositEscrowed { .. } => EventPriority::High,
            NodeEvent::DepositSlashed { .. } => EventPriority::High,
            NodeEvent::BalanceChanged { .. } => EventPriority::High,
            NodeEvent::TransferRecorded { .. } => EventPriority::High,
            NodeEvent::PeerConnected { .. } => EventPriority::High,
            NodeEvent::PeerDisconnected { .. } => EventPriority::High,

            // Medium priority: Frequent metric updates
            NodeEvent::DiversityUpdated { .. } => EventPriority::Medium,
            NodeEvent::EnergyUpdated { .. } => EventPriority::Medium,
            NodeEvent::KCoreUpdated { .. } => EventPriority::Medium,
            NodeEvent::DepositReleased { .. } => EventPriority::Medium,
            NodeEvent::SyncStarted { .. } => EventPriority::Medium,
            NodeEvent::SyncProgress { .. } => EventPriority::Medium,
            NodeEvent::SyncCompleted { .. } => EventPriority::Medium,
            NodeEvent::ReputationChanged { .. } => EventPriority::Medium,

            // Low priority: Infrequent updates
            NodeEvent::AdmissibilityUpdated { .. } => EventPriority::Low,
            NodeEvent::EconomicsUpdated { .. } => EventPriority::Low,
            NodeEvent::NodeStarted { .. } => EventPriority::Low,
            NodeEvent::NodeStopped { .. } => EventPriority::Low,
            NodeEvent::GenesisLoaded { .. } => EventPriority::Low,
        }
    }

    /// Get the timestamp of the event
    #[allow(dead_code)]
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            NodeEvent::TipsUpdated { timestamp, .. } => *timestamp,
            NodeEvent::MessageFinalized { timestamp, .. } => *timestamp,
            NodeEvent::DiversityUpdated { timestamp, .. } => *timestamp,
            NodeEvent::EnergyUpdated { timestamp, .. } => *timestamp,
            NodeEvent::KCoreUpdated { timestamp, .. } => *timestamp,
            NodeEvent::AdmissibilityUpdated { timestamp, .. } => *timestamp,
            NodeEvent::EconomicsUpdated { timestamp, .. } => *timestamp,
            NodeEvent::MessageAdded { timestamp, .. } => *timestamp,
            NodeEvent::MessageRejected { timestamp, .. } => *timestamp,
            NodeEvent::DepositEscrowed { timestamp, .. } => *timestamp,
            NodeEvent::DepositSlashed { timestamp, .. } => *timestamp,
            NodeEvent::DepositReleased { timestamp, .. } => *timestamp,
            NodeEvent::BalanceChanged { timestamp, .. } => *timestamp,
            NodeEvent::TransferRecorded { timestamp, .. } => *timestamp,
            NodeEvent::PeerConnected { timestamp, .. } => *timestamp,
            NodeEvent::PeerDisconnected { timestamp, .. } => *timestamp,
            NodeEvent::SyncStarted { timestamp, .. } => *timestamp,
            NodeEvent::SyncProgress { timestamp, .. } => *timestamp,
            NodeEvent::SyncCompleted { timestamp, .. } => *timestamp,
            NodeEvent::ReputationChanged { timestamp, .. } => *timestamp,
            NodeEvent::NodeStarted { timestamp, .. } => *timestamp,
            NodeEvent::NodeStopped { timestamp, .. } => *timestamp,
            NodeEvent::GenesisLoaded { timestamp, .. } => *timestamp,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventPriority {
    High,
    Medium,
    Low,
}

/// Event bus for broadcasting node state changes
///
/// Uses multiple priority channels to ensure high-priority events
/// (tips, finality) are delivered with minimal latency even under load.
#[derive(Clone)]
pub struct EventBus {
    high_priority: broadcast::Sender<NodeEvent>,
    medium_priority: broadcast::Sender<NodeEvent>,
    low_priority: broadcast::Sender<NodeEvent>,
    metrics: Arc<std::sync::atomic::AtomicU64>,
}

impl EventBus {
    /// Create a new event bus
    pub fn new() -> Self {
        let (high_tx, _) = broadcast::channel(HIGH_PRIORITY_BUFFER);
        let (medium_tx, _) = broadcast::channel(MEDIUM_PRIORITY_BUFFER);
        let (low_tx, _) = broadcast::channel(LOW_PRIORITY_BUFFER);

        Self {
            high_priority: high_tx,
            medium_priority: medium_tx,
            low_priority: low_tx,
            metrics: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Subscribe to all event channels
    ///
    /// Returns three receivers: (high, medium, low)
    /// Consumers should listen to all three channels
    pub fn subscribe_all(
        &self,
    ) -> (
        broadcast::Receiver<NodeEvent>,
        broadcast::Receiver<NodeEvent>,
        broadcast::Receiver<NodeEvent>,
    ) {
        (
            self.high_priority.subscribe(),
            self.medium_priority.subscribe(),
            self.low_priority.subscribe(),
        )
    }

    /// Subscribe to high-priority events only
    #[allow(dead_code)]
    pub fn subscribe_high_priority(&self) -> broadcast::Receiver<NodeEvent> {
        self.high_priority.subscribe()
    }

    /// Subscribe to medium-priority events only
    #[allow(dead_code)]
    pub fn subscribe_medium_priority(&self) -> broadcast::Receiver<NodeEvent> {
        self.medium_priority.subscribe()
    }

    /// Subscribe to low-priority events only
    #[allow(dead_code)]
    pub fn subscribe_low_priority(&self) -> broadcast::Receiver<NodeEvent> {
        self.low_priority.subscribe()
    }

    /// Emit an event to all subscribers
    ///
    /// Events are routed to the appropriate priority channel based on their type.
    /// If no subscribers are listening, the event is dropped (this is expected).
    pub fn emit(&self, event: NodeEvent) {
        let channel = match event.priority() {
            EventPriority::High => &self.high_priority,
            EventPriority::Medium => &self.medium_priority,
            EventPriority::Low => &self.low_priority,
        };

        match channel.send(event.clone()) {
            Ok(subscriber_count) => {
                debug!(
                    event_type = event.event_type(),
                    priority = ?event.priority(),
                    subscribers = subscriber_count,
                    "Event emitted"
                );
                self.metrics
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Err(_) => {
                // No subscribers, this is normal and not an error
                debug!(
                    event_type = event.event_type(),
                    "Event emitted but no subscribers listening"
                );
            }
        }
    }

    /// Emit multiple events in a batch
    ///
    /// More efficient than calling emit() multiple times when you have
    /// multiple events to send at once.
    #[allow(dead_code)]
    pub fn emit_batch(&self, events: Vec<NodeEvent>) {
        for event in events {
            self.emit(event);
        }
    }

    /// Get the number of active subscribers across all channels
    #[allow(dead_code)]
    pub fn subscriber_count(&self) -> usize {
        self.high_priority.receiver_count()
            + self.medium_priority.receiver_count()
            + self.low_priority.receiver_count()
    }

    /// Get the total number of events emitted since creation
    #[allow(dead_code)]
    pub fn total_events_emitted(&self) -> u64 {
        self.metrics.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Check if any subscribers are listening
    #[allow(dead_code)]
    pub fn has_subscribers(&self) -> bool {
        self.subscriber_count() > 0
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_event_bus_creation() {
        let bus = EventBus::new();
        assert_eq!(bus.subscriber_count(), 0);
        assert!(!bus.has_subscribers());
    }

    #[tokio::test]
    async fn test_subscribe_and_emit() {
        let bus = EventBus::new();
        let (mut high_rx, mut medium_rx, _low_rx) = bus.subscribe_all();

        assert_eq!(bus.subscriber_count(), 3);
        assert!(bus.has_subscribers());

        // Emit high-priority event
        bus.emit(NodeEvent::TipsUpdated {
            tips: vec!["tip1".to_string()],
            count: 1,
            timestamp: Utc::now(),
        });

        // Should receive on high priority channel
        let received = high_rx.try_recv();
        assert!(received.is_ok());

        // Should not receive on medium priority channel
        let not_received = medium_rx.try_recv();
        assert!(not_received.is_err());
    }

    #[tokio::test]
    async fn test_event_priority_routing() {
        let bus = EventBus::new();
        let (mut high_rx, mut medium_rx, mut low_rx) = bus.subscribe_all();

        // High priority event
        bus.emit(NodeEvent::MessageFinalized {
            message_id: "msg1".to_string(),
            finality_type: FinalityType::KCore,
            timestamp: Utc::now(),
        });
        assert!(high_rx.try_recv().is_ok());

        // Medium priority event
        bus.emit(NodeEvent::DiversityUpdated {
            diversity_score: 0.85,
            axes: vec![],
            total_tips: 10,
            timestamp: Utc::now(),
        });
        assert!(medium_rx.try_recv().is_ok());

        // Low priority event
        bus.emit(NodeEvent::EconomicsUpdated {
            total_supply: "1000000".to_string(),
            circulating_supply: "500000".to_string(),
            treasury_balance: "100000".to_string(),
            timestamp: Utc::now(),
        });
        assert!(low_rx.try_recv().is_ok());
    }

    #[tokio::test]
    async fn test_event_metrics() {
        let bus = EventBus::new();
        let (_rx1, _rx2, _rx3) = bus.subscribe_all();

        assert_eq!(bus.total_events_emitted(), 0);

        bus.emit(NodeEvent::TipsUpdated {
            tips: vec![],
            count: 0,
            timestamp: Utc::now(),
        });

        assert_eq!(bus.total_events_emitted(), 1);
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let bus = EventBus::new();
        let (_rx1a, _rx1b, _rx1c) = bus.subscribe_all();
        let (_rx2a, _rx2b, _rx2c) = bus.subscribe_all();

        assert_eq!(bus.subscriber_count(), 6); // 3 channels * 2 subscribers
    }
}
