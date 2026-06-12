# RZN Implementation Guide - FSM-Driven Browser Automation

This guide provides step-by-step instructions for implementing RZN's revolutionary FSM-driven browser automation architecture based on Sam's architectural insights.

## Architecture Overview

RZN uses a Finite State Machine (FSM) with strict policy validation and tool-only LLM calls to achieve deterministic, reliable browser automation. The key components are:

- **FSM (planner_fsm.rs)**: Strict state management preventing drift
- **Policy Layer (policy.rs)**: Blocks bad patterns like Google search URL construction  
- **Tool-only LLM (llm_client.rs)**: Forces structured output with temperature=0
- **CDP Integration**: First-class keyboard/mouse input for reliability

## Core Components Implementation

### 1. FSM State Machine (IMPLEMENTED)

```rust
// crates/rzn_plan/src/planner/planner_fsm.rs

#[derive(Debug, Clone, PartialEq)]
pub enum PlannerMode {
    Bootstrap,  // Initial navigation only
    Search,     // Type in search box + Enter  
    Results,    // Extract data or navigate (NO TYPING)
    Form,       // Fill forms
    Browse,     // General browsing
    Complete,   // Task finished
}

impl PlannerState {
    pub fn get_allowed_tools(&self) -> Vec<&'static str> {
        match self.mode {
            PlannerMode::Bootstrap => vec!["navigate", "wait"],
            PlannerMode::Search => vec!["type", "press_key", "wait"], 
            PlannerMode::Results => vec!["extract", "click", "scroll", "wait"], // STRICT
            PlannerMode::Form => vec!["type", "click", "press_key", "wait"],
            PlannerMode::Browse => vec!["click", "scroll", "extract", "navigate", "wait"],
            PlannerMode::Complete => vec!["complete"],
        }
    }
    
    pub fn transition(&mut self, new_mode: PlannerMode) {
        log::info!("[{}] State transition: {:?} -> {:?}", 
            self.correlation_id, self.mode, new_mode);
        self.mode = new_mode;
    }
    
    pub fn infer_next_mode(&self, url: &str, _dom_summary: &str) -> PlannerMode {
        if url.contains("google.com") && !url.contains("/search?") {
            PlannerMode::Search
        } else if url.contains("/search?") || url.contains("results") {
            PlannerMode::Results  
        } else {
            PlannerMode::Browse
        }
    }
}
```

### 2. Policy Validation Layer (IMPLEMENTED)

```rust
// crates/rzn_plan/src/planner/policy.rs

pub struct PolicyValidator {
    correlation_id: String,
}

impl PolicyValidator {
    pub fn validate_batch(&self, actions: &[Value], state: &PlannerState) -> Result<(), String> {
        // Validate each action individually first
        for (i, action) in actions.iter().enumerate() {
            self.validate_action(action, state)
                .map_err(|e| format!("Action {}: {}", i, e))?;
        }
        
        // Check for dangerous patterns
        if actions.len() >= 2 {
            self.check_batch_patterns(actions, state)?;
        }
        
        Ok(())
    }
    
    fn validate_action(&self, action: &Value, state: &PlannerState) -> Result<(), String> {
        let cmd = action.get("cmd").and_then(|v| v.as_str())
            .ok_or("Missing cmd field")?;
            
        // CRITICAL: Block Google search URL construction
        if cmd == "navigate" {
            if let Some(url) = action.get("url").and_then(|v| v.as_str()) {
                if url.contains("google.com/search?") {
                    return Err("POLICY VIOLATION: Never construct Google search URLs. Use type + press_key instead.".to_string());
                }
            }
        }
        
        // Validate tool is allowed in current FSM state
        let allowed_tools = state.get_allowed_tools();
        if !allowed_tools.contains(&cmd) {
            return Err(format!("Tool '{}' not allowed in mode {:?}", cmd, state.mode));
        }
        
        Ok(())
    }
}
```

### 3. Tool-Only LLM Client (IMPLEMENTED) 

```rust
// crates/rzn_plan/src/tool_llm/llm_client.rs

pub struct ToolOnlyLLMClient {
    client: Client,
    api_key: String,
    correlation_id: String,
}

impl ToolOnlyLLMClient {
    pub async fn call_with_tools(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        all_tools: Vec<ChatCompletionTool>,
        allowed_tools: Vec<&str>,
    ) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        
        // Filter tools by FSM allowlist
        let filtered_tools: Vec<_> = all_tools.into_iter()
            .filter(|tool| {
                if let Some(name) = &tool.function.name {
                    allowed_tools.contains(&name.as_str())
                } else {
                    false
                }
            })
            .collect();
            
        let request_body = json!({
            "model": "gpt-5-mini-2025-08-07",
            "temperature": 0.0,  // CRITICAL: Deterministic output
            "max_tokens": 1000,
            "tool_choice": "required",  // CRITICAL: Force tool use
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt}
            ],
            "tools": filtered_tools
        });
        
        // Execute request and return parsed tool calls
        let response = self.execute_request(request_body).await?;
        self.parse_tool_calls(&response)
    }
}
```
```

### 4. Main LLM Autonomous Flow (IMPLEMENTED)

```rust
// crates/rzn_plan/src/llm_autonomous.rs

