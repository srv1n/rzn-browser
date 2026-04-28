#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
SCRIPT_PATH="./test/manual/scripts/test_llm_demo.sh"
cd "$ROOT_DIR"

echo "============================================"
echo "RZN LLM AUTONOMOUS MODE - DEMO"
echo "============================================"
echo ""
echo "This demonstrates how LLM autonomous mode works."
echo ""

# Check if user wants to use their API key
if [ -n "$1" ]; then
    export OPENAI_API_KEY="$1"
    echo "✓ Using provided OpenAI API key"
    echo ""
    echo "Running real LLM autonomous task..."
    ./target/release/rzn-browser llm-auto "Go to example.com and extract the main heading" --max-steps 5
else
    echo "ℹ️  No API key provided. Showing what would happen with a valid key:"
    echo ""
    echo "The system would:"
    echo "1. Navigate to example.com"
    echo "2. Analyze the page using DOM capture"
    echo "3. Use LLM to understand the page structure"
    echo "4. Extract the main heading (usually 'Example Domain')"
    echo "5. Return the result"
    echo ""
    echo "To run with a real API key:"
    echo "  ${SCRIPT_PATH} 'your-openai-api-key'"
    echo ""
    echo "Or set it in your environment:"
    echo "  export OPENAI_API_KEY='your-key'"
    echo "  ./target/release/rzn-browser llm-auto 'your task' --max-steps 20"
    echo ""
    echo "API keys can be obtained from:"
    echo "  - OpenAI: https://platform.openai.com/api-keys"
    echo "  - Google Gemini: https://makersuite.google.com/app/apikey"
    echo ""
    echo "For testing without an API key, you can use workflow mode:"
    echo "  cargo run -p rzn-browser -- run test/manual/workflows/test_google_simple.json"
fi
