//! WebSocket API for real-time event streaming
//!
//! This module provides WebSocket endpoints for clients to subscribe to
//! node events without polling. Clients can filter events by type and priority.

use crate::events::{EventBus, EventPriority, NodeEvent};
use crate::metrics::Metrics;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info, warn};

/// WebSocket connection pool manager
/// Manages connection limits and tracks active connections
#[derive(Clone)]
pub struct WsConnectionPool {
    /// Current number of active connections
    active_connections: Arc<AtomicUsize>,
    /// Maximum allowed connections
    max_connections: usize,
}

impl WsConnectionPool {
    pub fn new(max_connections: usize) -> Self {
        Self {
            active_connections: Arc::new(AtomicUsize::new(0)),
            max_connections,
        }
    }

    /// Try to acquire a connection slot
    /// Returns None if the pool is full
    pub fn try_acquire(&self) -> Option<WsConnectionGuard> {
        let current = self.active_connections.load(Ordering::SeqCst);

        if current >= self.max_connections {
            return None;
        }

        // Try to increment atomically
        let prev = self.active_connections.fetch_add(1, Ordering::SeqCst);

        if prev >= self.max_connections {
            // Race condition - another thread took the last slot
            self.active_connections.fetch_sub(1, Ordering::SeqCst);
            return None;
        }

        Some(WsConnectionGuard { pool: self.clone() })
    }

    /// Get current connection count
    pub fn active_count(&self) -> usize {
        self.active_connections.load(Ordering::SeqCst)
    }

    /// Get maximum connection limit
    pub fn max_count(&self) -> usize {
        self.max_connections
    }

    fn release(&self) {
        self.active_connections.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Default for WsConnectionPool {
    fn default() -> Self {
        Self::new(1000) // Default: 1000 concurrent connections
    }
}

/// RAII guard for WebSocket connection
/// Automatically releases the connection slot when dropped
pub struct WsConnectionGuard {
    pool: WsConnectionPool,
}

impl Drop for WsConnectionGuard {
    fn drop(&mut self) {
        self.pool.release();
    }
}

/// Query parameters for WebSocket connection
#[derive(Debug, Deserialize)]
pub struct WsQuery {
    /// Filter by event types (comma-separated)
    /// Examples: "tips.update,message.new" or "all"
    #[serde(default)]
    pub events: Option<String>,

    /// Filter by priority: high, medium, low, or all (default: all)
    #[serde(default)]
    pub priority: Option<String>,
}

/// Client-to-server messages
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    /// Subscribe to specific event types
    Subscribe { events: Vec<String> },
    /// Unsubscribe from event types
    Unsubscribe { events: Vec<String> },
    /// Ping to keep connection alive
    Ping,
}

/// Server-to-client messages
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    /// Event notification
    Event { event: NodeEvent },
    /// Subscription confirmation
    Subscribed { events: Vec<String> },
    /// Unsubscription confirmation
    Unsubscribed { events: Vec<String> },
    /// Pong response
    Pong,
    /// Error message
    Error { message: String },
}

/// Internal commands sent from client message handler to event handler
#[derive(Debug)]
enum SubscriptionCommand {
    Subscribe(Vec<String>),
    Unsubscribe(Vec<String>),
    Ping,
}

/// WebSocket connection state
struct WsConnection {
    /// Event types this client is subscribed to ("all" means all events)
    subscribed_events: HashSet<String>,
    /// Priority levels this client wants
    subscribed_priorities: HashSet<EventPriority>,
}

impl WsConnection {
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

    fn subscribe(&mut self, events: Vec<String>) {
        for event in events {
            self.subscribed_events.insert(event);
        }
        self.subscribed_events.remove("all");
    }

    fn unsubscribe(&mut self, events: Vec<String>) {
        for event in events {
            self.subscribed_events.remove(&event);
        }
        if self.subscribed_events.is_empty() {
            self.subscribed_events.insert("all".to_string());
        }
    }
}

/// WebSocket handler with connection pooling
pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(event_bus): State<Arc<EventBus>>,
    State(metrics): State<Metrics>,
    State(pool): State<WsConnectionPool>,
) -> Response {
    // Try to acquire a connection slot from the pool
    let guard = match pool.try_acquire() {
        Some(guard) => guard,
        None => {
            warn!(
                active = pool.active_count(),
                max = pool.max_count(),
                "WebSocket connection rejected - pool full"
            );
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Too many WebSocket connections",
            )
                .into_response();
        }
    };

    info!(
        events = ?query.events,
        priority = ?query.priority,
        active_connections = pool.active_count(),
        "WebSocket connection request accepted"
    );

    ws.on_upgrade(move |socket| handle_socket(socket, query, event_bus, metrics, guard))
}

