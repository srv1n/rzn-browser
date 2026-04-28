//! Comprehensive telemetry and cost tracking system for autonomous sessions
//!
//! This module provides:
//! - Real-time cost tracking with per-model rates
//! - Action execution statistics and success rates
//! - Streaming JSONL trace files for debugging
//! - Replay functionality for analyzing past sessions
//! - Budget alerts and usage monitoring

use crate::{ExecutionResult, PlanError, PlanResult};
use chrono::{DateTime, Local, Utc};
use log::{debug, error, info, warn};
use rzn_core::{Step, StepKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

/// Main telemetry collector that tracks everything about an autonomous session
#[derive(Debug)]
pub struct TelemetryCollector {
    /// Unique session ID
    pub session_id: String,
    /// All traces for this session
    traces: Vec<AutonomousTrace>,
    /// Cost tracking across all LLM calls
    cost_tracker: CostTracker,
    /// Statistics per action type
    action_stats: HashMap<String, ActionStats>,
    /// Writer for streaming JSONL output
    session_writer: TraceWriter,
    /// Session start time
    session_start: DateTime<Utc>,
    /// Goal being pursued
    goal: String,
    /// Starting URL
    start_url: Option<String>,
}

/// Individual trace entry for a single step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousTrace {
    /// Session identifier
    pub session_id: String,
    /// Step sequence number within session
    pub step_number: u32,
    /// Timestamp when step started
    pub timestamp: DateTime<Utc>,
    /// The step that was executed
    pub step: Step,
    /// Execution result
    pub result: ExecutionResult,
    /// Time taken to execute (milliseconds)
    pub execution_time_ms: u64,
    /// Number of retries attempted
    pub retry_count: u32,
    /// DOM snapshot before execution (optional, can be large)
    pub dom_snapshot: Option<String>,
    /// Screenshot path (if taken)
    pub screenshot_path: Option<String>,
    /// LLM usage for this step (if applicable)
    pub llm_usage: Option<LLMUsage>,
    /// Cost incurred for this step
    pub step_cost: f64,
    /// Error category if step failed
    pub error_category: Option<String>,
    /// Recovery strategy applied (if any)  
    pub recovery_strategy: Option<String>,
}

/// LLM usage statistics for a single call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMUsage {
    /// Model used (e.g., "gpt-4o", "claude-3-haiku")
    pub model: String,
    /// Prompt tokens consumed
    pub prompt_tokens: u32,
    /// Completion tokens generated
    pub completion_tokens: u32,
    /// Total tokens (prompt + completion)
    pub total_tokens: u32,
    /// Provider-reported cost (if available)
    pub provider_cost: Option<f64>,
    /// Our estimated cost based on rates
    pub estimated_cost: f64,
    /// Request duration in milliseconds
    pub duration_ms: u64,
}

/// Cost tracking with per-model rates and running totals
#[derive(Debug, Clone)]
pub struct CostTracker {
    /// Cost rates per model
    model_rates: HashMap<String, CostRate>,
    /// Total accumulated cost across all models
    total_cost: f64,
    /// Cost breakdown by model
    cost_by_model: HashMap<String, f64>,
    /// Token usage by model
    token_usage: HashMap<String, TokenUsage>,
    /// Budget limit (if set)
    budget_limit: Option<f64>,
    /// Cost alerts configuration
    alert_thresholds: Vec<f64>,
    /// Alerts already fired (to avoid spam)
    fired_alerts: Vec<f64>,
}

/// Per-model pricing information (November 2024 rates)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRate {
    /// Model identifier
    pub model: String,
    /// USD per 1K prompt tokens
    pub usd_per_1k_prompt: f64,
    /// USD per 1K completion tokens
    pub usd_per_1k_completion: f64,
    /// Provider name
    pub provider: String,
}

/// Token usage statistics per model
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_requests: u32,
}

/// Action execution statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActionStats {
    /// Successful DOM-based executions
    pub dom_success: u32,
    /// Failed DOM-based executions
    pub dom_failure: u32,
    /// Successful native input executions
    pub native_success: u32,
    /// Failed native input executions
    pub native_failure: u32,
    /// Average execution time in milliseconds
    pub avg_execution_time_ms: f64,
    /// Average retry count
    pub avg_retry_count: f32,
    /// Total execution attempts
    pub total_attempts: u32,
    /// Success rate (0.0 to 1.0)
    pub success_rate: f64,
}

/// Writes trace entries to JSONL files
#[derive(Debug)]
pub struct TraceWriter {
    /// Output file handle
    file: Option<File>,
    /// Path to the trace file
    file_path: PathBuf,
    /// Number of entries written
    entries_written: u64,
}

/// Session summary for replay and analysis
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub goal: String,
    pub start_url: Option<String>,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub total_steps: u32,
    pub successful_steps: u32,
    pub failed_steps: u32,
    pub total_cost: f64,
    pub cost_breakdown: HashMap<String, f64>,
    pub action_stats: HashMap<String, ActionStats>,
    pub final_result: Option<serde_json::Value>,
    pub error_summary: Option<String>,
}

/// Configuration for telemetry collection
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Enable telemetry collection
    pub enabled: bool,
    /// Base directory for trace files (default: ~/rzn_traces)
    pub traces_dir: PathBuf,
    /// Include DOM snapshots in traces
    pub include_dom_snapshots: bool,
    /// Include screenshots in traces
    pub include_screenshots: bool,
    /// Budget limit in USD
    pub budget_limit: Option<f64>,
    /// Cost alert thresholds in USD
    pub alert_thresholds: Vec<f64>,
    /// Async write buffer size
    pub write_buffer_size: usize,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

        Self {
            enabled: true,
            traces_dir: home_dir.join("rzn_traces"),
            include_dom_snapshots: false, // Disabled by default due to size
            include_screenshots: true,
            budget_limit: None,
            alert_thresholds: vec![1.0, 5.0, 10.0, 25.0, 50.0], // Alert at $1, $5, $10, $25, $50
            write_buffer_size: 8192,
        }
    }
}

