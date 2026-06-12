use crate::{
    broker_client::{BrokerClient, Transport},
    dom_analyzer::DomAnalyzer,
    dom_processor::{DomContext, DomProcessor},
    element_ref::{InputRung, ResolvedElement, ResultEnvelope, TargetSpec},
    failure_cache::{FailureCache, FailureContext},
    llm::LLMClient,
    mode_selector::{EscalationReason, ExecutionMode, ModeSelector, ModeStatistics},
    plan_sanitizer::PlanSanitizer,
    policy_gate::PolicyGate,
    prompt_builder::PromptBuilder,
    self_healing::SelfHealer,
    telemetry::{LLMUsage, TelemetryCollector, TelemetryConfig},
    wait_strategies::{DOMObservation, SmartWaitStrategy},
    workflow_manager::WorkflowManager,
    ExecutionResult, PlanConfig, PlanError, PlanRequest, PlanResponse, PlanResult, PlanningSession,
    RunRequest, RunResponse, StepExecution,
};
use base64::engine::general_purpose;
use base64::Engine;
use regex::{Regex, RegexBuilder};
use rzn_core::{Step, StepKind, Workflow};
// Removed heuristic planner dependencies - using LLM-only planning
use ansi_term;
use log::{debug, error, info, warn};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tokio::time::{timeout, Duration};
use uuid::Uuid;

/// Main orchestrator that coordinates LLM planning with workflow execution
pub struct Orchestrator {
    config: PlanConfig,
    llm_client: LLMClient,
    workflow_manager: WorkflowManager,
    dom_analyzer: DomAnalyzer,
    dom_processor: DomProcessor,
    prompt_builder: PromptBuilder,
    self_healer: SelfHealer,
    broker_client: BrokerClient,
    // heuristic_planner: HeuristicPlanner, // Removed - using LLM-only planning
    plan_sanitizer: PlanSanitizer,
    policy_gate: PolicyGate,
    current_dom_context: Option<DomContext>,
    dom_revision: u64, // For DOM delta tracking (optimization #7)
    telemetry_collector: Option<TelemetryCollector>,
    // New DOM snapshot system components
    failure_cache: FailureCache,
    use_snapshot_system: bool,
    // DOM hash tracking for loop detection in Validator tier
    dom_hash_history: Vec<String>,
    max_dom_hash_history: usize,
    // Smart wait strategies for dynamic content
    wait_strategy: SmartWaitStrategy,

    // CDP-based architecture components
    mode_selector: ModeSelector,
    current_domain: Option<String>,
    resolved_elements: HashMap<String, ResolvedElement>,
}

impl Orchestrator {
    fn substitute_known_params_in_string(text: &mut String, parameters: &HashMap<String, String>) {
        for (key, value) in parameters {
            let placeholder = format!("{{{}}}", key);
            *text = text.replace(&placeholder, value);
        }
    }

    fn strip_unresolved_param_placeholders(text: &mut String) {
        if !text.contains('{') {
            return;
        }

        let placeholder_re = Regex::new(r"\{[A-Za-z_][A-Za-z0-9_]*\}")
            .expect("static unresolved parameter regex must compile");
        *text = placeholder_re.replace_all(text, "").into_owned();
    }

    fn substitute_params_in_json_value(value: &mut Value, parameters: &HashMap<String, String>) {
        match value {
            Value::String(text) => {
                Self::substitute_known_params_in_string(text, parameters);
                Self::strip_unresolved_param_placeholders(text);
            }
            Value::Array(items) => {
                for item in items {
                    Self::substitute_params_in_json_value(item, parameters);
                }
            }
            Value::Object(map) => {
                for child in map.values_mut() {
                    Self::substitute_params_in_json_value(child, parameters);
                }
            }
            _ => {}
        }
    }

    fn required_parameter_names(workflow: &Workflow) -> Vec<String> {
        let mut out = HashSet::<String>::new();
        for seq in &workflow.browser_automation.sequences {
            for var in &seq.required_variables {
                if !var.name.trim().is_empty() {
                    out.insert(var.name.clone());
                }
            }
        }
        let mut v: Vec<String> = out.into_iter().collect();
        v.sort();
        v
    }

    fn apply_common_parameter_aliases(
        required: &HashSet<String>,
        params: &mut HashMap<String, String>,
    ) {
        // Alias groups: if any key in the group is provided, fill the others (do not override).
        // Keep generic: this is about parameter names, not domains.
        const GROUPS: &[&[&str]] = &[&["search_query", "query", "q"]];

        for group in GROUPS {
            if !group.iter().any(|k| required.contains(*k)) {
                continue;
            }

            let mut value: Option<String> = None;
            for &k in *group {
                if let Some(v) = params.get(k) {
                    value = Some(v.clone());
                    break;
                }
            }

            if let Some(v) = value {
                for &k in *group {
                    params.entry(k.to_string()).or_insert_with(|| v.clone());
                }
            }
        }
    }

    fn format_missing_params_error(
        workflow: &Workflow,
        missing: &[String],
        provided: &[String],
    ) -> String {
        let required = Self::required_parameter_names(workflow);
        let mut msg = format!(
            "Missing required workflow parameter(s): {}",
            missing.join(", ")
        );

        if !required.is_empty() {
            msg.push_str(&format!(". Required: {}", required.join(", ")));
        }
        if !provided.is_empty() {
            msg.push_str(&format!(". Provided: {}", provided.join(", ")));
        }

        // Friendly hint for the common search parameter name mismatch.
        if missing.iter().any(|m| m == "search_query") {
            msg.push_str(". Hint: this workflow expects 'search_query' (aliases: 'query', 'q').");
        }

        msg
    }

    pub async fn new(config: PlanConfig) -> PlanResult<Self> {
        let llm_client = LLMClient::new(&config)?;
        let mut workflow_manager = WorkflowManager::new(&config.workflows_dir)?;

        // Initialize workflow manager (creates directory if needed)
        workflow_manager.initialize().await?;

        let dom_analyzer = DomAnalyzer::new(config.max_dom_size);
        let dom_processor = DomProcessor::with_defaults();
        let prompt_builder = PromptBuilder::new();
        let self_healer = SelfHealer::new(config.max_healing_attempts);
        let policy_gate = PolicyGate::from_env();
        let transport = match config.broker_transport.as_str() {
            "native" | "endpoint" | "auto" => Transport::Native,
            "pipe" => Transport::Pipe,
            _ => Transport::Tcp,
        };
        let mut broker_client = BrokerClient::new(transport);

        //  Connect to broker immediately to ensure it's available
        let quiet_stdout = std::env::var("RZN_JSON")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if !quiet_stdout {
            println!(" CONNECTING TO BROKER...");
        }
        info!(" Connecting to broker...");
        broker_client.connect().await.map_err(|e| {
            if !quiet_stdout {
                println!("[ERROR] BROKER CONNECTION FAILED: {}", e);
            }
            error!("[ERROR] Failed to connect to broker: {}", e);
            PlanError::BrokerError(format!(
                "Broker connection failed: {}. Make sure the broker is running.",
                e
            ))
        })?;
        if !quiet_stdout {
            println!("[OK] BROKER CONNECTION SUCCESSFUL");
        }
        info!("[OK] Successfully connected to broker");

        Ok(Self {
            config,
            llm_client,
            workflow_manager,
            dom_analyzer,
            dom_processor,
            prompt_builder,
            self_healer,
            broker_client,
            // heuristic_planner: HeuristicPlanner::new(), // Removed - using LLM-only planning
            plan_sanitizer: PlanSanitizer::new(),
            policy_gate,
            current_dom_context: None,
            dom_revision: 0,
            telemetry_collector: None,
            // Initialize DOM snapshot system
            failure_cache: FailureCache::new(),
            use_snapshot_system: std::env::var("RZN_USE_SNAPSHOT")
                .map(|v| v.to_lowercase() == "true" || v == "1")
                .unwrap_or(true), // Default to enabled
            dom_hash_history: Vec::new(),
            max_dom_hash_history: 20, // Track last 20 DOM states for loop detection
            wait_strategy: SmartWaitStrategy::new(),

            // CDP-based architecture components
            mode_selector: ModeSelector::new(),
            current_domain: None,
            resolved_elements: HashMap::new(),
        })
    }

    /// Initialize telemetry collection for a session
    pub async fn initialize_telemetry(
        &mut self,
        goal: String,
        start_url: Option<String>,
    ) -> PlanResult<()> {
        // Check if telemetry is enabled via environment variable
        let telemetry_enabled = std::env::var("RZN_TELEMETRY_ENABLED")
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(true); // Default to enabled

        if !telemetry_enabled {
            info!("Telemetry disabled via RZN_TELEMETRY_ENABLED");
            return Ok(());
        }

        let mut telemetry_config = TelemetryConfig::default();

        // Check for custom traces directory
        if let Ok(traces_dir) = std::env::var("RZN_TRACES_DIR") {
            telemetry_config.traces_dir = std::path::PathBuf::from(traces_dir);
        }

        // Check for budget limit
        if let Ok(budget_str) = std::env::var("RZN_BUDGET_LIMIT") {
            if let Ok(budget) = budget_str.parse::<f64>() {
                telemetry_config.budget_limit = Some(budget);
                info!("Telemetry budget limit set to ${:.2}", budget);
            }
        }

        // Check for DOM snapshot inclusion
        if let Ok(include_dom) = std::env::var("RZN_INCLUDE_DOM_SNAPSHOTS") {
            telemetry_config.include_dom_snapshots =
                include_dom.to_lowercase() == "true" || include_dom == "1";
        }

        let collector = TelemetryCollector::new(goal, start_url, telemetry_config).await?;
        info!(
            "Telemetry initialized for session: {}",
            collector.session_id
        );

        self.telemetry_collector = Some(collector);
        Ok(())
    }

    /// Record LLM usage in telemetry
    async fn record_llm_usage(
        &mut self,
        model: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
        duration_ms: u64,
    ) -> PlanResult<()> {
        if let Some(ref mut collector) = self.telemetry_collector {
            let usage = LLMUsage {
                model: model.to_string(),
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
                provider_cost: None, // Could be filled if provider returns cost
                estimated_cost: 0.0, // Will be calculated in the collector
                duration_ms,
            };

            collector.record_llm_usage(usage).await?;
        }
        Ok(())
    }

    /// Record step execution in telemetry
    async fn record_step_execution(
        &mut self,
        step: Step,
        result: ExecutionResult,
        execution_time_ms: u64,
        retry_count: u32,
        dom_snapshot: Option<String>,
        screenshot_path: Option<String>,
        llm_usage: Option<LLMUsage>,
    ) -> PlanResult<()> {
        if let Some(ref mut collector) = self.telemetry_collector {
            collector
                .record_step(
                    step,
                    result,
                    execution_time_ms,
                    retry_count,
                    dom_snapshot,
                    screenshot_path,
                    llm_usage,
                )
                .await?;
        }
        Ok(())
    }

    /// Finalize telemetry session
    async fn finalize_telemetry(
        &mut self,
        final_result: Option<serde_json::Value>,
    ) -> PlanResult<()> {
        if let Some(mut collector) = self.telemetry_collector.take() {
            let summary = collector.finalize_session(final_result).await?;

            println!();
            println!(" SESSION TELEMETRY SUMMARY");
            println!("   Session ID: {}", summary.session_id);
            println!("   Total Steps: {}", summary.total_steps);
            println!(
                "   Success Rate: {:.1}%",
                if summary.total_steps > 0 {
                    summary.successful_steps as f64 / summary.total_steps as f64 * 100.0
                } else {
                    0.0
                }
            );
            println!("   Total Cost: ${:.4}", summary.total_cost);

            if !summary.cost_breakdown.is_empty() {
                println!("   Cost by Model:");
                for (model, cost) in &summary.cost_breakdown {
                    println!("     {}: ${:.4}", model, cost);
                }
            }

            if let Some(ref error) = summary.error_summary {
                println!("   Errors: {}", error);
            }

            println!(
                "   Trace files: ~/rzn_traces/{}/{}.jsonl",
                summary.start_time.format("%Y-%m"),
                summary.session_id
            );
        }
        Ok(())
    }

    /// Plan and execute a new workflow using LLM only (no workflow caching)
    pub async fn plan_llm_only(&mut self, request: PlanRequest) -> PlanResult<PlanResponse> {
        // Check if DOM snapshot system is enabled
        if self.use_snapshot_system {
            println!("🔀 DOM SNAPSHOT MODE ENABLED - Using snapshot-based planning");
            return self.plan_with_snapshots(request).await;
        }

        println!("[START] STARTING LLM-ONLY PLAN EXECUTION");
        println!("   Goal: {}", request.goal);
        println!("   Start URL: {:?}", request.start_url);
        println!("   Mode: LLM-only (no caching, no self-healing)");

        info!(
            "Starting LLM-only planning session for goal: {}",
            request.goal
        );

        // Initialize telemetry
        if let Err(e) = self
            .initialize_telemetry(request.goal.clone(), request.start_url.clone())
            .await
        {
            warn!("Failed to initialize telemetry: {}", e);
        }

        // Skip workflow caching - go directly to LLM planning
        let result = self.execute_llm_planning(request).await;

        // Finalize telemetry
        let final_data = match &result {
            Ok(response) => response.data.clone(),
            Err(_) => None,
        };

        if let Err(e) = self.finalize_telemetry(final_data).await {
            warn!("Failed to finalize telemetry: {}", e);
        }

        result
    }

    /// Plan and execute with full automation (workflow caching + self-healing)
    pub async fn plan_auto(&mut self, request: PlanRequest) -> PlanResult<PlanResponse> {
        println!("[START] STARTING AUTO PLAN EXECUTION");
        println!("   Goal: {}", request.goal);
        println!("   Start URL: {:?}", request.start_url);
        println!("   Mode: Full auto (caching + self-healing)");

        info!("Starting auto planning session for goal: {}", request.goal);

        // Check if we already have a workflow for this goal
        if let Some(existing_workflow) = self
            .workflow_manager
            .find_similar_workflow(&request.goal)
            .await?
        {
            println!("[SEARCH] FOUND EXISTING WORKFLOW");
            info!("Found existing workflow, attempting to run it first");

            let run_request = RunRequest {
                workflow: existing_workflow.clone(), // Use the actual workflow path/ID
                parameters: request.parameters.clone(),
                auto_heal: true,
            };

            match self.run(run_request).await {
                Ok(response) if response.success => {
                    println!("[OK] EXISTING WORKFLOW SUCCEEDED");
                    info!("Existing workflow succeeded");

                    // Load the workflow object for the response
                    let workflow_obj = self
                        .workflow_manager
                        .load_workflow(&existing_workflow)
                        .await
                        .ok();

                    return Ok(PlanResponse {
                        success: true,
                        workflow: workflow_obj,
                        data: response.data,
                        error: None,
                        steps_executed: response.steps_executed,
                        workflow_path: Some(existing_workflow),
                    });
                }
                Ok(_) => {
                    println!("[ERROR] EXISTING WORKFLOW FAILED - PROCEEDING WITH LLM PLANNING");
                    warn!("Existing workflow failed, proceeding with new planning");
                }
                Err(e) => {
                    println!("[ERROR] ERROR RUNNING EXISTING WORKFLOW: {} - PROCEEDING WITH LLM PLANNING", e);
                    warn!(
                        "Error running existing workflow: {}, proceeding with new planning",
                        e
                    );
                }
            }
        } else {
            println!("[SEARCH] NO EXISTING WORKFLOW FOUND - PROCEEDING WITH LLM PLANNING");
        }

        // Fall back to LLM planning
        self.execute_llm_planning(request).await
    }

    /// Legacy plan method - redirects to plan_auto for backward compatibility
    pub async fn plan(&mut self, request: PlanRequest) -> PlanResult<PlanResponse> {
        println!(
            "[WARNING]  Using legacy 'plan' method - consider using 'plan_auto' or 'plan_llm_only'"
        );
        self.plan_auto(request).await
    }

