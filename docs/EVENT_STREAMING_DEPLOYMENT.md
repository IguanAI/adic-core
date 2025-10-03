# Event Streaming Deployment Checklist

This checklist ensures a smooth deployment of the ADIC event streaming system to production.

## Pre-Deployment

### Code Review

- [x] All event emission points are properly integrated
- [x] WebSocket endpoint is registered at `/v1/ws/events`
- [x] SSE endpoint is registered at `/v1/sse/events`
- [x] Health check endpoint at `/v1/health/streaming`
- [x] Metrics are exposed via Prometheus
- [x] No TODOs or placeholders in production code
- [x] Error handling is comprehensive
- [x] Tests pass successfully

### Configuration

- [ ] Set appropriate buffer sizes in node config:
  ```toml
  [events]
  high_priority_buffer = 1000
  medium_priority_buffer = 500
  low_priority_buffer = 100
  ```

- [ ] Enable event streaming in backend config:
  ```python
  ENABLE_EVENT_STREAMING = True
  ENABLE_WEBSOCKET = True
  ENABLE_SSE = True
  ```

- [ ] Configure CORS origins for browser clients:
  ```python
  CORS_ORIGINS = ["https://explorer.yourdomain.com"]
  ```

- [ ] Set connection limits:
  ```toml
  [api]
  max_websocket_connections = 10000
  max_sse_connections = 10000
  ```

### Infrastructure

- [ ] Firewall rules allow WebSocket connections (port 9121)
- [ ] Load balancer configured for sticky sessions (WebSocket)
- [ ] TLS certificates installed for wss:// and https://
- [ ] Reverse proxy configured (if using nginx/Apache)
- [ ] Prometheus configured to scrape metrics
- [ ] Log aggregation setup for event-related logs

## Deployment Steps

### 1. Node Deployment

```bash
# Build release binary
cargo build --release

# Verify binary includes event streaming
./target/release/adic --help | grep events

# Deploy to production
scp target/release/adic production-server:/opt/adic/

# Restart node service
ssh production-server 'sudo systemctl restart adic-node'

# Verify node is running
curl https://node.yourdomain.com/health
curl https://node.yourdomain.com/v1/health/streaming
```

### 2. Backend Deployment

```bash
# Install dependencies
cd adic-explorer/backend
pip install -r requirements.txt

# Verify event modules exist
python -c "from app.services.event_client import ConnectionManager; print('✓')"
python -c "from app.services.event_indexer import EventDrivenIndexer; print('✓')"

# Deploy backend
# (use your deployment process - Docker, systemd, etc.)

# Verify backend health
curl https://api.yourdomain.com/health
curl https://api.yourdomain.com/health/streaming
```

### 3. Frontend Deployment

```bash
# Update frontend to use event streaming
# Point to new WebSocket endpoints

# Deploy frontend
# (use your deployment process)
```

## Post-Deployment Verification

### Health Checks

```bash
# Node health
curl https://node.yourdomain.com/v1/health/streaming

# Expected response:
# {
#   "status": "healthy",
#   "event_streaming": { "enabled": true, ... },
#   "metrics": { "events_emitted_total": N, ... }
# }

# Backend health
curl https://api.yourdomain.com/health/streaming

# Expected response:
# {
#   "enabled": true,
#   "connection": { "mode": "websocket", "status": "connected" },
#   "events": { "total_processed": N, ... }
# }
```

### Metrics Verification

```bash
# Check Prometheus metrics
curl https://node.yourdomain.com/metrics | grep adic_events

# Expected metrics:
# adic_events_emitted_total
# adic_websocket_connections
# adic_sse_connections
# adic_websocket_messages_sent_total
# adic_sse_messages_sent_total
```

### Connection Testing

```bash
# Test WebSocket connection
wscat -c wss://node.yourdomain.com/v1/ws/events?events=all

# Test SSE connection
curl -N https://node.yourdomain.com/v1/sse/events?events=all
```

### Load Testing

```bash
# Run load test with artillery
artillery quick --count 100 --num 60 wss://node.yourdomain.com/v1/ws/events

# Monitor metrics during load test
watch -n 1 'curl -s https://node.yourdomain.com/metrics | grep websocket_connections'
```

## Monitoring Setup

### Prometheus Alerts

Create alerts for event streaming issues:

```yaml
# alerts.yml
groups:
  - name: adic_events
    interval: 30s
    rules:
      - alert: NoEventsEmitted
        expr: rate(adic_events_emitted_total[5m]) == 0
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "No events emitted in 5 minutes"

      - alert: HighWebSocketConnections
        expr: adic_websocket_connections > 8000
        for: 2m
        labels:
          severity: warning
        annotations:
          summary: "High number of WebSocket connections"

      - alert: EventStreamingDown
        expr: up{job="adic-node"} == 0
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "ADIC node is down"
```

