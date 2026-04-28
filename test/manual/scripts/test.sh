#!/bin/bash

# RZN Browser Native - Comprehensive Testing Suite
# This script provides all testing capabilities for both workflow and LLM modes

set -e

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
SCRIPT_PATH="./test/manual/scripts/test.sh"
cd "$ROOT_DIR"

# Default settings
MODE="workflow"
WORKFLOW=""
LLM_TASK=""
DEBUG=false
PROVIDER="openai"
BUILD=false

# Function to print colored output
print_color() {
    local color=$1
    shift
    echo -e "${color}$@${NC}"
}

# Function to display usage
usage() {
    cat << EOF
RZN Browser Native Testing Suite

This testing script supports deterministic workflow execution and LLM-driven automation.
The system operates through your actual Chrome browser via the extension + native host bridge.

USAGE:
    ${SCRIPT_PATH} [OPTIONS]

MODES OF OPERATION:

1. WORKFLOW MODE (Default)
   Execute predefined workflow JSON files for deterministic automation:
   
   ${SCRIPT_PATH} -w workflows/google/google-search.json --param query="test automation"
   ${SCRIPT_PATH} -w workflows/bing/bing-news.json --param query="rust"

2. LLM MODE
   Natural language driven automation using AI planning:
   
   ${SCRIPT_PATH} -l "Search Google for rust and summarize top results" -p dummy
   ${SCRIPT_PATH} -l "Search for the latest AI news on Google" -p openai
   ${SCRIPT_PATH} -l "Find iPhone prices on Amazon" -p gemini

3. DOM TESTING MODE
   Test DOM extraction and analysis capabilities:
   
   ${SCRIPT_PATH} --dom-test https://example.com
   ${SCRIPT_PATH} --dom-test https://news.ycombinator.com --debug

OPTIONS:
    -w, --workflow PATH       Path to workflow JSON file
    -l, --llm TASK           Natural language task for LLM mode
    -p, --provider NAME      LLM provider (openai, gemini, claude|anthropic, groq, dummy) [default: openai]
    --param KEY=VALUE        Pass parameters to workflow (repeatable)
    --debug                  Enable debug logging
    --build                  Rebuild before testing
    --dom-test URL           Test DOM extraction on specific URL
    --list-workflows         List available workflows
    --validate WORKFLOW      Validate workflow JSON structure
    -h, --help              Show this help message

ENVIRONMENT VARIABLES:
    LLM_PROVIDER            LLM provider (openai, gemini, claude|anthropic, groq, dummy)
    OPENAI_API_KEY          Required for LLM_PROVIDER=openai
    GEMINI_API_KEY          Required for LLM_PROVIDER=gemini
    ANTHROPIC_API_KEY       Required for LLM_PROVIDER=claude|anthropic
    GROQ_API_KEY            Required for LLM_PROVIDER=groq
    RUST_LOG                Set to 'debug' for verbose output
    RZN_TRANSPORT           Set to 'tcp' or 'pipe' (default: pipe)

EXAMPLES:

    # Basic workflow execution
    ${SCRIPT_PATH} -w workflows/google/google-search.json --param query="test automation"

    # Workflow with parameters
    ${SCRIPT_PATH} -w workflows/pubmed/pubmed-search.json --param query="asthma" --param max_results=5

    # LLM mode with natural language
    ${SCRIPT_PATH} -l "Go to Hacker News and find a post about rust" -p dummy

    # Debug mode with specific provider
    ${SCRIPT_PATH} -l "Search for news" -p gemini --debug

    # Validate workflow before execution
    ${SCRIPT_PATH} --validate workflows/custom/my-workflow.json

    # Test DOM extraction
    ${SCRIPT_PATH} --dom-test https://github.com --debug

WORKFLOW STRUCTURE:
    Workflows are JSON files that define sequential browser actions via the extension.
    Most workflows live under workflows/<category>/*.json.

LLM MODE BEHAVIOR:
    In LLM mode, the system analyzes the page structure, plans actions, and executes
    them with self-healing capabilities. If an action fails, it will attempt alternative
    selectors or strategies automatically.

NOTES:
    - The browser extension must be installed in Chrome
    - The native messaging host will be auto-launched by Chrome
    - Use --debug to see detailed execution logs
    - Check ~/rzn_build.log for complete execution history

EOF
}

# Function to list available workflows
list_workflows() {
    print_color "$BLUE" "\n=== Available Workflows ===\n"
    
    for category in workflows/*/; do
        if [ -d "$category" ]; then
            category_name=$(basename "$category")
            print_color "$GREEN" "$category_name:"
            
            for workflow in "$category"*.json; do
                if [ -f "$workflow" ]; then
                    workflow_name=$(basename "$workflow")
                    name=$(jq -r '.name // .metadata.name // empty' "$workflow" 2>/dev/null || true)
                    description=$(jq -r '.description // .metadata.description // empty' "$workflow" 2>/dev/null || true)
                    if [ -z "$name" ]; then name="$workflow_name"; fi
                    if [ -z "$description" ]; then description="(no description)"; fi
                    echo "  - $workflow_name: $name — $description"
                fi
            done
            echo
        fi
    done
}