pub async fn execute_autonomous(instruction: &str) -> Result<Response> {
    let correlation_id = uuid::Uuid::new_v4().to_string();
    let mut planner_state = PlannerState::new(correlation_id.clone());
    let policy = PolicyValidator::new(correlation_id.clone());
    let tool_llm = ToolOnlyLLMClient::new(api_key, correlation_id.clone());
    let mut broker = BrokerClient::new().await?;
    let mut step_count = 0;
    const MAX_STEPS: u32 = 50;
    let mut consecutive_failures = 0;
    const MAX_CONSECUTIVE_FAILURES: u32 = 3;
    
    log::info!("[{}] Starting autonomous execution: {}", correlation_id, instruction);
    
    while !planner_state.is_complete() && step_count < MAX_STEPS {
        step_count += 1;
        
        // 1. Get allowed tools for current FSM state
        let allowed_tools = planner_state.get_allowed_tools();
        log::debug!("[{}] Step {}: Mode {:?}, Allowed tools: {:?}", 
            correlation_id, step_count, planner_state.mode, allowed_tools);
        
        // 2. Get page context
        let page_context = get_page_context(&mut broker).await
            .unwrap_or_else(|_| "Failed to get page context".to_string());
        
        // 3. Build user prompt
        let user_prompt = format!(
            "Task: {}\n\nCurrent page: {}\n\nWhat should I do next?",
            instruction, page_context
        );
        
        // 4. Call tool-only LLM (temperature=0, structured output)
        let tool_calls = tool_llm.call_with_tools(
            &planner_state.get_system_prompt(),
            &user_prompt,
            get_all_tools(),
            allowed_tools
        ).await?;
        
        // 5. Validate actions against policy
        policy.validate_batch(&tool_calls, &planner_state)?;
        
        // 6. Execute actions
        let mut action_success = true;
        for action in &tool_calls {
            let result = broker.execute_step(action).await?;
            
            if !result.success {
                log::error!("[{}] Action failed: {:?}", correlation_id, result.error);
                action_success = false;
                break;
            }
            
            // 7. Update FSM state based on action results
            if let Some(url) = &result.current_url {
                match action.get("cmd").and_then(|v| v.as_str()) {
                    Some("navigate") => {
                        let next_mode = planner_state.infer_next_mode(url, &page_context);
                        planner_state.transition(next_mode);
                    }
                    Some("press_key") if planner_state.mode == PlannerMode::Search => {
                        planner_state.transition(PlannerMode::Results);
                    }
                    Some("extract") => {
                        planner_state.transition(PlannerMode::Complete);
                    }
                    _ => {}
                }
            }
        }
        
        // 8. Handle consecutive failures
        if action_success {
            consecutive_failures = 0;
        } else {
            consecutive_failures += 1;
            if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                log::warn!("[{}] Too many consecutive failures, resetting to Browse mode", correlation_id);
                planner_state.transition(PlannerMode::Browse);
                consecutive_failures = 0;
            }
        }
    }
    
    Ok(Response::success(format!("Completed in {} steps", step_count)))
}
```

### 5. CDP-Based Key Input (IMPLEMENTED)

```typescript
// extension/src/actions/press_key.ts

export async function press_key_cdp(key: string): Promise<ActionResult> {
    try {
        const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
        if (tabs.length === 0) {
            throw new Error('No active tab found');
        }
        
        const tabId = tabs[0].id!;
        const debuggee = { tabId };
        
        // Attach debugger
        await chrome.debugger.attach(debuggee, '1.3');
        
        try {
            // Get key mapping
            const mapping = getKeyMapping(key);
            
            // Dispatch keyDown event
            await chrome.debugger.sendCommand(debuggee, 'Input.dispatchKeyEvent', {
                type: 'keyDown',
                key: mapping.key,
                code: mapping.code,
                windowsVirtualKeyCode: mapping.keyCode,
                nativeVirtualKeyCode: mapping.keyCode,
                modifiers: 0
            });
            
            // Small delay for realistic timing
            await new Promise(resolve => setTimeout(resolve, 50));
            
            // Dispatch keyUp event  
            await chrome.debugger.sendCommand(debuggee, 'Input.dispatchKeyEvent', {
                type: 'keyUp',
                key: mapping.key,
                code: mapping.code,
                windowsVirtualKeyCode: mapping.keyCode,
                nativeVirtualKeyCode: mapping.keyCode,
                modifiers: 0
            });
            
            return { success: true, message: `Pressed key: ${key}` };
            
        } finally {
            // Always detach debugger
            await chrome.debugger.detach(debuggee);
        }
        
    } catch (error) {
        console.error('CDP key press failed:', error);
        return { success: false, error: error.message };
    }
}

