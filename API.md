# ADIC Core API Documentation

## Overview

The ADIC node provides a RESTful API for interacting with the ADIC-DAG network. All endpoints return JSON responses and follow standard HTTP status codes.

> **📋 For comprehensive integration status and Explorer Backend compatibility, see [INTEGRATION-STATUS.md](./INTEGRATION-STATUS.md)**

## Implementation Framework

**Note**: API code samples in this documentation are illustrative and show expected request/response patterns. The actual implementation uses **Axum's router builder pattern** with Rust handlers. Code samples represent the conceptual API structure, not literal implementation code.

## Data Normalization Notes

**Timestamp Handling**: All timestamps are returned as Unix milliseconds (UTC). Clients should convert to their local timezone and handle potential precision loss when converting to seconds-based systems.

**ID Transforms**: Message IDs and public keys use consistent hexadecimal encoding (lowercase). Clients should validate hex format and length before processing to prevent parsing errors and off-by-one indexing issues.

## Base URL

```
http://localhost:8080
```

## Versioning

The API provides both versioned and unversioned endpoints:
- **Root endpoints**: `/health`, `/status`, `/submit`, `/message/:id`, `/tips` (legacy compatibility)
- **Versioned endpoints**: `/v1/*` (recommended for new integrations)

Future versions may consolidate all endpoints under `/v1` for consistency.

## Authentication

Some endpoints require authentication via JWT tokens. Include the token in the `Authorization` header:

```
Authorization: Bearer <your-jwt-token>
```

## Rate Limiting

**Current Implementation**: In-process flat rate limiter (100 requests/minute by default)

**Default Limits:**
- 100 requests per minute for all clients (flat rate)
- Applied per IP address using middleware

**Recommended Production Setup**: 
- Deploy auth-aware tiering at gateway level (Cloudflare, HAProxy, etc.)
- Higher limits for authenticated clients based on auth claims
- Consider extending middleware to read auth claims and set per-user limits

**Headers Returned:**
- `X-RateLimit-Limit`: Request limit per window
- `X-RateLimit-Remaining`: Requests remaining in current window  
- `X-RateLimit-Reset`: Time when the rate limit resets (Unix timestamp)

## Endpoints

### Health & Status

#### GET /v1/health **[IMPLEMENTED]**
Health check endpoint.

**Response:**
```
OK
```

#### GET /v1/status **[IMPLEMENTED]**
Get node status including network statistics and finality metrics.

**Response:**
```json
{
  "node_id": "a1b2c3d4e5f6789...",
  "version": "0.3.0",
  "capabilities": {
    "sse_streaming": false,
    "websocket": false,
    "bulk_queries": true,
    "weights_in_tips": false,
    "versioned_api": true
  },
  "network": {
    "peers": 5,
    "messages": 1000,
    "tips": 3
  },
  "finality": {
    "k_core_messages": 450,
    "homology_stabilized": 200
  }
}
```

**Fields:**
- `node_id`: Node's public key in hex format (no 0x prefix)
- `version`: Crate version from Cargo.toml
- `capabilities`: Feature flags for client capability detection
- `network`: Basic network counters (peers, total messages, current tips)
- `finality`: Finality metrics (k-core finalized count, homology stabilized count)

**Capability Flags:**
- `sse_streaming`: Server-Sent Events support for `/v1/stream/events` (currently false)
- `websocket`: WebSocket support (always false for nodes; Explorer Backend provides WebSocket)
- `bulk_queries`: Bulk endpoints available (`/v1/messages/bulk`, `/v1/messages/range`, `/v1/messages/since/:id`)
- `weights_in_tips`: Weighted tips with MRW scores (currently false - future enhancement)
- `versioned_api`: API versioning support (true)

### Network Operations

#### GET /v1/network/peers **[IMPLEMENTED]**
Get list of connected peers.

**Response:**
```json
{
  "peers": [
    {
      "id": "peer_id_hex",
      "address": "192.168.1.100:19000",
      "connected_since": "2025-01-15T10:30:00Z",
      "messages_received": 150,
      "messages_sent": 200,
      "latency_ms": 45
    }
  ],
  "count": 1
}
```

#### GET /v1/network/status **[IMPLEMENTED]**
Get detailed network status information.

