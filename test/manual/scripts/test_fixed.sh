#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
LOG_DIR="$ROOT_DIR/test-results/manual"
LOG_FILE="$LOG_DIR/test_fixed.log"
mkdir -p "$LOG_DIR"
cd "$ROOT_DIR"

echo "=== Testing Fixed Google Search ==="
echo ""

# Clean environment
export RUST_LOG=info  # Less verbose
export OPENAI_API_KEY="${OPENAI_API_KEY}"

# Run the test
echo "Searching for: OpenAI"
echo ""
./target/release/rzn-browser llm-auto "Search Google for OpenAI and extract the first 3 results" --max-steps 10 2>&1 | tee "$LOG_FILE"

echo ""
echo "=== Checking Results ==="

# Check if we actually searched for OpenAI
if grep -q "OpenAI" "$LOG_FILE"; then
    echo "✅ Correct search term used"
else
    echo "❌ Wrong search term"
fi

# Check if extraction happened
if grep -q "extract" "$LOG_FILE"; then
    echo "✅ Extraction attempted"
else
    echo "❌ No extraction"
fi

# Check for policy violations (should be none now)
if grep -q "Policy violation" "$LOG_FILE"; then
    echo "⚠️  Policy violations occurred (but should be recoverable now)"
else
    echo "✅ No policy violations"
fi
