# ADIC Core Logging Best Practices

## Introduction

This guide provides best practices for implementing and using the structured logging system in ADIC Core. Following these guidelines ensures consistent, useful, and performant logging across the codebase.

## Core Principles

### 1. Log State Changes, Not Actions

❌ **Bad**: Logging that you're about to do something
```rust
info!("Starting balance transfer");
balance.transfer(from, to, amount)?;
info!("Completed balance transfer");
```

✅ **Good**: Log the actual state change with before/after values
```rust
let balance_before = balance.get(from)?;
balance.transfer(from, to, amount)?;
let balance_after = balance.get(from)?;
info!(
    from = %from,
    to = %to,
    amount = amount.to_adic(),
    balance_before = balance_before.to_adic(),
    balance_after = balance_after.to_adic(),
    "💸 Balance transferred"
);
```

### 2. Use Structured Fields for Context

❌ **Bad**: Embedding data in the message string
```rust
info!("Peer {} connected with reputation {}", peer_id, reputation);
```

✅ **Good**: Using structured fields
```rust
info!(
    peer_id = %peer_id,
    reputation = reputation,
    "🤝 Peer connected"
);
```

### 3. Keep Messages Concise

The message should describe WHAT happened in 2-5 words. Details go in fields.

❌ **Bad**: Long, detailed messages
```rust
info!("Successfully validated message {} from peer {} with score {} after checking {} parents",
    message_id, peer_id, score, parent_count);
```

✅ **Good**: Concise message with structured fields
```rust
info!(
    message_id = %message_id,
    peer_id = %peer_id,
    score = score,
    parent_count = parent_count,
    "✅ Message validated"
);
```

## Log Levels Guide

### ERROR - System Failures

Use ERROR for unrecoverable failures that require immediate attention:

```rust
error!(
    error = %e,
    peer_id = %peer_id,
    retry_count = retries,
    "Connection failed permanently"
);
```

### WARN - Degraded Operations

Use WARN for recoverable issues or suspicious behavior:

```rust
warn!(
    peer_id = %peer_id,
    invalid_messages = count,
    threshold = max_invalid,
    "⚠️ Peer misbehavior detected"
);
```

### INFO - State Changes

Use INFO for significant state changes and major operations:

```rust
info!(
    height_before = old_height,
    height_after = new_height,
    "📦 Blockchain height updated"
);
```

### DEBUG - Operation Flow

Use DEBUG for detailed operation flow and intermediate states:

```rust
debug!(
    message_id = %msg_id,
    validation_steps = steps.len(),
    "Processing validation pipeline"
);
```

### TRACE - Verbose Data

Use TRACE for very detailed debugging information:

```rust
trace!(
    raw_bytes = ?bytes,
    decoded_value = ?value,
    "Decoding network packet"
);
```

## Field Naming Conventions

Use consistent field names across the codebase:

| Field Pattern | Example | Usage |
|--------------|---------|-------|
| `*_before` | `balance_before` | State before change |
| `*_after` | `balance_after` | State after change |
| `*_count` | `peer_count` | Quantities |
| `*_id` | `message_id` | Identifiers |
| `*_ms` | `duration_ms` | Time in milliseconds |
| `*_bytes` | `size_bytes` | Size in bytes |
| `is_*` | `is_valid` | Boolean flags |
| `has_*` | `has_parent` | Boolean existence |

## Performance Guidelines

### 1. Lazy Evaluation

Fields are only evaluated when the log level is active:

```rust
// This expensive computation only runs at DEBUG level
debug!(
    expensive_metric = calculate_complex_metric(),
    "Metric calculated"
);
```

### 2. Avoid Allocations in Hot Paths

Use references and display implementations:

```rust
// Good: Uses Display trait, no allocation
info!(peer_id = %peer_id, "Peer added");

// Avoid: Creates String unnecessarily
info!(peer_id = format!("{}", peer_id), "Peer added");
```

### 3. Rate Limiting

For high-frequency events, use sampling or rate limiting:

```rust
use std::sync::atomic::{AtomicUsize, Ordering};

static MESSAGE_COUNT: AtomicUsize = AtomicUsize::new(0);

// Only log every 1000th message
let count = MESSAGE_COUNT.fetch_add(1, Ordering::Relaxed);
if count % 1000 == 0 {
    info!(
        total_messages = count,
        "Message processing milestone"
    );
}
```

## Common Patterns

### Pattern 1: Operation with Timing

```rust
let start = Instant::now();
let result = expensive_operation()?;
let elapsed = start.elapsed();

info!(
    duration_ms = elapsed.as_millis(),
    result_size = result.len(),
    "🎯 Operation completed"
);
```

### Pattern 2: State Transition

```rust
let old_state = entity.state.clone();
entity.transition_to(new_state)?;

info!(
    entity_id = %entity.id,
    state_before = ?old_state,
    state_after = ?entity.state,
    "🔄 State transitioned"
);
```

