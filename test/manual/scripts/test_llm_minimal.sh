#!/bin/bash

# Minimal LLM test - uses the least API credits possible
# This script tests basic functionality with minimal token usage

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
SCRIPT_PATH="./test/manual/scripts/test_llm_minimal.sh"
cd "$ROOT_DIR"

echo "============================================"
echo "RZN LLM MODE - MINIMAL API CREDIT TEST"
echo "============================================"
echo ""
echo "This test uses minimal API credits (< $0.01)"
echo ""

# Check for API key
if [ -z "$1" ]; then
    echo "Usage: ${SCRIPT_PATH} 'your-api-key'"
    echo ""
    echo "This will run a simple test that:"
    echo "  1. Navigates to example.com (no API call)"
    echo "  2. Extracts the heading (1 small API call)"
    echo "  3. Completes immediately"
    echo ""
    echo "Estimated cost: < $0.001 (less than 1/10th of a cent)"
    exit 1
fi

export OPENAI_API_KEY="$1"
echo "✓ API key set"
echo ""

# Kill any existing processes
pkill -f rzn-browser 2>/dev/null
pkill -f rzn-native-host 2>/dev/null
sleep 1

echo "Starting minimal test..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Run the simplest possible task - just navigate and extract
timeout 30 ./target/release/rzn-browser llm-auto \
    "Go to example.com and tell me the text of the main heading only" \
    --max-steps 3 2>&1 | tee /tmp/llm_test.log

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Check if it worked
if grep -q "Example Domain" /tmp/llm_test.log; then
    echo "✅ SUCCESS! LLM mode is working correctly"
    echo "   The system successfully:"
    echo "   - Connected to the native host bridge"
    echo "   - Navigated to example.com"
    echo "   - Used LLM to extract the heading"
    echo "   - Returned 'Example Domain'"
else
    echo "⚠️  Test may have issues. Checking logs..."
    echo ""
    echo "Last few error messages:"
    grep -i "error\|failed" /tmp/llm_test.log | tail -5
fi

echo ""
echo "Full log saved to: /tmp/llm_test.log"
echo "View with: cat /tmp/llm_test.log"
echo ""
echo "Token usage estimate: ~500 tokens (~$0.0001)"