# Function to validate workflow
validate_workflow() {
    local workflow_file=$1
    
    if [ ! -f "$workflow_file" ]; then
        print_color "$RED" "Error: Workflow file not found: $workflow_file"
        exit 1
    fi
    
    print_color "$BLUE" "Validating workflow: $workflow_file"
    
    # Check JSON validity
    if ! jq empty "$workflow_file" 2>/dev/null; then
        print_color "$RED" "Error: Invalid JSON in workflow file"
        exit 1
    fi
    
    # Check for supported workflow shapes:
    # - v1: browser_automation.sequences[0].steps
    # - legacy: top-level steps
    local has_steps_v1=$(jq -r '(.browser_automation.sequences[0].steps // empty) | type' "$workflow_file" 2>/dev/null || echo "")
    local has_steps_legacy=$(jq -r '(.steps // empty) | type' "$workflow_file" 2>/dev/null || echo "")

    if [ "$has_steps_v1" != "array" ] && [ "$has_steps_legacy" != "array" ]; then
        print_color "$RED" "Error: Could not find steps array (expected .browser_automation.sequences[0].steps)"
        exit 1
    fi
    
    print_color "$GREEN" "✓ Workflow structure is valid"
    
    # Count steps
    local step_count=$(jq '(.browser_automation.sequences[0].steps // .steps) | length' "$workflow_file")
    print_color "$GREEN" "✓ Found $step_count steps"
    
    # Show step types
    print_color "$BLUE" "\nStep types:"
    jq -r '(.browser_automation.sequences[0].steps // .steps)[] | .type' "$workflow_file" | sort | uniq -c
}

# Function to build the project
build_project() {
    print_color "$BLUE" "\n=== Building RZN Browser Native ===\n"
    
    # Build Rust components
    print_color "$YELLOW" "Building Rust components..."
    cargo build --release -p rzn-browser -p rzn-browser-worker -p rzn-native-host
    
    # Build extension
    print_color "$YELLOW" "Building Chrome extension..."
    cd extension
    bun install
    bun run build
    cd ..
    
    print_color "$GREEN" "✓ Build complete"
}

# Function to test DOM extraction
test_dom_extraction() {
    local url=$1
    
    print_color "$BLUE" "\n=== Testing DOM Extraction ===\n"
    print_color "$YELLOW" "URL: $url"
    
    # Create temporary workflow for DOM testing (navigate + get_page_source)
    local temp_workflow=$(mktemp /tmp/rzn-dom-test-XXXXX.json)
    cat > "$temp_workflow" << EOF
{
  "system_id": "tests",
  "id": "dom_test_v1",
  "name": "DOM Test",
  "description": "Test DOM extraction via get_page_source",
  "domain": "",
  "version": "1.0.0",
  "last_updated": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "browser_automation": {
    "description": "Temporary test workflow",
    "sequences": [
      {
        "name": "dom_test",
        "description": "Navigate then fetch page source",
        "required_variables": [],
        "steps": [
          { "id": "s1", "name": "Navigate", "type": "navigate_to_url", "url": "$url" },
          { "id": "s2", "name": "Wait for settle", "type": "wait_for_timeout", "timeout_ms": 2000 },
          { "id": "s3", "name": "Get page source", "type": "get_page_source" }
        ]
      }
    ]
  }
}
EOF
    
    # Execute with debug output
    if [ "$DEBUG" = true ]; then
        RUST_LOG=debug cargo run -p rzn-browser -- run "$temp_workflow"
    else
        cargo run -p rzn-browser -- run "$temp_workflow"
    fi
    
    rm -f "$temp_workflow"
}

# Function to execute workflow
execute_workflow() {
    local workflow_file=$1
    shift
    local params="$@"
    
    print_color "$BLUE" "\n=== Executing Workflow ===\n"
    print_color "$YELLOW" "Workflow: $workflow_file"
    
    if [ ! -f "$workflow_file" ]; then
        print_color "$RED" "Error: Workflow file not found"
        exit 1
    fi
    
    # Show workflow info (supports both schemas)
    local name=$(jq -r '.name // .metadata.name // "Unnamed"' "$workflow_file")
    local description=$(jq -r '.description // .metadata.description // "No description"' "$workflow_file")
    print_color "$GREEN" "Name: $name"
    print_color "$GREEN" "Description: $description"
    
    # Execute workflow
    if [ "$DEBUG" = true ]; then
        RUST_LOG=debug cargo run -p rzn-browser -- run "$workflow_file" $params
    else
        cargo run -p rzn-browser -- run "$workflow_file" $params
    fi
}

