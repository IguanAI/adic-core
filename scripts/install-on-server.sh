#!/bin/bash

# ADIC-Core Server Installation Script
# Run this on your server to set up ADIC environment

set -e  # Exit on error

# Configuration
ADIC_DIR="/opt/adic"
ADIC_USER="adic"
SERVICE_NAME="adic"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Functions
print_status() {
    echo -e "${GREEN}[+]${NC} $1"
}

print_error() {
    echo -e "${RED}[!]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[*]${NC} $1"
}

# Check if running as root or with sudo
if [ "$EUID" -ne 0 ]; then
    print_error "This script must be run as root or with sudo"
    exit 1
fi

print_status "Starting ADIC-Core server installation..."

# Step 1: Create service user (if not exists)
if ! id "$ADIC_USER" &>/dev/null; then
    print_status "Creating service user '$ADIC_USER'..."
    useradd -r -s /bin/false -m -d /var/lib/adic $ADIC_USER
else
    print_status "User '$ADIC_USER' already exists"
fi

# Step 2: Create directory structure
print_status "Creating directory structure..."
mkdir -p $ADIC_DIR/{bin,config,data,logs,backup}
mkdir -p /var/lib/adic

# Step 3: Set permissions
print_status "Setting directory permissions..."
chown -R $ADIC_USER:$ADIC_USER $ADIC_DIR
chmod 755 $ADIC_DIR
chmod 755 $ADIC_DIR/bin
chmod 755 $ADIC_DIR/config
chmod 755 $ADIC_DIR/data
chmod 755 $ADIC_DIR/logs
chmod 755 $ADIC_DIR/backup

# Step 4: Install dependencies
print_status "Installing system dependencies..."
apt-get update > /dev/null 2>&1
apt-get install -y curl wget jq > /dev/null 2>&1

# Step 5: Configure firewall (if UFW is installed)
if command -v ufw &> /dev/null; then
    print_status "Configuring firewall rules..."
    ufw allow 8080/tcp comment 'ADIC HTTP API' > /dev/null 2>&1 || true
    ufw allow 9000/tcp comment 'ADIC P2P TCP' > /dev/null 2>&1 || true
    ufw allow 9001/udp comment 'ADIC QUIC UDP' > /dev/null 2>&1 || true
    print_status "Firewall rules added (ports 8080, 9000, 9001)"
else
    print_warning "UFW not installed, please configure firewall manually"
fi

# Step 6: Create default configuration
print_status "Creating default configuration..."
cat > $ADIC_DIR/config/adic-config.toml << 'EOF'
[node]
data_dir = "/opt/adic/data"
validator = true
name = "adic-node-1"

[consensus]
p = 3
d = 3
rho = [2, 2, 1]
q = 3
k = 20
depth_star = 12
delta = 5
r_sum_min = 4.0
r_min = 1.0
deposit = 1.0
lambda = 1.0
beta = 0.5
mu = 1.0
gamma = 0.9

[storage]
backend = "rocksdb"
cache_size = 10000
snapshot_interval = 3600
max_snapshots = 10

[api]
enabled = true
host = "0.0.0.0"
port = 8080
max_connections = 100

[network]
enabled = true
p2p_port = 9000
quic_port = 9001
bootstrap_peers = []
dns_seeds = ["_seeds.adicl1.com"]
max_peers = 50
use_production_tls = true
enable_websocket = true

[logging]
level = "info"
format = "json"
file_output = "/opt/adic/logs/adic.log"
file_rotation_size_mb = 100
EOF

chown $ADIC_USER:$ADIC_USER $ADIC_DIR/config/adic-config.toml

# Step 7: Create startup script
print_status "Creating startup script..."
cat > $ADIC_DIR/start-adic.sh << 'EOF'
#!/bin/bash
# ADIC startup script

export RUST_LOG=info
export RUST_BACKTRACE=1

cd /opt/adic

exec ./bin/adic start \
    --config config/adic-config.toml \
    --data-dir data \
    --api-port 8080 \
    --port 9000 \
    --quic-port 9001 \
    --log-level info \
    --validator
EOF

chmod +x $ADIC_DIR/start-adic.sh
chown $ADIC_USER:$ADIC_USER $ADIC_DIR/start-adic.sh

