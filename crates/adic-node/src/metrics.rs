use prometheus::{Histogram, HistogramOpts, IntCounter, IntGauge, Registry, TextEncoder};
use std::sync::Arc;

#[allow(dead_code)]
#[derive(Clone)]
pub struct Metrics {
    registry: Arc<Registry>,
    pub messages_submitted: IntCounter,
    pub messages_processed: IntCounter,
    pub messages_failed: IntCounter,

    // MRW Metrics
    pub mrw_attempts_total: IntCounter,
    pub mrw_widens_total: IntCounter,
    pub mrw_selection_duration: Histogram,

    // Admissibility Metrics
    pub admissibility_checks_total: IntCounter,
    pub admissibility_s_failures: IntCounter,
    pub admissibility_c2_failures: IntCounter,
    pub admissibility_c3_failures: IntCounter,

    // Deposit Metrics
    pub deposits_escrowed: IntCounter,
    pub deposits_refunded: IntCounter,
    pub deposits_slashed: IntCounter,

    // Finality Metrics
    pub finalizations_total: IntCounter,
    pub kcore_size: IntGauge,
    pub finality_depth: Histogram,

    // Validation Metrics
    pub signature_verifications: IntCounter,
    pub signature_failures: IntCounter,

    // DAG State Metrics
    pub current_tips: IntGauge,
    pub dag_messages: IntGauge,

    // Event Streaming Metrics
    pub events_emitted_total: IntCounter,
    pub websocket_connections: IntGauge,
    pub sse_connections: IntGauge,
    pub websocket_messages_sent: IntCounter,
    pub sse_messages_sent: IntCounter,

    // Message Propagation Metrics
    pub messages_gossiped: IntCounter,
    pub messages_received_network: IntCounter,
    pub duplicate_messages: IntCounter,
    pub propagation_latency: Histogram,
    pub validation_latency: Histogram,
    pub gossip_failures: IntCounter,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Arc::new(Registry::new());

        let messages_submitted =
            IntCounter::new("adic_messages_submitted_total", "Total messages submitted").unwrap();
        let messages_processed =
            IntCounter::new("adic_messages_processed_total", "Total messages processed").unwrap();
        let messages_failed =
            IntCounter::new("adic_messages_failed_total", "Total messages failed").unwrap();

        let mrw_attempts_total =
            IntCounter::new("adic_mrw_attempts_total", "Total MRW attempts").unwrap();
        let mrw_widens_total =
            IntCounter::new("adic_mrw_widens_total", "Total MRW widens").unwrap();
        let mrw_selection_duration = Histogram::with_opts(HistogramOpts::new(
            "adic_mrw_selection_duration_seconds",
            "MRW selection duration",
        ))
        .unwrap();

        let admissibility_checks_total = IntCounter::new(
            "adic_admissibility_checks_total",
            "Total admissibility checks",
        )
        .unwrap();
        let admissibility_s_failures = IntCounter::new(
            "adic_admissibility_s_failures_total",
            "Admissibility S failures",
        )
        .unwrap();
        let admissibility_c2_failures = IntCounter::new(
            "adic_admissibility_c2_failures_total",
            "Admissibility C2 failures",
        )
        .unwrap();
        let admissibility_c3_failures = IntCounter::new(
            "adic_admissibility_c3_failures_total",
            "Admissibility C3 failures",
        )
        .unwrap();

        let deposits_escrowed =
            IntCounter::new("adic_deposits_escrowed_total", "Total deposits escrowed").unwrap();
        let deposits_refunded =
            IntCounter::new("adic_deposits_refunded_total", "Total deposits refunded").unwrap();
        let deposits_slashed =
            IntCounter::new("adic_deposits_slashed_total", "Total deposits slashed").unwrap();

        let finalizations_total =
            IntCounter::new("adic_finalizations_total", "Total finalizations").unwrap();
        let kcore_size = IntGauge::new("adic_kcore_size", "K-core size").unwrap();
        let finality_depth = Histogram::with_opts(HistogramOpts::new(
            "adic_finality_depth_seconds",
            "Finality depth",
        ))
        .unwrap();

        let signature_verifications = IntCounter::new(
            "adic_signature_verifications_total",
            "Total signature verifications",
        )
        .unwrap();
        let signature_failures =
            IntCounter::new("adic_signature_failures_total", "Total signature failures").unwrap();

