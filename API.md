# ADIC Core API Documentation

## Overview

The ADIC node provides a RESTful API for interacting with the ADIC-DAG network. All endpoints return JSON responses and follow standard HTTP status codes.

## Base URL

```
http://localhost:8080
```

## Authentication

Some endpoints require authentication via JWT tokens. Include the token in the `Authorization` header:

```
Authorization: Bearer <your-jwt-token>
```

## Rate Limiting

API requests are rate-limited to prevent abuse. Default limits:
- 100 requests per minute for unauthenticated requests
- 1000 requests per minute for authenticated requests

## Endpoints

### Health & Status

#### GET /health
Health check endpoint.

**Response:**
```
OK
```

#### GET /status
Get node status including network statistics and finality metrics.

**Response:**
```json
{
  "node_id": "hex_string",
  "version": "0.1.4",
  "network": {
    "peers": 5,
    "messages": 1000,
    "tips": 3
  },
  "finality": {
    "k_core_messages": 450,
    "f2_stabilized": 200
  }
}
```

### Message Operations

#### POST /submit
Submit a new message to the DAG.

**Authentication Required:** Yes

**Request Body:**
```json
{
  "content": "string",
  "features": {
    "axes": [
      {"axis": 0, "value": 42},
      {"axis": 1, "value": 100},
      {"axis": 2, "value": 7}
    ]
  }
}
```

**Response:**
```json
{
  "message_id": "hex_string"
}
```

#### GET /message/:id
Retrieve a specific message by ID.

**Parameters:**
- `id` (path) - Message ID in hex format

**Response:**
```json
{
  "id": "hex_string",
  "parents": ["hex_string", "hex_string"],
  "features": {
    "axes": [...]
  },
  "content": "base64_string",
  "timestamp": "2025-08-29T12:00:00Z",
  "signature": "hex_string"
}
```

#### GET /tips
Get current DAG tips (messages without children).

**Response:**
```json
{
  "tips": [
    {
      "id": "hex_string",
      "weight": 0.95
    }
  ]
}
```

### P-adic Operations

#### GET /ball/:axis/:radius
Get all messages within a p-adic ball.

**Parameters:**
- `axis` (path) - Axis index (0 to d-1)
- `radius` (path) - P-adic radius

**Response:**
```json
{
  "axis": 0,
  "radius": 3,
  "center": "hex_string",
  "messages": ["hex_string", ...]
}
```

### Proofs & Security

#### POST /proof/membership
Generate a p-adic ball membership proof.

**Request Body:**
```json
{
  "message_id": "hex_string",
  "axis": 0,
  "radius": 3
}
```

**Response:**
```json
{
  "proof": "base64_string",
  "valid_until": "2025-08-29T13:00:00Z"
}
```

#### POST /proof/verify
Verify a membership proof.

**Request Body:**
```json
{
  "proof": "base64_string",
  "message_id": "hex_string"
}
```

**Response:**
```json
{
  "valid": true,
  "details": "Proof valid for axis 0, radius 3"
}
```

#### GET /security/score/:id
Get the security score for a message.

**Parameters:**
- `id` (path) - Message ID

**Response:**
```json
{
  "message_id": "hex_string",
  "c1_score": 0.95,
  "c2_score": 0.88,
  "c3_score": 0.92,
  "overall": 0.91
}
```

### Finality & Consensus

#### GET /v1/finality/:id
Get finality artifact for a message.

**Parameters:**
- `id` (path) - Message ID

**Response:**
```json
{
  "message_id": "hex_string",
  "f1_finalized": true,
  "f1_k_core": 25,
  "f1_depth": 15,
  "f2_stabilized": false,
  "f2_homology_rank": 2
}
```

#### GET /v1/mrw/traces
Get Multi-axis Random Walk traces.

**Query Parameters:**
- `limit` (optional) - Maximum number of traces (default: 100)

**Response:**
```json
{
  "traces": [
    {
      "id": "hex_string",
      "path": ["hex_string", ...],
      "weight": 0.85,
      "timestamp": "2025-08-29T12:00:00Z"
    }
  ]
}
```

