#!/bin/bash
# Tail logs from ADIC development environment

set -e

# Color output
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${YELLOW}📋 Tailing logs from all services...${NC}"
echo -e "${YELLOW}Press Ctrl+C to exit${NC}"
echo ""

# Use docker compose (new) or docker-compose (old)
if docker compose version &> /dev/null 2>&1; then
    COMPOSE_CMD="docker compose"
else
    COMPOSE_CMD="docker-compose"
fi

# Tail logs from all services
$COMPOSE_CMD -f docker-compose.dev.yml logs -f "$@"
