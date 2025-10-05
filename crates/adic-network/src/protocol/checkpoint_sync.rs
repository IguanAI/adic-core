use adic_finality::Checkpoint;
use adic_types::{MessageId, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, warn};

/// Checkpoint synchronization protocol
/// Per ADIC-DAG Yellow Paper §5 - Anti-Entropy Protocol
///
/// Enables efficient synchronization between nodes using checkpoints
/// to identify divergence and missing messages.
pub struct CheckpointSyncProtocol {
    event_sender: mpsc::UnboundedSender<CheckpointSyncEvent>,
    event_receiver: Arc<RwLock<mpsc::UnboundedReceiver<CheckpointSyncEvent>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CheckpointSyncMessage {
    /// Request the latest checkpoint from a peer
    RequestLatestCheckpoint,

    /// Response with the latest checkpoint
    LatestCheckpoint(Option<Checkpoint>),

    /// Request a specific checkpoint by height
    RequestCheckpoint(u64),

    /// Response with requested checkpoint
    Checkpoint(Option<Checkpoint>),

    /// Request all checkpoints from height onwards
    RequestCheckpointsFrom(u64),

    /// Response with multiple checkpoints
    Checkpoints(Vec<Checkpoint>),

    /// Detect divergence - send our checkpoint at height
    DetectDivergence {
        height: u64,
        our_checkpoint: Checkpoint,
    },

    /// Divergence detected - checkpoint hashes don't match
    DivergenceDetected {
        height: u64,
        their_hash: [u8; 32],
        our_hash: [u8; 32],
    },

    /// Request messages missing between checkpoints
    RequestMissingMessages {
        from_height: u64,
        to_height: u64,
        missing_ids: Vec<MessageId>,
    },

    /// Response with requested messages
    Messages(Vec<adic_types::AdicMessage>),
}

#[derive(Debug, Clone)]
pub enum CheckpointSyncEvent {
    MessageReceived(Box<CheckpointSyncMessage>, libp2p::PeerId),
    DivergenceDetected {
        peer_id: libp2p::PeerId,
        height: u64,
    },
    SyncCompleted {
        peer_id: libp2p::PeerId,
        messages_synced: usize,
    },
    SyncFailed {
        peer_id: libp2p::PeerId,
        error: String,
    },
}

impl CheckpointSyncProtocol {
    pub fn new() -> Self {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();

        Self {
            event_sender,
            event_receiver: Arc::new(RwLock::new(event_receiver)),
        }
    }

    /// Request the latest checkpoint from a peer
    pub async fn request_latest_checkpoint(&self) -> Result<()> {
        // This would send via network - for now just log
        debug!("Requesting latest checkpoint from peer");
        Ok(())
    }

    /// Compare checkpoints to detect divergence
    pub fn detect_divergence(
        our_checkpoint: &Checkpoint,
        their_checkpoint: &Checkpoint,
    ) -> Option<DivergenceInfo> {
        if our_checkpoint.height != their_checkpoint.height {
            return None; // Can't compare different heights
        }

        if our_checkpoint.diverges_from(their_checkpoint) {
            Some(DivergenceInfo {
                height: our_checkpoint.height,
                our_hash: our_checkpoint.compute_hash(),
                their_hash: their_checkpoint.compute_hash(),
                our_message_root: our_checkpoint.messages_merkle_root,
                their_message_root: their_checkpoint.messages_merkle_root,
            })
        } else {
            None
        }
    }

    /// Analyze divergence to determine missing messages
    pub fn analyze_divergence(
        our_checkpoint: &Checkpoint,
        their_checkpoint: &Checkpoint,
    ) -> Vec<MessageId> {
        // Compare finalized message sets to find missing ones
        let our_messages: std::collections::HashSet<_> = our_checkpoint
            .f1_metadata
            .finalized_messages
            .iter()
            .collect();
        let their_messages: std::collections::HashSet<_> = their_checkpoint
            .f1_metadata
            .finalized_messages
            .iter()
            .collect();

        // Messages they have that we don't
        their_messages
            .difference(&our_messages)
            .copied()
            .copied()
            .collect()
    }

    /// Get event stream for checkpoint sync events
    pub fn event_stream(&self) -> Arc<RwLock<mpsc::UnboundedReceiver<CheckpointSyncEvent>>> {
        self.event_receiver.clone()
    }