**Response:**
```json
{
  "status": "healthy",
  "peers_connected": 5,
  "inbound_connections": 3,
  "outbound_connections": 2,
  "network_bandwidth": {
    "upload_bytes": 1048576,
    "download_bytes": 2097152
  },
  "last_message_received": "2025-01-15T12:30:00Z"
}
```

### Message Operations

#### POST /v1/messages **[IMPLEMENTED]**
Submit a new message to the DAG.

**Authentication Required:** Yes

**Request Body:**
```json
{
  "content": "string",
  "features": {
    "axes": [
      {"axis": 0, "value": 42},  // Simplified format for submission
      {"axis": 1, "value": 100},
      {"axis": 2, "value": 7}
    ]
  }
}
```

**Note:** The node currently derives features from content; provided features are ignored for now and may be used in future versions.

**Response:**
```json
{
  "message_id": "hex_string"
}
```

#### GET /v1/messages/:id **[IMPLEMENTED]**
Retrieve a specific message by ID.

**Parameters:**
- `id` (path) - Message ID in hex format

**Response:**
```json
{
  "id": "hex_string",
  "parents": ["hex_string", "hex_string"],
  "features": {
    "axes": [
      {
        "axis": 0,
        "p": 3,
        "digits": [1, 2, 0, 1, 2]  // LSB-first p-adic digits
      },
      {
        "axis": 1,
        "p": 3,
        "digits": [2, 0, 1, 0, 1]
      }
    ]
  },
  "content": "base64_string",
  "timestamp": "2025-08-29T12:00:00Z",
  "signature": "hex_string"
}
```

#### GET /v1/tips **[IMPLEMENTED]**
Get current DAG tips (messages without children).

**Response (Current Implementation):**
```json
{
  "tips": ["hex_string", "hex_string", "hex_string"]
}
```

**Response (Future Enhancement with Weights):**
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

#### GET /v1/balls/:axis/:radius/:center
Get all messages within a p-adic ball.

**Parameters:**
- `axis` (path) - Axis index (0 to d-1)
- `radius` (path) - P-adic radius (must match length of center)
- `center` (path) - Hex-encoded ball center (p-adic features)

**Response:**
```json
{
  "axis": 0,
  "radius": 3,
  "center": "hex_string",
  "message_count": 5,
  "messages": ["hex_string", ...]
}
```

### Proofs & Security