# Function to execute LLM task
execute_llm_task() {
    local task=$1
    
    print_color "$BLUE" "\n=== Executing LLM Task ===\n"
    print_color "$YELLOW" "Task: $task"
    print_color "$YELLOW" "Provider: $PROVIDER"
    
    # Check API key (skip for dummy)
    case $PROVIDER in
        dummy)
            ;;
        openai)
            if [ -z "$OPENAI_API_KEY" ]; then
                print_color "$RED" "Error: OPENAI_API_KEY not set"
                exit 1
            fi
            ;;
        gemini)
            if [ -z "$GEMINI_API_KEY" ]; then
                print_color "$RED" "Error: GEMINI_API_KEY not set"
                exit 1
            fi
            ;;
        claude|anthropic)
            if [ -z "$ANTHROPIC_API_KEY" ]; then
                print_color "$RED" "Error: ANTHROPIC_API_KEY not set"
                exit 1
            fi
            ;;
        groq)
            if [ -z "$GROQ_API_KEY" ]; then
                print_color "$RED" "Error: GROQ_API_KEY not set"
                exit 1
            fi
            ;;
    esac

    # Build CLI if missing
    if [ ! -x "./target/release/rzn-browser" ] || find crates/rzn_browser crates/rzn_plan crates/rzn_core -type f -newer "./target/release/rzn-browser" | head -n 1 | grep -q .; then
        print_color "$YELLOW" "[WARN] rzn-browser is missing or stale. Building rzn-browser (release)..."
        cargo build --release -p rzn-browser
    fi

    # Execute LLM task (autonomous loop)
    if [ "$DEBUG" = true ]; then
        RUST_LOG=debug LLM_PROVIDER="$PROVIDER" ./target/release/rzn-browser llm-auto "$task" --max-steps 20
    else
        LLM_PROVIDER="$PROVIDER" ./target/release/rzn-browser llm-auto "$task" --max-steps 20
    fi
}

# Parse command line arguments
PARAMS=()
while [[ $# -gt 0 ]]; do
    case $1 in
        -w|--workflow)
            MODE="workflow"
            WORKFLOW="$2"
            shift 2
            ;;
        -l|--llm)
            MODE="llm"
            LLM_TASK="$2"
            shift 2
            ;;
        -p|--provider)
            PROVIDER="$2"
            shift 2
            ;;
        --param)
            PARAMS+=("--param" "$2")
            shift 2
            ;;
        --debug)
            DEBUG=true
            shift
            ;;
        --build)
            BUILD=true
            shift
            ;;
        --dom-test)
            MODE="dom"
            DOM_URL="$2"
            shift 2
            ;;
        --list-workflows)
            list_workflows
            exit 0
            ;;
        --validate)
            validate_workflow "$2"
            exit 0
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            print_color "$RED" "Unknown option: $1"
            usage
            exit 1
            ;;
    esac
done

# Main execution
print_color "$GREEN" "\n╔══════════════════════════════════════╗"
print_color "$GREEN" "║     RZN Browser Native Testing      ║"
print_color "$GREEN" "╚══════════════════════════════════════╝"

# Build if requested
if [ "$BUILD" = true ]; then
    build_project
fi

# Execute based on mode
case $MODE in
    workflow)
        if [ -z "$WORKFLOW" ]; then
            print_color "$RED" "Error: No workflow specified"
            echo "Use -w <workflow-file> or --list-workflows to see available workflows"
            exit 1
        fi
        execute_workflow "$WORKFLOW" "${PARAMS[@]}"
        ;;
    llm)
        if [ -z "$LLM_TASK" ]; then
            print_color "$RED" "Error: No LLM task specified"
            echo "Use -l <task-description>"
            exit 1
        fi
        execute_llm_task "$LLM_TASK"
        ;;
    dom)
        test_dom_extraction "$DOM_URL"
        ;;
    *)
        print_color "$RED" "Error: Invalid mode"
        usage
        exit 1
        ;;
esac

print_color "$GREEN" "\n✓ Test execution complete"

# Show log location
print_color "$BLUE" "\nLogs available at:"
echo "  - Execution log: ~/rzn_build.log"
echo "  - Use 'tail -f ~/rzn_build.log | jq .' for formatted output"
