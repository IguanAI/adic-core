use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, RwLock, Semaphore};
use tokio::time::interval;
use tracing::{debug, info};

use adic_types::{AdicError, AdicMessage, MessageId, Result};
// Import verify_signature when available
use libp2p::PeerId;

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub batch_size: usize,
    pub validation_workers: usize,
    pub max_queue_size: usize,
    pub priority_queue_size: usize,
    pub rate_limit_per_peer: usize,
    pub rate_limit_window: Duration,
    pub compression_threshold: usize,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            batch_size: 100,
            validation_workers: 4,
            max_queue_size: 10000,
            priority_queue_size: 1000,
            rate_limit_per_peer: 100,
            rate_limit_window: Duration::from_secs(1),
            compression_threshold: 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessagePriority {
    Critical = 0, // Finality messages
    High = 1,     // Conflict messages
    Normal = 2,   // Regular messages
    Low = 3,      // Background sync
}

#[derive(Debug, Clone)]
pub struct QueuedMessage {
    pub message: AdicMessage,
    pub priority: MessagePriority,
    pub source: PeerId,
    pub received_at: Instant,
    pub attempts: usize,
}

impl QueuedMessage {
    fn new(message: AdicMessage, priority: MessagePriority, source: PeerId) -> Self {
        Self {
            message,
            priority,
            source,
            received_at: Instant::now(),
            attempts: 0,
        }
    }
}

pub struct RateLimiter {
    limits: Arc<RwLock<HashMap<PeerId, VecDeque<Instant>>>>,
    max_per_window: usize,
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_per_window: usize, window: Duration) -> Self {
        Self {
            limits: Arc::new(RwLock::new(HashMap::new())),
            max_per_window,
            window,
        }
    }

    pub async fn check_rate_limit(&self, peer: &PeerId) -> bool {
        let mut limits = self.limits.write().await;
        let now = Instant::now();

        let timestamps = limits.entry(*peer).or_insert_with(VecDeque::new);

        // Remove old timestamps outside the window
        while let Some(&front) = timestamps.front() {
            if now.duration_since(front) > self.window {
                timestamps.pop_front();
            } else {
                break;
            }
        }

        if timestamps.len() >= self.max_per_window {
            false
        } else {
            timestamps.push_back(now);
            true
        }
    }

    pub async fn cleanup_old_entries(&self) {
        let mut limits = self.limits.write().await;
        let now = Instant::now();

        limits.retain(|_, timestamps| {
            timestamps.retain(|&t| now.duration_since(t) <= self.window);
            !timestamps.is_empty()
        });
    }
}

pub struct MessagePipeline {
    config: PipelineConfig,
    // Separate queues per priority level for O(1) insertion instead of O(n log n) sorting
    critical_queue: Arc<RwLock<VecDeque<QueuedMessage>>>,
    high_queue: Arc<RwLock<VecDeque<QueuedMessage>>>,
    normal_queue: Arc<RwLock<VecDeque<QueuedMessage>>>,
    low_queue: Arc<RwLock<VecDeque<QueuedMessage>>>,
    rate_limiter: Arc<RateLimiter>,
    validation_semaphore: Arc<Semaphore>,
    stats: Arc<RwLock<PipelineStats>>,
    event_sender: mpsc::UnboundedSender<PipelineEvent>,
    event_receiver: Arc<RwLock<mpsc::UnboundedReceiver<PipelineEvent>>>,
}

#[derive(Debug, Clone, Default)]
pub struct PipelineStats {
    pub messages_received: u64,
    pub messages_validated: u64,
    pub messages_rejected: u64,
    pub messages_rate_limited: u64,
    pub average_validation_time_ms: f64,
    pub queue_depth: usize,
    pub priority_queue_depth: usize,
}

#[derive(Debug, Clone)]
pub enum PipelineEvent {
    MessageQueued(MessageId, MessagePriority),
    MessageValidated(MessageId),
    MessageRejected(MessageId, String),
    RateLimitExceeded(PeerId),
    QueueFull,
}

