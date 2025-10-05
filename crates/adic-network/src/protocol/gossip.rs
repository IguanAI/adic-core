use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use libp2p::{
    gossipsub::{
        Behaviour as Gossipsub, ConfigBuilder, Event as GossipsubEvent, IdentTopic as Topic,
        MessageAuthenticity, TopicHash, ValidationMode,
    },
    identity::Keypair,
    PeerId,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

use crate::protocol::AxisOverlay;
use adic_types::{AdicError, AdicMessage, AxisPhi, MessageId, Result};

#[derive(Debug, Clone)]
pub struct GossipConfig {
    pub mesh_n: usize,
    pub mesh_n_low: usize,
    pub mesh_n_high: usize,
    pub mesh_outbound_min: usize,
    pub gossip_lazy: usize,
    pub gossip_factor: f64,
    pub heartbeat_interval: Duration,
    pub fanout_ttl: Duration,
    pub max_transmit_size: usize,
    pub duplicate_cache_time: Duration,
    pub validation_mode: ValidationMode,
}

impl Default for GossipConfig {
    fn default() -> Self {
        Self {
            mesh_n: 8,
            mesh_n_low: 6,
            mesh_n_high: 12,
            mesh_outbound_min: 4,
            gossip_lazy: 6,
            gossip_factor: 0.25,
            heartbeat_interval: Duration::from_secs(1),
            fanout_ttl: Duration::from_secs(60),
            max_transmit_size: 65536,
            duplicate_cache_time: Duration::from_secs(120),
            validation_mode: ValidationMode::Strict,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum GossipMessage {
    Message(AdicMessage),
    Tips(Vec<MessageId>),
    FinalityUpdate(MessageId, u64), // message_id, k-core value
    ConflictAnnouncement(String, Vec<MessageId>), // conflict_id, involved messages
    PeerAnnouncement(String, Vec<String>), // peer_id as string, multiaddrs
}

pub struct GossipProtocol {
    gossipsub: Arc<RwLock<Gossipsub>>,
    config: GossipConfig,
    topics: Arc<RwLock<HashMap<String, Topic>>>,
    message_cache: Arc<RwLock<MessageCache>>,
    validation_queue: Arc<RwLock<ValidationQueue>>,
    event_sender: mpsc::UnboundedSender<GossipEvent>,
    event_receiver: Arc<RwLock<mpsc::UnboundedReceiver<GossipEvent>>>,
    /// Axis-aware overlay for p-adic gossip routing per ADIC-DAG paper §4
    axis_overlay: Arc<AxisOverlay>,
}

#[derive(Debug, Clone)]
pub enum GossipEvent {
    MessageReceived(Box<GossipMessage>, PeerId),
    MessageValidated(MessageId, bool),
    PeerSubscribed(PeerId, TopicHash),
    PeerUnsubscribed(PeerId, TopicHash),
}

struct MessageCache {
    messages: HashMap<MessageId, (AdicMessage, Instant)>,
    max_size: usize,
    ttl: Duration,
}

impl MessageCache {
    fn new(max_size: usize, ttl: Duration) -> Self {
        Self {
            messages: HashMap::new(),
            max_size,
            ttl,
        }
    }

    fn insert(&mut self, message: AdicMessage) {
        let now = Instant::now();

        // Clean old entries
        self.messages
            .retain(|_, (_, timestamp)| now.duration_since(*timestamp) < self.ttl);

        // Remove oldest if at capacity
        if self.messages.len() >= self.max_size {
            if let Some(oldest_id) = self
                .messages
                .iter()
                .min_by_key(|(_, (_, ts))| *ts)
                .map(|(id, _)| *id)
            {
                self.messages.remove(&oldest_id);
            }
        }

        self.messages.insert(message.id, (message, now));
    }

    fn contains(&self, id: &MessageId) -> bool {
        if let Some((_, timestamp)) = self.messages.get(id) {
            Instant::now().duration_since(*timestamp) < self.ttl
        } else {
            false
        }
    }

    #[allow(dead_code)]
    fn get(&self, id: &MessageId) -> Option<&AdicMessage> {
        self.messages.get(id).map(|(msg, _)| msg)
    }
}

struct ValidationQueue {
    pending: VecDeque<(MessageId, AdicMessage, PeerId)>,
    max_size: usize,
}

impl ValidationQueue {
    fn new(max_size: usize) -> Self {
        Self {
            pending: VecDeque::new(),
            max_size,
        }
    }

    fn push(&mut self, id: MessageId, message: AdicMessage, source: PeerId) -> bool {
        if self.pending.len() >= self.max_size {
            return false;
        }
        self.pending.push_back((id, message, source));
        true
    }

    fn pop(&mut self) -> Option<(MessageId, AdicMessage, PeerId)> {
        self.pending.pop_front()
    }

    #[allow(dead_code)]
    fn len(&self) -> usize {
        self.pending.len()
    }
}

impl GossipProtocol {
    pub fn new(keypair: &Keypair, config: GossipConfig) -> Result<Self> {
        let gossipsub_config = ConfigBuilder::default()
            .mesh_n(config.mesh_n)
            .mesh_n_low(config.mesh_n_low)
            .mesh_n_high(config.mesh_n_high)
            .mesh_outbound_min(config.mesh_outbound_min)
            .gossip_lazy(config.gossip_lazy)
            .gossip_factor(config.gossip_factor)
            .heartbeat_interval(config.heartbeat_interval)
            .fanout_ttl(config.fanout_ttl)
            .max_transmit_size(config.max_transmit_size)
            .duplicate_cache_time(config.duplicate_cache_time)
            .validation_mode(config.validation_mode.clone())
            .build()
            .map_err(|e| AdicError::Network(format!("Failed to build gossipsub config: {}", e)))?;

        let gossipsub = Gossipsub::new(
            MessageAuthenticity::Signed(keypair.clone()),
            gossipsub_config,
        )
        .map_err(|e| AdicError::Network(format!("Failed to create gossipsub: {}", e)))?;

        let (event_sender, event_receiver) = mpsc::unbounded_channel();

        // Initialize axis overlay with p=3, ball_radius=3 per ADIC-DAG paper
        let axis_overlay = Arc::new(AxisOverlay::new(3, 3));

        Ok(Self {
            gossipsub: Arc::new(RwLock::new(gossipsub)),
            config,
            topics: Arc::new(RwLock::new(HashMap::new())),
            message_cache: Arc::new(RwLock::new(MessageCache::new(
                10000,
                Duration::from_secs(300),
            ))),
            validation_queue: Arc::new(RwLock::new(ValidationQueue::new(1000))),
            event_sender,
            event_receiver: Arc::new(RwLock::new(event_receiver)),
            axis_overlay,
        })
    }

    pub async fn subscribe(&self, topic_name: &str) -> Result<()> {
        debug!(
            "GossipProtocol::subscribe - Starting for topic {}",
            topic_name
        );
        let topic = Topic::new(topic_name);

        debug!(
            "GossipProtocol::subscribe - Acquiring gossipsub write lock for {}",
            topic_name
        );
        let mut gossipsub = self.gossipsub.write().await;
        debug!(
            "GossipProtocol::subscribe - Got gossipsub write lock, subscribing to {}",
            topic_name
        );
        gossipsub.subscribe(&topic).map_err(|e| {
            AdicError::Network(format!(
                "Failed to subscribe to topic {}: {:?}",
                topic_name, e
            ))
        })?;
        debug!(
            "GossipProtocol::subscribe - Subscribed via gossipsub, now updating topics for {}",
            topic_name
        );

        drop(gossipsub); // Explicitly release the lock before acquiring the next one

        debug!(
            "GossipProtocol::subscribe - Acquiring topics write lock for {}",
            topic_name
        );
        let mut topics = self.topics.write().await;
        debug!(
            "GossipProtocol::subscribe - Got topics write lock for {}",
            topic_name
        );
        topics.insert(topic_name.to_string(), topic);
        debug!("GossipProtocol::subscribe - Inserted topic {}", topic_name);

        info!("Subscribed to topic: {}", topic_name);
        Ok(())
    }

    pub async fn unsubscribe(&self, topic_name: &str) -> Result<()> {
        let mut topics = self.topics.write().await;
        if let Some(topic) = topics.remove(topic_name) {
            let mut gossipsub = self.gossipsub.write().await;
            if !gossipsub.unsubscribe(&topic) {
                return Err(AdicError::Network(format!(
                    "Failed to unsubscribe from topic {}",
                    topic_name
                )));
            }

            info!("Unsubscribed from topic: {}", topic_name);
        }
        Ok(())
    }

    pub async fn publish_message(&self, message: GossipMessage, topic_name: &str) -> Result<()> {
        let topics = self.topics.read().await;
        let topic = topics
            .get(topic_name)
            .ok_or_else(|| AdicError::Network(format!("Topic {} not found", topic_name)))?;

        let data = serde_json::to_vec(&message)
            .map_err(|e| AdicError::Serialization(format!("Failed to serialize message: {}", e)))?;

        let mut gossipsub = self.gossipsub.write().await;
        match gossipsub.publish(topic.clone(), data) {
            Ok(_) => {
                debug!("Published message to topic: {}", topic_name);
            }
            Err(e) => {
                // Note: In libp2p 0.56+, insufficient peers is no longer a separate error variant
                debug!("Failed to publish message to {}: {:?}", topic_name, e);
                // Don't fail hard - we might be the first node or operating in standalone mode
            }
        }

        Ok(())
    }

    pub async fn broadcast_message(&self, message: AdicMessage) -> Result<()> {
        // Check message size against config limits
        let message_size = serde_json::to_vec(&message)
            .map_err(|e| AdicError::Serialization(format!("Failed to serialize: {}", e)))?
            .len();

        if message_size > self.config.max_transmit_size {
            return Err(AdicError::Network(format!(
                "Message size {} exceeds max size {}",
                message_size, self.config.max_transmit_size
            )));
        }

        // Cache the message
        {
            let mut cache = self.message_cache.write().await;
            cache.insert(message.clone());
        }

        // Broadcast to main message topic
        self.publish_message(GossipMessage::Message(message.clone()), "adic/messages")
            .await?;

        // Also broadcast to axis-specific topics if applicable
        for axis_key in message.meta.axes.keys() {
            let topic = format!("adic/axis/{}", axis_key);
            self.publish_message(GossipMessage::Message(message.clone()), &topic)
                .await?;
        }

        Ok(())
    }

    pub async fn announce_tips(&self, tips: Vec<MessageId>) -> Result<()> {
        self.publish_message(GossipMessage::Tips(tips), "adic/tips")
            .await
    }

    pub async fn announce_finality(&self, message_id: MessageId, k_core: u64) -> Result<()> {
        self.publish_message(
            GossipMessage::FinalityUpdate(message_id, k_core),
            "adic/finality",
        )
        .await
    }

    pub async fn announce_conflict(
        &self,
        conflict_id: String,
        messages: Vec<MessageId>,
    ) -> Result<()> {
        self.publish_message(
            GossipMessage::ConflictAnnouncement(conflict_id, messages),
            "adic/conflicts",
        )
        .await
    }

    pub async fn handle_gossip_event(&self, event: GossipsubEvent) {
        match event {
            GossipsubEvent::Message {
                propagation_source,
                message_id: _,
                message,
            } => {
                if let Ok(gossip_msg) = serde_json::from_slice::<GossipMessage>(&message.data) {
                    self.handle_received_message(gossip_msg, propagation_source)
                        .await;
                }
            }
            GossipsubEvent::Subscribed { peer_id, topic } => {
                self.event_sender
                    .send(GossipEvent::PeerSubscribed(peer_id, topic))
                    .ok();
            }
            GossipsubEvent::Unsubscribed { peer_id, topic } => {
                self.event_sender
                    .send(GossipEvent::PeerUnsubscribed(peer_id, topic))
                    .ok();
            }
            _ => {}
        }
    }

    async fn handle_received_message(&self, message: GossipMessage, source: PeerId) {
        match message {
            GossipMessage::Message(adic_msg) => {
                // Check cache for duplicates
                let mut cache = self.message_cache.write().await;
                if !cache.contains(&adic_msg.id) {
                    cache.insert(adic_msg.clone());

                    // Queue for validation
                    let mut queue = self.validation_queue.write().await;
                    if queue.push(adic_msg.id, adic_msg.clone(), source) {
                        debug!("Queued message {} for validation", adic_msg.id);
                    } else {
                        warn!("Validation queue full, dropping message {}", adic_msg.id);
                    }
                }
            }
            _ => {
                self.event_sender
                    .send(GossipEvent::MessageReceived(Box::new(message), source))
                    .ok();
            }
        }
    }

    pub async fn process_validation_queue<V>(&self, validator: V)
    where
        V: Fn(&AdicMessage) -> bool,
    {
        let mut queue = self.validation_queue.write().await;
        let mut validated = Vec::new();

        while let Some((id, message, source)) = queue.pop() {
            let is_valid = validator(&message);
            validated.push((id, is_valid));

            if is_valid {
                self.event_sender
                    .send(GossipEvent::MessageReceived(
                        Box::new(GossipMessage::Message(message)),
                        source,
                    ))
                    .ok();
            }
        }

        for (id, is_valid) in validated {
            self.event_sender
                .send(GossipEvent::MessageValidated(id, is_valid))
                .ok();
        }
    }

    pub fn event_stream(&self) -> Arc<RwLock<mpsc::UnboundedReceiver<GossipEvent>>> {
        self.event_receiver.clone()
    }

    pub async fn peers_in_topic(&self, topic_name: &str) -> Vec<PeerId> {
        let topics = self.topics.read().await;
        if let Some(topic) = topics.get(topic_name) {
            let gossipsub = self.gossipsub.read().await;
            gossipsub.mesh_peers(&topic.hash()).cloned().collect()
        } else {
            Vec::new()
        }
    }

    pub async fn all_peers(&self) -> HashSet<PeerId> {
        let gossipsub = self.gossipsub.read().await;
        gossipsub.all_peers().map(|(peer_id, _)| *peer_id).collect()
    }

    /// Update peer features in the axis overlay
    /// Should be called when peer metadata is received
    pub async fn update_peer_features(
        &self,
        peer_id: PeerId,
        features: Vec<AxisPhi>,
    ) -> Result<()> {
        self.axis_overlay.assign_peer(peer_id, features).await?;
        info!(peer_id = %peer_id, "Updated peer features in axis overlay");
        Ok(())
    }

    /// Remove peer from axis overlay when they disconnect
    pub async fn remove_peer_from_overlay(&self, peer_id: &PeerId) -> Result<()> {
        self.axis_overlay.remove_peer(peer_id).await?;
        info!(peer_id = %peer_id, "Removed peer from axis overlay");
        Ok(())
    }

    /// Get diverse peers for axis-aware gossip routing
    /// Uses p-adic proximity to select peers from diverse regions
    pub async fn get_diverse_peers(
        &self,
        num_peers: usize,
        target_features: &[AxisPhi],
    ) -> Vec<PeerId> {
        self.axis_overlay
            .select_diverse_peers(num_peers, target_features)
            .await
    }

    /// Get axis overlay statistics for debugging
    pub async fn overlay_stats(&self) -> crate::protocol::OverlayStats {
        self.axis_overlay.stats().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::{AdicFeatures, AdicMeta, PublicKey};
    use libp2p::identity::Keypair;

    #[tokio::test]
    async fn test_gossip_protocol_creation() {
        let keypair = Keypair::generate_ed25519();
        let config = GossipConfig::default();
        let protocol = GossipProtocol::new(&keypair, config).unwrap();

        assert_eq!(protocol.all_peers().await.len(), 0);
    }

    #[tokio::test]
    async fn test_topic_subscription() {
        let keypair = Keypair::generate_ed25519();
        let config = GossipConfig::default();
        let protocol = GossipProtocol::new(&keypair, config).unwrap();

        assert!(protocol.subscribe("test/topic").await.is_ok());
        assert!(protocol.unsubscribe("test/topic").await.is_ok());
    }

    #[tokio::test]
    async fn test_message_cache() {
        let mut cache = MessageCache::new(2, Duration::from_secs(60));

        let msg1 = AdicMessage::new(
            vec![],
            AdicFeatures::new(vec![]),
            AdicMeta::new(chrono::Utc::now()),
            PublicKey::from_bytes([0; 32]),
            vec![],
        );

        cache.insert(msg1.clone());
        assert!(cache.contains(&msg1.id));
    }
}
