# Issue #103: Implement Core Interact Family Actions

## Priority: 🔴 Critical (Week 1)

## Description
Implement the 4 most important actions in the Interact family. These handle clicking and typing - the core of browser automation.

## Actions to Implement

### 1. fill_input_field
The workhorse for typing text. Must handle YouTube/Google special cases.

```rust
ActionDetails {
    name: "fill_input_field",
    description: "Type text into an input field",
    parameters: vec![
        ActionParameter {
            name: "selector",
            param_type: "string",
            required: true,
            description: "CSS selector for input element",
        },
        ActionParameter {
            name: "value",
            param_type: "string",
            required: true,
            description: "Text to type",
        },
        ActionParameter {
            name: "clear_first",
            param_type: "boolean",
            required: false,
            description: "Clear field before typing (default: true)",
        },
    ],
}
```

### 2. click_element  
Standard click action with shadow DOM support.

```rust
ActionDetails {
    name: "click_element",
    description: "Click on an element",
    parameters: vec![
        ActionParameter {
            name: "selector",
            param_type: "string",
            required: true,
            description: "CSS selector of element to click",
        },
        ActionParameter {
            name: "wait_for_navigation",
            param_type: "boolean",
            required: false,
            description: "Wait for page navigation after click",
        },
    ],
}
```

### 3. submit_input
**NEW** - Combined type + submit action for search boxes.

```rust
ActionDetails {
    name: "submit_input",
    description: "Type text and submit (search boxes)",
    parameters: vec![
        ActionParameter {
            name: "selector",
            param_type: "string",
            required: true,
            description: "CSS selector for input element",
        },
        ActionParameter {
            name: "text",
            param_type: "string",
            required: true,
            description: "Text to type and submit",
        },
    ],
}
```

### 4. press_special_key
For Enter, Tab, Escape keys.

```rust
ActionDetails {
    name: "press_special_key",
    description: "Press a special keyboard key",
    parameters: vec![
        ActionParameter {
            name: "key",
            param_type: "string",
            required: true,
            description: "Key name: Enter, Tab, Escape",
        },
        ActionParameter {
            name: "selector",
            param_type: "string",
            required: false,
            description: "Optional element to focus first",
        },
    ],
}
```

## Critical Implementation Details

### Shadow DOM Support
```typescript
// In extension - must traverse shadow roots
function findElement(selector: string): Element | null {
    // Try regular DOM first
    let element = document.querySelector(selector);
    if (element) return element;
    
    // Traverse shadow roots
    const allElements = document.querySelectorAll('*');
    for (const el of allElements) {
        if (el.shadowRoot) {
            element = el.shadowRoot.querySelector(selector);
            if (element) return element;
        }
    }
    return null;
}
```

### YouTube/Google Fix for submit_input
```typescript
async function submitInput(selector: string, text: string) {
    const element = findElement(selector);
    
    // Use native setter (critical for React/Polymer)
    const nativeSetter = Object.getOwnPropertyDescriptor(
        window.HTMLInputElement.prototype, 'value'
    )?.set;
    
    nativeSetter?.call(element, text);
    
    // Trigger events
    element.dispatchEvent(new Event('input', { bubbles: true }));
    element.dispatchEvent(new Event('change', { bubbles: true }));
    
    // Send Enter key sequence
    const enterEvents = ['keydown', 'keypress', 'keyup'];
    for (const eventType of enterEvents) {
        element.dispatchEvent(new KeyboardEvent(eventType, {
            key: 'Enter',
            code: 'Enter',
            keyCode: 13,
            bubbles: true
        }));
    }
}
```

### DOM-First, Native-Fallback Pattern
```rust
pub async fn execute_interact_action(
    action: &str,
    params: &Value,
    memory: &mut PlanningMemory
) -> Result<ActionResult> {
    // Try DOM layer first
    match dom_executor.execute(action, params).await {
        Ok(result) => Ok(result.with_layer(ExecutionLayer::Dom)),
        Err(e) if e.needs_native_fallback() => {
            // Fall back to native input
            native_executor.execute(action, params).await
                .map(|r| r.with_layer(ExecutionLayer::Native))
        }
        Err(e) => Err(e),
    }
}
```

## Test Cases

### YouTube Search Test (Most Important!)
```rust
#[tokio::test]
async fn test_youtube_search() {
    // Navigate to YouTube
    execute_action("navigate_to_url", &json!({
        "url": "https://youtube.com"
    })).await?;
    
    // Use submit_input for search
    let result = execute_action("submit_input", &json!({
        "selector": "input#search",
        "text": "OpenAI GPT-4"
    })).await?;
    
    assert!(result.is_ok());
    // Should navigate to search results
    assert!(world_state.url.contains("results"));
}
```

### Google Search Test
```rust
#[tokio::test]
async fn test_google_search() {
    execute_action("navigate_to_url", &json!({
        "url": "https://google.com"
    })).await?;
    
    let result = execute_action("submit_input", &json!({
        "selector": "input[name='q']",
        "text": "rust programming"
    })).await?;
    
    assert!(result.is_ok());
}
```

## Acceptance Criteria
- [ ] All 4 actions work on regular sites
- [ ] YouTube search works with submit_input
- [ ] Google search works with submit_input
- [ ] Shadow DOM traversal works
- [ ] Native fallback triggers when needed
- [ ] Telemetry tracks which layer was used

## Common Issues to Avoid
1. Don't use `element.value = text` on YouTube/Google
2. Don't forget to traverse shadow DOM
3. Don't skip the keyboard event sequence for Enter
4. Don't trust focus events on modern frameworks

## Resources
- YouTube fix details: [Original Design - submitInput](../docs/archive/design/ORIGINAL_DESIGN.md#the-youtubegoogle-fix-submitinput)
- Current implementation: `extension/src/content/actions.ts`

## Time Estimate: 8 hours (this is the most complex family)