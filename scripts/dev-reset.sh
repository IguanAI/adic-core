#!/bin/bash
# Reset ADIC development environment (removes all data)

set -e

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${RED}⚠️  WARNING: This will delete all development data!${NC}"
echo ""
read -p "Are you sure you want to reset the environment? (yes/no): " -r
echo

if [[ ! $REPLY =~ ^[Yy][Ee][Ss]$ ]]; then
    echo -e "${YELLOW}Reset cancelled.${NC}"
    exit 0
fi

# Use docker compose (new) or docker-compose (old)
if docker compose version &> /dev/null 2>&1; then
    COMPOSE_CMD="docker compose"
else
    COMPOSE_CMD="docker-compose"
fi

echo -e "${YELLOW}🗑️  Stopping services and removing volumes...${NC}"
$COMPOSE_CMD -f docker-compose.dev.yml down -v

echo ""
echo -e "${YELLOW}🧹 Cleaning up Docker images...${NC}"
docker images | grep adic-core | awk '{print $3}' | xargs -r docker rmi -f 2>/dev/null || true

echo ""
echo -e "${GREEN}✅ Environment reset complete!${NC}"
echo ""
echo -e "${YELLOW}Run ./scripts/dev-start.sh to start fresh.${NC}"