#### POST /v1/proofs/membership
Generate a p-adic ball membership proof for a message.

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
  "message_id": "hex_string",
  "axis": 0,
  "radius": 3,
  "ball_id": "hex_string",
  "proof": {
    "type": "ball_membership",
    "features": {
      "p": 3,
      "digits": [1, 2, 0, 1]
    },
    "ball_id": "hex_string",
    "verification": "Ball ID computed from message p-adic features"
  },
  "verified": true
}
```

#### POST /v1/proofs/verify
Verify a p-adic ball membership proof.

**Request Body:**
```json
{
  "proof": {
    "type": "ball_membership",
    "features": {
      "p": 3,
      "digits": [1, 2, 0, 1]
    },
    "ball_id": "hex_string"
  }
}
```

**Response:**
```json
{
  "verified": true,
  "proof_type": "ball_membership",
  "details": "Proof valid: computed ball ID matches claimed ball ID",
  "computed_ball_id": "hex_string",
  "claimed_ball_id": "hex_string"
}
```

#### GET /v1/security/score/:id
Get the security score and admissibility check for a message.

**Parameters:**
- `id` (path) - Message ID (hex-encoded)

**Response:**
```json
{
  "message_id": "hex_string",
  "is_admissible": true,
  "admissibility_score": 0.95,
  "c1_score": 0.95,
  "c2_score": 1.0,
  "c3_score": 1.0,
  "overall": 0.98,
  "details": "Admissibility check details"
}
```

### Finality & Consensus

#### GET /v1/finality/:id **[IMPLEMENTED]**
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

#### GET /v1/mrw/traces **[IMPLEMENTED]**
Get Multi-axis Random Walk traces.

**Query Parameters:**
- `limit` (optional) - Maximum number of traces (default: 10, max: 50)

**Response:**
```json
{
  "traces": [
    {
      "id": "mrw_1693123456789",
      "success": true,
      "parent_count": 2,
      "step_count": 5,
      "duration_ms": 45,
      "widen_count": 0,
      "candidates_considered": 12
    }
  ],
  "total": 1
}
```

**Note:** Fields are evolving and will stabilize. Current implementation returns execution metrics rather than path details.

#### GET /v1/mrw/trace/:id **[IMPLEMENTED]**
Get a specific MRW trace by ID.

**Parameters:**
- `id` (path) - Trace ID (e.g., "mrw_1693123456789")

**Response:**
Returns the full trace object with detailed execution information.

**Note:** Response format is evolving. Currently returns the internal trace structure with execution details and selected parents.

### Reputation System

#### GET /v1/reputation **[IMPLEMENTED]**
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

#### GET /v1/reputation/:pubkey **[IMPLEMENTED]**
Get reputation score for a specific public key.

**Parameters:**
- `pubkey` (path) - Public key in hex format

**Response:**
```json
{
  "public_key": "hex_string",
  "reputation": 0.95,
  "messages_approved": 450,
  "last_updated": "2025-08-29T12:00:00Z"
}
```

**Note:** `conflicts_resolved` field not yet implemented - currently returns 0 or is omitted.

### Conflict Resolution

#### GET /v1/conflicts **[IMPLEMENTED]**
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

#### GET /v1/conflicts/:id **[IMPLEMENTED]**
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

#### GET /v1/economics/supply **[IMPLEMENTED]**
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

#### GET /v1/economics/balance/:address **[IMPLEMENTED]**
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

#### GET /v1/economics/balance **[IMPLEMENTED]**
Get balance using query parameter.

**Query Parameters:**
- `address` - Account address

**Response:** Same as above

#### GET /v1/economics/emissions **[IMPLEMENTED]**
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

#### GET /v1/economics/treasury **[IMPLEMENTED]**
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

#### GET /v1/economics/genesis **[IMPLEMENTED]**
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

#### POST /v1/economics/initialize **[IMPLEMENTED]**
Initialize genesis allocation (can only be called once).

**Authentication Required:** Yes (admin only)

**Response:**
```json
{
  "success": true,
  "message": "Genesis allocation completed"
}
```

#### GET /v1/deposits
Get deposits summary and recent deposit history.

**Response:**
```json
{
  "summary": {
    "escrowed_count": 45,
    "total_escrowed": 10000000000000,
    "slashed_count": 2,
    "total_slashed": 200000000000,
    "refunded_count": 10,
    "total_refunded": 1000000000000
  },
  "recent_deposits": [
    {
      "message_id": "hex_string",
      "proposer": "pubkey_hex",
      "amount": 100000000000,
      "status": "Escrowed",
      "timestamp": "2025-08-29T12:00:00Z"
    }
  ]
}
```

#### GET /v1/deposits/:id
Get deposit status for a specific message.

**Parameters:**
- `id` (path) - Message ID (hex-encoded)

**Response:**
```json
{
  "message_id": "hex_string",
  "proposer": "pubkey_hex",
  "amount": 100000000000,
  "status": "Escrowed",
  "timestamp": "2025-08-29T12:00:00Z",
  "slashed": false,
  "refunded": false
}
```

### Statistics & Monitoring

#### GET /v1/statistics **[IMPLEMENTED]**
Get detailed node statistics.

**Response:**
```json
{
  "node": {
    "uptime": 3600,
    "version": "0.3.0",
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

#### GET /v1/metrics **[IMPLEMENTED]**
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

### Advanced Metrics

#### GET /v1/diversity **[IMPLEMENTED]**
Get diversity metrics across DAG axes.

**Response:**
```json
{
  "axes": [
    {
      "axis": 0,
      "diversity_score": 0.85,
      "coverage": 0.92,
      "message_distribution": {
        "min": 10,
        "max": 150,
        "mean": 75.5,
        "stddev": 25.3
      }
    },
    {
      "axis": 1,
      "diversity_score": 0.78,
      "coverage": 0.88,
      "message_distribution": {
        "min": 15,
        "max": 140,
        "mean": 70.2,
        "stddev": 22.1
      }
    }
  ],
  "overall_diversity": 0.82,
  "timestamp": "2025-01-20T12:00:00Z"
}
```

**Example:**
```bash
curl http://localhost:8080/v1/diversity | jq '.'
```

#### GET /v1/energy **[IMPLEMENTED]**
Get active energy descent paths and metrics.

**Response:**
```json
{
  "active_paths": [
    {
      "message_id": "hex_string",
      "current_energy": 0.75,
      "initial_energy": 1.0,
      "descent_steps": 5,
      "stabilized": false,
      "last_updated": "2025-01-20T12:00:00Z"
    }
  ],
  "total_active": 12,
  "average_energy": 0.65,
  "convergence_rate": 0.92
}
```

**Example:**
```bash
curl http://localhost:8080/v1/energy | jq '.'
```

#### GET /v1/finality/kcore/metrics **[IMPLEMENTED]**
Get k-core finality metrics and thresholds.

**Response:**
```json
{
  "k_value": 20,
  "current_k_core_size": 450,
  "messages_finalized_count": 445,
  "finalization_rate": 0.95,
  "average_finalization_depth": 15.5,
  "pending_messages": 5,
  "metrics": {
    "min_depth": 12,
    "max_depth": 20,
    "median_depth": 15
  }
}
```

**Example:**
```bash
curl http://localhost:8080/v1/finality/kcore/metrics | jq '.'
```

#### GET /v1/admissibility **[IMPLEMENTED]**
Get admissibility check statistics and rates.

**Response:**
```json
{
  "total_checks": 10000,
  "passed": 9500,
  "failed": 500,
  "pass_rate": 0.95,
  "recent_checks": [
    {
      "message_id": "hex_string",
      "passed": true,
      "timestamp": "2025-01-20T12:00:00Z",
      "reasons": []
    },
    {
      "message_id": "hex_string",
      "passed": false,
      "timestamp": "2025-01-20T11:59:00Z",
      "reasons": ["insufficient_diversity", "low_reputation"]
    }
  ],
  "failure_reasons": {
    "insufficient_diversity": 200,
    "low_reputation": 150,
    "invalid_parents": 100,
    "other": 50
  }
}
```

**Example:**
```bash
curl http://localhost:8080/v1/admissibility | jq '.'
```

### Update System Endpoints

#### GET /v1/updates/swarm **[IMPLEMENTED]**
Get real-time swarm-wide update statistics.

**Response:**
```json
{
  "success": true,
  "swarm": {
    "total_download_speed": 52428800,
    "total_upload_speed": 104857600,
    "downloading_peers": 12,
    "seeding_peers": 45,
    "idle_peers": 8,
    "total_active_transfers": 24,
    "average_download_progress": 67.5,
    "version_distribution": {
      "0.1.7": 45,
      "0.1.8": 20
    },
    "total_peers": 65,
    "download_speed_mbps": 50.0,
    "upload_speed_mbps": 100.0
  }
}
```

#### GET /v1/updates/status **[IMPLEMENTED]**
Get current node update status.

**Response:**
```json
{
  "current_version": "0.1.7",
  "latest_version": "0.1.8",
  "update_available": true,
  "update_state": "idle",
  "auto_update_enabled": false,
  "last_check": "2024-12-28T12:00:00Z",
  "next_check": "2024-12-28T13:00:00Z"
}
```

#### POST /v1/updates/check **[IMPLEMENTED]**
Manually trigger an update check.

**Response:**
```json
{
  "success": true,
  "message": "Update check initiated",
  "update_available": true,
  "latest_version": "0.1.8",
  "current_version": "0.1.7"
}
```

#### GET /v1/updates/progress **[IMPLEMENTED]**
Get detailed update download progress.

**Response:**
```json
{
  "status": "downloading",
  "version": "0.1.8",
  "progress_percent": 45.2,
  "chunks_received": 23,
  "total_chunks": 51,
  "download_speed": 2097152,
  "upload_speed": 524288,
  "eta_seconds": 120,
  "peers_connected": 8,
  "verification_status": "pending"
}
```

#### POST /v1/updates/apply **[IMPLEMENTED]**
Apply a downloaded update (restart required).

**Authentication Required:** Yes (admin only)

**Response:**
```json
{
  "success": true,
  "message": "Update applied successfully. Restart required.",
  "version": "0.1.8",
  "restart_required": true
}
```

**Note:** This endpoint triggers the application of a downloaded update. The node must be restarted for the update to take effect.

### Bulk Query Endpoints

#### GET /v1/messages/bulk **[IMPLEMENTED]**
Query multiple messages in a single request for efficient indexing.

**Query Parameters:**
- `ids` - Comma-separated list of message IDs (max 1000)

**Response:**
```json
{
  "messages": [
    { /* full message object */ },
    { /* full message object */ }
  ],
  "not_found": ["id3", "id4"]
}
```

#### GET /v1/messages/range
Get messages within a time range with pagination support.

**Query Parameters:**
- `start` - ISO8601 timestamp (required)
- `end` - ISO8601 timestamp (required)
- `limit` - Max messages (default 1000, max 1000)
- `cursor` - Pagination cursor (optional)

**Response:**
```json
{
  "messages": [ /* array of messages */ ],
  "next_cursor": "cursor_string",
  "has_more": true
}
```

#### GET /v1/messages/since/:id
Get all messages since a specific checkpoint for incremental sync.

**Parameters:**
- `id` (path) - Checkpoint message ID (hex-encoded)

**Query Parameters:**
- `limit` - Max messages (default 1000, max 1000)

**Response:**
```json
{
  "messages": [ /* messages since checkpoint */ ],
  "count": 150
}
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

## Real-time Updates

**Note:** WebSocket support is not implemented in the core node. Real-time updates are available through:

1. **Server-Sent Events (SSE)** - For lightweight streaming from the node (planned)
2. **Explorer WebSocket API** - Full WebSocket support is provided by the separate Explorer backend service

The Explorer backend (separate from this node) provides comprehensive WebSocket support for:
- Real-time message updates
- Live finality notifications
- Network statistics streaming
- Custom subscriptions and filters

For Explorer WebSocket documentation, see the Explorer API documentation.

## Examples

### Submit a Message with cURL

```bash
curl -X POST http://localhost:8080/v1/messages \
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
curl http://localhost:8080/v1/status | jq '.'
```

### Check Balance

```bash
curl http://localhost:8080/v1/economics/balance/YOUR_ADDRESS | jq '.'
```

### Wallet Operations

#### Get Wallet Info
```bash
curl http://localhost:8080/v1/wallets/self | jq '.'
```

#### Check Wallet Balance
```bash
curl http://localhost:8080/v1/wallets/ADDRESS/balance | jq '.'
```

#### Request from Faucet
```bash
curl -X POST http://localhost:8080/v1/wallets/faucet \
  -H "Content-Type: application/json" \
  -d '{
    "address": "YOUR_ADDRESS",
    "amount": 1000.0
  }' | jq '.'
```

#### Sign Message
```bash
curl -X POST http://localhost:8080/v1/wallets/self/sign \
  -H "Content-Type: application/json" \
  -d '{
    "message": "Message to sign"
  }' | jq '.'