impl MessagePipeline {
    pub fn new(config: PipelineConfig) -> Self {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();

        Self {
            // Allocate priority queues - total capacity split among priority levels
            critical_queue: Arc::new(RwLock::new(VecDeque::with_capacity(config.priority_queue_size / 4))),
            high_queue: Arc::new(RwLock::new(VecDeque::with_capacity(config.priority_queue_size / 4))),
            normal_queue: Arc::new(RwLock::new(VecDeque::with_capacity(config.max_queue_size))),
            low_queue: Arc::new(RwLock::new(VecDeque::with_capacity(config.priority_queue_size / 2))),
            rate_limiter: Arc::new(RateLimiter::new(
                config.rate_limit_per_peer,
                config.rate_limit_window,
            )),
            validation_semaphore: Arc::new(Semaphore::new(config.validation_workers)),
            stats: Arc::new(RwLock::new(PipelineStats::default())),
            event_sender,
            event_receiver: Arc::new(RwLock::new(event_receiver)),
            config,
        }
    }

    pub async fn submit_message(&self, message: AdicMessage, source: PeerId) -> Result<()> {
        // Check rate limit
        if !self.rate_limiter.check_rate_limit(&source).await {
            let mut stats = self.stats.write().await;
            stats.messages_rate_limited += 1;

            self.event_sender
                .send(PipelineEvent::RateLimitExceeded(source))
                .ok();
            return Err(AdicError::Network(format!(
                "Rate limit exceeded for peer {}",
                source
            )));
        }

        // Determine priority
        let priority = self.determine_priority(&message);

        // Queue the message - no sorting needed, each priority has its own queue
        let queued = QueuedMessage::new(message.clone(), priority, source);

        match priority {
            MessagePriority::Critical => {
                let mut queue = self.critical_queue.write().await;
                if queue.len() >= self.config.priority_queue_size / 4 {
                    self.event_sender.send(PipelineEvent::QueueFull).ok();
                    return Err(AdicError::Network("Critical queue full".to_string()));
                }
                queue.push_back(queued);
            }
            MessagePriority::High => {
                let mut queue = self.high_queue.write().await;
                if queue.len() >= self.config.priority_queue_size / 4 {
                    self.event_sender.send(PipelineEvent::QueueFull).ok();
                    return Err(AdicError::Network("High priority queue full".to_string()));
                }
                queue.push_back(queued);
            }
            MessagePriority::Normal => {
                let mut queue = self.normal_queue.write().await;
                if queue.len() >= self.config.max_queue_size {
                    self.event_sender.send(PipelineEvent::QueueFull).ok();
                    return Err(AdicError::Network("Normal queue full".to_string()));
                }
                queue.push_back(queued);
            }
            MessagePriority::Low => {
                let mut queue = self.low_queue.write().await;
                if queue.len() >= self.config.priority_queue_size / 2 {
                    self.event_sender.send(PipelineEvent::QueueFull).ok();
                    return Err(AdicError::Network("Low priority queue full".to_string()));
                }
                queue.push_back(queued);
            }
        }

        let mut stats = self.stats.write().await;
        stats.messages_received += 1;
        stats.queue_depth = self.normal_queue.read().await.len();
        // Priority queue depth is now the sum of critical + high + low queues
        stats.priority_queue_depth =
            self.critical_queue.read().await.len() +
            self.high_queue.read().await.len() +
            self.low_queue.read().await.len();

        self.event_sender
            .send(PipelineEvent::MessageQueued(message.id, priority))
            .ok();

        Ok(())
    }

    fn determine_priority(&self, message: &AdicMessage) -> MessagePriority {
        // Note: Finality updates are sent via GossipMessage::FinalityUpdate,
        // not through AdicMessage axes. This pipeline prioritizes message validation.

        // Check if it has a value transfer - high priority for economic activity
        if message.has_value_transfer() {
            return MessagePriority::High;
        }

        // Check if it's a conflict message
        if !message.meta.conflict.is_none() {
            return MessagePriority::High;
        }

        // Check message size for background sync
        if message.data.len() > 10000 {
            return MessagePriority::Low;
        }

        MessagePriority::Normal
    }

