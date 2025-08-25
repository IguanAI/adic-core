use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{RwLock, mpsc, Semaphore};
use serde::{Serialize, Deserialize};
use tracing::{debug, info};

use adic_types::{Result, MessageId, AdicError};
use libp2p::PeerId;

#[derive(Debug, Clone)]
pub struct StreamConfig {
    pub chunk_size: usize,
    pub max_concurrent_streams: usize,
    pub stream_timeout: Duration,
    pub compression_threshold: usize,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            chunk_size: 65536, // 64KB
            max_concurrent_streams: 10,
            stream_timeout: Duration::from_secs(120),
            compression_threshold: 1024, // Compress if > 1KB
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamRequest {
    Snapshot {
        from_height: u64,
        to_height: u64,
    },
    BulkMessages {
        message_ids: Vec<MessageId>,
        include_proofs: bool,
    },
    StateChunk {
        chunk_id: u64,
        total_chunks: u64,
    },
}

#[derive(Debug, Clone)]
pub struct StreamTransfer {
    pub request: StreamRequest,
    pub peer: PeerId,
    pub total_bytes: usize,
    pub transferred_bytes: usize,
    pub chunks_sent: usize,
    pub chunks_received: usize,
    pub start_time: std::time::Instant,
}

impl StreamTransfer {
    fn new(request: StreamRequest, peer: PeerId) -> Self {
        Self {
            request,
            peer,
            total_bytes: 0,
            transferred_bytes: 0,
            chunks_sent: 0,
            chunks_received: 0,
            start_time: std::time::Instant::now(),
        }
    }

    pub fn progress(&self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.transferred_bytes as f64 / self.total_bytes as f64).min(1.0)
    }

    pub fn throughput_mbps(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed == 0.0 {
            return 0.0;
        }
        (self.transferred_bytes as f64 / elapsed) / (1024.0 * 1024.0)
    }
}

pub struct StreamProtocol {
    config: StreamConfig,
    active_transfers: Arc<RwLock<Vec<StreamTransfer>>>,
    stream_semaphore: Arc<Semaphore>,
    event_sender: mpsc::UnboundedSender<StreamEvent>,
    event_receiver: Arc<RwLock<mpsc::UnboundedReceiver<StreamEvent>>>,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    TransferStarted(PeerId, StreamRequest),
    TransferProgress(PeerId, f64),
    TransferCompleted(PeerId, usize), // total bytes
    TransferFailed(PeerId, String),
    ChunkReceived(PeerId, usize), // chunk size
}

impl StreamProtocol {
    pub fn new(config: StreamConfig) -> Self {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        
        Self {
            config: config.clone(),
            active_transfers: Arc::new(RwLock::new(Vec::new())),
            stream_semaphore: Arc::new(Semaphore::new(config.max_concurrent_streams)),
            event_sender,
            event_receiver: Arc::new(RwLock::new(event_receiver)),
        }
    }

