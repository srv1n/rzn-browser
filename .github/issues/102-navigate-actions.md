# Issue #102: Implement Navigate Family Actions

## Priority: 🔴 Critical (Week 1)

## Description
Implement the 4 core actions in the Navigate family. This is the simplest family and will be used to test the handshake flow.

## Actions to Implement

### 1. navigate_to_url
```rust
ActionDetails {
    name: "navigate_to_url",
    description: "Navigate browser to specified URL",
    parameters: vec![
        ActionParameter {
            name: "url",
            param_type: "string",
            required: true,
            description: "Complete URL including https://",
        }
    ],
}
```

### 2. open_new_tab
```rust
ActionDetails {
    name: "open_new_tab",
    description: "Open URL in new browser tab",
    parameters: vec![
        ActionParameter {
            name: "url",
            param_type: "string", 
            required: false,
            description: "URL to open (optional, defaults to blank)",
        }
    ],
}
```

### 3. switch_to_tab
```rust
ActionDetails {
    name: "switch_to_tab",
    description: "Switch to existing tab by ID or index",
    parameters: vec![
        ActionParameter {
            name: "tab_identifier",
            param_type: "string",
            required: true,
            description: "Tab ID or index (0-based)",
        }
    ],
}
```

### 4. close_current_tab
```rust
ActionDetails {
    name: "close_current_tab",
    description: "Close the active tab",
    parameters: vec![],
}
```

## Implementation Steps

1. **Update Action Catalog**
```rust
// In crates/rzn_plan/src/action_taxonomy.rs
lazy_static! {
    pub static ref ACTION_CATALOG: HashMap<ActionFamily, Vec<ActionDetails>> = {
        let mut catalog = HashMap::new();
        catalog.insert(ActionFamily::Navigate, get_navigate_actions());
        // ... other families
        catalog
    };
}

fn get_navigate_actions() -> Vec<ActionDetails> {
    vec![
        // Implement all 4 actions here
    ]
}
```

2. **Add Execution Logic**
```rust
// In crates/rzn_plan/src/action_executor.rs
pub async fn execute_navigate_action(
    action: &str,
    params: &Value
) -> Result<ActionResult> {
    match action {
        "navigate_to_url" => {
            let url = params["url"].as_str()
                .ok_or("Missing url parameter")?;
            // Send to browser extension
            broker_client.send_action("navigate_to_url", json!({
                "url": url
            })).await
        }
        // ... other actions
    }
}
```

3. **Update WorldState After Navigation**
```rust
// Track active tab
world_state.active_tab_id = response.tab_id;
world_state.url = response.current_url;
world_state.page_title = response.page_title;
```

## Test Cases

### Unit Test
```rust
#[test]
fn test_navigate_action_formatting() {
    let actions = get_navigate_actions();
    assert_eq!(actions.len(), 4);
    assert_eq!(actions[0].name, "navigate_to_url");
}
```

### Integration Test
```rust
#[tokio::test]
async fn test_navigate_to_youtube() {
    let result = execute_navigate_action(
        "navigate_to_url",
        &json!({"url": "https://youtube.com"})
    ).await;
    
    assert!(result.is_ok());
    assert_eq!(result.unwrap().url, "https://youtube.com");
}
```

## Acceptance Criteria
- [ ] All 4 Navigate actions defined in catalog
- [ ] Each action has proper parameter validation
- [ ] Actions integrate with broker/extension
- [ ] WorldState updates after navigation
- [ ] Tab tracking works for multi-tab scenarios
- [ ] Tests pass

## Edge Cases to Handle
- Invalid URLs (missing protocol)
- Navigation timeout (> 30s)
- Tab not found for switch_to_tab
- Can't close last tab

## Resources
- Current navigation implementation: `crates/rzn_core/src/actions.rs`
- Browser extension handler: `extension/src/content/actions.ts`

## Time Estimate: 6 hours