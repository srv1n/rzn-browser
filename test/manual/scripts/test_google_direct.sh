#!/bin/bash

# Direct test of Google search with site profiles
# This bypasses workflow issues and tests our actual features

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "================================================"
echo "DIRECT GOOGLE SEARCH TEST"
echo "Testing: Site Profiles & Feature Flags"
echo "================================================"

echo ""
echo "INSTRUCTIONS:"
echo "1. Open Chrome"
echo "2. Go to chrome://extensions/"
echo "3. Click reload on RZN extension"
echo "4. Open a NEW tab to https://www.google.com"
echo "5. Open DevTools Console (F12)"
echo "6. Keep Google tab active and run this script"
echo ""
echo "Press Enter when ready..."
read

# Create a minimal test workflow that just extracts from Google
cat > /tmp/test_google_now.json << 'EOF'
{
  "system_id": "test",
  "id": "test_google_direct",
  "name": "Direct Google Test",
  "description": "Test site profiles on Google",
  "domain": "google.com",
  "version": "1.0.0",
  "browser_automation": {
    "description": "Direct test",
    "sequences": [
      {
        "name": "test_google",
        "description": "Test Google extraction",
        "required_variables": [],
        "steps": [
          {
            "id": "s1",
            "name": "Fill search box",
            "type": "fill_input_field",
            "selector": "textarea[name='q'], input[name='q']",
            "value": "test query"
          },
          {
            "id": "s2",
            "name": "Submit search",
            "type": "press_special_key",
            "key": "Enter"
          },
          {
            "id": "s3",
            "name": "Wait for results",
            "type": "wait_for_timeout",
            "timeout_ms": 3000
          },
          {
            "id": "s4",
            "name": "Extract with site profile",
            "type": "extract_structured_data",
            "extraction_type": "search_results",
            "item_selector": "#search",
            "fields": [
              {
                "name": "results",
                "selector": ".g"
              }
            ],
            "output_variable": "search_results"
          }
        ]
      }
    ]
  }
}
EOF

echo "Running Google test workflow..."
cargo run -p rzn-browser -- run /tmp/test_google_now.json

echo ""
echo "================================================"
echo "CHECK THE CONSOLE:"
echo "1. Look for: [RZN] Using site profile extraction"
echo "2. Look for: [Flags] Resolved for google.com"
echo "3. Press Ctrl+Shift+E to test flight recorder"
echo "================================================"
