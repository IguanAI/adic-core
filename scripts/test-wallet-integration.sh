#!/bin/bash

# ADIC Wallet System Integration Test
# This script tests the complete wallet functionality

set -e

echo "========================================="
echo "ADIC Wallet Integration Test"
echo "========================================="

# Configuration
API_PORT=${API_PORT:-8090}
DATA_DIR=${DATA_DIR:-./test-wallet-data}
NODE_NAME=${NODE_NAME:-test-node}
BASE_URL="http://localhost:$API_PORT"

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Helper functions
log_success() {
    echo -e "${GREEN}✓${NC} $1"
}

log_error() {
    echo -e "${RED}✗${NC} $1"
    exit 1
}

log_info() {
    echo -e "${YELLOW}→${NC} $1"
}

# Cleanup function
cleanup() {
    log_info "Cleaning up..."
    pkill -f "adic.*$DATA_DIR" 2>/dev/null || true
    rm -rf $DATA_DIR
}

# Start fresh
cleanup

echo ""
log_info "Starting ADIC node on port $API_PORT..."

# Start node in background
./target/debug/adic start --data-dir $DATA_DIR --api-port $API_PORT > /tmp/adic-test.log 2>&1 &
NODE_PID=$!

# Wait for node to start
log_info "Waiting for node to initialize..."
sleep 5

# Check if node is running
if ! kill -0 $NODE_PID 2>/dev/null; then
    log_error "Node failed to start. Check /tmp/adic-test.log"
fi

echo ""
echo "1. Testing Wallet Creation"
echo "----------------------------"

# Get wallet info
log_info "Getting wallet info..."
WALLET_INFO=$(curl -s $BASE_URL/wallet/info)

if [ -z "$WALLET_INFO" ]; then
    log_error "Failed to get wallet info"
fi

ADDRESS=$(echo $WALLET_INFO | jq -r .address)
PUBLIC_KEY=$(echo $WALLET_INFO | jq -r .public_key)
NODE_ID=$(echo $WALLET_INFO | jq -r .node_id)

log_success "Wallet created successfully"
log_info "Address: $ADDRESS"
log_info "Node ID: $NODE_ID"

echo ""
echo "2. Testing Genesis Allocation"
echo "------------------------------"

# Check treasury balance
log_info "Checking treasury balance..."
TREASURY_ADDR="0100000000000000000000000000000000000000000000000000000000000000"
TREASURY_BALANCE=$(curl -s $BASE_URL/wallet/balance/$TREASURY_ADDR | jq .total)

if [ "$TREASURY_BALANCE" == "60000000" ]; then
    log_success "Treasury has correct balance: 60M ADIC"
else
    log_error "Treasury balance incorrect: $TREASURY_BALANCE"
fi

# Check if genesis marker exists
if [ -f "$DATA_DIR/.genesis_applied" ]; then
    log_success "Genesis marker file created"
else
    log_error "Genesis marker file not found"
fi

echo ""
echo "3. Testing Faucet"
echo "------------------"

# Get initial balance
log_info "Checking initial balance..."
INITIAL_BALANCE=$(curl -s $BASE_URL/wallet/balance/$ADDRESS | jq .total)
log_info "Initial balance: $INITIAL_BALANCE ADIC"

# Request from faucet
log_info "Requesting ADIC from faucet..."
FAUCET_RESPONSE=$(curl -s -X POST $BASE_URL/wallet/faucet \
    -H "Content-Type: application/json" \
    -d "{\"address\": \"$ADDRESS\"}")

FAUCET_AMOUNT=$(echo $FAUCET_RESPONSE | jq .amount)
FAUCET_TX=$(echo $FAUCET_RESPONSE | jq -r .tx_hash)

if [ "$FAUCET_AMOUNT" == "100" ]; then
    log_success "Received 100 ADIC from faucet (tx: $FAUCET_TX)"
else
    log_error "Faucet request failed"
fi

