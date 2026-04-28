# RZN Architecture - Consolidated LLM-Driven Browser Automation

## Consolidated Architecture (Post-Cleanup)

**Key Simplifications:**
- **Single LLM Entry Point**: `llm_autonomous.rs` replaces multiple competing implementations
- **Unified DOM Processing**: Consolidated `dom_analyzer.rs`, `dom_processor.rs`, and `dom_context.rs` functionality  
- **Removed Redundancies**: Eliminated duplicate autonomous planners, CLI handlers, and schema files
- **Focused Component Set**: Only essential, actively-used modules remain

## Enhanced Core Flow with FSM

```
┌─────────────┐     Native      ┌──────────────────┐   Chrome   ┌──────────────┐
│   CLI/API   │ ←─────────────→ │ Runtime Bridge   │ ←────────→ │  Extension   │
│             │     TCP/Pipe    │ + Native Host    │  Messaging │  (JS/TS)    │
│ llm-auto    │                 │  +FSM       │              │ +CDP Actions │
│ plan-auto   │                 │  +Policy    │              │ +Static Safe │
└─────────────┘                 └──────────────────┘         └──────────────┘
                                       │                             │
                                ┌──────────────┐                     ↓
                                │ FSM Engine   │              ┌──────────────┐
                                │ Bootstrap    │              │Content Script│
                                │ Search       │              │ (Injected)   │
                                │ Results      │              └──────────────┘
                                │ Form         │                     │
                                │ Browse       │                     ↓
                                │ Complete     │              ┌──────────────┐
                                └──────────────┘              │   Web Page   │
                                       │                      │    (DOM)     │
                                ┌──────────────┐              └──────────────┘
                                │Policy Layer  │
                                │ Block URLs   │
                                │ Tool Restrict│
                                │ Batch Valid  │
                                └──────────────┘
```

## Message Flow Example

Here's what happens when you click a button:

```javascript
// 1. Your command
rzn-browser run workflow.json

// 2. CLI sends to the runtime bridge
{
  "id": "msg-123",
  "action": "click_element",
  "payload": {
    "selector": "#submit-button"
  }
}

// 3. Native host forwards to extension (via Chrome Native Messaging)
// Chrome handles stdin/stdout with length-prefixed JSON

// 4. Extension service worker routes to content script
chrome.tabs.sendMessage(tabId, {
  cmd: "execute_action",
  action: "click_element",
  selector: "#submit-button"
});

// 5. Content script executes
const element = document.querySelector("#submit-button");
element.click();

// 6. Result flows back
{ "success": true, "result": "clicked" }
```

## Three-Tier Action System

### 1. Static Actions (CSP-Safe, Default)
```
Extension → Content Script → Static DOM APIs → Browser Engine
           ↓
    No JavaScript execution
    CSP compliant
    Undetectable
    Limited functionality
```

**When it works:**
- Modern CSP-protected sites
- Simple interactions
- Data extraction
- First-class press_key via CDP

**Examples:**
- `extract_structured_data` 
- `click_element`
- `fill_input_field`
- `press_special_key` (via CDP)

### 2. Enhanced JavaScript Actions (Fallback)
```
Extension → Content Script → Enhanced Events → DOM manipulation
           ↓
    Better event simulation
    Multiple fallback strategies
    Human-like behavior
    Works with most sites
```

**When static fails:**
- Complex interactions
- Dynamic content
- Sites expecting specific event sequences

### 3. CDP Actions (Final Fallback)
```
Extension → chrome.debugger.attach() → CDP Commands → Browser Engine
           ↓
    Uses DevTools Protocol
    Can access any frame
    Events are "trusted"
    Minimal detection risk
```

**When all else fails:**
- Cross-origin iframe interaction
- File upload dialogs
- Sites that check `event.isTrusted`

## The Escalation Ladder

```
Try #1: Pure JS
  ↓ (fails)
Try #2: Enhanced JS (better event simulation)
  ↓ (fails)
Try #3: CDP via chrome.debugger
  ↓ (fails)
Try #4: Native OS events (future)
```

Code that implements this:

```typescript
// inputLadder.ts
async function executeAction(params: ActionParams) {
  // Start with Pure JS
  let result = await executePureJS(params);
  if (result.success) return result;
  
  // Escalate to Enhanced JS
  result = await executeEnhancedJS(params);
  if (result.success) return result;
  
  // Only use CDP if needed
  if (params.crossOrigin || params.requiresTrust) {
    result = await executeCDP(params);
  }
  
  return result;
}
```

## Component Deep Dive

### Browser Extension (TypeScript)

