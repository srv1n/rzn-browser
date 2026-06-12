# RZN Browser Automation Framework - Architect's Deep Dive
## Part 5: LLM Integration and Planning System

### LLM Architecture Overview

The LLM integration is the intelligence layer that converts high-level goals into executable browser actions. It uses a two-stage routing system to minimize token usage while maximizing accuracy.

#### Key Design Principles

1. **Token Efficiency**: Keep context under 8KB through intelligent summarization
2. **Two-Stage Routing**: Mode selection → Action selection
3. **Memory Management**: Rolling summaries with exponential decay
4. **Provider Agnostic**: Support for OpenAI, Anthropic, Google, and local models
5. **Retry Logic**: Automatic retry with backoff for transient failures

### LLM Client Implementation

```rust
// crates/rzn_plan/src/llm_client.rs
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[async_trait]
pub trait LLMProvider: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, Error>;
    async fn complete_with_tools(&self, request: ToolRequest) -> Result<ToolResponse, Error>;
    fn max_context_tokens(&self) -> usize;
    fn name(&self) -> &str;
}

pub struct LLMClient {
    provider: Arc<dyn LLMProvider>,
    config: LLMConfig,
    context_manager: ContextManager,
    tool_registry: ToolRegistry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMConfig {
    pub model: String,
    pub temperature: f32,
    pub max_tokens: usize,
    pub timeout_ms: u64,
    pub retry_attempts: u32,
    pub system_prompt: String,
}

impl LLMClient {
    pub fn new(provider: impl LLMProvider + 'static, config: LLMConfig) -> Self {
        Self {
            provider: Arc::new(provider),
            config,
            context_manager: ContextManager::new(),
            tool_registry: ToolRegistry::new(),
        }
    }
    
    pub fn with_tools(mut self, tools: Vec<Tool>) -> Self {
        for tool in tools {
            self.tool_registry.register(tool);
        }
        self
    }
    
    pub async fn plan_action(
        &self,
        goal: &str,
        context: &PlanContext,
    ) -> Result<PlannedAction, Error> {
        // Stage 1: Choose mode
        let mode = self.choose_mode(goal, context).await?;
        
        // Stage 2: Choose specific action
        let action = self.choose_action(&mode, goal, context).await?;
        
        Ok(PlannedAction {
            mode,
            action,
            confidence: self.calculate_confidence(&action, context),
        })
    }
    
    async fn choose_mode(
        &self,
        goal: &str,
        context: &PlanContext,
    ) -> Result<ActionMode, Error> {
        let request = ToolRequest {
            messages: vec![
                Message {
                    role: Role::System,
                    content: CHOOSE_MODE_PROMPT.to_string(),
                },
                Message {
                    role: Role::User,
                    content: format!(
                        "Goal: {}\nContext: {}\nAvailable modes: {:?}",
                        goal,
                        context.to_summary(),
                        ActionMode::all()
                    ),
                },
            ],
            tools: vec![self.tool_registry.get("choose_mode").unwrap()],
            tool_choice: ToolChoice::Required,
        };
        
        let response = self.provider.complete_with_tools(request).await?;
        
        // Parse mode from tool call
        let mode = response.tool_calls
            .first()
            .and_then(|call| call.arguments.get("mode"))
            .and_then(|m| serde_json::from_value::<ActionMode>(m.clone()).ok())
            .ok_or_else(|| Error::InvalidLLMResponse("No mode selected".to_string()))?;
        
        Ok(mode)
    }
    
    async fn choose_action(
        &self,
        mode: &ActionMode,
        goal: &str,
        context: &PlanContext,
    ) -> Result<Action, Error> {
        let available_actions = mode.available_actions();
        
        let request = ToolRequest {
            messages: vec![
                Message {
                    role: Role::System,
                    content: CHOOSE_ACTION_PROMPT.to_string(),
                },
                Message {
                    role: Role::User,
                    content: format!(
                        "Mode: {:?}\nGoal: {}\nContext: {}\nAvailable actions: {:?}",
                        mode,
                        goal,
                        context.to_summary(),
                        available_actions
                    ),
                },
            ],
            tools: vec![self.tool_registry.get("choose_action").unwrap()],
            tool_choice: ToolChoice::Required,
        };
        
        let response = self.provider.complete_with_tools(request).await?;
        
        // Parse action from tool call
        let action = response.tool_calls
            .first()
            .and_then(|call| serde_json::from_value::<Action>(call.arguments.clone()).ok())
            .ok_or_else(|| Error::InvalidLLMResponse("No action selected".to_string()))?;
        
        Ok(action)
    }
}

// System prompts
const CHOOSE_MODE_PROMPT: &str = r#"
You are RZN Planner, a browser automation assistant. Your task is to choose the appropriate high-level mode for the next action.

Modes:
- navigate: Change URL, open/close tabs
- act: Click, type, select, drag, scroll
- extract: Get data from page
- wait: Wait for conditions
- assert: Verify page state
- browser: Manage cookies, storage, downloads
- human: Request user intervention
- system: Low-level operations

Choose based on the immediate next step needed to progress toward the goal.
Be specific and deterministic. Return only the mode name.
"#;

const CHOOSE_ACTION_PROMPT: &str = r#"
You are RZN Planner. Given the selected mode, choose the specific action and fill its parameters.

Rules:
- Use role+name selectors when possible (e.g., role="button", name="Submit")
- Be precise with selectors
- Include reasonable timeouts
- Prefer stable selectors over positional ones
- Return complete action JSON
"#;
```