    pub async fn process_batch<V>(&self, validator: V) -> Vec<AdicMessage>
    where
        V: Fn(&AdicMessage) -> bool + Send + Sync + 'static,
    {
        let mut validated_messages = Vec::new();
        let mut batch = Vec::new();

        // Drain from queues in priority order: Critical -> High -> Normal -> Low
        // Process up to batch_size messages

        // Critical queue first (highest priority)
        {
            let mut critical_queue = self.critical_queue.write().await;
            while batch.len() < self.config.batch_size && !critical_queue.is_empty() {
                if let Some(queued) = critical_queue.pop_front() {
                    batch.push(queued);
                }
            }
        }

        // High priority queue
        {
            let mut high_queue = self.high_queue.write().await;
            while batch.len() < self.config.batch_size && !high_queue.is_empty() {
                if let Some(queued) = high_queue.pop_front() {
                    batch.push(queued);
                }
            }
        }

        // Normal queue
        {
            let mut normal_queue = self.normal_queue.write().await;
            while batch.len() < self.config.batch_size && !normal_queue.is_empty() {
                if let Some(queued) = normal_queue.pop_front() {
                    batch.push(queued);
                }
            }
        }

        // Low priority queue (only if batch not full)
        {
            let mut low_queue = self.low_queue.write().await;
            while batch.len() < self.config.batch_size && !low_queue.is_empty() {
                if let Some(queued) = low_queue.pop_front() {
                    batch.push(queued);
                }
            }
        }

        // Process batch in parallel
        let validator = Arc::new(validator);
        let mut handles = Vec::new();
        for queued in batch {
            let permit = self
                .validation_semaphore
                .clone()
                .acquire_owned()
                .await
                .unwrap();
            let validator = validator.clone();

            handles.push(tokio::spawn(async move {
                let start = Instant::now();
                let is_valid = validator(&queued.message);
                let validation_time = start.elapsed();
                drop(permit);
                (queued, is_valid, validation_time)
            }));
        }

        // Collect results
        for handle in handles {
            if let Ok((queued, is_valid, validation_time)) = handle.await {
                let mut stats = self.stats.write().await;

                if is_valid {
                    stats.messages_validated += 1;
                    validated_messages.push(queued.message.clone());

                    self.event_sender
                        .send(PipelineEvent::MessageValidated(queued.message.id))
                        .ok();
                } else {
                    stats.messages_rejected += 1;

                    self.event_sender
                        .send(PipelineEvent::MessageRejected(
                            queued.message.id,
                            "Validation failed".to_string(),
                        ))
                        .ok();
                }

                // Update average validation time
                let current_avg = stats.average_validation_time_ms;
                let new_time = validation_time.as_millis() as f64;
                stats.average_validation_time_ms =
                    (current_avg * (stats.messages_validated as f64 - 1.0) + new_time)
                        / stats.messages_validated as f64;
            }
        }

        validated_messages
    }

    pub async fn parallel_signature_verification(
        &self,
        messages: Vec<AdicMessage>,
    ) -> Vec<(MessageId, bool)> {
        let mut handles = Vec::new();

        for message in messages {
            let permit = self
                .validation_semaphore
                .clone()
                .acquire_owned()
                .await
                .unwrap();

            handles.push(tokio::spawn(async move {
                // Full cryptographic signature verification using Ed25519
                let crypto = adic_crypto::CryptoEngine::new();
                let is_valid = crypto
                    .verify_signature(&message, &message.proposer_pk, &message.signature)
                    .unwrap_or(false);

                if !is_valid {
                    debug!(
                        message_id = ?message.id,
                        "❌ Signature verification failed for message"
                    );
                }

                drop(permit);
                (message.id, is_valid)
            }));
        }

        let mut results = Vec::new();
        for handle in handles {
            if let Ok(result) = handle.await {
                results.push(result);
            }
        }

        results
    }

    pub fn compress_message(&self, message: &AdicMessage) -> Result<Vec<u8>> {
        let serialized = serde_json::to_vec(message)?;

        if serialized.len() > self.config.compression_threshold {
            let mut encoder = snap::raw::Encoder::new();
            let compressed = encoder
                .compress_vec(&serialized[..])
                .map_err(|e| AdicError::Serialization(format!("Compression failed: {}", e)))?;
            debug!(
                "Compressed message from {} to {} bytes",
                serialized.len(),
                compressed.len()
            );
            Ok(compressed)
        } else {
            Ok(serialized)
        }
    }

    pub fn decompress_message(&self, data: &[u8]) -> Result<AdicMessage> {
        // Try to deserialize directly first
        if let Ok(message) = serde_json::from_slice::<AdicMessage>(data) {
            return Ok(message);
        }

        // If that fails, try decompressing first
        let mut decoder = snap::raw::Decoder::new();
        let decompressed = decoder
            .decompress_vec(data)
            .map_err(|e| AdicError::Serialization(format!("Decompression failed: {}", e)))?;
        let message = serde_json::from_slice(&decompressed)?;
        Ok(message)
    }

    pub async fn get_stats(&self) -> PipelineStats {
        self.stats.read().await.clone()
    }