function getKeyMapping(key: string) {
    const mappings = {
        'Enter': { key: 'Enter', code: 'Enter', keyCode: 13 },
        'Escape': { key: 'Escape', code: 'Escape', keyCode: 27 },
        'Tab': { key: 'Tab', code: 'Tab', keyCode: 9 },
        'Space': { key: ' ', code: 'Space', keyCode: 32 },
        'ArrowUp': { key: 'ArrowUp', code: 'ArrowUp', keyCode: 38 },
        'ArrowDown': { key: 'ArrowDown', code: 'ArrowDown', keyCode: 40 }
    };
    
    return mappings[key] || { key, code: key, keyCode: key.charCodeAt(0) };
}
```

## Usage and Testing

### CLI Commands

```bash
# FSM-driven autonomous execution (RECOMMENDED)
./target/release/rzn-browser llm-auto "Search Google for OpenAI" --max-steps 10

# Enable debug logging to see FSM transitions
RUST_LOG=debug ./target/release/rzn-browser llm-auto "Your instruction"

# View FSM state transitions
tail -f ~/rzn_build.log | grep "State transition"

# View policy violations  
tail -f ~/rzn_build.log | grep "POLICY VIOLATION"

# View raw LLM calls (with correlation ID)
cat /tmp/llm_raw_*.jsonl | jq .
```

### Legacy Planning Modes (Still Available)

```bash
# Orchestrator-based planning (older system)
./target/release/rzn-browser plan-llm "Search Google for OpenAI"
./target/release/rzn-browser plan-auto "Search Google for OpenAI"

# Workflow execution
./target/release/rzn-browser run workflows/google-search.json
```

## Common Implementation Patterns

### 1. FSM State Transitions

The FSM prevents problematic state transitions:

```rust
// Only transition to Search if we're in Bootstrap or Browse mode
match self.planner_state.mode {
    PlannerMode::Bootstrap | PlannerMode::Browse => {
        self.planner_state.transition(PlannerMode::Search);
    }
    _ => {} // Stay in current mode (Form, Search, etc.)
}

// Automatic transitions based on action success
match action.cmd.as_str() {
    "navigate" => {
        let next_mode = fsm.infer_next_mode(&url, &dom_summary);
        fsm.transition(next_mode); // Bootstrap → Search (if google.com)
    }
    "press_key" if key == "Enter" && fsm.mode == Search => {
        fsm.transition(Results); // Search → Results (after pressing Enter)
    }
    "extract" => {
        fsm.transition(Complete); // Results → Complete (data extracted)
    }
    _ => {} // Stay in current state
}
```

### 2. Policy Enforcement Patterns

```rust
// Block dangerous URL construction patterns
if cmd == "navigate" {
    if let Some(url) = action.get("url").and_then(|v| v.as_str()) {
        if url.contains("google.com/search?") {
            return Err("POLICY VIOLATION: Never construct Google search URLs. Use type + press_key instead.".to_string());
        }
    }
}

// Enforce FSM tool restrictions
let allowed_tools = state.get_allowed_tools();
if !allowed_tools.contains(&cmd) {
    return Err(format!("Tool '{}' not allowed in mode {:?}", cmd, state.mode));
}
```

### 3. Tool-Only LLM Integration

```rust
// Force structured output with temperature=0
let request_body = json!({
    "model": "gpt-5-mini-2025-08-07",
    "temperature": 0.0,  // CRITICAL: Deterministic output
    "tool_choice": "required",  // CRITICAL: Force tool use
    "tools": filtered_tools  // Only tools allowed by FSM
});
```

## Debugging and Monitoring

### Debug Logging

```bash
# View FSM state transitions
tail -f ~/rzn_build.log | grep "State transition"
# [87a21fb2-bd8b-4a00-8abb-17d90a635a5e] State transition: Bootstrap -> Search