# Step 8: Create systemd service
print_status "Creating systemd service..."
cat > /etc/systemd/system/adic.service << 'EOF'
[Unit]
Description=ADIC Core Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=adic
Group=adic
WorkingDirectory=/opt/adic
ExecStart=/opt/adic/start-adic.sh
ExecStop=/bin/kill -TERM $MAINPID
Restart=always
RestartSec=10
StandardOutput=append:/opt/adic/logs/adic.log
StandardError=append:/opt/adic/logs/adic.error.log

# Security settings
PrivateTmp=true
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/opt/adic/data /opt/adic/logs

# Resource limits
LimitNOFILE=65536
LimitNPROC=4096

# Environment
Environment="RUST_LOG=info"
Environment="RUST_BACKTRACE=1"

[Install]
WantedBy=multi-user.target
EOF

# Step 9: Create log rotation config
print_status "Setting up log rotation..."
cat > /etc/logrotate.d/adic << 'EOF'
/opt/adic/logs/*.log {
    daily
    rotate 14
    compress
    delaycompress
    missingok
    notifempty
    create 0644 adic adic
    sharedscripts
    postrotate
        systemctl reload adic > /dev/null 2>&1 || true
    endscript
}
EOF

# Step 10: Create helper scripts
print_status "Creating helper scripts..."

# Status check script
cat > $ADIC_DIR/check-status.sh << 'EOF'
#!/bin/bash
echo "=== ADIC Node Status ==="
echo ""
echo "Service status:"
systemctl status adic --no-pager | head -15
echo ""
echo "API Health:"
curl -s http://localhost:8080/health | jq '.' 2>/dev/null || echo "API not responding"
echo ""
echo "Recent logs:"
tail -5 /opt/adic/logs/adic.log
echo ""
echo "Disk usage:"
du -sh /opt/adic/data
EOF

chmod +x $ADIC_DIR/check-status.sh

# Backup script
cat > $ADIC_DIR/backup.sh << 'EOF'
#!/bin/bash
BACKUP_DIR="/opt/adic/backup"
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
BACKUP_FILE="$BACKUP_DIR/adic-backup-$TIMESTAMP.tar.gz"

echo "Creating backup at $BACKUP_FILE..."
tar czf $BACKUP_FILE -C /opt/adic data config
echo "Backup completed: $(du -h $BACKUP_FILE | cut -f1)"

# Keep only last 7 backups
ls -t $BACKUP_DIR/*.tar.gz | tail -n +8 | xargs rm -f 2>/dev/null
EOF

chmod +x $ADIC_DIR/backup.sh

# Step 11: Reload systemd
print_status "Reloading systemd daemon..."
systemctl daemon-reload

# Step 12: Create deployment user sudo rules (optional)
print_status "Setting up deployment permissions..."
cat > /etc/sudoers.d/adic-deploy << 'EOF'
# Allow deployment user to manage adic service
%sudo ALL=(ALL) NOPASSWD: /bin/systemctl start adic
%sudo ALL=(ALL) NOPASSWD: /bin/systemctl stop adic
%sudo ALL=(ALL) NOPASSWD: /bin/systemctl restart adic
%sudo ALL=(ALL) NOPASSWD: /bin/systemctl status adic
%sudo ALL=(ALL) NOPASSWD: /bin/systemctl reload adic
EOF

# Final summary
echo ""
print_status "Installation completed!"
echo -e "${GREEN}════════════════════════════════════════${NC}"
echo "ADIC-Core has been installed to: $ADIC_DIR"
echo ""
echo "Directory structure:"
echo "  Binary:    $ADIC_DIR/bin/adic"
echo "  Config:    $ADIC_DIR/config/adic-config.toml"
echo "  Data:      $ADIC_DIR/data/"
echo "  Logs:      $ADIC_DIR/logs/"
echo ""
echo "Service management:"
echo "  Start:     sudo systemctl start adic"
echo "  Stop:      sudo systemctl stop adic"
echo "  Status:    sudo systemctl status adic"
echo "  Logs:      journalctl -u adic -f"
echo ""
echo "Helper scripts:"
echo "  $ADIC_DIR/check-status.sh - Check node status"
echo "  $ADIC_DIR/backup.sh - Create backup"
echo ""
echo "Next steps:"
echo "  1. Deploy the ADIC binary to $ADIC_DIR/bin/adic"
echo "  2. Edit config at $ADIC_DIR/config/adic-config.toml"
echo "  3. Start the service: sudo systemctl start adic"
echo "  4. Enable auto-start: sudo systemctl enable adic"
echo -e "${GREEN}════════════════════════════════════════${NC}"