    pub async fn clear_queues(&self) {
        self.critical_queue.write().await.clear();
        self.high_queue.write().await.clear();
        self.normal_queue.write().await.clear();
        self.low_queue.write().await.clear();

        let mut stats = self.stats.write().await;
        stats.queue_depth = 0;
        stats.priority_queue_depth = 0;

        info!("Message pipeline queues cleared");
    }

    pub fn event_stream(&self) -> Arc<RwLock<mpsc::UnboundedReceiver<PipelineEvent>>> {
        self.event_receiver.clone()
    }

    pub async fn get_pending_messages(&self) -> Vec<AdicMessage> {
        // Get a batch of pending messages from all queues in priority order
        let mut messages = Vec::new();

        // Critical priority first
        {
            let mut critical_queue = self.critical_queue.write().await;
            while messages.len() < 25 && !critical_queue.is_empty() {
                if let Some(queued) = critical_queue.pop_front() {
                    messages.push(queued.message);
                }
            }
        }

        // High priority
        {
            let mut high_queue = self.high_queue.write().await;
            while messages.len() < 50 && !high_queue.is_empty() {
                if let Some(queued) = high_queue.pop_front() {
                    messages.push(queued.message);
                }
            }
        }

        // Normal priority
        {
            let mut normal_queue = self.normal_queue.write().await;
            while messages.len() < 100 && !normal_queue.is_empty() {
                if let Some(queued) = normal_queue.pop_front() {
                    messages.push(queued.message);
                }
            }
        }

        // Low priority (if space available)
        {
            let mut low_queue = self.low_queue.write().await;
            while messages.len() < 100 && !low_queue.is_empty() {
                if let Some(queued) = low_queue.pop_front() {
                    messages.push(queued.message);
                }
            }
        }

        messages
    }

    pub async fn start_cleanup_task(&self) {
        debug!("MessagePipeline::start_cleanup_task - Starting cleanup task");
        let rate_limiter = self.rate_limiter.clone();

        tokio::spawn(async move {
            debug!("MessagePipeline cleanup task - Started");
            let mut cleanup_interval = interval(Duration::from_secs(60));

            loop {
                cleanup_interval.tick().await;
                rate_limiter.cleanup_old_entries().await;
                debug!("Cleaned up rate limiter entries");
            }
        });
        debug!("MessagePipeline::start_cleanup_task - Cleanup task spawned");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::{AdicFeatures, AdicMeta, ConflictId, PublicKey};

    #[tokio::test]
    async fn test_rate_limiter() {
        let limiter = RateLimiter::new(3, Duration::from_secs(1));
        let peer = PeerId::random();

        assert!(limiter.check_rate_limit(&peer).await);
        assert!(limiter.check_rate_limit(&peer).await);
        assert!(limiter.check_rate_limit(&peer).await);
        assert!(!limiter.check_rate_limit(&peer).await);

        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(limiter.check_rate_limit(&peer).await);
    }

    #[tokio::test]
    async fn test_message_pipeline() {
        let config = PipelineConfig::default();
        let pipeline = MessagePipeline::new(config);

        let message = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![]),
            AdicMeta::new(chrono::Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![],
        );

        let peer = PeerId::random();
        assert!(pipeline.submit_message(message, peer).await.is_ok());

        let stats = pipeline.get_stats().await;
        assert_eq!(stats.messages_received, 1);
    }

    #[tokio::test]
    async fn test_priority_ordering() {
        let config = PipelineConfig::default();
        let pipeline = MessagePipeline::new(config);

        // Create a high-priority message with a conflict (per determine_priority logic)
        let high_priority_msg = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![]),
            AdicMeta::new(chrono::Utc::now()).with_conflict(ConflictId::new("test-conflict".to_string())),
            PublicKey::from_bytes([0; 32]),
            vec![],
        );

        // Create a normal priority message (no conflict, no transfer)
        let normal_msg = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![]),
            AdicMeta::new(chrono::Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![],
        );

        let peer = PeerId::random();
        pipeline.submit_message(normal_msg, peer).await.unwrap();
        pipeline.submit_message(high_priority_msg, peer).await.unwrap();

        let stats = pipeline.get_stats().await;
        assert_eq!(stats.messages_received, 2);
        assert_eq!(stats.priority_queue_depth, 1); // Conflict message goes to priority queue
        assert_eq!(stats.queue_depth, 1); // Normal message goes to normal queue
    }
}
