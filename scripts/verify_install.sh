#!/bin/bash
# Verification script for README instructions

set -e

echo "=== ADIC Core Installation Verification ==="
echo ""

# Check if running in CI or local
if [ -n "$CI" ]; then
    echo "Running in CI environment"
fi

# 1. Check Rust version
echo "1. Checking Rust installation..."
if command -v rustc &> /dev/null; then
    RUST_VERSION=$(rustc --version | cut -d' ' -f2)
    echo "   ✓ Rust installed: $RUST_VERSION"
    
    # Check if version is >= 1.70
    MIN_VERSION="1.70.0"
    if [ "$(printf '%s\n' "$MIN_VERSION" "$RUST_VERSION" | sort -V | head -n1)" = "$MIN_VERSION" ]; then
        echo "   ✓ Rust version meets requirement (>= 1.70)"
    else
        echo "   ✗ Rust version too old (need >= 1.70)"
        exit 1
    fi
else
    echo "   ✗ Rust not found. Install from https://rustup.rs/"
    exit 1
fi

# 2. Check system dependencies
echo ""
echo "2. Checking system dependencies..."

check_command() {
    if command -v $1 &> /dev/null; then
        echo "   ✓ $1 found"
        return 0
    else
        echo "   ✗ $1 not found"
        return 1
    fi
}

MISSING_DEPS=0

# Check for essential build tools
check_command gcc || check_command clang || MISSING_DEPS=1
check_command pkg-config || MISSING_DEPS=1
check_command protoc || echo "   ⚠ protoc not found (optional but recommended)"

# 3. Check if project builds
echo ""
echo "3. Checking if project builds..."

if [ -f "target/release/adic" ]; then
    echo "   ✓ Binary already built"
else
    echo "   Building project (this may take a while)..."
    if cargo build --release --quiet 2>/dev/null; then
        echo "   ✓ Project builds successfully"
    else
        echo "   ✗ Build failed. Check dependencies."
        exit 1
    fi
fi

# 4. Verify binary works
echo ""
echo "4. Verifying binary functionality..."

if ./target/release/adic --help > /dev/null 2>&1; then
    echo "   ✓ Binary runs successfully"
else
    echo "   ✗ Binary failed to run"
    exit 1
fi

# 5. Test README commands
echo ""
echo "5. Testing README commands..."

# Test keygen
if ./target/release/adic keygen --output test.key 2>/dev/null; then
    if [ -f test.key ]; then
        echo "   ✓ keygen command works"
        rm -f test.key
    else
        echo "   ✗ keygen didn't create key file"
    fi
else
    echo "   ✗ keygen command failed"
fi

# Test init
if ./target/release/adic init --params v1 --output ./test_config 2>/dev/null; then
    if [ -f ./test_config/adic-config.toml ]; then
        echo "   ✓ init command works"
        rm -rf ./test_config
    else
        echo "   ✗ init didn't create config"
    fi
else
    echo "   ✗ init command failed"
fi

# Test test command
if ./target/release/adic test --count 2 2>/dev/null; then
    echo "   ✓ test command works"
else
    echo "   ✗ test command failed"
fi

# 6. Quick API test (if requested)
if [ "$1" = "--with-api" ]; then
    echo ""
    echo "6. Testing API endpoints..."
    
    # Start node in background
    ./target/release/adic start --data-dir ./test_data --api-port 8899 > /dev/null 2>&1 &
    NODE_PID=$!
    
    echo "   Waiting for node to start..."
    sleep 5
    
    # Test health endpoint
    if curl -s -f http://localhost:8899/health > /dev/null 2>&1; then
        echo "   ✓ Health endpoint responds"
    else
        echo "   ✗ Health endpoint failed"
    fi
    
    # Test status endpoint
    if curl -s -f http://localhost:8899/status > /dev/null 2>&1; then
        echo "   ✓ Status endpoint responds"
    else
        echo "   ✗ Status endpoint failed"
    fi
    
    # Clean up
    kill $NODE_PID 2>/dev/null || true
    rm -rf ./test_data
fi

# Summary
echo ""
echo "=== Verification Summary ==="

if [ $MISSING_DEPS -eq 0 ]; then
    echo "✅ All checks passed! Installation instructions are correct."
    echo ""
    echo "You can now run:"
    echo "  ./target/release/adic start"
else
    echo "⚠️  Some dependencies are missing. Please install them according to the README."
fi

exit $MISSING_DEPS