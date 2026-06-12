use crate::mode_selector::{EscalationReason, ExecutionMode};
use crate::{PlanError, PlanResult};
use log::{debug, info, warn};
use rzn_core::Workflow;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Workflow execution metadata for mode tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowExecutionContext {
    pub workflow_id: String,
    pub start_url: String,
    pub target_domain: String,
    pub execution_mode: ExecutionMode,
    pub cross_origin_detected: bool,
    pub escalation_history: Vec<EscalationReason>,
    pub total_steps: usize,
    pub completed_steps: usize,
    pub current_step_index: usize,
}

/// Manages workflow storage and retrieval with mode-aware execution
pub struct WorkflowManager {
    workflows_dir: PathBuf,
    cache: WorkflowCache,
    execution_contexts: HashMap<String, WorkflowExecutionContext>,
}

/// In-memory cache of workflows
pub struct WorkflowCache {
    workflows: HashMap<String, Workflow>,
    goal_index: HashMap<String, Vec<String>>, // goal keywords -> workflow IDs
}

impl WorkflowManager {
    pub fn new(workflows_dir: &str) -> PlanResult<Self> {
        let workflows_dir = PathBuf::from(workflows_dir);
        let cache = WorkflowCache::new();

        Ok(Self {
            workflows_dir,
            cache,
            execution_contexts: HashMap::new(),
        })
    }

    /// Initialize the workflow manager and load existing workflows
    pub async fn initialize(&mut self) -> PlanResult<()> {
        // Create workflows directory if it doesn't exist
        if !self.workflows_dir.exists() {
            fs::create_dir_all(&self.workflows_dir).await?;
            info!("Created workflows directory: {:?}", self.workflows_dir);
        }

        // Load existing workflows into cache
        self.load_all_workflows().await?;

        Ok(())
    }

