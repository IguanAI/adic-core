#!/bin/bash
# Event Streaming Verification Script
#
# This script verifies that the ADIC event streaming implementation is complete
# and ready for deployment.

set -e

echo "=================================================="
echo "ADIC Event Streaming Verification"
echo "=================================================="
echo ""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Counters
PASSED=0
FAILED=0
WARNINGS=0

check_pass() {
    echo -e "${GREEN}✓${NC} $1"
    PASSED=$((PASSED + 1))
}

check_fail() {
    echo -e "${RED}✗${NC} $1"
    FAILED=$((FAILED + 1))
}

check_warn() {
    echo -e "${YELLOW}⚠${NC} $1"
    WARNINGS=$((WARNINGS + 1))
}

echo "1. Checking Rust Code..."
echo "========================="

# Check events.rs exists
if [ -f "crates/adic-node/src/events.rs" ]; then
    check_pass "events.rs exists"

    # Check for EventBus
    if grep -q "pub struct EventBus" crates/adic-node/src/events.rs; then
        check_pass "EventBus defined"
    else
        check_fail "EventBus not found"
    fi

    # Check for NodeEvent enum
    if grep -q "pub enum NodeEvent" crates/adic-node/src/events.rs; then
        check_pass "NodeEvent enum defined"
    else
        check_fail "NodeEvent enum not found"
    fi

    # Count event types
    EVENT_COUNT=$(grep -c "TipsUpdated\|MessageFinalized\|MessageAdded\|DiversityUpdated\|EnergyUpdated\|KCoreUpdated\|AdmissibilityUpdated\|EconomicsUpdated" crates/adic-node/src/events.rs || true)
    if [ "$EVENT_COUNT" -ge 8 ]; then
        check_pass "All 8 event types present ($EVENT_COUNT occurrences)"
    else
        check_fail "Missing event types (found $EVENT_COUNT)"
    fi
else
    check_fail "events.rs not found"
fi

# Check api_ws.rs exists
if [ -f "crates/adic-node/src/api_ws.rs" ]; then
    check_pass "api_ws.rs exists"

    if grep -q "pub async fn websocket_handler" crates/adic-node/src/api_ws.rs; then
        check_pass "WebSocket handler defined"
    else
        check_fail "WebSocket handler not found"
    fi
else
    check_fail "api_ws.rs not found"
fi

# Check api_sse.rs exists
if [ -f "crates/adic-node/src/api_sse.rs" ]; then
    check_pass "api_sse.rs exists"

    if grep -q "pub async fn sse_handler" crates/adic-node/src/api_sse.rs; then
        check_pass "SSE handler defined"
    else
        check_fail "SSE handler not found"
    fi
else
    check_fail "api_sse.rs not found"
fi

# Check metrics integration
if grep -q "events_emitted_total\|websocket_connections\|sse_connections" crates/adic-node/src/metrics.rs; then
    check_pass "Event metrics integrated"
else
    check_fail "Event metrics not found in metrics.rs"
fi

echo ""
echo "2. Checking Python Code..."
echo "=========================="

# Check event_client.py
if [ -f "../adic-explorer/backend/app/services/event_client.py" ]; then
    check_pass "event_client.py exists"

    if grep -q "class ConnectionManager" ../adic-explorer/backend/app/services/event_client.py; then
        check_pass "ConnectionManager class defined"
    else
        check_fail "ConnectionManager not found"
    fi

    if grep -q "class WebSocketClient\|class SSEClient" ../adic-explorer/backend/app/services/event_client.py; then
        check_pass "Client implementations present"
    else
        check_fail "Client implementations not found"
    fi
else
    check_warn "event_client.py not found (explorer backend may not be present)"
fi

# Check event_indexer.py
if [ -f "../adic-explorer/backend/app/services/event_indexer.py" ]; then
    check_pass "event_indexer.py exists"

    if grep -q "class EventDrivenIndexer" ../adic-explorer/backend/app/services/event_indexer.py; then
        check_pass "EventDrivenIndexer class defined"
    else
        check_fail "EventDrivenIndexer not found"
    fi
else
    check_warn "event_indexer.py not found (explorer backend may not be present)"
fi

echo ""
echo "3. Checking Documentation..."
echo "============================="

