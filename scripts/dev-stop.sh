#!/bin/bash
# Stop the ADIC development environment

set -e

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${YELLOW}🛑 Stopping ADIC Development Environment${NC}"
echo ""

# Use docker compose (new) or docker-compose (old)
if docker compose version &> /dev/null 2>&1; then
    COMPOSE_CMD="docker compose"
else
    COMPOSE_CMD="docker-compose"
fi

# Stop services
$COMPOSE_CMD -f docker-compose.dev.yml down

echo ""
echo -e "${GREEN}✅ All services stopped${NC}"
echo ""
echo -e "${YELLOW}💡 Note: Data volumes are preserved. Use ./scripts/dev-reset.sh to remove data.${NC}"