    /// Save a workflow to disk and cache
    pub async fn save_workflow(&mut self, workflow: &Workflow) -> PlanResult<String> {
        let filename = format!("{}.yaml", workflow.id);
        let file_path = self.workflows_dir.join(&filename);

        // Serialize workflow to YAML
        let yaml_content = serde_yaml::to_string(workflow).map_err(|e| {
            PlanError::SerializationError(serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                e,
            )))
        })?;

        // Write to file
        fs::write(&file_path, yaml_content).await?;

        // Add to cache
        self.cache.add_workflow(workflow.clone());

        info!("Saved workflow '{}' to {:?}", workflow.id, file_path);
        Ok(file_path.to_string_lossy().to_string())
    }

    /// Load a workflow by ID or filename
    pub async fn load_workflow(&mut self, identifier: &str) -> PlanResult<Workflow> {
        // First check cache
        if let Some(workflow) = self.cache.get_workflow(identifier) {
            debug!("Found workflow '{}' in cache", identifier);
            return Ok(workflow.clone());
        }

        // Try to load from disk
        let workflow = self.load_workflow_from_disk(identifier).await?;

        // Add to cache
        self.cache.add_workflow(workflow.clone());

        Ok(workflow)
    }

    /// Find workflows similar to the given goal
    pub async fn find_similar_workflow(&mut self, goal: &str) -> PlanResult<Option<String>> {
        // Ensure cache is loaded
        if self.cache.is_empty() {
            self.load_all_workflows().await?;
        }

        // Simple keyword matching for now
        let goal_words: Vec<String> = goal
            .to_lowercase()
            .split_whitespace()
            .filter(|w| w.len() > 3) // Filter out short words
            .map(|w| w.to_string())
            .collect();

        for word in goal_words {
            if let Some(workflow_ids) = self.cache.goal_index.get(&word) {
                for workflow_id in workflow_ids {
                    if let Some(_workflow) = self.cache.get_workflow(workflow_id) {
                        debug!(
                            "Found similar workflow '{}' for goal '{}'",
                            workflow_id, goal
                        );
                        return Ok(Some(workflow_id.clone()));
                    }
                }
            }
        }

        debug!("No similar workflow found for goal: {}", goal);
        Ok(None)
    }

    /// List all available workflows
    pub async fn list_workflows(&mut self) -> PlanResult<Vec<Workflow>> {
        if self.cache.is_empty() {
            self.load_all_workflows().await?;
        }

        Ok(self.cache.workflows.values().cloned().collect())
    }

    /// Delete a workflow
    pub async fn delete_workflow(&mut self, identifier: &str) -> PlanResult<()> {
        // Remove from cache
        self.cache.remove_workflow(identifier);

        // Remove from disk
        let possible_paths = vec![
            self.workflows_dir.join(format!("{}.yaml", identifier)),
            self.workflows_dir.join(format!("{}.yml", identifier)),
            self.workflows_dir.join(identifier),
        ];

        for path in possible_paths {
            if path.exists() {
                fs::remove_file(&path).await?;
                info!("Deleted workflow file: {:?}", path);
                return Ok(());
            }
        }

        warn!("Workflow file not found for deletion: {}", identifier);
        Ok(())
    }

    /// Load all workflows from disk into cache
    async fn load_all_workflows(&mut self) -> PlanResult<()> {
        let mut entries = fs::read_dir(&self.workflows_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.is_file() {
                if let Some(extension) = path.extension() {
                    let is_yaml = extension == "yaml" || extension == "yml";
                    let is_json = extension == "json";
                    // Skip parameter meta files
                    if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                        if name.ends_with(".params.json") {
                            continue;
                        }
                    }
                    if is_yaml || is_json {
                        match self.load_workflow_from_path(&path).await {
                            Ok(workflow) => {
                                self.cache.add_workflow(workflow);
                            }
                            Err(e) => {
                                warn!("Failed to load workflow from {:?}: {}", path, e);
                            }
                        }
                    }
                }
            }
        }

        info!("Loaded {} workflows into cache", self.cache.workflows.len());
        Ok(())
    }

    /// Load a workflow from disk by identifier
    async fn load_workflow_from_disk(&self, identifier: &str) -> PlanResult<Workflow> {
        // Try different possible file paths
        let possible_paths = vec![
            self.workflows_dir.join(format!("{}.yaml", identifier)),
            self.workflows_dir.join(format!("{}.yml", identifier)),
            self.workflows_dir.join(identifier),
            PathBuf::from(identifier), // Direct path
        ];

        for path in possible_paths {
            if path.exists() {
                return self.load_workflow_from_path(&path).await;
            }
        }

        Err(PlanError::WorkflowNotFound(identifier.to_string()))
    }

    /// Load a workflow from a specific file path
    async fn load_workflow_from_path(&self, path: &Path) -> PlanResult<Workflow> {
        let content = fs::read_to_string(path).await?;

        // Try to parse as JSON first (since workflows are JSON)
        let workflow: Workflow = if path.extension().and_then(|s| s.to_str()) == Some("json") {
            serde_json::from_str(&content)?
        } else {
            serde_yaml::from_str(&content).map_err(|e| {
                PlanError::SerializationError(serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    e,
                )))
            })?
        };

        debug!("Loaded workflow '{}' from {:?}", workflow.id, path);
        Ok(workflow)
    }
}

impl WorkflowCache {
    fn new() -> Self {
        Self {
            workflows: HashMap::new(),
            goal_index: HashMap::new(),
        }
    }

    fn add_workflow(&mut self, workflow: Workflow) {
        // Index by goal keywords
        let goal_words: Vec<String> = workflow
            .description
            .to_lowercase()
            .split_whitespace()
            .filter(|w| w.len() > 3)
            .map(|w| w.to_string())
            .collect();

        for word in goal_words {
            self.goal_index
                .entry(word)
                .or_default()
                .push(workflow.id.clone());
        }

        // Store workflow
        self.workflows.insert(workflow.id.clone(), workflow);
    }

    fn get_workflow(&self, identifier: &str) -> Option<&Workflow> {
        self.workflows.get(identifier)
    }