# Check new balance
sleep 1
NEW_BALANCE=$(curl -s $BASE_URL/wallet/balance/$ADDRESS | jq .total)
log_info "New balance: $NEW_BALANCE ADIC"

if [ "$NEW_BALANCE" == "100" ]; then
    log_success "Balance updated correctly"
else
    log_error "Balance not updated"
fi

echo ""
echo "4. Testing Message Submission with Deposit"
echo "-------------------------------------------"

# Submit a message
log_info "Submitting message (requires 0.1 ADIC deposit)..."
MESSAGE_RESPONSE=$(curl -s -X POST $BASE_URL/submit \
    -H "Authorization: Bearer test-token" \
    -H "Content-Type: application/json" \
    -d '{"content": "Integration test message"}')

MESSAGE_ID=$(echo $MESSAGE_RESPONSE | jq -r .message_id)
DEPOSIT_AMOUNT=$(echo $MESSAGE_RESPONSE | jq .deposit_escrowed)

if [ "$DEPOSIT_AMOUNT" == "0.1" ]; then
    log_success "Message submitted with 0.1 ADIC deposit"
    log_info "Message ID: $MESSAGE_ID"
else
    log_error "Failed to submit message with deposit"
fi

echo ""
echo "5. Testing Wallet Persistence"
echo "------------------------------"

# Kill node
log_info "Stopping node..."
kill $NODE_PID 2>/dev/null
sleep 2

# Restart node
log_info "Restarting node..."
./target/debug/adic start --data-dir $DATA_DIR --api-port $API_PORT > /tmp/adic-test2.log 2>&1 &
NODE_PID=$!
sleep 5

# Check if wallet was loaded
if grep -q "Loading existing wallet" /tmp/adic-test2.log; then
    log_success "Wallet loaded from disk on restart"
else
    log_error "Wallet not loaded on restart"
fi

# Verify same address
NEW_WALLET_INFO=$(curl -s $BASE_URL/wallet/info)
NEW_ADDRESS=$(echo $NEW_WALLET_INFO | jq -r .address)

if [ "$ADDRESS" == "$NEW_ADDRESS" ]; then
    log_success "Wallet address persisted: $ADDRESS"
else
    log_error "Wallet address changed after restart"
fi

# Check genesis not reapplied
if grep -q "Genesis already applied" /tmp/adic-test2.log; then
    log_success "Genesis not reapplied on restart"
else
    log_error "Genesis incorrectly reapplied"
fi

echo ""
echo "6. Testing Balance Query"
echo "-------------------------"

# Query various addresses
log_info "Querying different address balances..."

# Node wallet
NODE_BALANCE=$(curl -s $BASE_URL/wallet/balance/$ADDRESS | jq .total)
log_success "Node balance: $NODE_BALANCE ADIC"

# Treasury
TREASURY_BALANCE=$(curl -s $BASE_URL/wallet/balance/$TREASURY_ADDR | jq .total)
log_success "Treasury balance: $TREASURY_BALANCE ADIC"

# Random address (should be 0)
RANDOM_ADDR="deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
RANDOM_BALANCE=$(curl -s $BASE_URL/wallet/balance/$RANDOM_ADDR | jq .total)
if [ "$RANDOM_BALANCE" == "0" ]; then
    log_success "Unknown address has 0 balance"
else
    log_error "Unknown address has non-zero balance"
fi

echo ""
echo "========================================="
echo -e "${GREEN}All tests passed successfully!${NC}"
echo "========================================="

# Cleanup
log_info "Cleaning up test environment..."
kill $NODE_PID 2>/dev/null || true
rm -rf $DATA_DIR

echo ""
echo "Test Summary:"
echo "- Wallet creation: ✓"
echo "- Genesis allocation: ✓"
echo "- Faucet distribution: ✓"
echo "- Deposit requirement: ✓"
echo "- Wallet persistence: ✓"
echo "- Balance queries: ✓"

exit 0