    pub async fn send_snapshot(
        &self,
        peer: PeerId,
        from_height: u64,
        to_height: u64,
        data: Vec<u8>,
    ) -> Result<()> {
        let _permit = self.stream_semaphore.acquire().await
            .map_err(|e| AdicError::Network(format!("Failed to acquire semaphore: {}", e)))?;
        
        let request = StreamRequest::Snapshot { from_height, to_height };
        let mut transfer = StreamTransfer::new(request.clone(), peer);
        transfer.total_bytes = data.len();
        
        self.event_sender.send(StreamEvent::TransferStarted(peer, request)).ok();
        
        // Add to active transfers
        {
            let mut transfers = self.active_transfers.write().await;
            transfers.push(transfer.clone());
        }
        
        // Compress if needed
        let compressed_data = if data.len() > self.config.compression_threshold {
            self.compress_data(&data)?
        } else {
            data
        };
        
        // Send in chunks
        for chunk in compressed_data.chunks(self.config.chunk_size) {
            // In real implementation, send chunk via network
            transfer.chunks_sent += 1;
            transfer.transferred_bytes += chunk.len();
            
            self.event_sender.send(StreamEvent::TransferProgress(
                peer,
                transfer.progress()
            )).ok();
            
            // Simulate network delay
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        
        // Remove from active transfers
        {
            let mut transfers = self.active_transfers.write().await;
            transfers.retain(|t| t.peer != peer);
        }
        
        self.event_sender.send(StreamEvent::TransferCompleted(
            peer,
            transfer.transferred_bytes
        )).ok();
        
        info!("Snapshot sent to {} ({} bytes, {:.2} MB/s)", 
            peer, 
            transfer.transferred_bytes,
            transfer.throughput_mbps()
        );
        
        Ok(())
    }

    pub async fn receive_snapshot(
        &self,
        peer: PeerId,
        from_height: u64,
        to_height: u64,
    ) -> Result<Vec<u8>> {
        let _permit = self.stream_semaphore.acquire().await
            .map_err(|e| AdicError::Network(format!("Failed to acquire semaphore: {}", e)))?;
        
        let request = StreamRequest::Snapshot { from_height, to_height };
        let transfer = StreamTransfer::new(request.clone(), peer);
        
        self.event_sender.send(StreamEvent::TransferStarted(peer, request)).ok();
        
        // Add to active transfers
        {
            let mut transfers = self.active_transfers.write().await;
            transfers.push(transfer.clone());
        }
        
        // In real implementation, receive chunks from network
        // For now, simulate receiving compressed data
        let compressed_data = Vec::new();
        
        // Decompress the received data
        let received_data = if !compressed_data.is_empty() {
            self.decompress_data(&compressed_data)?
        } else {
            Vec::new()
        };
        
        // Remove from active transfers
        {
            let mut transfers = self.active_transfers.write().await;
            transfers.retain(|t| t.peer != peer);
        }
        
        self.event_sender.send(StreamEvent::TransferCompleted(
            peer,
            received_data.len()
        )).ok();
        
        Ok(received_data)
    }

    pub async fn stream_bulk_messages(
        &self,
        peer: PeerId,
        message_ids: Vec<MessageId>,
        include_proofs: bool,
    ) -> Result<()> {
        let _permit = self.stream_semaphore.acquire().await
            .map_err(|e| AdicError::Network(format!("Failed to acquire semaphore: {}", e)))?;
        
        let request = StreamRequest::BulkMessages { 
            message_ids: message_ids.clone(), 
            include_proofs 
        };
        
        let _transfer = StreamTransfer::new(request.clone(), peer);
        
        self.event_sender.send(StreamEvent::TransferStarted(peer, request)).ok();
        
        // In real implementation, stream messages
        
        info!("Streamed {} messages to {}", message_ids.len(), peer);
        
        Ok(())
    }

    fn compress_data(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut encoder = snap::raw::Encoder::new();
        let compressed = encoder
            .compress_vec(data)
            .map_err(|e| AdicError::Serialization(format!("Compression failed: {}", e)))?;

        debug!("Compressed {} bytes to {} bytes", data.len(), compressed.len());
        Ok(compressed)
    }

    fn decompress_data(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut decoder = snap::raw::Decoder::new();
        let decompressed = decoder
            .decompress_vec(data)
            .map_err(|e| AdicError::Serialization(format!("Decompression failed: {}", e)))?;

        debug!("Decompressed {} bytes to {} bytes", data.len(), decompressed.len());
        Ok(decompressed)
    }

    pub async fn active_transfer_count(&self) -> usize {
        let transfers = self.active_transfers.read().await;
        transfers.len()
    }

    pub async fn get_transfer_progress(&self, peer: &PeerId) -> Option<f64> {
        let transfers = self.active_transfers.read().await;
        transfers.iter()
            .find(|t| t.peer == *peer)
            .map(|t| t.progress())
    }

    pub async fn get_transfer_stats(&self, peer: &PeerId) -> Option<(f64, f64)> {
        let transfers = self.active_transfers.read().await;
        transfers.iter()
            .find(|t| t.peer == *peer)
            .map(|t| (t.progress(), t.throughput_mbps()))
    }

    pub async fn cancel_transfer(&self, peer: &PeerId) -> bool {
        let mut transfers = self.active_transfers.write().await;
        let initial_len = transfers.len();
        transfers.retain(|t| t.peer != *peer);
        
        if transfers.len() < initial_len {
            self.event_sender.send(StreamEvent::TransferFailed(
                *peer,
                "Transfer cancelled".to_string()
            )).ok();
            true
        } else {
            false
        }
    }

    pub fn event_stream(&self) -> Arc<RwLock<mpsc::UnboundedReceiver<StreamEvent>>> {
        self.event_receiver.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stream_protocol_creation() {
        let config = StreamConfig::default();
        let protocol = StreamProtocol::new(config);
        
        assert_eq!(protocol.active_transfer_count().await, 0);
    }

    #[tokio::test]
    async fn test_transfer_tracking() {
        let config = StreamConfig::default();
        let _protocol = StreamProtocol::new(config);
        let peer = PeerId::random();
        
        // Start a simulated transfer
        let request = StreamRequest::Snapshot { from_height: 0, to_height: 100 };
        let transfer = StreamTransfer::new(request, peer);
        
        assert_eq!(transfer.progress(), 0.0);
        assert!(transfer.throughput_mbps() >= 0.0);
    }

    #[tokio::test]
    async fn test_compression() {
        let config = StreamConfig::default();
        let protocol = StreamProtocol::new(config);
        
        let data = vec![0u8; 10000];
        let compressed = protocol.compress_data(&data).unwrap();
        assert!(compressed.len() < data.len());
        
        let decompressed = protocol.decompress_data(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }
}
