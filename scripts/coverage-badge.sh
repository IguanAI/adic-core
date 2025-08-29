#!/bin/bash

# Generate coverage badge for README
# This script generates a coverage percentage and creates a badge URL

set -e

# Run coverage quietly
echo "Calculating coverage..."
cargo tarpaulin --print-summary --quiet 2>/dev/null | grep "Coverage" | awk '{print $2}' | sed 's/%//' > /tmp/coverage.txt || echo "0" > /tmp/coverage.txt

COVERAGE=$(cat /tmp/coverage.txt)

# Determine color based on coverage
if (( $(echo "$COVERAGE < 50" | bc -l) )); then
    COLOR="red"
elif (( $(echo "$COVERAGE < 70" | bc -l) )); then
    COLOR="yellow"
elif (( $(echo "$COVERAGE < 90" | bc -l) )); then
    COLOR="green"
else
    COLOR="brightgreen"
fi

# Generate badge URL (using shields.io)
BADGE_URL="https://img.shields.io/badge/coverage-${COVERAGE}%25-${COLOR}"

echo "Coverage: ${COVERAGE}%"
echo "Badge URL: ${BADGE_URL}"
echo ""
echo "Add this to your README.md:"
echo "![Coverage](${BADGE_URL})"

# Optionally update README.md automatically
if [ "$1" == "--update-readme" ]; then
    if [ -f "README.md" ]; then
        # Check if coverage badge already exists
        if grep -q "img.shields.io/badge/coverage" README.md; then
            # Update existing badge
            sed -i "s|https://img.shields.io/badge/coverage-[0-9.]*%25-[a-z]*|${BADGE_URL}|g" README.md
            echo "README.md updated with new coverage badge"
        else
            echo "Coverage badge not found in README.md"
            echo "Add the following line where you want the badge:"
            echo "![Coverage](${BADGE_URL})"
        fi
    fi
fi

rm /tmp/coverage.txt