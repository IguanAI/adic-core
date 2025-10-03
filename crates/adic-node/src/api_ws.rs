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
    response::Response,
};
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info, warn};

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

/// WebSocket handler
pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(event_bus): State<Arc<EventBus>>,
    State(metrics): State<Metrics>,
) -> Response {
    info!(
        events = ?query.events,
        priority = ?query.priority,
        "WebSocket connection request"
    );

    ws.on_upgrade(move |socket| handle_socket(socket, query, event_bus, metrics))
}

/// Handle WebSocket connection
async fn handle_socket(
    socket: WebSocket,
    query: WsQuery,
    event_bus: Arc<EventBus>,
    metrics: Metrics,
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
                    }
                    SubscriptionCommand::Unsubscribe(events) => {
                        debug!(events = ?events, "Processing unsubscribe command");
                        connection.unsubscribe(events.clone());

                        let response = ServerMessage::Unsubscribed { events };
                        let msg = serde_json::to_string(&response)?;
                        sender.send(Message::Text(msg)).await?;
                    }
                    SubscriptionCommand::Ping => {
                        debug!("Processing ping command");
                        let response = ServerMessage::Pong;
                        let msg = serde_json::to_string(&response)?;
                        sender.send(Message::Text(msg)).await?;
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
}
