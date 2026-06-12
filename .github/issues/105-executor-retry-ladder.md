# Issue #105: Implement Executor Retry Ladder

## Priority: 🟡 Important (Week 1)

## Description
Implement the DOM-first, native-fallback execution pattern with proper retry logic and failure tracking.

## Background
90% of actions work with DOM manipulation, but some sites (YouTube, Google) need native input. We try DOM first, then escalate to native if needed.

## Retry Ladder Design

```
1. DOM with original selector
   ↓ (fails with recoverable error)
2. DOM with alternative selector (if provided)
   ↓ (fails)
3. Native input with same selector
   ↓ (fails)
4. Mark consecutive_failures += 1
   ↓ (if >= 3)
5. Request user intervention
```

## Implementation

### 1. Create Retry Configuration
```rust
// In crates/rzn_plan/src/executor.rs

pub struct RetryConfig {
    pub max_dom_attempts: u32,          // 2
    pub max_native_attempts: u32,       // 1  
    pub backoff_ms: u64,               // 100ms between attempts
    pub escalation_threshold: u32,      // 3 consecutive failures
    pub native_eligible_actions: HashSet<String>, // fill_input, click, etc
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_dom_attempts: 2,
            max_native_attempts: 1,
            backoff_ms: 100,
            escalation_threshold: 3,
            native_eligible_actions: vec![
                "fill_input_field",
                "click_element", 
                "submit_input",
                "press_special_key"
            ].into_iter().map(String::from).collect(),
        }
    }
}
```

### 2. Create Unified Executor
```rust
pub struct UnifiedExecutor {
    dom_executor: DomExecutor,
    native_executor: NativeExecutor,
    config: RetryConfig,
    telemetry: TelemetryCollector,
}

impl UnifiedExecutor {
    pub async fn execute_with_retry(
        &mut self,
        action: &str,
        params: &Value,
        memory: &mut PlanningMemory
    ) -> Result<ActionResult> {
        let start_time = Instant::now();
        
        // Check if action is eligible for native fallback
        let can_use_native = self.config.native_eligible_actions.contains(action);
        
        // Try DOM layer first
        for attempt in 0..self.config.max_dom_attempts {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(self.config.backoff_ms)).await;
            }
            
            match self.dom_executor.execute(action, params).await {
                Ok(result) => {
                    self.telemetry.record_success(action, ExecutionLayer::Dom, attempt + 1);
                    return Ok(ActionResult {
                        success: true,
                        data: result.data,
                        layer_used: ExecutionLayer::Dom,
                        execution_time_ms: start_time.elapsed().as_millis() as u64,
                    });
                }
                Err(DomError::SelectorNotFound) if attempt == 0 => {
                    // Try alternative selector on second attempt
                    if let Some(alt_selector) = self.suggest_alternative_selector(params) {
                        let mut alt_params = params.clone();
                        alt_params["selector"] = json!(alt_selector);
                        params = &alt_params;
                        continue;
                    }
                }
                Err(e) => {
                    warn!("DOM attempt {} failed: {:?}", attempt + 1, e);
                }
            }
        }
        
        // Try native layer if eligible
        if can_use_native {
            for attempt in 0..self.config.max_native_attempts {
                if attempt > 0 {
                    tokio::time::sleep(Duration::from_millis(self.config.backoff_ms)).await;
                }
                
                match self.native_executor.execute(action, params).await {
                    Ok(result) => {
                        self.telemetry.record_success(action, ExecutionLayer::Native, attempt + 1);
                        info!("Native fallback succeeded for {}", action);
                        return Ok(ActionResult {
                            success: true,
                            data: result.data,
                            layer_used: ExecutionLayer::Native,
                            execution_time_ms: start_time.elapsed().as_millis() as u64,
                        });
                    }
                    Err(e) => {
                        error!("Native attempt {} failed: {:?}", attempt + 1, e);
                    }
                }
            }
        }
        
        // All attempts failed
        memory.consecutive_failures += 1;
        self.telemetry.record_failure(action, memory.consecutive_failures);
        
        Err(ExecutorError::AllAttemptsFailed {
            action: action.to_string(),
            attempts: self.config.max_dom_attempts + self.config.max_native_attempts,
        })
    }
    
    fn suggest_alternative_selector(&self, params: &Value) -> Option<String> {
        let selector = params["selector"].as_str()?;
        
        // Common selector alternatives
        match selector {
            s if s.contains("#") => {
                // Try class selector instead of ID
                Some(s.replace("#", "."))
            }
            s if s.contains("[name=") => {
                // Try ID selector for named inputs
                let name = s.split("'").nth(1)?;
                Some(format!("#{}", name))
            }
            _ => None
        }
    }
}
```

