#!/bin/bash

# Basic LLM Autonomous Test Script
# This demonstrates the LLM autonomous capabilities

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
SCRIPT_PATH="./test/manual/scripts/test_llm_basic.sh"
cd "$ROOT_DIR"

echo "============================================"
echo "RZN LLM AUTONOMOUS MODE - BASIC TEST"
echo "============================================"
echo ""

# Check if API key is provided as argument or environment variable
if [ -n "$1" ]; then
    export OPENAI_API_KEY="$1"
    echo "✓ Using provided API key"
elif [ -n "$OPENAI_API_KEY" ]; then
    echo "✓ Using existing OPENAI_API_KEY from environment"
else
    echo "⚠️  No API key provided. The system will use fallback mode."
    echo "   To use full LLM capabilities, run:"
    echo "   ${SCRIPT_PATH} 'your-api-key'"
    echo ""
    export OPENAI_API_KEY="sk-test-dummy-key"
fi

echo ""
echo "Testing basic autonomous tasks..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Test 1: Simple navigation and extraction
echo "📝 Test 1: Navigate to a website and extract information"
echo "   Task: 'Go to example.com and tell me what the main heading says'"
echo ""
./target/release/rzn-browser llm-auto "Go to example.com and tell me what the main heading says" --max-steps 5

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Test 2: Search and extract
echo "📝 Test 2: Search for information"
echo "   Task: 'Search for OpenAI on Google and get the first result'"
echo ""
./target/release/rzn-browser llm-auto "Search for OpenAI on Google and tell me what the first search result is" --max-steps 10

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ Basic tests complete"
echo ""
echo "📊 To monitor execution in real-time:"
echo "   tail -f ~/rzn_build.log | jq '.message'"
echo ""
echo "📄 To see LLM interactions:"
echo "   cat /tmp/llm_raw_*.jsonl | jq ."
