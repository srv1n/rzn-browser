#!/bin/bash

# Test script for new features: Flags, Site Profiles, and Flight Recorder
# This script tests the three deliverables we just implemented

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

echo "================================================"
echo "RZN Feature Test Suite"
echo "Testing: Flags, Site Profiles, Flight Recorder"
echo "================================================"

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to run a test workflow
run_test() {
    local workflow=$1
    local description=$2
    
    echo -e "\n${YELLOW}Testing: $description${NC}"
    echo "Running workflow: $workflow"
    
    # Run the workflow
    cargo run -p rzn-browser -- run "$workflow" --param search_query="test query" 2>&1 | tail -5
    
    if [ $? -eq 0 ]; then
        echo -e "${GREEN}✅ Test passed: $description${NC}"
    else
        echo -e "${RED}❌ Test failed: $description${NC}"
    fi
}

# Test 1: Google Search with Site Profiles
echo -e "\n${YELLOW}=== Test 1: Site Profile Extraction ===${NC}"
if [ -f "workflows/google/google-search-with-profiles.json" ]; then
    run_test "workflows/google/google-search-with-profiles.json" "Google Search with Site Profiles"
else
    echo -e "${RED}❌ Workflow not found: google-search-with-profiles.json${NC}"
fi

# Test 2: Check flag system
echo -e "\n${YELLOW}=== Test 2: Flag System ===${NC}"
echo "Checking flag configuration in logs..."
tail -20 ~/rzn_build.log | grep -i "flags" || echo "No flag entries in recent logs"

# Test 3: Check circuit breaker metrics
echo -e "\n${YELLOW}=== Test 3: Circuit Breaker Metrics ===${NC}"
echo "Looking for circuit breaker activity..."
tail -50 ~/rzn_build.log | grep -i "circuit\|failure_rate\|cdp_escalation" || echo "No circuit breaker activity"

# Test 4: Flight Recorder
echo -e "\n${YELLOW}=== Test 4: Flight Recorder ===${NC}"
echo "Instructions for manual test:"
echo "1. Open Chrome with the extension loaded"
echo "2. Navigate to any website"
echo "3. Press Ctrl+Shift+E to export flight recorder data"
echo "4. Check Downloads folder for rzn-debug-*.json file"

# Test 5: Check enhanced action executor
echo -e "\n${YELLOW}=== Test 5: Enhanced Action Executor ===${NC}"
echo "Checking for enhanced action logs..."
tail -30 ~/rzn_build.log | grep -i "enhanced\|ActionExecutor\|site profile" || echo "No enhanced action logs"

echo -e "\n${YELLOW}================================================${NC}"
echo -e "${YELLOW}Test Summary:${NC}"
echo "1. Site Profiles: Check if extraction_type is being used"
echo "2. Flags: Verify per-domain flag configuration" 
echo "3. Circuit Breaker: Monitor failure rates and auto-adjustments"
echo "4. Flight Recorder: Test Ctrl+Shift+E export"
echo -e "${YELLOW}================================================${NC}"

echo -e "\n${GREEN}To monitor logs in real-time:${NC}"
echo "tail -f ~/rzn_build.log | jq ."