impl TelemetryCollector {
    /// Create new telemetry collector for a session
    pub async fn new(
        goal: String,
        start_url: Option<String>,
        config: TelemetryConfig,
    ) -> PlanResult<Self> {
        let session_id = Uuid::new_v4().to_string();
        let session_start = Utc::now();

        info!("Starting telemetry collection for session: {}", session_id);

        // Create traces directory structure: ~/rzn_traces/YYYY-MM/
        let month_dir = config
            .traces_dir
            .join(session_start.format("%Y-%m").to_string());
        fs::create_dir_all(&month_dir)
            .await
            .map_err(|e| PlanError::IoError(e))?;

        // Initialize trace writer
        let trace_file = month_dir.join(format!("{}.jsonl", session_id));
        let session_writer = TraceWriter::new(trace_file).await?;

        // Initialize cost tracker with November 2024 rates
        let cost_tracker = CostTracker::new_with_default_rates(
            config.budget_limit,
            config.alert_thresholds.clone(),
        );

        Ok(Self {
            session_id: session_id.clone(),
            traces: Vec::new(),
            cost_tracker,
            action_stats: HashMap::new(),
            session_writer,
            session_start,
            goal,
            start_url,
        })
    }

    /// Record a step execution with timing and cost information
    pub async fn record_step(
        &mut self,
        step: Step,
        result: ExecutionResult,
        execution_time_ms: u64,
        retry_count: u32,
        dom_snapshot: Option<String>,
        screenshot_path: Option<String>,
        llm_usage: Option<LLMUsage>,
    ) -> PlanResult<()> {
        let step_number = self.traces.len() as u32 + 1;

        // Calculate step cost
        let step_cost = if let Some(ref usage) = llm_usage {
            self.cost_tracker.calculate_cost(
                &usage.model,
                usage.prompt_tokens,
                usage.completion_tokens,
            )
        } else {
            0.0
        };

        // Update cost tracking
        if let Some(ref usage) = llm_usage {
            self.cost_tracker.add_usage(usage).await?;
        }

        // Determine error category and recovery strategy
        let (error_category, recovery_strategy) = match &result {
            ExecutionResult::Error { message, .. } => {
                let category = self.categorize_error(message);
                let strategy = self.suggest_recovery_strategy(&category);
                (Some(category), strategy)
            }
            ExecutionResult::Success { .. } => (None, None),
        };

        // Update action statistics
        self.update_action_stats(&step, &result, execution_time_ms, retry_count);

        // Create trace entry
        let trace = AutonomousTrace {
            session_id: self.session_id.clone(),
            step_number,
            timestamp: Utc::now(),
            step: step.clone(),
            result: result.clone(),
            execution_time_ms,
            retry_count,
            dom_snapshot,
            screenshot_path,
            llm_usage,
            step_cost,
            error_category,
            recovery_strategy,
        };

        // Write to JSONL file immediately (streaming)
        self.session_writer.write_trace(&trace).await?;

        // Store in memory for session analysis
        self.traces.push(trace.clone());

        info!(
            "Recorded step {}: {} ({}ms, {} retries, ${:.4})",
            step_number, step.name, execution_time_ms, retry_count, step_cost
        );

        // Check for cost alerts
        self.cost_tracker.check_alerts();

        Ok(())
    }

    /// Record LLM usage without step execution (e.g., planning calls)
    pub async fn record_llm_usage(&mut self, usage: LLMUsage) -> PlanResult<()> {
        self.cost_tracker.add_usage(&usage).await?;

        debug!(
            "Recorded LLM usage: {} - {} tokens, ${:.4}",
            usage.model, usage.total_tokens, usage.estimated_cost
        );

        self.cost_tracker.check_alerts();
        Ok(())
    }

    /// Get current session cost
    pub fn get_total_cost(&self) -> f64 {
        self.cost_tracker.total_cost
    }

    /// Get cost breakdown by model
    pub fn get_cost_breakdown(&self) -> HashMap<String, f64> {
        self.cost_tracker.cost_by_model.clone()
    }

    /// Get action statistics
    pub fn get_action_stats(&self) -> HashMap<String, ActionStats> {
        self.action_stats.clone()
    }

    /// Get session summary
    pub fn get_session_summary(&self, final_result: Option<serde_json::Value>) -> SessionSummary {
        let successful_steps = self
            .traces
            .iter()
            .filter(|t| matches!(t.result, ExecutionResult::Success { .. }))
            .count() as u32;

        let failed_steps = self.traces.len() as u32 - successful_steps;

        let error_summary = if failed_steps > 0 {
            let errors: Vec<String> = self
                .traces
                .iter()
                .filter_map(|t| match &t.result {
                    ExecutionResult::Error { message, .. } => Some(message.clone()),
                    _ => None,
                })
                .collect();
            Some(format!("{} errors: {}", errors.len(), errors.join("; ")))
        } else {
            None
        };

        SessionSummary {
            session_id: self.session_id.clone(),
            goal: self.goal.clone(),
            start_url: self.start_url.clone(),
            start_time: self.session_start,
            end_time: Some(Utc::now()),
            total_steps: self.traces.len() as u32,
            successful_steps,
            failed_steps,
            total_cost: self.cost_tracker.total_cost,
            cost_breakdown: self.cost_tracker.cost_by_model.clone(),
            action_stats: self.action_stats.clone(),
            final_result,
            error_summary,
        }
    }

    /// Finalize session and write summary
    pub async fn finalize_session(
        &mut self,
        final_result: Option<serde_json::Value>,
    ) -> PlanResult<SessionSummary> {
        let summary = self.get_session_summary(final_result);

        // Write session summary to separate file
        let summary_path = self.session_writer.file_path.with_extension("summary.json");
        let summary_json =
            serde_json::to_string_pretty(&summary).map_err(|e| PlanError::SerializationError(e))?;

        fs::write(summary_path, summary_json)
            .await
            .map_err(|e| PlanError::IoError(e))?;

        // Close trace writer
        self.session_writer.close().await?;

        info!(
            "Session {} completed: {} steps, ${:.4} total cost",
            self.session_id, summary.total_steps, summary.total_cost
        );

        Ok(summary)
    }

