# Issue #104: Implement DOM Hash Utility for Change Detection

## Priority: 🟡 Important (Week 1)

## Description
Create a utility to hash the DOM state and detect when actions have no effect. This is critical for the "ActionNoEffect" error category.

## Background
When we click a button or type text, we need to know if the DOM actually changed. Comparing full DOM is expensive, so we use SHA-1 hashes.

## Requirements

### 1. Create DOM Hashing Function
```rust
// In crates/rzn_plan/src/dom_utils.rs

use sha1::{Sha1, Digest};

pub fn hash_dom(dom: &CompressedDom) -> String {
    let mut hasher = Sha1::new();
    
    // Include structural elements
    hasher.update(&dom.structure_hash);
    
    // Include interactive elements (order matters!)
    for elem in &dom.interactive_elements {
        hasher.update(&elem.selector);
        hasher.update(&elem.text);
        hasher.update(&elem.tag_name);
        
        // Include key attributes that might change
        if let Some(value) = &elem.value {
            hasher.update(value);
        }
        if let Some(href) = &elem.href {
            hasher.update(href);
        }
    }
    
    // Include visible text content
    hasher.update(&dom.text_content);
    
    format!("{:x}", hasher.finalize())
}
```

### 2. Create Change Detection
```rust
pub struct DomChangeDetector {
    previous_hash: Option<String>,
    no_change_count: u32,
}

impl DomChangeDetector {
    pub fn new() -> Self {
        Self {
            previous_hash: None,
            no_change_count: 0,
        }
    }
    
    pub fn check_change(&mut self, current_dom: &CompressedDom) -> ChangeResult {
        let current_hash = hash_dom(current_dom);
        
        let result = match &self.previous_hash {
            None => ChangeResult::FirstObservation,
            Some(prev) if prev == &current_hash => {
                self.no_change_count += 1;
                ChangeResult::NoChange {
                    consecutive_count: self.no_change_count
                }
            }
            Some(_) => {
                self.no_change_count = 0;
                ChangeResult::Changed
            }
        };
        
        self.previous_hash = Some(current_hash);
        result
    }
}

pub enum ChangeResult {
    FirstObservation,
    Changed,
    NoChange { consecutive_count: u32 },
}
```

### 3. Integrate with Action Execution
```rust
// In autonomous_loop
let dom_before = world_state.dom.clone();
let hash_before = hash_dom(&dom_before);

// Execute action
let result = execute_action(&action, &params).await?;

// Re-observe
world_state = observe_current_state().await?;
let hash_after = hash_dom(&world_state.dom);

// Detect if action had effect
if hash_before == hash_after {
    warn!("Action had no effect on DOM");
    memory.last_error_category = Some(ErrorCategory::ActionNoEffect);
}
```

### 4. Add Structural Hash Generation
```rust
// In extension TypeScript
function generateStructuralHash(doc: Document): string {
    // Create a simplified representation of DOM structure
    const structure = [];
    
    const traverse = (element: Element, depth: number) => {
        if (depth > 5) return; // Limit depth
        
        // Only include semantically important tags
        const importantTags = ['DIV', 'FORM', 'INPUT', 'BUTTON', 'A', 'SECTION', 'MAIN'];
        if (importantTags.includes(element.tagName)) {
            structure.push(`${' '.repeat(depth)}${element.tagName}#${element.id || ''}.${element.className || ''}`);
        }
        
        for (const child of element.children) {
            traverse(child, depth + 1);
        }
    };
    
    traverse(document.body, 0);
    return structure.join('\n');
}
```

## Test Cases

### Unit Tests
```rust
#[test]
fn test_dom_hash_stability() {
    let dom1 = create_test_dom();
    let dom2 = create_test_dom(); // Identical
    
    assert_eq!(hash_dom(&dom1), hash_dom(&dom2));
}

#[test]
fn test_dom_hash_sensitivity() {
    let mut dom1 = create_test_dom();
    let mut dom2 = create_test_dom();
    
    // Change one element's text
    dom2.interactive_elements[0].text = "Different".to_string();
    
    assert_ne!(hash_dom(&dom1), hash_dom(&dom2));
}

#[test]
fn test_change_detection() {
    let mut detector = DomChangeDetector::new();
    let dom = create_test_dom();
    
    // First observation
    assert!(matches!(detector.check_change(&dom), ChangeResult::FirstObservation));
    
    // No change
    assert!(matches!(
        detector.check_change(&dom), 
        ChangeResult::NoChange { consecutive_count: 1 }
    ));
}
```

### Integration Test
```rust
#[tokio::test]
async fn test_action_effect_detection() {
    // Click a button that does nothing
    let before = observe_current_state().await?;
    execute_action("click_element", &json!({
        "selector": ".disabled-button"
    })).await?;
    let after = observe_current_state().await?;
    
    assert_eq!(hash_dom(&before.dom), hash_dom(&after.dom));
}
```

## Performance Requirements
- Hashing must complete in < 10ms for typical DOM
- Should handle DOMs with 1000+ elements
- Memory usage < 1MB

## Edge Cases
1. Dynamic content (ads, timestamps) - exclude from hash
2. Invisible changes (style, attributes) - include critical ones
3. Order changes - interactive elements order matters
4. Shadow DOM - include in structural hash

## Acceptance Criteria
- [ ] Hash function is deterministic
- [ ] Detects text changes
- [ ] Detects structural changes
- [ ] Detects attribute changes (value, href)
- [ ] Ignores irrelevant changes (timestamps, ads)
- [ ] Performance < 10ms
- [ ] Integration with error detection works

## Resources
- SHA-1 crate: `sha1 = "0.10"`
- DOM compression: `crates/rzn_plan/src/dom_analyzer.rs`

## Time Estimate: 4 hours