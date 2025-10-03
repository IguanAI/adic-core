# 🚀 Event Streaming Quick Start

Get ADIC's real-time event streaming running in 5 minutes.

## Prerequisites

- Rust 1.70+
- Python 3.9+ (for backend)
- Running ADIC node

## 1. Verify Implementation ✓

```bash
bash scripts/verify_event_streaming.sh
```

Expected output: `✓ All checks passed! Implementation is production ready.`

## 2. Start the Node

```bash
# Build (first time only)
cargo build --release

# Start node
./target/release/adic start
```

Node will start on `http://localhost:9121`

## 3. Test WebSocket Connection

### Option A: Browser (Easiest)

Open `docs/examples/browser/websocket-client.html` in your browser.

1. Enter node URL: `ws://localhost:9121`
2. Click "Connect"
3. Watch live events stream in!

### Option B: Command Line

```bash
# Install wscat if needed
npm install -g wscat

# Connect to event stream
wscat -c ws://localhost:9121/v1/ws/events?events=all
```

### Option C: Python Script

```bash
cd docs/examples/python
python3 event_stream_client.py --node-url ws://localhost:9121
```

## 4. Test SSE Connection

```bash
curl -N http://localhost:9121/v1/sse/events?events=all
```

You should see events streaming in real-time:

```
event: TipsUpdated
data: {"tips":["abc123..."],"count":1,"timestamp":"2024-01-15T10:00:00Z"}

event: MessageFinalized
data: {"message_id":"def456...","finality_type":"KCore","timestamp":"2024-01-15T10:00:05Z"}
```

## 5. Check Health & Metrics

```bash
# Health check
curl http://localhost:9121/v1/health/streaming

# Prometheus metrics
curl http://localhost:9121/metrics | grep adic_events
```

## 6. Start Backend (Optional)

If you have the explorer backend:

```bash
cd ../adic-explorer/backend

# Install dependencies
pip install -r requirements.txt

# Set environment
export ENABLE_EVENT_STREAMING=True
export ADIC_NODE_URL=http://localhost:9121

# Start backend
uvicorn app.main:app --host 0.0.0.0 --port 9122
```

Check backend health:

```bash
curl http://localhost:9122/health/streaming
```

## 7. Filter Events (Optional)

Customize what events you receive:

### High Priority Only

```bash
wscat -c 'ws://localhost:9121/v1/ws/events?priority=high'
```

### Specific Event Types

```bash
wscat -c 'ws://localhost:9121/v1/ws/events?events=TipsUpdated,MessageFinalized'
```

### Combined Filters

```bash
wscat -c 'ws://localhost:9121/v1/ws/events?events=TipsUpdated,MessageFinalized&priority=high'
```

## Event Types

| Event | Priority | Description |
|-------|----------|-------------|
| **TipsUpdated** | High | DAG tips changed |
| **MessageFinalized** | High | Message achieved finality |
| **MessageAdded** | High | New message added to DAG |
| **DiversityUpdated** | Medium | Diversity metrics updated |
| **EnergyUpdated** | Medium | Conflict energy changed |
| **KCoreUpdated** | Medium | K-core finality updated |
| **AdmissibilityUpdated** | Low | Compliance rates updated |
| **EconomicsUpdated** | Low | Token economics changed |

## Troubleshooting

### Connection Refused

```bash
# Check if node is running
curl http://localhost:9121/health

# Check logs
journalctl -u adic-node -f
```

### No Events Received

```bash
# Verify events are being emitted
curl http://localhost:9121/metrics | grep events_emitted_total

# Check node is processing messages
curl http://localhost:9121/tips
```

### WebSocket Closes Immediately

- Check firewall allows WebSocket connections
- Verify no proxy is blocking upgrades
- Try SSE as fallback

## Next Steps

- 📖 Read full [API documentation](docs/EVENT_STREAMING_API.md)
- 🔧 Follow [integration guide](docs/EVENT_STREAMING_INTEGRATION_GUIDE.md)
- 🚀 Review [deployment checklist](docs/EVENT_STREAMING_DEPLOYMENT.md)
- 💻 Explore [examples](docs/examples/)

## Performance

Event streaming vs polling:

| Metric | Polling | Streaming | Improvement |
|--------|---------|-----------|-------------|
| **Requests/sec** | ~11 | ~0 | 11x fewer |
| **Latency** | 1-10s | <100ms | 10-100x faster |
| **Server load** | High | Low | Significant |
| **Bandwidth** | High | Low | ~70% reduction |

## Support

- 📚 Documentation: `/docs/EVENT_STREAMING_*.md`
- 🐛 Issues: https://github.com/adic/adic-core/issues
- 💬 Discord: https://discord.gg/adic

---

**Status**: ✅ Production Ready | **Version**: 1.0.0 | **Date**: January 2025
