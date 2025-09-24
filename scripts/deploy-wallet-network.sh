#!/bin/bash

# ADIC Wallet-Enabled Network Deployment Script
# Deploys a complete 6-node ADIC network with wallet support

set -e

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
EXPLORER_DIR="$PROJECT_ROOT/../adic-explorer"
NODE_COUNT=${NODE_COUNT:-6}
BASE_PORT=${BASE_PORT:-8081}

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m'

# Functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
    exit 1
}

clear
echo "==========================================="
echo "   ADIC Wallet Network Deployment"
echo "==========================================="
echo ""

# Step 1: Check prerequisites
log_info "Checking prerequisites..."

if ! command -v docker &> /dev/null; then
    log_error "Docker is not installed"
fi

if ! command -v docker-compose &> /dev/null; then
    log_error "Docker Compose is not installed"
fi

if ! command -v cargo &> /dev/null; then
    log_error "Rust/Cargo is not installed"
fi

log_success "All prerequisites met"

# Step 2: Build ADIC with wallet support
log_info "Building ADIC with wallet support..."

cd "$PROJECT_ROOT"

# Check if wallet modules exist
if [ ! -f "crates/adic-node/src/wallet.rs" ]; then
    log_error "Wallet module not found. Please ensure wallet implementation is complete."
fi

# Build the project
log_info "Running cargo build..."
cargo build --release 2>&1 | tail -5

if [ ! -f "target/release/adic" ]; then
    log_error "Build failed - adic binary not found"
fi

log_success "ADIC built successfully"

# Step 3: Build Docker image
log_info "Building Docker image with wallet support..."

# Create temporary Dockerfile with wallet support
cat > Dockerfile.wallet <<EOF
FROM rust:1.70 as builder

WORKDIR /app

# Copy source code
COPY . .

# Build with wallet support
RUN cargo build --release

# Runtime image
FROM debian:bookworm-slim

