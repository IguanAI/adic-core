#!/bin/bash

set -e

echo "Building ADIC Core..."
echo "====================="

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Check for required tools
check_command() {
    if ! command -v $1 &> /dev/null; then
        echo -e "${RED}Error: $1 is not installed${NC}"
        exit 1
    fi
}

echo "Checking prerequisites..."
check_command cargo
check_command rustc

echo -e "${GREEN}✓ Prerequisites satisfied${NC}"

# Build the project
echo ""
echo "Building Rust workspace..."
cargo build --all 2>&1 | while IFS= read -r line; do
    if [[ $line == *"error"* ]]; then
        echo -e "${RED}$line${NC}"
    elif [[ $line == *"warning"* ]]; then
        echo -e "${YELLOW}$line${NC}"
    else
        echo "$line"
    fi
done

# Check if build succeeded
if [ ${PIPESTATUS[0]} -eq 0 ]; then
    echo -e "${GREEN}✓ Build successful!${NC}"
else
    echo -e "${RED}✗ Build failed${NC}"
    exit 1
fi

# Run tests
echo ""
echo "Running tests..."
cargo test --all --quiet

if [ $? -eq 0 ]; then
    echo -e "${GREEN}✓ All tests passed!${NC}"
else
    echo -e "${RED}✗ Some tests failed${NC}"
    exit 1
fi

echo ""
echo -e "${GREEN}ADIC Core build completed successfully!${NC}"
echo ""
echo "Next steps:"
echo "  1. Run 'cargo run --bin adic-node' to start a node"
echo "  2. Check documentation with 'cargo doc --open'"
echo "  3. Run benchmarks with 'cargo bench'"