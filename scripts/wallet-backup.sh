#!/bin/bash

# ADIC Wallet Backup and Recovery Utility
# Safely backs up and restores wallet files for ADIC nodes

set -e

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
BACKUP_DIR="${BACKUP_DIR:-$HOME/.adic-wallet-backups}"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

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

# Show usage
usage() {
    echo "ADIC Wallet Backup and Recovery Utility"
    echo ""
    echo "Usage: $0 [command] [options]"
    echo ""
    echo "Commands:"
    echo "  backup <wallet_path>     Backup a wallet file"
    echo "  restore <backup_file>    Restore a wallet from backup"
    echo "  list                     List all backups"
    echo "  verify <wallet_path>     Verify wallet integrity"
    echo "  export <wallet_path>     Export wallet info (address, public key)"
    echo ""
    echo "Options:"
    echo "  --backup-dir <dir>       Custom backup directory (default: ~/.adic-wallet-backups)"
    echo "  --force                  Force overwrite existing files"
    echo ""
    echo "Examples:"
    echo "  $0 backup ./data/node-1/wallet.json"
    echo "  $0 restore ~/.adic-wallet-backups/wallet_20240101_120000.json"
    echo "  $0 list"
    exit 0
}

# Backup wallet
backup_wallet() {
    local wallet_path="$1"

    if [ ! -f "$wallet_path" ]; then
        log_error "Wallet file not found: $wallet_path"
    fi

    # Create backup directory if it doesn't exist
    mkdir -p "$BACKUP_DIR"

    # Extract wallet info
    local wallet_address=$(jq -r .address "$wallet_path" 2>/dev/null || echo "unknown")
    local node_id=$(jq -r .node_id "$wallet_path" 2>/dev/null || echo "unknown")

    # Create backup filename
    local backup_name="wallet_${node_id}_${TIMESTAMP}.json"
    local backup_path="$BACKUP_DIR/$backup_name"

    # Check if backup already exists
    if [ -f "$backup_path" ] && [ "$FORCE" != "true" ]; then
        log_error "Backup already exists: $backup_path (use --force to overwrite)"
    fi

    # Copy wallet file
    cp "$wallet_path" "$backup_path"

    # Create metadata file
    cat > "$backup_path.meta" <<EOF
{
    "original_path": "$wallet_path",
    "backup_time": "$(date -Iseconds)",
    "wallet_address": "$wallet_address",
    "node_id": "$node_id",
    "file_size": $(stat -f%z "$wallet_path" 2>/dev/null || stat -c%s "$wallet_path"),
    "checksum": "$(sha256sum "$wallet_path" | cut -d' ' -f1)"
}
EOF

    # Set appropriate permissions
    chmod 600 "$backup_path"
    chmod 600 "$backup_path.meta"

    log_success "Wallet backed up to: $backup_path"
    log_info "Address: $wallet_address"
    log_info "Node ID: $node_id"

    # Create symlink to latest backup
    ln -sf "$backup_name" "$BACKUP_DIR/latest"
}

# Restore wallet
restore_wallet() {
    local backup_path="$1"
    local target_path="$2"

    if [ ! -f "$backup_path" ]; then
        log_error "Backup file not found: $backup_path"
    fi

    # If no target path specified, try to get from metadata
    if [ -z "$target_path" ]; then
        if [ -f "$backup_path.meta" ]; then
            target_path=$(jq -r .original_path "$backup_path.meta")
            log_info "Restoring to original location: $target_path"
        else
            log_error "No target path specified and no metadata found"
        fi
    fi

    # Check if target exists
    if [ -f "$target_path" ] && [ "$FORCE" != "true" ]; then
        log_warning "Target file exists: $target_path"
        read -p "Overwrite? (y/n) " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            log_info "Restore cancelled"
            exit 0
        fi
    fi

    # Verify backup integrity
    if [ -f "$backup_path.meta" ]; then
        local expected_checksum=$(jq -r .checksum "$backup_path.meta")
        local actual_checksum=$(sha256sum "$backup_path" | cut -d' ' -f1)

        if [ "$expected_checksum" != "$actual_checksum" ]; then
            log_error "Backup checksum mismatch! File may be corrupted."
        fi
    fi

    # Create target directory if needed
    target_dir=$(dirname "$target_path")
    mkdir -p "$target_dir"

    # Restore wallet
    cp "$backup_path" "$target_path"
    chmod 600 "$target_path"

    # Extract wallet info
    local wallet_address=$(jq -r .address "$target_path" 2>/dev/null || echo "unknown")
    local node_id=$(jq -r .node_id "$target_path" 2>/dev/null || echo "unknown")

    log_success "Wallet restored to: $target_path"
    log_info "Address: $wallet_address"
    log_info "Node ID: $node_id"
}