### 3. Error Classification
```rust
#[derive(Debug)]
pub enum DomError {
    SelectorNotFound,
    ElementNotInteractable, 
    CrossOriginFrame,
    TimeoutExceeded,
    Other(String),
}

impl DomError {
    pub fn is_recoverable(&self) -> bool {
        matches!(self, 
            DomError::SelectorNotFound | 
            DomError::ElementNotInteractable
        )
    }
    
    pub fn needs_native_fallback(&self) -> bool {
        matches!(self,
            DomError::ElementNotInteractable |
            DomError::CrossOriginFrame
        )
    }
}
```

### 4. Telemetry Collection
```rust
pub struct TelemetryCollector {
    action_stats: HashMap<String, ActionStats>,
}

#[derive(Default)]
pub struct ActionStats {
    pub dom_success: u32,
    pub dom_failure: u32,
    pub native_success: u32,
    pub native_failure: u32,
    pub avg_retry_count: f32,
}

impl TelemetryCollector {
    pub fn record_success(&mut self, action: &str, layer: ExecutionLayer, attempts: u32) {
        let stats = self.action_stats.entry(action.to_string()).or_default();
        match layer {
            ExecutionLayer::Dom => stats.dom_success += 1,
            ExecutionLayer::Native => stats.native_success += 1,
        }
        // Update average retry count
    }
    
    pub fn get_success_rate(&self, action: &str) -> f32 {
        if let Some(stats) = self.action_stats.get(action) {
            let total = stats.dom_success + stats.dom_failure + 
                       stats.native_success + stats.native_failure;
            let success = stats.dom_success + stats.native_success;
            if total > 0 {
                success as f32 / total as f32
            } else {
                0.0
            }
        } else {
            0.0
        }
    }
}
```

## Test Cases

### Unit Tests
```rust
#[test]
fn test_retry_config_defaults() {
    let config = RetryConfig::default();
    assert_eq!(config.max_dom_attempts, 2);
    assert!(config.native_eligible_actions.contains("fill_input_field"));
}

#[test]
fn test_alternative_selector_generation() {
    let executor = UnifiedExecutor::new();
    
    // ID to class
    assert_eq!(
        executor.suggest_alternative_selector(&json!({"selector": "#search"})),
        Some(".search".to_string())
    );
    
    // Name to ID
    assert_eq!(
        executor.suggest_alternative_selector(&json!({"selector": "input[name='q']"})),
        Some("input#q".to_string())
    );
}
```

### Integration Test
```rust
#[tokio::test] 
async fn test_dom_to_native_escalation() {
    let mut executor = UnifiedExecutor::new();
    let mut memory = PlanningMemory::new();
    
    // Mock DOM executor to fail
    executor.dom_executor = MockDomExecutor::new()
        .with_failure(DomError::ElementNotInteractable);
    
    // Execute should escalate to native
    let result = executor.execute_with_retry(
        "fill_input_field",
        &json!({
            "selector": "input#search",
            "value": "test"
        }),
        &mut memory
    ).await;
    
    assert!(result.is_ok());
    assert_eq!(result.unwrap().layer_used, ExecutionLayer::Native);
}
```

## Acceptance Criteria
- [ ] DOM attempts happen first (up to 2 times)
- [ ] Native fallback only for eligible actions
- [ ] Alternative selectors are tried
- [ ] Backoff delay between attempts
- [ ] Consecutive failures tracked
- [ ] Telemetry records all attempts
- [ ] Success rates calculable per action

## Performance Requirements
- Retry overhead < 200ms total
- Telemetry updates < 1ms
- Memory usage for stats < 10KB

## Edge Cases
1. Selector exists but element is invisible
2. Element appears after delay (dynamic content)
3. Cross-origin iframes block access
4. Native input unavailable (headless mode)

## Resources
- Current executor: `crates/rzn_plan/src/executor.rs`
- DOM executor: `extension/src/content/actions.ts`
- Native executor: `crates/rzn_broker/src/native_input.rs`

## Time Estimate: 6 hours