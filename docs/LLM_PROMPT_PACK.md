# RZN FSM-Driven LLM Prompt Pack

This document contains system prompts for the **FSM-driven, tool-only LLM architecture**. The system uses strict state management with policy validation to prevent drift and ensure reliable automation.

## Core System Prompt

```
You are RZN FSM Planner, a tool-using browser automation agent that operates through strict state management.

CRITICAL ARCHITECTURE:
- You MUST use tool calls only (no free-form text responses)
- Temperature is set to 0.0 for deterministic behavior
- You operate within FSM (Finite State Machine) constraints
- Policy layer blocks dangerous patterns automatically

FSM STATES & ALLOWED TOOLS:
- Bootstrap: ["navigate", "wait"] - Initial navigation only
- Search: ["type", "press_key", "wait"] - Type + Enter pattern
- Results: ["extract", "click", "scroll", "wait"] - NO TYPING in results mode
- Form: ["type", "click", "press_key", "wait"] - Form filling
- Browse: ["click", "scroll", "extract", "navigate", "wait"] - General browsing
- Complete: ["complete"] - Task finished

STRICT RULES:
1. NEVER construct Google search URLs (policy blocks this)
2. Use type + press_key pattern instead: type "query" then press_key "Enter" 
3. Only use tools allowed by current FSM state
4. One action per tool call (batching handled at execution layer)
5. CDP press_key provides trusted keyboard events

TARGETING:
- Prefer CSS selectors: "input[name='q']", "button[type='submit']"
- For Google search: "input[name='q']:not([type='hidden'])" (avoids hidden inputs)
- Text-based: find elements by visible text content
- Simple, reliable selectors over complex XPath

COMMON PATTERNS:
✅ Google Search Flow:
1. navigate "https://google.com"
2. type in search box "query text"  
3. press_key "Enter" 
4. extract results

✅ Form Filling:
1. type in field "value"
2. press_key "Tab" (to next field)
3. repeat until done
4. press_key "Enter" or click submit

✅ Data Extraction:
1. wait for content to load
2. extract with item selector and field mappings
3. return structured data

❌ NEVER DO:
- Construct URLs with query parameters
- Type in Results mode (FSM blocks this)
- Multiple actions in single call
- Complex nested tool calls
```

## FSM Tool Schema (IMPLEMENTED)

### Available Tools by FSM State

```json
{
  "Bootstrap": {
    "navigate": {
      "cmd": "navigate",
      "url": "string (required)"
    },
    "wait": {
      "cmd": "wait", 
      "seconds": "number (required)"
    }
  },
  "Search": {
    "type": {
      "cmd": "type",
      "selector": "string (required)",
      "text": "string (required)"
    },
    "press_key": {
      "cmd": "press_key",
      "key": "string (required: Enter, Tab, Escape, etc.)"
    },
    "wait": {
      "cmd": "wait",
      "seconds": "number (required)"
    }
  },
  "Results": {
    "extract": {
      "cmd": "extract",
      "item_selector": "string (required)",
      "fields": [{
        "name": "string",
        "selector": "string", 
        "attribute": "string (optional)"
      }],
      "limit": "number (optional)"
    },
    "click": {
      "cmd": "click",
      "selector": "string (required)"
    },
    "scroll": {
      "cmd": "scroll",
      "direction": "string (up|down|left|right)",
      "pixels": "number (optional)"
    },
    "wait": {
      "cmd": "wait",
      "seconds": "number (required)"
    }
  },
  "Form": {
    "type": "...",
    "click": "...", 
    "press_key": "...",
    "wait": "..."
  },
  "Browse": {
    "click": "...",
    "scroll": "...", 
    "extract": "...",
    "navigate": "...",
    "wait": "..."
  },
  "Complete": {
    "complete": {
      "cmd": "complete",
      "result": "any (final extracted data or success message)"
    }
  }
}
```

## FSM Flow Examples

### Google Search with FSM States:

```json
// State: Bootstrap -> Search
{
  "cmd": "navigate",
  "url": "https://www.google.com"
}

// State: Search (type + press_key pattern)
{
  "cmd": "type",
  "selector": "input[name='q']:not([type='hidden'])",
  "text": "OpenAI GPT-4"
}

{
  "cmd": "press_key", 
  "key": "Enter"
}

// State: Search -> Results (automatic transition)
// Now in Results mode - can extract, click, scroll, wait
{
  "cmd": "extract",
  "item_selector": "div#search div.g",
  "fields": [
    {"name": "title", "selector": "h3"},
    {"name": "url", "selector": "a", "attribute": "href"},
    {"name": "snippet", "selector": ".VwiC3b"}
  ],
  "limit": 10
}

// State: Results -> Complete
{
  "cmd": "complete",
  "result": "Successfully extracted 10 search results"
}
```

### E-commerce Product Search:

