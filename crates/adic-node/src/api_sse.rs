//! Server-Sent Events (SSE) API for real-time event streaming
//!
//! This module provides SSE endpoints as an alternative to WebSocket
//! for clients that prefer HTTP-based streaming. SSE is simpler than
//! WebSocket but only supports server-to-client messages.

use crate::events::{EventBus, EventPriority, NodeEvent};
use crate::metrics::Metrics;
use axum::{
    extract::{Query, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
};
use futures_util::stream::{self, Stream};
use serde::Deserialize;
use std::collections::HashSet;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info};

/// Query parameters for SSE connection
#[derive(Debug, Deserialize)]
pub struct SseQuery {
    /// Filter by event types (comma-separated)
    /// Examples: "tips.update,message.new" or "all"
    #[serde(default)]
    pub events: Option<String>,

    /// Filter by priority: high, medium, low, or all (default: all)
    #[serde(default)]
    pub priority: Option<String>,

    /// Include heartbeat events (default: true)
    #[serde(default = "default_heartbeat")]
    pub heartbeat: bool,
}

fn default_heartbeat() -> bool {
    true
}

/// SSE connection state
#[derive(Clone)]
struct SseConnection {
    /// Event types this client is subscribed to ("all" means all events)
    subscribed_events: HashSet<String>,
    /// Priority levels this client wants
    subscribed_priorities: HashSet<EventPriority>,
}

/// Connection guard that decrements SSE connection count on drop
struct ConnectionGuard {
    metrics: Metrics,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.metrics.sse_connections.dec();
        debug!("SSE connection closed");
    }
}

impl SseConnection {
    fn new(events_filter: Option<String>, priority_filter: Option<String>) -> Self {
        let subscribed_events = if let Some(events) = events_filter {
            if events == "all" {
                let mut set = HashSet::new();
                set.insert("all".to_string());
                set
            } else {
                events.split(',').map(|s| s.trim().to_string()).collect()
            }
        } else {
            let mut set = HashSet::new();
            set.insert("all".to_string());
            set
        };

        let subscribed_priorities = if let Some(priority) = priority_filter {
            match priority.to_lowercase().as_str() {
                "high" => vec![EventPriority::High].into_iter().collect(),
                "medium" => vec![EventPriority::Medium].into_iter().collect(),
                "low" => vec![EventPriority::Low].into_iter().collect(),
                _ => vec![
                    EventPriority::High,
                    EventPriority::Medium,
                    EventPriority::Low,
                ]
                .into_iter()
                .collect(),
            }
        } else {
            vec![
                EventPriority::High,
                EventPriority::Medium,
                EventPriority::Low,
            ]
            .into_iter()
            .collect()
        };

        Self {
            subscribed_events,
            subscribed_priorities,
        }
    }

    fn should_send_event(&self, event: &NodeEvent) -> bool {
        // Check priority filter
        if !self.subscribed_priorities.contains(&event.priority()) {
            return false;
        }

        // Check event type filter
        if self.subscribed_events.contains("all") {
            return true;
        }

        self.subscribed_events.contains(event.event_type())
    }
}

/// SSE handler
pub async fn sse_handler(
    Query(query): Query<SseQuery>,
    State(event_bus): State<Arc<EventBus>>,
    State(metrics): State<Metrics>,
) -> impl IntoResponse {
    info!(
        events = ?query.events,
        priority = ?query.priority,
        heartbeat = query.heartbeat,
        "SSE connection request"
    );

    // Increment connection count
    metrics.sse_connections.inc();

    let stream = create_event_stream(event_bus, query, metrics);

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    )
}

/// Create event stream for SSE
fn create_event_stream(
    event_bus: Arc<EventBus>,
    query: SseQuery,
    metrics: Metrics,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let connection = SseConnection::new(query.events, query.priority);

    // Create connection guard to decrement counter when stream is dropped
    let _guard = ConnectionGuard {
        metrics: metrics.clone(),
    };

    // Subscribe to event channels
    let (high_rx, medium_rx, low_rx) = event_bus.subscribe_all();

    info!(
        subscribed_events = ?connection.subscribed_events,
        subscribed_priorities = ?connection.subscribed_priorities,
        "SSE connection established"
    );

    // Create merged stream from all priority channels
    let high_stream = tokio_stream::wrappers::BroadcastStream::new(high_rx);
    let medium_stream = tokio_stream::wrappers::BroadcastStream::new(medium_rx);
    let low_stream = tokio_stream::wrappers::BroadcastStream::new(low_rx);

    let merged = stream::select_all(vec![
        Box::new(high_stream) as Box<dyn Stream<Item = _> + Send + Unpin>,
        Box::new(medium_stream),
        Box::new(low_stream),
    ]);

    use futures_util::StreamExt;

    // Wrap the stream to keep the guard alive
    let filtered = merged
        .filter_map(|result| async move {
            match result {
                Ok(event) => Some(event),
                Err(e) => {
                    debug!(error = ?e, "Broadcast stream error");
                    None
                }
            }
        })
        .filter(move |event| futures_util::future::ready(connection.should_send_event(event)))
        .map(move |event| create_sse_event(event, metrics.clone()))
        .chain(stream::once(async move {
            // Keep guard alive until stream ends
            drop(_guard);
            Ok(Event::default())
        }));

    Box::pin(filtered) as Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>
}

/// Create SSE event from NodeEvent
fn create_sse_event(event: NodeEvent, metrics: Metrics) -> Result<Event, Infallible> {
    let event_type = event.event_type();
    let data = match serde_json::to_string(&event) {
        Ok(json) => json,
        Err(e) => {
            error!(error = %e, "Failed to serialize event");
            return Ok(Event::default().data("error"));
        }
    };

    // Increment metrics
    metrics.sse_messages_sent.inc();
    metrics.events_emitted_total.inc();

    Ok(Event::default().event(event_type).data(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_connection_filters() {
        let conn = SseConnection::new(
            Some("tips.update,message.new".to_string()),
            Some("high".to_string()),
        );

        assert_eq!(conn.subscribed_events.len(), 2);
        assert!(conn.subscribed_events.contains("tips.update"));
        assert!(conn.subscribed_events.contains("message.new"));
        assert_eq!(conn.subscribed_priorities.len(), 1);
        assert!(conn.subscribed_priorities.contains(&EventPriority::High));
    }

    #[test]
    fn test_sse_connection_all_events() {
        let conn = SseConnection::new(Some("all".to_string()), None);

        assert!(conn.subscribed_events.contains("all"));
        assert_eq!(conn.subscribed_priorities.len(), 3);
    }
}
