#!/bin/bash

# ADIC Network Wallet Monitor
# Monitors wallet balances and health across all nodes

set -e

# Configuration
BASE_PORT=${BASE_PORT:-8080}
NODE_COUNT=${NODE_COUNT:-6}

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Special addresses
TREASURY="0100000000000000000000000000000000000000000000000000000000000000"
FAUCET_ID="faucet"

clear

echo "========================================="
echo "     ADIC Network Wallet Monitor"
echo "========================================="
echo ""

# Function to get wallet info
get_wallet_info() {
    local port=$1
    curl -s "http://localhost:$port/wallet/info" 2>/dev/null
}

# Function to get balance
get_balance() {
    local port=$1
    local address=$2
    curl -s "http://localhost:$port/wallet/balance/$address" 2>/dev/null | jq -r .total
}

# Function to format ADIC amount
format_adic() {
    local amount=$1
    if (( $(echo "$amount >= 1000000" | bc -l) )); then
        echo "$(echo "scale=2; $amount/1000000" | bc)M"
    elif (( $(echo "$amount >= 1000" | bc -l) )); then
        echo "$(echo "scale=2; $amount/1000" | bc)K"
    else
        echo "$amount"
    fi
}

# Collect node information
echo -e "${BLUE}Collecting node information...${NC}"
echo ""

declare -a NODE_ADDRESSES
declare -a NODE_BALANCES
declare -a NODE_IDS

# Get info for each node
for i in $(seq 1 $NODE_COUNT); do
    PORT=$((BASE_PORT + i))

    # Get wallet info
    WALLET_INFO=$(get_wallet_info $PORT)

    if [ -n "$WALLET_INFO" ]; then
        ADDRESS=$(echo $WALLET_INFO | jq -r .address)
        NODE_ID=$(echo $WALLET_INFO | jq -r .node_id)
        BALANCE=$(get_balance $PORT $ADDRESS)

        NODE_ADDRESSES[$i]=$ADDRESS
        NODE_BALANCES[$i]=$BALANCE
        NODE_IDS[$i]=$NODE_ID

        echo -e "${GREEN}✓${NC} Node $i connected"
    else
        NODE_ADDRESSES[$i]="OFFLINE"
        NODE_BALANCES[$i]="0"
        NODE_IDS[$i]="OFFLINE"

        echo -e "${RED}✗${NC} Node $i offline"
    fi
done

echo ""
echo "========================================="
echo "           Node Wallets"
echo "========================================="
echo ""

# Display node wallet information
printf "%-7s %-20s %-66s %15s\n" "Node" "ID" "Address" "Balance (ADIC)"
printf "%-7s %-20s %-66s %15s\n" "----" "---" "-------" "--------------"

for i in $(seq 1 $NODE_COUNT); do
    if [ "${NODE_ADDRESSES[$i]}" != "OFFLINE" ]; then
        printf "%-7s %-20s %-66s %15s\n" \
            "Node $i" \
            "${NODE_IDS[$i]}" \
            "${NODE_ADDRESSES[$i]}" \
            "$(format_adic ${NODE_BALANCES[$i]})"
    else
        printf "${RED}%-7s %-20s %-66s %15s${NC}\n" \
            "Node $i" \
            "OFFLINE" \
            "OFFLINE" \
            "-"
    fi
done

echo ""
echo "========================================="
echo "         System Addresses"
echo "========================================="
echo ""

# Check treasury balance
TREASURY_BALANCE=$(get_balance $((BASE_PORT + 1)) $TREASURY 2>/dev/null || echo "0")
echo -e "Treasury Balance: ${GREEN}$(format_adic $TREASURY_BALANCE) ADIC${NC}"
echo "Address: $TREASURY"

# Calculate faucet address
FAUCET_ADDR=$(echo -n "$FAUCET_ID" | sha256sum | cut -d' ' -f1)
FAUCET_BALANCE=$(get_balance $((BASE_PORT + 1)) $FAUCET_ADDR 2>/dev/null || echo "0")
echo ""
echo -e "Faucet Balance: ${YELLOW}$(format_adic $FAUCET_BALANCE) ADIC${NC}"
echo "Address: $FAUCET_ADDR"

echo ""
echo "========================================="
echo "          Network Statistics"
echo "========================================="
echo ""

# Calculate totals
TOTAL_NODE_BALANCE=0
ACTIVE_NODES=0

for i in $(seq 1 $NODE_COUNT); do
    if [ "${NODE_ADDRESSES[$i]}" != "OFFLINE" ]; then
        TOTAL_NODE_BALANCE=$(echo "$TOTAL_NODE_BALANCE + ${NODE_BALANCES[$i]}" | bc)
        ACTIVE_NODES=$((ACTIVE_NODES + 1))
    fi
done

TOTAL_SUPPLY=$(echo "$TREASURY_BALANCE + $FAUCET_BALANCE + $TOTAL_NODE_BALANCE" | bc)

echo "Active Nodes: $ACTIVE_NODES / $NODE_COUNT"
echo "Total Node Balance: $(format_adic $TOTAL_NODE_BALANCE) ADIC"
echo "Total Supply Tracked: $(format_adic $TOTAL_SUPPLY) ADIC"
echo "Expected Genesis Supply: 300.164M ADIC"

# Check genesis application
echo ""
echo "========================================="
echo "         Genesis Application"
echo "========================================="
echo ""

for i in $(seq 1 $NODE_COUNT); do
    if [ "${NODE_ADDRESSES[$i]}" != "OFFLINE" ]; then
        # Check if genesis file exists (would need docker exec in real deployment)
        echo -n "Node $i: "
        if docker exec adic-node-$i test -f /data/.genesis_applied 2>/dev/null; then
            echo -e "${GREEN}Genesis Applied ✓${NC}"
        else
            echo -e "${YELLOW}Genesis Not Applied${NC}"
        fi
    fi
done

echo ""
echo "========================================="
echo "            Health Check"
echo "========================================="
echo ""

# Test message submission capability
echo -n "Testing message submission with deposit: "
TEST_RESPONSE=$(curl -s -X POST "http://localhost:$((BASE_PORT + 1))/submit" \
    -H "Authorization: Bearer test-token" \
    -H "Content-Type: application/json" \
    -d '{"content": "Monitor test"}' 2>/dev/null)

if echo "$TEST_RESPONSE" | jq -e .deposit_escrowed > /dev/null 2>&1; then
    DEPOSIT=$(echo "$TEST_RESPONSE" | jq -r .deposit_escrowed)
    echo -e "${GREEN}Working (${DEPOSIT} ADIC deposit)${NC}"
else
    echo -e "${RED}Failed${NC}"
fi

# Test faucet
echo -n "Testing faucet availability: "
if [ "${NODE_ADDRESSES[1]}" != "OFFLINE" ]; then
    FAUCET_TEST=$(curl -s -X POST "http://localhost:$((BASE_PORT + 1))/wallet/faucet" \
        -H "Content-Type: application/json" \
        -d "{\"address\": \"${NODE_ADDRESSES[1]}\"}" 2>/dev/null)

    if echo "$FAUCET_TEST" | jq -e .amount > /dev/null 2>&1; then
        echo -e "${GREEN}Available${NC}"
    else
        echo -e "${RED}Unavailable${NC}"
    fi
else
    echo -e "${YELLOW}Cannot test (Node 1 offline)${NC}"
fi

echo ""
echo "========================================="
echo "Monitor completed at $(date '+%Y-%m-%d %H:%M:%S')"
echo "========================================="