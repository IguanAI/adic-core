use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use prometheus::{
    register_counter, register_counter_vec, register_gauge, register_gauge_vec,
    register_histogram_vec, Counter, CounterVec, Encoder, Gauge, GaugeVec, HistogramVec,
    TextEncoder,
};
use tokio::sync::RwLock;
use tracing::debug;

use adic_types::MessageId;
use libp2p::PeerId;

pub struct NetworkMetrics {
    // Message metrics
    messages_sent: Counter,
    messages_received: Counter,
    messages_by_type: CounterVec,
    message_latency: HistogramVec,

    // Peer metrics
    peer_count: Gauge,
    connected_peers: Gauge,
    peer_bandwidth: GaugeVec,
    peer_latency: HistogramVec,

    // Network metrics
    bytes_sent: Counter,
    bytes_received: Counter,
    active_connections: Gauge,
    connection_duration: HistogramVec,

    // Protocol metrics
    gossip_messages: CounterVec,
    sync_progress: Gauge,
    consensus_proposals: Counter,

    // Custom tracking
    message_timestamps: Arc<RwLock<HashMap<MessageId, Instant>>>,
    peer_stats: Arc<RwLock<HashMap<PeerId, PeerStats>>>,
}

#[derive(Debug, Clone, Default)]
pub struct PeerStats {
    pub messages_sent: u64,
    pub messages_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub last_seen: Option<Instant>,
}