### Provider Implementations

#### OpenAI Provider

```rust
// crates/rzn_plan/src/providers/openai.rs
use async_openai::{Client, types::*};

pub struct OpenAIProvider {
    client: Client,
    model: String,
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, Error> {
        let openai_request = CreateChatCompletionRequest {
            model: self.model.clone(),
            messages: request.messages.into_iter().map(|m| {
                match m.role {
                    Role::System => ChatCompletionRequestSystemMessage {
                        content: m.content,
                        ..Default::default()
                    }.into(),
                    Role::User => ChatCompletionRequestUserMessage {
                        content: m.content.into(),
                        ..Default::default()
                    }.into(),
                    Role::Assistant => ChatCompletionRequestAssistantMessage {
                        content: Some(m.content),
                        ..Default::default()
                    }.into(),
                }
            }).collect(),
            temperature: Some(request.temperature),
            max_tokens: Some(request.max_tokens as u16),
            ..Default::default()
        };
        
        let response = self.client
            .chat()
            .create(openai_request)
            .await
            .map_err(|e| Error::ProviderError(e.to_string()))?;
        
        let content = response.choices
            .first()
            .and_then(|c| c.message.content.clone())
            .ok_or_else(|| Error::InvalidResponse)?;
        
        Ok(CompletionResponse {
            content,
            usage: Usage {
                prompt_tokens: response.usage.map(|u| u.prompt_tokens).unwrap_or(0),
                completion_tokens: response.usage.map(|u| u.completion_tokens).unwrap_or(0),
            },
        })
    }
    
    async fn complete_with_tools(&self, request: ToolRequest) -> Result<ToolResponse, Error> {
        let tools: Vec<ChatCompletionTool> = request.tools.into_iter().map(|tool| {
            ChatCompletionTool {
                r#type: ChatCompletionToolType::Function,
                function: FunctionObject {
                    name: tool.name,
                    description: Some(tool.description),
                    parameters: Some(tool.parameters),
                },
            }
        }).collect();
        
        let openai_request = CreateChatCompletionRequest {
            model: self.model.clone(),
            messages: convert_messages(request.messages),
            tools: Some(tools),
            tool_choice: match request.tool_choice {
                ToolChoice::Auto => Some(ChatCompletionToolChoiceOption::Auto),
                ToolChoice::None => None,
                ToolChoice::Required => Some(ChatCompletionToolChoiceOption::Required),
                ToolChoice::Specific(name) => Some(ChatCompletionToolChoiceOption::Named(
                    ChatCompletionNamedToolChoice {
                        r#type: ChatCompletionToolType::Function,
                        function: FunctionName { name },
                    }
                )),
            },
            ..Default::default()
        };
        
        let response = self.client
            .chat()
            .create(openai_request)
            .await?;
        
        let tool_calls = response.choices
            .first()
            .and_then(|c| c.message.tool_calls.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|tc| ToolCall {
                id: tc.id,
                name: tc.function.name,
                arguments: serde_json::from_str(&tc.function.arguments).unwrap_or_default(),
            })
            .collect();
        
        Ok(ToolResponse {
            tool_calls,
            usage: convert_usage(response.usage),
        })
    }
    
    fn max_context_tokens(&self) -> usize {
        match self.model.as_str() {
            "gpt-4-turbo" | "gpt-5-mini-2025-08-07" => 128000,
            "gpt-4" => 8192,
            "gpt-3.5-turbo" => 16384,
            _ => 4096,
        }
    }
    
    fn name(&self) -> &str {
        "OpenAI"
    }
}
```

