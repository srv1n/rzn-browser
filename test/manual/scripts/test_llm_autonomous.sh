#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "🤖 RZN Autonomous LLM Test Suite"
echo "================================="
echo ""

# Check for API key
if [ -z "$OPENAI_API_KEY" ] && [ -z "$GEMINI_API_KEY" ]; then
    echo "❌ No API key found!"
    echo ""
    echo "Set one of these environment variables:"
    echo "  export OPENAI_API_KEY='your-key'"
    echo "  export GEMINI_API_KEY='your-key'"
    exit 1
fi

# Build first
echo "📦 Building latest changes..."
cargo build --release -p rzn-browser -p rzn-browser-worker -p rzn-native-host 2>&1 | grep -E "(error|Finished)"

echo ""
echo "🧪 Test Cases:"
echo ""

# Test 1: Google search
echo "1️⃣ Testing Google search..."
echo "   Task: 'Search Google for Rust programming tutorials'"
./target/release/rzn-browser llm-auto "Search Google for Rust programming tutorials" --max-steps 10

echo ""
echo "2️⃣ Testing Reddit search..."
echo "   Task: 'Search Reddit for best mechanical keyboards'"
./target/release/rzn-browser llm-auto "Search Reddit for best mechanical keyboards" --max-steps 10

echo ""
echo "3️⃣ Testing LinkedIn..."
echo "   Task: 'Search LinkedIn for software engineering jobs in San Francisco'"
./target/release/rzn-browser llm-auto "Search LinkedIn for software engineering jobs in San Francisco" --max-steps 10

echo ""
echo "4️⃣ Testing DuckDuckGo..."
echo "   Task: 'Search DuckDuckGo for privacy-focused browsers'"
./target/release/rzn-browser llm-auto "Search DuckDuckGo for privacy-focused browsers" --max-steps 10

echo ""
echo "✅ Test suite complete!"
