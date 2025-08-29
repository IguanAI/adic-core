# Multi-stage build for ADIC Core
FROM rust:1.75-slim as builder

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

# Build in release mode
RUN cargo build --release --bin adic

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 adic

# Copy binary from builder
COPY --from=builder /usr/src/adic-core/target/release/adic /usr/local/bin/adic

# Create data directory
RUN mkdir -p /data && chown adic:adic /data

# Switch to non-root user
USER adic

# Set working directory
WORKDIR /data

# Expose ports
EXPOSE 8080 9000

# Default command
CMD ["adic", "start", "--data-dir", "/data", "--api-port", "8080", "--port", "9000"]