use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock, Semaphore};
use tracing::{debug, info, warn};

use adic_storage::StorageEngine;
use adic_types::{AdicError, AdicMessage, MessageId, Result};
use libp2p::PeerId;

#[derive(Debug, Clone)]
pub struct SyncConfig {
    pub batch_size: usize,
    pub parallel_downloads: usize,
    pub request_timeout: Duration,
    pub retry_attempts: usize,
    pub chunk_size: usize,
    pub max_pending_requests: usize,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            batch_size: 1000,
            parallel_downloads: 8,
            request_timeout: Duration::from_secs(30),
            retry_attempts: 3,
            chunk_size: 100,
            max_pending_requests: 50,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncRequest {
    GetFrontier,
    GetMessages(Vec<MessageId>),
    GetRange {
        from: MessageId,
        to: MessageId,
        max_messages: usize,
    },
    GetAncestors {
        message_id: MessageId,
        depth: usize,
    },
    GetDescendants {
        message_id: MessageId,
        depth: usize,
    },
    GetCheckpoint(u64), // Block height
    GetProof(MessageId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncResponse {
    Frontier(Vec<MessageId>),
    Messages(Vec<AdicMessage>),
    Range {
        messages: Vec<AdicMessage>,
        has_more: bool,
    },
    Ancestors(Vec<AdicMessage>),
    Descendants(Vec<AdicMessage>),
    Checkpoint {
        height: u64,
        root: MessageId,
        messages: Vec<AdicMessage>,
    },
    Proof {
        message_id: MessageId,
        path: Vec<[u8; 32]>,
        root: [u8; 32],
    },
    Error(String),
}

#[derive(Debug, Clone)]
struct SyncState {
    frontier: HashSet<MessageId>,
    syncing_from: HashMap<PeerId, Instant>,
    pending_requests: HashMap<u64, (SyncRequest, PeerId, Instant)>,
    retry_count: HashMap<u64, usize>,
    next_request_id: u64,
}

impl SyncState {
    fn new() -> Self {
        Self {
            frontier: HashSet::new(),
            syncing_from: HashMap::new(),
            pending_requests: HashMap::new(),
            retry_count: HashMap::new(),
            next_request_id: 0,
        }
    }

    fn generate_request_id(&mut self) -> u64 {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
    }
}

pub struct SyncProtocol {
    config: SyncConfig,
    state: Arc<RwLock<SyncState>>,
    storage: Arc<StorageEngine>,
    download_semaphore: Arc<Semaphore>,
    event_sender: mpsc::UnboundedSender<SyncEvent>,
    event_receiver: Arc<RwLock<mpsc::UnboundedReceiver<SyncEvent>>>,
}

#[derive(Debug, Clone)]
pub enum SyncEvent {
    SyncStarted(PeerId),
    SyncProgress(PeerId, f64), // 0.0 to 1.0
    SyncCompleted(PeerId),
    SyncFailed(PeerId, String),
    MessagesReceived(Vec<AdicMessage>),
    FrontierUpdated(Vec<MessageId>),
}

impl SyncProtocol {
    pub fn new(config: SyncConfig, storage: Arc<StorageEngine>) -> Self {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();

        Self {
            config: config.clone(),
            state: Arc::new(RwLock::new(SyncState::new())),
            storage,
            download_semaphore: Arc::new(Semaphore::new(config.parallel_downloads)),
            event_sender,
            event_receiver: Arc::new(RwLock::new(event_receiver)),
        }
    }

    pub async fn request_frontier(&self, peer: PeerId) -> Result<u64> {
        let mut state = self.state.write().await;
        let request_id = state.generate_request_id();

        state
            .pending_requests
            .insert(request_id, (SyncRequest::GetFrontier, peer, Instant::now()));

        debug!("Requesting frontier from peer {}", peer);
        Ok(request_id)
    }

    pub async fn request_messages(&self, peer: PeerId, message_ids: Vec<MessageId>) -> Result<u64> {
        let mut state = self.state.write().await;
        let request_id = state.generate_request_id();

        state.pending_requests.insert(
            request_id,
            (
                SyncRequest::GetMessages(message_ids.clone()),
                peer,
                Instant::now(),
            ),
        );

        debug!(
            "Requesting {} messages from peer {}",
            message_ids.len(),
            peer
        );
        Ok(request_id)
    }

    pub async fn request_range(
        &self,
        peer: PeerId,
        from: MessageId,
        to: MessageId,
        max_messages: usize,
    ) -> Result<u64> {
        let mut state = self.state.write().await;
        let request_id = state.generate_request_id();

        state.pending_requests.insert(
            request_id,
            (
                SyncRequest::GetRange {
                    from,
                    to,
                    max_messages,
                },
                peer,
                Instant::now(),
            ),
        );

        debug!(
            "Requesting range from {} to {} from peer {}",
            from, to, peer
        );
        Ok(request_id)
    }

    pub async fn handle_request(&self, request: SyncRequest) -> SyncResponse {
        match request {
            SyncRequest::GetFrontier => {
                let state = self.state.read().await;
                SyncResponse::Frontier(state.frontier.iter().cloned().collect())
            }
            SyncRequest::GetMessages(ids) => {
                // Fetch messages from storage
                let mut messages = Vec::new();
                for id in ids {
                    match self.storage.get_message(&id).await {
                        Ok(Some(msg)) => messages.push(msg),
                        Ok(None) => {
                            debug!("Message {} not found in storage", id);
                        }
                        Err(e) => {
                            warn!("Error fetching message {}: {}", id, e);
                        }
                    }
                }
                SyncResponse::Messages(messages)
            }
            SyncRequest::GetRange {
                from,
                to: _,
                max_messages,
            } => {
                // Get messages since the 'from' checkpoint
                // Note: 'to' is currently ignored - a full implementation would
                // traverse the DAG to find messages between from and to
                match self.storage.get_messages_since(&from, max_messages).await {
                    Ok(messages) => {
                        let has_more = messages.len() == max_messages;
                        SyncResponse::Range { messages, has_more }
                    }
                    Err(e) => {
                        warn!("Error getting range from {}: {}", from, e);
                        SyncResponse::Error(format!("Failed to get range: {}", e))
                    }
                }
            }
            SyncRequest::GetAncestors { message_id, depth } => {
                // Traverse DAG to get ancestors
                match self.storage.get_ancestors(&message_id, depth).await {
                    Ok(ancestors) => SyncResponse::Ancestors(ancestors),
                    Err(e) => {
                        warn!("Error getting ancestors for {}: {}", message_id, e);
                        SyncResponse::Error(format!("Failed to get ancestors: {}", e))
                    }
                }
            }
            SyncRequest::GetDescendants { message_id, depth } => {
                // Traverse DAG to get descendants
                match self.storage.get_descendants(&message_id, depth).await {
                    Ok(descendants) => SyncResponse::Descendants(descendants),
                    Err(e) => {
                        warn!("Error getting descendants for {}: {}", message_id, e);
                        SyncResponse::Error(format!("Failed to get descendants: {}", e))
                    }
                }
            }
            SyncRequest::GetCheckpoint(height) => {
                // Fetch checkpoint from storage
                match self.storage.get_checkpoint_data(height, 100).await {
                    Ok(Some((root, messages))) => SyncResponse::Checkpoint {
                        height,
                        root,
                        messages,
                    },
                    Ok(None) => {
                        debug!("Checkpoint at height {} not found", height);
                        SyncResponse::Error(format!("Checkpoint not found at height {}", height))
                    }
                    Err(e) => {
                        warn!("Error fetching checkpoint at height {}: {}", height, e);
                        SyncResponse::Error(format!("Failed to fetch checkpoint: {}", e))
                    }
                }
            }
            SyncRequest::GetProof(message_id) => {
                // Generate Merkle proof for the message
                match self.storage.generate_merkle_proof(&message_id).await {
                    Ok((path, root)) => SyncResponse::Proof {
                        message_id,
                        path,
                        root,
                    },
                    Err(e) => {
                        warn!("Error generating proof for {}: {}", message_id, e);
                        SyncResponse::Error(format!("Failed to generate proof: {}", e))
                    }
                }
            }
        }
    }

    pub async fn handle_response(&self, request_id: u64, response: SyncResponse) -> Result<()> {
        let mut state = self.state.write().await;

        if let Some((request, peer, _)) = state.pending_requests.remove(&request_id) {
            match response {
                SyncResponse::Frontier(frontier) => {
                    state.frontier = frontier.iter().cloned().collect();
                    self.event_sender
                        .send(SyncEvent::FrontierUpdated(frontier))
                        .ok();
                }
                SyncResponse::Messages(messages) => {
                    self.event_sender
                        .send(SyncEvent::MessagesReceived(messages))
                        .ok();
                }
                SyncResponse::Range { messages, has_more } => {
                    self.event_sender
                        .send(SyncEvent::MessagesReceived(messages))
                        .ok();
                    if has_more {
                        // Request next batch
                        debug!("More messages available, requesting next batch");
                    }
                }
                SyncResponse::Error(err) => {
                    warn!("Sync request {} failed: {}", request_id, err);
                    self.handle_retry(&mut state, request_id, request, peer)
                        .await?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    async fn handle_retry(
        &self,
        state: &mut SyncState,
        request_id: u64,
        request: SyncRequest,
        peer: PeerId,
    ) -> Result<()> {
        let retry_count = state.retry_count.entry(request_id).or_insert(0);
        *retry_count += 1;
        let current_retry_count = *retry_count;

        if current_retry_count <= self.config.retry_attempts {
            state
                .pending_requests
                .insert(request_id, (request, peer, Instant::now()));
            debug!(
                "Retrying request {} (attempt {})",
                request_id, current_retry_count
            );
        } else {
            state.retry_count.remove(&request_id);
            self.event_sender
                .send(SyncEvent::SyncFailed(
                    peer,
                    format!(
                        "Request {} failed after {} attempts",
                        request_id, current_retry_count
                    ),
                ))
                .ok();
        }

        Ok(())
    }

    pub async fn start_incremental_sync(&self, peer: PeerId) -> Result<()> {
        let permit = self
            .download_semaphore
            .acquire()
            .await
            .map_err(|e| AdicError::Network(format!("Failed to acquire semaphore: {}", e)))?;

        {
            let mut state = self.state.write().await;
            state.syncing_from.insert(peer, Instant::now());
        }

        self.event_sender.send(SyncEvent::SyncStarted(peer)).ok();

        // Store the request for tracking
        let request_id = {
            let mut state = self.state.write().await;
            let id = state.generate_request_id();
            state
                .pending_requests
                .insert(id, (SyncRequest::GetFrontier, peer, Instant::now()));
            id
        };

        info!(
            "Starting incremental sync with peer {} (request {})",
            peer, request_id
        );

        // Note: The actual sync request sending is handled by the NetworkEngine
        // which has access to the transport layer. This function just tracks the sync state.
        // The NetworkEngine will call send_sync_request() to actually send the GetFrontier request.

        drop(permit);

        // Don't mark as complete here - wait for actual response
        // The sync will be marked complete when we receive and process the response

        Ok(())
    }

    pub async fn start_checkpoint_sync(&self, peer: PeerId, checkpoint_height: u64) -> Result<()> {
        let mut state = self.state.write().await;
        let request_id = state.generate_request_id();

        state.pending_requests.insert(
            request_id,
            (
                SyncRequest::GetCheckpoint(checkpoint_height),
                peer,
                Instant::now(),
            ),
        );

        info!(
            "Starting checkpoint sync from height {} with peer {}",
            checkpoint_height, peer
        );
        Ok(())
    }

    pub async fn cleanup_stale_requests(&self) {
        let mut state = self.state.write().await;
        let now = Instant::now();
        let timeout = self.config.request_timeout;

        let stale_requests: Vec<u64> = state
            .pending_requests
            .iter()
            .filter(|(_, (_, _, timestamp))| now.duration_since(*timestamp) > timeout)
            .map(|(id, _)| *id)
            .collect();

        for request_id in stale_requests {
            if let Some((request, peer, _)) = state.pending_requests.remove(&request_id) {
                warn!("Request {} timed out for peer {}", request_id, peer);
                self.handle_retry(&mut state, request_id, request, peer)
                    .await
                    .ok();
            }
        }
    }

    pub async fn get_sync_progress(&self, peer: &PeerId) -> Option<f64> {
        let state = self.state.read().await;
        if state.syncing_from.contains_key(peer) {
            // In a real implementation, calculate actual progress
            Some(0.5)
        } else {
            None
        }
    }

    pub async fn is_syncing(&self) -> bool {
        let state = self.state.read().await;
        !state.syncing_from.is_empty()
    }

    pub async fn syncing_peers(&self) -> Vec<PeerId> {
        let state = self.state.read().await;
        state.syncing_from.keys().cloned().collect()
    }

    pub fn event_stream(&self) -> Arc<RwLock<mpsc::UnboundedReceiver<SyncEvent>>> {
        self.event_receiver.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_storage::{StorageConfig, StorageEngine, store::BackendType};
    use adic_types::{AdicFeatures, AdicMeta, AdicMessage, AxisPhi, QpDigits, PublicKey};
    use chrono::Utc;

    fn create_test_storage() -> Arc<StorageEngine> {
        let storage_config = StorageConfig {
            backend_type: BackendType::Memory,
            cache_size: 1000,
            flush_interval_ms: 5000,
            max_batch_size: 100,
        };
        Arc::new(StorageEngine::new(storage_config).unwrap())
    }

    #[tokio::test]
    async fn test_sync_protocol_creation() {
        let config = SyncConfig::default();
        let storage = create_test_storage();
        let protocol = SyncProtocol::new(config, storage);

        assert!(!protocol.is_syncing().await);
        assert_eq!(protocol.syncing_peers().await.len(), 0);
    }

    #[tokio::test]
    async fn test_request_generation() {
        let config = SyncConfig::default();
        let storage = create_test_storage();
        let protocol = SyncProtocol::new(config, storage);
        let peer = PeerId::random();

        let request_id1 = protocol.request_frontier(peer).await.unwrap();
        let request_id2 = protocol.request_frontier(peer).await.unwrap();

        assert_ne!(request_id1, request_id2);
    }

    #[tokio::test]
    async fn test_get_messages_handler() {
        let config = SyncConfig::default();
        let storage = create_test_storage();

        // Store test messages
        let msg1 = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(10, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([1; 32]),
            vec![1, 2, 3],
        );
        let msg2 = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(20, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([2; 32]),
            vec![4, 5, 6],
        );

        storage.store_message(&msg1).await.unwrap();
        storage.store_message(&msg2).await.unwrap();

        let protocol = SyncProtocol::new(config, storage);

        // Test GetMessages request
        let request = SyncRequest::GetMessages(vec![msg1.id.clone(), msg2.id.clone()]);
        let response = protocol.handle_request(request).await;

        match response {
            SyncResponse::Messages(messages) => {
                assert_eq!(messages.len(), 2);
                assert!(messages.iter().any(|m| m.id == msg1.id));
                assert!(messages.iter().any(|m| m.id == msg2.id));
            }
            _ => panic!("Expected Messages response"),
        }
    }

    #[tokio::test]
    async fn test_get_ancestors_handler() {
        let config = SyncConfig::default();
        let storage = create_test_storage();

        // Create a chain: genesis -> msg1 -> msg2
        let genesis = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(1, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![0],
        );
        storage.store_message(&genesis).await.unwrap();

        let msg1 = AdicMessage::new(
            vec![genesis.id.clone()],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(2, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([1; 32]),
            vec![1],
        );
        storage.store_message(&msg1).await.unwrap();

        let msg2 = AdicMessage::new(
            vec![msg1.id.clone()],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(3, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([2; 32]),
            vec![2],
        );
        storage.store_message(&msg2).await.unwrap();

        let protocol = SyncProtocol::new(config, storage);

        // Test GetAncestors request
        let request = SyncRequest::GetAncestors {
            message_id: msg2.id.clone(),
            depth: 5,
        };
        let response = protocol.handle_request(request).await;

        match response {
            SyncResponse::Ancestors(ancestors) => {
                assert!(ancestors.len() >= 1); // Should include at least msg1
                assert!(ancestors.iter().any(|m| m.id == msg1.id));
            }
            _ => panic!("Expected Ancestors response"),
        }
    }

    #[tokio::test]
    async fn test_checkpoint_handler() {
        let config = SyncConfig::default();
        let storage = create_test_storage();

        // Create and store a checkpoint
        let checkpoint_msg = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(100, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([99; 32]),
            vec![99],
        );
        storage.store_message(&checkpoint_msg).await.unwrap();
        storage
            .store_checkpoint(1000, &checkpoint_msg.id)
            .await
            .unwrap();

        let protocol = SyncProtocol::new(config, storage);

        // Test GetCheckpoint request
        let request = SyncRequest::GetCheckpoint(1000);
        let response = protocol.handle_request(request).await;

        match response {
            SyncResponse::Checkpoint {
                height,
                root,
                messages,
            } => {
                assert_eq!(height, 1000);
                assert_eq!(root, checkpoint_msg.id);
                assert!(!messages.is_empty());
            }
            SyncResponse::Error(e) => panic!("Expected Checkpoint response, got error: {}", e),
            _ => panic!("Expected Checkpoint response"),
        }
    }

    #[tokio::test]
    async fn test_merkle_proof_handler() {
        let config = SyncConfig::default();
        let storage = create_test_storage();

        // Create a message
        let msg = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![AxisPhi::new(0, QpDigits::from_u64(42, 3, 5))]),
            AdicMeta::new(Utc::now()),
            PublicKey::from_bytes([42; 32]),
            vec![42],
        );
        storage.store_message(&msg).await.unwrap();

        let protocol = SyncProtocol::new(config, storage);

        // Test GetProof request
        let request = SyncRequest::GetProof(msg.id.clone());
        let response = protocol.handle_request(request).await;

        match response {
            SyncResponse::Proof {
                message_id,
                path,
                root,
            } => {
                assert_eq!(message_id, msg.id);
                assert_eq!(root.len(), 32);
                // Path might be empty for a genesis message
                assert!(path.len() <= 20); // Max depth is 20
            }
            SyncResponse::Error(e) => panic!("Expected Proof response, got error: {}", e),
            _ => panic!("Expected Proof response"),
        }
    }
}
