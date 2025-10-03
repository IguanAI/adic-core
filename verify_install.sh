#!/bin/bash
set -e

echo "🔍 Verifying ADIC installation..."
echo ""

echo "Step 1: Building release binary..."
cargo build --release
echo "✅ Build successful"
echo ""

echo "Step 2: Testing CLI help..."
./target/release/adic --help > /dev/null
echo "✅ CLI help works"
echo ""

echo "Step 3: Testing keygen..."
./target/release/adic keygen --output /tmp/test_adic.key > /dev/null
echo "✅ Keygen works"
echo ""

echo "Step 4: Testing init command..."
./target/release/adic init --params v1 > /dev/null
echo "✅ Init works"
echo ""

echo "Step 5: Running test command..."
./target/release/adic test --count 2
echo "✅ Test works"
echo ""

# Cleanup
rm -f /tmp/test_adic.key

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ Installation verified successfully!"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