```json
// Bootstrap -> Browse (inferred from URL)
{
  "cmd": "navigate",
  "url": "https://www.amazon.com"
}

// Browse -> Search (click on search box transitions to Search mode)
{
  "cmd": "click",
  "selector": "input[placeholder*='Search']"
}

// Now in Search mode
{
  "cmd": "type",
  "selector": "input[placeholder*='Search']",
  "text": "wireless headphones"
}

{
  "cmd": "press_key",
  "key": "Enter"
}

// Search -> Results (automatic transition)
{
  "cmd": "extract",
  "item_selector": "[data-component-type='s-search-result']",
  "fields": [
    {"name": "title", "selector": "h2 a span"},
    {"name": "price", "selector": ".a-price-whole"},
    {"name": "rating", "selector": ".a-icon-alt"},
    {"name": "link", "selector": "h2 a", "attribute": "href"}
  ],
  "limit": 20
}
```

### Form Interaction Flow:

```json
// Navigate to contact form
{
  "cmd": "navigate",
  "url": "https://example.com/contact"
}

// Browse -> Form (click on form field transitions to Form mode) 
{
  "cmd": "click",
  "selector": "input[name='name']"
}

// Now in Form mode - can type, click, press_key
{
  "cmd": "type",
  "selector": "input[name='name']", 
  "text": "John Doe"
}

{
  "cmd": "press_key",
  "key": "Tab"
}

{
  "cmd": "type",
  "selector": "input[name='email']",
  "text": "john@example.com"
}

{
  "cmd": "press_key",
  "key": "Tab"
}

{
  "cmd": "type",
  "selector": "textarea[name='message']",
  "text": "Hello, I would like to learn more about your services."
}

{
  "cmd": "click",
  "selector": "button[type='submit']"
}

// Form -> Complete
{
  "cmd": "complete",
  "result": "Contact form submitted successfully"
}
```

## Quick Reference

### Target Types
```javascript
{kind: "role_name", value: {role: "button", name: "Submit"}}
{kind: "css", value: "#search-box"}
{kind: "encoded_id", value: "7:90123"}
{kind: "deep_xpath", value: "//iframe[2]//button"}
{kind: "text_near", value: {text: "Click here"}}
{kind: "viewport_xy", value: {x: 500, y: 300}}
```

### Common Waits
```javascript
wait_for_element {target: ..., state: "visible"}
wait_for_navigation {timeout_ms: 30000}
wait_for_network_idle {idle_ms: 500}
wait_for_timeout {timeout_ms: 2000}
```

### Extraction Pattern
```javascript
extract_structured {
  item_query: {kind: "css", value: ".item"},
  fields: [
    {name: "title", query: {kind: "css", value: "h3"}},
    {name: "price", query: {kind: "css", value: ".price"}},
    {name: "link", query: {kind: "css", value: "a"}, attribute: "href"}
  ],
  limit: 10,
  across_iframes: true
}
```

## Error Recovery Patterns

### Element Not Found
```
1. wait_for_element with longer timeout
2. Try alternative selector (role_name if using css)
3. Check if in iframe (set frame_policy: "any_frame")
4. Request user intervention
```

### Action Failed
```
1. Automatic escalation through Input Ladder
2. Retry with different targeting
3. Wait and retry
4. Human intervention
```

### Navigation Issues
```
1. wait_for_network_idle after navigation
2. Check current_url
3. Handle redirects
4. Check for auth requirements
```

## Memory Management

Keep a rolling summary (max 2-8KB):
```
S1: Navigated to google.com
S2: Typed "weather" in search box
S3: Pressed Enter; navigated to results
S4: Extracted 10 results with weather data
S5: Clicked first result; opened weather.com
```

## Implementation Architecture

### Tool-Only LLM Configuration
```rust
// CRITICAL: Temperature 0.0 for deterministic output
let request_body = json!({
    "model": "gpt-5-mini-2025-08-07",
    "temperature": 0.0,  
    "tool_choice": "required",  // Force tool use
    "tools": filtered_tools     // Only FSM-allowed tools
});
```

### Correlation ID Tracking
```rust
// Every action tracked with correlation ID
let correlation_id = uuid::Uuid::new_v4().to_string();
log::info!("[{}] Action: {:?}", correlation_id, action);
```

### Consecutive Failure Handling
```rust
// Reset to Browse mode after 3 consecutive failures
if consecutive_failures >= 3 {
    log::warn!("[{}] Resetting to Browse mode", correlation_id);
    fsm.transition(PlannerMode::Browse);
    consecutive_failures = 0;
}
```

## Usage Commands

### Run FSM-Driven Autonomous Mode
```bash
# Primary command for LLM-driven automation
./target/release/rzn-browser llm-auto "Search Google for OpenAI" --max-steps 10

# With debug logging
RUST_LOG=debug ./target/release/rzn-browser llm-auto "Your task here"
```

### Legacy Commands (Still Available)
```bash
# Orchestrator-based planning
./target/release/rzn-browser plan-llm "Task description"
./target/release/rzn-browser plan-auto "Task description"

# Workflow execution
./target/release/rzn-browser run workflows/example.json
```

## Key Benefits

✅ **Deterministic**: Temperature 0.0 + tool-only calls
✅ **Drift-Proof**: FSM prevents invalid state transitions  
✅ **Policy-Enforced**: Blocks dangerous patterns automatically
✅ **Traceable**: Correlation IDs for full execution tracking
✅ **Reliable**: CDP press_key for trusted keyboard events
✅ **Recoverable**: Consecutive failure handling with mode reset

This FSM-driven approach eliminates the common failure modes of free-form LLM automation while maintaining the intelligence and adaptability that makes LLM automation powerful.