    /// Handle incoming checkpoint sync message
    pub async fn handle_message(&self, message: CheckpointSyncMessage, peer_id: libp2p::PeerId) {
        self.event_sender
            .send(CheckpointSyncEvent::MessageReceived(Box::new(message), peer_id))
            .ok();
    }
}

impl Default for CheckpointSyncProtocol {
    fn default() -> Self {
        Self::new()
    }
}

/// Information about detected divergence
#[derive(Debug, Clone)]
pub struct DivergenceInfo {
    pub height: u64,
    pub our_hash: [u8; 32],
    pub their_hash: [u8; 32],
    pub our_message_root: [u8; 32],
    pub their_message_root: [u8; 32],
}

impl DivergenceInfo {
    /// Log divergence details for debugging
    pub fn log(&self) {
        warn!(
            height = self.height,
            our_hash = hex::encode(self.our_hash),
            their_hash = hex::encode(self.their_hash),
            our_msg_root = hex::encode(self.our_message_root),
            their_msg_root = hex::encode(self.their_message_root),
            "🚨 Checkpoint divergence detected"
        );
    }
}

/// Checkpoint synchronization strategy
pub struct CheckpointSyncStrategy {
    /// How often to perform checkpoint sync (in seconds)
    pub sync_interval_secs: u64,

    /// Maximum number of checkpoints to request at once
    pub max_checkpoints_per_request: usize,

    /// Maximum number of messages to request at once
    pub max_messages_per_request: usize,
}

impl Default for CheckpointSyncStrategy {
    fn default() -> Self {
        Self {
            sync_interval_secs: 60, // Sync every minute
            max_checkpoints_per_request: 100,
            max_messages_per_request: 1000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_finality::{F1Metadata, F2Metadata};

    fn create_test_checkpoint(height: u64, messages: Vec<MessageId>) -> Checkpoint {
        let f1_meta = F1Metadata::new(3, messages);
        let f2_meta = F2Metadata::empty();

        Checkpoint::new(
            height,
            chrono::Utc::now().timestamp(),
            [0u8; 32],
            [1u8; 32],
            f1_meta,
            f2_meta,
            None,
        )
    }

    #[test]
    fn test_checkpoint_sync_protocol_creation() {
        let protocol = CheckpointSyncProtocol::new();
        assert!(!protocol.event_sender.is_closed());
    }

    #[test]
    fn test_detect_divergence_same_checkpoints() {
        let msg_id = MessageId::from_bytes([1u8; 32]);
        let checkpoint1 = create_test_checkpoint(1, vec![msg_id]);
        let checkpoint2 = create_test_checkpoint(1, vec![msg_id]);

        let divergence = CheckpointSyncProtocol::detect_divergence(&checkpoint1, &checkpoint2);
        assert!(divergence.is_none());
    }

    #[test]
    fn test_detect_divergence_different_checkpoints() {
        let msg_id1 = MessageId::from_bytes([1u8; 32]);
        let msg_id2 = MessageId::from_bytes([2u8; 32]);

        let mut checkpoint1 = create_test_checkpoint(1, vec![msg_id1]);
        checkpoint1.messages_merkle_root = [1u8; 32];

        let mut checkpoint2 = create_test_checkpoint(1, vec![msg_id2]);
        checkpoint2.messages_merkle_root = [2u8; 32];

        let divergence = CheckpointSyncProtocol::detect_divergence(&checkpoint1, &checkpoint2);
        assert!(divergence.is_some());

        let div_info = divergence.unwrap();
        assert_eq!(div_info.height, 1);
        assert_ne!(div_info.our_hash, div_info.their_hash);
    }

    #[test]
    fn test_analyze_divergence() {
        let msg_id1 = MessageId::from_bytes([1u8; 32]);
        let msg_id2 = MessageId::from_bytes([2u8; 32]);
        let msg_id3 = MessageId::from_bytes([3u8; 32]);

        let checkpoint1 = create_test_checkpoint(1, vec![msg_id1, msg_id2]);
        let checkpoint2 = create_test_checkpoint(1, vec![msg_id2, msg_id3]);

        let missing = CheckpointSyncProtocol::analyze_divergence(&checkpoint1, &checkpoint2);

        // We should be missing msg_id3 (they have it, we don't)
        assert_eq!(missing.len(), 1);
        assert!(missing.contains(&msg_id3));
    }

    #[test]
    fn test_checkpoint_sync_strategy_defaults() {
        let strategy = CheckpointSyncStrategy::default();
        assert_eq!(strategy.sync_interval_secs, 60);
        assert_eq!(strategy.max_checkpoints_per_request, 100);
        assert_eq!(strategy.max_messages_per_request, 1000);
    }
}
