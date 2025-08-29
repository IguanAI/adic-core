#!/bin/bash
# Validate GitHub Actions workflows before pushing

set -e

echo "=== GitHub Actions Workflow Validation ==="
echo ""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Check if workflows directory exists
if [ ! -d ".github/workflows" ]; then
  echo -e "${RED}❌ .github/workflows directory not found${NC}"
  exit 1
fi

echo "📁 Found workflows directory"
echo ""

# List all workflow files
echo "📄 Workflow files found:"
for file in .github/workflows/*.yml; do
  echo "   - $(basename $file)"
done
echo ""

# Validate YAML syntax
echo "🔍 Validating YAML syntax..."
YAML_VALID=true
for file in .github/workflows/*.yml; do
  echo -n "   Checking $(basename $file)... "
  if python3 -c "import yaml; yaml.safe_load(open('$file'))" 2>/dev/null; then
    echo -e "${GREEN}✅ Valid${NC}"
  else
    echo -e "${RED}❌ Invalid YAML${NC}"
    python3 -c "import yaml; yaml.safe_load(open('$file'))" 2>&1 | head -5
    YAML_VALID=false
  fi
done

if [ "$YAML_VALID" = false ]; then
  echo -e "${RED}❌ YAML validation failed${NC}"
  exit 1
fi
echo ""

# Check for deprecated action versions
echo "🔍 Checking for deprecated actions..."
DEPRECATED_FOUND=false

# Check for v1, v2, v3 versions of commonly updated actions
for file in .github/workflows/*.yml; do
  echo "   Checking $(basename $file)..."
  
  # Check for old upload-artifact versions
  if grep -q "actions/upload-artifact@v[123]" "$file"; then
    echo -e "      ${YELLOW}⚠️  Found deprecated upload-artifact version (should be v4)${NC}"
    DEPRECATED_FOUND=true
  fi
  
  # Check for old cache versions
  if grep -q "actions/cache@v[123]" "$file"; then
    echo -e "      ${YELLOW}⚠️  Found deprecated cache version (should be v4)${NC}"
    DEPRECATED_FOUND=true
  fi
  
  # Check for old checkout versions
  if grep -q "actions/checkout@v[123]" "$file"; then
    echo -e "      ${YELLOW}⚠️  Found old checkout version (v4 is latest)${NC}"
    DEPRECATED_FOUND=true
  fi
  
  # Check for old codecov versions
  if grep -q "codecov/codecov-action@v[123]" "$file"; then
    echo -e "      ${YELLOW}⚠️  Found old codecov-action version (should be v4)${NC}"
    DEPRECATED_FOUND=true
  fi
done

if [ "$DEPRECATED_FOUND" = false ]; then
  echo -e "   ${GREEN}✅ No deprecated actions found${NC}"
fi
echo ""

# Check for required secrets usage
echo "🔍 Checking for secret usage..."
if grep -r "CODECOV_TOKEN" .github/workflows/*.yml > /dev/null; then
  echo -e "   ${YELLOW}ℹ️  CODECOV_TOKEN required - make sure to add it in GitHub settings${NC}"
fi
if grep -r "GITHUB_TOKEN" .github/workflows/*.yml > /dev/null; then
  echo -e "   ${GREEN}✅ GITHUB_TOKEN used (automatically provided by GitHub)${NC}"
fi
echo ""

# Check workflow triggers
echo "🔍 Checking workflow triggers..."
for file in .github/workflows/*.yml; do
  echo "   $(basename $file):"
  if grep -q "on:" "$file"; then
    grep -A 5 "^on:" "$file" | grep -E "push:|pull_request:|schedule:|workflow_dispatch:" | sed 's/^/      - /'
  fi
done
echo ""

# Test that common commands work locally
echo "🧪 Testing workflow commands locally..."
echo ""

# Test format check
echo -n "   Testing: cargo fmt --all -- --check ... "
if cargo fmt --all -- --check 2>/dev/null; then
  echo -e "${GREEN}✅ Pass${NC}"
else
  echo -e "${RED}❌ Failed - run 'cargo fmt --all' to fix${NC}"
fi

# Test clippy
echo -n "   Testing: cargo clippy --all-targets -- -D warnings ... "
if cargo clippy --all-targets -- -D warnings 2>/dev/null; then
  echo -e "${GREEN}✅ Pass${NC}"
else
  echo -e "${YELLOW}⚠️  Warnings found${NC}"
fi

# Test build
echo -n "   Testing: cargo check --all ... "
if cargo check --all 2>/dev/null; then
  echo -e "${GREEN}✅ Pass${NC}"
else
  echo -e "${RED}❌ Build check failed${NC}"
fi

# Test that unit tests compile
echo -n "   Testing: cargo test --lib --all --no-run ... "
if cargo test --lib --all --no-run 2>/dev/null; then
  echo -e "${GREEN}✅ Tests compile${NC}"
else
  echo -e "${RED}❌ Test compilation failed${NC}"
fi

echo ""
echo "=== Summary ==="
echo ""

if [ "$YAML_VALID" = true ] && [ "$DEPRECATED_FOUND" = false ]; then
  echo -e "${GREEN}✅ All workflow files are valid and ready to push!${NC}"
  echo ""
  echo "Next steps:"
  echo "1. Commit the workflows: git add .github/workflows/"
  echo "2. Push to a test branch first: git push origin test-workflows"
  echo "3. Check the Actions tab on GitHub"
  echo "4. If successful, merge to main"
else
  echo -e "${YELLOW}⚠️  Some issues found - review above${NC}"
fi

echo ""
echo "=== Workflow Statistics ==="
WORKFLOW_COUNT=$(ls -1 .github/workflows/*.yml 2>/dev/null | wc -l)
JOB_COUNT=$(grep -h "^\s*[a-zA-Z_-]*:\s*$" .github/workflows/*.yml | grep -v "steps:" | wc -l)
echo "   📊 Total workflows: $WORKFLOW_COUNT"
echo "   📊 Total jobs defined: ~$JOB_COUNT"