#!/bin/bash
set -e

# ADIC Core Deployment Script
# Usage: ./deploy.sh [environment] [action]
# Environments: local, staging, production
# Actions: setup, deploy, update, rollback, status

ENVIRONMENT=${1:-local}
ACTION=${2:-deploy}
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log() {
    echo -e "${GREEN}[$(date +'%Y-%m-%d %H:%M:%S')]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1" >&2
    exit 1
}

warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

# Check prerequisites
check_prerequisites() {
    log "Checking prerequisites..."
    
    # Check Docker
    if ! command -v docker &> /dev/null; then
        error "Docker is not installed"
    fi
    
    # Check Docker Compose
    if ! command -v docker-compose &> /dev/null; then
        error "Docker Compose is not installed"
    fi
    
    # Check Rust (for local builds)
    if [[ "$ENVIRONMENT" == "local" ]] && ! command -v cargo &> /dev/null; then
        warning "Rust is not installed. Will use Docker builds only."
    fi
    
    log "Prerequisites check passed"
}

# Setup environment
setup_environment() {
    log "Setting up $ENVIRONMENT environment..."
    
    case $ENVIRONMENT in
        local)
            setup_local
            ;;
        staging)
            setup_staging
            ;;
        production)
            setup_production
            ;;
        *)
            error "Unknown environment: $ENVIRONMENT"
            ;;
    esac
}

setup_local() {
    log "Setting up local development environment..."
    
    # Create necessary directories
    mkdir -p "$PROJECT_ROOT/data"
    mkdir -p "$PROJECT_ROOT/logs"
    mkdir -p "$PROJECT_ROOT/configs"
    
    # Build Rust project
    if command -v cargo &> /dev/null; then
        log "Building Rust project..."
        cd "$PROJECT_ROOT"
        cargo build --release
    fi
    
    # Generate local config if not exists
    if [ ! -f "$PROJECT_ROOT/configs/local.toml" ]; then
        cat > "$PROJECT_ROOT/configs/local.toml" << EOF
[node]
name = "local-node"
data_dir = "./data"
api_port = 8080

[consensus]
k = 10
c = 0.5
alpha = 2.0
tau = 0.1
max_parents = 8
p = 3

[storage]
backend = "memory"
max_messages = 100000

[metrics]
enabled = true
port = 9090
EOF
        log "Created local configuration"
    fi
    
    # Generate keypair if not exists
    if [ ! -f "$PROJECT_ROOT/data/node.key" ]; then
        log "Generating node keypair..."
        if [ -f "$PROJECT_ROOT/target/release/adic" ]; then
            "$PROJECT_ROOT/target/release/adic" keygen -o "$PROJECT_ROOT/data/node.key"
        fi
    fi
}

setup_staging() {
    log "Setting up staging environment..."
    
    # Build Docker images
    docker-compose -f "$PROJECT_ROOT/docker-compose.yml" build
    
    # Setup staging network
    docker network create adic-staging 2>/dev/null || true
    
    log "Staging environment ready"
}

setup_production() {
    log "Setting up production environment..."
    
    # Production safety checks
    read -p "Are you sure you want to deploy to production? (yes/no): " confirm
    if [[ "$confirm" != "yes" ]]; then
        error "Production deployment cancelled"
    fi
    
    # Build optimized Docker images
    docker-compose -f "$PROJECT_ROOT/docker-compose.yml" \
        -f "$PROJECT_ROOT/docker-compose.prod.yml" build
    
    log "Production environment ready"
}

# Deploy application
deploy() {
    log "Deploying to $ENVIRONMENT..."
    
    case $ENVIRONMENT in
        local)
            deploy_local
            ;;
        staging)
            deploy_staging
            ;;
        production)
            deploy_production
            ;;
    esac
}

deploy_local() {
    log "Starting local deployment..."
    
    # Start single node
    if [ -f "$PROJECT_ROOT/target/release/adic" ]; then
        log "Starting ADIC node..."
        "$PROJECT_ROOT/target/release/adic" start \
            --config "$PROJECT_ROOT/configs/local.toml" &
        
        echo $! > "$PROJECT_ROOT/adic.pid"
        log "ADIC node started (PID: $(cat "$PROJECT_ROOT/adic.pid"))"
    else
        # Fallback to Docker
        docker-compose -f "$PROJECT_ROOT/docker-compose.yml" up -d adic-node
        log "ADIC node started in Docker"
    fi
    
    # Start monitoring if requested
    if [[ "$3" == "--with-monitoring" ]]; then
        docker-compose --profile monitoring up -d
        log "Monitoring stack started"
        log "Grafana: http://localhost:3000"
        log "Prometheus: http://localhost:9090"
    fi
}