#### GET /v1/mrw/trace/:id
Get a specific MRW trace.

**Parameters:**
- `id` (path) - Trace ID

**Response:**
```json
{
  "id": "hex_string",
  "start": "hex_string",
  "end": "hex_string",
  "path": ["hex_string", ...],
  "axis_weights": [0.3, 0.3, 0.4]
}
```

### Reputation System

#### GET /v1/reputation/all
Get all reputation scores.

**Response:**
```json
{
  "reputations": {
    "pubkey_hex": 0.95,
    "pubkey_hex": 0.72
  }
}
```

#### GET /v1/reputation/:pubkey
Get reputation score for a specific public key.

**Parameters:**
- `pubkey` (path) - Public key in hex format

**Response:**
```json
{
  "public_key": "hex_string",
  "reputation": 0.95,
  "messages_approved": 450,
  "conflicts_resolved": 12,
  "last_updated": "2025-08-29T12:00:00Z"
}
```

### Conflict Resolution

#### GET /v1/conflicts
Get all active conflicts.

**Response:**
```json
{
  "conflicts": [
    {
      "id": "hex_string",
      "messages": ["hex_string", "hex_string"],
      "status": "pending",
      "created": "2025-08-29T12:00:00Z"
    }
  ]
}
```

#### GET /v1/conflict/:id
Get details of a specific conflict.

**Parameters:**
- `id` (path) - Conflict ID

**Response:**
```json
{
  "id": "hex_string",
  "conflicting_messages": ["hex_string", "hex_string"],
  "energy_scores": [0.45, 0.55],
  "resolution": "message_2",
  "resolved_at": "2025-08-29T12:30:00Z"
}
```

### Economics & Token Management

#### GET /v1/economics/supply
Get token supply metrics.

**Response:**
```json
{
  "total_supply": "1000000000000000000",
  "circulating_supply": "500000000000000000",
  "treasury_balance": "100000000000000000",
  "liquidity_balance": "50000000000000000",
  "genesis_balance": "250000000000000000",
  "burned_amount": "0",
  "emission_issued": "100000000000000000",
  "max_supply": "2000000000000000000",
  "genesis_supply": "1000000000000000000"
}
```

#### GET /v1/economics/balance/:address
Get balance for a specific address.

**Parameters:**
- `address` (path) - Account address

**Response:**
```json
{
  "address": "hex_string",
  "balance": "1000000000000000",
  "locked_balance": "100000000000000",
  "unlocked_balance": "900000000000000"
}
```

#### GET /v1/economics/balance
Get balance using query parameter.

**Query Parameters:**
- `address` - Account address

**Response:** Same as above

#### GET /v1/economics/emissions
Get emission schedule information.

**Response:**
```json
{
  "total_emitted": "100000000000000000",
  "current_rate": 0.05,
  "years_elapsed": 1.5,
  "projected_1_year": "105000000000000000",
  "projected_5_years": "127628156000000000",
  "projected_10_years": "162889463000000000"
}
```

#### GET /v1/economics/treasury
Get treasury information.

**Response:**
```json
{
  "balance": "100000000000000000",
  "active_proposals": [
    {
      "id": "hex_string",
      "recipient": "address",
      "amount": "1000000000000",
      "reason": "Development fund",
      "proposer": "address",
      "approvals": 3,
      "threshold_required": 5,
      "expires_at": 1735488000
    }
  ]
}
```

#### GET /v1/economics/genesis
Get genesis allocation status.

**Response:**
```json
{
  "allocated": true,
  "treasury_amount": "100000000000000000",
  "liquidity_amount": "50000000000000000",
  "genesis_amount": "250000000000000000",
  "timestamp": 1735401600
}
```

#### POST /v1/economics/initialize
Initialize genesis allocation (can only be called once).

**Authentication Required:** Yes (admin only)

**Response:**
```json
{
  "success": true,
  "message": "Genesis allocation completed"
}
```

#### GET /v1/economics/deposits
Get deposits summary.

**Response:**
```json
{
  "total_escrowed": "10000000000000",
  "active_deposits": 45,
  "refunded_amount": "2000000000000"
}
```

