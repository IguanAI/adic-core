use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{RwLock, mpsc, Semaphore};
use serde::{Serialize, Deserialize};
use tracing::{debug, warn, info};

use adic_types::{AdicMessage, MessageId, Result, AdicError};
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
    pub fn new(config: SyncConfig) -> Self {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        
        Self {
            config: config.clone(),
            state: Arc::new(RwLock::new(SyncState::new())),
            download_semaphore: Arc::new(Semaphore::new(config.parallel_downloads)),
            event_sender,
            event_receiver: Arc::new(RwLock::new(event_receiver)),
        }
    }

    pub async fn request_frontier(&self, peer: PeerId) -> Result<u64> {
        let mut state = self.state.write().await;
        let request_id = state.generate_request_id();
        
        state.pending_requests.insert(
            request_id,
            (SyncRequest::GetFrontier, peer, Instant::now())
        );
        
        debug!("Requesting frontier from peer {}", peer);
        Ok(request_id)
    }

    pub async fn request_messages(
        &self,
        peer: PeerId,
        message_ids: Vec<MessageId>,
    ) -> Result<u64> {
        let mut state = self.state.write().await;
        let request_id = state.generate_request_id();
        
        state.pending_requests.insert(
            request_id,
            (SyncRequest::GetMessages(message_ids.clone()), peer, Instant::now())
        );
        
        debug!("Requesting {} messages from peer {}", message_ids.len(), peer);
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
            (SyncRequest::GetRange { from, to, max_messages }, peer, Instant::now())
        );
        
        debug!("Requesting range from {} to {} from peer {}", from, to, peer);
        Ok(request_id)
    }

    pub async fn handle_request(&self, request: SyncRequest) -> SyncResponse {
        match request {
            SyncRequest::GetFrontier => {
                let state = self.state.read().await;
                SyncResponse::Frontier(state.frontier.iter().cloned().collect())
            }
            SyncRequest::GetMessages(_ids) => {
                // In a real implementation, fetch from storage
                SyncResponse::Messages(vec![])
            }
            SyncRequest::GetRange { from: _, to: _, max_messages: _ } => {
                // In a real implementation, fetch from storage
                SyncResponse::Range {
                    messages: vec![],
                    has_more: false,
                }
            }
            SyncRequest::GetAncestors { message_id: _, depth: _ } => {
                // In a real implementation, traverse DAG
                SyncResponse::Ancestors(vec![])
            }
            SyncRequest::GetDescendants { message_id: _, depth: _ } => {
                // In a real implementation, traverse DAG
                SyncResponse::Descendants(vec![])
            }
            SyncRequest::GetCheckpoint(_height) => {
                // In a real implementation, fetch checkpoint
                SyncResponse::Error("Checkpoint not found".to_string())
            }
            SyncRequest::GetProof(_message_id) => {
                // In a real implementation, generate Merkle proof
                SyncResponse::Error("Proof generation not implemented".to_string())
            }
        }
    }

    pub async fn handle_response(
        &self,
        request_id: u64,
        response: SyncResponse,
    ) -> Result<()> {
        let mut state = self.state.write().await;
        
        if let Some((request, peer, _)) = state.pending_requests.remove(&request_id) {
            match response {
                SyncResponse::Frontier(frontier) => {
                    state.frontier = frontier.iter().cloned().collect();
                    self.event_sender.send(SyncEvent::FrontierUpdated(frontier)).ok();
                }
                SyncResponse::Messages(messages) => {
                    self.event_sender.send(SyncEvent::MessagesReceived(messages)).ok();
                }
                SyncResponse::Range { messages, has_more } => {
                    self.event_sender.send(SyncEvent::MessagesReceived(messages)).ok();
                    if has_more {
                        // Request next batch
                        debug!("More messages available, requesting next batch");
                    }
                }
                SyncResponse::Error(err) => {
                    warn!("Sync request {} failed: {}", request_id, err);
                    self.handle_retry(&mut state, request_id, request, peer).await?;
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
            state.pending_requests.insert(
                request_id,
                (request, peer, Instant::now())
            );
            debug!("Retrying request {} (attempt {})", request_id, current_retry_count);
        } else {
            state.retry_count.remove(&request_id);
            self.event_sender.send(SyncEvent::SyncFailed(
                peer,
                format!("Request {} failed after {} attempts", request_id, current_retry_count)
            )).ok();
        }
        
        Ok(())
    }

    pub async fn start_incremental_sync(&self, peer: PeerId) -> Result<()> {
        let permit = self.download_semaphore.acquire().await
            .map_err(|e| AdicError::Network(format!("Failed to acquire semaphore: {}", e)))?;
        
        {
            let mut state = self.state.write().await;
            state.syncing_from.insert(peer, Instant::now());
        }
        
        self.event_sender.send(SyncEvent::SyncStarted(peer)).ok();
        
        // Request frontier
        let _frontier_request_id = self.request_frontier(peer).await?;
        
        // Wait for frontier response
        tokio::time::sleep(self.config.request_timeout).await;
        
        // In a real implementation, process frontier and request missing messages
        
        drop(permit);
        
        self.event_sender.send(SyncEvent::SyncCompleted(peer)).ok();
        
        {
            let mut state = self.state.write().await;
            state.syncing_from.remove(&peer);
        }
        
        Ok(())
    }

    pub async fn start_checkpoint_sync(
        &self,
        peer: PeerId,
        checkpoint_height: u64,
    ) -> Result<()> {
        let mut state = self.state.write().await;
        let request_id = state.generate_request_id();
        
        state.pending_requests.insert(
            request_id,
            (SyncRequest::GetCheckpoint(checkpoint_height), peer, Instant::now())
        );
        
        info!("Starting checkpoint sync from height {} with peer {}", checkpoint_height, peer);
        Ok(())
    }

    pub async fn cleanup_stale_requests(&self) {
        let mut state = self.state.write().await;
        let now = Instant::now();
        let timeout = self.config.request_timeout;
        
        let stale_requests: Vec<u64> = state.pending_requests
            .iter()
            .filter(|(_, (_, _, timestamp))| now.duration_since(*timestamp) > timeout)
            .map(|(id, _)| *id)
            .collect();
        
        for request_id in stale_requests {
            if let Some((request, peer, _)) = state.pending_requests.remove(&request_id) {
                warn!("Request {} timed out for peer {}", request_id, peer);
                self.handle_retry(&mut state, request_id, request, peer).await.ok();
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

    #[tokio::test]
    async fn test_sync_protocol_creation() {
        let config = SyncConfig::default();
        let protocol = SyncProtocol::new(config);
        
        assert!(!protocol.is_syncing().await);
        assert_eq!(protocol.syncing_peers().await.len(), 0);
    }

    #[tokio::test]
    async fn test_request_generation() {
        let config = SyncConfig::default();
        let protocol = SyncProtocol::new(config);
        let peer = PeerId::random();
        
        let request_id1 = protocol.request_frontier(peer).await.unwrap();
        let request_id2 = protocol.request_frontier(peer).await.unwrap();
        
        assert_ne!(request_id1, request_id2);
    }
}