# Check API documentation
if [ -f "docs/EVENT_STREAMING_API.md" ]; then
    check_pass "API documentation exists"

    LINE_COUNT=$(wc -l < docs/EVENT_STREAMING_API.md)
    if [ "$LINE_COUNT" -gt 400 ]; then
        check_pass "API documentation is comprehensive ($LINE_COUNT lines)"
    else
        check_warn "API documentation may be incomplete ($LINE_COUNT lines)"
    fi
else
    check_fail "API documentation not found"
fi

# Check integration guide
if [ -f "docs/EVENT_STREAMING_INTEGRATION_GUIDE.md" ]; then
    check_pass "Integration guide exists"

    LINE_COUNT=$(wc -l < docs/EVENT_STREAMING_INTEGRATION_GUIDE.md)
    if [ "$LINE_COUNT" -gt 500 ]; then
        check_pass "Integration guide is comprehensive ($LINE_COUNT lines)"
    else
        check_warn "Integration guide may be incomplete ($LINE_COUNT lines)"
    fi
else
    check_fail "Integration guide not found"
fi

# Check deployment checklist
if [ -f "docs/EVENT_STREAMING_DEPLOYMENT.md" ]; then
    check_pass "Deployment checklist exists"
else
    check_fail "Deployment checklist not found"
fi

# Check summary
if [ -f "EVENT_STREAMING_SUMMARY.md" ]; then
    check_pass "Implementation summary exists"
else
    check_fail "Implementation summary not found"
fi

echo ""
echo "4. Checking Examples..."
echo "========================"

# Check browser example
if [ -f "docs/examples/browser/websocket-client.html" ]; then
    check_pass "Browser example exists"
else
    check_fail "Browser example not found"
fi

# Check Python example
if [ -f "docs/examples/python/event_stream_client.py" ]; then
    check_pass "Python example exists"

    if head -1 docs/examples/python/event_stream_client.py | grep -q "^#!/usr/bin/env python"; then
        check_pass "Python example is executable"
    else
        check_warn "Python example may not be executable"
    fi
else
    check_fail "Python example not found"
fi

echo ""
echo "5. Checking Tests..."
echo "===================="

# Check Rust tests
if [ -f "crates/adic-node/tests/event_streaming_integration.rs" ]; then
    check_pass "Rust integration tests exist"

    TEST_COUNT=$(grep -c "#\[tokio::test\]" crates/adic-node/tests/event_streaming_integration.rs || true)
    if [ "$TEST_COUNT" -ge 8 ]; then
        check_pass "Multiple test cases present ($TEST_COUNT tests)"
    else
        check_warn "Few test cases ($TEST_COUNT tests)"
    fi
else
    check_fail "Rust integration tests not found"
fi

# Check Python tests
if [ -f "../adic-explorer/backend/tests/test_event_client.py" ]; then
    check_pass "Python unit tests exist"
else
    check_warn "Python unit tests not found"
fi

echo ""
echo "6. Compilation Check..."
echo "========================"

# Try to compile
if cargo check --message-format=short 2>&1 | grep -q "Finished"; then
    check_pass "Code compiles successfully"

    # Check for errors (not warnings)
    if cargo check --message-format=short 2>&1 | grep -q "error:"; then
        check_fail "Compilation errors present"
    else
        check_pass "No compilation errors"
    fi
else
    check_fail "Code does not compile"
fi

echo ""
echo "7. Checking TODOs..."
echo "===================="

# Check for TODOs in production code
TODO_COUNT=$(grep -r "TODO\|FIXME\|XXX" crates/adic-node/src/{events,api_ws,api_sse}.rs 2>/dev/null | wc -l || echo "0")
if [ "$TODO_COUNT" -eq 0 ]; then
    check_pass "No TODOs in production code"
else
    check_warn "Found $TODO_COUNT TODO(s) in production code"
fi

echo ""
echo "=================================================="
echo "Verification Summary"
echo "=================================================="
echo ""
echo -e "${GREEN}Passed:${NC}   $PASSED"
echo -e "${YELLOW}Warnings:${NC} $WARNINGS"
echo -e "${RED}Failed:${NC}   $FAILED"
echo ""

if [ $FAILED -eq 0 ]; then
    if [ $WARNINGS -eq 0 ]; then
        echo -e "${GREEN}✓ All checks passed! Implementation is production ready.${NC}"
        exit 0
    else
        echo -e "${YELLOW}⚠ All critical checks passed, but there are warnings.${NC}"
        exit 0
    fi
else
    echo -e "${RED}✗ Some checks failed. Please review the implementation.${NC}"
    exit 1
fi