/// Handle WebSocket connection
async fn handle_socket(
    socket: WebSocket,
    query: WsQuery,
    event_bus: Arc<EventBus>,
    metrics: Metrics,
    _guard: WsConnectionGuard, // Hold guard to maintain connection slot
) {
    // Increment connection count
    metrics.websocket_connections.inc();

    let (sender, receiver) = socket.split();
    let mut connection = WsConnection::new(query.events, query.priority);

    // Subscribe to event channels
    let (high_rx, medium_rx, low_rx) = event_bus.subscribe_all();

    // Create channel for subscription commands
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

    info!(
        subscribed_events = ?connection.subscribed_events,
        subscribed_priorities = ?connection.subscribed_priorities,
        "WebSocket connection established"
    );

    // Spawn task to handle incoming messages from client
    let client_handle = tokio::spawn(handle_client_messages(receiver, cmd_tx));

    // Handle outgoing events to client
    if let Err(e) = handle_events(
        sender,
        &mut connection,
        high_rx,
        medium_rx,
        low_rx,
        cmd_rx,
        metrics.clone(),
    )
    .await
    {
        error!(error = %e, "WebSocket error");
    }

    // Wait for client message handler to complete
    client_handle.abort();

    // Decrement connection count
    metrics.websocket_connections.dec();

    info!("WebSocket connection closed");
}

/// Handle client messages (subscribe/unsubscribe/ping)
async fn handle_client_messages(
    mut receiver: SplitStream<WebSocket>,
    cmd_tx: mpsc::UnboundedSender<SubscriptionCommand>,
) {
    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => {
                if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                    debug!(message = ?client_msg, "Received client message");

                    // Send command to event handler
                    let cmd = match client_msg {
                        ClientMessage::Subscribe { events } => {
                            SubscriptionCommand::Subscribe(events)
                        }
                        ClientMessage::Unsubscribe { events } => {
                            SubscriptionCommand::Unsubscribe(events)
                        }
                        ClientMessage::Ping => SubscriptionCommand::Ping,
                    };

                    if cmd_tx.send(cmd).is_err() {
                        debug!("Failed to send command - event handler closed");
                        break;
                    }
                } else {
                    warn!(text = %text, "Failed to parse client message");
                }
            }
            Message::Close(_) => {
                debug!("Client closed connection");
                break;
            }
            _ => {}
        }
    }
}

/// Handle event broadcasting to client
async fn handle_events(
    mut sender: SplitSink<WebSocket, Message>,
    connection: &mut WsConnection,
    mut high_rx: broadcast::Receiver<NodeEvent>,
    mut medium_rx: broadcast::Receiver<NodeEvent>,
    mut low_rx: broadcast::Receiver<NodeEvent>,
    mut cmd_rx: mpsc::UnboundedReceiver<SubscriptionCommand>,
    metrics: Metrics,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    loop {
        tokio::select! {
            // Handle subscription commands from client
            Some(cmd) = cmd_rx.recv() => {
                match cmd {
                    SubscriptionCommand::Subscribe(events) => {
                        debug!(events = ?events, "Processing subscribe command");
                        connection.subscribe(events.clone());

                        let response = ServerMessage::Subscribed { events };
                        let msg = serde_json::to_string(&response)?;
                        sender.send(Message::Text(msg)).await?;
                        metrics.websocket_messages_sent.inc();
                    }
                    SubscriptionCommand::Unsubscribe(events) => {
                        debug!(events = ?events, "Processing unsubscribe command");
                        connection.unsubscribe(events.clone());

                        let response = ServerMessage::Unsubscribed { events };
                        let msg = serde_json::to_string(&response)?;
                        sender.send(Message::Text(msg)).await?;
                        metrics.websocket_messages_sent.inc();
                    }
                    SubscriptionCommand::Ping => {
                        debug!("Processing ping command");
                        let response = ServerMessage::Pong;
                        let msg = serde_json::to_string(&response)?;
                        sender.send(Message::Text(msg)).await?;
                        metrics.websocket_messages_sent.inc();
                    }
                }
            }
            // High priority events
            Ok(event) = high_rx.recv() => {
                if connection.should_send_event(&event) {
                    send_event(&mut sender, event, &metrics).await?;
                }
            }
            // Medium priority events
            Ok(event) = medium_rx.recv() => {
                if connection.should_send_event(&event) {
                    send_event(&mut sender, event, &metrics).await?;
                }
            }
            // Low priority events
            Ok(event) = low_rx.recv() => {
                if connection.should_send_event(&event) {
                    send_event(&mut sender, event, &metrics).await?;
                }
            }
            // If all channels are closed, exit
            else => break,
        }
    }

    Ok(())
}

