# Dockerfile additions for wallet support
# Add this to your existing ADIC Dockerfile

# Install additional dependencies
RUN cargo add sha2 --package adic-node

# Ensure wallet modules are included in build
COPY crates/adic-node/src/wallet.rs crates/adic-node/src/
COPY crates/adic-node/src/genesis.rs crates/adic-node/src/
COPY crates/adic-node/src/api_wallet.rs crates/adic-node/src/

# Create data directory for wallet storage
RUN mkdir -p /data

# Set environment for wallet
ENV ADIC_WALLET_ENABLED=true
ENV ADIC_GENESIS_AUTOCONFIG=true

# Volume for persistent wallet storage
VOLUME ["/data"]

# Health check includes wallet endpoint
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD curl -f http://localhost:8080/wallet/info || exit 1