    /// Update action statistics
    fn update_action_stats(
        &mut self,
        step: &Step,
        result: &ExecutionResult,
        execution_time_ms: u64,
        retry_count: u32,
    ) {
        let action_type = self.get_action_type(&step.kind);
        let stats = self.action_stats.entry(action_type).or_default();

        stats.total_attempts += 1;

        match result {
            ExecutionResult::Success { .. } => {
                // For now, we'll classify all successes as DOM-based
                // In practice, you'd need to track whether native input was used
                stats.dom_success += 1;
            }
            ExecutionResult::Error { .. } => {
                stats.dom_failure += 1;
            }
        }

        // Update averages
        let total_time = stats.avg_execution_time_ms * (stats.total_attempts - 1) as f64
            + execution_time_ms as f64;
        stats.avg_execution_time_ms = total_time / stats.total_attempts as f64;

        let total_retries =
            stats.avg_retry_count * (stats.total_attempts - 1) as f32 + retry_count as f32;
        stats.avg_retry_count = total_retries / stats.total_attempts as f32;

        // Update success rate
        let total_successes = stats.dom_success + stats.native_success;
        stats.success_rate = total_successes as f64 / stats.total_attempts as f64;
    }

    /// Get action type string from StepKind
    fn get_action_type(&self, kind: &StepKind) -> String {
        match kind {
            StepKind::NavigateToUrl { .. } => "navigate".to_string(),
            StepKind::ClickElement { .. } => "click".to_string(),
            StepKind::FillInputField { .. } => "fill".to_string(),
            StepKind::PressSpecialKey { .. } => "keypress".to_string(),
            StepKind::HoverElement { .. } => "hover".to_string(),
            StepKind::WaitForElement { .. } | StepKind::WaitForTimeout { .. } => "wait".to_string(),
            StepKind::ExtractStructuredData { .. } => "extract".to_string(),
            StepKind::ScrollWindowTo { .. } | StepKind::ScrollElementIntoView { .. } => {
                "scroll".to_string()
            }
            StepKind::TakeScreenshot { .. } => "screenshot".to_string(),
            _ => "other".to_string(),
        }
    }

    /// Categorize error for analytics
    fn categorize_error(&self, message: &str) -> String {
        let msg = message.to_lowercase();

        if msg.contains("timeout") || msg.contains("timed out") {
            "timeout".to_string()
        } else if msg.contains("not found") || msg.contains("no such element") {
            "element_not_found".to_string()
        } else if msg.contains("click") && msg.contains("intercepted") {
            "element_intercepted".to_string()
        } else if msg.contains("frame") || msg.contains("iframe") {
            "frame_error".to_string()
        } else if msg.contains("network") || msg.contains("connection") {
            "network_error".to_string()
        } else if msg.contains("permission") || msg.contains("denied") {
            "permission_error".to_string()
        } else {
            "unknown".to_string()
        }
    }

    /// Suggest recovery strategy based on error category
    fn suggest_recovery_strategy(&self, category: &str) -> Option<String> {
        match category {
            "timeout" => Some("increase_timeout".to_string()),
            "element_not_found" => Some("retry_with_fallback_selector".to_string()),
            "element_intercepted" => Some("scroll_into_view_then_click".to_string()),
            "frame_error" => Some("switch_frame_context".to_string()),
            "network_error" => Some("retry_after_delay".to_string()),
            _ => None,
        }
    }
}

impl CostTracker {
    /// Create new cost tracker with default November 2024 rates
    pub fn new_with_default_rates(budget_limit: Option<f64>, alert_thresholds: Vec<f64>) -> Self {
        let mut model_rates = HashMap::new();

        // November 2024 pricing (per 1M tokens)
        // GPT-4o
        model_rates.insert(
            "gpt-4o".to_string(),
            CostRate {
                model: "gpt-4o".to_string(),
                usd_per_1k_prompt: 2.50,
                usd_per_1k_completion: 10.00,
                provider: "openai".to_string(),
            },
        );

        // GPT-4o-mini
        model_rates.insert(
            "o4-mini".to_string(),
            CostRate {
                model: "o4-mini".to_string(),
                usd_per_1k_prompt: 0.15,
                usd_per_1k_completion: 0.60,
                provider: "openai".to_string(),
            },
        );

        // GPT-3.5-turbo
        model_rates.insert(
            "gpt-3.5-turbo".to_string(),
            CostRate {
                model: "gpt-3.5-turbo".to_string(),
                usd_per_1k_prompt: 0.50,
                usd_per_1k_completion: 1.50,
                provider: "openai".to_string(),
            },
        );

        // Claude 3 Haiku
        model_rates.insert(
            "claude-3-haiku".to_string(),
            CostRate {
                model: "claude-3-haiku".to_string(),
                usd_per_1k_prompt: 0.25,
                usd_per_1k_completion: 1.25,
                provider: "anthropic".to_string(),
            },
        );

        // Claude 3.5 Sonnet
        model_rates.insert(
            "claude-3-5-sonnet".to_string(),
            CostRate {
                model: "claude-3-5-sonnet".to_string(),
                usd_per_1k_prompt: 3.00,
                usd_per_1k_completion: 15.00,
                provider: "anthropic".to_string(),
            },
        );

        // Gemini 1.5 Flash
        model_rates.insert(
            "gemini-2.0-flash".to_string(),
            CostRate {
                model: "gemini-2.0-flash".to_string(),
                usd_per_1k_prompt: 0.075,
                usd_per_1k_completion: 0.30,
                provider: "google".to_string(),
            },
        );

        // Gemini 1.5 Pro
        model_rates.insert(
            "gemini-1.5-pro".to_string(),
            CostRate {
                model: "gemini-1.5-pro".to_string(),
                usd_per_1k_prompt: 1.25,
                usd_per_1k_completion: 5.00,
                provider: "google".to_string(),
            },
        );

        Self {
            model_rates,
            total_cost: 0.0,
            cost_by_model: HashMap::new(),
            token_usage: HashMap::new(),
            budget_limit,
            alert_thresholds,
            fired_alerts: Vec::new(),
        }
    }