    fn remove_workflow(&mut self, identifier: &str) {
        if let Some(workflow) = self.workflows.remove(identifier) {
            // Remove from goal index
            let goal_words: Vec<String> = workflow
                .description
                .to_lowercase()
                .split_whitespace()
                .filter(|w| w.len() > 3)
                .map(|w| w.to_string())
                .collect();

            for word in goal_words {
                if let Some(workflow_ids) = self.goal_index.get_mut(&word) {
                    workflow_ids.retain(|id| id != identifier);
                    if workflow_ids.is_empty() {
                        self.goal_index.remove(&word);
                    }
                }
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.workflows.is_empty()
    }
}

// Dynamic mode switching methods
impl WorkflowManager {
    /// Start workflow execution with context tracking
    pub fn start_workflow_execution(
        &mut self,
        workflow_id: &str,
        start_url: &str,
        initial_mode: ExecutionMode,
    ) -> PlanResult<String> {
        let workflow = self
            .cache
            .get_workflow(workflow_id)
            .ok_or_else(|| PlanError::WorkflowNotFound(workflow_id.to_string()))?;

        let target_domain = self.extract_domain_from_url(start_url);
        let execution_id = uuid::Uuid::new_v4().to_string();

        let context = WorkflowExecutionContext {
            workflow_id: workflow_id.to_string(),
            start_url: start_url.to_string(),
            target_domain,
            execution_mode: initial_mode,
            cross_origin_detected: false,
            escalation_history: Vec::new(),
            total_steps: workflow
                .browser_automation
                .sequences
                .iter()
                .map(|seq| seq.steps.len())
                .sum(),
            completed_steps: 0,
            current_step_index: 0,
        };

        self.execution_contexts
            .insert(execution_id.clone(), context);

        info!(
            "Started workflow execution: {} for workflow {} on {}",
            execution_id, workflow_id, start_url
        );

        Ok(execution_id)
    }

    /// Update execution context after step completion
    pub fn update_execution_progress(
        &mut self,
        execution_id: &str,
        step_completed: bool,
        mode_changed: Option<ExecutionMode>,
        escalation_reason: Option<EscalationReason>,
    ) -> PlanResult<()> {
        let context = self
            .execution_contexts
            .get_mut(execution_id)
            .ok_or_else(|| {
                PlanError::WorkflowNotFound("Execution context not found".to_string())
            })?;

        if step_completed {
            context.completed_steps += 1;
            context.current_step_index += 1;
        }

        if let Some(new_mode) = mode_changed {
            if new_mode != context.execution_mode {
                info!(
                    "Mode switched from {:?} to {:?} for execution {}",
                    context.execution_mode, new_mode, execution_id
                );
                context.execution_mode = new_mode;
            }
        }

        if let Some(reason) = escalation_reason {
            // Mark cross-origin detection for certain escalation reasons
            if matches!(reason, EscalationReason::CrossOriginRequired) {
                context.cross_origin_detected = true;
            }
            context.escalation_history.push(reason);
        }

        debug!(
            "Updated execution progress: {}/{} steps completed",
            context.completed_steps, context.total_steps
        );

        Ok(())
    }

    /// Check if workflow requires cross-origin handling
    pub fn requires_cross_origin_handling(&self, execution_id: &str) -> bool {
        self.execution_contexts
            .get(execution_id)
            .map(|ctx| ctx.cross_origin_detected || self.is_cross_origin_workflow(&ctx.workflow_id))
            .unwrap_or(false)
    }

    /// Get recommended execution mode for workflow
    pub fn get_recommended_mode(&self, workflow_id: &str, _target_url: &str) -> ExecutionMode {
        // Check if this is a known complex workflow
        if let Some(workflow) = self.cache.get_workflow(workflow_id) {
            if self.is_complex_workflow(workflow) {
                return ExecutionMode::Pro;
            }
        }

        // If a workflow has explicit cross-origin requirements, prefer Pro mode.
        // Avoid baked-in per-domain heuristics.
        if self.is_cross_origin_workflow(workflow_id) {
            return ExecutionMode::Pro;
        }

        // Default to Light mode
        ExecutionMode::Light
    }

    /// Get execution statistics
    pub fn get_execution_stats(&self, execution_id: &str) -> Option<WorkflowExecutionStats> {
        self.execution_contexts
            .get(execution_id)
            .map(|ctx| WorkflowExecutionStats {
                progress_percentage: if ctx.total_steps > 0 {
                    (ctx.completed_steps as f64 / ctx.total_steps as f64) * 100.0
                } else {
                    0.0
                },
                current_mode: ctx.execution_mode,
                escalation_count: ctx.escalation_history.len(),
                is_cross_origin: ctx.cross_origin_detected,
                steps_remaining: ctx.total_steps.saturating_sub(ctx.completed_steps),
            })
    }

    /// Finish workflow execution and get summary
    pub fn finish_workflow_execution(
        &mut self,
        execution_id: &str,
    ) -> Option<WorkflowExecutionSummary> {
        self.execution_contexts.remove(execution_id).map(|ctx| {
            let success_rate = if ctx.total_steps > 0 {
                ctx.completed_steps as f64 / ctx.total_steps as f64
            } else {
                0.0
            };

            info!(
                "Finished workflow execution: {} with {:.1}% completion",
                execution_id,
                success_rate * 100.0
            );

            WorkflowExecutionSummary {
                workflow_id: ctx.workflow_id,
                execution_id: execution_id.to_string(),
                success_rate,
                final_mode: ctx.execution_mode,
                total_escalations: ctx.escalation_history.len(),
                cross_origin_required: ctx.cross_origin_detected,
                execution_duration: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            }
        })
    }

    /// Get all active execution contexts
    pub fn get_active_executions(&self) -> Vec<&WorkflowExecutionContext> {
        self.execution_contexts.values().collect()
    }

    /// Clean up stale execution contexts
    pub fn cleanup_stale_executions(&mut self, max_age_minutes: u64) {
        let _cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(max_age_minutes * 60);

        let stale_ids: Vec<String> = self
            .execution_contexts
            .iter()
            .filter_map(|(id, _)| {
                // For simplicity, assume all contexts older than max_age are stale
                // In a real implementation, we'd track creation timestamps
                if rand::random::<u64>() % 100 < 10 {
                    // 10% chance of cleanup for demo
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();

        for id in stale_ids {
            self.execution_contexts.remove(&id);
            debug!("Cleaned up stale execution context: {}", id);
        }
    }

    // Helper methods

    /// Extract domain from URL
    fn extract_domain_from_url(&self, url: &str) -> String {
        url::Url::parse(url)
            .map(|parsed| parsed.host_str().unwrap_or("unknown").to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    }

    /// Check if workflow is complex and needs Pro mode by default
    fn is_complex_workflow(&self, workflow: &Workflow) -> bool {
        // Complex workflows typically have:
        // - Many steps (>10)
        // - File upload steps
        // - Multiple frame interactions
        // - Cross-origin requirements in description

        let step_count = workflow
            .browser_automation
            .sequences
            .iter()
            .map(|seq| seq.steps.len())
            .sum::<usize>();
        let description = workflow.description.to_lowercase();

        step_count > 10
            || description.contains("upload")
            || description.contains("iframe")
            || description.contains("cross-origin")
            || description.contains("payment")
            || description.contains("banking")
    }

    /// Check if workflow is known to be cross-origin
    fn is_cross_origin_workflow(&self, workflow_id: &str) -> bool {
        if let Some(workflow) = self.cache.get_workflow(workflow_id) {
            // Check if any steps have frame_id specified
            workflow.browser_automation.sequences.iter().any(|seq| {
                seq.steps.iter().any(|step| match &step.kind {
                    rzn_core::StepKind::ClickElement { frame_id, .. } => frame_id.is_some(),
                    rzn_core::StepKind::FillInputField { frame_id, .. } => frame_id.is_some(),
                    rzn_core::StepKind::WaitForElement { frame_id, .. } => frame_id.is_some(),
                    rzn_core::StepKind::GetElementText { frame_id, .. } => frame_id.is_some(),
                    rzn_core::StepKind::ExtractStructuredData { frame_id, .. } => {
                        frame_id.is_some()
                    }
                    _ => false,
                })
            })
        } else {
            false
        }
    }

    // NOTE: domain-tuned mode heuristics intentionally removed. Mode selection should
    // be driven by target characteristics (cross-origin) and observed failure patterns.
}

/// Statistics for ongoing workflow execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowExecutionStats {
    pub progress_percentage: f64,
    pub current_mode: ExecutionMode,
    pub escalation_count: usize,
    pub is_cross_origin: bool,
    pub steps_remaining: usize,
}

/// Summary of completed workflow execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowExecutionSummary {
    pub workflow_id: String,
    pub execution_id: String,
    pub success_rate: f64,
    pub final_mode: ExecutionMode,
    pub total_escalations: usize,
    pub cross_origin_required: bool,
    pub execution_duration: u64,
}

// Add uuid dependency for execution IDs
use uuid;

// Add rand for cleanup demo
use rand;