```

#### Get Transaction History
```bash
curl http://localhost:8080/v1/wallets/ADDRESS/transactions | jq '.'
```

**Note:** The `/v1/wallets/:address/transactions` endpoint accepts addresses in both formats:
- **Bech32 format** (recommended): `adic14ua43vrnw3pjm8x60fwgva3l2s0lnd3e39q6d0nujnr4fx469nhqhzuuff`
- **Hex format**: `a72ea59cd73d08fad1bb0a6b1af2e1dd9c42a3b1e11d95fa731aa5f0b52fd7c1`

The same applies to `/v1/wallets/:address/balance` endpoint.

### Wallet Registry

#### POST /v1/wallets/registry **[IMPLEMENTED]**
Register a wallet with optional metadata.

**Request Body:**
```json
{
  "address": "hex_string",
  "metadata": {
    "label": "My Main Wallet",
    "type": "validator"
  }
}
```

**Response:**
```json
{
  "success": true,
  "address": "hex_string",
  "registered_at": "2025-01-15T12:00:00Z"
}
```

**Example:**
```bash
curl -X POST http://localhost:8080/v1/wallets/registry \
  -H "Content-Type: application/json" \
  -d '{
    "address": "YOUR_ADDRESS",
    "metadata": {
      "label": "Validator Node 1"
    }
  }' | jq '.'