# Install dependencies
RUN apt-get update && apt-get install -y \\
    ca-certificates \\
    libssl3 \\
    curl \\
    && rm -rf /var/lib/apt/lists/*

# Copy binary
COPY --from=builder /app/target/release/adic /usr/local/bin/adic

# Create data directory
RUN mkdir -p /data

# Environment variables
ENV ADIC_WALLET_ENABLED=true
ENV ADIC_GENESIS_AUTOCONFIG=true

VOLUME ["/data"]

EXPOSE 8080 19000

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \\
  CMD curl -f http://localhost:8080/wallet/info || exit 1

ENTRYPOINT ["adic"]
CMD ["start", "--data-dir", "/data"]
EOF

docker build -t adic:wallet -f Dockerfile.wallet . 2>&1 | tail -5
rm Dockerfile.wallet

log_success "Docker image built"

# Step 4: Prepare Docker Compose configuration
log_info "Preparing Docker Compose configuration..."

cd "$EXPLORER_DIR"

# Check if docker-compose file exists
if [ ! -f "docker-compose.yml" ]; then
    log_error "docker-compose.yml not found in $EXPLORER_DIR"
fi

# Step 5: Stop existing containers
log_info "Stopping existing containers..."
docker-compose down 2>/dev/null || true

# Clean up old data
log_warning "Cleaning up old node data..."
for i in $(seq 1 $NODE_COUNT); do
    sudo rm -rf "./data/node-$i" 2>/dev/null || true
    mkdir -p "./data/node-$i"
done

# Step 6: Start the network
log_info "Starting 6-node ADIC network..."

docker-compose up -d

# Wait for nodes to start
log_info "Waiting for nodes to initialize..."
sleep 10

# Step 7: Verify node health
log_info "Verifying node health..."

HEALTHY_NODES=0
for i in $(seq 1 $NODE_COUNT); do
    PORT=$((BASE_PORT + i - 1))
    if curl -s -f "http://localhost:$PORT/health" > /dev/null 2>&1; then
        log_success "Node $i is healthy (port $PORT)"
        HEALTHY_NODES=$((HEALTHY_NODES + 1))
    else
        log_warning "Node $i is not responding (port $PORT)"
    fi
done

if [ $HEALTHY_NODES -eq 0 ]; then
    log_error "No nodes are healthy! Check docker logs."
fi

echo ""
log_success "$HEALTHY_NODES/$NODE_COUNT nodes are running"

# Step 8: Verify wallet functionality
log_info "Verifying wallet functionality..."

for i in $(seq 1 $NODE_COUNT); do
    PORT=$((BASE_PORT + i - 1))
    WALLET_INFO=$(curl -s "http://localhost:$PORT/wallet/info" 2>/dev/null)

    if [ -n "$WALLET_INFO" ]; then
        ADDRESS=$(echo "$WALLET_INFO" | jq -r .address 2>/dev/null || echo "unknown")
        if [ "$ADDRESS" != "unknown" ] && [ "$ADDRESS" != "null" ]; then
            log_success "Node $i wallet: ${ADDRESS:0:16}..."
        else
            log_warning "Node $i wallet not initialized"
        fi
    fi
done

# Step 9: Check genesis allocation
log_info "Checking genesis allocations..."

TREASURY="0100000000000000000000000000000000000000000000000000000000000000"
TREASURY_BALANCE=$(curl -s "http://localhost:$BASE_PORT/wallet/balance/$TREASURY" 2>/dev/null | jq -r .total 2>/dev/null || echo "0")

if [ "$TREASURY_BALANCE" = "60000000" ]; then
    log_success "Treasury initialized with 60M ADIC"
else
    log_warning "Treasury balance: $TREASURY_BALANCE ADIC (expected 60M)"
fi

# Step 10: Initialize faucet
log_info "Initializing faucet for testing..."

# Get first node's address for faucet test
NODE1_INFO=$(curl -s "http://localhost:$BASE_PORT/wallet/info" 2>/dev/null)
NODE1_ADDR=$(echo "$NODE1_INFO" | jq -r .address 2>/dev/null)

if [ -n "$NODE1_ADDR" ] && [ "$NODE1_ADDR" != "null" ]; then
    FAUCET_RESPONSE=$(curl -s -X POST "http://localhost:$BASE_PORT/wallet/faucet" \
        -H "Content-Type: application/json" \
        -d "{\"address\": \"$NODE1_ADDR\"}" 2>/dev/null)

    if echo "$FAUCET_RESPONSE" | jq -e .amount > /dev/null 2>&1; then
        AMOUNT=$(echo "$FAUCET_RESPONSE" | jq -r .amount)
        log_success "Faucet test successful: $AMOUNT ADIC distributed"
    else
        log_warning "Faucet test failed"
    fi
fi

# Step 11: Start monitoring
echo ""
echo "==========================================="
echo "        Network Deployment Complete"
echo "==========================================="
echo ""
echo "Network Status:"
echo "  • Nodes running: $HEALTHY_NODES/$NODE_COUNT"
echo "  • Base API port: $BASE_PORT"
echo "  • Explorer URL: http://localhost:3000"
echo ""
echo "Available Scripts:"
echo "  • Monitor wallets: $SCRIPT_DIR/monitor-wallets.sh"
echo "  • Test integration: $SCRIPT_DIR/test-wallet-integration.sh"
echo "  • Run simulation: $EXPLORER_DIR/test-wallet-simulation.sh"
echo ""
echo "Docker Commands:"
echo "  • View logs: docker-compose logs -f"
echo "  • Stop network: docker-compose down"
echo "  • Restart network: docker-compose restart"
echo ""
echo "API Endpoints:"
for i in $(seq 1 $NODE_COUNT); do
    PORT=$((BASE_PORT + i - 1))
    echo "  • Node $i: http://localhost:$PORT"
done
echo ""

# Optional: Start simulation
read -p "Start wallet simulation? (y/n) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    log_info "Starting wallet simulation..."
    cd "$EXPLORER_DIR"
    if [ -f "simulation/wallet_simulation.py" ]; then
        python3 simulation/wallet_simulation.py \
            --base-port $BASE_PORT \
            --nodes $NODE_COUNT \
            --duration 60 &
        SIM_PID=$!
        log_success "Simulation started (PID: $SIM_PID)"
        echo "Stop simulation with: kill $SIM_PID"
    else
        log_warning "Simulation script not found"
    fi
fi

echo ""
log_success "Deployment complete! 🚀"