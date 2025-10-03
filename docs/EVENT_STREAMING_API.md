# ADIC Event Streaming API

## Overview

The ADIC node provides real-time event streaming via WebSocket and Server-Sent Events (SSE) to eliminate the need for polling. This allows clients to receive instant notifications when state changes occur in the DAG.

## Endpoints

### WebSocket Endpoint

**URL**: `/v1/ws/events`

**Protocol**: WebSocket (ws:// or wss://)

**Description**: Bidirectional connection for real-time events with support for subscribe/unsubscribe messages.

#### Query Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `events` | string | `all` | Comma-separated list of event types to subscribe to, or "all" |
| `priority` | string | `all` | Event priority filter: `high`, `medium`, `low`, or `all` |

#### Client-to-Server Messages

```json
{
  "type": "Subscribe",
  "events": ["TipsUpdated", "MessageFinalized"]
}
```

```json
{
  "type": "Unsubscribe",
  "events": ["TipsUpdated"]
}
```

```json
{
  "type": "Ping"
}
```

#### Server-to-Client Messages

**Event Notification**:
```json
{
  "type": "Event",
  "event": {
    "type": "TipsUpdated",
    "tips": ["abc123def456", "789ghi012jkl"],
    "count": 2,
    "timestamp": "2024-01-15T10:30:00Z"
  }
}
```

**Subscription Confirmation**:
```json
{
  "type": "Subscribed",
  "events": ["TipsUpdated", "MessageFinalized"]
}
```

**Error**:
```json
{
  "type": "Error",
  "message": "Invalid event type"
}
```

#### Example Usage

```javascript
const ws = new WebSocket('ws://localhost:9121/v1/ws/events?events=TipsUpdated,MessageFinalized&priority=high');

ws.onopen = () => {
  console.log('Connected to event stream');
};

ws.onmessage = (event) => {
  const message = JSON.parse(event.data);

  if (message.type === 'Event') {
    console.log('Received event:', message.event);
  }
};

// Subscribe to additional events
ws.send(JSON.stringify({
  type: 'Subscribe',
  events: ['DiversityUpdated']
}));
```

### SSE Endpoint

**URL**: `/v1/sse/events`

**Protocol**: HTTP(S) with `text/event-stream`

**Description**: Unidirectional server-to-client event stream. Simpler than WebSocket but no client-to-server messaging.

#### Query Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `events` | string | `all` | Comma-separated list of event types to subscribe to, or "all" |
| `priority` | string | `all` | Event priority filter: `high`, `medium`, `low`, or `all` |
| `heartbeat` | boolean | `true` | Enable heartbeat messages every 15 seconds |

#### Event Format

Events are sent in Server-Sent Events format:

```
event: TipsUpdated
data: {"tips":["abc123"],"count":1,"timestamp":"2024-01-15T10:30:00Z"}

event: MessageFinalized
data: {"message_id":"def456","finality_type":"KCore","timestamp":"2024-01-15T10:30:05Z"}
```

#### Example Usage

```javascript
const eventSource = new EventSource('http://localhost:9121/v1/sse/events?priority=high');

eventSource.addEventListener('TipsUpdated', (event) => {
  const data = JSON.parse(event.data);
  console.log('Tips updated:', data);
});

eventSource.addEventListener('MessageFinalized', (event) => {
  const data = JSON.parse(event.data);
  console.log('Message finalized:', data);
});

eventSource.onerror = (error) => {
  console.error('SSE error:', error);
  // Browser automatically reconnects
};
```

## Event Types

### High Priority Events

Events that require immediate notification (buffer size: 1000).

#### TipsUpdated
Emitted when the set of current tips changes.

```json
{
  "type": "TipsUpdated",
  "tips": ["abc123", "def456"],
  "count": 2,
  "timestamp": "2024-01-15T10:30:00Z"
}
```

#### MessageFinalized
Emitted when a message achieves finality.

```json
{
  "type": "MessageFinalized",
  "message_id": "abc123def456",
  "finality_type": "KCore",
  "timestamp": "2024-01-15T10:30:00Z"
}
```

#### MessageAdded
Emitted when a new message is added to the DAG.

```json
{
  "type": "MessageAdded",
  "message_id": "abc123def456",
  "depth": 42,
  "timestamp": "2024-01-15T10:30:00Z"
}
```

### Medium Priority Events

Regular updates about DAG metrics (buffer size: 500).

#### DiversityUpdated
Emitted when diversity metrics are recalculated.

```json
{
  "type": "DiversityUpdated",
  "diversity_score": 0.75,
  "axes": [
    {"axis": 0, "entropy": 2.5, "unique_values": 8},
    {"axis": 1, "entropy": 3.1, "unique_values": 12}
  ],
  "total_tips": 15,
  "timestamp": "2024-01-15T10:30:00Z"
}
```

#### EnergyUpdated
Emitted when conflict resolution energy changes.

```json
{
  "type": "EnergyUpdated",
  "total_conflicts": 100,
  "resolved_conflicts": 95,
  "active_conflicts": 5,
  "total_energy": 12.5,
  "timestamp": "2024-01-15T10:30:00Z"
}
```

#### KCoreUpdated
Emitted when k-core finality metrics change.

```json
{
  "type": "KCoreUpdated",
  "finalized_count": 1000,
  "pending_count": 15,
  "current_k_value": 3,
  "timestamp": "2024-01-15T10:30:00Z"
}
```

### Low Priority Events

Infrequent updates and statistics (buffer size: 100).

#### AdmissibilityUpdated
Emitted periodically with admissibility compliance rates.

```json
{
  "type": "AdmissibilityUpdated",
  "c1_rate": 0.99,
  "c2_rate": 0.98,
  "c3_rate": 0.97,
  "overall_rate": 0.98,
  "sample_size": 100,
  "timestamp": "2024-01-15T10:30:00Z"
}
```

#### EconomicsUpdated
Emitted when token economics metrics change.

```json
{
  "type": "EconomicsUpdated",
  "total_supply": "1000000",
  "circulating_supply": "750000",
  "treasury_balance": "250000",
  "timestamp": "2024-01-15T10:30:00Z"
}
```

## Event Priorities

Events are categorized into three priority levels to optimize bandwidth and allow clients to subscribe only to events they need:

- **High Priority**: Critical state changes (tips, finality, new messages)
- **Medium Priority**: Regular metrics updates (diversity, energy, k-core)
- **Low Priority**: Infrequent statistics (admissibility, economics)

## Connection Management

### Automatic Reconnection

Both WebSocket and SSE clients should implement automatic reconnection with exponential backoff:

- Initial delay: 1 second
- Maximum delay: 60 seconds
- Backoff multiplier: 2x

### Health Monitoring

Check the health of event streaming via:

**Node Health**: `GET /v1/health/streaming`

```json
{
  "status": "healthy",
  "event_streaming": {
    "enabled": true,
    "websocket_enabled": true,
    "sse_enabled": true
  },
  "metrics": {
    "events_emitted_total": 1234,
    "websocket_connections": 5,
    "sse_connections": 3,
    "websocket_messages_sent": 5678,
    "sse_messages_sent": 3456
  }
}
```

## Metrics

Event streaming metrics are exposed via Prometheus at `/metrics`:

```
# Total events emitted
adic_events_emitted_total

# Active WebSocket connections
adic_websocket_connections

# Active SSE connections
adic_sse_connections

# Total WebSocket messages sent
adic_websocket_messages_sent_total

# Total SSE messages sent
adic_sse_messages_sent_total
```

## Best Practices

### 1. Choose the Right Transport

- **WebSocket**: Best for bidirectional communication, dynamic subscriptions, and maximum performance
- **SSE**: Best for simple unidirectional streaming, works through proxies, automatic reconnection

### 2. Filter Events

Only subscribe to events you need to reduce bandwidth:

```
/v1/ws/events?events=TipsUpdated,MessageFinalized&priority=high
```

### 3. Handle Reconnection

Always implement reconnection logic with exponential backoff to handle network interruptions.

### 4. Process Events Asynchronously

Don't block the event handler with long-running operations. Queue events for async processing.

### 5. Monitor Connection Health

Regularly check connection status and reconnect if the connection becomes stale.

## Error Handling

### Connection Errors

- **WebSocket**: Check `onerror` and `onclose` events, reconnect automatically
- **SSE**: Browser handles reconnection automatically, listen for `onerror`

### Event Processing Errors

If event processing fails, log the error but don't disconnect. Failed events should be skipped or retried individually.

## Security

### Authentication

Event streaming endpoints respect the same authentication middleware as other API endpoints. Include authentication headers if required.

### Rate Limiting

Event streams are not subject to standard API rate limits, but may be throttled if a client causes excessive load.

### CORS

Event streaming endpoints support CORS for browser clients. Configure allowed origins in node configuration.

## Migration from Polling

### Before (Polling)

```javascript
// Poll every 5 seconds
setInterval(async () => {
  const tips = await fetch('/tips').then(r => r.json());
  const messages = await fetch('/messages/since/' + lastId).then(r => r.json());
  // Process updates...
}, 5000);
```

### After (Event Streaming)

```javascript
const ws = new WebSocket('ws://node/v1/ws/events?events=TipsUpdated,MessageAdded');

ws.onmessage = (event) => {
  const message = JSON.parse(event.data);
  if (message.type === 'Event') {
    // Process update in real-time
  }
};
```

**Benefits**:
- 11x reduction in HTTP requests
- Sub-100ms latency instead of 5-second polling delay
- Lower server load
- Reduced bandwidth usage

## Example Implementations

See `/docs/examples/` for complete implementation examples in:
- JavaScript (Browser)
- Python (async/await)
- Rust (tokio)
- Go (goroutines)
