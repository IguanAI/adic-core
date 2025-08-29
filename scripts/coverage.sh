#!/bin/bash

# ADIC Core Test Coverage Script
# This script runs tests with coverage measurement using cargo-tarpaulin

set -e

echo "================================================"
echo "ADIC Core - Test Coverage Report"
echo "================================================"

# Check if cargo-tarpaulin is installed
if ! command -v cargo-tarpaulin &> /dev/null; then
    echo "Installing cargo-tarpaulin..."
    cargo install cargo-tarpaulin
fi

# Clean previous coverage data
echo "Cleaning previous coverage data..."
rm -rf target/coverage
rm -f tarpaulin-report.html
rm -f cobertura.xml
rm -f lcov.info

# Run tests with coverage for all packages
echo "Running tests with coverage measurement..."

# Run tarpaulin with comprehensive options
cargo tarpaulin \
    --all \
    --all-features \
    --workspace \
    --timeout 300 \
    --out Html \
    --out Lcov \
    --out Xml \
    --output-dir target/coverage \
    --exclude-files "*/tests/*" \
    --exclude-files "*/target/*" \
    --exclude-files "*/node_modules/*" \
    --ignore-panics \
    --verbose \
    --color always \
    --print-summary \
    --print-source-files \
    || true

# Generate detailed report per crate
echo ""
echo "================================================"
echo "Per-Crate Coverage Summary:"
echo "================================================"

for crate in crates/*/; do
    if [ -d "$crate" ]; then
        crate_name=$(basename "$crate")
        echo ""
        echo "Testing $crate_name..."
        
        cd "$crate"
        
        # Run coverage for individual crate
        cargo tarpaulin \
            --all-features \
            --timeout 120 \
            --print-summary \
            --skip-clean \
            --quiet \
            2>/dev/null | grep -E "Coverage|%" || echo "  No tests found"
        
        cd ../..
    fi
done

echo ""
echo "================================================"
echo "Coverage Report Generated!"
echo "================================================"
echo "HTML Report: target/coverage/tarpaulin-report.html"
echo "LCOV Report: target/coverage/lcov.info"
echo "XML Report:  target/coverage/cobertura.xml"
echo ""

# Show overall coverage
if [ -f "target/coverage/lcov.info" ]; then
    # Calculate total coverage from lcov.info
    total_lines=$(grep -c "^DA:" target/coverage/lcov.info || echo "0")
    covered_lines=$(grep "^DA:" target/coverage/lcov.info | grep -v ",0$" | wc -l || echo "0")
    
    if [ "$total_lines" -gt 0 ]; then
        coverage=$(awk "BEGIN {printf \"%.2f\", ($covered_lines/$total_lines)*100}")
        echo "Overall Coverage: $coverage%"
        
        # Set color based on coverage percentage
        if (( $(echo "$coverage < 50" | bc -l) )); then
            echo "⚠️  Warning: Coverage is below 50%"
        elif (( $(echo "$coverage < 70" | bc -l) )); then
            echo "📊 Coverage is moderate (50-70%)"
        elif (( $(echo "$coverage < 90" | bc -l) )); then
            echo "✅ Good coverage (70-90%)"
        else
            echo "🎉 Excellent coverage (90%+)!"
        fi
    fi
fi

echo "================================================"