deploy_staging() {
    log "Starting staging deployment..."
    
    # Deploy multi-node setup
    docker-compose --profile multi-node up -d
    
    # Wait for nodes to start
    sleep 5
    
    # Check node health
    for i in 1 2 3; do
        if docker-compose exec -T adic-node-$i curl -f http://localhost:8080/health &>/dev/null; then
            log "Node $i is healthy"
        else
            warning "Node $i health check failed"
        fi
    done
    
    log "Staging deployment complete"
}

deploy_production() {
    log "Starting production deployment..."
    
    # Blue-green deployment
    docker-compose -f "$PROJECT_ROOT/docker-compose.yml" \
        -f "$PROJECT_ROOT/docker-compose.prod.yml" \
        up -d --scale adic-node=3
    
    # Health checks
    sleep 10
    health_check
    
    log "Production deployment complete"
}

# Update deployment
update() {
    log "Updating $ENVIRONMENT deployment..."
    
    # Pull latest changes
    if [ -d "$PROJECT_ROOT/.git" ]; then
        git pull origin main
    fi
    
    # Rebuild
    case $ENVIRONMENT in
        local)
            if command -v cargo &> /dev/null; then
                cargo build --release
            fi
            ;;
        *)
            docker-compose build --no-cache
            ;;
    esac
    
    # Rolling update
    if [[ "$ENVIRONMENT" != "local" ]]; then
        docker-compose up -d --no-deps --build adic-node
    else
        stop
        deploy
    fi
    
    log "Update complete"
}

# Rollback deployment
rollback() {
    log "Rolling back $ENVIRONMENT deployment..."
    
    if [[ "$ENVIRONMENT" == "production" ]]; then
        # Get previous image tag
        PREVIOUS_TAG=$(docker images adic-node --format "{{.Tag}}" | head -2 | tail -1)
        
        if [ -z "$PREVIOUS_TAG" ]; then
            error "No previous version found for rollback"
        fi
        
        log "Rolling back to version: $PREVIOUS_TAG"
        docker-compose down
        docker tag adic-node:$PREVIOUS_TAG adic-node:latest
        docker-compose up -d
    else
        warning "Rollback is only supported in production"
    fi
}

# Stop deployment
stop() {
    log "Stopping $ENVIRONMENT deployment..."
    
    if [[ "$ENVIRONMENT" == "local" ]] && [ -f "$PROJECT_ROOT/adic.pid" ]; then
        kill $(cat "$PROJECT_ROOT/adic.pid") 2>/dev/null || true
        rm "$PROJECT_ROOT/adic.pid"
    fi
    
    docker-compose down
    log "Deployment stopped"
}

# Check status
status() {
    log "Checking $ENVIRONMENT status..."
    
    if [[ "$ENVIRONMENT" == "local" ]] && [ -f "$PROJECT_ROOT/adic.pid" ]; then
        if ps -p $(cat "$PROJECT_ROOT/adic.pid") > /dev/null; then
            log "ADIC node is running (PID: $(cat "$PROJECT_ROOT/adic.pid"))"
        else
            warning "ADIC node is not running"
        fi
    fi
    
    # Docker status
    docker-compose ps
    
    # Health check
    health_check
}

# Health check
health_check() {
    log "Performing health checks..."
    
    # Check API endpoint
    if curl -f http://localhost:8080/health &>/dev/null; then
        log "API health check: OK"
    else
        warning "API health check: FAILED"
    fi
    
    # Check metrics endpoint
    if curl -f http://localhost:9090/metrics &>/dev/null; then
        log "Metrics endpoint: OK"
    else
        warning "Metrics endpoint: FAILED"
    fi
}

# Backup data
backup() {
    log "Creating backup..."
    
    BACKUP_DIR="$PROJECT_ROOT/backups/$(date +'%Y%m%d_%H%M%S')"
    mkdir -p "$BACKUP_DIR"
    
    # Backup data directory
    if [ -d "$PROJECT_ROOT/data" ]; then
        tar czf "$BACKUP_DIR/data.tar.gz" -C "$PROJECT_ROOT" data/
        log "Data backed up to $BACKUP_DIR/data.tar.gz"
    fi
    
    # Backup configs
    if [ -d "$PROJECT_ROOT/configs" ]; then
        tar czf "$BACKUP_DIR/configs.tar.gz" -C "$PROJECT_ROOT" configs/
        log "Configs backed up to $BACKUP_DIR/configs.tar.gz"
    fi
}

# Main execution
main() {
    check_prerequisites
    
    case $ACTION in
        setup)
            setup_environment
            ;;
        deploy)
            deploy
            ;;
        update)
            update
            ;;
        rollback)
            rollback
            ;;
        stop)
            stop
            ;;
        status)
            status
            ;;
        backup)
            backup
            ;;
        *)
            error "Unknown action: $ACTION"
            ;;
    esac
}

# Run main function
main