# List backups
list_backups() {
    if [ ! -d "$BACKUP_DIR" ]; then
        log_warning "No backup directory found"
        return
    fi

    echo "Available wallet backups:"
    echo "========================="
    echo ""

    for backup in "$BACKUP_DIR"/wallet_*.json; do
        if [ -f "$backup" ]; then
            local filename=$(basename "$backup")
            local size=$(du -h "$backup" | cut -f1)

            # Get metadata if available
            if [ -f "$backup.meta" ]; then
                local backup_time=$(jq -r .backup_time "$backup.meta")
                local wallet_address=$(jq -r .wallet_address "$backup.meta")
                local node_id=$(jq -r .node_id "$backup.meta")

                echo "  $filename"
                echo "    Size: $size"
                echo "    Time: $backup_time"
                echo "    Node: $node_id"
                echo "    Address: ${wallet_address:0:16}..."
            else
                echo "  $filename ($size)"
            fi
            echo ""
        fi
    done

    # Show latest symlink
    if [ -L "$BACKUP_DIR/latest" ]; then
        local latest=$(readlink "$BACKUP_DIR/latest")
        echo "Latest backup: $latest"
    fi
}

# Verify wallet
verify_wallet() {
    local wallet_path="$1"

    if [ ! -f "$wallet_path" ]; then
        log_error "Wallet file not found: $wallet_path"
    fi

    log_info "Verifying wallet: $wallet_path"

    # Check JSON validity
    if ! jq empty "$wallet_path" 2>/dev/null; then
        log_error "Invalid JSON format"
    fi

    # Check required fields
    local has_error=false

    for field in private_key public_key address node_id; do
        if ! jq -e ".$field" "$wallet_path" > /dev/null 2>&1; then
            log_warning "Missing field: $field"
            has_error=true
        fi
    done

    if [ "$has_error" = true ]; then
        log_error "Wallet verification failed"
    fi

    # Extract and display wallet info
    local address=$(jq -r .address "$wallet_path")
    local public_key=$(jq -r .public_key "$wallet_path")
    local node_id=$(jq -r .node_id "$wallet_path")

    # Verify key lengths
    if [ ${#address} -ne 64 ]; then
        log_warning "Address has unexpected length: ${#address} (expected 64)"
    fi

    if [ ${#public_key} -ne 64 ]; then
        log_warning "Public key has unexpected length: ${#public_key} (expected 64)"
    fi

    log_success "Wallet verification passed"
    echo ""
    echo "Wallet Information:"
    echo "  Node ID: $node_id"
    echo "  Address: $address"
    echo "  Public Key: $public_key"
    echo "  File size: $(stat -f%z "$wallet_path" 2>/dev/null || stat -c%s "$wallet_path") bytes"
    echo "  Checksum: $(sha256sum "$wallet_path" | cut -d' ' -f1)"
}

# Export wallet info
export_wallet() {
    local wallet_path="$1"

    if [ ! -f "$wallet_path" ]; then
        log_error "Wallet file not found: $wallet_path"
    fi

    # Extract wallet info
    local address=$(jq -r .address "$wallet_path")
    local public_key=$(jq -r .public_key "$wallet_path")
    local node_id=$(jq -r .node_id "$wallet_path")

    # Create export data (without private key)
    cat <<EOF
{
    "node_id": "$node_id",
    "address": "$address",
    "public_key": "$public_key",
    "exported_at": "$(date -Iseconds)"
}
EOF
}

# Parse command line arguments
COMMAND=""
FORCE="false"

while [[ $# -gt 0 ]]; do
    case $1 in
        backup|restore|list|verify|export)
            COMMAND="$1"
            shift
            ;;
        --backup-dir)
            BACKUP_DIR="$2"
            shift 2
            ;;
        --force)
            FORCE="true"
            shift
            ;;
        --help|-h)
            usage
            ;;
        *)
            if [ -z "$COMMAND" ]; then
                log_error "Unknown command: $1"
            fi
            break
            ;;
    esac
done

# Execute command
case $COMMAND in
    backup)
        if [ -z "$1" ]; then
            log_error "Please specify wallet path to backup"
        fi
        backup_wallet "$1"
        ;;
    restore)
        if [ -z "$1" ]; then
            log_error "Please specify backup file to restore"
        fi
        restore_wallet "$1" "$2"
        ;;
    list)
        list_backups
        ;;
    verify)
        if [ -z "$1" ]; then
            log_error "Please specify wallet path to verify"
        fi
        verify_wallet "$1"
        ;;
    export)
        if [ -z "$1" ]; then
            log_error "Please specify wallet path to export"
        fi
        export_wallet "$1"
        ;;
    *)
        usage
        ;;
esac