        let current_tips = IntGauge::new("adic_current_tips", "Current tips").unwrap();
        let dag_messages = IntGauge::new("adic_dag_messages", "Total DAG messages").unwrap();

        let events_emitted_total = IntCounter::new(
            "adic_events_emitted_total",
            "Total events emitted to subscribers",
        )
        .unwrap();
        let websocket_connections =
            IntGauge::new("adic_websocket_connections", "Active WebSocket connections").unwrap();
        let sse_connections =
            IntGauge::new("adic_sse_connections", "Active SSE connections").unwrap();
        let websocket_messages_sent = IntCounter::new(
            "adic_websocket_messages_sent_total",
            "Total WebSocket messages sent",
        )
        .unwrap();
        let sse_messages_sent =
            IntCounter::new("adic_sse_messages_sent_total", "Total SSE messages sent").unwrap();

        let messages_gossiped = IntCounter::new(
            "adic_messages_gossiped_total",
            "Total messages broadcast via gossip",
        )
        .unwrap();
        let messages_received_network = IntCounter::new(
            "adic_messages_received_network_total",
            "Total messages received from network",
        )
        .unwrap();
        let duplicate_messages = IntCounter::new(
            "adic_duplicate_messages_total",
            "Total duplicate messages received",
        )
        .unwrap();
        let propagation_latency = Histogram::with_opts(HistogramOpts::new(
            "adic_message_propagation_latency_seconds",
            "Message propagation latency from submission to reception",
        ))
        .unwrap();
        let validation_latency = Histogram::with_opts(HistogramOpts::new(
            "adic_message_validation_latency_seconds",
            "Message validation latency",
        ))
        .unwrap();
        let gossip_failures = IntCounter::new(
            "adic_gossip_failures_total",
            "Total gossip/broadcast failures",
        )
        .unwrap();

        registry
            .register(Box::new(messages_submitted.clone()))
            .unwrap();
        registry
            .register(Box::new(messages_processed.clone()))
            .unwrap();
        registry
            .register(Box::new(messages_failed.clone()))
            .unwrap();
        registry
            .register(Box::new(mrw_attempts_total.clone()))
            .unwrap();
        registry
            .register(Box::new(mrw_widens_total.clone()))
            .unwrap();
        registry
            .register(Box::new(mrw_selection_duration.clone()))
            .unwrap();
        registry
            .register(Box::new(admissibility_checks_total.clone()))
            .unwrap();
        registry
            .register(Box::new(admissibility_s_failures.clone()))
            .unwrap();
        registry
            .register(Box::new(admissibility_c2_failures.clone()))
            .unwrap();
        registry
            .register(Box::new(admissibility_c3_failures.clone()))
            .unwrap();
        registry
            .register(Box::new(deposits_escrowed.clone()))
            .unwrap();
        registry
            .register(Box::new(deposits_refunded.clone()))
            .unwrap();
        registry
            .register(Box::new(deposits_slashed.clone()))
            .unwrap();
        registry
            .register(Box::new(finalizations_total.clone()))
            .unwrap();
        registry.register(Box::new(kcore_size.clone())).unwrap();
        registry.register(Box::new(finality_depth.clone())).unwrap();
        registry
            .register(Box::new(signature_verifications.clone()))
            .unwrap();
        registry
            .register(Box::new(signature_failures.clone()))
            .unwrap();
        registry.register(Box::new(current_tips.clone())).unwrap();
        registry.register(Box::new(dag_messages.clone())).unwrap();
        registry
            .register(Box::new(events_emitted_total.clone()))
            .unwrap();
        registry
            .register(Box::new(websocket_connections.clone()))
            .unwrap();
        registry
            .register(Box::new(sse_connections.clone()))
            .unwrap();
        registry
            .register(Box::new(websocket_messages_sent.clone()))
            .unwrap();
        registry
            .register(Box::new(sse_messages_sent.clone()))
            .unwrap();
        registry
            .register(Box::new(messages_gossiped.clone()))
            .unwrap();
        registry
            .register(Box::new(messages_received_network.clone()))
            .unwrap();
        registry
            .register(Box::new(duplicate_messages.clone()))
            .unwrap();
        registry
            .register(Box::new(propagation_latency.clone()))
            .unwrap();
        registry
            .register(Box::new(validation_latency.clone()))
            .unwrap();
        registry
            .register(Box::new(gossip_failures.clone()))
            .unwrap();