impl Default for NetworkMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkMetrics {
    pub fn new() -> Self {
        let messages_sent =
            register_counter!("adic_messages_sent_total", "Total number of messages sent").unwrap();

        let messages_received = register_counter!(
            "adic_messages_received_total",
            "Total number of messages received"
        )
        .unwrap();

        let messages_by_type =
            register_counter_vec!("adic_messages_by_type", "Messages by type", &["type"]).unwrap();

        let message_latency = register_histogram_vec!(
            "adic_message_latency_seconds",
            "Message propagation latency",
            &["type"]
        )
        .unwrap();

        let peer_count = register_gauge!("adic_peer_count", "Total number of known peers").unwrap();

        let connected_peers =
            register_gauge!("adic_connected_peers", "Number of connected peers").unwrap();

        let peer_bandwidth = register_gauge_vec!(
            "adic_peer_bandwidth_mbps",
            "Peer bandwidth in Mbps",
            &["peer", "direction"]
        )
        .unwrap();

        let peer_latency = register_histogram_vec!(
            "adic_peer_latency_ms",
            "Peer latency in milliseconds",
            &["peer"]
        )
        .unwrap();

        let bytes_sent = register_counter!("adic_bytes_sent_total", "Total bytes sent").unwrap();

        let bytes_received =
            register_counter!("adic_bytes_received_total", "Total bytes received").unwrap();

        let active_connections =
            register_gauge!("adic_active_connections", "Number of active connections").unwrap();

        let connection_duration = register_histogram_vec!(
            "adic_connection_duration_seconds",
            "Connection duration",
            &["peer"]
        )
        .unwrap();

        let gossip_messages = register_counter_vec!(
            "adic_gossip_messages",
            "Gossip protocol messages",
            &["topic", "action"]
        )
        .unwrap();

        let sync_progress =
            register_gauge!("adic_sync_progress", "Sync progress percentage").unwrap();

        let consensus_proposals = register_counter!(
            "adic_consensus_proposals_total",
            "Total consensus proposals"
        )
        .unwrap();

        Self {
            messages_sent,
            messages_received,
            messages_by_type,
            message_latency,
            peer_count,
            connected_peers,
            peer_bandwidth,
            peer_latency,
            bytes_sent,
            bytes_received,
            active_connections,
            connection_duration,
            gossip_messages,
            sync_progress,
            consensus_proposals,
            message_timestamps: Arc::new(RwLock::new(HashMap::new())),
            peer_stats: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn record_message_sent(&self, message_id: &MessageId) {
        self.messages_sent.inc();

        let mut timestamps = self.message_timestamps.write().await;
        timestamps.insert(*message_id, Instant::now());

        // Cleanup old timestamps
        if timestamps.len() > 10000 {
            timestamps.clear();
        }
    }

    pub async fn record_message_received(&self, message_id: &MessageId, message_type: &str) {
        self.messages_received.inc();
        self.messages_by_type
            .with_label_values(&[message_type])
            .inc();

        // Calculate latency if we sent this message
        let timestamps = self.message_timestamps.read().await;
        if let Some(sent_time) = timestamps.get(message_id) {
            let latency = sent_time.elapsed().as_secs_f64();
            self.message_latency
                .with_label_values(&[message_type])
                .observe(latency);
        }
    }

    pub fn update_peer_count(&self, count: usize) {
        self.peer_count.set(count as f64);
    }

    pub fn update_connected_peers(&self, count: usize) {
        self.connected_peers.set(count as f64);
        self.active_connections.set(count as f64);
    }

    pub async fn record_peer_bandwidth(&self, peer: &PeerId, upload_mbps: f64, download_mbps: f64) {
        let peer_str = peer.to_string();
        self.peer_bandwidth
            .with_label_values(&[&peer_str, "upload"])
            .set(upload_mbps);
        self.peer_bandwidth
            .with_label_values(&[&peer_str, "download"])
            .set(download_mbps);

        let mut stats = self.peer_stats.write().await;
        let entry = stats.entry(*peer).or_default();
        entry.last_seen = Some(Instant::now());
    }

    pub fn record_peer_latency(&self, peer: &PeerId, latency_ms: f64) {
        let peer_str = peer.to_string();
        self.peer_latency
            .with_label_values(&[&peer_str])
            .observe(latency_ms);
    }

    pub fn record_bytes(&self, sent: u64, received: u64) {
        self.bytes_sent.inc_by(sent as f64);
        self.bytes_received.inc_by(received as f64);
    }

    pub fn record_connection_duration(&self, peer: &PeerId, duration: Duration) {
        let peer_str = peer.to_string();
        self.connection_duration
            .with_label_values(&[&peer_str])
            .observe(duration.as_secs_f64());
    }

    pub fn record_gossip_event(&self, topic: &str, action: &str) {
        self.gossip_messages
            .with_label_values(&[topic, action])
            .inc();
    }

    pub fn update_sync_progress(&self, progress: f64) {
        self.sync_progress.set(progress * 100.0);
    }

    pub fn record_consensus_proposal(&self) {
        self.consensus_proposals.inc();
    }

    pub async fn record_peer_message_sent(&self, peer: &PeerId, bytes: u64) {
        let mut stats = self.peer_stats.write().await;
        let peer_stat = stats.entry(*peer).or_default();
        peer_stat.messages_sent += 1;
        peer_stat.bytes_sent += bytes;
        peer_stat.last_seen = Some(Instant::now());
    }

    pub async fn record_peer_message_received(&self, peer: &PeerId, bytes: u64) {
        let mut stats = self.peer_stats.write().await;
        let peer_stat = stats.entry(*peer).or_default();
        peer_stat.messages_received += 1;
        peer_stat.bytes_received += bytes;
        peer_stat.last_seen = Some(Instant::now());
    }

    pub async fn get_peer_stats(&self, peer: &PeerId) -> Option<PeerStats> {
        let stats = self.peer_stats.read().await;
        stats.get(peer).cloned()
    }

    pub fn export_metrics(&self) -> String {
        let encoder = TextEncoder::new();
        let metric_families = prometheus::gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer).unwrap();
        String::from_utf8(buffer).unwrap()
    }

    pub async fn cleanup_old_data(&self) {
        let mut timestamps = self.message_timestamps.write().await;
        let now = Instant::now();
        timestamps.retain(|_, timestamp| now.duration_since(*timestamp) < Duration::from_secs(300));

        let mut stats = self.peer_stats.write().await;
        stats.retain(|_, peer_stats| {
            if let Some(last_seen) = peer_stats.last_seen {
                now.duration_since(last_seen) < Duration::from_secs(3600)
            } else {
                false
            }
        });

        debug!("Cleaned up old metrics data");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Metrics registration conflicts with other tests"]
    async fn test_message_metrics() {
        let metrics = NetworkMetrics::new();
        let message_id = MessageId::new(b"test");

        metrics.record_message_sent(&message_id).await;
        metrics
            .record_message_received(&message_id, "test_type")
            .await;

        // Metrics should be recorded
        let export = metrics.export_metrics();
        assert!(export.contains("adic_messages_sent_total"));
        assert!(export.contains("adic_messages_received_total"));
    }

    #[tokio::test]
    #[ignore = "Metrics registration conflicts with other tests"]
    async fn test_peer_metrics() {
        let metrics = NetworkMetrics::new();

        metrics.update_peer_count(10);
        metrics.update_connected_peers(5);

        let export = metrics.export_metrics();
        assert!(export.contains("adic_peer_count"));
        assert!(export.contains("adic_connected_peers"));
    }
}