```
extension/
├── src/
│   ├── background.ts         # Service worker (message routing)
│   ├── content/
│   │   ├── actions.ts        # DOM manipulation
│   │   ├── actions-static.ts # CSP-safe actions
│   │   └── dom-analyzer.ts   # Page analysis
│   └── cdp/
│       ├── cdpHelper.ts      # chrome.debugger wrapper
│       └── inputLadder.ts    # Escalation logic
└── manifest.json
```

**Service Worker (background.ts):**
```typescript
// Persistent connection to the native host
let nativePort = chrome.runtime.connectNative(BROKER_HOST);

// Route messages to tabs
nativePort.onMessage.addListener((message) => {
  chrome.tabs.sendMessage(tabId, message);
});
```

**Content Script (actions.ts):**
```typescript
// Executes in page context
function clickElement(selector: string) {
  const element = findElement(selector); // Smart selector with fallbacks
  if (!element) throw new Error("Not found");
  
  // Try multiple strategies
  element.click();                    // Method 1
  element.dispatchEvent(clickEvent);  // Method 2
  simulateMouseSequence(element);     // Method 3
}
```

### Native Host (Rust)

```rust
// crates/rzn_native_host/src/main.rs

fn main() {
    // Read from stdin (Chrome)
    let mut stdin = io::stdin();
    let length = read_u32_native(&mut stdin);
    let message = read_exact(&mut stdin, length);
    
    // Parse and route
    let msg: Message = serde_json::from_slice(&message)?;
    let response = handle_message(msg);
    
    // Write to stdout (back to Chrome)
    write_message(stdout(), &response);
}

struct Message {
    id: String,
    action: Action,
    payload: Value,
}
```

**Message size limits:**
- Host → Extension: 1MB max
- Extension → Host: 64MB max
- Chunking for large payloads

### Orchestrator (rzn_plan)

```
crates/rzn_plan/
├── workflow_manager.rs    # JSON workflow execution
├── llm.rs                # LLM integration
├── dom_analyzer.rs       # Page understanding
└── self_healing.rs       # Error recovery
```

**Workflow execution:**
```rust
pub fn execute_workflow(workflow: Workflow) -> Result<()> {
    for step in workflow.steps {
        match step.action {
            Action::Click(selector) => {
                runtime.send(ClickAction { selector })?;
            }
            Action::Extract(config) => {
                let data = runtime.send(ExtractAction { config })?;
                context.store(data);
            }
        }
    }
}
```

**FSM-Driven LLM Planning:**
```rust
pub async fn execute_autonomous(instruction: &str) -> Result<Response> {
    let mut fsm = PlannerState::new(correlation_id);
    let policy = PolicyValidator::new(correlation_id);
    let tool_llm = ToolOnlyLLMClient::new(api_key, correlation_id);
    
    while !fsm.is_complete() {
        // 1. Get allowed tools for current FSM state
        let allowed_tools = fsm.get_allowed_tools();
        
        // 2. Call tool-only LLM (temperature=0, structured output)
        let tool_calls = tool_llm.call_with_tools(
            &fsm.get_system_prompt(),
            user_prompt,
            all_tools,
            allowed_tools
        ).await?;
        
        // 3. Validate actions against policy
        policy.validate_batch(&tool_calls, &fsm)?;
        
        // 4. Execute actions
        for action in tool_calls {
            let result = runtime.execute_step(&action).await?;
            
            // 5. Update FSM state based on action results
            match action.cmd.as_str() {
                "navigate" => fsm.transition(infer_mode(&url)),
                "press_key" if key == "Enter" => fsm.transition(Results),
                "extract" => fsm.transition(Complete),
                _ => {}
            }
        }
    }
}
```

## DOM Analysis Pipeline

```
Raw HTML (100KB+)
    ↓
DOM Analyzer
    ↓
Semantic Tree (5KB)
    ↓
LLM Context (2KB)
```

**Example transformation:**
```html
<!-- Input: Raw HTML -->
<div class="product-card-wrapper">
  <div class="product-inner">
    <img src="..." alt="iPhone">
    <h3 class="title">iPhone 15 Pro</h3>
    <span class="price">$999</span>
    <button class="add-to-cart">Add to Cart</button>
  </div>
</div>

<!-- Output: Semantic representation -->
{
  "type": "product",
  "name": "iPhone 15 Pro",
  "price": 999,
  "actions": ["add_to_cart"],
  "selector": ".product-card-wrapper button"
}
```

## Security Boundaries

