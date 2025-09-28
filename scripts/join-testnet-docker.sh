#!/bin/bash

# ADIC Testnet Validator Setup Script
# This script helps users quickly join the ADIC testnet as a validator node

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Print colored output
print_status() {
    echo -e "${GREEN}✓${NC} $1"
}

print_error() {
    echo -e "${RED}✗${NC} $1"
}

print_info() {
    echo -e "${BLUE}ℹ${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}⚠${NC} $1"
}

# Banner
echo -e "${GREEN}"
echo "╔═══════════════════════════════════════════╗"
echo "║     ADIC Testnet Validator Setup         ║"
echo "║         https://adicl1.com                ║"
echo "╚═══════════════════════════════════════════╝"
echo -e "${NC}"
echo

# Check for Docker
print_info "Checking prerequisites..."
if ! command -v docker &> /dev/null; then
    print_error "Docker is required but not installed."
    echo "Please install Docker first: https://docs.docker.com/get-docker/"
    exit 1
fi

if ! command -v docker-compose &> /dev/null; then
    # Check for docker compose (without hyphen)
    if ! docker compose version &> /dev/null; then
        print_error "Docker Compose is required but not installed."
        echo "Please install Docker Compose: https://docs.docker.com/compose/install/"
        exit 1
    else
        # Use docker compose instead of docker-compose
        DOCKER_COMPOSE="docker compose"
    fi
else
    DOCKER_COMPOSE="docker-compose"
fi

print_status "Docker and Docker Compose found"

# Create directory
INSTALL_DIR="${ADIC_DIR:-./adic-testnet}"
print_info "Creating directory: $INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
cd "$INSTALL_DIR"

# Download configuration files
print_info "Downloading testnet configuration..."

# Download docker-compose.testnet.yml
if curl -fsSL -o docker-compose.yml https://raw.githubusercontent.com/IguanAI/adic-core/main/docker-compose.testnet.yml; then
    print_status "Downloaded docker-compose.yml"
else
    print_error "Failed to download docker-compose.yml"
    exit 1
fi

# Download testnet-config.toml
if curl -fsSL -o testnet-config.toml https://raw.githubusercontent.com/IguanAI/adic-core/main/testnet-config.toml; then
    print_status "Downloaded testnet-config.toml"
else
    print_error "Failed to download testnet-config.toml"
    exit 1
fi

# Create .env file with defaults
cat > .env << EOF
# ADIC Testnet Configuration
NODE_NAME=testnet-validator-$(openssl rand -hex 4)
RUST_LOG=info
API_PORT=8080
P2P_PORT=9000
QUIC_PORT=9001
GRAFANA_PASSWORD=admin
EOF

print_status "Created .env file with default settings"

# Check if ports are available
check_port() {
    local port=$1
    if lsof -Pi :$port -sTCP:LISTEN -t >/dev/null 2>&1; then
        return 1
    else
        return 0
    fi
}

print_info "Checking port availability..."
PORTS_OK=true
for port in 8080 9000 9001; do
    if ! check_port $port; then
        print_warning "Port $port is already in use"
        PORTS_OK=false
    fi
done

if [ "$PORTS_OK" = false ]; then
    print_warning "Some ports are in use. You may need to:"
    echo "  1. Stop conflicting services, or"
    echo "  2. Modify the ports in the .env file"
    echo
    read -p "Continue anyway? (y/N): " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        exit 1
    fi
fi

# Build and start the container
print_info "Building ADIC node from source (this may take 5-10 minutes)..."
echo

if $DOCKER_COMPOSE up -d --build; then
    print_status "ADIC testnet validator node started successfully!"
else
    print_error "Failed to start the node"
    echo "Check the logs with: $DOCKER_COMPOSE logs"
    exit 1
fi

# Wait for node to start
print_info "Waiting for node to initialize..."
sleep 10

# Check if node is running
if curl -s http://localhost:8080/health > /dev/null 2>&1; then
    print_status "Node is healthy and responding"
else
    print_warning "Node may still be starting up. Check status in a few moments."
fi

# Display information
echo
echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
echo -e "${GREEN} SUCCESS! Your ADIC testnet validator is running!${NC}"
echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
echo
echo "📊 Node Information:"
echo "   Directory: $INSTALL_DIR"
echo "   Config: $INSTALL_DIR/testnet-config.toml"
echo "   Data: Docker volume (adic-testnet-data)"
echo
echo "🌐 Network Endpoints:"
echo "   API: http://localhost:8080"
echo "   P2P: localhost:9000"
echo "   QUIC: localhost:9001"
echo
echo "🔧 Useful Commands:"
echo "   View logs:    $DOCKER_COMPOSE logs -f"
echo "   Stop node:    $DOCKER_COMPOSE down"
echo "   Start node:   $DOCKER_COMPOSE up -d"
echo "   Node status:  curl http://localhost:8080/api/v1/node/info"
echo "   Peer count:   curl http://localhost:8080/api/v1/network/peers | jq '.peers | length'"
echo
echo "📚 Documentation: https://github.com/IguanAI/adic-core/blob/main/TESTNET.md"
echo "🔍 Explorer: https://adicl1.com/explorer"
echo
echo "Your node is configured as a VALIDATOR and will:"
echo "  • Participate in consensus"
echo "  • Validate transactions"
echo "  • Auto-update when new versions are released"
echo
print_info "Monitor your node with: $DOCKER_COMPOSE logs -f adic-testnet"