#### Anthropic Provider

```rust
// crates/rzn_plan/src/providers/anthropic.rs
use anthropic_sdk::{Client, messages::*};

pub struct AnthropicProvider {
    client: Client,
    model: String,
}

#[async_trait]
impl LLMProvider for AnthropicProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, Error> {
        let mut system_prompt = String::new();
        let mut messages = Vec::new();
        
        for msg in request.messages {
            match msg.role {
                Role::System => system_prompt.push_str(&msg.content),
                Role::User => messages.push(Message {
                    role: "user".to_string(),
                    content: msg.content,
                }),
                Role::Assistant => messages.push(Message {
                    role: "assistant".to_string(),
                    content: msg.content,
                }),
            }
        }
        
        let response = self.client
            .messages()
            .create(CreateMessageRequest {
                model: self.model.clone(),
                messages,
                system: Some(system_prompt),
                max_tokens: request.max_tokens,
                temperature: Some(request.temperature),
                ..Default::default()
            })
            .await?;
        
        let content = response.content
            .into_iter()
            .filter_map(|c| match c {
                ContentBlock::Text { text } => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        
        Ok(CompletionResponse {
            content,
            usage: Usage {
                prompt_tokens: response.usage.input_tokens,
                completion_tokens: response.usage.output_tokens,
            },
        })
    }
    
    fn max_context_tokens(&self) -> usize {
        match self.model.as_str() {
            "claude-3-opus" => 200000,
            "claude-3-sonnet" => 200000,
            "claude-3-haiku" => 200000,
            "claude-2.1" => 100000,
            "claude-2" => 100000,
            _ => 100000,
        }
    }
    
    fn name(&self) -> &str {
        "Anthropic"
    }
}
```

### Context Management

```rust
// crates/rzn_plan/src/context_manager.rs
use circular_buffer::CircularBuffer;

pub struct ContextManager {
    short_term: CircularBuffer<ContextEntry>,
    long_term: Vec<ContextSummary>,
    max_tokens: usize,
    compression_ratio: f32,
}

#[derive(Debug, Clone)]
struct ContextEntry {
    timestamp: DateTime<Utc>,
    action: String,
    result: String,
    importance: f32,
    tokens: usize,
}

#[derive(Debug, Clone)]
struct ContextSummary {
    period: DateRange,
    summary: String,
    key_events: Vec<String>,
    tokens: usize,
}

impl ContextManager {
    pub fn new() -> Self {
        Self {
            short_term: CircularBuffer::with_capacity(100),
            long_term: Vec::new(),
            max_tokens: 8000,
            compression_ratio: 0.3,
        }
    }
    
    pub fn add_entry(&mut self, action: &str, result: &str) {
        let entry = ContextEntry {
            timestamp: Utc::now(),
            action: action.to_string(),
            result: result.to_string(),
            importance: self.calculate_importance(action, result),
            tokens: self.estimate_tokens(&format!("{}: {}", action, result)),
        };
        
        self.short_term.push_back(entry);
        
        // Compress if needed
        if self.total_tokens() > self.max_tokens {
            self.compress();
        }
    }
    
    pub fn get_context(&self, max_tokens: usize) -> String {
        let mut context = String::new();
        let mut remaining_tokens = max_tokens;
        
        // Add recent entries (most important)
        for entry in self.short_term.iter().rev() {
            if entry.tokens <= remaining_tokens {
                context.push_str(&format!(
                    "[{}] {}: {}\n",
                    entry.timestamp.format("%H:%M:%S"),
                    entry.action,
                    entry.result
                ));
                remaining_tokens -= entry.tokens;
            } else {
                break;
            }
        }
        
        // Add summaries if space available
        for summary in self.long_term.iter().rev() {
            if summary.tokens <= remaining_tokens {
                context.push_str(&format!(
                    "Period {}-{}: {}\n",
                    summary.period.start.format("%H:%M"),
                    summary.period.end.format("%H:%M"),
                    summary.summary
                ));
                remaining_tokens -= summary.tokens;
            } else {
                break;
            }
        }
        
        context
    }
    
    fn compress(&mut self) {
        // Take oldest 50% of entries
        let compress_count = self.short_term.len() / 2;
        let mut to_compress = Vec::new();
        
        for _ in 0..compress_count {
            if let Some(entry) = self.short_term.pop_front() {
                to_compress.push(entry);
            }
        }
        
        if to_compress.is_empty() {
            return;
        }
        
        // Create summary
        let summary = self.create_summary(to_compress);
        self.long_term.push(summary);
        
        // Limit long-term storage
        if self.long_term.len() > 10 {
            self.long_term.remove(0);
        }
    }
    
    fn create_summary(&self, entries: Vec<ContextEntry>) -> ContextSummary {
        let start = entries.first().unwrap().timestamp;
        let end = entries.last().unwrap().timestamp;
        
        // Group by action type
        let mut action_counts = HashMap::new();
        let mut key_results = Vec::new();
        
        for entry in &entries {
            *action_counts.entry(entry.action.clone()).or_insert(0) += 1;
            
            if entry.importance > 0.7 {
                key_results.push(format!("{}: {}", entry.action, entry.result));
            }
        }
        
        // Create summary text
        let summary = format!(
            "Performed {} actions: {}. Key outcomes: {}",
            entries.len(),
            action_counts.iter()
                .map(|(k, v)| format!("{} ({}x)", k, v))
                .collect::<Vec<_>>()
                .join(", "),
            if key_results.is_empty() {
                "none".to_string()
            } else {
                key_results.join("; ")
            }
        );
        
        ContextSummary {
            period: DateRange { start, end },
            summary: summary.clone(),
            key_events: key_results,
            tokens: self.estimate_tokens(&summary),
        }
    }
    
    fn calculate_importance(&self, action: &str, result: &str) -> f32 {
        let mut score = 0.5;
        
        // Navigation is important
        if action.contains("navigate") {
            score += 0.3;
        }
        
        // Errors are important
        if result.contains("error") || result.contains("failed") {
            score += 0.3;
        }
        
        // Extractions are important
        if action.contains("extract") {
            score += 0.2;
        }
        
        score.min(1.0)
    }
    
    fn estimate_tokens(&self, text: &str) -> usize {
        // Rough estimation: 1 token ≈ 4 characters
        text.len() / 4
    }
}
```

