# Event Streaming Integration Guide

This guide walks you through integrating ADIC's real-time event streaming into your application, replacing traditional REST API polling with efficient WebSocket or SSE connections.

## Table of Contents

1. [Quick Start](#quick-start)
2. [Architecture Overview](#architecture-overview)
3. [Implementation Steps](#implementation-steps)
4. [Client Libraries](#client-libraries)
5. [Testing](#testing)
6. [Production Deployment](#production-deployment)
7. [Troubleshooting](#troubleshooting)

## Quick Start

### JavaScript/Browser

```html
<!DOCTYPE html>
<html>
<head>
    <title>ADIC Event Stream Demo</title>
</head>
<body>
    <h1>Live DAG Events</h1>
    <div id="events"></div>

    <script>
        const nodeUrl = 'ws://localhost:9121';
        const ws = new WebSocket(`${nodeUrl}/v1/ws/events?events=all&priority=all`);

        ws.onopen = () => {
            console.log('Connected to ADIC event stream');
            document.getElementById('events').innerHTML += '<p>✓ Connected</p>';
        };

        ws.onmessage = (event) => {
            const message = JSON.parse(event.data);

            if (message.type === 'Event') {
                const eventData = message.event;
                console.log('Event:', eventData);

                const eventDiv = document.createElement('div');
                eventDiv.textContent = `${eventData.type} at ${eventData.timestamp}`;
                document.getElementById('events').prepend(eventDiv);
            }
        };

        ws.onerror = (error) => {
            console.error('WebSocket error:', error);
        };

        ws.onclose = () => {
            console.log('Connection closed, reconnecting in 5s...');
            setTimeout(() => window.location.reload(), 5000);
        };
    </script>
</body>
</html>
```

### Python/asyncio

```python
import asyncio
import aiohttp
import json

async def connect_to_events():
    node_url = 'http://localhost:9121'

    async with aiohttp.ClientSession() as session:
        async with session.ws_connect(
            f'{node_url}/v1/ws/events',
            params={'events': 'all', 'priority': 'high'}
        ) as ws:
            print('Connected to ADIC event stream')

            async for msg in ws:
                if msg.type == aiohttp.WSMsgType.TEXT:
                    data = json.loads(msg.data)

                    if data['type'] == 'Event':
                        event = data['event']
                        print(f"Event: {event['type']} at {event['timestamp']}")

                        # Handle specific event types
                        if event['type'] == 'TipsUpdated':
                            print(f"  Tips count: {event['count']}")
                        elif event['type'] == 'MessageFinalized':
                            print(f"  Message: {event['message_id'][:16]}...")

if __name__ == '__main__':
    asyncio.run(connect_to_events())
```

## Architecture Overview

### Event Flow

```
┌─────────────┐      ┌──────────────┐      ┌─────────────┐
│  DAG State  │─────>│  Event Bus   │─────>│  Broadcast  │
│   Changes   │      │   (Tokio)    │      │  Channels   │
└─────────────┘      └──────────────┘      └─────────────┘
                                                    │
                     ┌──────────────────────────────┼──────────────────────┐
                     │                              │                      │
                     ▼                              ▼                      ▼
              ┌─────────────┐              ┌─────────────┐        ┌─────────────┐
              │  WebSocket  │              │     SSE     │        │   Metrics   │
              │  Endpoint   │              │  Endpoint   │        │  Prometheus │
              └─────────────┘              └─────────────┘        └─────────────┘
                     │                              │
                     ▼                              ▼
              ┌─────────────┐              ┌─────────────┐
              │   Clients   │              │   Clients   │
              │ (WebSocket) │              │    (SSE)    │
              └─────────────┘              └─────────────┘
```

### Priority Channels

Events are routed through three priority channels:

- **High**: Tips, Finality, Message Added (buffer: 1000)
- **Medium**: Diversity, Energy, K-core (buffer: 500)
- **Low**: Admissibility, Economics (buffer: 100)

This design prevents low-priority events from delaying critical notifications.

## Implementation Steps

### Step 1: Choose Your Transport

**WebSocket** - Best for:
- Interactive applications
- Bidirectional communication
- Dynamic event subscriptions
- Maximum performance

**SSE** - Best for:
- Simple unidirectional streaming
- Browser-only clients
- Environments with strict proxy requirements
- Automatic browser reconnection

### Step 2: Implement Connection Manager

Create a connection manager with automatic fallback and reconnection:

```python
class ConnectionManager:
    def __init__(self, node_url: str):
        self.node_url = node_url
        self.connection = None
        self.mode = None  # 'websocket', 'sse', or 'polling'
        self.reconnect_delay = 1.0
        self.max_reconnect_delay = 60.0

    async def connect(self) -> bool:
        # Try WebSocket first
        try:
            self.connection = await self.connect_websocket()
            self.mode = 'websocket'
            self.reconnect_delay = 1.0
            return True
        except Exception as e:
            print(f"WebSocket failed: {e}")

        # Fall back to SSE
        try:
            self.connection = await self.connect_sse()
            self.mode = 'sse'
            self.reconnect_delay = 1.0
            return True
        except Exception as e:
            print(f"SSE failed: {e}")

        # Fall back to polling
        self.mode = 'polling'
        return False

    async def reconnect_loop(self):
        while True:
            if not await self.connect():
                await asyncio.sleep(self.reconnect_delay)
                self.reconnect_delay = min(
                    self.reconnect_delay * 2,
                    self.max_reconnect_delay
                )
            else:
                break
```

### Step 3: Handle Events

Create event handlers for each event type:

```python
async def handle_event(event: dict):
    event_type = event['type']

    handlers = {
        'TipsUpdated': handle_tips_updated,
        'MessageFinalized': handle_message_finalized,
        'DiversityUpdated': handle_diversity_updated,
        'EnergyUpdated': handle_energy_updated,
        'KCoreUpdated': handle_kcore_updated,
        'AdmissibilityUpdated': handle_admissibility_updated,
        'EconomicsUpdated': handle_economics_updated,
        'MessageAdded': handle_message_added,
    }

    handler = handlers.get(event_type)
    if handler:
        await handler(event)
    else:
        print(f"Unknown event type: {event_type}")

async def handle_tips_updated(event: dict):
    tips = event['tips']
    count = event['count']
    print(f"Tips updated: {count} tips")

    # Update your application state
    await update_tips_display(tips)

async def handle_message_finalized(event: dict):
    message_id = event['message_id']
    finality_type = event['finality_type']
    print(f"Message finalized: {message_id} ({finality_type})")

    # Update finality status in your database
    await update_message_finality(message_id, finality_type)
```

### Step 4: Filter Events

Subscribe only to events you need:

```python
# High priority events only
params = {'priority': 'high'}

# Specific event types
params = {'events': 'TipsUpdated,MessageFinalized'}

# Combination
params = {
    'events': 'TipsUpdated,MessageFinalized,DiversityUpdated',
    'priority': 'high'
}
```

### Step 5: Add Metrics and Monitoring

Track connection health:

```python
class EventStreamMetrics:
    def __init__(self):
        self.events_received = 0
        self.events_by_type = {}
        self.connection_status = 'disconnected'
        self.last_event_time = None
        self.reconnections = 0

    def record_event(self, event_type: str):
        self.events_received += 1
        self.events_by_type[event_type] = \
            self.events_by_type.get(event_type, 0) + 1
        self.last_event_time = datetime.now()

    def is_healthy(self) -> bool:
        if self.connection_status != 'connected':
            return False

        if self.last_event_time:
            age = (datetime.now() - self.last_event_time).seconds
            return age < 60  # No events in 60s = unhealthy

        return True
```

## Client Libraries

### Backend Explorer Integration

The ADIC Explorer backend includes a complete event-driven indexer:

```python
from app.services.event_indexer import EventDrivenIndexer

# Initialize indexer
indexer = EventDrivenIndexer(node_url="https://node.example.com")

# Start receiving events
await indexer.start()

# Get statistics
stats = indexer.get_stats()
print(f"Events processed: {stats['events_processed']}")
print(f"Connection mode: {stats['connection_status']['mode']}")
```

### Custom Client

Use the provided connection manager:

```python
from app.services.event_client import ConnectionManager, EventSubscription

# Create subscription
subscription = EventSubscription(
    event_types={"all"},
    priority="all",
    callback=handle_event
)

# Create connection manager
manager = ConnectionManager(
    node_url="https://node.example.com",
    subscription=subscription,
    enable_websocket=True,
    enable_sse=True,
)

# Connect (automatically falls back if needed)
if await manager.connect():
    print(f"Connected via {manager.mode.value}")
else:
    print("Falling back to polling")
```

## Testing

### Unit Tests

Test event handling in isolation:

```python
import pytest
from unittest.mock import AsyncMock

@pytest.mark.asyncio
async def test_event_handling():
    handler = AsyncMock()

    event = {
        'type': 'TipsUpdated',
        'tips': ['abc123'],
        'count': 1,
        'timestamp': '2024-01-15T10:00:00Z'
    }

    await handler(event)

    handler.assert_called_once_with(event)
```

### Integration Tests

Test actual WebSocket/SSE connections:

```python
@pytest.mark.asyncio
async def test_websocket_connection():
    async with aiohttp.ClientSession() as session:
        async with session.ws_connect('ws://localhost:9121/v1/ws/events') as ws:
            # Should receive Subscribed confirmation
            msg = await ws.receive_json()
            assert msg['type'] == 'Subscribed'

            # Should receive events
            msg = await asyncio.wait_for(ws.receive_json(), timeout=5.0)
            assert msg['type'] == 'Event'
```

### Load Testing

Test connection limits:

```bash
# Install artillery
npm install -g artillery

# Create load test config
cat > load-test.yml <<EOF
config:
  target: "ws://localhost:9121"
  phases:
    - duration: 60
      arrivalRate: 10
scenarios:
  - engine: ws
    flow:
      - connect:
          path: "/v1/ws/events?events=all"
      - think: 60
EOF

# Run load test
artillery run load-test.yml
```

## Production Deployment

### Node Configuration

Enable event streaming in your node config:

```toml
[api]
host = "0.0.0.0"
port = 9121
enable_websocket = true
enable_sse = true

[events]
high_priority_buffer = 1000
medium_priority_buffer = 500
low_priority_buffer = 100
```

### Monitoring

Monitor event streaming health:

```bash
# Check node health
curl http://localhost:9121/v1/health/streaming

# Check Prometheus metrics
curl http://localhost:9121/metrics | grep adic_events

# Expected metrics:
# adic_events_emitted_total
# adic_websocket_connections
# adic_sse_connections
# adic_websocket_messages_sent_total
# adic_sse_messages_sent_total
```

### Load Balancing

When load balancing across multiple nodes:

1. Use sticky sessions for WebSocket connections
2. SSE can be load balanced freely
3. Monitor connection distribution

Example nginx configuration:

```nginx
upstream adic_nodes {
    ip_hash;  # Sticky sessions
    server node1:9121;
    server node2:9121;
    server node3:9121;
}

server {
    listen 80;

    location /v1/ws/ {
        proxy_pass http://adic_nodes;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_read_timeout 86400;
    }

    location /v1/sse/ {
        proxy_pass http://adic_nodes;
        proxy_buffering off;
        proxy_cache off;
        proxy_read_timeout 86400;
    }
}
```

### Security

1. **Use TLS in production**: wss:// and https://
2. **Rate limiting**: Prevent abuse with connection limits
3. **Authentication**: Require auth tokens if needed
4. **CORS**: Configure allowed origins

## Troubleshooting

### Connection Fails

**Symptom**: Can't establish WebSocket/SSE connection

**Solutions**:
1. Check node is running: `curl http://localhost:9121/health`
2. Verify endpoint exists: `curl http://localhost:9121/v1/health/streaming`
3. Check firewall rules allow WebSocket (port 9121)
4. Review node logs for errors

### No Events Received

**Symptom**: Connected but not receiving events

**Solutions**:
1. Check event filters aren't too restrictive
2. Verify node is emitting events: check metrics endpoint
3. Ensure connection isn't stale (check last_event_time)
4. Look for network issues (check latency)

### High Memory Usage

**Symptom**: Node using excessive memory

**Solutions**:
1. Reduce event buffer sizes in config
2. Limit number of concurrent connections
3. Implement connection timeouts
4. Check for event handler memory leaks

### Events Delayed

**Symptom**: Events arrive late

**Solutions**:
1. Check system load (CPU, network)
2. Reduce event buffer sizes (counterintuitive but helps)
3. Use priority filters to reduce traffic
4. Check for slow event handlers blocking the stream

### Connection Drops Frequently

**Symptom**: Connection reconnects often

**Solutions**:
1. Increase heartbeat interval
2. Check network stability
3. Add exponential backoff to reconnection logic
4. Use SSE instead of WebSocket (more stable through proxies)

## Performance Tips

1. **Use WebSocket for best performance**: Lower overhead than SSE
2. **Filter events aggressively**: Only subscribe to what you need
3. **Process events asynchronously**: Don't block the event handler
4. **Batch UI updates**: Don't update UI on every event
5. **Monitor connection health**: Reconnect proactively if stale

## Examples

Complete example implementations are available in the repository:

- `/docs/examples/browser/`: JavaScript browser client
- `/docs/examples/python/`: Python asyncio client
- `/docs/examples/rust/`: Rust tokio client
- `/docs/examples/explorer/`: Full explorer backend integration

## Support

- **Documentation**: https://docs.adic.com/event-streaming
- **Issues**: https://github.com/adic/adic-core/issues
- **Discord**: https://discord.gg/adic

## Summary

Event streaming replaces polling with push-based notifications:

- **11x fewer HTTP requests**
- **Sub-100ms latency** vs 1-10s polling delay
- **Lower server load** and bandwidth usage
- **Real-time updates** for better UX

Follow this guide to integrate event streaming into your application and enjoy the benefits of real-time DAG state updates!