### Pattern 3: Batch Operations

```rust
let total = items.len();
let mut processed = 0;
let mut failed = 0;

for item in items {
    match process(item) {
        Ok(_) => processed += 1,
        Err(e) => {
            failed += 1;
            debug!(error = %e, item = ?item, "Item failed");
        }
    }
}

info!(
    total = total,
    processed = processed,
    failed = failed,
    success_rate = (processed as f64 / total as f64),
    "📦 Batch processed"
);
```

### Pattern 4: Cache Operations

```rust
let cache_hit = cache.get(key).is_some();
let value = if cache_hit {
    cache.get(key).unwrap()
} else {
    let value = compute_value(key)?;
    cache.insert(key, value.clone());
    value
};

debug!(
    key = %key,
    cache_hit = cache_hit,
    cache_size = cache.len(),
    "Cache accessed"
);
```

## Module-Specific Guidelines

### Economics Module

Always include amounts in ADIC units:
```rust
info!(
    amount_adic = amount.to_adic(),  // Not raw units
    "💰 Amount processed"
);
```

### Network Module

Include peer context for network operations:
```rust
info!(
    peer_id = %peer_id,
    remote_addr = %addr,
    direction = if outgoing { "outgoing" } else { "incoming" },
    "🔗 Connection established"
);
```

### Consensus Module

Include validation details:
```rust
info!(
    message_id = %msg_id,
    score = score,
    threshold = required_score,
    passed = score >= required_score,
    "🗳️ Consensus check"
);
```

### Storage Module

Include I/O metrics:
```rust
info!(
    key = %key,
    size_bytes = value.len(),
    operation = "write",
    backend = "rocksdb",
    "💾 Storage operation"
);
```

## Testing Logs

### Development Testing

```bash
# See all structured fields
RUST_LOG=debug cargo test

# Focus on specific module
RUST_LOG=adic_consensus=trace cargo test consensus_tests

# Disable test harness output
RUST_LOG=debug cargo test -- --nocapture
```

### Production Testing

```bash
# JSON output for parsing
RUST_LOG=info cargo run -- --log-format json

# File output with rotation
cargo run -- --log-file /var/log/adic/test.log --log-rotation 100
```

## Debugging Tips

### 1. Finding State Changes

Look for before/after patterns:
```bash
grep "balance_before\|balance_after" adic.log
```

### 2. Tracking Specific Entity

Use structured field filtering:
```bash
grep "peer_id=12D3KooW" adic.log
```

### 3. Performance Analysis

Extract timing fields:
```bash
grep -o "duration_ms=[0-9]*" adic.log | cut -d= -f2 | stats
```

### 4. Error Investigation

Find error context:
```bash
grep -B5 -A5 "ERROR\|WARN" adic.log
```

## Anti-Patterns to Avoid

### ❌ Don't Log Sensitive Data

Never log private keys, passwords, or PII:
```rust
// NEVER DO THIS
error!(private_key = %key, "Key error");

// Safe alternative
error!(key_id = %key.public_id(), "Key error");
```

### ❌ Don't Use println! or dbg!

These bypass the logging system:
```rust
// Bad
println!("Debug: {}", value);
dbg!(value);

// Good
debug!(value = ?value, "Debug output");
```

### ❌ Don't Log in Tight Loops

Avoid logging in performance-critical loops:
```rust
// Bad
for i in 0..1_000_000 {
    debug!(iteration = i, "Processing");
    process(i);
}

// Good
for i in 0..1_000_000 {
    process(i);
}
debug!(total_processed = 1_000_000, "Batch complete");
```

### ❌ Don't Duplicate Information

Avoid logging the same information multiple times:
```rust
// Bad
info!(peer_id = %peer, "Starting handshake");
info!(peer_id = %peer, "Handshake step 1");
info!(peer_id = %peer, "Handshake step 2");

// Good - log once with all context
info!(
    peer_id = %peer,
    steps_completed = 2,
    "Handshake completed"
);
```

## Monitoring Integration

### Prometheus Metrics from Logs

Extract metrics using log parsing:

```rust
// Log with metric-friendly fields
info!(
    metric_type = "counter",
    metric_name = "messages_processed",
    value = 1,
    labels = { peer_id: peer.to_string() },
    "Metric recorded"
);
```

### Alert Rules

Define alerts based on log patterns:

```yaml
alert: HighErrorRate
expr: rate(log_lines{level="ERROR"}[5m]) > 10
annotations:
  summary: "High error rate detected"
```

## Conclusion

Following these best practices ensures that ADIC Core's logging is:
- **Consistent**: Same patterns across all modules
- **Useful**: Contains actionable information
- **Performant**: Minimal overhead in production
- **Debuggable**: Rich context when needed

Remember: Good logging is an investment in operational excellence and developer productivity.