#### GET /v1/economics/deposit/:id
Get deposit status for a message.

**Parameters:**
- `id` (path) - Message ID

**Response:**
```json
{
  "message_id": "hex_string",
  "amount": "100000000000",
  "status": "escrowed",
  "depositor": "pubkey_hex",
  "escrowed_at": "2025-08-29T12:00:00Z",
  "refundable_at": "2025-08-29T13:00:00Z"
}
```

### Statistics & Monitoring

#### GET /v1/statistics/detailed
Get detailed node statistics.

**Response:**
```json
{
  "node": {
    "uptime": 3600,
    "version": "0.1.4",
    "memory_usage": 1024000,
    "cpu_usage": 0.15
  },
  "network": {
    "peers_connected": 5,
    "messages_processed": 10000,
    "messages_per_second": 15.5,
    "bandwidth_in": 1024000,
    "bandwidth_out": 512000
  },
  "consensus": {
    "tips_count": 3,
    "k_core_size": 450,
    "conflicts_active": 2,
    "conflicts_resolved": 98
  },
  "storage": {
    "messages_stored": 100000,
    "database_size": 52428800,
    "snapshots_created": 5
  }
}
```

#### GET /metrics
Prometheus-compatible metrics endpoint.

**Response:**
```
# HELP adic_messages_submitted_total Total messages submitted
# TYPE adic_messages_submitted_total counter
adic_messages_submitted_total 10000

# HELP adic_messages_processed_total Total messages processed
# TYPE adic_messages_processed_total counter
adic_messages_processed_total 9950

# HELP adic_peers_connected Current number of connected peers
# TYPE adic_peers_connected gauge
adic_peers_connected 5

# ... additional metrics
```

## Error Responses

All endpoints may return error responses with the following format:

```json
{
  "error": "Error description",
  "code": "ERROR_CODE",
  "details": "Additional information"
}
```

### Common Status Codes

- `200 OK` - Request successful
- `201 Created` - Resource created successfully
- `400 Bad Request` - Invalid request parameters
- `401 Unauthorized` - Authentication required
- `403 Forbidden` - Insufficient permissions
- `404 Not Found` - Resource not found
- `429 Too Many Requests` - Rate limit exceeded
- `500 Internal Server Error` - Server error

## WebSocket API

For real-time updates, connect to the WebSocket endpoint:

```
ws://localhost:8080/ws
```

### Events

- `message.new` - New message added to DAG
- `tips.updated` - Tips have changed
- `finality.achieved` - Message achieved finality
- `conflict.detected` - New conflict detected
- `peer.connected` - New peer connected
- `peer.disconnected` - Peer disconnected

### Example WebSocket Client

```javascript
const ws = new WebSocket('ws://localhost:8080/ws');

ws.on('message', (data) => {
  const event = JSON.parse(data);
  console.log('Event:', event.type, event.data);
});

// Subscribe to specific events
ws.send(JSON.stringify({
  action: 'subscribe',
  events: ['message.new', 'tips.updated']
}));
```

## Examples

### Submit a Message with cURL

```bash
curl -X POST http://localhost:8080/submit \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -d '{
    "content": "Hello ADIC-DAG!",
    "features": {
      "axes": [
        {"axis": 0, "value": 42},
        {"axis": 1, "value": 100},
        {"axis": 2, "value": 7}
      ]
    }
  }'
```

### Get Node Status

```bash
curl http://localhost:8080/status | jq '.'
```

### Check Balance

```bash
curl http://localhost:8080/v1/economics/balance/YOUR_ADDRESS | jq '.'
```

### Monitor Metrics

```bash
curl http://localhost:8080/metrics | grep adic_messages
```

## SDK Support

Official SDKs are planned for:
- JavaScript/TypeScript
- Python
- Rust
- Go

## Changelog

### v0.1.4
- Added comprehensive economics endpoints
- Improved error responses
- Added WebSocket support for real-time updates

### v0.1.3
- Initial API implementation
- Basic authentication and rate limiting
- Core endpoints for message submission and retrieval