### Planning Orchestrator

```rust
// crates/rzn_plan/src/orchestrator.rs
use std::collections::VecDeque;

pub struct PlanningOrchestrator {
    llm_client: LLMClient,
    executor: ActionExecutor,
    state: RunState,
    plan_queue: VecDeque<PlannedAction>,
    max_steps: usize,
    backtrack_enabled: bool,
}

impl PlanningOrchestrator {
    pub async fn execute_goal(&mut self, goal: String) -> Result<GoalResult, Error> {
        info!("Starting goal execution: {}", goal);
        
        self.state = RunState::new(goal.clone());
        let start_time = Instant::now();
        
        // Initial planning
        let initial_plan = self.create_initial_plan(&goal).await?;
        self.plan_queue.extend(initial_plan);
        
        // Execution loop
        while !self.is_goal_achieved(&goal).await? && self.state.steps.len() < self.max_steps {
            // Get next action
            let action = match self.plan_queue.pop_front() {
                Some(action) => action,
                None => {
                    // Need to plan more steps
                    let context = self.build_plan_context().await?;
                    let next_action = self.llm_client.plan_action(&goal, &context).await?;
                    next_action
                }
            };
            
            // Execute action
            let result = self.execute_action(action.clone()).await;
            
            // Handle result
            match result {
                Ok(success) => {
                    self.state.record_success(action, success);
                    
                    // Check if we need to replan
                    if self.should_replan(&success) {
                        self.replan(&goal).await?;
                    }
                }
                Err(error) => {
                    self.state.record_failure(action.clone(), error.clone());
                    
                    // Try to recover
                    if !self.recover_from_error(action, error).await? {
                        // Can't recover, might need to backtrack
                        if self.backtrack_enabled {
                            self.backtrack().await?;
                        } else {
                            return Err(Error::Unrecoverable(error.to_string()));
                        }
                    }
                }
            }
            
            // Update progress
            self.report_progress(&goal).await;
        }
        
        // Prepare result
        Ok(GoalResult {
            goal,
            success: self.is_goal_achieved(&goal).await?,
            steps_taken: self.state.steps.len(),
            duration: start_time.elapsed(),
            final_state: self.state.clone(),
        })
    }
    
    async fn create_initial_plan(&self, goal: &str) -> Result<Vec<PlannedAction>, Error> {
        let prompt = format!(
            r#"Create a step-by-step plan to achieve this goal: {}
            
            Current page: {}
            Available actions: Navigate, Click, Type, Extract, Wait
            
            Return a list of high-level steps (not detailed actions).
            Keep it concise and focused."#,
            goal,
            self.get_current_page_summary().await?
        );
        
        let response = self.llm_client.complete(CompletionRequest {
            messages: vec![
                Message {
                    role: Role::System,
                    content: "You are a browser automation planner.".to_string(),
                },
                Message {
                    role: Role::User,
                    content: prompt,
                },
            ],
            temperature: 0.3,
            max_tokens: 500,
        }).await?;
        
        // Parse plan into actions
        self.parse_plan_text(&response.content)
    }
    
    async fn execute_action(&mut self, action: PlannedAction) -> Result<ActionResult, Error> {
        info!("Executing action: {:?}", action);
        
        // Convert to concrete action
        let concrete_action = self.concretize_action(action).await?;
        
        // Execute with retries
        let mut attempts = 0;
        let max_attempts = 3;
        
        loop {
            attempts += 1;
            
            match self.executor.execute(concrete_action.clone()).await {
                Ok(result) => return Ok(result),
                Err(e) if attempts < max_attempts && e.is_retryable() => {
                    warn!("Action failed (attempt {}/{}): {}", attempts, max_attempts, e);
                    tokio::time::sleep(Duration::from_secs(attempts as u64)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
    
    async fn should_replan(&self, result: &ActionResult) -> bool {
        // Replan if:
        // 1. Navigation occurred
        if result.navigation_occurred {
            return true;
        }
        
        // 2. Unexpected result
        if result.unexpected {
            return true;
        }
        
        // 3. Plan queue is empty
        if self.plan_queue.is_empty() {
            return true;
        }
        
        false
    }
    
    async fn replan(&mut self, goal: &str) -> Result<(), Error> {
        info!("Replanning for goal: {}", goal);
        
        // Clear current queue
        self.plan_queue.clear();
        
        // Get current context
        let context = self.build_plan_context().await?;
        
        // Ask LLM for next steps
        let prompt = format!(
            r#"Goal: {}
            Progress so far: {}
            Current state: {}
            
            What should we do next? Provide 1-3 concrete next steps."#,
            goal,
            self.state.get_progress_summary(),
            context.to_summary()
        );
        
        let response = self.llm_client.complete(CompletionRequest {
            messages: vec![
                Message {
                    role: Role::System,
                    content: "You are a browser automation planner. Be concrete and specific.".to_string(),
                },
                Message {
                    role: Role::User,
                    content: prompt,
                },
            ],
            temperature: 0.3,
            max_tokens: 300,
        }).await?;
        
        // Parse and add to queue
        let new_actions = self.parse_plan_text(&response.content)?;
        self.plan_queue.extend(new_actions);
        
        Ok(())
    }
    
    async fn recover_from_error(
        &mut self,
        failed_action: PlannedAction,
        error: Error,
    ) -> Result<bool, Error> {
        info!("Attempting to recover from error: {}", error);
        
        // Ask LLM for recovery strategy
        let prompt = format!(
            r#"An action failed. Help me recover.
            
            Failed action: {:?}
            Error: {}
            Current page: {}
            
            Suggest ONE recovery action. Be specific."#,
            failed_action,
            error,
            self.get_current_page_summary().await?
        );
        
        let response = self.llm_client.complete(CompletionRequest {
            messages: vec![
                Message {
                    role: Role::System,
                    content: "You are a browser automation recovery expert.".to_string(),
                },
                Message {
                    role: Role::User,
                    content: prompt,
                },
            ],
            temperature: 0.5,
            max_tokens: 200,
        }).await?;
        
        // Try recovery action
        if let Ok(recovery_action) = self.parse_single_action(&response.content) {
            self.plan_queue.push_front(recovery_action);
            Ok(true)
        } else {
            Ok(false)
        }
    }
    
    async fn is_goal_achieved(&self, goal: &str) -> Result<bool, Error> {
        // Ask LLM to evaluate
        let prompt = format!(
            r#"Evaluate if this goal has been achieved:
            Goal: {}
            
            Current state:
            - URL: {}
            - Page content summary: {}
            - Actions completed: {}
            
            Reply with just "YES" or "NO"."#,
            goal,
            self.state.current_url,
            self.get_current_page_summary().await?,
            self.state.get_actions_summary()
        );
        
        let response = self.llm_client.complete(CompletionRequest {
            messages: vec![
                Message {
                    role: Role::System,
                    content: "You evaluate goal achievement. Be strict.".to_string(),
                },
                Message {
                    role: Role::User,
                    content: prompt,
                },
            ],
            temperature: 0.1,
            max_tokens: 10,
        }).await?;
        
        Ok(response.content.trim().to_uppercase() == "YES")
    }
}
```

