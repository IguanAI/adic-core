# Multi-stage build for ADIC Core
FROM rust:1.89.0-slim as builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    libclang-dev \
    protobuf-compiler \
    cmake \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /usr/src/adic-core

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY config/ ./config/

# Build in debug mode for testnet (allows use_production_tls=false for self-signed certs)
# For production/mainnet, change this to: cargo build --release --bin adic
RUN cargo build --bin adic

# Runtime stage
FROM debian:trixie-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    libssl-dev \
    curl \
    build-essential \
    pkg-config \
    libclang-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 adic

# Copy binary from builder (debug mode for testnet)
COPY --from=builder /usr/src/adic-core/target/debug/adic /usr/local/bin/adic

# Create data directory
RUN mkdir -p /data && chown adic:adic /data

# Switch to non-root user
USER adic

# Set working directory
WORKDIR /data

# Expose ports
EXPOSE 8080 9000

# Default command
CMD ["adic", "start", "--data-dir", "/data", "--api-port", "8080", "--port", "9000", "--validator"]