```
┌─────────────────────────────────────────┐
│            Browser Process              │
├─────────────────────────────────────────┤
│  Extension (Has permissions)            │
│  - Can inject into allowed origins      │
│  - Can use chrome.* APIs                │
├─────────────────────────────────────────┤
│  Content Script (Isolated World)        │
│  - DOM access                           │
│  - No page JS access                    │
│  - No cross-origin access               │
├─────────────────────────────────────────┤
│  Web Page (Untrusted)                   │
│  - Can't see extension                  │
│  - Can't access content script          │
└─────────────────────────────────────────┘
```

## Performance Characteristics

```
Action Type         Latency    Success Rate    Detection Risk
─────────────────────────────────────────────────────────────
Pure JS             ~10ms      95%             None
Enhanced JS         ~50ms      98%             Minimal
CDP                 ~100ms     99.9%           Low
Native OS           ~200ms     100%            None
```

## Error Recovery Flow

```
Action Fails
    ↓
Analyze Failure Type
    ├─ Element Not Found → Try alternate selectors
    ├─ Not Visible → Scroll into view
    ├─ Covered → Dismiss overlay
    ├─ Not Trusted → Escalate to CDP
    └─ Network Error → Retry with backoff
```

**Self-healing example:**
```typescript
async function selfHealingClick(selector: string) {
  // Try primary selector
  let element = await findElement(selector);
  
  if (!element) {
    // Try text-based fallback
    element = await findByText(selector);
  }
  
  if (!element) {
    // Ask LLM for help
    const alt = await llm.findAlternative(selector, getPageContext());
    element = await findElement(alt);
  }
  
  if (element) {
    await ensureClickable(element); // Scroll, dismiss popups, etc.
    return click(element);
  }
  
  throw new Error("Could not recover");
}
```

## State Management

```
Global State (Service Worker)
├── Session info
├── Tab mapping
└── Configuration

Tab State (Content Script)
├── Page URL
├── DOM cache
├── Action history
└── Error context

Workflow State (Orchestrator)
├── Variables
├── Loop counters
├── Checkpoints
└── Results
```

## CLI Commands

### Autonomous LLM Mode (Recommended)
```bash
# FSM-driven autonomous execution with policy validation
./target/release/rzn-browser llm-auto "Search Google for OpenAI" --max-steps 10

# Enable debug logging
RUST_LOG=debug ./target/release/rzn-browser llm-auto "Your instruction"

# View FSM state transitions
tail -f ~/rzn_build.log | grep "State transition"

# View policy violations  
tail -f ~/rzn_build.log | grep "POLICY VIOLATION"

# View raw LLM calls (with correlation ID)
cat /tmp/llm_raw_*.jsonl | jq .
```

### Legacy Planning Modes
```bash
# Orchestrator-based planning (older system)
./target/release/rzn-browser plan-llm "Search Google for OpenAI"
./target/release/rzn-browser plan-auto "Search Google for OpenAI"

# Workflow execution
./target/release/rzn-browser run workflows/google-search.json
```

## Debugging Tools

```bash
# Extension logs
chrome://extensions → Service Worker → Inspect

# Runtime logs with structure
tail -f ~/rzn_build.log | jq '.message'

# FSM state debugging  
tail -f ~/rzn_build.log | grep -E "(State transition|Policy|Tool.*allowed)"

# Full trace with correlation IDs
RUST_LOG=debug ./target/release/rzn-browser llm-auto "task" 2>&1 | tee debug.log
```

## Common Patterns

### Wait for dynamic content
```json
{
  "type": "wait_for_element",
  "selector": ".results-loaded",
  "timeout": 5000
}
```

### Extract data from list
```json
{
  "type": "extract_structured_data",
  "item_selector": ".product",
  "fields": [
    {"name": "title", "selector": "h3"},
    {"name": "price", "selector": ".price"}
  ]
}
```

### Handle popups
```json
{
  "type": "conditional",
  "condition": "document.querySelector('.popup')",
  "then": [
    {"type": "click_element", "selector": ".popup .close"}
  ]
}
```

### Cross-origin iframe
```json
{
  "type": "click_element",
  "selector": "iframe#payment >>> button#pay",
  "use_cdp": true
}
```

## Why This Architecture?

**Native Messaging vs WebDriver:**
- ✅ No `navigator.webdriver` flag
- ✅ Uses real browser profile
- ✅ Extensions work normally
- ❌ Limited to Chrome/Firefox

**Extension vs External CDP:**
- ✅ No detectable CDP port
- ✅ User installed = trusted
- ✅ Survives page navigation
- ❌ Requires manual install

**Content Scripts vs Page Scripts:**
- ✅ Isolated from page JS
- ✅ Can't be detected by page
- ✅ Full DOM access
- ❌ Can't access page variables

This architecture makes RZN undetectable while maintaining full automation capabilities.
