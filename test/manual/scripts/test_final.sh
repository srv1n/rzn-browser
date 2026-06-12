#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
LOG_DIR="$ROOT_DIR/test-results/manual"
LOG_FILE="$LOG_DIR/final_test.log"
mkdir -p "$LOG_DIR"
cd "$ROOT_DIR"

echo "🔧 FINAL TEST: Sam's Implementation with All Fixes"
echo "=================================================="
echo ""

# Set environment
export RUST_LOG=info
export OPENAI_API_KEY="${OPENAI_API_KEY}"

if [ -z "$OPENAI_API_KEY" ]; then
    echo "❌ OPENAI_API_KEY required for this test"
    exit 1
fi

echo "📋 Testing: Search Google for OpenAI and extract results"
echo "🎯 Expected behavior:"
echo "   1. Navigate to google.com ✅"
echo "   2. FSM: Bootstrap → Search"
echo "   3. Type 'OpenAI' in search box"  
echo "   4. Press Enter (using CDP)"
echo "   5. FSM: Search → Results"
echo "   6. Extract search results"
echo "   7. No policy violations"
echo "   8. No URL construction"
echo ""

# Run the test
echo "🚀 Starting test..."
./target/release/rzn-browser llm-auto "Search Google for OpenAI and extract the first 3 results" --max-steps 8 2>&1 | tee "$LOG_FILE"

echo ""
echo "📊 RESULTS ANALYSIS"
echo "==================="

# Check critical success indicators
echo ""
echo "✅ SUCCESS INDICATORS:"

if grep -q "Search Google for OpenAI" "$LOG_FILE"; then
    echo "   ✅ Correct search query used"
else
    echo "   ❌ Wrong search query"
fi

if grep -q "State transition.*Bootstrap.*Search" "$LOG_FILE"; then
    echo "   ✅ FSM: Bootstrap → Search transition"
else
    echo "   ⚠️  FSM transition not logged"
fi

if grep -q "State transition.*Search.*Results" "$LOG_FILE"; then
    echo "   ✅ FSM: Search → Results transition"
else
    echo "   ❌ Missing Search → Results transition"
fi

if grep -q "Valid search pattern detected.*type.*press_key" "$LOG_FILE"; then
    echo "   ✅ Policy: Type + press_key pattern validated"
else
    echo "   ⚠️  Search pattern validation not logged"
fi

if grep -q "Enhanced press_special_key" "$LOG_FILE"; then
    echo "   ✅ CDP: Enhanced press_key handler used"
else
    echo "   ⚠️  CDP handler not used"
fi

# Check for errors/failures
echo ""
echo "❌ FAILURE INDICATORS:"

if grep -q "POLICY VIOLATION" "$LOG_FILE"; then
    echo "   ❌ POLICY VIOLATIONS DETECTED:"
    grep "POLICY VIOLATION" "$LOG_FILE" | head -3
else
    echo "   ✅ No policy violations"
fi

if grep -q "google.com/search?q=" "$LOG_FILE"; then
    echo "   ❌ URL CONSTRUCTION DETECTED"
else
    echo "   ✅ No URL construction"
fi

if grep -q "KeyboardEvent.*error\|TypeError.*KeyboardEvent" "$LOG_FILE"; then
    echo "   ❌ KeyboardEvent errors detected"
else
    echo "   ✅ No KeyboardEvent errors"
fi

# Check extraction
echo ""
if grep -q "extracted_data\|extract.*success" "$LOG_FILE"; then
    echo "🎯 DATA EXTRACTION: ✅ Success"
else
    echo "🎯 DATA EXTRACTION: ❌ Failed"
fi

# Overall result
echo ""
echo "📈 OVERALL STATUS:"
violations=$(grep -c "POLICY VIOLATION" "$LOG_FILE" 2>/dev/null || echo "0")
errors=$(grep -c "ERROR\|Failed\|Error" "$LOG_FILE" 2>/dev/null || echo "0")

if [ "$violations" -eq 0 ] && [ "$errors" -lt 3 ]; then
    echo "   🎉 TEST PASSED: Sam's implementation working correctly"
    echo "   📊 Policy violations: $violations"
    echo "   📊 Errors: $errors"
else
    echo "   ❌ TEST FAILED: Implementation has issues"
    echo "   📊 Policy violations: $violations"
    echo "   📊 Errors: $errors"
fi

echo ""
echo "📄 Full log saved to: $LOG_FILE"
echo "🔍 Review with: grep -E '(State transition|POLICY|ERROR|extract)' \"$LOG_FILE\""