    /// Calculate cost for a given model and token usage
    pub fn calculate_cost(&self, model: &str, prompt_tokens: u32, completion_tokens: u32) -> f64 {
        if let Some(rate) = self.model_rates.get(model) {
            let prompt_cost = (prompt_tokens as f64 / 1000.0) * rate.usd_per_1k_prompt;
            let completion_cost = (completion_tokens as f64 / 1000.0) * rate.usd_per_1k_completion;
            prompt_cost + completion_cost
        } else {
            warn!("Unknown model for cost calculation: {}", model);
            // Fallback to GPT-4o rates for unknown models
            let fallback_rate = self.model_rates.get("gpt-4o").unwrap();
            let prompt_cost = (prompt_tokens as f64 / 1000.0) * fallback_rate.usd_per_1k_prompt;
            let completion_cost =
                (completion_tokens as f64 / 1000.0) * fallback_rate.usd_per_1k_completion;
            prompt_cost + completion_cost
        }
    }

    /// Add LLM usage and update costs
    pub async fn add_usage(&mut self, usage: &LLMUsage) -> PlanResult<()> {
        // Use estimated cost from usage if available, otherwise calculate
        let cost = if usage.estimated_cost > 0.0 {
            usage.estimated_cost
        } else {
            self.calculate_cost(&usage.model, usage.prompt_tokens, usage.completion_tokens)
        };

        // Update totals
        self.total_cost += cost;
        *self.cost_by_model.entry(usage.model.clone()).or_insert(0.0) += cost;

        // Update token usage
        let token_stats = self.token_usage.entry(usage.model.clone()).or_default();
        token_stats.total_prompt_tokens += usage.prompt_tokens as u64;
        token_stats.total_completion_tokens += usage.completion_tokens as u64;
        token_stats.total_requests += 1;

        debug!(
            "Updated costs: {} +${:.4} = ${:.4} total",
            usage.model, cost, self.total_cost
        );

        // Check for cost alerts
        self.check_alerts();

        Ok(())
    }

    /// Check if any cost alerts should be fired
    pub fn check_alerts(&mut self) {
        for &threshold in &self.alert_thresholds {
            if self.total_cost >= threshold && !self.fired_alerts.contains(&threshold) {
                warn!(
                    " COST ALERT: Session cost has reached ${:.2} (threshold: ${:.2})",
                    self.total_cost, threshold
                );
                self.fired_alerts.push(threshold);
            }
        }

        if let Some(budget) = self.budget_limit {
            if self.total_cost >= budget {
                error!(
                    "🛑 BUDGET EXCEEDED: Session cost ${:.2} exceeds budget ${:.2}",
                    self.total_cost, budget
                );
            }
        }
    }

    /// Get detailed cost report
    pub fn get_cost_report(&self) -> serde_json::Value {
        serde_json::json!({
            "total_cost": self.total_cost,
            "cost_by_model": self.cost_by_model,
            "token_usage": self.token_usage,
            "budget_limit": self.budget_limit,
            "budget_remaining": self.budget_limit.map(|b| b - self.total_cost),
            "fired_alerts": self.fired_alerts,
        })
    }
}

impl TraceWriter {
    /// Create new trace writer for a session
    pub async fn new(file_path: PathBuf) -> PlanResult<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await
            .map_err(|e| PlanError::IoError(e))?;

        info!("Created trace writer: {}", file_path.display());

        Ok(Self {
            file: Some(file),
            file_path,
            entries_written: 0,
        })
    }

    /// Write a trace entry to JSONL file
    pub async fn write_trace(&mut self, trace: &AutonomousTrace) -> PlanResult<()> {
        if let Some(ref mut file) = self.file {
            let json_line =
                serde_json::to_string(trace).map_err(|e| PlanError::SerializationError(e))?;

            file.write_all(json_line.as_bytes())
                .await
                .map_err(|e| PlanError::IoError(e))?;
            file.write_all(b"\n")
                .await
                .map_err(|e| PlanError::IoError(e))?;
            file.flush().await.map_err(|e| PlanError::IoError(e))?;

            self.entries_written += 1;

            if self.entries_written % 10 == 0 {
                debug!(
                    "Written {} trace entries to {}",
                    self.entries_written,
                    self.file_path.display()
                );
            }
        }

        Ok(())
    }

    /// Close the trace writer
    pub async fn close(&mut self) -> PlanResult<()> {
        if let Some(file) = self.file.take() {
            drop(file); // Async close
            info!(
                "Closed trace writer: {} ({} entries)",
                self.file_path.display(),
                self.entries_written
            );
        }
        Ok(())
    }
}

/// Replay functionality for analyzing past sessions
pub struct TraceReplay {
    traces_dir: PathBuf,
}

