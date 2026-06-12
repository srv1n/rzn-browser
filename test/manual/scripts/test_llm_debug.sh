#!/bin/bash

# Debug test for LLM mode - captures detailed logs for analysis
# Run this on your machine with your API key and share the output

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
SCRIPT_PATH="./test/manual/scripts/test_llm_debug.sh"
cd "$ROOT_DIR"

echo "============================================"
echo "RZN LLM MODE - DEBUG TEST"
echo "============================================"
echo ""
echo "This test captures detailed logs for debugging"
echo ""

# Check for API key
if [ -z "$1" ]; then
    echo "Usage: ${SCRIPT_PATH} 'your-api-key'"
    echo ""
    echo "This will:"
    echo "  1. Run a simple LLM task"
    echo "  2. Capture all debug logs"
    echo "  3. Create a summary file you can share"
    exit 1
fi

export OPENAI_API_KEY="$1"
export RUST_LOG=debug  # Enable debug logging
export RUST_BACKTRACE=1  # Include backtraces on errors

echo "✓ API key set"
echo "✓ Debug logging enabled"
echo ""

# Clean up old logs
rm -f /tmp/rzn_debug_*.log
rm -f /tmp/llm_raw_*.jsonl

# Kill any existing processes
pkill -f rzn-browser 2>/dev/null
pkill -f rzn-native-host 2>/dev/null
sleep 1

echo "Running debug test..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Run with timeout and capture everything
timeout 30 ./target/release/rzn-browser llm-auto \
    "Navigate to example.com and extract the main heading" \
    --max-steps 3 2>&1 | tee /tmp/rzn_debug_main.log

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Capture recent runtime logs
tail -100 ~/rzn_build.log > /tmp/rzn_debug_broker.log 2>/dev/null

# Find LLM raw logs
LLM_LOG=$(ls -t /tmp/llm_raw_*.jsonl 2>/dev/null | head -1)
if [ -n "$LLM_LOG" ]; then
    cp "$LLM_LOG" /tmp/rzn_debug_llm.jsonl
    echo "✓ Captured LLM interaction log"
fi

# Create summary
echo "Creating debug summary..."
cat > /tmp/rzn_debug_summary.txt << EOF
=== RZN LLM Debug Summary ===
Date: $(date)
Test: Navigate to example.com and extract heading

=== Success Indicators ===
$(grep -c "Successfully connected to broker" /tmp/rzn_debug_main.log) - Runtime bridge connections
$(grep -c "Starting LLM autonomous execution" /tmp/rzn_debug_main.log) - LLM starts
$(grep -c "Example Domain" /tmp/rzn_debug_main.log) - Found target text
$(grep -c "State transition" /tmp/rzn_debug_main.log) - FSM transitions

=== Errors/Warnings ===
$(grep -i "error" /tmp/rzn_debug_main.log | wc -l) - Error messages
$(grep -i "failed" /tmp/rzn_debug_main.log | wc -l) - Failure messages
$(grep -i "timeout" /tmp/rzn_debug_main.log | wc -l) - Timeouts

=== Key Events ===
$(grep "State transition\|Tool call\|Executing step" /tmp/rzn_debug_main.log | head -10)

=== Last Error (if any) ===
$(grep -i "error" /tmp/rzn_debug_main.log | tail -3)
EOF

echo ""
echo "📊 TEST RESULTS:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cat /tmp/rzn_debug_summary.txt
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Check success
if grep -q "Example Domain" /tmp/rzn_debug_main.log; then
    echo "✅ Test appears SUCCESSFUL!"
else
    echo "⚠️  Test may have issues"
fi

echo ""
echo "📁 Debug files created:"
echo "  /tmp/rzn_debug_summary.txt - Quick summary (share this)"
echo "  /tmp/rzn_debug_main.log - Full execution log"
echo "  /tmp/rzn_debug_broker.log - Runtime logs"
echo "  /tmp/rzn_debug_llm.jsonl - LLM interactions (if any)"
echo ""
echo "To share with developer:"
echo "  cat /tmp/rzn_debug_summary.txt"
echo ""
echo "For detailed analysis:"
echo "  cat /tmp/rzn_debug_main.log | grep -A2 -B2 'error'"