```

#### GET /v1/wallets/registry **[IMPLEMENTED]**
Get list of all registered wallets.

**Response:**
```json
{
  "wallets": [
    {
      "address": "hex_string",
      "registered_at": "2025-01-15T12:00:00Z",
      "metadata": {
        "label": "My Main Wallet"
      }
    }
  ],
  "count": 1
}
```

**Example:**
```bash
curl http://localhost:8080/v1/wallets/registry | jq '.'
```

#### GET /v1/wallets/registry/stats **[IMPLEMENTED]**
Get wallet registry statistics.

**Response:**
```json
{
  "total_registered": 150,
  "active_wallets": 120,
  "validator_wallets": 45,
  "total_balance": "15000000000000000"
}
```

**Example:**
```bash
curl http://localhost:8080/v1/wallets/registry/stats | jq '.'
```

#### GET /v1/wallets/:address **[IMPLEMENTED]**
Get detailed information about a specific wallet.

**Parameters:**
- `address` (path) - Wallet address in hex format

**Response:**
```json
{
  "address": "hex_string",
  "balance": "1000000000000000",
  "registered": true,
  "registered_at": "2025-01-15T12:00:00Z",
  "metadata": {
    "label": "My Main Wallet"
  },
  "transaction_count": 50,
  "last_activity": "2025-01-20T15:30:00Z"
}
```

**Example:**
```bash
curl http://localhost:8080/v1/wallets/YOUR_ADDRESS | jq '.'
```

#### POST /v1/wallets/self/export **[IMPLEMENTED]**
Export wallet data (encrypted).

**Authentication Required:** Yes

**Response:**
```json
{
  "success": true,
  "export_data": "encrypted_base64_string",
  "exported_at": "2025-01-20T12:00:00Z"
}
```

**Example:**
```bash
curl -X POST http://localhost:8080/v1/wallets/self/export \
  -H "Authorization: Bearer YOUR_TOKEN" | jq '.'