impl TraceReplay {
    /// Create new replay analyzer
    pub fn new(traces_dir: Option<PathBuf>) -> Self {
        let dir = traces_dir.unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("rzn_traces")
        });

        Self { traces_dir: dir }
    }

    /// List all available sessions
    pub async fn list_sessions(&self) -> PlanResult<Vec<SessionSummary>> {
        let mut sessions = Vec::new();

        if !self.traces_dir.exists() {
            return Ok(sessions);
        }

        let mut entries = fs::read_dir(&self.traces_dir)
            .await
            .map_err(|e| PlanError::IoError(e))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| PlanError::IoError(e))?
        {
            if entry
                .file_type()
                .await
                .map_err(|e| PlanError::IoError(e))?
                .is_dir()
            {
                // This is a month directory (YYYY-MM)
                let mut month_entries = fs::read_dir(entry.path())
                    .await
                    .map_err(|e| PlanError::IoError(e))?;

                while let Some(file_entry) = month_entries
                    .next_entry()
                    .await
                    .map_err(|e| PlanError::IoError(e))?
                {
                    if let Some(name) = file_entry.file_name().to_str() {
                        if name.ends_with(".summary.json") {
                            if let Ok(summary_data) = fs::read_to_string(file_entry.path()).await {
                                if let Ok(summary) =
                                    serde_json::from_str::<SessionSummary>(&summary_data)
                                {
                                    sessions.push(summary);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Sort by start time (newest first)
        sessions.sort_by(|a, b| b.start_time.cmp(&a.start_time));

        Ok(sessions)
    }

    /// Load traces for a specific session
    pub async fn load_session_traces(&self, session_id: &str) -> PlanResult<Vec<AutonomousTrace>> {
        let mut traces = Vec::new();

        // Search for the trace file in all month directories
        let mut month_dirs = fs::read_dir(&self.traces_dir)
            .await
            .map_err(|e| PlanError::IoError(e))?;

        while let Some(month_entry) = month_dirs
            .next_entry()
            .await
            .map_err(|e| PlanError::IoError(e))?
        {
            if month_entry
                .file_type()
                .await
                .map_err(|e| PlanError::IoError(e))?
                .is_dir()
            {
                let trace_file = month_entry.path().join(format!("{}.jsonl", session_id));

                if trace_file.exists() {
                    let file = File::open(trace_file)
                        .await
                        .map_err(|e| PlanError::IoError(e))?;
                    let reader = BufReader::new(file);
                    let mut lines = reader.lines();

                    while let Some(line) =
                        lines.next_line().await.map_err(|e| PlanError::IoError(e))?
                    {
                        if let Ok(trace) = serde_json::from_str::<AutonomousTrace>(&line) {
                            traces.push(trace);
                        }
                    }

                    break;
                }
            }
        }

        // Sort by step number
        traces.sort_by_key(|t| t.step_number);

        Ok(traces)
    }

    /// Replay a session with detailed output
    pub async fn replay_session(&self, session_id: &str, include_dom: bool) -> PlanResult<()> {
        println!("🎬 Replaying session: {}", session_id);
        println!();

        let traces = self.load_session_traces(session_id).await?;

        if traces.is_empty() {
            println!("[ERROR] No traces found for session: {}", session_id);
            return Ok(());
        }

        let mut total_cost = 0.0;
        let mut total_time = 0u64;

        for trace in &traces {
            println!("[LIST] Step {}: {}", trace.step_number, trace.step.name);
            println!(
                "   Timestamp: {}",
                trace
                    .timestamp
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S")
            );
            println!("   Action: {:?}", trace.step.kind);

            match &trace.result {
                ExecutionResult::Success { payload } => {
                    println!(
                        "   [OK] Success ({}ms, {} retries)",
                        trace.execution_time_ms, trace.retry_count
                    );
                    if let Some(data) = payload {
                        println!(
                            "    Data: {}",
                            serde_json::to_string_pretty(data)
                                .unwrap_or_else(|_| "Invalid JSON".to_string())
                        );
                    }
                }
                ExecutionResult::Error { message, .. } => {
                    println!(
                        "   [ERROR] Error ({}ms, {} retries): {}",
                        trace.execution_time_ms, trace.retry_count, message
                    );
                    if let Some(ref category) = trace.error_category {
                        println!("     Category: {}", category);
                    }
                    if let Some(ref strategy) = trace.recovery_strategy {
                        println!("    Suggested recovery: {}", strategy);
                    }
                }
            }

            if let Some(ref usage) = trace.llm_usage {
                println!(
                    "   [BOT] LLM: {} ({} tokens, ${:.4})",
                    usage.model, usage.total_tokens, usage.estimated_cost
                );
            }

            if trace.step_cost > 0.0 {
                println!("    Cost: ${:.4}", trace.step_cost);
            }

            if let Some(ref screenshot) = trace.screenshot_path {
                println!("   📸 Screenshot: {}", screenshot);
            }

            if include_dom && trace.dom_snapshot.is_some() {
                println!(
                    "    DOM snapshot available ({} chars)",
                    trace.dom_snapshot.as_ref().unwrap().len()
                );
            }

            total_cost += trace.step_cost;
            total_time += trace.execution_time_ms;

            println!();
        }

        println!(" Session Summary:");
        println!("   Total steps: {}", traces.len());
        println!("   Total time: {:.2}s", total_time as f64 / 1000.0);
        println!("   Total cost: ${:.4}", total_cost);
        println!(
            "   Success rate: {:.1}%",
            traces
                .iter()
                .filter(|t| matches!(t.result, ExecutionResult::Success { .. }))
                .count() as f64
                / traces.len() as f64
                * 100.0
        );

        Ok(())
    }

    /// Generate analytics report for a time period
    pub async fn generate_analytics_report(&self, days: u32) -> PlanResult<serde_json::Value> {
        let sessions = self.list_sessions().await?;
        let cutoff = Utc::now() - chrono::Duration::days(days as i64);

        let recent_sessions: Vec<_> = sessions
            .into_iter()
            .filter(|s| s.start_time > cutoff)
            .collect();

        let total_sessions = recent_sessions.len();
        let total_cost: f64 = recent_sessions.iter().map(|s| s.total_cost).sum();
        let total_steps: u32 = recent_sessions.iter().map(|s| s.total_steps).sum();
        let successful_sessions = recent_sessions
            .iter()
            .filter(|s| s.error_summary.is_none())
            .count();

        let mut cost_by_model: HashMap<String, f64> = HashMap::new();
        for session in &recent_sessions {
            for (model, cost) in &session.cost_breakdown {
                *cost_by_model.entry(model.clone()).or_insert(0.0) += cost;
            }
        }

        Ok(serde_json::json!({
            "period_days": days,
            "total_sessions": total_sessions,
            "successful_sessions": successful_sessions,
            "success_rate": if total_sessions > 0 { successful_sessions as f64 / total_sessions as f64 } else { 0.0 },
            "total_steps": total_steps,
            "total_cost": total_cost,
            "average_cost_per_session": if total_sessions > 0 { total_cost / total_sessions as f64 } else { 0.0 },
            "cost_by_model": cost_by_model,
            "sessions": recent_sessions,
        }))
    }
}

impl ActionStats {
    /// Calculate total attempts
    pub fn total_attempts(&self) -> u32 {
        self.dom_success + self.dom_failure + self.native_success + self.native_failure
    }

    /// Calculate overall success rate
    pub fn success_rate(&self) -> f64 {
        let total = self.total_attempts();
        if total == 0 {
            0.0
        } else {
            (self.dom_success + self.native_success) as f64 / total as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use tempfile::TempDir;
    use tokio::fs;

    #[tokio::test]
    async fn test_cost_calculation() {
        let cost_tracker = CostTracker::new_with_default_rates(None, vec![]);

        // Test GPT-4o cost calculation
        let cost = cost_tracker.calculate_cost("gpt-4o", 1000, 500);
        assert_eq!(cost, 2.5 + 5.0); // $2.50 per 1K prompt + $10.00 per 1K completion

        // Test GPT-4o-mini cost calculation
        let mini_cost = cost_tracker.calculate_cost("o4-mini", 1000, 500);
        assert_eq!(mini_cost, 0.15 + 0.30); // Much cheaper rates

        // Test Gemini Flash cost calculation
        let gemini_cost = cost_tracker.calculate_cost("gemini-2.0-flash", 1000, 500);
        assert_eq!(gemini_cost, 0.075 + 0.15); // Cheapest option

        // Test unknown model falls back to GPT-4o rates
        let fallback_cost = cost_tracker.calculate_cost("unknown-model", 1000, 500);
        assert_eq!(fallback_cost, cost);
    }

    #[tokio::test]
    async fn test_cost_tracking_and_alerts() {
        let mut cost_tracker = CostTracker::new_with_default_rates(
            Some(10.0),           // $10 budget
            vec![1.0, 5.0, 10.0], // Alert thresholds
        );

        // Test adding usage and cost calculation
        let cost_calc = cost_tracker.calculate_cost("gpt-4o", 2000, 1000);
        let usage = LLMUsage {
            model: "gpt-4o".to_string(),
            prompt_tokens: 2000,
            completion_tokens: 1000,
            total_tokens: 3000,
            provider_cost: None,
            estimated_cost: cost_calc,
            duration_ms: 1500,
        };

        cost_tracker.add_usage(&usage).await.unwrap();

        // Should have calculated cost: (2000/1000 * 2.50) + (1000/1000 * 10.00) = 5.0 + 10.0 = 15.0
        assert_eq!(cost_tracker.total_cost, 15.0);
        assert_eq!(cost_tracker.cost_by_model["gpt-4o"], 15.0);

        // alerts are automatically checked in add_usage, so should have fired
        assert!(cost_tracker.fired_alerts.contains(&1.0));
        assert!(cost_tracker.fired_alerts.contains(&5.0));
        assert!(cost_tracker.fired_alerts.contains(&10.0));

        // Test token usage tracking
        let token_stats = &cost_tracker.token_usage["gpt-4o"];
        assert_eq!(token_stats.total_prompt_tokens, 2000);
        assert_eq!(token_stats.total_completion_tokens, 1000);
        assert_eq!(token_stats.total_requests, 1);
    }

    #[tokio::test]
    async fn test_telemetry_collection() {
        let temp_dir = TempDir::new().unwrap();
        let config = TelemetryConfig {
            traces_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let mut collector = TelemetryCollector::new(
            "Test goal".to_string(),
            Some("https://example.com".to_string()),
            config,
        )
        .await
        .unwrap();

        // Record a successful step
        let step = Step {
            id: "test-1".to_string(),
            name: "Test step".to_string(),
            kind: StepKind::WaitForTimeout { timeout_ms: 1000 },
        };

        let result = ExecutionResult::Success { payload: None };

        collector
            .record_step(step, result, 1500, 0, None, None, None)
            .await
            .unwrap();

        assert_eq!(collector.traces.len(), 1);
        assert_eq!(collector.get_total_cost(), 0.0); // No LLM usage

        let stats = collector.get_action_stats();
        assert!(stats.contains_key("wait"));
        assert_eq!(stats["wait"].dom_success, 1);
        assert_eq!(stats["wait"].total_attempts, 1);
        assert_eq!(stats["wait"].success_rate, 1.0);
    }

    #[tokio::test]
    async fn test_telemetry_with_llm_usage() {
        let temp_dir = TempDir::new().unwrap();
        let config = TelemetryConfig {
            traces_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let mut collector = TelemetryCollector::new("LLM test goal".to_string(), None, config)
            .await
            .unwrap();

        // Record LLM usage
        let llm_usage = LLMUsage {
            model: "o4-mini".to_string(),
            prompt_tokens: 1000,
            completion_tokens: 500,
            total_tokens: 1500,
            provider_cost: Some(0.30),
            estimated_cost: 0.45, // Our calculated cost
            duration_ms: 2000,
        };

        collector.record_llm_usage(llm_usage.clone()).await.unwrap();

        // Record a step with LLM usage
        let step = Step {
            id: "llm-step".to_string(),
            name: "LLM planning step".to_string(),
            kind: StepKind::NavigateToUrl {
                url: "https://test.com".to_string(),
                wait: Some("load".to_string()),
            },
        };

        let result = ExecutionResult::Success {
            payload: Some(serde_json::json!({"test": "data"})),
        };

        collector
            .record_step(
                step,
                result,
                3000,
                1,
                Some("<html>...</html>".to_string()),
                Some("/tmp/screenshot.png".to_string()),
                Some(llm_usage),
            )
            .await
            .unwrap();

        // Check costs accumulated
        let total_cost = collector.get_total_cost();
        assert!(total_cost > 0.0);

        let cost_breakdown = collector.get_cost_breakdown();
        assert!(cost_breakdown.contains_key("o4-mini"));

        // Check trace recorded
        assert_eq!(collector.traces.len(), 1);
        let trace = &collector.traces[0];
        assert_eq!(trace.step.name, "LLM planning step");
        assert_eq!(trace.execution_time_ms, 3000);
        assert_eq!(trace.retry_count, 1);
        assert!(trace.dom_snapshot.is_some());
        assert!(trace.screenshot_path.is_some());
        assert!(trace.llm_usage.is_some());
        assert!(trace.step_cost > 0.0);
    }

    #[tokio::test]
    async fn test_session_summary() {
        let temp_dir = TempDir::new().unwrap();
        let config = TelemetryConfig {
            traces_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let mut collector = TelemetryCollector::new(
            "Session summary test".to_string(),
            Some("https://start.com".to_string()),
            config,
        )
        .await
        .unwrap();

        // Record multiple steps with different outcomes
        for i in 0..5 {
            let step = Step {
                id: format!("step-{}", i),
                name: format!("Test step {}", i),
                kind: StepKind::WaitForTimeout { timeout_ms: 1000 },
            };

            let result = if i < 3 {
                ExecutionResult::Success { payload: None }
            } else {
                ExecutionResult::Error {
                    message: format!("Error in step {}", i),
                    retry_suggested: true,
                }
            };

            collector
                .record_step(
                    step,
                    result,
                    1000 + (i as u64 * 100),
                    i as u32,
                    None,
                    None,
                    None,
                )
                .await
                .unwrap();
        }

        let summary = collector.get_session_summary(Some(serde_json::json!({"final": "result"})));

        assert_eq!(summary.goal, "Session summary test");
        assert_eq!(summary.start_url, Some("https://start.com".to_string()));
        assert_eq!(summary.total_steps, 5);
        assert_eq!(summary.successful_steps, 3);
        assert_eq!(summary.failed_steps, 2);
        assert!(summary.error_summary.is_some());
        assert!(summary.final_result.is_some());
    }

    #[tokio::test]
    async fn test_trace_file_writing() {
        let temp_dir = TempDir::new().unwrap();
        let trace_file = temp_dir.path().join("test-session.jsonl");

        let mut writer = TraceWriter::new(trace_file.clone()).await.unwrap();

        // Write some traces
        for i in 0..3 {
            let trace = AutonomousTrace {
                session_id: "test-session".to_string(),
                step_number: i + 1,
                timestamp: Utc::now(),
                step: Step {
                    id: format!("step-{}", i),
                    name: format!("Test step {}", i),
                    kind: StepKind::WaitForTimeout { timeout_ms: 1000 },
                },
                result: ExecutionResult::Success { payload: None },
                execution_time_ms: 1000,
                retry_count: 0,
                dom_snapshot: None,
                screenshot_path: None,
                llm_usage: None,
                step_cost: 0.0,
                error_category: None,
                recovery_strategy: None,
            };

            writer.write_trace(&trace).await.unwrap();
        }

        writer.close().await.unwrap();

        // Verify file was written
        assert!(trace_file.exists());

        // Read and verify content
        let content = fs::read_to_string(&trace_file).await.unwrap();
        let lines: Vec<&str> = content.trim().split('\n').collect();
        assert_eq!(lines.len(), 3);

        // Verify each line is valid JSON
        for line in lines {
            let _trace: AutonomousTrace = serde_json::from_str(line).unwrap();
        }
    }

    #[tokio::test]
    async fn test_trace_replay() {
        let temp_dir = TempDir::new().unwrap();

        // Create a test session with traces
        let session_id = "test-replay-session";
        let month_dir = temp_dir.path().join("2024-07");
        fs::create_dir_all(&month_dir).await.unwrap();

        // Write traces file
        let trace_file = month_dir.join(format!("{}.jsonl", session_id));
        let mut content = String::new();

        for i in 0..3 {
            let trace = AutonomousTrace {
                session_id: session_id.to_string(),
                step_number: i + 1,
                timestamp: Utc::now(),
                step: Step {
                    id: format!("step-{}", i),
                    name: format!("Replay test step {}", i),
                    kind: StepKind::ClickElement {
                        selector: format!("button#{}", i),
                        frame_id: None,
                        random_offset: None,
                        timeout_ms: Some(5000),
                    },
                },
                result: if i == 2 {
                    ExecutionResult::Error {
                        message: "Click failed".to_string(),
                        retry_suggested: true,
                    }
                } else {
                    ExecutionResult::Success { payload: None }
                },
                execution_time_ms: 1000 + (i as u64 * 200),
                retry_count: if i == 2 { 2 } else { 0 },
                dom_snapshot: None,
                screenshot_path: Some(format!("/tmp/screenshot-{}.png", i)),
                llm_usage: Some(LLMUsage {
                    model: "o4-mini".to_string(),
                    prompt_tokens: 100,
                    completion_tokens: 50,
                    total_tokens: 150,
                    provider_cost: None,
                    estimated_cost: 0.075,
                    duration_ms: 1500,
                }),
                step_cost: 0.075,
                error_category: if i == 2 {
                    Some("element_not_found".to_string())
                } else {
                    None
                },
                recovery_strategy: if i == 2 {
                    Some("retry_with_fallback_selector".to_string())
                } else {
                    None
                },
            };

            content.push_str(&serde_json::to_string(&trace).unwrap());
            content.push('\n');
        }

        fs::write(&trace_file, content).await.unwrap();

        // Write session summary
        let summary = SessionSummary {
            session_id: session_id.to_string(),
            goal: "Replay test goal".to_string(),
            start_url: Some("https://replay-test.com".to_string()),
            start_time: Utc::now() - Duration::hours(1),
            end_time: Some(Utc::now()),
            total_steps: 3,
            successful_steps: 2,
            failed_steps: 1,
            total_cost: 0.225,
            cost_breakdown: {
                let mut map = HashMap::new();
                map.insert("o4-mini".to_string(), 0.225);
                map
            },
            action_stats: HashMap::new(),
            final_result: Some(serde_json::json!({"success": false})),
            error_summary: Some("1 error: Click failed".to_string()),
        };

        let summary_file = month_dir.join(format!("{}.summary.json", session_id));
        fs::write(
            &summary_file,
            serde_json::to_string_pretty(&summary).unwrap(),
        )
        .await
        .unwrap();

        // Test replay functionality
        let replay = TraceReplay::new(Some(temp_dir.path().to_path_buf()));

        // List sessions
        let sessions = replay.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, session_id);
        assert_eq!(sessions[0].total_steps, 3);
        assert_eq!(sessions[0].total_cost, 0.225);

        // Load session traces
        let traces = replay.load_session_traces(session_id).await.unwrap();
        assert_eq!(traces.len(), 3);
        assert_eq!(traces[0].step_number, 1);
        assert_eq!(traces[2].step_number, 3);
        assert!(matches!(traces[2].result, ExecutionResult::Error { .. }));
    }

    #[tokio::test]
    async fn test_analytics_report() {
        let temp_dir = TempDir::new().unwrap();
        let replay = TraceReplay::new(Some(temp_dir.path().to_path_buf()));

        // Create multiple test sessions
        let month_dir = temp_dir.path().join("2024-07");
        fs::create_dir_all(&month_dir).await.unwrap();

        for i in 0..5 {
            let session_id = format!("analytics-session-{}", i);
            let session_start = Utc::now() - Duration::days(i as i64);

            let summary = SessionSummary {
                session_id: session_id.clone(),
                goal: format!("Analytics test goal {}", i),
                start_url: Some(format!("https://test{}.com", i)),
                start_time: session_start,
                end_time: Some(session_start + Duration::minutes(30)),
                total_steps: 5 + i,
                successful_steps: 4 + i - (i % 2), // Some failures
                failed_steps: 1 + (i % 2),
                total_cost: 0.5 + (i as f64 * 0.1),
                cost_breakdown: {
                    let mut map = HashMap::new();
                    map.insert(
                        if i % 2 == 0 {
                            "o4-mini"
                        } else {
                            "gemini-2.0-flash"
                        }
                        .to_string(),
                        0.5 + (i as f64 * 0.1),
                    );
                    map
                },
                action_stats: HashMap::new(),
                final_result: Some(serde_json::json!({"success": i % 2 == 0})),
                error_summary: if i % 2 == 1 {
                    Some(format!("Error in session {}", i))
                } else {
                    None
                },
            };

            let summary_file = month_dir.join(format!("{}.summary.json", session_id));
            fs::write(
                &summary_file,
                serde_json::to_string_pretty(&summary).unwrap(),
            )
            .await
            .unwrap();
        }

        // Generate analytics report
        let report = replay.generate_analytics_report(7).await.unwrap();

        // Verify report structure
        assert_eq!(report["period_days"], 7);
        assert_eq!(report["total_sessions"], 5);
        assert_eq!(report["successful_sessions"], 3); // Sessions 0, 2, 4 are successful
        assert_eq!(report["success_rate"], 0.6); // 3/5 = 0.6

        let total_cost = report["total_cost"].as_f64().unwrap();
        // Total cost should be: 0.5 + 0.6 + 0.7 + 0.8 + 0.9 = 3.5
        assert!(total_cost > 3.0 && total_cost < 4.0); // Sum of all session costs

        // Check cost by model
        let cost_by_model = report["cost_by_model"].as_object().unwrap();
        assert!(cost_by_model.contains_key("o4-mini"));
        assert!(cost_by_model.contains_key("gemini-2.0-flash"));
    }

    #[test]
    fn test_action_stats() {
        let mut stats = ActionStats::default();
        stats.dom_success = 8;
        stats.dom_failure = 2;

        assert_eq!(stats.total_attempts(), 10);
        assert_eq!(stats.success_rate(), 0.8);

        // Test with native success too
        stats.native_success = 3;
        stats.native_failure = 1;

        assert_eq!(stats.total_attempts(), 14);
        assert_eq!(stats.success_rate(), 11.0 / 14.0); // (8+3)/(8+2+3+1)
    }

    #[tokio::test]
    async fn test_error_categorization() {
        let temp_dir = TempDir::new().unwrap();
        let config = TelemetryConfig {
            traces_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let collector = TelemetryCollector::new("Error test".to_string(), None, config)
            .await
            .unwrap();

        // Test different error categories
        assert_eq!(collector.categorize_error("Operation timed out"), "timeout");
        assert_eq!(
            collector.categorize_error("Element not found"),
            "element_not_found"
        );
        assert_eq!(
            collector.categorize_error("Click intercepted by overlay"),
            "element_intercepted"
        );
        assert_eq!(
            collector.categorize_error("Frame context lost"),
            "frame_error"
        );
        assert_eq!(
            collector.categorize_error("Network connection failed"),
            "network_error"
        );
        assert_eq!(
            collector.categorize_error("Permission denied"),
            "permission_error"
        );
        assert_eq!(
            collector.categorize_error("Unknown error occurred"),
            "unknown"
        );
    }

    #[tokio::test]
    async fn test_recovery_strategies() {
        let temp_dir = TempDir::new().unwrap();
        let config = TelemetryConfig {
            traces_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let collector = TelemetryCollector::new("Recovery test".to_string(), None, config)
            .await
            .unwrap();

        // Test recovery strategy suggestions
        assert_eq!(
            collector.suggest_recovery_strategy("timeout"),
            Some("increase_timeout".to_string())
        );
        assert_eq!(
            collector.suggest_recovery_strategy("element_not_found"),
            Some("retry_with_fallback_selector".to_string())
        );
        assert_eq!(
            collector.suggest_recovery_strategy("element_intercepted"),
            Some("scroll_into_view_then_click".to_string())
        );
        assert_eq!(
            collector.suggest_recovery_strategy("frame_error"),
            Some("switch_frame_context".to_string())
        );
        assert_eq!(
            collector.suggest_recovery_strategy("network_error"),
            Some("retry_after_delay".to_string())
        );
        assert_eq!(collector.suggest_recovery_strategy("unknown"), None);
    }
}