# View policy violations  
tail -f ~/rzn_build.log | grep "POLICY VIOLATION"

# View raw LLM calls with correlation IDs
cat /tmp/llm_raw_87a21fb2-bd8b-4a00-8abb-17d90a635a5e.jsonl | jq .

# Full trace with correlation IDs
RUST_LOG=debug ./target/release/rzn-browser llm-auto "task" 2>&1 | tee debug.log
```

### Extension Debugging

```bash
# Extension logs
chrome://extensions → Service Worker → Inspect

# Runtime logs with structure
tail -f ~/rzn_build.log | jq '.message'

# FSM state debugging  
tail -f ~/rzn_build.log | grep -E "(State transition|Policy|Tool.*allowed)"
```

## Error Handling and Recovery

### Consecutive Failure Handling

```rust
if action_success {
    consecutive_failures = 0;
} else {
    consecutive_failures += 1;
    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
        log::warn!("[{}] Too many consecutive failures, resetting to Browse mode", correlation_id);
        planner_state.transition(PlannerMode::Browse);
        consecutive_failures = 0;
    }
}
```

### Policy Validation Errors

```rust
// Policy violations are logged and prevent action execution
if url.contains("google.com/search?") {
    return Err("POLICY VIOLATION: Never construct Google search URLs. Use type + press_key instead.".to_string());
}

// Tool restrictions enforce FSM discipline
let allowed_tools = state.get_allowed_tools();
if !allowed_tools.contains(&cmd) {
    return Err(format!("Tool '{}' not allowed in mode {:?}", cmd, state.mode));
}
```

## Testing and Validation

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_fsm_tool_restrictions() {
        let mut fsm = PlannerState::new("test".to_string());
        
        // Bootstrap mode should only allow navigate and wait
        fsm.mode = PlannerMode::Bootstrap;
        let allowed = fsm.get_allowed_tools();
        assert_eq!(allowed, vec!["navigate", "wait"]);
        
        // Results mode should NOT allow typing
        fsm.mode = PlannerMode::Results;
        let allowed = fsm.get_allowed_tools();
        assert!(!allowed.contains(&"type"));
        assert!(allowed.contains(&"extract"));
    }
    
    #[test]
    fn test_policy_blocks_google_urls() {
        let policy = PolicyValidator::new("test".to_string());
        let state = PlannerState::new("test".to_string());
        
        let bad_action = json!({
            "cmd": "navigate",
            "url": "https://www.google.com/search?q=test"
        });
        
        let result = policy.validate_action(&bad_action, &state);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("POLICY VIOLATION"));
    }
}
```

### Integration Tests

```bash
# Test FSM flow end-to-end
RUST_LOG=debug ./target/release/rzn-browser llm-auto "Search Google for OpenAI" --max-steps 5

# Test policy enforcement
echo '{"cmd":"navigate","url":"https://google.com/search?q=test"}' | \
  ./target/release/rzn-browser validate-action

# Test CDP key input
./target/release/rzn-browser test-cdp-keys
```

## Implementation Checklist

- [x] FSM State Machine implemented (`planner_fsm.rs`)
- [x] Policy Validation Layer implemented (`policy.rs`) 
- [x] Tool-only LLM Client implemented (`llm_client.rs`)
- [x] Main Autonomous Flow implemented (`llm_autonomous.rs`)
- [x] CDP-based key input implemented (`press_key.ts`)
- [x] Content script CDP integration (`contentScript.ts`)
- [x] Correlation ID tracking throughout system
- [x] FSM state transition logging
- [x] Policy violation logging and blocking
- [x] Consecutive failure handling and recovery
- [x] Debug logging with structured output
- [x] Unit tests for FSM and Policy components
- [x] Integration tests for full flow

## Common Issues and Solutions

### Issue: LLM constructs Google search URLs
**Solution**: Policy layer blocks this pattern and forces type+press_key instead.

### Issue: FSM gets stuck in wrong mode
**Solution**: Consecutive failure handling resets to Browse mode after 3 failures.

### Issue: Actions fail with "not allowed in mode" errors
**Solution**: Check FSM state and allowed tools; ensure state transitions are correct.

### Issue: CDP key events not working
**Solution**: Ensure extension has debugger permission and tab is active.

## Performance Considerations

- Tool-only LLM calls reduce token usage vs free-form responses
- Temperature=0 provides deterministic, cacheable responses
- Accessibility tree outline (2-8KB) vs full HTML (1.6MB)
- Correlation IDs enable efficient log filtering
- FSM prevents unnecessary state exploration

This implementation provides the foundation for reliable, FSM-driven browser automation as designed by Sam's architectural insights.