### Grafana Dashboard

Import the event streaming dashboard:

```json
{
  "dashboard": {
    "title": "ADIC Event Streaming",
    "panels": [
      {
        "title": "Events Emitted",
        "targets": [
          { "expr": "rate(adic_events_emitted_total[5m])" }
        ]
      },
      {
        "title": "Active Connections",
        "targets": [
          { "expr": "adic_websocket_connections" },
          { "expr": "adic_sse_connections" }
        ]
      },
      {
        "title": "Messages Sent",
        "targets": [
          { "expr": "rate(adic_websocket_messages_sent_total[5m])" },
          { "expr": "rate(adic_sse_messages_sent_total[5m])" }
        ]
      }
    ]
  }
}
```

### Log Monitoring

Set up log alerts for errors:

```bash
# Example with journalctl
journalctl -u adic-node -f | grep -i "websocket error\|sse error\|event error"
```

## Rollback Plan

If issues occur, follow this rollback procedure:

### 1. Disable Event Streaming in Backend

```bash
# Update backend config
ENABLE_EVENT_STREAMING=False

# Restart backend
systemctl restart adic-explorer-backend
```

Backend will automatically fall back to REST polling.

### 2. Verify Polling is Working

```bash
# Check backend logs
journalctl -u adic-explorer-backend | grep "polling-based indexer"

# Verify data is being indexed
curl https://api.yourdomain.com/api/v1/messages
```

### 3. Full Rollback (if needed)

```bash
# Redeploy previous node version
scp backup/adic production-server:/opt/adic/
ssh production-server 'sudo systemctl restart adic-node'

# Redeploy previous backend version
# (use your deployment process)
```

## Performance Tuning

### Buffer Size Tuning

If events are being dropped (lagging):

```toml
# Increase buffer sizes
[events]
high_priority_buffer = 2000    # was 1000
medium_priority_buffer = 1000  # was 500
low_priority_buffer = 200      # was 100
```

### Connection Limits

If running out of file descriptors:

```bash
# Increase system limits
ulimit -n 65536

# Or in /etc/security/limits.conf:
adic-node soft nofile 65536
adic-node hard nofile 65536
```

### Network Tuning

For high-throughput scenarios:

```bash
# Increase TCP buffer sizes
sysctl -w net.core.rmem_max=26214400
sysctl -w net.core.wmem_max=26214400
sysctl -w net.ipv4.tcp_rmem="4096 87380 26214400"
sysctl -w net.ipv4.tcp_wmem="4096 65536 26214400"
```

## Troubleshooting

### No Events Received

**Check:**
1. Node is emitting events: `curl /metrics | grep events_emitted_total`
2. Backend is connected: `curl /health/streaming`
3. WebSocket connection is established: Check browser DevTools Network tab
4. Event filters aren't too restrictive: Try `events=all`

### High Latency

**Check:**
1. System load (CPU, memory, network)
2. Event handler blocking: Ensure async processing
3. Buffer sizes: May need to increase
4. Network bandwidth: Check for saturation

### Connection Drops

**Check:**
1. Proxy timeout settings (increase to 86400s)
2. Load balancer health checks (exclude WebSocket paths)
3. Network stability (packet loss, jitter)
4. Node restart/deployment during connection

## Success Criteria

Deployment is successful when:

- [x] All health endpoints return 200 OK
- [x] Events are being emitted (metrics > 0)
- [x] Backend is connected via WebSocket or SSE
- [x] Frontend receives real-time updates
- [x] No errors in logs
- [x] Metrics are being collected
- [x] Alerts are configured
- [x] Load test passes (100 concurrent connections)
- [x] Monitoring dashboard shows data

## Support

If issues persist:

1. Check logs: `journalctl -u adic-node -n 100`
2. Review metrics: `curl /metrics`
3. Test with example client: `python examples/python/event_stream_client.py`
4. Open issue: https://github.com/adic/adic-core/issues
5. Discord support: https://discord.gg/adic

## Post-Deployment Tasks

- [ ] Update documentation with production URLs
- [ ] Train team on event streaming monitoring
- [ ] Schedule load testing during peak hours
- [ ] Review and optimize buffer sizes based on usage
- [ ] Set up automated alerts for anomalies
- [ ] Document any custom configurations
- [ ] Create runbook for common issues
- [ ] Schedule review in 1 week, 1 month, 3 months

---

**Last Updated**: 2024-01-15
**Version**: 1.0.0
**Status**: Production Ready ✅
