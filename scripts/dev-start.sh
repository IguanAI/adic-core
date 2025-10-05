#!/bin/bash
# Start the ADIC development environment

set -e

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}🚀 Starting ADIC Development Environment${NC}"
echo ""

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo -e "${RED}❌ Docker is not running. Please start Docker first.${NC}"
    exit 1
fi

# Check if docker-compose is available
if ! command -v docker-compose &> /dev/null && ! docker compose version &> /dev/null 2>&1; then
    echo -e "${RED}❌ docker-compose is not installed.${NC}"
    exit 1
fi

# Use docker compose (new) or docker-compose (old)
if docker compose version &> /dev/null 2>&1; then
    COMPOSE_CMD="docker compose"
else
    COMPOSE_CMD="docker-compose"
fi

# Build and start services
echo -e "${YELLOW}📦 Building Docker images...${NC}"
$COMPOSE_CMD -f docker-compose.dev.yml build

echo ""
echo -e "${YELLOW}🏗️  Starting services...${NC}"
$COMPOSE_CMD -f docker-compose.dev.yml up -d

echo ""
echo -e "${GREEN}✅ Development environment started!${NC}"
echo ""
echo -e "${YELLOW}📊 Services available at:${NC}"
echo "  • Node 1 API:    http://localhost:8080"
echo "  • Node 2 API:    http://localhost:8081"
echo "  • Grafana:       http://localhost:3000 (admin/admin)"
echo "  • Prometheus:    http://localhost:9090"
echo "  • PostgreSQL:    localhost:5432 (adic/adic_dev_password)"
echo ""
echo -e "${YELLOW}📝 Useful commands:${NC}"
echo "  • View logs:     ./scripts/dev-logs.sh"
echo "  • Stop services: ./scripts/dev-stop.sh"
echo "  • Reset data:    ./scripts/dev-reset.sh"
echo ""
echo -e "${GREEN}🎉 Happy developing!${NC}"