```

#### POST /v1/wallets/self/import **[IMPLEMENTED]**
Import wallet data from encrypted export.

**Authentication Required:** Yes

**Request Body:**
```json
{
  "export_data": "encrypted_base64_string"
}
```

**Response:**
```json
{
  "success": true,
  "address": "hex_string",
  "imported_at": "2025-01-20T12:00:00Z"
}
```

**Example:**
```bash
curl -X POST http://localhost:8080/v1/wallets/self/import \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -d '{
    "export_data": "YOUR_ENCRYPTED_DATA"
  }' | jq '.'
```

### Monitor Metrics

```bash
curl http://localhost:8080/v1/metrics | grep adic_messages
```

## SDK Support

Official SDKs are planned for:
- JavaScript/TypeScript
- Python
- Rust
- Go

## Changelog

### v0.3.0
- Phase 2 complete: governance, storage market, PoUW framework integrated
- Added BLS threshold cryptography support for decentralized governance
- Enhanced security with libp2p hardening (67% feature reduction, custom DHT protocol)
- No new API endpoints (focus on internal improvements and security hardening)

### v0.1.8
- Added genesis system endpoints (GET /v1/economics/genesis, POST /v1/economics/initialize)
- Added network operation endpoints (GET /peers, GET /network/status)
- Added wallet registry endpoints (POST /wallet/register, GET /wallet/registered, GET /wallet/registry/stats, GET /wallet/info/:address)
- Added wallet import/export endpoints (POST /wallet/export, POST /wallet/import)
- Added advanced metrics endpoints (GET /v1/diversity/stats, GET /v1/energy/active, GET /v1/finality/kcore/metrics, GET /v1/admissibility/rates)
- Added update apply endpoint (POST /update/apply)
- Updated version references to 0.1.8
- Enhanced API documentation with complete endpoint coverage

### v0.1.7
- Added update system endpoints (GET /update/swarm, GET /update/status, POST /update/check, GET /update/progress)
- Improved peer-to-peer update distribution
- Enhanced swarm-wide statistics tracking
- Added version distribution metrics

### v0.1.6
- Added bulk query endpoints (GET /v1/messages/bulk, GET /v1/messages/range, GET /v1/messages/since/:id)
- Enhanced capability detection in /status endpoint
- Improved pagination support for large queries
- Added Explorer Backend compatibility

### v0.1.5
- Added complete wallet implementation with transaction support
- Added wallet API endpoints for transfers, faucet, and signing
- Added transaction history tracking
- Enhanced message submission with deposit checking
- Added energy descent tracking endpoints
- Added finalization metrics endpoints

### v0.1.4
- Added comprehensive economics endpoints
- Improved error responses
- Planned SSE support for real-time updates

### v0.1.3
- Initial API implementation
- Basic authentication and rate limiting
- Core endpoints for message submission and retrieval