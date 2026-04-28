#!/bin/bash

# Test script for Google search flow with FSM and policy validation
# This tests that the LLM correctly types and presses Enter instead of constructing URLs

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
LOG_DIR="$ROOT_DIR/test-results/manual"
LOG_FILE="$LOG_DIR/test_google_flow.log"
mkdir -p "$LOG_DIR"
cd "$ROOT_DIR"

echo "=== Testing Google Search Flow with FSM/Policy ==="
echo "This test will search for 'OpenAI' on Google"
echo ""

# Set correlation ID for tracking
export CORRELATION_ID="test-google-$(date +%s)"
echo "Correlation ID: $CORRELATION_ID"
echo ""

# Enable debug logging
export RUST_LOG=debug
export OPENAI_API_KEY="${OPENAI_API_KEY:-dummy-key-for-testing}"

# Build the project
echo "Building project..."
cargo build --release -p rzn-browser -p rzn-browser-worker -p rzn-native-host 2>&1 | tail -5

# Start the test
echo ""
echo "Starting autonomous search test..."
echo "Expected behavior:"
echo "1. Navigate to google.com"
echo "2. Type 'OpenAI' in search box"
echo "3. Press Enter (NOT construct URL)"
echo "4. Extract results"
echo ""

# Run the autonomous flow
./target/release/rzn-browser plan-llm "Search Google for OpenAI and extract the first 3 results" 2>&1 | tee "$LOG_FILE"

# Check for policy violations
echo ""
echo "=== Checking for policy violations ==="
grep -i "policy violation" "$LOG_FILE" && echo "❌ FAILED: Policy violations detected" || echo "✅ PASSED: No policy violations"

# Check for URL construction
echo ""
echo "=== Checking for URL construction ==="
grep -E "google\.com/search\?q=" "$LOG_FILE" && echo "❌ FAILED: URL construction detected" || echo "✅ PASSED: No URL construction"

# Check for press_key/Enter usage
echo ""
echo "=== Checking for Enter key press ==="
grep -E "(press_key|press).*Enter" "$LOG_FILE" && echo "✅ PASSED: Enter key press detected" || echo "❌ FAILED: No Enter key press"

# Check FSM state transitions
echo ""
echo "=== Checking FSM state transitions ==="
grep -E "State transition.*Search.*Results" "$LOG_FILE" && echo "✅ PASSED: FSM transition detected" || echo "⚠️  WARNING: FSM transition not detected"

# Check for extracted data
echo ""
echo "=== Checking for extracted data ==="
grep -i "extracted.*data" "$LOG_FILE" && echo "✅ PASSED: Data extraction detected" || echo "❌ FAILED: No data extraction"

# Show correlation ID logs if they exist
LOG_FILE="/tmp/llm_raw_${CORRELATION_ID}.jsonl"
if [ -f "$LOG_FILE" ]; then
    echo ""
    echo "=== Raw LLM logs saved to: $LOG_FILE ==="
    echo "First request:"
    head -1 "$LOG_FILE" | jq '.data.tools[0]' 2>/dev/null || head -1 "$LOG_FILE"
fi

echo ""
echo "=== Test Complete ==="
echo "Check $LOG_FILE for full details"