    /// Plan and execute using the new DOM snapshot-based three-tier system
    pub async fn plan_with_snapshots(&mut self, request: PlanRequest) -> PlanResult<PlanResponse> {
        println!("\n{}", "═".repeat(80));
        println!(
            "[START] {} {}",
            ansi_term::Colour::Yellow
                .bold()
                .paint("DOM SNAPSHOT PLANNING ACTIVATED"),
            ansi_term::Colour::Green.dimmed().paint("(v3.0)")
        );
        println!("{}", "═".repeat(80));
        println!(
            "\n[TARGET] {}: {}",
            ansi_term::Colour::Cyan.paint("Goal"),
            ansi_term::Colour::Yellow.bold().paint(&request.goal)
        );
        println!(
            " {}: {}",
            ansi_term::Colour::Cyan.paint("Start URL"),
            ansi_term::Colour::Blue.underline().paint(
                request
                    .start_url
                    .as_ref()
                    .unwrap_or(&"about:blank".to_string())
            )
        );
        println!(
            "\n {} Three-tier DOM snapshot system:",
            ansi_term::Colour::Cyan.paint("Mode:")
        );
        println!(
            "   1️⃣  {} - Strategic planning with pre-formatted DOM",
            ansi_term::Colour::Purple.paint("PLANNER")
        );
        println!(
            "   2️⃣  {} - Convert element references to CSS selectors",
            ansi_term::Colour::Purple.paint("NAVIGATOR")
        );
        println!(
            "   3️⃣  {} - Validate execution outcomes with DOM hash tracking",
            ansi_term::Colour::Purple.paint("VALIDATOR")
        );
        println!("{}", "═".repeat(80));
        println!();

        info!(
            "Starting DOM snapshot-based planning session for goal: {}",
            request.goal
        );

        // Initialize telemetry
        if let Err(e) = self
            .initialize_telemetry(request.goal.clone(), request.start_url.clone())
            .await
        {
            warn!("Failed to initialize telemetry: {}", e);
        }

        // Clear previous state
        self.failure_cache.cleanup_old_failures();

        // Start interactive planning session
        let mut session = PlanningSession {
            goal: request.goal.clone(),
            steps: Vec::new(),
            history: Vec::new(),
            current_dom: String::new(),
            current_url: request
                .start_url
                .clone()
                .unwrap_or_else(|| "about:blank".to_string()),
            parameters: request.parameters.clone(),
            failure_tracker: crate::failure_recovery::FailureTracker::new(),
            dom_change_detector: crate::failure_recovery::DomChangeDetector::new(),
        };

        // Navigate to starting URL if provided
        if let Some(start_url) = &request.start_url {
            println!(" NAVIGATING TO: {}", start_url);
            let navigate_step = Step {
                id: "start_navigate".to_string(),
                name: "Navigate to starting URL".to_string(),
                kind: StepKind::NavigateToUrl {
                    url: start_url.clone(),
                    wait: Some("domcontentloaded".to_string()),
                },
            };

            match self.execute_step(&navigate_step, &mut session).await {
                Ok((_, _)) => {
                    println!("[OK] INITIAL NAVIGATION COMPLETED");
                    session.steps.push(navigate_step);
                }
                Err(e) => {
                    error!("[ERROR] Failed to navigate to starting URL: {}", e);
                    return Ok(PlanResponse {
                        success: false,
                        workflow: None,
                        data: None,
                        error: Some(format!("Failed to navigate to starting URL: {}", e)),
                        steps_executed: 0,
                        workflow_path: None,
                    });
                }
            }
        }

        // Main DOM snapshot-based planning loop
        let mut final_data = None;

        for step_count in 0..self.config.max_steps {
            println!(
                " DOM SNAPSHOT PLANNING STEP {}/{}",
                step_count + 1,
                self.config.max_steps
            );

            // Step 1: Get DOM snapshot from broker
            let dom_snapshot = self.broker_client.get_current_dom_snapshot();

            if let Some(snapshot) = dom_snapshot {
                println!("[LIST] USING DOM SNAPSHOT...");
                info!(
                    "[OK] DOM snapshot available: {} elements, hash: {}",
                    snapshot.elements.len(),
                    snapshot.hash
                );

                // Track DOM hash for loop detection
                self.dom_hash_history.push(snapshot.hash.clone());
                if self.dom_hash_history.len() > self.max_dom_hash_history {
                    self.dom_hash_history.remove(0);
                }

                // Check for DOM loops (same hash appearing multiple times recently)
                let hash_count = self
                    .dom_hash_history
                    .iter()
                    .filter(|&h| h == &snapshot.hash)
                    .count();
                if hash_count > 2 {
                    warn!(
                        "[WARNING] DOM loop detected (hash {} seen {} times)",
                        snapshot.hash, hash_count
                    );
                }

                if snapshot.elements.is_empty() {
                    warn!("[WARNING] No elements found in DOM snapshot!");
                }
            } else {
                warn!("[WARNING] No DOM snapshot available from broker");
            }

            // Step 2: Tier 1 - Planner (strategic planning with pre-formatted DOM)
            println!("🧠 TIER 1: STRATEGIC PLANNING");

            let planner_prompt = if let Some(snapshot) = dom_snapshot {
                // Use the pre-formatted DOM prompt from the snapshot
                self.prompt_builder.build_snapshot_planner_prompt(
                    &session.goal,
                    &session.current_url,
                    snapshot,
                    Some(&self.failure_cache),
                    &session.history,
                )
            } else {
                // Fallback to empty prompt if no snapshot available
                warn!("No DOM snapshot available, using fallback prompt");
                vec![
                    json!({
                        "role": "system",
                        "content": "You are a web automation planner. No DOM data is currently available."
                    }),
                    json!({
                        "role": "user",
                        "content": format!("Goal: {}\nCurrent URL: {}\nNo DOM elements available. Please suggest a navigation action.", session.goal, session.current_url)
                    }),
                ]
            };

            // Pretty print the snapshot summary being sent
            println!("\n{}", "━".repeat(80));
            println!(
                " {} {}",
                ansi_term::Colour::Cyan
                    .bold()
                    .paint("DOM SNAPSHOT MESSAGE TO LLM - TIER 1"),
                ansi_term::Colour::Yellow
                    .dimmed()
                    .paint(format!("[{}]", chrono::Local::now().format("%H:%M:%S")))
            );
            println!("{}", "─".repeat(60));
            println!(
                "[TARGET] {}: {}",
                ansi_term::Colour::Cyan.paint("Goal"),
                ansi_term::Colour::Yellow.bold().paint(&session.goal)
            );
            println!(
                " {}: {}",
                ansi_term::Colour::Cyan.paint("Current URL"),
                ansi_term::Colour::Blue.paint(&session.current_url)
            );
            if let Some(snapshot) = dom_snapshot {
                println!(
                    "[LIST] {}: {} elements (hash: {})",
                    ansi_term::Colour::Green.paint("Elements"),
                    ansi_term::Colour::Green.paint(snapshot.elements.len().to_string()),
                    ansi_term::Colour::Yellow
                        .dimmed()
                        .paint(&snapshot.hash[..8])
                );
            } else {
                println!(
                    "[LIST] {}: {}",
                    ansi_term::Colour::Red.paint("Elements"),
                    ansi_term::Colour::Red.paint("No snapshot available")
                );
            }

            // Show sample DOM elements from snapshot
            if let Some(snapshot) = dom_snapshot {
                if !snapshot.elements.is_empty() {
                    println!(
                        "\n[TARGET] {} (first 5):",
                        ansi_term::Colour::Cyan.paint("Sample Elements")
                    );
                    for (i, element) in snapshot.elements.iter().take(5).enumerate() {
                        let display_text = element
                            .text
                            .as_ref()
                            .map(|t| t.chars().take(50).collect::<String>())
                            .unwrap_or_else(|| "".to_string());
                        println!(
                            "  [{}] {} - {} ({})",
                            ansi_term::Colour::Yellow.bold().paint(i.to_string()),
                            ansi_term::Colour::Purple.paint(element.tag.to_uppercase()),
                            ansi_term::Colour::White.paint(&display_text),
                            ansi_term::Colour::Red.paint(&element.selector)
                        );
                    }
                }
            }

            println!("{}", "━".repeat(80));

            // Log the actual prompt being sent to LLM
            debug!(
                "=== PLANNER PROMPT ===\n{}",
                serde_json::to_string_pretty(&planner_prompt).unwrap_or_default()
            );

            let planned_action = match timeout(
                Duration::from_secs(self.config.llm_timeout),
                self.llm_client.chat_json(planner_prompt, Some(0.7)),
            )
            .await
            {
                Ok(Ok(json_response)) => {
                    // Log the raw LLM response
                    debug!(
                        "=== PLANNER RESPONSE ===\n{}",
                        serde_json::to_string_pretty(&json_response).unwrap_or_default()
                    );

                    // Pretty print the response
                    println!("\n{}", "━".repeat(80));
                    println!(
                        "[BOT] {} {}",
                        ansi_term::Colour::Green
                            .bold()
                            .paint("LLM RESPONSE - TIER 1 (Planner)"),
                        ansi_term::Colour::Yellow
                            .dimmed()
                            .paint(format!("[{}]", chrono::Local::now().format("%H:%M:%S")))
                    );
                    println!("{}", "─".repeat(60));

                    // Debug: Show parsed JSON response
                    println!(
                        "[SEARCH] {}: Valid JSON received",
                        ansi_term::Colour::Cyan.paint("Response Format")
                    );

                    // The response is already parsed as JSON
                    let action = json_response;

                    // Pretty print the action details
                    if let Some(action_str) = action.get("action").and_then(|a| a.as_str()) {
                        println!(
                            "[OK] {}: {}",
                            ansi_term::Colour::Cyan.paint("Action"),
                            ansi_term::Colour::Yellow.bold().paint(action_str)
                        );
                    }
                    if let Some(reasoning) = action.get("reasoning").and_then(|r| r.as_str()) {
                        println!(
                            "💭 {}: {}",
                            ansi_term::Colour::Cyan.paint("Reasoning"),
                            reasoning
                        );
                    }
                    if let Some(params) = action.get("parameters").and_then(|p| p.as_object()) {
                        if let Some(index) = params.get("index").and_then(|i| i.as_u64()) {
                            println!(
                                "[TARGET] {}: [{}]",
                                ansi_term::Colour::Cyan.paint("Target Index"),
                                ansi_term::Colour::Red.bold().paint(index.to_string())
                            );
                        }
                        if let Some(url) = params.get("url").and_then(|u| u.as_str()) {
                            println!(
                                " {}: {}",
                                ansi_term::Colour::Cyan.paint("URL"),
                                ansi_term::Colour::Blue.underline().paint(url)
                            );
                        }
                    }
                    if let Some(confidence) = action.get("confidence").and_then(|c| c.as_f64()) {
                        println!(
                            " {}: {:.0}%",
                            ansi_term::Colour::Cyan.paint("Confidence"),
                            confidence * 100.0
                        );
                    }
                    println!("{}", "━".repeat(80));

                    action
                }
                Ok(Err(e)) => {
                    error!("Planner LLM request failed: {}", e);
                    return Err(e);
                }
                Err(_) => {
                    error!("Planner LLM request timed out");
                    return Err(PlanError::LLMError("Planner timed out".to_string()));
                }
            };

            // Check if planner says we're complete
            if planned_action.get("status").and_then(|s| s.as_str()) == Some("complete") {
                info!("🏁 Planner indicates task is complete");
                if let Some(data) = planned_action.get("extracted_data") {
                    final_data = Some(data.clone());
                }
                break;
            }

            // Step 3: Tier 2 - Navigator (convert indexes to selectors)
            println!("🧭 TIER 2: SELECTOR NAVIGATION");
            let navigator_prompt = if let Some(snapshot) = dom_snapshot {
                self.prompt_builder.build_snapshot_navigator_prompt(
                    &planned_action,
                    snapshot,
                    Some(&self.failure_cache),
                    &session.current_url,
                )
            } else {
                warn!("No DOM snapshot for navigator, using fallback");
                vec![
                    json!({
                        "role": "system",
                        "content": "You are a web automation navigator. No DOM snapshot available."
                    }),
                    json!({
                        "role": "user",
                        "content": format!("Action to execute: {}", serde_json::to_string_pretty(&planned_action).unwrap_or_default())
                    }),
                ]
            };

            // Pretty print navigator request
            println!("\n{}", "━".repeat(80));
            println!(
                " {} {}",
                ansi_term::Colour::Cyan
                    .bold()
                    .paint("NAVIGATOR MESSAGE TO LLM - TIER 2"),
                ansi_term::Colour::Yellow
                    .dimmed()
                    .paint(format!("[{}]", chrono::Local::now().format("%H:%M:%S")))
            );
            println!("{}", "─".repeat(60));
            println!(
                "[TARGET] Converting element reference to selector for action family: {}",
                planned_action
                    .get("action")
                    .and_then(|a| a.as_str())
                    .unwrap_or("unknown")
            );
            if let Some(params) = planned_action.get("parameters").and_then(|p| p.as_object()) {
                if let Some(index) = params.get("index").and_then(|i| i.as_u64()) {
                    if let Some(snapshot) = dom_snapshot {
                        if let Some(element) = snapshot.elements.get(index as usize) {
                            let display_text = element
                                .text
                                .as_ref()
                                .map(|t| t.chars().take(30).collect::<String>())
                                .unwrap_or_else(|| "".to_string());
                            println!(
                                " Element [{}]: {} - {}",
                                ansi_term::Colour::Yellow.bold().paint(index.to_string()),
                                ansi_term::Colour::Purple.paint(&element.tag),
                                ansi_term::Colour::White.paint(&display_text)
                            );
                        }
                    }
                }
            }
            println!("{}", "━".repeat(80));

            // Log the actual prompt being sent to Navigator
            debug!(
                "=== NAVIGATOR PROMPT ===\n{}",
                serde_json::to_string_pretty(&navigator_prompt).unwrap_or_default()
            );

            let validated_action = match timeout(
                Duration::from_secs(self.config.llm_timeout),
                self.llm_client.chat_json(navigator_prompt, Some(0.3)),
            )
            .await
            {
                Ok(Ok(json_response)) => {
                    // Log the raw Navigator response
                    debug!(
                        "=== NAVIGATOR RESPONSE ===\n{}",
                        serde_json::to_string_pretty(&json_response).unwrap_or_default()
                    );

                    // Pretty print the response
                    println!("\n{}", "━".repeat(80));
                    println!(
                        "[BOT] {} {}",
                        ansi_term::Colour::Green
                            .bold()
                            .paint("LLM RESPONSE - TIER 2 (Navigator)"),
                        ansi_term::Colour::Yellow
                            .dimmed()
                            .paint(format!("[{}]", chrono::Local::now().format("%H:%M:%S")))
                    );
                    println!("{}", "─".repeat(60));

                    // Already parsed as JSON
                    let validation = json_response;
                    if validation.get("status").and_then(|s| s.as_str()) == Some("validated") {
                        // Pretty print validated action
                        if let Some(selector) = validation.get("selector").and_then(|s| s.as_str())
                        {
                            println!(
                                "[OK] {}: {}",
                                ansi_term::Colour::Cyan.paint("Selector"),
                                ansi_term::Colour::Red.bold().paint(selector)
                            );
                        }
                        if let Some(encoded_id) =
                            validation.get("encoded_id").and_then(|s| s.as_str())
                        {
                            println!(
                                "[OK] {}: {}",
                                ansi_term::Colour::Cyan.paint("EncodedId"),
                                ansi_term::Colour::Yellow.bold().paint(encoded_id)
                            );
                        }
                        if let Some(action_type) =
                            validation.get("action_type").and_then(|a| a.as_str())
                        {
                            println!(
                                "[TARGET] {}: {}",
                                ansi_term::Colour::Cyan.paint("Action Type"),
                                ansi_term::Colour::Purple.paint(action_type)
                            );
                        }
                        if let Some(frame_ordinal) =
                            validation.get("frame_ordinal").and_then(|v| v.as_u64())
                        {
                            println!(
                                "[OK] {}: {}",
                                ansi_term::Colour::Cyan.paint("Frame"),
                                ansi_term::Colour::Yellow.paint(frame_ordinal.to_string())
                            );
                        }
                        println!("{}", "━".repeat(80));
                        validation
                    } else {
                        warn!("Navigator validation failed: {:?}", validation);
                        // Record failure and continue with next iteration
                        if let Some(selector) = validation.get("attempted_selectors") {
                            // Record failure in cache
                            let context = FailureContext {
                                url: session.current_url.clone(),
                                page_title: None,
                                action_type: planned_action
                                    .get("action")
                                    .and_then(|a| a.as_str())
                                    .unwrap_or("unknown")
                                    .to_string(),
                                element_index: planned_action
                                    .get("parameters")
                                    .and_then(|p| p.get("index"))
                                    .and_then(|i| i.as_u64())
                                    .map(|i| i as u32),
                                dom_context: Some(session.current_dom.clone()),
                                goal: Some(session.goal.clone()),
                            };

                            if let Some(selector_str) = selector.as_str() {
                                self.failure_cache.record_failure(
                                    selector_str,
                                    "Navigator validation failed",
                                    context,
                                );
                            }
                        }
                        continue; // Try next planning iteration
                    }
                }
                Ok(Err(e)) => {
                    warn!("Navigator LLM request failed: {}", e);
                    continue; // Try next planning iteration
                }
                Err(_) => {
                    warn!("Navigator LLM request timed out");
                    continue; // Try next planning iteration
                }
            };

            // Step 4: Execute the validated action
            println!("[ACTION] EXECUTING VALIDATED ACTION");

            // Convert validated action back to Step format
            let step =
                self.convert_validated_action_to_step(&validated_action, step_count as usize)?;

            let before_state = session.current_dom.clone();

            match self.execute_step(&step, &mut session).await {
                Ok((execution_result, _raw_dom)) => {
                    println!("[OK] ACTION EXECUTED SUCCESSFULLY");
                    session.steps.push(step.clone());

                    // Raw DOM is no longer needed as we use DOM snapshots from the broker

                    // Step 5: Tier 3 - Validator (assess outcome)
                    println!("[SEARCH] TIER 3: OUTCOME VALIDATION");

                    let step_execution = StepExecution {
                        step: step.clone(),
                        result: execution_result.clone(),
                        timestamp: chrono::Utc::now(),
                        dom_snapshot: Some(session.current_dom.clone()),
                    };

                    let validator_prompt = self.prompt_builder.build_validator_prompt(
                        &step_execution,
                        &before_state,
                        &session.current_dom,
                        &session.goal,
                        &session.history,
                    );

                    // Pretty print validator request
                    println!("\n{}", "━".repeat(80));
                    println!(
                        " {} {}",
                        ansi_term::Colour::Cyan
                            .bold()
                            .paint("VALIDATOR MESSAGE TO LLM - TIER 3"),
                        ansi_term::Colour::Yellow
                            .dimmed()
                            .paint(format!("[{}]", chrono::Local::now().format("%H:%M:%S")))
                    );
                    println!("{}", "─".repeat(60));
                    println!(
                        "[TARGET] {}: {}",
                        ansi_term::Colour::Cyan.paint("Executed Action"),
                        ansi_term::Colour::Purple.paint(&step.name)
                    );
                    println!(
                        " {}: DOM changed = {}",
                        ansi_term::Colour::Cyan.paint("Result"),
                        if before_state != session.current_dom {
                            ansi_term::Colour::Green.paint("YES")
                        } else {
                            ansi_term::Colour::Red.paint("NO")
                        }
                    );
                    println!("{}", "━".repeat(80));

                    match timeout(
                        Duration::from_secs(self.config.llm_timeout),
                        self.llm_client.chat_json(validator_prompt, Some(0.3)),
                    )
                    .await
                    {
                        Ok(Ok(json_response)) => {
                            // Pretty print the response
                            println!("\n{}", "━".repeat(80));
                            println!(
                                "[BOT] {} {}",
                                ansi_term::Colour::Green
                                    .bold()
                                    .paint("LLM RESPONSE - TIER 3 (Validator)"),
                                ansi_term::Colour::Yellow.dimmed().paint(format!(
                                    "[{}]",
                                    chrono::Local::now().format("%H:%M:%S")
                                ))
                            );
                            println!("{}", "─".repeat(60));

                            // Already parsed as JSON
                            let validation = json_response;
                            let status = validation
                                .get("status")
                                .and_then(|s| s.as_str())
                                .unwrap_or("unknown");
                            let status_color = match status {
                                "complete" => ansi_term::Colour::Green,
                                "continue" => ansi_term::Colour::Yellow,
                                "failed" => ansi_term::Colour::Red,
                                _ => ansi_term::Colour::White,
                            };

                            println!(
                                "🏁 {}: {}",
                                ansi_term::Colour::Cyan.paint("Status"),
                                status_color.bold().paint(status.to_uppercase())
                            );

                            if let Some(feedback) =
                                validation.get("feedback").and_then(|f| f.as_str())
                            {
                                println!(
                                    "💬 {}: {}",
                                    ansi_term::Colour::Cyan.paint("Feedback"),
                                    feedback
                                );
                            }

                            if let Some(suggestions) =
                                validation.get("suggestions").and_then(|s| s.as_array())
                            {
                                if !suggestions.is_empty() {
                                    println!(
                                        "[TIP] {}:",
                                        ansi_term::Colour::Cyan.paint("Suggestions")
                                    );
                                    for suggestion in suggestions.iter().filter_map(|s| s.as_str())
                                    {
                                        println!("   - {}", suggestion);
                                    }
                                }
                            }

                            println!("{}", "━".repeat(80));

                            if validation.get("status").and_then(|s| s.as_str()) == Some("complete")
                            {
                                info!("🏁 Validator indicates goal is complete");
                                if let Some(data) = validation.get("extracted_data") {
                                    final_data = Some(data.clone());
                                }
                                break;
                            }

                            // Store extracted data if present
                            if let ExecutionResult::Success {
                                payload: Some(data),
                            } = &execution_result
                            {
                                final_data = Some(data.clone());
                            }
                        }
                        Ok(Err(e)) => {
                            warn!("Validator LLM request failed: {}", e);
                        }
                        Err(_) => {
                            warn!("Validator LLM request timed out");
                        }
                    }
                }
                Err(e) => {
                    error!("[ERROR] Action execution failed: {}", e);

                    // Record failure in cache
                    if let Some(selector) =
                        validated_action.get("selector").and_then(|s| s.as_str())
                    {
                        let context = FailureContext {
                            url: session.current_url.clone(),
                            page_title: None,
                            action_type: planned_action
                                .get("action")
                                .and_then(|a| a.as_str())
                                .unwrap_or("unknown")
                                .to_string(),
                            element_index: None,
                            dom_context: Some(session.current_dom.clone()),
                            goal: Some(session.goal.clone()),
                        };

                        self.failure_cache
                            .record_failure(selector, &e.to_string(), context);
                    }

                    // Continue to next iteration to try alternative approach
                    continue;
                }
            }
        }

        // Finalize telemetry
        if let Err(e) = self.finalize_telemetry(final_data.clone()).await {
            warn!("Failed to finalize telemetry: {}", e);
        }

        // Create workflow from successful steps
        let workflow = if !session.steps.is_empty() {
            let workflow = self.create_workflow_from_session(&session, &request)?;

            // Save workflow if requested
            let workflow_path = if request.save_workflow {
                Some(self.workflow_manager.save_workflow(&workflow).await?)
            } else {
                None
            };

            Some((workflow, workflow_path))
        } else {
            None
        };

        Ok(PlanResponse {
            success: !session.steps.is_empty(),
            workflow: workflow.as_ref().map(|(w, _)| w.clone()),
            data: final_data,
            error: if session.steps.is_empty() {
                Some("No successful steps were executed".to_string())
            } else {
                None
            },
            steps_executed: session.steps.len() as u32,
            workflow_path: workflow.and_then(|(_, path)| path),
        })
    }

