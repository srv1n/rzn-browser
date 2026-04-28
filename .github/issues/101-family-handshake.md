# Issue #101: Implement Family Handshake Protocol

## Priority: 🔴 Critical (Week 1)

## Description
Implement the two-tier action taxonomy handshake that allows the LLM to first select an action family (Navigate, Interact, etc.) before seeing the detailed actions.

## Background
Currently, sending all 40+ actions to the LLM costs ~10,000 tokens per turn. The handshake protocol reduces this to ~500 tokens by showing only 6 families initially.

## Requirements

### 1. Create Family Selection Types
```rust
// In crates/rzn_plan/src/action_taxonomy.rs

#[derive(Debug, Deserialize)]
pub struct FamilySelection {
    pub family: ActionFamily,
    pub intent: String,  // Brief explanation from LLM
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum LLMDecision {
    SelectFamily(FamilySelection),
    ExecuteAction(ActionExecution),
    Complete(CompletionData),
}
```

### 2. Implement Family Description Formatter
```rust
pub fn format_families_for_prompt() -> String {
    // Return ~50 tokens per family:
    // Navigate: URL navigation and tab management
    // Interact: Click, type, key press, and form interactions
    // Extract: Get data from page elements
    // Wait: Wait for conditions or elements
    // Handle: Manage popups, captchas, and interventions
    // Observe: Detect page state and capture information
}
```

### 3. Add Handshake State Tracking
```rust
// In PlanningMemory
pub current_family: Option<ActionFamily>,
pub last_family_injected: Option<ActionFamily>, // To skip re-injection
```

### 4. Create Unit Tests
```rust
#[test]
fn test_family_selection_parsing() {
    let json = r#"{"family": "Navigate", "intent": "go to YouTube"}"#;
    let decision: FamilySelection = serde_json::from_str(json).unwrap();
    assert_eq!(decision.family, ActionFamily::Navigate);
}

#[test]
fn test_family_format_token_count() {
    let formatted = format_families_for_prompt();
    let token_count = estimate_tokens(&formatted);
    assert!(token_count < 400); // 6 families * 50 tokens + overhead
}
```

## Acceptance Criteria
- [ ] Family selection JSON parsing works
- [ ] Family descriptions stay under 400 tokens total
- [ ] Handshake state is tracked in memory
- [ ] Unit tests pass
- [ ] Integration test shows family → action flow

## Files to Modify
- `crates/rzn_plan/src/action_taxonomy.rs`
- `crates/rzn_plan/src/planner_loop.rs`
- `crates/rzn_plan/src/prompt_builder.rs`

## Example Test Case
```
Turn 1: Show families → LLM picks "Navigate"
Turn 2: Show Navigate actions → LLM picks "navigate_to_url"
Turn 3: Show families again (after action completes)
```

## Resources
- [Original Design Doc - Two-Level Tool Ontology](../LLM_AUTONOMOUS_MODE_DESIGN.md#two-tiered-action-taxonomy)
- [Implementation Guide - Handshake](../LLM_IMPLEMENTATION_GUIDE.md#task-101-family-handshake-skeleton)

## Time Estimate: 4 hours