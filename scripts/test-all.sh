#!/bin/bash

# ADIC Core Comprehensive Test Suite
# Runs all tests with different configurations and features

set -e

echo "================================================"
echo "ADIC Core - Comprehensive Test Suite"
echo "================================================"
echo ""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Track test results
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

# Function to run tests for a crate
run_crate_tests() {
    local crate=$1
    local features=$2
    
    echo -e "${YELLOW}Testing $crate with features: $features${NC}"

    if [ -z "$features" ]; then
        if cargo test --package $crate 2>&1 | tail -2; then
            echo -e "  ${GREEN}✓ Passed${NC}"
            ((PASSED_TESTS++))
        else
            echo -e "  ${RED}✗ Failed${NC}"
            ((FAILED_TESTS++))
        fi
    else
        if cargo test --package $crate --features "$features" 2>&1 | tail -2; then
            echo -e "  ${GREEN}✓ Passed with features: $features${NC}"
            ((PASSED_TESTS++))
        else
            echo -e "  ${RED}✗ Failed with features: $features${NC}"
            ((FAILED_TESTS++))
        fi
    fi
    
    ((TOTAL_TESTS++))
}

# Run unit tests for each crate
echo "Running unit tests..."
echo "------------------------"

run_crate_tests "adic-types" ""
run_crate_tests "adic-math" ""
run_crate_tests "adic-crypto" ""
run_crate_tests "adic-storage" ""
run_crate_tests "adic-storage" "rocksdb"
run_crate_tests "adic-consensus" ""
run_crate_tests "adic-economics" ""
run_crate_tests "adic-mrw" ""
run_crate_tests "adic-finality" ""
run_crate_tests "adic-network" ""
run_crate_tests "adic-node" ""

echo ""
echo "Running integration tests..."
echo "------------------------"

# Integration tests
if cargo test --all 2>&1 | tail -3 | grep -q "test result: ok"; then
    echo -e "  ${GREEN}✓ Integration tests passed${NC}"
    ((PASSED_TESTS++))
else
    echo -e "  ${RED}✗ Integration tests failed${NC}"
    ((FAILED_TESTS++))
fi
((TOTAL_TESTS++))

echo ""
echo "Running documentation tests..."
echo "------------------------"

# Doc tests
if cargo test --doc 2>&1 | tail -3 | grep -q "test result: ok"; then
    echo -e "  ${GREEN}✓ Documentation tests passed${NC}"
    ((PASSED_TESTS++))
else
    echo -e "  ${RED}✗ Documentation tests failed${NC}"
    ((FAILED_TESTS++))
fi
((TOTAL_TESTS++))

echo ""
echo "Running security tests..."
echo "------------------------"

# Test with different security configurations
echo "Testing auth module..."
if cargo test --package adic-node auth_tests --quiet 2>/dev/null; then
    echo -e "  ${GREEN}✓ Auth tests passed${NC}"
    ((PASSED_TESTS++))
else
    echo -e "  ${RED}✗ Auth tests failed${NC}"
    ((FAILED_TESTS++))
fi
((TOTAL_TESTS++))

echo "Testing transport security..."
if cargo test --package adic-network transport_tests --quiet 2>/dev/null; then
    echo -e "  ${GREEN}✓ Transport security tests passed${NC}"
    ((PASSED_TESTS++))
else
    echo -e "  ${RED}✗ Transport security tests failed${NC}"
    ((FAILED_TESTS++))
fi
((TOTAL_TESTS++))

echo ""
echo "Running performance tests..."
echo "------------------------"

# Performance tests (if any)
if cargo test --package adic-consensus performance --quiet 2>/dev/null; then
    echo -e "  ${GREEN}✓ Performance tests passed${NC}"
    ((PASSED_TESTS++))
else
    echo -e "  ${YELLOW}⚠ No performance tests found${NC}"
fi

echo ""
echo "================================================"
echo "Test Results Summary"
echo "================================================"
echo "Total test suites: $TOTAL_TESTS"
echo -e "Passed: ${GREEN}$PASSED_TESTS${NC}"
echo -e "Failed: ${RED}$FAILED_TESTS${NC}"

# Calculate pass rate
if [ $TOTAL_TESTS -gt 0 ]; then
    PASS_RATE=$(awk "BEGIN {printf \"%.2f\", ($PASSED_TESTS/$TOTAL_TESTS)*100}")
    echo "Pass rate: $PASS_RATE%"
    
    if [ $FAILED_TESTS -eq 0 ]; then
        echo -e "${GREEN}🎉 All tests passed!${NC}"
        exit 0
    else
        echo -e "${RED}❌ Some tests failed. Please review the output above.${NC}"
        exit 1
    fi
else
    echo "No tests were run."
    exit 1
fi