    /// Execute LLM planning (shared by both plan_llm_only and plan_auto)
    async fn execute_llm_planning(&mut self, request: PlanRequest) -> PlanResult<PlanResponse> {
        println!("[START] STARTING PLAN EXECUTION");
        println!("   Goal: {}", request.goal);
        println!("   Start URL: {:?}", request.start_url);

        info!("Starting planning session for goal: {}", request.goal);

        // Start interactive planning session
        let mut session = PlanningSession {
            goal: request.goal.clone(),
            steps: Vec::new(),
            history: Vec::new(),
            current_dom: String::new(),
            current_url: request
                .start_url
                .clone()
                .unwrap_or_else(|| "about:blank".to_string()),
            parameters: request.parameters.clone(),
            failure_tracker: crate::failure_recovery::FailureTracker::new(),
            dom_change_detector: crate::failure_recovery::DomChangeDetector::new(),
        };

        // Navigate to starting URL if provided
        if let Some(start_url) = &request.start_url {
            println!(" ATTEMPTING INITIAL NAVIGATION TO: {}", start_url);
            info!(" Navigating to starting URL: {}", start_url);
            let navigate_step = Step {
                id: "start_navigate".to_string(),
                name: "Navigate to starting URL".to_string(),
                kind: StepKind::NavigateToUrl {
                    url: start_url.clone(),
                    wait: Some("domcontentloaded".to_string()),
                },
            };

            println!(" EXECUTING INITIAL NAVIGATION STEP");
            info!(" Executing initial navigation step");
            match self.execute_step(&navigate_step, &mut session).await {
                Ok((_, _)) => {
                    println!("[OK] INITIAL NAVIGATION COMPLETED");
                    info!("[OK] Initial navigation completed successfully");
                    session.steps.push(navigate_step);

                    // Add cricket widget hint for sports-related goals
                    if session.goal.to_lowercase().contains("cricket")
                        || session.goal.to_lowercase().contains("score")
                        || session.goal.to_lowercase().contains("ipl")
                    {
                        let hint_execution = StepExecution {
                            step: Step {
                                id: "hint_cricket_widget".to_string(),
                                name: "Hint – Cricket score container".to_string(),
                                kind: StepKind::WaitForTimeout { timeout_ms: 1 }, // Placeholder step type
                            },
                            result: ExecutionResult::Success { payload: None },
                            timestamp: chrono::Utc::now(),
                            dom_snapshot: None,
                        };
                        session.history.push(hint_execution);
                        info!("[NOTE] Added cricket widget hint to planning history");
                    }

                    // [SEARCH] DEBUG: Show session state after initial navigation
                    println!("[SEARCH] SESSION STATE AFTER INITIAL NAVIGATION:");
                    println!("   ├─ URL: {}", session.current_url);
                    println!("   ├─ DOM Size: {} characters", session.current_dom.len());
                    println!("   └─ Steps: {}", session.steps.len());
                    if !session.current_dom.is_empty() {
                        let dom_preview = if session.current_dom.len() > 300 {
                            format!("{}...", &session.current_dom[..300])
                        } else {
                            session.current_dom.clone()
                        };
                        println!("   └─ DOM Preview: {}", dom_preview);
                    } else {
                        println!("   └─ [WARNING]  DOM IS EMPTY! This will confuse the LLM.");
                    }
                }
                Err(e) => {
                    println!("[ERROR] INITIAL NAVIGATION FAILED: {}", e);
                    error!("[ERROR] Failed to navigate to starting URL: {}", e);
                    return Ok(PlanResponse {
                        success: false,
                        workflow: None,
                        data: None,
                        error: Some(format!("Failed to navigate to starting URL: {}", e)),
                        steps_executed: 0,
                        workflow_path: None,
                    });
                }
            }
        } else {
            println!("ℹ️  NO STARTING URL PROVIDED");
            info!("ℹ️  No starting URL provided, starting from current page");
        }

        // Interactive planning loop
        let mut final_data = None;
        for step_count in 0..self.config.max_steps {
            println!(
                " PLANNING STEP {}/{}",
                step_count + 1,
                self.config.max_steps
            );
            debug!("Planning step {}/{}", step_count + 1, self.config.max_steps);

            // 🚫 HEURISTIC DISABLED - Force LLM-only planning per user request
            // The user wants pure LLM planning without heuristic shortcuts
            println!("🧠 USING LLM-ONLY PLANNING (heuristics disabled)");

            // Build prompt with RAW DOM for LLM to analyze selectors itself
            // This is the key change - sending raw DOM instead of processed DomContext

            let prompt = self.prompt_builder.build_planning_prompt(
                &session.goal,
                &session.current_dom, // RAW DOM string for LLM to analyze
                &session.current_url,
                &session.history,
            );

            // [SEARCH] DEBUG: Log current state being sent to LLM
            info!(" Current State for LLM:");
            if let Some(ref context) = self.current_dom_context {
                info!("   ├─ URL: {}", context.url);
                info!("   ├─ Page Type: {}", context.page_type);
                info!(
                    "   ├─ Interactive Elements: {}",
                    context.interactive_elements.len()
                );
                info!(
                    "   ├─ Semantic Groups: {:?}",
                    context.semantic_groups.keys().collect::<Vec<_>>()
                );
                info!("   ├─ History Steps: {}", session.history.len());
                info!(
                    "   └─ DOM Context: {} total elements, {} processed",
                    context.total_elements, context.processed_elements
                );
            } else {
                info!("   ├─ URL: {}", session.current_url);
                info!("   ├─ DOM Size: {} characters", session.current_dom.len());
                info!("   └─ History Steps: {}", session.history.len());
            }

            // Get next step from LLM using two-tier planning with basic retry
            println!("🧠 USING TWO-TIER LLM PLANNING");
            let llm_response = {
                let mut attempts = 0;
                let max_attempts = 1; // Reduced from 3 to 1 to avoid spam
                loop {
                    attempts += 1;
                    match timeout(
                        Duration::from_secs(self.config.llm_timeout),
                        self.llm_client.plan_step(prompt.clone()),
                    )
                    .await
                    {
                        Ok(Ok(response)) => break response,
                        Ok(Err(e)) => {
                            // For API key errors or other auth issues, fail immediately
                            if e.to_string().contains("API key")
                                || e.to_string().contains("invalid_request_error")
                            {
                                return Err(e);
                            }

                            if attempts >= max_attempts {
                                return Err(PlanError::LLMError(format!(
                                    "LLM request failed after {} attempts: {}",
                                    max_attempts, e
                                )));
                            }
                            warn!(
                                "LLM attempt {}/{} failed, retrying in 1 second...",
                                attempts, max_attempts
                            );
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                        Err(_timeout) => {
                            if attempts >= max_attempts {
                                return Err(PlanError::LLMError(format!(
                                    "LLM request timed out after {} attempts",
                                    max_attempts
                                )));
                            }
                            warn!(
                                "LLM attempt {}/{} timed out, retrying in 1 second...",
                                attempts, max_attempts
                            );
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            };

            // [SEARCH] DEBUG: Log the complete LLM response
            println!("[BOT] LLM RESPONSE:");
            println!("   ├─ Is Complete: {}", llm_response.is_complete);
            if let Some(ref reasoning) = llm_response.reasoning {
                println!("   ├─ Reasoning: {}", reasoning);
            }
            if let Some(ref step) = llm_response.next_step {
                println!("   ├─ Next Step: {} - {}", step.id, step.name);
                println!("   └─ Step Kind: {:?}", step.kind);
            } else {
                println!("   ├─ Next Step: None");
            }
            if let Some(ref data) = llm_response.extracted_data {
                println!(
                    "   └─ Extracted Data: {}",
                    serde_json::to_string_pretty(data)
                        .unwrap_or_else(|_| "Failed to serialize".to_string())
                );
            } else {
                println!("   └─ Extracted Data: None");
            }

            // Log LLM response concisely
            if llm_response.is_complete {
                println!("🏁 LLM indicates task is complete");
            } else if let Some(ref step) = llm_response.next_step {
                println!("[TARGET] LLM suggests: {}", step.name);
            } else {
                println!("[WARNING] LLM provided no next step");
            }

            // Check if LLM says we're done
            if llm_response.is_complete {
                info!("🏁 LLM indicates planning is complete");
                info!("   ├─ Steps executed so far: {}", session.steps.len());
                info!("   ├─ Current URL: {}", session.current_url);
                info!("   └─ DOM size: {} characters", session.current_dom.len());

                // If we have extracted data from LLM, use it
                if let Some(data) = llm_response.extracted_data {
                    info!(" Using extracted data from LLM response");
                    final_data = Some(data);
                } else {
                    info!("[WARNING]  No extracted data from LLM - LLM should handle extraction through steps");
                    // The LLM should handle all extraction through its planning steps
                    // No hardcoded extraction logic should be used
                }

                // Add cleanup step to close the tab
                info!("🧹 Adding cleanup step to close browser tab");
                let cleanup_step = Step {
                    id: "cleanup_close_tab".to_string(),
                    name: "Close browser tab".to_string(),
                    kind: StepKind::CloseCurrentTab {
                        tab_identifier: serde_json::Value::Null,
                    },
                };

                match self.execute_step(&cleanup_step, &mut session).await {
                    Ok((_, _)) => {
                        session.steps.push(cleanup_step);
                        info!("[OK] Cleanup completed - browser tab closed");
                    }
                    Err(e) => {
                        warn!("[WARNING]  Cleanup step failed: {}", e);
                        // Don't fail the entire workflow for cleanup issues
                    }
                }

                break;
            }

            // Execute the suggested step
            if let Some(step) = llm_response.next_step {
                info!(
                    "[TARGET] LLM provided a step to execute: {} - {}",
                    step.id, step.name
                );
                info!("   └─ Step details: {:?}", step.kind);

                // 🚫 CRITICAL FIX: Sanitize step before execution to block IFRAME steps
                let sanitized_step = match self.plan_sanitizer.sanitize_step(&step)? {
                    Some(sanitized) => {
                        if sanitized.id != step.id || sanitized.name != step.name {
                            info!(
                                " Plan sanitizer modified step: {} -> {}",
                                step.name, sanitized.name
                            );
                        }
                        sanitized
                    }
                    None => {
                        warn!("🚫 Plan sanitizer dropped problematic step: {} - continuing to next iteration", step.name);
                        continue; // Skip this step and ask LLM for next one
                    }
                };

                match self.execute_step(&sanitized_step, &mut session).await {
                    Ok((execution_result, _)) => {
                        info!("[OK] Step execution succeeded");
                        session.steps.push(sanitized_step.clone());

                        // Store extracted data if this was an extraction step
                        if let ExecutionResult::Success {
                            payload: Some(data),
                        } = &execution_result
                        {
                            info!(" Step returned extracted data");
                            info!(
                                "[SEARCH] DEBUG: Payload type: {}",
                                if data.is_object() {
                                    "object"
                                } else if data.is_array() {
                                    "array"
                                } else {
                                    "other"
                                }
                            );
                            if data.is_object() {
                                info!(
                                    "[SEARCH] DEBUG: Payload keys: {:?}",
                                    data.as_object().map(|o| o.keys().collect::<Vec<_>>())
                                );
                            }

                            final_data = Some(data.clone());

                            // [TARGET] AUTO-EXTRACTION DETECTION: Check if this data came from auto-extraction
                            if let Some(extraction_data) = data.get("extraction_data") {
                                info!("[SEARCH] DEBUG: Found extraction_data in payload");
                                let auto_extracted = data
                                    .get("auto_extracted")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                let goal_completed = data
                                    .get("goal_completed")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                info!(
                                    "[SEARCH] DEBUG: auto_extracted={}, goal_completed={}",
                                    auto_extracted, goal_completed
                                );

                                if auto_extracted && goal_completed {
                                    info!("[TARGET] AUTO-EXTRACTION COMPLETED! Workflow goal achieved.");
                                    info!(
                                        " Final extracted data: {}",
                                        serde_json::to_string_pretty(extraction_data)
                                            .unwrap_or_else(|_| "Failed to serialize".to_string())
                                    );

                                    // Set final_data to the extraction_data for display
                                    final_data = Some(extraction_data.clone());

                                    // Mark planning as complete and break out of loop
                                    break;
                                }
                            } else {
                                info!("[SEARCH] DEBUG: No extraction_data found in payload");
                            }
                        }

                        // [SEARCH] DEBUG: Special handling for extraction steps
                        if let StepKind::ExtractStructuredData { .. } = &step.kind {
                            match &execution_result {
                                ExecutionResult::Success {
                                    payload: Some(data),
                                } => {
                                    if let Some(array) = data.as_array() {
                                        println!(
                                            "[SEARCH] EXTRACTION RESULT: Found {} items",
                                            array.len()
                                        );
                                        if array.is_empty() {
                                            println!("[WARNING]  EXTRACTION RETURNED 0 ITEMS - LLM will likely try a different strategy");
                                        } else {
                                            println!(
                                                "[OK] EXTRACTION SUCCESSFUL - {} items found",
                                                array.len()
                                            );
                                            // Show first item as sample
                                            if let Some(first_item) = array.first() {
                                                println!("[LIST] SAMPLE EXTRACTED ITEM:");
                                                println!(
                                                    "{}",
                                                    serde_json::to_string_pretty(first_item)
                                                        .unwrap_or_else(
                                                            |_| "Failed to serialize".to_string()
                                                        )
                                                );
                                            }
                                        }
                                    } else {
                                        println!(
                                            "[SEARCH] EXTRACTION RESULT: Non-array data returned"
                                        );
                                        println!(
                                            "{}",
                                            serde_json::to_string_pretty(data).unwrap_or_else(
                                                |_| "Failed to serialize".to_string()
                                            )
                                        );
                                    }
                                }
                                ExecutionResult::Success { payload: None } => {
                                    println!("[SEARCH] EXTRACTION RESULT: No payload returned");
                                }
                                ExecutionResult::Error { message, .. } => {
                                    println!("[SEARCH] EXTRACTION RESULT: Error - {}", message);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("[ERROR] Step execution failed: {}", e);

                        // Record failure and get recovery action
                        let recovery_action = session
                            .failure_tracker
                            .record_failure(&e.to_string(), Some(&session.current_dom));

                        // Check for DOM changes/frame swaps
                        if let Some(frame_swap_category) = session
                            .dom_change_detector
                            .detect_frame_swap(&session.current_dom, &session.current_url)
                        {
                            warn!(" Detected frame swap: DOM structure changed significantly");
                            session.failure_tracker.last_error_category = Some(frame_swap_category);
                        }

                        // Add failed execution to history with recovery context
                        let failed_execution = StepExecution {
                            step: step.clone(),
                            result: ExecutionResult::Error {
                                message: format!("{}\n{}", recovery_action.llm_prefix, e),
                                retry_suggested: true,
                            },
                            timestamp: chrono::Utc::now(),
                            dom_snapshot: Some(session.current_dom.clone()),
                        };
                        session.history.push(failed_execution);
                        info!("[NOTE] Added failed step to history with recovery context");

                        // Check if we should abort
                        if session.failure_tracker.should_abort() {
                            error!("🛑 Too many consecutive failures - aborting workflow");
                            return Ok(PlanResponse {
                                success: false,
                                workflow: None,
                                data: final_data,
                                error: Some(format!(
                                    "Workflow aborted after {} consecutive failures. {}",
                                    session.failure_tracker.consecutive_failures,
                                    session.failure_tracker.get_error_summary()
                                )),
                                steps_executed: session.steps.len() as u32,
                                workflow_path: None,
                            });
                        }

                        // Apply recovery strategy based on failure count
                        match recovery_action.strategy {
                            crate::failure_recovery::RecoveryStrategy::EscalateToNative => {
                                info!(" Escalating to native input methods");
                                // The LLM will see the prefix and adjust its approach
                            }
                            crate::failure_recovery::RecoveryStrategy::RefreshFullDom => {
                                info!(" Refreshing DOM analysis");
                                // Force a full DOM refresh
                                self.current_dom_context = None;
                                self.dom_revision = 0;
                            }
                            crate::failure_recovery::RecoveryStrategy::SuggestHandleFamily => {
                                info!(" Suggesting Handle family for popups/overlays");
                                // The LLM will see the prefix suggesting Handle actions
                            }
                            crate::failure_recovery::RecoveryStrategy::BroaderSelector => {
                                // Try self-healing for selector issues
                                if let Ok(healed) =
                                    self.self_healer.heal_step(&step, &e.to_string()).await
                                {
                                    info!(" Self-Healer produced alternative selector – retrying");
                                    if let Ok((execution_result, _)) =
                                        self.execute_step(&healed, &mut session).await
                                    {
                                        info!("[OK] Self-healing succeeded!");
                                        session.steps.push(healed.clone());
                                        session.failure_tracker.record_success();

                                        // Process execution result like normal success
                                        if let ExecutionResult::Success { payload } =
                                            execution_result
                                        {
                                            if payload.is_some() {
                                                final_data = payload;
                                            }
                                        }
                                        continue; // skip LLM this turn
                                    }
                                }
                            }
                            crate::failure_recovery::RecoveryStrategy::RequestUserIntervention => {
                                warn!("[WARNING] Requesting user intervention - workflow paused");
                                // In future, could implement actual user prompting
                            }
                            _ => {
                                // Other strategies handled by LLM seeing the prefix
                            }
                        }

                        warn!(" Letting LLM decide next action with recovery context");
                        // Continue to let LLM handle the error in next iteration
                    }
                }
            } else {
                error!("[ERROR] LLM didn't provide a next step and didn't mark as complete!");
                warn!(" This might indicate an LLM parsing issue, continuing...");
            }
        }

        // Display extracted results if any
        if let Some(ref data) = final_data {
            info!("[TARGET] Extracted Results:");
            info!(
                "{}",
                serde_json::to_string_pretty(data)
                    .unwrap_or_else(|_| "Failed to format results".to_string())
            );
        }

        // Create workflow from successful steps
        let workflow = if !session.steps.is_empty() {
            let workflow = self.create_workflow_from_session(&session, &request)?;

            // Save workflow if requested
            let workflow_path = if request.save_workflow {
                Some(self.workflow_manager.save_workflow(&workflow).await?)
            } else {
                None
            };

            Some((workflow, workflow_path))
        } else {
            None
        };

        Ok(PlanResponse {
            success: !session.steps.is_empty(),
            workflow: workflow.as_ref().map(|(w, _)| w.clone()),
            data: final_data,
            error: if session.steps.is_empty() {
                Some("No successful steps were executed".to_string())
            } else {
                None
            },
            steps_executed: session.steps.len() as u32,
            workflow_path: workflow.and_then(|(_, path)| path),
        })
    }

    /// Run an existing workflow
    pub async fn run(&mut self, request: RunRequest) -> PlanResult<RunResponse> {
        info!("Running workflow: {}", request.workflow);

        // Load workflow
        let mut workflow = self
            .workflow_manager
            .load_workflow(&request.workflow)
            .await?;

        // Normalize + validate workflow parameters before executing any steps.
        // This prevents silent runs where placeholders remain in the executed steps.
        let required_list = Self::required_parameter_names(&workflow);
        let required_set: HashSet<String> = required_list.iter().cloned().collect();
        let mut parameters = request.parameters.clone();
        Self::apply_common_parameter_aliases(&required_set, &mut parameters);

        // Common optional defaults (keep lightweight and generic).
        // Used by workflows like Google Maps Directions that accept an optional `mode`.
        parameters
            .entry("mode".to_string())
            .or_insert_with(|| "driving".to_string());
        // Google Maps expects "bicycling" rather than "cycling" (accept both for parity).
        if let Some(mode) = parameters.get_mut("mode") {
            if mode.trim().eq_ignore_ascii_case("cycling") {
                *mode = "bicycling".to_string();
            }
        }

        let mut missing: Vec<String> = Vec::new();
        for req in &required_list {
            if !parameters.contains_key(req) {
                missing.push(req.clone());
            }
        }
        if !missing.is_empty() {
            let mut provided: Vec<String> = parameters.keys().cloned().collect();
            provided.sort();
            return Ok(RunResponse {
                success: false,
                data: None,
                error: Some(Self::format_missing_params_error(
                    &workflow, &missing, &provided,
                )),
                steps_executed: 0,
                healing_attempted: false,
                healing_successful: false,
            });
        }

        // Execute workflow steps
        let mut steps_executed = 0;
        let mut steps_attempted = 0;
        let mut final_data = None;
        let mut screenshots: Vec<Value> = Vec::new();
        let mut healing_attempted = false;
        let mut healing_successful = false;
        let mut workflow_modified = false;
        let mut terminal_error: Option<String> = None;

        'workflow_run: for sequence in workflow.browser_automation.sequences.iter_mut() {
            for step in sequence.steps.iter_mut() {
                steps_attempted += 1;
                let original_step = step.clone();
                match self
                    .execute_workflow_step(&original_step, &parameters)
                    .await
                {
                    Ok(result) => {
                        steps_executed += 1;
                        if let Some(data) = result {
                            if matches!(original_step.kind, StepKind::TakeScreenshot { .. }) {
                                if let Some(saved) = Self::persist_screenshot_artifact_best_effort(
                                    &request.workflow,
                                    &original_step.id,
                                    &data,
                                )
                                .await
                                {
                                    screenshots.push(saved);
                                } else {
                                    let error = data
                                        .get("error")
                                        .and_then(|v| v.as_str())
                                        .or_else(|| data.get("error_msg").and_then(|v| v.as_str()))
                                        .unwrap_or("missing data_url in screenshot response");
                                    screenshots.push(json!({
                                        "type": "screenshot",
                                        "step_id": original_step.id,
                                        "error": error,
                                    }));
                                }
                            } else if matches!(
                                original_step.kind,
                                StepKind::ExtractStructuredData { .. }
                            ) {
                                if let StepKind::ExtractStructuredData {
                                    extraction_type, ..
                                } = &original_step.kind
                                {
                                    Self::merge_extracted_payload(
                                        &mut final_data,
                                        data,
                                        extraction_type.as_deref(),
                                    );
                                } else {
                                    Self::merge_extracted_payload(&mut final_data, data, None);
                                }
                            } else if !matches!(data, Value::Null | Value::Bool(_)) {
                                final_data = Some(data);
                            }
                        }
                    }
                    Err(e) => {
                        // Screenshots are best-effort artifacts. They should not fail the entire
                        // workflow (especially when the user's installed extension is stale).
                        if matches!(original_step.kind, StepKind::TakeScreenshot { .. }) {
                            let mut error = e.to_string();
                            if error.contains("Unknown action type: take_screenshot") {
                                error = format!(
                                    "{}\nHint: your loaded extension does not support `take_screenshot` yet. Rebuild and reload it:\n  - `make build-ext`\n  - reload the unpacked extension from `extension/dist/chrome` (or run `make reload-ext`)\nThen re-run the workflow.",
                                    error
                                );
                            }
                            warn!(
                                "Screenshot step {} failed (continuing): {}",
                                original_step.id, error
                            );
                            screenshots.push(json!({
                                "type": "screenshot",
                                "step_id": original_step.id,
                                "error": error,
                            }));
                            continue;
                        }

                        if let PlanError::PolicyBlocked(reason) = &e {
                            terminal_error = Some(format!(
                                "Policy blocked step {} ({}): {}",
                                original_step.id, original_step.name, reason
                            ));
                            break 'workflow_run;
                        }

                        if request.auto_heal {
                            info!("Step failed, attempting self-healing: {}", e);
                            healing_attempted = true;

                            // Attempt 1: heuristic self-healing (fast, no tokens)
                            let mut healed_step: Option<Step> = self
                                .self_healer
                                .heal_step(&original_step, &e.to_string())
                                .await
                                .ok();

                            // Attempt 2: LLM self-healing (selector repair / strategy change)
                            if healed_step.is_none() {
                                match self
                                    .heal_step_with_llm(&original_step, &e.to_string())
                                    .await
                                {
                                    Ok(step) => healed_step = Some(step),
                                    Err(heal_error) => {
                                        terminal_error =
                                            Some(format!("Self-healing failed: {}", heal_error));
                                        break 'workflow_run;
                                    }
                                }
                            }

                            let healed_step = healed_step.expect("healed_step must be Some");

                            match self.execute_workflow_step(&healed_step, &parameters).await {
                                Ok(result) => {
                                    steps_executed += 1;
                                    healing_successful = true;
                                    if let Some(data) = result {
                                        if matches!(
                                            healed_step.kind,
                                            StepKind::TakeScreenshot { .. }
                                        ) {
                                            if let Some(saved) =
                                                Self::persist_screenshot_artifact_best_effort(
                                                    &request.workflow,
                                                    &healed_step.id,
                                                    &data,
                                                )
                                                .await
                                            {
                                                screenshots.push(saved);
                                            }
                                        } else if matches!(
                                            healed_step.kind,
                                            StepKind::ExtractStructuredData { .. }
                                        ) {
                                            if let StepKind::ExtractStructuredData {
                                                extraction_type,
                                                ..
                                            } = &healed_step.kind
                                            {
                                                Self::merge_extracted_payload(
                                                    &mut final_data,
                                                    data,
                                                    extraction_type.as_deref(),
                                                );
                                            } else {
                                                Self::merge_extracted_payload(
                                                    &mut final_data,
                                                    data,
                                                    None,
                                                );
                                            }
                                        } else {
                                            final_data = Some(data);
                                        }
                                    }

                                    // Update the workflow in memory so future runs can reuse the healed selector.
                                    *step = healed_step;
                                    workflow_modified = true;
                                    continue;
                                }
                                Err(heal_error) => {
                                    terminal_error =
                                        Some(format!("Healing failed: {}", heal_error));
                                    break 'workflow_run;
                                }
                            }
                        } else {
                            terminal_error = Some(e.to_string());
                            break 'workflow_run;
                        }
                    }
                }
            }
        }

        // If the workflow was healed, persist it into the workflow cache dir for reuse.
        // We intentionally do not overwrite an external workflow file path.
        if workflow_modified {
            if let Err(e) = self.workflow_manager.save_workflow(&workflow).await {
                warn!("Failed to save healed workflow: {}", e);
            }
        }

        // Attach any non-text artifacts (e.g., screenshots) to the final output.
        // Keep the default array output for normal search workflows unless artifacts exist.
        if !screenshots.is_empty() {
            let artifacts = json!({ "screenshots": screenshots });
            final_data = Some(match final_data.take() {
                Some(Value::Object(mut obj)) => {
                    obj.insert("artifacts".to_string(), artifacts);
                    Value::Object(obj)
                }
                Some(other) => json!({ "results": other, "artifacts": artifacts }),
                None => json!({ "artifacts": artifacts }),
            });
        }

        // Best-effort cleanup: close the workflow tab after execution to avoid tab buildup.
        // This should not impact the overall workflow success/failure.
        if steps_attempted > 0 {
            let cleanup_step = Step {
                id: "cleanup_close_tab".to_string(),
                name: "Close workflow tab".to_string(),
                kind: StepKind::CloseCurrentTab {
                    tab_identifier: serde_json::Value::Null,
                },
            };

            match self.execute_step_through_broker(&cleanup_step).await {
                Ok((execution_result, _raw_dom)) => {
                    if let ExecutionResult::Error { message, .. } = execution_result {
                        let mut msg = message;
                        if msg.contains("Unknown action type: close_current_tab") {
                            msg = format!(
                                "{}\nHint: your loaded extension does not support `close_current_tab` yet. Rebuild and reload it:\n  - `make build-ext`\n  - reload the unpacked extension from `extension/dist/chrome` (or run `make reload-ext`)\nThen re-run the workflow.",
                                msg
                            );
                        }
                        warn!("Best-effort tab cleanup failed: {}", msg);
                    }
                }
                Err(e) => {
                    warn!("Best-effort tab cleanup failed: {}", e);
                }
            }

            // The extension may omit current_tab_id on close responses; clear session state so
            // future runs don't try to reuse a closed tab id.
            self.broker_client.session.current_tab_id = None;
            self.broker_client.session.current_url = None;
        }

        Ok(RunResponse {
            success: terminal_error.is_none(),
            data: final_data,
            error: terminal_error,
            steps_executed,
            healing_attempted,
            healing_successful,
        })
    }

    fn merge_extracted_payload(
        final_data: &mut Option<Value>,
        payload: Value,
        extraction_type: Option<&str>,
    ) {
        let key = extraction_type
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            // Treat "search_results" as the default (keep legacy array output so CLI pretty-prints).
            .filter(|s| !s.eq_ignore_ascii_case("search_results"));

        // Legacy behavior: plain array merge (keeps google_search pretty output).
        if key.is_none() {
            match final_data {
                Some(Value::Array(existing)) if payload.is_array() => {
                    if let Value::Array(incoming) = payload {
                        for item in incoming {
                            if !existing.iter().any(|current| current == &item) {
                                existing.push(item);
                            }
                        }
                    }
                }
                _ => {
                    *final_data = Some(payload);
                }
            }
            return;
        }

        // Keyed behavior: group extracted payloads by `extraction_type`.
        let key = key.unwrap().to_string();
        let mut obj = match final_data.take() {
            Some(Value::Object(map)) => map,
            Some(other) => {
                let mut map = serde_json::Map::new();
                // Ignore trivial outputs from prior steps (e.g., wait/dismiss returning `true`).
                if !matches!(other, Value::Null | Value::Bool(_)) {
                    map.insert("results".to_string(), other);
                }
                map
            }
            None => serde_json::Map::new(),
        };

        match obj.get_mut(&key) {
            Some(Value::Array(existing)) if payload.is_array() => {
                if let Value::Array(incoming) = payload {
                    for item in incoming {
                        if !existing.iter().any(|current| current == &item) {
                            existing.push(item);
                        }
                    }
                }
            }
            _ => {
                obj.insert(key, payload);
            }
        }

        *final_data = Some(Value::Object(obj));
    }

    fn post_process_step_payload_best_effort(step: &Step, payload: Value) -> Value {
        let StepKind::ExtractStructuredData {
            limit,
            fields,
            extraction_type,
            ..
        } = &step.kind
        else {
            return payload;
        };

        let limit_usize = limit
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n > 0);

        let mut out = payload;

        let apply_fields = |obj: &mut serde_json::Map<String, Value>| {
            for field in fields {
                if field.post_processing.is_empty() {
                    continue;
                }
                let Some(Value::String(current)) = obj.get(&field.name).cloned() else {
                    continue;
                };
                let updated =
                    Self::apply_post_processing_ops_best_effort(&current, &field.post_processing);
                obj.insert(field.name.clone(), Value::String(updated));
            }
        };

        match &mut out {
            Value::Array(items) => {
                if let Some(n) = limit_usize {
                    if items.len() > n {
                        items.truncate(n);
                    }
                }

                for item in items.iter_mut() {
                    if let Value::Object(obj) = item {
                        apply_fields(obj);
                    }
                }

                // For non-search extractions that are explicitly limited to 1, unwrap the array
                // to a single object for cleaner structured outputs.
                if limit == &Some(1) {
                    if let Some(t) = extraction_type.as_deref() {
                        if !t.eq_ignore_ascii_case("search_results") && items.len() == 1 {
                            if let Some(first) = items.first().cloned() {
                                return first;
                            }
                        }
                    }
                }

                out
            }
            Value::Object(obj) => {
                apply_fields(obj);
                out
            }
            _ => out,
        }
    }

    fn apply_post_processing_ops_best_effort(value: &str, ops: &[String]) -> String {
        let mut out = value.to_string();

        for op_raw in ops {
            let op = op_raw.trim();
            if op.is_empty() {
                continue;
            }

            if op == "trim" {
                out = out.trim().to_string();
                continue;
            }
            if op == "collapse_whitespace" {
                out = out.split_whitespace().collect::<Vec<_>>().join(" ");
                continue;
            }

            if let Some(rest) = op.strip_prefix("regex_group:") {
                let (pattern, group) = Self::split_regex_group(rest);
                if let Ok(re) = RegexBuilder::new(pattern).case_insensitive(true).build() {
                    if let Some(caps) = re.captures(&out) {
                        if let Some(m) = caps.get(group).or_else(|| caps.get(0)) {
                            out = m.as_str().to_string();
                        }
                    }
                }
                continue;
            }

            if let Some(pattern) = op.strip_prefix("regex:") {
                if let Ok(re) = RegexBuilder::new(pattern).case_insensitive(true).build() {
                    if let Some(caps) = re.captures(&out) {
                        if let Some(m) = caps.get(1).or_else(|| caps.get(0)) {
                            out = m.as_str().to_string();
                        }
                    }
                }
                continue;
            }
        }

        out
    }

    fn split_regex_group(rest: &str) -> (&str, usize) {
        let Some(pos) = rest.rfind(':') else {
            return (rest, 1);
        };

        let (pattern, group_raw) = rest.split_at(pos);
        let group_raw = group_raw.trim_start_matches(':');
        let group = group_raw.parse::<usize>().unwrap_or(1);
        (pattern, group)
    }

    fn sanitize_artifact_component(raw: &str) -> String {
        let mut out = String::with_capacity(raw.len());
        for c in raw.chars() {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                out.push(c);
            } else {
                out.push('_');
            }
        }
        if out.is_empty() {
            "workflow".to_string()
        } else {
            out
        }
    }

    fn workflow_artifacts_dir(workflow_path: &str) -> PathBuf {
        let stem = Path::new(workflow_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("workflow");
        let safe = Self::sanitize_artifact_component(stem);
        PathBuf::from("test-results")
            .join("workflow-artifacts")
            .join(safe)
    }

    fn infer_screenshot_ext(data_url: &str, format_hint: Option<&str>) -> &'static str {
        if data_url.starts_with("data:image/png") {
            return "png";
        }
        if data_url.starts_with("data:image/jpeg") || data_url.starts_with("data:image/jpg") {
            return "jpg";
        }

        let hint = format_hint.unwrap_or("png").trim().to_lowercase();
        if hint == "jpg" || hint == "jpeg" {
            "jpg"
        } else {
            "png"
        }
    }

    fn decode_screenshot_data_url(data_url: &str) -> Result<Vec<u8>, String> {
        let b64 = if let Some((_, data)) = data_url.split_once(',') {
            data
        } else {
            data_url
        };
        general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| format!("base64 decode failed: {}", e))
    }

    async fn persist_screenshot_artifact_best_effort(
        workflow_path: &str,
        step_id: &str,
        payload: &Value,
    ) -> Option<Value> {
        let data_url = payload
            .get("data_url")
            .and_then(|v| v.as_str())
            .or_else(|| payload.get("dataUrl").and_then(|v| v.as_str()));

        let Some(data_url) = data_url else {
            return None;
        };

        let format_hint = payload.get("format").and_then(|v| v.as_str());
        let ext = Self::infer_screenshot_ext(data_url, format_hint);

        let bytes = match Self::decode_screenshot_data_url(data_url) {
            Ok(b) => b,
            Err(e) => {
                warn!("Failed to decode screenshot data URL: {}", e);
                return Some(json!({ "type": "screenshot", "format": ext, "error": e }));
            }
        };

        let dir = Self::workflow_artifacts_dir(workflow_path);
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            warn!("Failed to create screenshot artifacts dir {:?}: {}", dir, e);
            return Some(json!({
                "type": "screenshot",
                "format": ext,
                "error": format!("failed to create artifacts dir: {}", e),
            }));
        }

        let safe_step = Self::sanitize_artifact_component(step_id);
        let filename = format!("{}-{}.{}", safe_step, Uuid::new_v4(), ext);
        let path = dir.join(filename);

        if let Err(e) = tokio::fs::write(&path, &bytes).await {
            warn!("Failed to write screenshot {:?}: {}", path, e);
            return Some(json!({
                "type": "screenshot",
                "format": ext,
                "error": format!("failed to write screenshot: {}", e),
            }));
        }

        Some(json!({
            "type": "screenshot",
            "format": ext,
            "path": path.to_string_lossy().to_string(),
        }))
    }

    async fn heal_step_with_llm(
        &mut self,
        failed_step: &Step,
        error_message: &str,
    ) -> PlanResult<Step> {
        // Gather a compact page snapshot for the repair prompt.
        let dom_snapshot = self
            .broker_client
            .get_dom_snapshot()
            .await
            .unwrap_or_else(|_| json!({}));
        let dom_prompt = dom_snapshot
            .get("dom_snapshot")
            .and_then(|v| v.get("prompt"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let current_url = dom_snapshot
            .get("current_url")
            .and_then(|v| v.as_str())
            .or_else(|| {
                dom_snapshot
                    .get("metadata")
                    .and_then(|m| m.get("url"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("");

        // Build a prompt that forces a single JSON step (StepKind shape).
        let failed_exec = StepExecution {
            step: failed_step.clone(),
            result: ExecutionResult::Error {
                message: error_message.to_string(),
                retry_suggested: true,
            },
            timestamp: chrono::Utc::now(),
            dom_snapshot: None,
        };
        let messages =
            self.prompt_builder
                .build_healing_prompt(&failed_exec, dom_prompt, error_message);

        let proposed = self.llm_client.chat_json(messages, Some(0.0)).await?;

        // Parse as StepKind (the caller will wrap id/name).
        let kind: StepKind = serde_json::from_value(proposed).map_err(|e| {
            PlanError::ExecutionError(format!("LLM healing produced invalid step JSON: {}", e))
        })?;

        let mut healed = Step {
            id: format!("{}_healed_llm", failed_step.id),
            name: format!("{} (healed)", failed_step.name),
            kind,
        };

        // Policy gate: LLM-healed steps run automatically, so keep it conservative.
        self.policy_gate
            .enforce_step(&healed, None, Some(current_url))
            .await?;

        // Sanitize (e.g., drop iframe selectors).
        if let Some(sanitized) = self.plan_sanitizer.sanitize_step(&healed)? {
            healed = sanitized;
        } else {
            return Err(PlanError::ExecutionError(
                "LLM healed step was blocked by sanitizer".to_string(),
            ));
        }

        info!(
            "LLM self-healing proposed step for {} on {}: {:?}",
            failed_step.id, current_url, healed.kind
        );

        Ok(healed)
    }

    /// Execute a single step and update session state
    async fn execute_step(
        &mut self,
        step: &Step,
        session: &mut PlanningSession,
    ) -> PlanResult<(ExecutionResult, String)> {
        info!("[START] Executing step: {} - {}", step.id, step.name);
        debug!("Step details: {:?}", step);

        // Policy gate for high-risk actions. Low-risk steps proceed without prompts.
        self.policy_gate
            .enforce_step(step, None, Some(&session.current_url))
            .await?;

        // Get appropriate wait strategy for this action
        let action_type = match &step.kind {
            StepKind::NavigateToUrl { .. } => "navigate_to_url",
            StepKind::ClickElement { .. } => "click_element",
            StepKind::FillInputField { .. } => "fill_input_field",
            StepKind::TypeText { .. } => "type_text",
            StepKind::SubmitInput { .. } => "submit_input",
            StepKind::PressSpecialKey { .. } => "press_special_key",
            _ => "default",
        };

        let wait_strategy = self
            .wait_strategy
            .get_strategy(&session.current_url, action_type);
        debug!(
            "Using wait strategy for {}: {:?}",
            action_type, wait_strategy
        );

        // Check if this step needs macro expansion
        match &step.kind {
            StepKind::InfiniteScroll {
                item_selector,
                target_count,
                max_cycles,
                frame_id: _,
            } => {
                let result = self
                    .execute_infinite_scroll_macro(
                        step,
                        session,
                        item_selector,
                        *target_count,
                        *max_cycles,
                    )
                    .await?;
                // For macros, return empty raw_dom since they handle their own DOM updates
                return Ok((result, String::new()));
            }
            StepKind::WaitForElement {
                selector,
                timeout_ms,
                condition,
                frame_id: _,
            } => {
                //  CRITICAL FIX: Enhanced wait that polls for text content
                let result = self
                    .execute_robust_wait_for_element(
                        step,
                        session,
                        selector,
                        *timeout_ms,
                        condition.as_deref(),
                    )
                    .await?;
                // For macros, return empty raw_dom since they handle their own DOM updates
                return Ok((result, String::new()));
            }
            // Add other macro expansions here in the future
            _ => {
                // Regular step execution
            }
        }

        info!(" Sending step to broker: {:?}", step);

        // Use combined execution for ALL steps to maintain tab state
        // TODO: Implement DOM delta optimization (optimization #7)
        // let delta_step = Step {
        //     id: format!("{}_delta", step.id),
        //     name: "Get DOM delta".to_string(),
        //     kind: StepKind::GetDomDelta { since_revision: self.dom_revision },
        // };

        // Build a small batch when action likely changes page state, to capture stable post-DOM
        let batched: Option<Vec<Step>> = match &step.kind {
            StepKind::NavigateToUrl { .. } => {
                let wait = Step {
                    id: format!("{}_wait_idle", step.id),
                    name: "Wait for network idle".to_string(),
                    kind: StepKind::WaitForNetworkIdle {
                        idle_time_ms: 1200,
                        max_wait_ms: 10_000,
                    },
                };
                Some(vec![step.clone(), wait])
            }
            StepKind::PressSpecialKey { key, .. } if key.eq_ignore_ascii_case("enter") => {
                let wait_nav = Step {
                    id: format!("{}_wait_nav", step.id),
                    name: "Wait for navigation".to_string(),
                    kind: StepKind::WaitForNavigation {
                        url_pattern: None,
                        timeout_ms: Some(10_000),
                    },
                };
                let wait_idle = Step {
                    id: format!("{}_wait_idle", step.id),
                    name: "Wait for network idle".to_string(),
                    kind: StepKind::WaitForNetworkIdle {
                        idle_time_ms: 800,
                        max_wait_ms: 8_000,
                    },
                };
                Some(vec![step.clone(), wait_nav, wait_idle])
            }
            StepKind::SubmitInput { .. } => {
                let wait_nav = Step {
                    id: format!("{}_wait_nav", step.id),
                    name: "Wait for navigation".to_string(),
                    kind: StepKind::WaitForNavigation {
                        url_pattern: None,
                        timeout_ms: Some(10_000),
                    },
                };
                let wait_idle = Step {
                    id: format!("{}_wait_idle", step.id),
                    name: "Wait for network idle".to_string(),
                    kind: StepKind::WaitForNetworkIdle {
                        idle_time_ms: 800,
                        max_wait_ms: 8_000,
                    },
                };
                Some(vec![step.clone(), wait_nav, wait_idle])
            }
            StepKind::ClickElement { .. } => {
                // Conservative idle wait after clicks; avoids flakiness on SPAs
                let wait_idle = Step {
                    id: format!("{}_wait_idle", step.id),
                    name: "Wait for network idle".to_string(),
                    kind: StepKind::WaitForNetworkIdle {
                        idle_time_ms: 600,
                        max_wait_ms: 6_000,
                    },
                };
                Some(vec![step.clone(), wait_idle])
            }
            _ => None,
        };

        match if let Some(batch) = batched {
            self.broker_client.execute_steps_and_get_dom(batch).await
        } else {
            self.broker_client.execute_step_and_get_dom(step).await
        } {
            Ok((response, raw_dom)) => {
                info!(" Received response from broker");
                debug!(
                    "Response: {}",
                    serde_json::to_string_pretty(&response)
                        .unwrap_or_else(|_| "Failed to serialize".to_string())
                );

                // [SEARCH] DEBUG: Log response structure for auto-extraction debugging
                if let StepKind::GetElementText { .. } = &step.kind {
                    info!("[SEARCH] DEBUG: get_element_text response structure:");
                    info!(
                        "  - Full response: {}",
                        serde_json::to_string_pretty(&response)
                            .unwrap_or_else(|_| "Failed to serialize".to_string())
                    );
                    if let Some(result) = response.get("result") {
                        info!("  - Has 'result' field");
                        if let Some(result_steps) = result.get("steps") {
                            info!(
                                "  - result.steps exists: {} items",
                                result_steps.as_array().map(|a| a.len()).unwrap_or(0)
                            );
                        }
                    }
                    if let Some(steps) = response.get("steps") {
                        info!(
                            "  - Has top-level 'steps' field: {} items",
                            steps.as_array().map(|a| a.len()).unwrap_or(0)
                        );
                        if let Some(steps_array) = steps.as_array() {
                            for (i, step) in steps_array.iter().enumerate() {
                                info!("  - Step {}: {:?}", i, step);
                            }
                        }
                    }
                }

                // Process DOM with both old and new methods for compatibility
                let reduced_dom = self.dom_analyzer.reduce_html(&raw_dom)?;

                // Debug: Log DOM reduction effectiveness
                eprintln!("[SEARCH] DOM Reduction Stats:");
                eprintln!("   Raw DOM size: {} chars", raw_dom.len());
                eprintln!("   Reduced DOM size: {} chars", reduced_dom.len());
                eprintln!(
                    "   Reduction ratio: {:.1}%",
                    (reduced_dom.len() as f64 / raw_dom.len() as f64) * 100.0
                );

                // Check if reduction is actually working
                if reduced_dom.len() > raw_dom.len() / 2 {
                    eprintln!("   [WARNING]  WARNING: DOM reduction is not effective! Still {}% of original size", 
                        (reduced_dom.len() as f64 / raw_dom.len() as f64 * 100.0) as i32);
                }

                // Extract structured DOM context for LLM planning
                let dom_context = self
                    .dom_processor
                    .extract_dom_context(&raw_dom, &session.current_url)
                    .map_err(|e| PlanError::DomError(format!("DOM processing failed: {}", e)))?;

                //  CRITICAL FIX: Don't try to detect URL from DOM content
                // Let the browser provide the actual URL instead of guessing
                let detected_url = None;

                let execution_result = match response.get("success") {
                    Some(success) if success.as_bool() == Some(false) => {
                        let error_msg = response
                            .get("error")
                            .and_then(|e| e.as_str())
                            .unwrap_or("Step execution failed");
                        error!("[ERROR] Step execution failed: {}", error_msg);
                        ExecutionResult::Error {
                            message: error_msg.to_string(),
                            retry_suggested: true,
                        }
                    }
                    _ => {
                        info!("[OK] Step executed successfully");

                        // Reset failure counter on success
                        session.failure_tracker.record_success();

                        //  FIX: Update URL from multiple sources
                        let mut url_updated = false;

                        // 1. Try to get URL from broker session
                        if let Some(broker_url) = self.broker_client.get_current_url() {
                            if broker_url != session.current_url {
                                info!(
                                    " URL updated from broker: {} -> {}",
                                    session.current_url, broker_url
                                );
                                session.current_url = broker_url;
                                url_updated = true;
                            }
                        }

                        // 2. Try to get URL from response
                        if !url_updated {
                            if let Some(response_url) =
                                response.get("current_url").and_then(|u| u.as_str())
                            {
                                if response_url != session.current_url {
                                    info!(
                                        " URL updated from response: {} -> {}",
                                        session.current_url, response_url
                                    );
                                    session.current_url = response_url.to_string();
                                    url_updated = true;
                                }
                            }
                        }

                        // 3. Fallback: Use detected URL from DOM analysis
                        if !url_updated {
                            if let Some(dom_url) = detected_url {
                                if dom_url != session.current_url {
                                    info!(
                                        " URL updated from DOM analysis: {} -> {}",
                                        session.current_url, dom_url
                                    );
                                    session.current_url = dom_url;
                                    url_updated = true;
                                }
                            }
                        }

                        // 4. No special handling needed - browser will provide actual URL

                        if url_updated {
                            info!("[TARGET] URL change detected - LLM will now see the new page context");
                        }

                        // Check for extracted data in response
                        let payload = response.get("extracted_data").cloned();
                        if payload.is_some() {
                            info!(" Extracted data found in response");
                        }

                        // [TARGET] EARLY AUTO-EXTRACTION CHECK: For get_element_text steps
                        // Check if the extension auto-extracted the data immediately
                        if let StepKind::GetElementText { .. } = &step.kind {
                            // Check in steps array first (most likely location)
                            if let Some(steps) = response.get("steps").and_then(|s| s.as_array()) {
                                for step_result in steps {
                                    if let Some(step_data) = step_result.get("data") {
                                        if step_data.is_object()
                                            && step_data
                                                .get("auto_extracted")
                                                .and_then(|v| v.as_bool())
                                                == Some(true)
                                            && step_data
                                                .get("goal_completed")
                                                .and_then(|v| v.as_bool())
                                                == Some(true)
                                        {
                                            if let Some(extraction_data) =
                                                step_data.get("extraction_data")
                                            {
                                                info!("[TARGET] AUTO-EXTRACTION DETECTED IMMEDIATELY! Goal completed by extension.");
                                                info!(
                                                    " Auto-extracted data: {}",
                                                    serde_json::to_string_pretty(extraction_data)
                                                        .unwrap_or_else(
                                                            |_| "Failed to serialize".to_string()
                                                        )
                                                );

                                                // Return success with the auto-extraction flags preserved
                                                let mut result_with_flags = serde_json::Map::new();
                                                result_with_flags.insert(
                                                    "auto_extracted".to_string(),
                                                    json!(true),
                                                );
                                                result_with_flags.insert(
                                                    "goal_completed".to_string(),
                                                    json!(true),
                                                );
                                                result_with_flags.insert(
                                                    "extraction_data".to_string(),
                                                    extraction_data.clone(),
                                                );

                                                return Ok((
                                                    ExecutionResult::Success {
                                                        payload: Some(serde_json::Value::Object(
                                                            result_with_flags,
                                                        )),
                                                    },
                                                    raw_dom.clone(),
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        //  CRITICAL FIX: Capture inspection data for failed extractions
                        if let StepKind::ExtractStructuredData { .. } = &step.kind {
                            if let Some(extracted_data) = &payload {
                                if let Some(array) = extracted_data.as_array() {
                                    if array.is_empty() {
                                        println!("[WARNING]  EXTRACTION RETURNED 0 ITEMS - Capturing inspection data for LLM feedback");

                                        // Capture inspection data from response and store it for LLM feedback
                                        if let Some(inspection_data) =
                                            self.extract_inspection_data_from_response(&response)
                                        {
                                            println!(
                                                " INSPECTION DATA CAPTURED: {}",
                                                serde_json::to_string_pretty(&inspection_data)
                                                    .unwrap_or_else(
                                                        |_| "Failed to serialize".to_string()
                                                    )
                                            );

                                            // Store inspection data in session for LLM feedback
                                            let inspection_step = StepExecution {
                                                step: Step {
                                                    id: format!("{}_inspection", step.id),
                                                    name: "Page Inspection for Failed Extraction"
                                                        .to_string(),
                                                    kind: StepKind::GetPageSource, // Placeholder step type
                                                },
                                                result: ExecutionResult::Success {
                                                    payload: Some(inspection_data),
                                                },
                                                timestamp: chrono::Utc::now(),
                                                dom_snapshot: Some(reduced_dom.clone()),
                                            };

                                            session.history.push(inspection_step);
                                            info!("[NOTE] Added inspection data to session history for LLM feedback");
                                        } else {
                                            println!("[WARNING]  No inspection data found in response - LLM will use standard guidance");
                                        }
                                    }
                                }
                            }
                        }

                        // [TARGET] AUTO-EXTRACTION DETECTION: Check if the extension auto-extracted goal data
                        // This happens when get_element_text finds goal-relevant data and auto-extracts it

                        // First check in the result.steps array (where the extension puts it)
                        if let Some(result_data) = response.get("result") {
                            if let Some(steps_array) =
                                result_data.get("steps").and_then(|s| s.as_array())
                            {
                                for step_result in steps_array {
                                    // Check for auto-extraction flags in step data
                                    if let Some(step_data) = step_result.get("data") {
                                        // Handle both object data (with auto_extracted flag) and direct data
                                        if step_data.is_object() {
                                            if let (Some(auto_extracted), Some(goal_completed)) = (
                                                step_data
                                                    .get("auto_extracted")
                                                    .and_then(|v| v.as_bool()),
                                                step_data
                                                    .get("goal_completed")
                                                    .and_then(|v| v.as_bool()),
                                            ) {
                                                if auto_extracted && goal_completed {
                                                    if let Some(extraction_data) =
                                                        step_data.get("extraction_data")
                                                    {
                                                        info!("[TARGET] AUTO-EXTRACTION DETECTED! Goal completed by extension.");
                                                        info!(
                                                            " Auto-extracted data: {}",
                                                            serde_json::to_string_pretty(
                                                                extraction_data
                                                            )
                                                            .unwrap_or_else(|_| {
                                                                "Failed to serialize".to_string()
                                                            })
                                                        );

                                                        // Send to LLM for verification
                                                        let verification_result = self
                                                            .verify_extraction_with_llm(
                                                                &session.goal,
                                                                extraction_data,
                                                                step_data
                                                                    .get("text")
                                                                    .and_then(|t| t.as_str()),
                                                            )
                                                            .await;

                                                        match verification_result {
                                                            Ok(verified_data) => {
                                                                info!("[OK] LLM verified extraction is complete and correct");
                                                                // Return the data with auto-extraction flags so the main loop can detect it
                                                                let mut result_with_flags =
                                                                    serde_json::Map::new();
                                                                result_with_flags.insert(
                                                                    "auto_extracted".to_string(),
                                                                    json!(true),
                                                                );
                                                                result_with_flags.insert(
                                                                    "goal_completed".to_string(),
                                                                    json!(true),
                                                                );
                                                                result_with_flags.insert(
                                                                    "extraction_data".to_string(),
                                                                    verified_data,
                                                                );
                                                                return Ok((ExecutionResult::Success {
                                                                    payload: Some(serde_json::Value::Object(result_with_flags))
                                                                }, raw_dom.clone()));
                                                            }
                                                            Err(e) => {
                                                                warn!("[WARNING] LLM verification failed: {}, continuing with planning", e);
                                                                // Don't return, let the LLM continue planning
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Also check in the top-level steps array (for backwards compatibility)
                        if let Some(steps) = response.get("steps").and_then(|s| s.as_array()) {
                            for step_result in steps {
                                // For get_element_text, check if data is an object with auto_extracted flag
                                if let Some(step_data) = step_result.get("data") {
                                    // Log what we found for debugging
                                    if let StepKind::GetElementText { .. } = &step.kind {
                                        info!(
                                            "[SEARCH] DEBUG: Found step data type: {}",
                                            if step_data.is_object() {
                                                "object"
                                            } else if step_data.is_string() {
                                                "string"
                                            } else {
                                                "other"
                                            }
                                        );

                                        if step_data.is_object() {
                                            info!(
                                                "[SEARCH] DEBUG: Step data keys: {:?}",
                                                step_data
                                                    .as_object()
                                                    .map(|o| o.keys().collect::<Vec<_>>())
                                            );
                                        }
                                    }

                                    if step_data.is_object() {
                                        if let (Some(auto_extracted), Some(goal_completed)) = (
                                            step_data
                                                .get("auto_extracted")
                                                .and_then(|v| v.as_bool()),
                                            step_data
                                                .get("goal_completed")
                                                .and_then(|v| v.as_bool()),
                                        ) {
                                            if auto_extracted && goal_completed {
                                                if let Some(extraction_data) =
                                                    step_data.get("extraction_data")
                                                {
                                                    info!("[TARGET] AUTO-EXTRACTION DETECTED (top-level)! Goal completed by extension.");
                                                    info!(
                                                        " Auto-extracted data: {}",
                                                        serde_json::to_string_pretty(
                                                            extraction_data
                                                        )
                                                        .unwrap_or_else(
                                                            |_| "Failed to serialize".to_string()
                                                        )
                                                    );

                                                    // For now, skip LLM verification to test if auto-extraction detection works
                                                    info!("[START] FAST PATH: Skipping LLM verification for testing");

                                                    // Return the data with auto-extraction flags so the main loop can detect it
                                                    let mut result_with_flags =
                                                        serde_json::Map::new();
                                                    result_with_flags.insert(
                                                        "auto_extracted".to_string(),
                                                        json!(true),
                                                    );
                                                    result_with_flags.insert(
                                                        "goal_completed".to_string(),
                                                        json!(true),
                                                    );
                                                    result_with_flags.insert(
                                                        "extraction_data".to_string(),
                                                        extraction_data.clone(),
                                                    );
                                                    return Ok((
                                                        ExecutionResult::Success {
                                                            payload: Some(
                                                                serde_json::Value::Object(
                                                                    result_with_flags,
                                                                ),
                                                            ),
                                                        },
                                                        raw_dom.clone(),
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        ExecutionResult::Success { payload }
                    }
                };

                // Update session state
                let execution = StepExecution {
                    step: step.clone(),
                    result: execution_result.clone(),
                    timestamp: chrono::Utc::now(),
                    dom_snapshot: Some(session.current_dom.clone()),
                };

                session.history.push(execution);
                session.current_dom = reduced_dom;

                // Update DOM context for LLM planning
                self.current_dom_context = Some(dom_context);

                // Increment DOM revision for delta tracking (optimization #7)
                self.dom_revision += 1;

                info!(" Updated session state:");
                info!("   ├─ URL: {}", session.current_url);
                info!("   ├─ DOM size: {} chars", session.current_dom.len());
                if let Some(ref context) = self.current_dom_context {
                    info!(
                        "   ├─ DOM context: {} interactive elements, page type: {}",
                        context.interactive_elements.len(),
                        context.page_type
                    );
                }
                info!("   └─ History steps: {}", session.history.len());

                // Apply wait strategy after step execution
                if wait_strategy.observe_after_action {
                    // Check if response contains DOM observations from the extension
                    if let Some(dom_observations) =
                        response.get("data").and_then(|d| d.get("domChanges"))
                    {
                        let observations: DOMObservation =
                            serde_json::from_value(dom_observations.clone()).unwrap_or({
                                DOMObservation {
                                    has_significant_changes: false,
                                    new_interactive_elements: 0,
                                    dom_stabilized: true,
                                    observation_duration_ms: 0,
                                    changes_count: 0,
                                }
                            });

                        info!("[SEARCH] DOM observations after {}: changes={}, new_elements={}, stabilized={}",
                            action_type,
                            observations.changes_count,
                            observations.new_interactive_elements,
                            observations.dom_stabilized);

                        // Determine if additional wait is needed
                        if let Some(additional_wait) = observations.needs_additional_wait() {
                            info!(
                                "⏳ Waiting additional {}ms for DOM to stabilize",
                                additional_wait.as_millis()
                            );
                            tokio::time::sleep(additional_wait).await;
                        }
                    } else if wait_strategy.wait_for_stability.is_some() {
                        // If no observations but wait_for_stability is configured, apply default wait
                        let wait_ms = wait_strategy.observation_duration.unwrap_or(500);
                        info!(
                            "⏳ Applying default wait of {}ms after {}",
                            wait_ms, action_type
                        );
                        tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                    }
                }

                Ok((execution_result, raw_dom))
            }
            Err(e) => {
                error!("[ERROR] Broker communication failed: {}", e);
                Err(e)
            }
        }
    }

    /// Execute a workflow step with parameter substitution
    async fn execute_workflow_step(
        &mut self,
        step: &Step,
        parameters: &HashMap<String, String>,
    ) -> PlanResult<Option<Value>> {
        // Substitute parameters in the step
        let mut substituted_step = step.clone();

        // Handle parameter substitution for different step types
        match &mut substituted_step.kind {
            rzn_core::StepKind::NavigateToUrl { url, .. } => {
                for (key, value) in parameters {
                    let placeholder = format!("{{{}}}", key);
                    *url = url.replace(&placeholder, value);
                }
            }
            rzn_core::StepKind::OpenNewTab { url, .. } => {
                if let Some(u) = url.as_mut() {
                    for (key, value) in parameters {
                        let placeholder = format!("{{{}}}", key);
                        *u = u.replace(&placeholder, value);
                    }
                }
            }
            rzn_core::StepKind::FillInputField { value, .. } => {
                for (key, val) in parameters {
                    let placeholder = format!("{{{}}}", key);
                    *value = value.replace(&placeholder, val);
                }
            }
            rzn_core::StepKind::TypeText {
                text,
                value,
                selector,
                ..
            } => {
                for (key, val) in parameters {
                    let placeholder = format!("{{{}}}", key);
                    if let Some(text_value) = text.as_mut() {
                        *text_value = text_value.replace(&placeholder, val);
                    }
                    if let Some(raw_value) = value.as_mut() {
                        *raw_value = raw_value.replace(&placeholder, val);
                    }
                    *selector = selector.replace(&placeholder, val);
                }
            }
            rzn_core::StepKind::ExecuteJavascript { script, args, .. } => {
                Self::substitute_known_params_in_string(script, parameters);
                if let Some(args) = args.as_mut() {
                    for arg in args {
                        Self::substitute_params_in_json_value(arg, parameters);
                    }
                }
            }
            rzn_core::StepKind::GetElementText { selector, .. } => {
                for (key, value) in parameters {
                    let placeholder = format!("{{{}}}", key);
                    *selector = selector.replace(&placeholder, value);
                }
            }
            rzn_core::StepKind::SubmitInput { text, selector, .. } => {
                for (key, value) in parameters {
                    let placeholder = format!("{{{}}}", key);
                    *text = text.replace(&placeholder, value);
                    *selector = selector.replace(&placeholder, value);
                }
            }
            rzn_core::StepKind::ExtractStructuredData {
                item_selector,
                limit,
                fields,
                ..
            } => {
                for (key, value) in parameters {
                    let placeholder = format!("{{{}}}", key);
                    *item_selector = item_selector.replace(&placeholder, value);
                    for field in fields.iter_mut() {
                        field.selector = field.selector.replace(&placeholder, value);
                        if let Some(attr) = field.attribute.as_mut() {
                            *attr = attr.replace(&placeholder, value);
                        }
                        for pp in field.post_processing.iter_mut() {
                            *pp = pp.replace(&placeholder, value);
                        }
                    }
                }

                // Optional top-N limiting: aligns with noapi-style `num_results`.
                // This is generic and only applies when the caller provides a limit param.
                let limit_param = parameters
                    .get("num_results")
                    .or_else(|| parameters.get("limit"))
                    .or_else(|| parameters.get("top"));
                if let Some(raw) = limit_param {
                    if let Ok(n) = raw.parse::<u32>() {
                        if n > 0 {
                            *limit = Some(n.min(50));
                        }
                    }
                }
            }
            _ => {} // Other step types don't typically have parameters
        }

        // Policy gate: workflow steps can be dangerous (uploads/cookies/JS). Gate before execution.
        let current_url = self.broker_client.get_current_url();
        self.policy_gate
            .enforce_step(&substituted_step, None, current_url.as_deref())
            .await?;

        // Execute the step through broker
        match self.execute_step_through_broker(&substituted_step).await {
            Ok((result, _raw_dom)) => match result {
                ExecutionResult::Success { payload } => Ok(payload
                    .map(|p| Self::post_process_step_payload_best_effort(&substituted_step, p))),
                ExecutionResult::Error { message, .. } => Err(PlanError::BrokerError(message)),
            },
            Err(e) => Err(e),
        }
    }

    /// Execute step through broker
    async fn execute_step_through_broker(
        &mut self,
        step: &Step,
    ) -> PlanResult<(ExecutionResult, String)> {
        debug!("Executing step through broker: {:?}", step);

        match self.broker_client.execute_step_compact(step).await {
            Ok(response) => {
                // For workflow execution, prefer the lightweight broker call that does NOT
                // append GetPageSource on every step. This reduces payload sizes and prevents
                // native-host/extension disconnects on heavy pages.
                //
                // Note: raw_dom is left empty here. Self-healing (when enabled) uses a
                // dedicated snapshot call instead of relying on per-step HTML capture.
                let raw_dom = String::new();

                // If the broker/extension reported failure, surface it as an ExecutionResult::Error.
                if !response
                    .get("success")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true)
                {
                    let message = response
                        .get("error")
                        .and_then(|v| v.as_str())
                        .or_else(|| response.get("error_msg").and_then(|v| v.as_str()))
                        .unwrap_or("Step failed")
                        .to_string();
                    return Ok((
                        ExecutionResult::Error {
                            message,
                            retry_suggested: true,
                        },
                        raw_dom,
                    ));
                }

                // Debug log the response structure
                // info!("[SEARCH] DEBUG: Raw response from broker: {}", serde_json::to_string_pretty(&response).unwrap_or_else(|_| "Failed to serialize".to_string()));

                // Check if this was an extraction step and extract the data
                match step {
                    Step {
                        kind: StepKind::ExtractStructuredData { .. },
                        ..
                    } => {
                        // First check if results (plural) is at top level
                        if let Some(results) = response.get("results") {
                            info!("[SEARCH] DEBUG: Found 'results' at top level");
                            if let Some(results_array) = results.as_array() {
                                if !results_array.is_empty() {
                                    // Get the first array in results (our extraction data)
                                    if let Some(first_result) = results_array.first() {
                                        if first_result.is_array() {
                                            info!("[SEARCH] DEBUG: Found extraction array with {} items", first_result.as_array().unwrap().len());
                                            return Ok((
                                                ExecutionResult::Success {
                                                    payload: Some(first_result.clone()),
                                                },
                                                raw_dom.clone(),
                                            ));
                                        }
                                    }
                                }
                            }
                        }

                        // Also check if result (singular) is at top level for backward compatibility
                        if let Some(result) = response.get("result") {
                            info!("[SEARCH] DEBUG: Found 'result' at top level");
                            if result.is_array() {
                                info!(
                                    "[SEARCH] DEBUG: Result is array with {} items",
                                    result.as_array().unwrap().len()
                                );
                                return Ok((
                                    ExecutionResult::Success {
                                        payload: Some(result.clone()),
                                    },
                                    raw_dom.clone(),
                                ));
                            }
                        }

                        // Look for extracted data in the response
                        if let Some(steps) = response.get("steps") {
                            if let Some(steps_array) = steps.as_array() {
                                for step_result in steps_array {
                                    if let Some(data) = step_result.get("data") {
                                        if data.is_array() || data.is_object() {
                                            return Ok((
                                                ExecutionResult::Success {
                                                    payload: Some(data.clone()),
                                                },
                                                raw_dom.clone(),
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        // If no data found, return empty array
                        Ok((
                            ExecutionResult::Success {
                                payload: Some(serde_json::json!([])),
                            },
                            raw_dom.clone(),
                        ))
                    }
                    _ => {
                        // For non-extraction steps, return the primary step result when available.
                        // Background execution wraps step outputs under result.results[0].
                        let payload = response
                            .pointer("/result/results/0")
                            .cloned()
                            .or_else(|| {
                                response
                                    .get("results")
                                    .and_then(|v| v.as_array())
                                    .and_then(|a| a.first())
                                    .cloned()
                            })
                            .or_else(|| response.get("result").cloned());

                        Ok((ExecutionResult::Success { payload }, raw_dom.clone()))
                    }
                }
            }
            Err(e) => {
                warn!("Step execution failed: {}", e);
                Ok((
                    ExecutionResult::Error {
                        message: e.to_string(),
                        retry_suggested: true,
                    },
                    String::new(),
                ))
            }
        }
    }

    /// Get current DOM from browser
    async fn get_current_dom(&mut self) -> PlanResult<String> {
        self.broker_client.get_current_dom().await
    }

    /// Create a workflow from a planning session
    fn create_workflow_from_session(
        &self,
        session: &PlanningSession,
        request: &PlanRequest,
    ) -> PlanResult<Workflow> {
        let workflow_name = request
            .workflow_name
            .clone()
            .unwrap_or_else(|| format!("auto_generated_{}", Uuid::new_v4()));

        let sequence = rzn_core::Sequence {
            name: "main".to_string(),
            description: session.goal.clone(),
            required_variables: session
                .parameters
                .keys()
                .map(|k| rzn_core::Variable {
                    name: k.clone(),
                    description: format!("Parameter: {}", k),
                    sensitive: None,
                })
                .collect(),
            steps: session.steps.clone(),
        };

        let workflow = Workflow {
            id: workflow_name.clone(),
            name: workflow_name.clone(),
            description: session.goal.clone(),
            version: "1.0.0".to_string(),
            last_updated: chrono::Utc::now().to_rfc3339(),
            browser_automation: rzn_core::BrowserAutomation {
                sequences: vec![sequence],
            },
        };

        Ok(workflow)
    }

    // Removed: convert_to_heuristic_context method - using LLM-only planning

    /// Execute infinite scroll macro by expanding into atomic scroll+wait+count cycles
    async fn execute_infinite_scroll_macro(
        &mut self,
        step: &Step,
        session: &mut PlanningSession,
        item_selector: &str,
        target_count: u32,
        max_cycles: u32,
    ) -> PlanResult<ExecutionResult> {
        info!(
            " Executing infinite scroll macro: target={} items, max={} cycles",
            target_count, max_cycles
        );

        let mut cycles = 0;
        let mut current_count = 0;
        let mut no_new_items_count = 0;

        while cycles < max_cycles {
            info!(" Infinite scroll cycle {}/{}", cycles + 1, max_cycles);

            // Step 1: Count current items
            let count_step = Step {
                id: format!("{}_count_{}", step.id, cycles),
                name: format!("Count items (cycle {})", cycles + 1),
                kind: StepKind::GetElementCount {
                    selector: item_selector.to_string(),
                    frame_id: None,
                },
            };

            match self
                .broker_client
                .execute_step_and_get_dom(&count_step)
                .await
            {
                Ok((response, raw_dom)) => {
                    let reduced_dom = self.dom_analyzer.reduce_html(&raw_dom)?;
                    session.current_dom = reduced_dom;

                    // Extract count from response
                    if let Some(steps) = response.get("steps").and_then(|s| s.as_array()) {
                        if let Some(step_result) = steps.first() {
                            if let Some(data) = step_result.get("data") {
                                if let Some(count) = data.get("count").and_then(|c| c.as_u64()) {
                                    let new_count = count as u32;
                                    info!(
                                        " Current item count: {} (was: {})",
                                        new_count, current_count
                                    );

                                    // Check if we've reached our target
                                    if new_count >= target_count {
                                        info!("[TARGET] Infinite scroll complete: reached target of {} items", target_count);
                                        return Ok(ExecutionResult::Success {
                                            payload: Some(serde_json::json!({
                                                "items_found": new_count,
                                                "cycles_completed": cycles + 1,
                                                "target_reached": true
                                            })),
                                        });
                                    }

                                    // Check if no new items were loaded
                                    if new_count == current_count {
                                        no_new_items_count += 1;
                                        if no_new_items_count >= 3 {
                                            info!("🛑 Infinite scroll stopped: no new items loaded for 3 consecutive cycles");
                                            return Ok(ExecutionResult::Success {
                                                payload: Some(serde_json::json!({
                                                    "items_found": new_count,
                                                    "cycles_completed": cycles + 1,
                                                    "target_reached": false,
                                                    "reason": "no_new_items"
                                                })),
                                            });
                                        }
                                    } else {
                                        no_new_items_count = 0; // Reset counter if new items were found
                                    }

                                    current_count = new_count;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("[WARNING] Count step failed in infinite scroll: {}", e);
                }
            }

            // Step 2: Scroll down to load more content
            let scroll_step = Step {
                id: format!("{}_scroll_{}", step.id, cycles),
                name: format!("Scroll down (cycle {})", cycles + 1),
                kind: StepKind::ScrollWindowTo {
                    x: None,
                    y: None,
                    direction: Some("bottom".to_string()),
                },
            };

            match self
                .broker_client
                .execute_step_and_get_dom(&scroll_step)
                .await
            {
                Ok((_, raw_dom)) => {
                    let reduced_dom = self.dom_analyzer.reduce_html(&raw_dom)?;
                    session.current_dom = reduced_dom;
                }
                Err(e) => {
                    warn!("[WARNING] Scroll step failed in infinite scroll: {}", e);
                }
            }

            // Step 3: Wait for content to load
            let wait_step = Step {
                id: format!("{}_wait_{}", step.id, cycles),
                name: format!("Wait for content (cycle {})", cycles + 1),
                kind: StepKind::WaitForTimeout {
                    timeout_ms: 1500, // Wait 1.5 seconds for content to load
                },
            };

            match self
                .broker_client
                .execute_step_and_get_dom(&wait_step)
                .await
            {
                Ok((_, raw_dom)) => {
                    let reduced_dom = self.dom_analyzer.reduce_html(&raw_dom)?;
                    session.current_dom = reduced_dom;
                }
                Err(e) => {
                    warn!("[WARNING] Wait step failed in infinite scroll: {}", e);
                }
            }

            cycles += 1;
        }

        info!(
            "🛑 Infinite scroll stopped: reached maximum cycles ({})",
            max_cycles
        );
        Ok(ExecutionResult::Success {
            payload: Some(serde_json::json!({
                "items_found": current_count,
                "cycles_completed": cycles,
                "target_reached": current_count >= target_count,
                "reason": "max_cycles_reached"
            })),
        })
    }

    /// Extract inspection data from broker response for LLM feedback
    fn extract_inspection_data_from_response(&self, response: &Value) -> Option<Value> {
        // Look for inspection data in the response - this comes from the browser extension
        // The extension collects this data during extract_structured_data operations

        // First check if inspection data is at the top level of the response
        if let Some(inspection) = response.get("inspection_data") {
            return Some(inspection.clone());
        }

        // Then check in the steps array for extraction step results
        if let Some(steps) = response.get("steps") {
            if let Some(steps_array) = steps.as_array() {
                for step_result in steps_array {
                    // Look for extract_structured_data steps
                    if let Some(step_type) = step_result.get("type") {
                        if step_type.as_str() == Some("extract_structured_data") {
                            // Check for inspection data in this step
                            if let Some(data) = step_result.get("data") {
                                // The browser extension logs inspection results in the step data
                                // Look for fields that indicate inspection was performed
                                if data.get("_inspection").is_some()
                                    || data.get("inspection_result").is_some()
                                    || data.get("discovered").is_some()
                                    || data.get("pageTitle").is_some()
                                {
                                    return Some(data.clone());
                                }

                                // Also check for debug information
                                if let Some(debug_fields) = data.as_object() {
                                    let mut inspection_info = serde_json::Map::new();
                                    let mut has_inspection_data = false;

                                    // Look for debug fields that contain page information
                                    for (key, value) in debug_fields {
                                        if key.ends_with("_debug") {
                                            inspection_info.insert(key.clone(), value.clone());
                                            has_inspection_data = true;
                                        }
                                    }

                                    if has_inspection_data {
                                        return Some(Value::Object(inspection_info));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Look for inspection data in the combined DOM response
        // The extension might include page structure information
        if let Some(html_content) = response.get("html_content") {
            if let Some(html_str) = html_content.as_str() {
                // Create basic inspection data from the page content
                let mut inspection = serde_json::Map::new();
                inspection.insert(
                    "source".to_string(),
                    Value::String("html_analysis".to_string()),
                );
                inspection.insert(
                    "html_length".to_string(),
                    Value::Number(serde_json::Number::from(html_str.len())),
                );

                // Basic DOM element counting for LLM feedback
                let div_count = html_str.matches("<div").count();
                let span_count = html_str.matches("<span").count();
                let a_count = html_str.matches("<a ").count();
                let input_count = html_str.matches("<input").count();
                let button_count = html_str.matches("<button").count();

                let mut element_counts = serde_json::Map::new();
                element_counts.insert(
                    "div".to_string(),
                    Value::Number(serde_json::Number::from(div_count)),
                );
                element_counts.insert(
                    "span".to_string(),
                    Value::Number(serde_json::Number::from(span_count)),
                );
                element_counts.insert(
                    "a".to_string(),
                    Value::Number(serde_json::Number::from(a_count)),
                );
                element_counts.insert(
                    "input".to_string(),
                    Value::Number(serde_json::Number::from(input_count)),
                );
                element_counts.insert(
                    "button".to_string(),
                    Value::Number(serde_json::Number::from(button_count)),
                );

                inspection.insert("element_counts".to_string(), Value::Object(element_counts));
                inspection.insert("message".to_string(), Value::String(
                    "Page structure analysis: Use this information to choose better selectors for extraction.".to_string()
                ));

                return Some(Value::Object(inspection));
            }
        }

        // No inspection data found
        None
    }

    ///  CRITICAL FIX: Enhanced wait for element that polls for actual content
    async fn execute_robust_wait_for_element(
        &mut self,
        step: &Step,
        session: &mut PlanningSession,
        selector: &str,
        timeout_ms: Option<u32>,
        condition: Option<&str>,
    ) -> PlanResult<ExecutionResult> {
        let timeout_duration = timeout_ms.unwrap_or(12000); // Default 12 seconds for live content
        let condition = condition.unwrap_or("visible");

        info!(
            " Robust wait for element: {} (condition: {}, timeout: {}ms)",
            selector, condition, timeout_duration
        );

        // Special handling for cricket widgets and other dynamic content
        let is_cricket_widget = selector.contains("imso-hov")
            || selector.contains("cricket")
            || selector.contains("jsname")
            || selector.contains("data-rzn-shadow");

        if is_cricket_widget {
            info!("🏏 Cricket widget detected - using enhanced polling for live score content");
        }

        let poll_interval = if is_cricket_widget { 250 } else { 500 }; // Cricket widgets need faster polling
        let max_polls = (timeout_duration as f64 / poll_interval as f64) as u32;

        for poll in 0..max_polls {
            let current_time_ms = poll * poll_interval;

            if poll > 0 {
                // Wait between polls
                tokio::time::sleep(Duration::from_millis(poll_interval as u64)).await;
            }

            info!(
                "[SEARCH] Poll {}/{} for element: {} ({}ms elapsed)",
                poll + 1,
                max_polls,
                selector,
                current_time_ms
            );

            // Create a custom step to check element state
            let check_step = Step {
                id: format!("{}_check_{}", step.id, poll),
                name: format!("Check element state (poll {})", poll + 1),
                kind: StepKind::GetElementText {
                    selector: selector.to_string(),
                    frame_id: None,
                },
            };

            match self
                .broker_client
                .execute_step_and_get_dom(&check_step)
                .await
            {
                Ok((response, raw_dom)) => {
                    // Update DOM context
                    let reduced_dom = self.dom_analyzer.reduce_html(&raw_dom)?;
                    session.current_dom = reduced_dom;

                    // Check if the step was successful and got meaningful content
                    let has_element = response
                        .get("success")
                        .and_then(|s| s.as_bool())
                        .unwrap_or(false);

                    if has_element {
                        // Check for actual text content if this is a cricket widget or dynamic content
                        let has_meaningful_content = if is_cricket_widget {
                            // For cricket widgets, check if we have actual score data
                            if let Some(steps) = response.get("steps").and_then(|s| s.as_array()) {
                                if let Some(step_result) = steps.first() {
                                    if let Some(data) = step_result.get("data") {
                                        if let Some(text) = data.as_str() {
                                            let meaningful_text = !text.trim().is_empty()
                                                && !text.trim().eq_ignore_ascii_case("loading")
                                                && !text.trim().eq_ignore_ascii_case("...")
                                                && (text.contains(char::is_numeric)
                                                    || text.contains("runs")
                                                    || text.contains("wickets")
                                                    || text.contains("over"));

                                            if meaningful_text {
                                                info!("🏏 Cricket widget has meaningful content: \"{}\"", text.trim());
                                            } else {
                                                info!("🏏 Cricket widget present but content not ready: \"{}\"", text.trim());
                                            }

                                            meaningful_text
                                        } else {
                                            false
                                        }
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        } else {
                            // For regular elements, just check that they exist and are visible
                            true
                        };

                        if has_meaningful_content {
                            info!("[OK] Robust wait successful: element found with meaningful content after {}ms", current_time_ms);

                            // Update session state with latest DOM
                            let dom_context = self
                                .dom_processor
                                .extract_dom_context(&raw_dom, &session.current_url)
                                .map_err(|e| {
                                    PlanError::DomError(format!("DOM processing failed: {}", e))
                                })?;
                            self.current_dom_context = Some(dom_context);
                            self.dom_revision += 1;

                            // Create successful execution record
                            let execution = StepExecution {
                                step: step.clone(),
                                result: ExecutionResult::Success { payload: None },
                                timestamp: chrono::Utc::now(),
                                dom_snapshot: Some(session.current_dom.clone()),
                            };
                            session.history.push(execution);

                            return Ok(ExecutionResult::Success { payload: None });
                        } else if is_cricket_widget {
                            info!("🏏 Cricket widget found but content not ready, continuing to poll...");
                        }
                    } else {
                        debug!("[SEARCH] Element not found yet: {}", selector);
                    }
                }
                Err(e) => {
                    debug!("[SEARCH] Poll failed: {}", e);
                }
            }
        }

        // Timeout reached
        let error_msg = if is_cricket_widget {
            format!("Timeout waiting for cricket widget '{}' to load meaningful content after {}ms. The widget may be present but scores are not yet available.", 
                   selector, timeout_duration)
        } else {
            format!(
                "Timeout waiting for element '{}' (condition: {}) after {}ms",
                selector, condition, timeout_duration
            )
        };

        warn!("⏰ {}", error_msg);

        // Create failed execution record
        let execution = StepExecution {
            step: step.clone(),
            result: ExecutionResult::Error {
                message: error_msg.clone(),
                retry_suggested: true,
            },
            timestamp: chrono::Utc::now(),
            dom_snapshot: Some(session.current_dom.clone()),
        };
        session.history.push(execution);

        Ok(ExecutionResult::Error {
            message: error_msg,
            retry_suggested: true,
        })
    }

    /// Verify auto-extracted data with LLM
    async fn verify_extraction_with_llm(
        &self,
        goal: &str,
        extracted_data: &Value,
        raw_text: Option<&str>,
    ) -> PlanResult<Value> {
        info!("[BOT] Verifying auto-extraction with LLM");

        let messages = vec![
            json!({
                "role": "system",
                "content": "You are a browser automation verification assistant. Your job is to verify if auto-extracted data correctly satisfies the user's goal. Be strict but reasonable in your verification."
            }),
            json!({
                "role": "user",
                "content": format!(
                    "Goal: {}\n\nExtracted Data:\n{}\n\nRaw Text (if available):\n{}\n\nPlease verify:\n1. Does this data correctly fulfill the goal?\n2. Is the data complete?\n3. Is the data accurate?\n\nRespond with JSON:\n{{\n  \"is_complete\": true/false,\n  \"is_accurate\": true/false,\n  \"verified_data\": <cleaned/formatted data>,\n  \"reasoning\": \"explanation\"\n}}",
                    goal,
                    serde_json::to_string_pretty(extracted_data).unwrap_or_else(|_| "Failed to serialize".to_string()),
                    raw_text.unwrap_or("Not provided")
                )
            }),
        ];

        match self.llm_client.chat(messages, Some(0.1)).await {
            Ok(response) => {
                // Parse LLM response
                match serde_json::from_str::<Value>(&response.content) {
                    Ok(verification) => {
                        let is_complete = verification
                            .get("is_complete")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let is_accurate = verification
                            .get("is_accurate")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        if is_complete && is_accurate {
                            info!("[OK] LLM verification passed");
                            Ok(verification
                                .get("verified_data")
                                .cloned()
                                .unwrap_or_else(|| extracted_data.clone()))
                        } else {
                            let reasoning = verification
                                .get("reasoning")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Verification failed");
                            Err(PlanError::LLMError(format!(
                                "Verification failed: {}",
                                reasoning
                            )))
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse LLM verification response: {}", e);
                        // If LLM response parsing fails, trust the auto-extraction
                        Ok(extracted_data.clone())
                    }
                }
            }
            Err(e) => {
                warn!("LLM verification request failed: {}", e);
                // If LLM fails, trust the auto-extraction
                Ok(extracted_data.clone())
            }
        }
    }

    /// Convert validated action from navigator back to Step format for execution
    fn convert_validated_action_to_step(
        &self,
        validated_action: &Value,
        step_count: usize,
    ) -> PlanResult<Step> {
        let action_name = validated_action
            .get("action")
            .and_then(|a| a.as_str())
            .ok_or_else(|| PlanError::InvalidStep("Missing action name".to_string()))?;

        // For navigate_to_url, selector might not exist
        let selector = validated_action
            .get("selector")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let frame_id_opt: Option<String> = validated_action
            .get("frame_ordinal")
            .and_then(|v| v.as_u64())
            .map(|n| n.to_string());

        let step_id = format!("snapshot_step_{}", step_count);
        let step_name = format!("Snapshot {}", action_name);

        let step_kind = match action_name {
            "click_element" => StepKind::ClickElement {
                selector: selector.to_string(),
                frame_id: frame_id_opt.clone(),
                random_offset: Some(true),
                timeout_ms: Some(5000),
            },
            "dbl_click_element" => StepKind::DblClickElement {
                selector: selector.to_string(),
                frame_id: frame_id_opt.clone(),
                random_offset: Some(true),
                timeout_ms: Some(5000),
            },
            "hover_element" => StepKind::HoverElement {
                selector: selector.to_string(),
                frame_id: frame_id_opt.clone(),
                random_offset: Some(true),
                timeout_ms: Some(5000),
            },
            "fill_input_field" => {
                let value = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("value"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        PlanError::InvalidStep("Missing value for fill_input_field".to_string())
                    })?;
                let clear_first = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("clear_first"))
                    .and_then(|v| v.as_bool());

                StepKind::FillInputField {
                    selector: selector.to_string(),
                    value: value.to_string(),
                    frame_id: frame_id_opt.clone(),
                    clear_first,
                    simulate_typing: Some(true),
                    delay_ms: Some(100),
                    timeout_ms: Some(5000),
                }
            }
            "type_text" => {
                let text = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("text").or_else(|| p.get("value")))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        PlanError::InvalidStep("Missing text for type_text".to_string())
                    })?;
                let use_native_input = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("use_native_input"))
                    .and_then(|v| v.as_bool());
                let delay_ms = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("delay_ms"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let typing_speed = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("typing_speed"))
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string());
                let timeout_ms = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("timeout_ms"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);

                StepKind::TypeText {
                    selector: selector.to_string(),
                    text: Some(text.to_string()),
                    value: None,
                    frame_id: frame_id_opt.clone(),
                    use_native_input,
                    delay_ms,
                    typing_speed,
                    timeout_ms,
                }
            }
            "submit_input" => {
                let text = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("text"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        PlanError::InvalidStep("Missing text for submit_input".to_string())
                    })?;

                StepKind::SubmitInput {
                    selector: selector.to_string(),
                    text: text.to_string(),
                    frame_id: frame_id_opt.clone(),
                }
            }
            "press_special_key" => {
                let key = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("key"))
                    .and_then(|k| k.as_str())
                    .ok_or_else(|| {
                        PlanError::InvalidStep("Missing key for press_special_key".to_string())
                    })?;

                StepKind::PressSpecialKey {
                    key: key.to_string(),
                    selector: Some(selector.to_string()),
                    frame_id: frame_id_opt.clone(),
                    timeout_ms: Some(5000),
                }
            }
            "select_option_in_dropdown" => {
                let value = validated_action
                    .get("parameters")
                    .and_then(|p| {
                        p.get("value")
                            .or_else(|| p.get("option_value"))
                            .or_else(|| p.get("option"))
                    })
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        PlanError::InvalidStep(
                            "Missing value for select_option_in_dropdown".to_string(),
                        )
                    })?;

                StepKind::SelectOptionInDropdown {
                    selector: selector.to_string(),
                    value: value.to_string(),
                    frame_id: frame_id_opt.clone(),
                    timeout_ms: Some(5000),
                }
            }
            "upload_file" => {
                let file_path = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("file_path"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        PlanError::InvalidStep("Missing file_path for upload_file".to_string())
                    })?;

                StepKind::UploadFile {
                    selector: selector.to_string(),
                    file_path: file_path.to_string(),
                    frame_id: frame_id_opt.clone(),
                    timeout_ms: Some(15_000),
                }
            }
            "drag_and_drop" => {
                let source_selector = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("source_selector"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        PlanError::InvalidStep(
                            "Missing source_selector for drag_and_drop".to_string(),
                        )
                    })?;
                let target_selector = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("target_selector"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        PlanError::InvalidStep(
                            "Missing target_selector for drag_and_drop".to_string(),
                        )
                    })?;

                StepKind::DragAndDrop {
                    source_selector: source_selector.to_string(),
                    target_selector: target_selector.to_string(),
                    frame_id: frame_id_opt.clone(),
                    timeout_ms: Some(10_000),
                }
            }
            "scroll_element_into_view" => StepKind::ScrollElementIntoView {
                selector: selector.to_string(),
                frame_id: frame_id_opt.clone(),
            },
            "scroll_window_to" => {
                let x = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("x"))
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);
                let y = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("y"))
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);
                let direction = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("direction"))
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string());

                StepKind::ScrollWindowTo { x, y, direction }
            }
            "get_element_text" => StepKind::GetElementText {
                selector: selector.to_string(),
                frame_id: frame_id_opt.clone(),
            },
            "get_element_value" => StepKind::GetElementValue {
                selector: selector.to_string(),
                frame_id: frame_id_opt.clone(),
            },
            "get_element_count" => StepKind::GetElementCount {
                selector: selector.to_string(),
                frame_id: frame_id_opt.clone(),
            },
            "get_element_attribute" => {
                let attribute = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("attribute"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        PlanError::InvalidStep(
                            "Missing attribute for get_element_attribute".to_string(),
                        )
                    })?;
                StepKind::GetElementAttribute {
                    selector: selector.to_string(),
                    attribute: attribute.to_string(),
                    frame_id: frame_id_opt.clone(),
                }
            }
            "extract_structured_data" => {
                let item_selector = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("item_selector"))
                    .and_then(|s| s.as_str())
                    .unwrap_or(selector);

                // Convert JSON fields to FieldSpec vector
                let fields = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("fields"))
                    .and_then(|f| f.as_object())
                    .map(|obj| {
                        obj.iter()
                            .map(|(name, selector_val)| rzn_core::FieldSpec {
                                name: name.clone(),
                                selector: selector_val.as_str().unwrap_or("").to_string(),
                                attribute: None,
                                post_processing: vec![],
                            })
                            .collect()
                    })
                    .unwrap_or_else(|| {
                        vec![rzn_core::FieldSpec {
                            name: "text".to_string(),
                            selector: "".to_string(),
                            attribute: None,
                            post_processing: vec![],
                        }]
                    });

                StepKind::ExtractStructuredData {
                    item_selector: item_selector.to_string(),
                    limit: None,
                    fields,
                    frame_id: frame_id_opt.clone(),
                    extraction_type: None,
                }
            }
            "navigate_to_url" => {
                // For navigate_to_url, the URL might be in parameters.url OR in the selector field
                let url = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("url"))
                    .and_then(|u| u.as_str())
                    .or_else(|| {
                        // If no parameters.url, use the selector as the URL
                        validated_action.get("selector").and_then(|s| s.as_str())
                    })
                    .ok_or_else(|| {
                        PlanError::InvalidStep("Missing url for navigate_to_url".to_string())
                    })?;

                StepKind::NavigateToUrl {
                    url: url.to_string(),
                    wait: Some("domcontentloaded".to_string()),
                }
            }
            "wait_for_element" => {
                let timeout = validated_action
                    .get("parameters")
                    .and_then(|p| {
                        p.get("timeout_ms")
                            .or_else(|| p.get("timeout"))
                            .or_else(|| p.get("timeoutMs"))
                    })
                    .and_then(|t| t.as_u64())
                    .unwrap_or(5000) as u32;

                let condition = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("condition"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("visible");

                StepKind::WaitForElement {
                    selector: selector.to_string(),
                    timeout_ms: Some(timeout),
                    condition: Some(condition.to_string()),
                    frame_id: frame_id_opt.clone(),
                }
            }
            "wait_for_timeout" => {
                let timeout_ms = validated_action
                    .get("parameters")
                    .and_then(|p| {
                        p.get("timeout_ms")
                            .or_else(|| p.get("timeout"))
                            .or_else(|| p.get("ms"))
                    })
                    .and_then(|t| t.as_u64())
                    .unwrap_or(1000) as u32;

                StepKind::WaitForTimeout { timeout_ms }
            }
            "wait_for_navigation" => {
                let url_pattern = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("url_pattern"))
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string());
                let timeout_ms = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("timeout_ms"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);

                StepKind::WaitForNavigation {
                    url_pattern,
                    timeout_ms,
                }
            }
            "wait_for_network_idle" => {
                let idle_time_ms = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("idle_time_ms"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
                    .unwrap_or(800);
                let max_wait_ms = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("max_wait_ms"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
                    .unwrap_or(10_000);

                StepKind::WaitForNetworkIdle {
                    idle_time_ms,
                    max_wait_ms,
                }
            }
            "take_screenshot" => {
                let full_page = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("full_page"))
                    .and_then(|v| v.as_bool());
                let annotate = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("annotate"))
                    .and_then(|v| v.as_bool());
                let annotate_max_labels = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("annotate_max_labels"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let annotate_max_elements = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("annotate_max_elements"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let quality = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("quality"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u8);
                let format = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("format"))
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string());

                StepKind::TakeScreenshot {
                    full_page,
                    annotate,
                    annotate_max_labels,
                    annotate_max_elements,
                    quality,
                    format,
                }
            }
            "assert_selector_state" => {
                let condition = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("condition"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        PlanError::InvalidStep(
                            "Missing condition for assert_selector_state".to_string(),
                        )
                    })?;

                StepKind::AssertSelectorState {
                    selector: selector.to_string(),
                    condition: condition.to_string(),
                    frame_id: frame_id_opt.clone(),
                }
            }
            "assert_text_in_element" => {
                let text = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("text"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        PlanError::InvalidStep(
                            "Missing text for assert_text_in_element".to_string(),
                        )
                    })?;

                let match_type = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("match_type"))
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string());

                StepKind::AssertTextInElement {
                    selector: selector.to_string(),
                    text: text.to_string(),
                    frame_id: frame_id_opt.clone(),
                    match_type,
                }
            }
            "assert_url_matches" => {
                let url_pattern = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("url_pattern"))
                    .or_else(|| validated_action.get("url_pattern"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        PlanError::InvalidStep(
                            "Missing url_pattern for assert_url_matches".to_string(),
                        )
                    })?;

                let match_type = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("match_type"))
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string());

                StepKind::AssertUrlMatches {
                    url_pattern: url_pattern.to_string(),
                    match_type,
                }
            }
            "get_current_url" => StepKind::GetCurrentUrl,
            "open_new_tab" => {
                let url = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("url"))
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string());
                StepKind::OpenNewTab { url }
            }
            "switch_to_tab" => {
                let tab_identifier = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("tab_identifier"))
                    .cloned()
                    .ok_or_else(|| {
                        PlanError::InvalidStep(
                            "Missing tab_identifier for switch_to_tab".to_string(),
                        )
                    })?;
                StepKind::SwitchToTab { tab_identifier }
            }
            "close_current_tab" => {
                let tab_identifier = validated_action
                    .get("parameters")
                    .and_then(|p| p.get("tab_identifier"))
                    .cloned()
                    .ok_or_else(|| {
                        PlanError::InvalidStep(
                            "Missing tab_identifier for close_current_tab".to_string(),
                        )
                    })?;
                StepKind::CloseCurrentTab { tab_identifier }
            }
            "go_back" => StepKind::ExecuteJavascript {
                script: "history.back();".to_string(),
                args: None,
                return_value: false,
                world: None,
                timeout_ms: None,
            },
            "go_forward" => StepKind::ExecuteJavascript {
                script: "history.forward();".to_string(),
                args: None,
                return_value: false,
                world: None,
                timeout_ms: None,
            },
            "refresh_page" | "reload_page" => StepKind::ExecuteJavascript {
                script: "location.reload();".to_string(),
                args: None,
                return_value: false,
                world: None,
                timeout_ms: None,
            },
            _ => {
                return Err(PlanError::InvalidStep(format!(
                    "Unsupported action: {}",
                    action_name
                )));
            }
        };

        Ok(Step {
            id: step_id,
            name: step_name,
            kind: step_kind,
        })
    }

    // New CDP-based architecture methods

    /// Execute step with TargetSpec and mode selection
    pub async fn execute_step_with_mode_selection(
        &mut self,
        step: &Step,
        target: Option<&TargetSpec>,
        session: &mut PlanningSession,
    ) -> PlanResult<(ExecutionResult, String)> {
        // Policy gate for desktop/agent-driven actions
        self.policy_gate
            .enforce_step(step, target, Some(&session.current_url))
            .await?;

        // Extract domain from current URL
        let domain = self.extract_domain_from_url(&session.current_url);
        if self.current_domain.as_ref() != Some(&domain) {
            self.current_domain = Some(domain.clone());
            self.mode_selector.reset_to_default();
        }

        // Determine action type
        let action_type = self.get_action_type_from_step(step);

        // If we have a TargetSpec, use it for mode selection
        let target_spec = if let Some(target) = target {
            target.clone()
        } else {
            // Create TargetSpec from step selectors
            self.extract_target_spec_from_step(step)
        };

        // Select execution mode
        let selected_mode = self
            .mode_selector
            .select_mode(&domain, &action_type, &target_spec);
        info!(
            "Selected execution mode: {:?} for action {} on {}",
            selected_mode, action_type, domain
        );

        // Ensure CDP is attached if Pro mode is selected
        if selected_mode == ExecutionMode::Pro && !self.broker_client.is_pro_mode_available() {
            info!("Attaching CDP for Pro mode execution");
            if let Err(e) = self.broker_client.attach_cdp().await {
                warn!("CDP attachment failed, escalating: {:?}", e);
                self.mode_selector
                    .escalate_to_pro(EscalationReason::ActionFailure);
                return Err(e);
            }
        }

        // Execute step with selected mode capabilities
        let result = if target.is_some() {
            self.execute_step_with_target_spec(step, &target_spec, session)
                .await
        } else {
            self.execute_step_legacy_with_tracking(step, session).await
        };

        // Record execution result for future mode selection
        match &result {
            Ok((execution_result, _)) => {
                let execution_time = match &execution_result {
                    ExecutionResult::Success { payload } => payload
                        .as_ref()
                        .and_then(|p| p.get("execution_time_ms"))
                        .and_then(|t| t.as_u64())
                        .unwrap_or(0),
                    ExecutionResult::Error { .. } => 0,
                };
                let result_envelope = ResultEnvelope::success(
                    execution_result.clone(),
                    if selected_mode == ExecutionMode::Pro {
                        InputRung::Cdp
                    } else {
                        InputRung::Scripted
                    },
                    execution_time,
                );
                self.mode_selector
                    .record_result(&domain, &action_type, &result_envelope);
            }
            Err(_) => {
                let result_envelope = ResultEnvelope {
                    result: ExecutionResult::Error {
                        message: "Step execution failed".to_string(),
                        retry_suggested: true,
                    },
                    rung_used: if selected_mode == ExecutionMode::Pro {
                        InputRung::Cdp
                    } else {
                        InputRung::Scripted
                    },
                    escalated: false,
                    success: false,
                    error: Some("Step execution failed".to_string()),
                    execution_time_ms: 0,
                    resolved_element: None,
                };
                self.mode_selector
                    .record_result(&domain, &action_type, &result_envelope);

                // If Light mode failed, try escalating to Pro mode
                if selected_mode == ExecutionMode::Light {
                    warn!("Light mode execution failed, attempting Pro mode escalation");
                    self.mode_selector
                        .escalate_to_pro(EscalationReason::ActionFailure);

                    if let Ok(_) = self.broker_client.attach_cdp().await {
                        info!("Successfully escalated to Pro mode, retrying step");
                        return self
                            .execute_step_with_target_spec(step, &target_spec, session)
                            .await;
                    }
                }
            }
        }

        result
    }

    /// Execute step with TargetSpec (Pro mode)
    async fn execute_step_with_target_spec(
        &mut self,
        step: &Step,
        target: &TargetSpec,
        _session: &mut PlanningSession,
    ) -> PlanResult<(ExecutionResult, String)> {
        info!("Executing step with TargetSpec: {:?}", target);

        // Try to resolve the target first if it has an encoded_id
        let _resolved_element = if target.encoded_id.is_some() {
            match self.broker_client.resolve_target(target).await {
                Ok(element) => {
                    info!("Successfully resolved element: {}", element.encoded_id);
                    Some(element)
                }
                Err(e) => {
                    warn!("Failed to resolve target: {:?}", e);
                    None
                }
            }
        } else {
            None
        };

        // Execute step with TargetSpec
        match self
            .broker_client
            .execute_step_with_target(step, target)
            .await
        {
            Ok(envelope) => {
                let execution_result = if envelope.success {
                    ExecutionResult::Success {
                        payload: Some(json!({
                            "rung_used": envelope.rung_used as u8,
                            "escalated": envelope.escalated,
                            "execution_time_ms": envelope.execution_time_ms,
                            "resolved_element": envelope.resolved_element
                        })),
                    }
                } else {
                    ExecutionResult::Error {
                        message: envelope.error.unwrap_or("Unknown error".to_string()),
                        retry_suggested: envelope.rung_used != InputRung::Cdp, // Suggest retry if not using highest rung
                    }
                };

                // Extract DOM content from result
                let raw_dom = envelope
                    .result
                    .get("html_content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                info!(
                    "Step executed successfully with rung {}, escalated: {}",
                    envelope.rung_used as u8, envelope.escalated
                );

                Ok((execution_result, raw_dom))
            }
            Err(e) => {
                error!("Step execution with TargetSpec failed: {:?}", e);
                Err(e)
            }
        }
    }

    /// Legacy step execution with result tracking
    async fn execute_step_legacy_with_tracking(
        &mut self,
        step: &Step,
        session: &mut PlanningSession,
    ) -> PlanResult<(ExecutionResult, String)> {
        // Use existing execute_step method but track results
        let result = self.execute_step(step, session).await;

        // The result tracking is handled in execute_step_with_mode_selection
        result
    }

    /// Extract domain from URL
    fn extract_domain_from_url(&self, url: &str) -> String {
        url::Url::parse(url)
            .map(|parsed| parsed.host_str().unwrap_or("unknown").to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    }

    /// Get action type string from step
    fn get_action_type_from_step(&self, step: &Step) -> String {
        match &step.kind {
            StepKind::NavigateToUrl { .. } => "navigate_to_url".to_string(),
            StepKind::ClickElement { .. } => "click_element".to_string(),
            StepKind::FillInputField { .. } => "fill_input_field".to_string(),
            StepKind::TypeText { .. } => "type_text".to_string(),
            StepKind::SubmitInput { .. } => "submit_input".to_string(),
            StepKind::PressSpecialKey { .. } => "press_special_key".to_string(),
            StepKind::WaitForElement { .. } => "wait_for_element".to_string(),
            StepKind::GetElementText { .. } => "get_element_text".to_string(),
            StepKind::ExtractStructuredData { .. } => "extract_structured_data".to_string(),
            StepKind::InfiniteScroll { .. } => "infinite_scroll".to_string(),
            _ => "other".to_string(),
        }
    }

    /// Extract TargetSpec from step selectors
    fn extract_target_spec_from_step(&self, step: &Step) -> TargetSpec {
        match &step.kind {
            StepKind::ClickElement {
                selector, frame_id, ..
            } => {
                let mut target = TargetSpec::from_css(selector.clone());
                if let Some(frame_str) = frame_id {
                    if let Ok(frame_ordinal) = frame_str.parse::<u32>() {
                        target = target.with_frame(frame_ordinal);
                    }
                }
                target
            }
            StepKind::FillInputField {
                selector, frame_id, ..
            } => {
                let mut target = TargetSpec::from_css(selector.clone());
                if let Some(frame_str) = frame_id {
                    if let Ok(frame_ordinal) = frame_str.parse::<u32>() {
                        target = target.with_frame(frame_ordinal);
                    }
                }
                target
            }
            StepKind::TypeText {
                selector, frame_id, ..
            } => {
                let mut target = TargetSpec::from_css(selector.clone());
                if let Some(frame_str) = frame_id {
                    if let Ok(frame_ordinal) = frame_str.parse::<u32>() {
                        target = target.with_frame(frame_ordinal);
                    }
                }
                target
            }
            StepKind::WaitForElement {
                selector, frame_id, ..
            } => {
                let mut target = TargetSpec::from_css(selector.clone());
                if let Some(frame_str) = frame_id {
                    if let Ok(frame_ordinal) = frame_str.parse::<u32>() {
                        target = target.with_frame(frame_ordinal);
                    }
                }
                target
            }
            StepKind::GetElementText {
                selector, frame_id, ..
            } => {
                let mut target = TargetSpec::from_css(selector.clone());
                if let Some(frame_str) = frame_id {
                    if let Ok(frame_ordinal) = frame_str.parse::<u32>() {
                        target = target.with_frame(frame_ordinal);
                    }
                }
                target
            }
            StepKind::ExtractStructuredData {
                item_selector,
                frame_id,
                ..
            } => {
                let mut target = TargetSpec::from_css(item_selector.clone());
                if let Some(frame_str) = frame_id {
                    if let Ok(frame_ordinal) = frame_str.parse::<u32>() {
                        target = target.with_frame(frame_ordinal);
                    }
                }
                target
            }
            _ => {
                // For steps without selectors, create an empty TargetSpec
                TargetSpec {
                    encoded_id: None,
                    css: None,
                    xpath: None,
                    role_name: None,
                    text_near: None,
                    frame_ordinal: None,
                }
            }
        }
    }

    /// Get mode selection statistics
    pub fn get_mode_statistics(&self) -> ModeStatistics {
        self.mode_selector.get_statistics()
    }

    /// Force mode selection (for testing/debugging)
    pub fn force_execution_mode(&mut self, mode: ExecutionMode) {
        self.mode_selector.set_default_mode(mode);
        info!("Forced execution mode to: {:?}", mode);
    }

    /// Get current execution mode
    pub fn get_current_execution_mode(&self) -> ExecutionMode {
        self.mode_selector.current_mode()
    }

    /// Manually escalate to Pro mode
    pub async fn escalate_to_pro_mode(&mut self, reason: EscalationReason) -> PlanResult<()> {
        self.mode_selector.escalate_to_pro(reason);
        if !self.broker_client.is_pro_mode_available() {
            self.broker_client.attach_cdp().await?;
        }
        Ok(())
    }

    /// Reset mode selector (call at workflow start)
    pub fn reset_mode_selector(&mut self) {
        self.mode_selector.reset_to_default();
        self.current_domain = None;
        self.resolved_elements.clear();
    }
}