/// Send event to client
async fn send_event(
    sender: &mut SplitSink<WebSocket, Message>,
    event: NodeEvent,
    metrics: &Metrics,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server_msg = ServerMessage::Event { event };
    let json = serde_json::to_string(&server_msg)?;
    sender.send(Message::Text(json)).await?;

    // Increment metrics
    metrics.websocket_messages_sent.inc();
    metrics.events_emitted_total.inc();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_connection_filters() {
        let conn = WsConnection::new(
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
    fn test_ws_connection_all_events() {
        let conn = WsConnection::new(Some("all".to_string()), None);

        assert!(conn.subscribed_events.contains("all"));
        assert_eq!(conn.subscribed_priorities.len(), 3);
    }

    #[test]
    fn test_subscribe_unsubscribe() {
        let mut conn = WsConnection::new(Some("tips.update".to_string()), None);

        conn.subscribe(vec!["message.new".to_string()]);
        assert!(conn.subscribed_events.contains("tips.update"));
        assert!(conn.subscribed_events.contains("message.new"));
        assert!(!conn.subscribed_events.contains("all"));

        conn.unsubscribe(vec!["tips.update".to_string()]);
        assert!(!conn.subscribed_events.contains("tips.update"));
        assert!(conn.subscribed_events.contains("message.new"));
    }

    #[test]
    fn test_ws_connection_pool_basic() {
        let pool = WsConnectionPool::new(5);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.max_count(), 5);

        // Acquire connections
        let _guard1 = pool.try_acquire().expect("Should acquire first connection");
        assert_eq!(pool.active_count(), 1);

        let _guard2 = pool
            .try_acquire()
            .expect("Should acquire second connection");
        assert_eq!(pool.active_count(), 2);
    }

    #[test]
    fn test_ws_connection_pool_limit() {
        let pool = WsConnectionPool::new(2);

        let _guard1 = pool.try_acquire().expect("Should acquire first connection");
        let _guard2 = pool
            .try_acquire()
            .expect("Should acquire second connection");

        // Pool is now full
        assert_eq!(pool.active_count(), 2);

        // Third connection should fail
        let guard3 = pool.try_acquire();
        assert!(
            guard3.is_none(),
            "Should reject connection when pool is full"
        );

        // Still at capacity
        assert_eq!(pool.active_count(), 2);
    }

    #[test]
    fn test_ws_connection_pool_release() {
        let pool = WsConnectionPool::new(2);

        {
            let _guard1 = pool.try_acquire().expect("Should acquire connection");
            assert_eq!(pool.active_count(), 1);

            {
                let _guard2 = pool
                    .try_acquire()
                    .expect("Should acquire second connection");
                assert_eq!(pool.active_count(), 2);
            } // guard2 dropped here

            // After guard2 is dropped, count should decrease
            assert_eq!(pool.active_count(), 1);
        } // guard1 dropped here

        // After both guards are dropped, pool should be empty
        assert_eq!(pool.active_count(), 0);

        // Should be able to acquire again
        let _guard3 = pool.try_acquire().expect("Should acquire after release");
        assert_eq!(pool.active_count(), 1);
    }

    #[test]
    fn test_ws_connection_pool_concurrent() {
        use std::sync::{Arc, Mutex};
        use std::thread;

        let pool = Arc::new(WsConnectionPool::new(10));
        let guards = Arc::new(Mutex::new(Vec::new()));

        let mut handles = vec![];

        // Spawn 20 threads trying to acquire connections
        for _ in 0..20 {
            let pool_clone = Arc::clone(&pool);
            let guards_clone = Arc::clone(&guards);
            let handle = thread::spawn(move || {
                if let Some(guard) = pool_clone.try_acquire() {
                    // Hold the guard so it doesn't get dropped immediately
                    guards_clone.lock().unwrap().push(guard);
                    true
                } else {
                    false
                }
            });
            handles.push(handle);
        }

        // Wait for all threads and collect results
        let mut successful = 0;
        let mut failed = 0;

        for handle in handles {
            if handle.join().unwrap() {
                successful += 1;
            } else {
                failed += 1;
            }
        }

        // Should have exactly 10 successful and 10 failed
        assert_eq!(successful, 10, "Should have 10 successful acquisitions");
        assert_eq!(failed, 10, "Should have 10 failed acquisitions");

        // Verify active count
        assert_eq!(
            pool.active_count(),
            10,
            "Pool should have 10 active connections"
        );
    }
}