### Prompt Engineering

```rust
// crates/rzn_plan/src/prompts.rs

pub struct PromptBuilder {
    templates: HashMap<String, String>,
    variables: HashMap<String, String>,
}

impl PromptBuilder {
    pub fn new() -> Self {
        let mut builder = Self {
            templates: HashMap::new(),
            variables: HashMap::new(),
        };
        
        builder.load_default_templates();
        builder
    }
    
    fn load_default_templates(&mut self) {
        self.templates.insert(
            "element_selection".to_string(),
            r#"Select the best element to interact with.

Available elements:
{elements}

Goal: {goal}

Selection criteria:
1. Prefer semantic selectors (role, aria-label)
2. Avoid position-dependent selectors
3. Choose visible and interactable elements
4. Consider element text and context

Return the element ID and explain why briefly."#.to_string()
        );
        
        self.templates.insert(
            "action_planning".to_string(),
            r#"Plan the next action to achieve the goal.

Goal: {goal}
Current URL: {url}
Page summary: {page_summary}
Previous actions: {history}

Available actions:
{actions}

Rules:
- Be specific and concrete
- One action at a time
- Include all required parameters
- Prefer robust selectors

Return action JSON."#.to_string()
        );
        
        self.templates.insert(
            "error_recovery".to_string(),
            r#"Recover from an error.

Error: {error}
Failed action: {action}
Current state: {state}

Possible recovery strategies:
1. Retry with different selector
2. Wait for element
3. Navigate back
4. Try alternative approach

Suggest ONE recovery action."#.to_string()
        );
        
        self.templates.insert(
            "extraction".to_string(),
            r#"Extract structured data from the page.

Target data: {target}
Page content: {content}

Extract and structure the following fields:
{fields}

Return as JSON."#.to_string()
        );
    }
    
    pub fn build(&self, template_name: &str) -> Result<String, Error> {
        let template = self.templates
            .get(template_name)
            .ok_or_else(|| Error::TemplateNotFound(template_name.to_string()))?;
        
        let mut result = template.clone();
        
        // Replace variables
        for (key, value) in &self.variables {
            let placeholder = format!("{{{}}}", key);
            result = result.replace(&placeholder, value);
        }
        
        // Check for unreplaced variables
        if result.contains("{") && result.contains("}") {
            warn!("Prompt may contain unreplaced variables: {}", result);
        }
        
        Ok(result)
    }
    
    pub fn set(&mut self, key: &str, value: String) -> &mut Self {
        self.variables.insert(key.to_string(), value);
        self
    }
    
    pub fn set_many(&mut self, vars: HashMap<String, String>) -> &mut Self {
        self.variables.extend(vars);
        self
    }
}

// Specialized prompt builders
pub fn build_click_prompt(elements: &[Element], goal: &str) -> String {
    let elements_text = elements.iter().enumerate()
        .map(|(i, elem)| {
            format!(
                "{}. [{}] {} - {} ({})",
                i,
                elem.tag_name,
                elem.text.as_ref().unwrap_or(&"".to_string()),
                elem.get_selector(),
                if elem.visible { "visible" } else { "hidden" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    
    format!(
        r#"Which element should I click to achieve: {}
        
Elements:
{}

Reply with the element number and brief reason."#,
        goal,
        elements_text
    )
}

pub fn build_type_prompt(fields: &[FormField], data: &HashMap<String, String>) -> String {
    let fields_text = fields.iter()
        .map(|field| {
            format!(
                "- {} ({}): {}",
                field.name.as_ref().unwrap_or(&field.id.as_ref().unwrap_or(&"unnamed".to_string())),
                field.field_type,
                field.get_selector()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    
    format!(
        r#"Fill the form with this data: {:?}
        
Form fields:
{}

Return a list of (selector, value) pairs."#,
        data,
        fields_text
    )
}
```

### Autonomous Planning Mode

```rust
// crates/rzn_plan/src/llm_autonomous.rs

// Entry point:
//   LLMAutonomousPlanner::execute_autonomous(LLMAutonomousRequest)
// Key pieces:
//   - PlannerState (FSM) + PolicyValidator gate allowed actions per mode
//   - BrokerClient executes concrete StepKind actions
//   - parse_llm_response() converts model output into ActionCommand(s)
```

This completes Part 5, covering the LLM integration and planning system in detail.