        Self {
            registry,
            messages_submitted,
            messages_processed,
            messages_failed,
            mrw_attempts_total,
            mrw_widens_total,
            mrw_selection_duration,
            admissibility_checks_total,
            admissibility_s_failures,
            admissibility_c2_failures,
            admissibility_c3_failures,
            deposits_escrowed,
            deposits_refunded,
            deposits_slashed,
            finalizations_total,
            kcore_size,
            finality_depth,
            signature_verifications,
            signature_failures,
            current_tips,
            dag_messages,
            events_emitted_total,
            websocket_connections,
            sse_connections,
            websocket_messages_sent,
            sse_messages_sent,
            messages_gossiped,
            messages_received_network,
            duplicate_messages,
            propagation_latency,
            validation_latency,
            gossip_failures,
        }
    }

    pub fn gather(&self) -> String {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        encoder
            .encode_to_string(&metric_families)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_gather() {
        let m = Metrics::new();
        m.messages_submitted.inc();
        m.finalizations_total.inc_by(2);
        let text = m.gather();
        assert!(text.contains("adic_messages_submitted_total"));
        assert!(text.contains("adic_finalizations_total"));
    }

    #[test]
    fn test_propagation_metrics() {
        let m = Metrics::new();

        // Test message propagation metrics
        m.messages_gossiped.inc();
        m.messages_received_network.inc_by(5);
        m.duplicate_messages.inc_by(2);
        m.gossip_failures.inc();

        let text = m.gather();

        // Verify all new propagation metrics are present
        assert!(text.contains("adic_messages_gossiped_total"));
        assert!(text.contains("adic_messages_received_network_total"));
        assert!(text.contains("adic_duplicate_messages_total"));
        assert!(text.contains("adic_gossip_failures_total"));
        assert!(text.contains("adic_message_propagation_latency_seconds"));
        assert!(text.contains("adic_message_validation_latency_seconds"));
    }

    #[test]
    fn test_propagation_latency_histogram() {
        let m = Metrics::new();

        // Record some latencies
        m.propagation_latency.observe(0.05); // 50ms
        m.propagation_latency.observe(0.1); // 100ms
        m.propagation_latency.observe(0.2); // 200ms

        m.validation_latency.observe(0.001); // 1ms
        m.validation_latency.observe(0.002); // 2ms

        let text = m.gather();

        // Verify histograms are tracked
        assert!(text.contains("adic_message_propagation_latency_seconds"));
        assert!(text.contains("adic_message_validation_latency_seconds"));

        // Histograms should have bucket and count metrics
        assert!(text.contains("_bucket"));
        assert!(text.contains("_count"));
        assert!(text.contains("_sum"));
    }

    #[test]
    fn test_all_metrics_registered() {
        let m = Metrics::new();
        let text = m.gather();

        // Verify all core metrics exist
        assert!(text.contains("adic_messages_submitted_total"));
        assert!(text.contains("adic_messages_processed_total"));
        assert!(text.contains("adic_messages_failed_total"));

        // MRW metrics
        assert!(text.contains("adic_mrw_attempts_total"));
        assert!(text.contains("adic_mrw_widens_total"));

        // Admissibility metrics
        assert!(text.contains("adic_admissibility_checks_total"));

        // Deposit metrics
        assert!(text.contains("adic_deposits_escrowed_total"));

        // Finality metrics
        assert!(text.contains("adic_finalizations_total"));
        assert!(text.contains("adic_kcore_size"));

        // Validation metrics
        assert!(text.contains("adic_signature_verifications_total"));

        // DAG state metrics
        assert!(text.contains("adic_current_tips"));
        assert!(text.contains("adic_dag_messages"));

        // Event streaming metrics
        assert!(text.contains("adic_websocket_connections"));
        assert!(text.contains("adic_sse_connections"));

        // Propagation metrics
        assert!(text.contains("adic_messages_gossiped_total"));
        assert!(text.contains("adic_duplicate_messages_total"));
    }
}
