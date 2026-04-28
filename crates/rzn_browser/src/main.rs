mod cloud;
mod native_runner;
mod result_formatter;
mod skill_installer;
mod workflow_catalog;
mod workflow_failure_report;

use chrono;
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use cloud::{handle_cloud_commands, CloudCommands};
use colored::Colorize;
use comfy_table::{
    modifiers::UTF8_ROUND_CORNERS,
    presets::{ASCII_FULL, UTF8_FULL_CONDENSED},
    Attribute, Cell, Color, ContentArrangement, Table,
};
use native_runner::{NativeRunConfig, NativeRunMode, SnapshotMode};
use rzn_core::dsl::LogMessage;
use rzn_core::{FieldSpec, Step, StepKind};
use rzn_plan::action_surface::{
    execute_act as action_surface_execute_act, extract as action_surface_extract,
    observe as action_surface_observe, SurfaceActOptions, SurfaceExtractField,
    SurfaceExtractRequest,
};
use rzn_sdk::host::{Host as Orchestrator, HostConfig as PlanConfig, PlanRequest, RunRequest};
use serde::Serialize;
use serde_json::{self, json, Value};
use skill_installer::{
    install_skill, parse_clients, remove_skill, skill_paths, update_skill, SkillInstallRequest,
    SkillInstallScope, SkillRemoveRequest, DEFAULT_SKILL_NAME,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{self, Command};
use terminal_size::{terminal_size, Width};
use url::Url;
use workflow_catalog::{
    compose_workflow_reference, default_user_workflows_dir, detect_catalog_source_root,
    import_user_workflows, install_builtin_catalog_from_repo_root, list_named_workflows_with_query,
    resolve_workflow_reference, workflow_roots, NamedWorkflowEntry, WorkflowCatalogQuery,
};
use workflow_failure_report::{
    build_failure_context_from_error, build_report_body,
    render_failure_report_block as render_report_block, report_success_output, submit_report,
    WorkflowBrokenReportInput, WorkflowFailureReportBody, WorkflowRunFailure,
};

#[derive(Parser, Debug)]
#[command(name = "rzn-browser")]
#[command(about = "RZN Browser standalone CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Plan and execute a workflow using natural language
    Plan(PlanArgs),

    /// Run a workflow using the preferred CLI surface
    Run(RunArgs),

    /// List installed workflow systems and workflows
    List(WorkflowListArgs),

    /// Pure LLM planning without caching or self-healing
    #[command(name = "plan-llm")]
    PlanLlm(PlanArgs),

    /// Full auto planning with caching and self-healing
    #[command(name = "plan-auto")]
    PlanAuto(PlanArgs),

    /// Test browser automation without requiring API key
    #[command(name = "test-browser")]
    TestBrowser(TestBrowserArgs),

    /// Manage browser sessions
    #[command(subcommand)]
    Session(SessionCommands),

    /// Performance monitoring and analysis
    #[command(subcommand)]
    Perf(PerfCommands),

    /// Telemetry and trace analysis
    #[command(subcommand)]
    Telemetry(TelemetryCommands),

    // Removed old autonomous mode - using llm-auto
    /// LLM Autonomous mode - uses static actions for CSP compliance
    #[command(name = "llm-auto")]
    LlmAuto(AutonomousArgs),

    /// Fast, code-first extraction for common sites (no LLM required)
    #[command(name = "quick-extract")]
    QuickExtract(QuickExtractArgs),

    /// Observe page structure and return candidate selectors (no LLM)
    #[command(name = "observe")]
    Observe(ObserveArgs),

    /// Surface-style single action planner + executor
    #[command(name = "act")]
    Act(ActArgs),

    /// Surface-style structured extraction using DOM inventory
    #[command(name = "extract-schema")]
    ExtractSchema(ExtractSchemaArgs),

    /// Surface-style observe wrapper using LLM summarisation
    #[command(name = "observe-llm")]
    ObserveLlm(ObserveCompatArgs),

    /// Deterministic helpers (no LLM)
    #[command(subcommand)]
    Nb(NbCommands),

    /// Manage cached workflows (list/show/run)
    #[command(subcommand)]
    Workflow(WorkflowCommands),

    /// Install, update, remove, and link bundled Agent Skills
    #[command(subcommand)]
    Skill(SkillCommands),

    /// Report broken workflows to RZN
    #[command(subcommand)]
    Report(ReportCommands),

    /// Hosted cloud control plane operations
    #[command(subcommand)]
    Cloud(CloudCommands),
}

#[derive(Args, Debug)]
struct PlanArgs {
    /// Natural language goal to accomplish
    goal: String,

    /// Starting URL (optional)
    #[arg(long)]
    url: Option<String>,

    /// Save the generated workflow for future use
    #[arg(long)]
    save: bool,

    /// Name for the saved workflow (auto-generated if not provided)
    #[arg(long)]
    name: Option<String>,

    /// Maximum number of steps to attempt
    #[arg(long, default_value = "25")]
    max_steps: u32,
}

const DEFAULT_NATIVE_MODE: &str = "auto";
const DEFAULT_SNAPSHOT_MODE: &str = "on-error";

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum RunVia {
    Native,
    Desktop,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum WorkflowSourceArg {
    User,
    Builtin,
    Legacy,
}

impl WorkflowSourceArg {
    fn as_str(self) -> &'static str {
        match self {
            WorkflowSourceArg::User => "user",
            WorkflowSourceArg::Builtin => "builtin",
            WorkflowSourceArg::Legacy => "legacy",
        }
    }
}

#[derive(Args, Debug)]
struct RunArgs {
    #[command(flatten)]
    workflow_ref: WorkflowRefArgs,

    /// Parameters for the workflow (format: --param key=value)
    #[arg(long = "param", value_parser = parse_key_val::<String, String>)]
    params: Vec<(String, String)>,

    /// Browser execution backend
    #[arg(long, value_enum, default_value_t = RunVia::Native)]
    via: RunVia,

    /// Connection mode for native runs: auto | attach | spawn
    #[arg(long, default_value = DEFAULT_NATIVE_MODE)]
    mode: String,

    /// Snapshot mode for native runs: none | after-step | on-error
    #[arg(long, default_value = DEFAULT_SNAPSHOT_MODE)]
    snapshot: String,

    /// Override APP_BASE (used to find broker_endpoint_v1.json)
    #[arg(long)]
    app_base: Option<String>,

    /// Override endpoint path (broker_endpoint_v1.json)
    #[arg(long)]
    endpoint_path: Option<String>,

    /// Worker command for native spawn mode
    #[arg(long)]
    worker_cmd: Option<String>,

    /// Worker args for native spawn mode (repeatable)
    #[arg(long = "worker-arg")]
    worker_args: Vec<String>,

    /// Desktop broker profile (desktop runs only)
    #[arg(long)]
    profile: Option<String>,
}

#[derive(Args, Debug)]
struct NativeRunArgs {
    #[command(flatten)]
    workflow_ref: WorkflowRefArgs,

    /// Parameters for the workflow (format: --param key=value)
    #[arg(long = "param", value_parser = parse_key_val::<String, String>)]
    params: Vec<(String, String)>,

    /// Connection mode: auto | attach | spawn
    #[arg(long, default_value = DEFAULT_NATIVE_MODE)]
    mode: String,

    /// Snapshot mode: none | after-step | on-error
    #[arg(long, default_value = DEFAULT_SNAPSHOT_MODE)]
    snapshot: String,

    /// Override APP_BASE (used to find broker_endpoint_v1.json)
    #[arg(long)]
    app_base: Option<String>,

    /// Override endpoint path (broker_endpoint_v1.json)
    #[arg(long)]
    endpoint_path: Option<String>,

    /// Worker command for spawn mode
    #[arg(long)]
    worker_cmd: Option<String>,

    /// Worker args for spawn mode (repeatable)
    #[arg(long = "worker-arg")]
    worker_args: Vec<String>,
}

#[derive(Args, Debug)]
struct DesktopRunArgs {
    #[command(flatten)]
    workflow_ref: WorkflowRefArgs,

    /// Parameters for the workflow (format: --param key=value)
    #[arg(long = "param", value_parser = parse_key_val::<String, String>)]
    params: Vec<(String, String)>,

    /// Override APP_BASE (used to find broker_endpoint_v1.json)
    #[arg(long)]
    app_base: Option<String>,

    /// Override endpoint path (broker_endpoint_v1.json)
    #[arg(long)]
    endpoint_path: Option<String>,

    /// Desktop broker profile (default: minimal)
    #[arg(long)]
    profile: Option<String>,
}

#[derive(Args, Debug)]
struct TestBrowserArgs {
    /// Starting URL for the test (optional)
    #[arg(long)]
    url: Option<String>,

    /// Test scenario to run
    #[arg(long, default_value = "google-search")]
    scenario: String,
}

#[derive(Args, Debug)]
struct AutonomousArgs {
    /// Natural language instruction to execute
    instruction: String,

    /// Optional starting URL (useful for deterministic tests / local fixtures)
    #[arg(long)]
    url: Option<String>,

    /// Optional context or additional information
    #[arg(long)]
    context: Option<String>,

    /// Maximum number of steps to attempt
    #[arg(long, default_value = "20")]
    max_steps: u32,

    /// Execution constraints (format: --constraint "constraint description")
    #[arg(long = "constraint")]
    constraints: Vec<String>,

    /// Emit machine-readable JSON instead of human output
    #[arg(long)]
    json: bool,

    /// Prefer running a cached workflow before using the LLM
    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    prefer_cached: bool,

    /// Save a workflow after successful LLM execution
    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    save_workflow: bool,

    /// Disable deterministic fast-path macros and run a pure observe→LLM→act loop
    #[arg(long, alias = "no-macros")]
    pure_llm: bool,

    /// Optional name for saved workflow
    #[arg(long)]
    name: Option<String>,
}

#[derive(Subcommand, Debug)]
enum WorkflowCommands {
    /// List installed workflows
    List(WorkflowListArgs),
    /// List installed workflows
    Catalog(WorkflowListArgs),
    /// Validate workflow metadata, parameter docs, and examples
    Validate(WorkflowValidateArgs),
    /// Show workflow storage directories
    Dirs,
    /// Import a JSON workflow file or directory into the user catalog
    Add(WorkflowAddArgs),
    /// Refresh bundled workflows/examples from the repo or a release archive
    Pull(WorkflowPullArgs),
    /// Show a cached workflow by id or file path
    Show(WorkflowShowArgs),
    /// Run a cached workflow by id or file path
    Run(WorkflowRunArgs),
    /// Create a new workflow via interactive builder (or with a template)
    New(WorkflowNewArgs),
}

#[derive(Args, Debug)]
struct WorkflowListArgs {
    /// Optional system namespace filter (e.g. google, x, chatgpt)
    system: Option<String>,

    /// Optional workflow name for detailed help (e.g. `rzn-browser list chatgpt continue-chat-v1`)
    workflow_name: Option<String>,

    /// Limit the list to a single catalog source
    #[arg(long, value_enum)]
    source: Option<WorkflowSourceArg>,

    /// Show shadowed workflows from every source instead of only the effective entry
    #[arg(long)]
    all_sources: bool,

    /// Show ids, aliases, and paths for each workflow
    #[arg(long, short = 'v')]
    verbose: bool,

    /// Emit JSON instead of table output
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct WorkflowAddArgs {
    /// File or directory containing JSON workflows
    source: String,

    /// Workflow system namespace for single-file imports
    #[arg(long)]
    system: Option<String>,

    /// Workflow name for single-file imports
    #[arg(long = "name")]
    workflow_name: Option<String>,

    /// Overwrite existing file(s) in the user catalog
    #[arg(long)]
    force: bool,
}

#[derive(Args, Debug)]
struct WorkflowPullArgs {
    /// Install bundled workflows/examples from a local repo checkout or extracted archive
    #[arg(long)]
    repo_root: Option<String>,

    /// Download a tar.gz catalog archive from this URL before installing
    #[arg(long)]
    url: Option<String>,

    /// GitHub repo to pull from when using --ref or the default release channel
    #[arg(long, default_value = "srv1n/rzn-browser")]
    repo: String,

    /// Pull directly from a GitHub source archive ref instead of the latest release workflows asset
    #[arg(long = "ref")]
    git_ref: Option<String>,
}

#[derive(Subcommand, Debug)]
enum ReportCommands {
    /// Report a broken workflow using only the explicit fields in this command
    #[command(name = "workflow-broken")]
    WorkflowBroken(WorkflowBrokenReportArgs),
}

#[derive(Subcommand, Debug)]
enum SkillCommands {
    /// Install a bundled skill and symlink it into agent-specific skill folders
    Install(SkillInstallArgs),
    /// Refresh an installed skill from the current checkout or bundled runtime copy
    Update(SkillInstallArgs),
    /// Remove symlinks and the managed installed skill copy
    Remove(SkillRemoveArgs),
    /// Show canonical and client-specific skill paths
    Paths(SkillPathsArgs),
}

#[derive(Args, Debug, Clone)]
struct SkillInstallArgs {
    /// Skill folder name
    #[arg(default_value = DEFAULT_SKILL_NAME)]
    skill: String,

    /// Use global scope for the current user
    #[arg(long, conflicts_with = "project")]
    global: bool,

    /// Use project scope for this project
    #[arg(long, conflicts_with = "global")]
    project: bool,

    /// Client(s) to link: all, codex, claude, gemini, agent. Repeat or comma-separate.
    #[arg(long = "client", value_delimiter = ',')]
    clients: Vec<String>,

    /// Project directory for --project; defaults to current directory
    #[arg(long = "project-dir")]
    project_dir: Option<PathBuf>,

    /// Explicit skill source directory containing SKILL.md
    #[arg(long)]
    source: Option<PathBuf>,

    /// Repo root containing skills/<skill>
    #[arg(long = "repo-root")]
    repo_root: Option<PathBuf>,

    /// Replace an existing non-managed install/link
    #[arg(long)]
    force: bool,

    /// Emit JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct SkillRemoveArgs {
    /// Skill folder name to remove
    #[arg(default_value = DEFAULT_SKILL_NAME)]
    skill: String,

    /// Remove global install/link targets
    #[arg(long, conflicts_with = "project")]
    global: bool,

    /// Remove project install/link targets
    #[arg(long, conflicts_with = "global")]
    project: bool,

    /// Client(s) to remove: all, codex, claude, gemini, agent. Defaults to manifest clients, then all.
    #[arg(long = "client", value_delimiter = ',')]
    clients: Vec<String>,

    /// Project directory for --project removes; defaults to current directory
    #[arg(long = "project-dir")]
    project_dir: Option<PathBuf>,

    /// Remove non-managed conflicting paths too
    #[arg(long)]
    force: bool,

    /// Emit JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct SkillPathsArgs {
    /// Skill folder name to inspect
    #[arg(default_value = DEFAULT_SKILL_NAME)]
    skill: String,

    /// Show global install/link targets
    #[arg(long, conflicts_with = "project")]
    global: bool,

    /// Show project install/link targets
    #[arg(long, conflicts_with = "global")]
    project: bool,

    /// Client(s) to show: all, codex, claude, gemini, agent. Repeat or comma-separate.
    #[arg(long = "client", value_delimiter = ',')]
    clients: Vec<String>,

    /// Project directory for --project paths; defaults to current directory
    #[arg(long = "project-dir")]
    project_dir: Option<PathBuf>,

    /// Explicit skill source directory containing SKILL.md
    #[arg(long)]
    source: Option<PathBuf>,

    /// Repo root containing skills/<skill>
    #[arg(long = "repo-root")]
    repo_root: Option<PathBuf>,

    /// Emit JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct WorkflowBrokenReportArgs {
    /// RZN product reporting the failure, for example rzn-browser
    #[arg(long, default_value = "rzn-browser")]
    product: String,

    /// Flow kind, for example workflow
    #[arg(long = "flow-kind", default_value = "workflow")]
    flow_kind: String,

    /// Workflow system namespace, for example google
    #[arg(long)]
    system: String,

    /// Full workflow id, for example google/search-v1
    #[arg(long)]
    workflow: String,

    /// Workflow version, release version, or content hash
    #[arg(long)]
    version: String,

    /// Failed workflow step id
    #[arg(long)]
    step: String,

    /// Stable error code
    #[arg(long)]
    error: String,

    /// RZN app/binary version
    #[arg(long = "app-version")]
    app_version: String,

    /// Platform family, for example macos, windows, or linux
    #[arg(long)]
    platform: String,

    /// Optional context written by the user
    #[arg(long)]
    note: Option<String>,

    /// Print the JSON that would be sent without making a network request
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args, Debug)]
struct WorkflowRefArgs {
    /// Workflow id/path, or the workflow system when using `<system> <workflow>`
    workflow_or_system: String,

    /// Optional workflow name when using `<system> <workflow>`
    workflow_name: Option<String>,
}

#[derive(Args, Debug)]
struct WorkflowShowArgs {
    #[command(flatten)]
    workflow_ref: WorkflowRefArgs,
    /// Output JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct WorkflowValidateArgs {
    #[command(flatten)]
    workflow_ref: WorkflowRefArgs,
    /// Write or refresh the top-level help block before validating
    #[arg(long)]
    write_help: bool,
    /// Output JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct WorkflowRunArgs {
    #[command(flatten)]
    workflow_ref: WorkflowRefArgs,
    /// Parameters for the workflow (format: --param key=value)
    #[arg(long = "param", value_parser = parse_key_val::<String, String>)]
    params: Vec<(String, String)>,
    /// Disable auto-healing if steps fail (enabled by default)
    #[arg(long = "no-auto-heal")]
    no_auto_heal: bool,
}

#[derive(Args, Debug)]
struct WorkflowNewArgs {
    /// Optional template: google-search|google-images|google-scholar
    #[arg(long)]
    template: Option<String>,
}

#[derive(Args, Debug)]
struct QuickExtractArgs {
    /// Site profile to use (google|amazon)
    site: String,

    /// Search query
    query: String,

    /// Number of results to return (default 10)
    #[arg(long, default_value = "10")]
    top: usize,

    /// Prefer CDP-first input strategy for this run
    #[arg(long)]
    cdp_first: bool,
}

#[derive(Args, Debug)]
struct ObserveArgs {
    /// Instruction (e.g., "find product cards", "find search results")
    instruction: String,

    /// Optional CSS scope to limit observation (e.g., "#main, main")
    #[arg(long)]
    scope: Option<String>,

    /// Max candidates to return
    #[arg(long, default_value = "10")]
    max: u32,
}

#[derive(Args, Debug)]
struct ActArgs {
    /// Natural language instruction like "Click the Sign In button"
    instruction: String,

    /// Navigate to URL before planning the action
    #[arg(long)]
    url: Option<String>,

    /// Limit inventory snapshot size
    #[arg(long, default_value = "120")]
    max_inventory: usize,

    /// Scope selector for DOM inventory
    #[arg(long)]
    scope: Option<String>,

    /// Return execution logs as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct ExtractSchemaArgs {
    /// JSON array of fields (e.g. '[{"name":"title"},{"name":"url","attribute":"href"}]')
    #[arg(long)]
    fields: String,

    /// Maximum number of items to return
    #[arg(long, default_value = "10")]
    limit: usize,

    /// Optional CSS scope selector to limit extraction
    #[arg(long)]
    scope: Option<String>,

    /// Return output as JSON only
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct ObserveCompatArgs {
    /// Focus for observation (e.g., "actions to sign in")
    instruction: String,

    /// Optional CSS scope selector
    #[arg(long)]
    scope: Option<String>,

    /// Max observation items to inspect
    #[arg(long, default_value = "120")]
    max_inventory: usize,

    /// Max results to return
    #[arg(long, default_value = "8")]
    max: usize,

    /// Emit JSON only
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand, Debug)]
enum NbCommands {
    /// Extract top-N repeated list items (titles + URLs) from a page
    TopList {
        /// Page URL to open
        url: String,
        /// Number of items to return (default 5)
        #[arg(long, default_value = "5")]
        top: usize,
    },
}

#[derive(Subcommand, Debug)]
enum SessionCommands {
    /// List all sessions
    List {
        /// Filter by session status
        #[arg(long)]
        status: Option<String>,

        /// Maximum number of sessions to display
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Create a new session
    Create {
        /// Session name
        #[arg(long)]
        name: Option<String>,
    },

    /// Suspend an active session
    Suspend {
        /// Session ID
        id: String,
    },

    /// Resume a suspended session
    Resume {
        /// Session ID
        id: String,
    },

    /// Replay a recorded session
    Replay {
        /// Session ID
        id: String,

        /// Playback speed (default: 1.0)
        #[arg(long, default_value = "1.0")]
        speed: f32,
    },
}

#[derive(Subcommand, Debug)]
enum PerfCommands {
    /// Show current performance status
    Status {
        /// Show detailed metrics
        #[arg(long)]
        detailed: bool,
    },

    /// Analyze performance for a specific workflow
    Analyze {
        /// Workflow ID to analyze
        workflow: String,

        /// Time period in hours (default: 24)
        #[arg(long, default_value = "24")]
        hours: u32,
    },

    /// Export performance metrics
    Export {
        /// Export format (json, prometheus, csv)
        #[arg(long, default_value = "json")]
        format: String,

        /// Output file (stdout if not specified)
        #[arg(long)]
        output: Option<String>,
    },

    /// Start the performance dashboard server
    Dashboard {
        /// Port to run the dashboard on
        #[arg(long, default_value = "8080")]
        port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum TelemetryCommands {
    /// List all recorded sessions
    List {
        /// Number of recent sessions to show
        #[arg(long, default_value = "20")]
        limit: usize,

        /// Show only sessions with errors
        #[arg(long)]
        errors_only: bool,

        /// Filter by date (YYYY-MM-DD format)
        #[arg(long)]
        date: Option<String>,
    },

    /// Replay a recorded session
    Replay {
        /// Session ID to replay
        session_id: String,

        /// Include DOM snapshots in output
        #[arg(long)]
        include_dom: bool,

        /// Show only failed steps
        #[arg(long)]
        failures_only: bool,
    },

    /// Generate analytics report
    Analytics {
        /// Number of days to include in report
        #[arg(long, default_value = "7")]
        days: u32,

        /// Output format (json, table, csv)
        #[arg(long, default_value = "table")]
        format: String,

        /// Output file (stdout if not specified)
        #[arg(long)]
        output: Option<String>,
    },

    /// Show cost breakdown and usage statistics
    Cost {
        /// Session ID (shows all sessions if not specified)
        #[arg(long)]
        session_id: Option<String>,

        /// Group by model
        #[arg(long)]
        by_model: bool,

        /// Number of days to include
        #[arg(long, default_value = "30")]
        days: u32,
    },

    /// Clean up old trace files
    Cleanup {
        /// Delete traces older than this many days
        #[arg(long, default_value = "90")]
        older_than_days: u32,

        /// Show what would be deleted without actually deleting
        #[arg(long)]
        dry_run: bool,
    },
}

/// Parse a single key-value pair
fn parse_key_val<T, U>(
    s: &str,
) -> Result<(T, U), Box<dyn std::error::Error + Send + Sync + 'static>>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
    U: std::str::FromStr,
    U::Err: std::error::Error + Send + Sync + 'static,
{
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{}`", s))?;
    Ok((s[..pos].parse()?, s[pos + 1..].parse()?))
}

fn force_dummy_llm(config: &mut PlanConfig) {
    // For deterministic-only runs, we still construct an Orchestrator which initializes an LLM client.
    // Force the dummy provider so users don't need real API keys for workflow smoke tests.
    config.llm_provider = "dummy".to_string();
    config.llm_api_key.clear();

    // Keep a placeholder key for legacy OpenAI paths that still read openai_api_key.
    if config.openai_api_key.is_empty() {
        config.openai_api_key = "dummy-key-for-non-llm-mode".to_string();
    }
}

fn ensure_llm_ready_for_auto_heal(config: &PlanConfig) -> Result<(), String> {
    let provider = config.llm_provider.as_str();
    if provider == "dummy" {
        return Ok(());
    }

    let has_key = match provider {
        // OpenAI supports either key field for backward compatibility.
        "openai" => !config.llm_api_key.is_empty() || !config.openai_api_key.is_empty(),
        _ => !config.llm_api_key.is_empty(),
    };

    if has_key {
        return Ok(());
    }

    let hint = match provider {
        "gemini" => "Set GEMINI_API_KEY (and LLM_PROVIDER=gemini).",
        "anthropic" | "claude" => "Set ANTHROPIC_API_KEY (and LLM_PROVIDER=claude).",
        "groq" => "Set GROQ_API_KEY (and LLM_PROVIDER=groq).",
        _ => "Set OPENAI_API_KEY (and LLM_PROVIDER=openai).",
    };

    Err(format!(
        "Auto-healing requires an API key for LLM_PROVIDER='{}'. {}",
        provider, hint
    ))
}

fn to_engine_plan_config(config: &PlanConfig) -> rzn_plan::PlanConfig {
    rzn_plan::PlanConfig {
        llm_provider: config.llm_provider.clone(),
        openai_api_key: config.openai_api_key.clone(),
        llm_api_key: config.llm_api_key.clone(),
        model: config.model.clone(),
        execution_model: config.execution_model.clone(),
        max_steps: config.max_steps,
        max_healing_attempts: config.max_healing_attempts,
        temperature: config.temperature,
        workflows_dir: config.workflows_dir.clone(),
        max_dom_size: config.max_dom_size,
        llm_timeout: config.llm_timeout,
        broker_transport: config.runtime_transport.clone(),
    }
}

// Simple log writer for CLI with optional metadata
fn write_log(level: &str, message: &str) {
    write_log_with_data(level, message, None);
}

fn write_log_with_data(level: &str, message: &str, data: Option<serde_json::Value>) {
    let log_msg = LogMessage {
        timestamp: chrono::Utc::now().to_rfc3339(),
        level: level.to_string(),
        component: "cli".to_string(),
        message: message.to_string(),
        data,
    };

    if let Some(home) = dirs::home_dir() {
        let log_path = home.join("rzn_build.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let _ = writeln!(
                file,
                "{}",
                serde_json::to_string(&log_msg).unwrap_or_default()
            );
        }
    }
}

#[tokio::main]
async fn main() {
    // Load .env file if it exists
    if let Err(e) = dotenvy::dotenv() {
        if !matches!(e, dotenvy::Error::Io(_)) {
            eprintln!("Warning: Error loading .env file: {}", e);
        }
        // It's OK if .env doesn't exist, we'll use environment variables
    }

    // Write startup log
    write_log("INFO", "Starting RZN CLI");
    write_log("DEBUG", "Debug logging enabled");

    env_logger::init();

    let cli = Cli::parse();
    let mut config = PlanConfig::default();

    // Set transport from environment if available
    if let Ok(transport) = std::env::var("RZN_TRANSPORT") {
        config.runtime_transport = transport;
    }

    match cli.command {
        Commands::Plan(args) => {
            // Legacy plan command - redirects to plan_auto
            handle_plan_auto(args, config).await;
        }
        Commands::Run(args) => {
            if let Err(err) = handle_run(args).await {
                eprintln!("❌ run failed: {}", err);
                process::exit(1);
            }
        }
        Commands::List(args) => {
            if let Err(e) = handle_workflow_catalog(args).await {
                eprintln!("❌ list failed: {}", e);
                process::exit(1);
            }
        }
        Commands::PlanLlm(args) => {
            handle_plan_llm(args, config).await;
        }
        Commands::PlanAuto(args) => {
            handle_plan_auto(args, config).await;
        }
        Commands::TestBrowser(args) => {
            handle_test_browser(args, config).await;
        }
        Commands::Session(cmd) => {
            handle_session_commands(cmd, config).await;
        }
        Commands::Perf(cmd) => {
            handle_perf_commands(cmd).await;
        }
        Commands::Telemetry(cmd) => {
            handle_telemetry_commands(cmd).await;
        }
        // Removed old autonomous command
        Commands::LlmAuto(args) => {
            handle_llm_autonomous(args, config).await;
        }
        Commands::QuickExtract(args) => {
            if let Err(e) = handle_quick_extract(args, config).await {
                eprintln!("❌ quick-extract failed: {}", e);
                process::exit(1);
            }
        }
        Commands::Observe(args) => {
            if let Err(e) = handle_observe(args, config).await {
                eprintln!("❌ observe failed: {}", e);
                process::exit(1);
            }
        }
        Commands::Act(args) => {
            if let Err(e) = handle_action_surface_act(args, config).await {
                eprintln!("❌ act failed: {}", e);
                process::exit(1);
            }
        }
        Commands::ExtractSchema(args) => {
            if let Err(e) = handle_action_surface_extract(args, config).await {
                eprintln!("❌ extract-schema failed: {}", e);
                process::exit(1);
            }
        }
        Commands::ObserveLlm(args) => {
            if let Err(e) = handle_action_surface_observe(args, config).await {
                eprintln!("❌ observe-llm failed: {}", e);
                process::exit(1);
            }
        }
        Commands::Workflow(cmd) => {
            if let Err(e) = handle_workflow_commands(cmd, config).await {
                eprintln!("❌ workflow command failed: {}", e);
                process::exit(1);
            }
        }
        Commands::Skill(cmd) => {
            if let Err(e) = handle_skill_commands(cmd).await {
                eprintln!("❌ skill command failed: {}", e);
                process::exit(1);
            }
        }
        Commands::Report(cmd) => {
            if let Err(e) = handle_report_commands(cmd).await {
                eprintln!("report command failed: {}", e);
                process::exit(1);
            }
        }
        Commands::Cloud(cmd) => {
            if let Err(e) = handle_cloud_commands(cmd).await {
                eprintln!("❌ cloud command failed: {}", e);
                process::exit(1);
            }
        }
        Commands::Nb(cmd) => {
            if let Err(e) = handle_nb(cmd).await {
                eprintln!("❌ nb failed: {}", e);
                process::exit(1);
            }
        }
    }
}

async fn handle_skill_commands(cmd: SkillCommands) -> anyhow::Result<()> {
    match cmd {
        SkillCommands::Install(args) => {
            let json = args.json;
            let req = skill_install_request(args, false)?;
            let summary = install_skill(req)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                render_skill_install_summary("Installed", &summary);
            }
        }
        SkillCommands::Update(args) => {
            let json = args.json;
            let req = skill_install_request(args, true)?;
            let summary = update_skill(req)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                render_skill_install_summary("Updated", &summary);
            }
        }
        SkillCommands::Remove(args) => {
            let json = args.json;
            let req = SkillRemoveRequest {
                skill: args.skill,
                scope: skill_scope(args.global, args.project),
                clients: parse_clients(&args.clients)?,
                project_dir: skill_project_dir(args.project_dir)?,
                force: args.force,
            };
            let summary = remove_skill(req)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                render_skill_remove_summary(&summary);
            }
        }
        SkillCommands::Paths(args) => {
            let json = args.json;
            let summary = skill_paths(
                &args.skill,
                skill_scope(args.global, args.project),
                &skill_project_dir(args.project_dir)?,
                &parse_clients(&args.clients)?,
                args.source.as_deref(),
                args.repo_root.as_deref(),
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                render_skill_paths_summary(&summary);
            }
        }
    }
    Ok(())
}

fn skill_install_request(
    args: SkillInstallArgs,
    update: bool,
) -> anyhow::Result<SkillInstallRequest> {
    Ok(SkillInstallRequest {
        skill: args.skill,
        scope: skill_scope(args.global, args.project),
        clients: parse_clients(&args.clients)?,
        project_dir: skill_project_dir(args.project_dir)?,
        source: args.source,
        repo_root: args.repo_root,
        force: args.force || update,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

fn skill_scope(_global: bool, project: bool) -> SkillInstallScope {
    if project {
        SkillInstallScope::Project
    } else {
        SkillInstallScope::Global
    }
}

fn skill_project_dir(project_dir: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    match project_dir {
        Some(path) => Ok(path),
        None => std::env::current_dir()
            .map_err(|err| anyhow::anyhow!("read current directory: {}", err)),
    }
}

fn render_skill_install_summary(verb: &str, summary: &skill_installer::SkillInstallSummary) {
    println!("{} skill '{}'", verb, summary.skill);
    println!("  scope: {}", summary.scope);
    println!("  version: {}", summary.version);
    println!("  source: {}", summary.source);
    println!("  canonical: {}", summary.canonical_path);
    println!("  content: {}", summary.content_hash);
    println!("  links:");
    for link in &summary.links {
        println!("    - {}: {} -> {}", link.client, link.path, link.target);
    }
}

fn render_skill_remove_summary(summary: &skill_installer::SkillRemoveSummary) {
    println!("Removed skill '{}'", summary.skill);
    println!("  scope: {}", summary.scope);
    println!("  canonical: {}", summary.canonical_path);
    if summary.removed.is_empty() {
        println!("  removed: none");
    } else {
        println!("  removed:");
        for path in &summary.removed {
            println!("    - {}", path);
        }
    }
    if !summary.skipped.is_empty() {
        println!("  skipped:");
        for path in &summary.skipped {
            println!("    - {}", path);
        }
    }
}

fn render_skill_paths_summary(summary: &skill_installer::SkillPathsSummary) {
    println!("Skill paths for '{}'", summary.skill);
    println!("  scope: {}", summary.scope);
    println!("  canonical: {}", summary.canonical_path);
    println!("  source candidates:");
    for path in &summary.source_candidates {
        println!("    - {}", path);
    }
    println!("  client links:");
    for link in &summary.links {
        let status = if link.points_to_target {
            "linked"
        } else if link.exists {
            "exists"
        } else {
            "missing"
        };
        println!(
            "    - {}: {} ({}) -> {}",
            link.client, link.path, status, link.target
        );
    }
}

async fn handle_report_commands(cmd: ReportCommands) -> anyhow::Result<()> {
    match cmd {
        ReportCommands::WorkflowBroken(args) => handle_workflow_broken_report(args).await,
    }
}

async fn handle_workflow_broken_report(args: WorkflowBrokenReportArgs) -> anyhow::Result<()> {
    let body = workflow_report_payload_from_args(&args)?;

    if args.dry_run {
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }

    match submit_report(&body).await {
        Ok(response) => {
            println!("{}", report_success_output(&response));
        }
        Err(_) => {
            println!("Report was not sent. You can ignore this; the original workflow failure is unchanged.");
        }
    }

    Ok(())
}

fn workflow_report_payload_from_args(
    args: &WorkflowBrokenReportArgs,
) -> anyhow::Result<WorkflowFailureReportBody> {
    build_report_body(WorkflowBrokenReportInput {
        product: args.product.clone(),
        flow_kind: args.flow_kind.clone(),
        system: args.system.clone(),
        workflow: args.workflow.clone(),
        version: args.version.clone(),
        step: args.step.clone(),
        error: args.error.clone(),
        app_version: args.app_version.clone(),
        platform: args.platform.clone(),
        note: args.note.clone(),
    })
}

async fn handle_run(args: RunArgs) -> anyhow::Result<()> {
    let RunArgs {
        workflow_ref,
        params,
        via,
        mode,
        snapshot,
        app_base,
        endpoint_path,
        worker_cmd,
        worker_args,
        profile,
    } = args;

    match via {
        RunVia::Native => {
            validate_native_run_surface(profile.as_deref())?;
            handle_native_run(NativeRunArgs {
                workflow_ref,
                params,
                mode,
                snapshot,
                app_base,
                endpoint_path,
                worker_cmd,
                worker_args,
            })
            .await
        }
        RunVia::Desktop => {
            validate_desktop_run_surface(&mode, &snapshot, worker_cmd.as_deref(), &worker_args)?;
            handle_desktop_run(DesktopRunArgs {
                workflow_ref,
                params,
                app_base,
                endpoint_path,
                profile,
            })
            .await
        }
    }
}

fn validate_native_run_surface(profile: Option<&str>) -> anyhow::Result<()> {
    if let Some(profile) = profile.filter(|value| !value.trim().is_empty()) {
        anyhow::bail!(
            "--profile only works with --via desktop (received profile '{}')",
            profile
        );
    }
    Ok(())
}

fn validate_desktop_run_surface(
    mode: &str,
    snapshot: &str,
    worker_cmd: Option<&str>,
    worker_args: &[String],
) -> anyhow::Result<()> {
    if mode.trim().to_ascii_lowercase() != DEFAULT_NATIVE_MODE {
        anyhow::bail!("--mode only works with --via native");
    }
    if snapshot.trim().to_ascii_lowercase() != DEFAULT_SNAPSHOT_MODE {
        anyhow::bail!("--snapshot only works with --via native");
    }
    if worker_cmd
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false)
    {
        anyhow::bail!("--worker-cmd only works with --via native");
    }
    if !worker_args.is_empty() {
        anyhow::bail!("--worker-arg only works with --via native");
    }
    Ok(())
}

async fn handle_desktop_run(args: DesktopRunArgs) -> anyhow::Result<()> {
    let workflow_ref = workflow_ref_value(&args.workflow_ref)?;
    let resolved_workflow = resolve_named_workflow_path(&workflow_ref)?;
    let params = normalize_run_params(args.params.into_iter().collect::<HashMap<_, _>>())?;
    ensure_run_parameters_present(&workflow_ref, &resolved_workflow, &params)?;

    println!("[LIST] RZN BROWSER RUN");
    println!("   ├─ Workflow: {}", workflow_ref);
    println!("   ├─ Resolved: {}", resolved_workflow.display());
    println!("   ├─ Backend: desktop");
    println!(
        "   └─ Profile: {}",
        args.profile.as_deref().unwrap_or("minimal")
    );
    println!();

    if !params.is_empty() {
        println!("[NOTE] Parameters:");
        for (key, value) in &params {
            println!("   ├─ {}: {}", key, value);
        }
        println!();
    }
    let config = native_runner::DesktopRunConfig {
        workflow_path: resolved_workflow.to_string_lossy().to_string(),
        params,
        app_base: args.app_base,
        endpoint_path: args.endpoint_path,
        profile: args.profile,
    };

    match native_runner::run_desktop_workflow(config).await {
        Ok(()) => Ok(()),
        Err(err) => {
            let context = err
                .downcast_ref::<WorkflowRunFailure>()
                .map(|failure| failure.report_context.clone())
                .unwrap_or_else(|| {
                    build_failure_context_from_error(
                        &workflow_ref,
                        &resolved_workflow,
                        &err.to_string(),
                    )
                });
            eprintln!("\n{}", render_report_block(&context));
            Err(err)
        }
    }
}

async fn handle_native_run(args: NativeRunArgs) -> anyhow::Result<()> {
    let workflow_ref = workflow_ref_value(&args.workflow_ref)?;
    let resolved_workflow = resolve_named_workflow_path(&workflow_ref)?;
    let params = normalize_run_params(args.params.into_iter().collect::<HashMap<_, _>>())?;
    ensure_run_parameters_present(&workflow_ref, &resolved_workflow, &params)?;

    println!("[LIST] RZN BROWSER RUN");
    println!("   ├─ Workflow: {}", workflow_ref);
    println!("   ├─ Resolved: {}", resolved_workflow.display());
    println!("   ├─ Backend: native");
    println!("   ├─ Mode: {}", args.mode);
    println!("   └─ Snapshot: {}", args.snapshot);
    println!();

    if !resolved_workflow.exists() {
        anyhow::bail!("Workflow file not found: {}", resolved_workflow.display());
    }
    if !params.is_empty() {
        println!("[NOTE] Parameters:");
        for (key, value) in &params {
            println!("   ├─ {}: {}", key, value);
        }
        println!();
    }

    let mode = match args.mode.to_lowercase().as_str() {
        "attach" => NativeRunMode::Attach,
        "spawn" => NativeRunMode::Spawn,
        _ => NativeRunMode::Auto,
    };

    let snapshot_mode = match args.snapshot.to_lowercase().as_str() {
        "after-step" | "after" => SnapshotMode::AfterStep,
        "none" => SnapshotMode::None,
        _ => SnapshotMode::OnError,
    };

    let config = NativeRunConfig {
        workflow_path: resolved_workflow.to_string_lossy().to_string(),
        params,
        mode,
        snapshot_mode,
        app_base: args.app_base,
        endpoint_path: args.endpoint_path,
        worker_cmd: args.worker_cmd,
        worker_args: args.worker_args,
    };

    match native_runner::run_native_workflow(config).await {
        Ok(()) => Ok(()),
        Err(err) => {
            let context = err
                .downcast_ref::<WorkflowRunFailure>()
                .map(|failure| failure.report_context.clone())
                .unwrap_or_else(|| {
                    build_failure_context_from_error(
                        &workflow_ref,
                        &resolved_workflow,
                        &err.to_string(),
                    )
                });
            eprintln!("\n{}", render_report_block(&context));
            Err(err)
        }
    }
}

async fn handle_plan_llm(args: PlanArgs, config: PlanConfig) {
    println!("🧠 RZN LLM-ONLY PLANNING");
    println!("   ├─ LLM planning: ENABLED");
    println!("   ├─ Workflow caching: DISABLED");
    println!("   ├─ Self-healing: DISABLED");
    println!("   └─ Transport: {}", config.runtime_transport);
    println!();

    let mut orchestrator = match Orchestrator::new(config).await {
        Ok(o) => o,
        Err(e) => {
            eprintln!("[ERROR] Failed to initialize orchestrator: {}", e);
            process::exit(1);
        }
    };

    let request = PlanRequest {
        goal: args.goal.clone(),
        start_url: args.url,
        parameters: HashMap::new(),
        save_workflow: args.save,
        workflow_name: args.name,
    };

    match orchestrator.plan_llm_only(request).await {
        Ok(response) => {
            if response.success {
                println!("[OK] Planning completed successfully!");
                println!(" Steps executed: {}", response.steps_executed);

                if let Some(data) = response.data {
                    let want_md = std::env::var("RZN_OUTPUT").ok().map(|v| v.to_lowercase())
                        == Some("markdown".to_string());
                    if want_md {
                        if let Some(md) = result_formatter::format_markdown_results(&data) {
                            println!("{}", md);
                        } else {
                            println!("{}", result_formatter::format_google_search_results(&data));
                        }
                    } else {
                        println!("{}", result_formatter::format_google_search_results(&data));
                    }
                }

                if let Some(path) = response.workflow_path {
                    println!(" Workflow saved to: {}", path);
                }
            } else {
                println!(
                    "[ERROR] Planning failed: {}",
                    response.error.unwrap_or("Unknown error".to_string())
                );
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("[ERROR] Planning error: {}", e);
            process::exit(1);
        }
    }
}

async fn handle_plan_auto(args: PlanArgs, config: PlanConfig) {
    println!("[BOT] RZN AUTO PLANNING");
    println!("   ├─ LLM planning: ENABLED");
    println!("   ├─ Workflow caching: ENABLED");
    println!("   ├─ Self-healing: ENABLED");
    println!("   ├─ Auto-extraction: ENABLED");
    println!("   └─ Transport: {}", config.runtime_transport);
    println!();

    let mut orchestrator = match Orchestrator::new(config).await {
        Ok(o) => o,
        Err(e) => {
            eprintln!("[ERROR] Failed to initialize orchestrator: {}", e);
            process::exit(1);
        }
    };

    let request = PlanRequest {
        goal: args.goal.clone(),
        start_url: args.url,
        parameters: HashMap::new(),
        save_workflow: args.save,
        workflow_name: args.name,
    };

    match orchestrator.plan_auto(request).await {
        Ok(response) => {
            if response.success {
                println!("[OK] Planning completed successfully!");
                println!(" Steps executed: {}", response.steps_executed);

                if let Some(data) = response.data {
                    println!("{}", result_formatter::format_google_search_results(&data));
                }

                if let Some(path) = response.workflow_path {
                    println!(" Workflow saved to: {}", path);
                }
            } else {
                println!(
                    "[ERROR] Planning failed: {}",
                    response.error.unwrap_or("Unknown error".to_string())
                );
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("[ERROR] Planning error: {}", e);
            process::exit(1);
        }
    }
}

async fn handle_test_browser(args: TestBrowserArgs, config: PlanConfig) {
    println!(" RZN BROWSER AUTOMATION TEST");
    println!("   ├─ Scenario: {}", args.scenario);
    println!("   ├─ URL: {:?}", args.url);
    println!("   ├─ Transport: {}", config.runtime_transport);
    println!("   └─ Mode: Hardcoded test (no LLM required)");
    println!();

    // Create a simple hardcoded workflow for testing
    let test_workflow = match args.scenario.as_str() {
        "google-search" => create_google_search_test_workflow(args.url),
        "simple-navigation" => create_simple_navigation_test_workflow(args.url),
        _ => {
            eprintln!("[ERROR] Unknown test scenario: {}", args.scenario);
            eprintln!("Available scenarios: google-search, simple-navigation");
            process::exit(1);
        }
    };

    // Create a temporary workflow file
    let temp_file = "/tmp/rzn_test_workflow.json";
    match std::fs::write(
        temp_file,
        serde_json::to_string_pretty(&test_workflow).unwrap(),
    ) {
        Ok(_) => {
            println!("[NOTE] Created temporary test workflow: {}", temp_file);
        }
        Err(e) => {
            eprintln!("[ERROR] Failed to create test workflow file: {}", e);
            process::exit(1);
        }
    }

    // Execute the test workflow
    println!("[START] Executing test workflow...");

    let mut config = config;
    force_dummy_llm(&mut config);

    let mut orchestrator = match Orchestrator::new(config).await {
        Ok(o) => o,
        Err(e) => {
            eprintln!("[ERROR] Failed to initialize orchestrator: {}", e);
            process::exit(1);
        }
    };

    let run_request = RunRequest {
        workflow: temp_file.to_string(),
        parameters: HashMap::new(),
        auto_heal: false, // Disable auto-healing for test
    };

    match orchestrator.run(run_request).await {
        Ok(response) => {
            if response.success {
                println!("[OK] Test completed successfully!");
                println!(" Steps executed: {}", response.steps_executed);

                if let Some(data) = response.data {
                    println!("[TARGET] Test data:");
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&data)
                            .unwrap_or_else(|_| "Failed to format data".to_string())
                    );
                }
            } else {
                println!(
                    "[ERROR] Test failed: {}",
                    response.error.unwrap_or("Unknown error".to_string())
                );
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("[ERROR] Test error: {}", e);
            process::exit(1);
        }
    }

    // Clean up temporary file
    let _ = std::fs::remove_file(temp_file);
    println!("🧹 Cleaned up temporary files");
}

fn create_google_search_test_workflow(start_url: Option<String>) -> serde_json::Value {
    let url = start_url.unwrap_or_else(|| "https://www.google.com".to_string());

    json!({
        "id": "test_google_search",
        "name": "Test Google Search",
        "description": "Simple test workflow for Google search",
        "version": "1.0.0",
        "last_updated": chrono::Utc::now().to_rfc3339(),
        "browser_automation": {
            "sequences": [{
                "name": "main",
                "description": "Main test sequence",
                "required_variables": [],
                "steps": [
                    {
                        "id": "step_1",
                        "name": "Navigate to Google",
                        "type": "navigate_to_url",
                        "url": url,
                        "wait": "domcontentloaded"
                    },
                    {
                        "id": "step_2",
                        "name": "Close browser tab",
                        "type": "close_current_tab",
                        "tab_identifier": null
                    }
                ]
            }]
        }
    })
}

fn create_simple_navigation_test_workflow(start_url: Option<String>) -> serde_json::Value {
    let url = start_url.unwrap_or_else(|| "https://example.com".to_string());

    json!({
        "id": "test_simple_navigation",
        "name": "Test Simple Navigation",
        "description": "Simple test workflow for basic navigation",
        "version": "1.0.0",
        "last_updated": chrono::Utc::now().to_rfc3339(),
        "browser_automation": {
            "sequences": [{
                "name": "main",
                "description": "Main test sequence",
                "required_variables": [],
                "steps": [
                    {
                        "id": "step_1",
                        "name": "Navigate to test URL",
                        "type": "navigate_to_url",
                        "url": url,
                        "wait": "domcontentloaded"
                    },
                    {
                        "id": "step_2",
                        "name": "Wait for page to load",
                        "type": "wait_for_timeout",
                        "timeout_ms": 3000
                    },
                    {
                        "id": "step_3",
                        "name": "Close browser tab",
                        "type": "close_current_tab",
                        "tab_identifier": null
                    }
                ]
            }]
        }
    })
}

async fn handle_session_commands(_cmd: SessionCommands, _config: PlanConfig) {
    // Session management temporarily disabled until rzn_session crate is ready
    eprintln!("Session management is not yet available");
    /*
    use rzn_session::{SessionController, SessionCommand, SessionResponse, SessionStatus, SessionConfig};
    use uuid::Uuid;

    println!(" RZN SESSION MANAGEMENT");
    println!("   └─ Transport: {}", config.runtime_transport);
    println!();

    // Initialize session controller
    let session_config = SessionConfig::default();
    let controller = match SessionController::new(session_config).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[ERROR] Failed to initialize session controller: {}", e);
            process::exit(1);
        }
    };

    // Restore sessions on startup
    match controller.restore_sessions().await {
        Ok(restored) => {
            if !restored.is_empty() {
                println!(" Restored {} sessions from disk", restored.len());
            }
        }
        Err(e) => {
            eprintln!("[WARNING]  Warning: Failed to restore sessions: {}", e);
        }
    }

    match cmd {
        SessionCommands::List { status, limit } => {
            let status_filter = status.and_then(|s| match s.to_lowercase().as_str() {
                "active" => Some(SessionStatus::Active),
                "suspended" => Some(SessionStatus::Suspended),
                "terminated" => Some(SessionStatus::Terminated),
                "error" => Some(SessionStatus::Error),
                _ => None,
            });

            let response = controller.handle_command(SessionCommand::List {
                filter: status_filter,
                limit,
            }).await;

            match response {
                Ok(SessionResponse::List { sessions }) => {
                    if sessions.is_empty() {
                        println!("No sessions found");
                    } else {
                        println!("[LIST] Sessions:");
                        for session in sessions {
                            println!("   ├─ {} [{}]",
                                session.id,
                                session.name.as_deref().unwrap_or("unnamed")
                            );
                            println!("   │  Status: {:?}", session.status);
                            println!("   │  Created: {}", session.created_at.format("%Y-%m-%d %H:%M:%S"));
                            if let Some(workflow_id) = &session.workflow_id {
                                println!("   │  Workflow: {}", workflow_id);
                            }
                            println!("   │");
                        }
                    }
                }
                Ok(_) => eprintln!("[ERROR] Unexpected response"),
                Err(e) => eprintln!("[ERROR] Error listing sessions: {}", e),
            }
        }

        SessionCommands::Create { name } => {
            let response = controller.handle_command(SessionCommand::Create {
                name: name.clone(),
                config: None,
            }).await;

            match response {
                Ok(SessionResponse::Created { id, name }) => {
                    println!("[OK] Created session: {}", id);
                    if let Some(n) = name {
                        println!("   Name: {}", n);
                    }
                }
                Ok(SessionResponse::Error { message }) => {
                    eprintln!("[ERROR] Failed to create session: {}", message);
                }
                Ok(_) => eprintln!("[ERROR] Unexpected response"),
                Err(e) => eprintln!("[ERROR] Error creating session: {}", e),
            }
        }

        SessionCommands::Suspend { id } => {
            let session_id = match Uuid::parse_str(&id) {
                Ok(uuid) => uuid,
                Err(_) => {
                    eprintln!("[ERROR] Invalid session ID format");
                    process::exit(1);
                }
            };

            let response = controller.handle_command(SessionCommand::Suspend { id: session_id }).await;

            match response {
                Ok(SessionResponse::Suspended { id }) => {
                    println!("[OK] Suspended session: {}", id);
                }
                Ok(SessionResponse::Error { message }) => {
                    eprintln!("[ERROR] Failed to suspend session: {}", message);
                }
                Ok(_) => eprintln!("[ERROR] Unexpected response"),
                Err(e) => eprintln!("[ERROR] Error suspending session: {}", e),
            }
        }

        SessionCommands::Resume { id } => {
            let session_id = match Uuid::parse_str(&id) {
                Ok(uuid) => uuid,
                Err(_) => {
                    eprintln!("[ERROR] Invalid session ID format");
                    process::exit(1);
                }
            };

            let response = controller.handle_command(SessionCommand::Resume { id: session_id }).await;

            match response {
                Ok(SessionResponse::Resumed { id }) => {
                    println!("[OK] Resumed session: {}", id);
                }
                Ok(SessionResponse::Error { message }) => {
                    eprintln!("[ERROR] Failed to resume session: {}", message);
                }
                Ok(_) => eprintln!("[ERROR] Unexpected response"),
                Err(e) => eprintln!("[ERROR] Error resuming session: {}", e),
            }
        }

        SessionCommands::Replay { id, speed } => {
            let session_id = match Uuid::parse_str(&id) {
                Ok(uuid) => uuid,
                Err(_) => {
                    eprintln!("[ERROR] Invalid session ID format");
                    process::exit(1);
                }
            };

            let response = controller.handle_command(SessionCommand::Replay {
                id: session_id,
                speed
            }).await;

            match response {
                Ok(SessionResponse::ReplayStarted { id }) => {
                    println!("[OK] Started replay for session: {}", id);
                    println!("   Speed: {}x", speed);
                }
                Ok(SessionResponse::Error { message }) => {
                    eprintln!("[ERROR] Failed to start replay: {}", message);
                }
                Ok(_) => eprintln!("[ERROR] Unexpected response"),
                Err(e) => eprintln!("[ERROR] Error starting replay: {}", e),
            }
        }
    }
    */
}

async fn handle_perf_commands(_cmd: PerfCommands) {
    // Performance monitoring temporarily disabled until rzn_telemetry crate is ready
    eprintln!("Performance monitoring is not yet available");
    /*
    use rzn_telemetry::{TelemetryService, TelemetryConfig, ExportFormat};
    use rzn_telemetry::dashboard::DashboardServer;
    use std::sync::Arc;

    println!(" RZN PERFORMANCE MONITORING");
    println!();

    // Initialize telemetry service
    let mut telemetry_config = TelemetryConfig::default();

    // Check if telemetry storage path is set
    if let Ok(storage_path) = std::env::var("RZN_TELEMETRY_PATH") {
        telemetry_config.storage_path = Some(storage_path);
    }

    let telemetry_service = match TelemetryService::new(telemetry_config).await {
        Ok(service) => Arc::new(service),
        Err(e) => {
            eprintln!("[ERROR] Failed to initialize telemetry service: {}", e);
            process::exit(1);
        }
    };

    match cmd {
        PerfCommands::Status { detailed } => {
            match telemetry_service.get_system_metrics().await {
                Ok(metrics) => {
                    println!("  System Status:");
                    println!("   ├─ CPU Usage: {:.1}%", metrics.cpu_usage_percent);
                    println!("   ├─ Memory: {} MB ({:.1}%)",
                        metrics.memory_usage_mb,
                        metrics.memory_usage_percent
                    );
                    println!("   ├─ Active Workflows: {}", metrics.active_workflows);
                    println!("   └─ Pending Actions: {}", metrics.pending_actions);

                    if detailed {
                        println!();
                        println!(" Network:");
                        println!("   ├─ Bytes Sent: {}", format_bytes(metrics.network_bytes_sent));
                        println!("   └─ Bytes Received: {}", format_bytes(metrics.network_bytes_received));
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] Failed to get system metrics: {}", e);
                    process::exit(1);
                }
            }
        }

        PerfCommands::Analyze { workflow, hours } => {
            println!("[SEARCH] Analyzing workflow: {}", workflow);
            println!("   Time period: Last {} hours", hours);
            println!();

            match telemetry_service.analyze_workflow(&workflow).await {
                Ok(report) => {
                    println!(" Performance Report:");
                    println!("   ├─ Total Executions: {}", report.total_executions);
                    println!("   ├─ Success Rate: {:.1}%", report.success_rate * 100.0);
                    println!("   ├─ Average Duration: {:.0}ms", report.average_duration_ms);
                    println!("   ├─ P50 Duration: {:.0}ms", report.p50_duration_ms);
                    println!("   ├─ P95 Duration: {:.0}ms", report.p95_duration_ms);
                    println!("   └─ P99 Duration: {:.0}ms", report.p99_duration_ms);

                    if !report.slowest_actions.is_empty() {
                        println!();
                        println!("🐌 Slowest Actions:");
                        for (i, action) in report.slowest_actions.iter().take(5).enumerate() {
                            println!("   {}. {} - avg: {:.0}ms ({} executions)",
                                i + 1,
                                action.action_type,
                                action.average_duration_ms,
                                action.execution_count
                            );
                        }
                    }

                    if report.failure_analysis.total_failures > 0 {
                        println!();
                        println!("[ERROR] Failure Analysis:");
                        println!("   └─ Total Failures: {}", report.failure_analysis.total_failures);
                    }

                    if !report.recommendations.is_empty() {
                        println!();
                        println!("[TIP] Recommendations:");
                        for rec in &report.recommendations {
                            println!("   • {}", rec);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] Failed to analyze workflow: {}", e);
                    process::exit(1);
                }
            }
        }

        PerfCommands::Export { format, output } => {
            let export_format = match format.as_str() {
                "json" => ExportFormat::Json,
                "prometheus" => ExportFormat::Prometheus,
                "csv" => ExportFormat::Csv,
                _ => {
                    eprintln!("[ERROR] Invalid export format: {}", format);
                    eprintln!("   Supported formats: json, prometheus, csv");
                    process::exit(1);
                }
            };

            match telemetry_service.export_metrics(export_format).await {
                Ok(data) => {
                    if let Some(output_file) = output {
                        match std::fs::write(&output_file, &data) {
                            Ok(_) => {
                                println!("[OK] Exported metrics to: {}", output_file);
                            }
                            Err(e) => {
                                eprintln!("[ERROR] Failed to write file: {}", e);
                                process::exit(1);
                            }
                        }
                    } else {
                        // Output to stdout
                        match String::from_utf8(data) {
                            Ok(s) => println!("{}", s),
                            Err(_) => {
                                eprintln!("[ERROR] Failed to convert export data to string");
                                process::exit(1);
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] Failed to export metrics: {}", e);
                    process::exit(1);
                }
            }
        }

        PerfCommands::Dashboard { port } => {
            println!("[START] Starting performance dashboard on port {}", port);
            println!("   Open http://localhost:{} in your browser", port);
            println!();
            println!("   Press Ctrl+C to stop the server");

            // Start telemetry background collection
            if let Err(e) = telemetry_service.start().await {
                eprintln!("[WARNING]  Warning: Failed to start background collection: {}", e);
            }

            let dashboard = DashboardServer::new(telemetry_service, port);

            // Set up graceful shutdown
            let shutdown = async {
                tokio::signal::ctrl_c()
                    .await
                    .expect("Failed to install CTRL+C signal handler");
                println!("\n👋 Shutting down dashboard server...");
            };

            tokio::select! {
                result = dashboard.start() => {
                    if let Err(e) = result {
                        eprintln!("[ERROR] Dashboard server error: {}", e);
                        process::exit(1);
                    }
                }
                _ = shutdown => {
                    println!("[OK] Dashboard server stopped");
                }
            }
        }
    }
    */
}

async fn handle_telemetry_commands(cmd: TelemetryCommands) {
    use chrono::Local;
    use rzn_plan::telemetry::TraceReplay;
    use std::path::PathBuf;

    println!(" RZN TELEMETRY ANALYSIS");
    println!();

    // Initialize trace replay with default directory
    let traces_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rzn_traces");

    let replay = TraceReplay::new(Some(traces_dir.clone()));

    match cmd {
        TelemetryCommands::List {
            limit,
            errors_only,
            date,
        } => {
            println!("[LIST] Listing recorded sessions...");

            match replay.list_sessions().await {
                Ok(sessions) => {
                    let mut filtered_sessions = sessions;

                    // Filter by errors if requested
                    if errors_only {
                        filtered_sessions.retain(|s| s.error_summary.is_some());
                    }

                    // Filter by date if provided
                    if let Some(date_str) = date {
                        if let Ok(target_date) =
                            chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
                        {
                            filtered_sessions.retain(|s| s.start_time.date_naive() == target_date);
                        } else {
                            eprintln!("[ERROR] Invalid date format. Use YYYY-MM-DD");
                            return;
                        }
                    }

                    // Limit results
                    filtered_sessions.truncate(limit);

                    if filtered_sessions.is_empty() {
                        println!("[NOTE] No sessions found matching criteria");
                        return;
                    }

                    println!("┌─────────────────────┬──────────────────────────────────────┬────────┬────────┬─────────┬──────────┐");
                    println!("│ Session ID          │ Goal                                 │ Steps  │ Success│ Cost    │ Start    │");
                    println!("├─────────────────────┼──────────────────────────────────────┼────────┼────────┼─────────┼──────────┤");

                    for session in filtered_sessions {
                        let session_id = &session.session_id[..20]; // Truncate for display
                        let goal = if session.goal.len() > 36 {
                            format!("{}...", &session.goal[..33])
                        } else {
                            session.goal.clone()
                        };
                        let success_rate = if session.total_steps > 0 {
                            (session.successful_steps as f64 / session.total_steps as f64 * 100.0)
                                as u32
                        } else {
                            0
                        };
                        let start_time = session
                            .start_time
                            .with_timezone(&Local)
                            .format("%m-%d %H:%M");

                        let status_icon = if session.error_summary.is_some() {
                            "[ERROR]"
                        } else {
                            "[OK]"
                        };

                        println!(
                            "│ {:<19} │ {:<36} │ {:>6} │ {:>5}% │ ${:>6.2} │ {} │ {}",
                            session_id,
                            goal,
                            session.total_steps,
                            success_rate,
                            session.total_cost,
                            start_time,
                            status_icon
                        );
                    }

                    println!("└─────────────────────┴──────────────────────────────────────┴────────┴────────┴─────────┴──────────┘");
                    println!("[TIP] Use 'rzn telemetry replay <session_id>' to replay a session");
                }
                Err(e) => {
                    eprintln!("[ERROR] Failed to list sessions: {}", e);
                    std::process::exit(1);
                }
            }
        }

        TelemetryCommands::Replay {
            session_id,
            include_dom,
            failures_only,
        } => {
            println!("🎬 Replaying session: {}", session_id);

            if failures_only {
                println!("   (showing only failed steps)");
            }

            println!();

            if let Err(e) = replay.replay_session(&session_id, include_dom).await {
                eprintln!("[ERROR] Failed to replay session: {}", e);
                std::process::exit(1);
            }
        }

        TelemetryCommands::Analytics {
            days,
            format,
            output,
        } => {
            println!(" Generating analytics report for last {} days...", days);

            match replay.generate_analytics_report(days).await {
                Ok(report) => {
                    let output_content = match format.as_str() {
                        "json" => serde_json::to_string_pretty(&report).unwrap(),
                        "csv" => {
                            // Simple CSV generation for key metrics
                            let mut csv = String::new();
                            csv.push_str("metric,value\n");
                            csv.push_str(&format!("total_sessions,{}\n", report["total_sessions"]));
                            csv.push_str(&format!(
                                "successful_sessions,{}\n",
                                report["successful_sessions"]
                            ));
                            csv.push_str(&format!("success_rate,{:.2}\n", report["success_rate"]));
                            csv.push_str(&format!("total_steps,{}\n", report["total_steps"]));
                            csv.push_str(&format!("total_cost,{:.4}\n", report["total_cost"]));
                            csv.push_str(&format!(
                                "average_cost_per_session,{:.4}\n",
                                report["average_cost_per_session"]
                            ));
                            csv
                        }
                        _ => {
                            // Table format (default)
                            let mut table = String::new();
                            table.push_str(" TELEMETRY ANALYTICS REPORT\n");
                            table.push_str("═══════════════════════════════\n\n");

                            table.push_str(&format!(" Period: Last {} days\n", days));
                            table.push_str(&format!(
                                "🔢 Total Sessions: {}\n",
                                report["total_sessions"]
                            ));
                            table.push_str(&format!(
                                "[OK] Successful Sessions: {}\n",
                                report["successful_sessions"]
                            ));
                            table.push_str(&format!(
                                " Success Rate: {:.1}%\n",
                                report["success_rate"].as_f64().unwrap_or(0.0) * 100.0
                            ));
                            table.push_str(&format!(" Total Steps: {}\n", report["total_steps"]));
                            table.push_str(&format!(" Total Cost: ${:.4}\n", report["total_cost"]));
                            table.push_str(&format!(
                                "💸 Average Cost/Session: ${:.4}\n",
                                report["average_cost_per_session"]
                            ));

                            if let Some(cost_by_model) = report["cost_by_model"].as_object() {
                                table.push_str("\n[BOT] Cost by Model:\n");
                                for (model, cost) in cost_by_model {
                                    table.push_str(&format!(
                                        "   {} ${:.4}\n",
                                        model,
                                        cost.as_f64().unwrap_or(0.0)
                                    ));
                                }
                            }

                            table
                        }
                    };

                    if let Some(output_file) = output {
                        match tokio::fs::write(&output_file, &output_content).await {
                            Ok(_) => println!("[OK] Report saved to: {}", output_file),
                            Err(e) => eprintln!("[ERROR] Failed to write report: {}", e),
                        }
                    } else {
                        println!("{}", output_content);
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] Failed to generate analytics report: {}", e);
                    std::process::exit(1);
                }
            }
        }

        TelemetryCommands::Cost {
            session_id,
            by_model,
            days,
        } => {
            println!(" Cost Analysis");

            if let Some(id) = session_id {
                println!("   Session: {}", id);
                // Load specific session cost data
                match replay.load_session_traces(&id).await {
                    Ok(traces) => {
                        let total_cost: f64 = traces.iter().map(|t| t.step_cost).sum();
                        let mut cost_by_model: std::collections::HashMap<String, f64> =
                            std::collections::HashMap::new();

                        for trace in &traces {
                            if let Some(ref usage) = trace.llm_usage {
                                *cost_by_model.entry(usage.model.clone()).or_insert(0.0) +=
                                    usage.estimated_cost;
                            }
                        }

                        println!();
                        println!("💸 Total Session Cost: ${:.4}", total_cost);

                        if by_model && !cost_by_model.is_empty() {
                            println!("\n[BOT] Cost by Model:");
                            for (model, cost) in cost_by_model {
                                println!("   {}: ${:.4}", model, cost);
                            }
                        }

                        let total_tokens: u32 = traces
                            .iter()
                            .filter_map(|t| t.llm_usage.as_ref())
                            .map(|u| u.total_tokens)
                            .sum();

                        if total_tokens > 0 {
                            println!("🔢 Total Tokens: {}", total_tokens);
                            println!(
                                "[TIP] Average Cost per 1K Tokens: ${:.4}",
                                total_cost / (total_tokens as f64 / 1000.0)
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!("[ERROR] Failed to load session data: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                // Show aggregate cost data for the time period
                match replay.generate_analytics_report(days).await {
                    Ok(report) => {
                        println!("   Period: Last {} days", days);
                        println!();

                        println!("💸 Total Cost: ${:.4}", report["total_cost"]);
                        println!(
                            " Average per Session: ${:.4}",
                            report["average_cost_per_session"]
                        );

                        if by_model {
                            if let Some(cost_by_model) = report["cost_by_model"].as_object() {
                                println!("\n[BOT] Cost by Model:");
                                let mut models: Vec<_> = cost_by_model.iter().collect();
                                models.sort_by(|a, b| {
                                    b.1.as_f64()
                                        .partial_cmp(&a.1.as_f64())
                                        .unwrap_or(std::cmp::Ordering::Equal)
                                });

                                for (model, cost) in models {
                                    println!("   {}: ${:.4}", model, cost.as_f64().unwrap_or(0.0));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[ERROR] Failed to generate cost report: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        }

        TelemetryCommands::Cleanup {
            older_than_days,
            dry_run,
        } => {
            println!("🧹 Cleaning up old trace files...");
            println!("   Criteria: Older than {} days", older_than_days);

            if dry_run {
                println!("   Mode: DRY RUN (no files will be deleted)");
            }

            println!();

            let cutoff = chrono::Utc::now() - chrono::Duration::days(older_than_days as i64);

            match replay.list_sessions().await {
                Ok(sessions) => {
                    let old_sessions: Vec<_> = sessions
                        .into_iter()
                        .filter(|s| s.start_time < cutoff)
                        .collect();

                    if old_sessions.is_empty() {
                        println!("✨ No old trace files found to clean up");
                        return;
                    }

                    println!("  Found {} sessions to clean up:", old_sessions.len());

                    let mut total_size = 0u64;
                    for session in &old_sessions {
                        let session_date =
                            session.start_time.with_timezone(&Local).format("%Y-%m-%d");
                        println!(
                            "   {} - {} ({} steps, ${:.4})",
                            &session.session_id[..20],
                            session_date,
                            session.total_steps,
                            session.total_cost
                        );

                        // Estimate file size (rough approximation)
                        total_size += session.total_steps as u64 * 1024; // ~1KB per step
                    }

                    println!(
                        "\n Estimated space to reclaim: {}",
                        format_bytes(total_size)
                    );

                    if !dry_run {
                        // In a real implementation, you would delete the actual files here
                        println!("[WARNING]  Actual file deletion not implemented in this demo");
                        println!("   Files that would be deleted:");
                        for session in old_sessions {
                            let month_dir = session.start_time.format("%Y-%m");
                            println!(
                                "   - ~/rzn_traces/{}/{}.jsonl",
                                month_dir, session.session_id
                            );
                            println!(
                                "   - ~/rzn_traces/{}/{}.summary.json",
                                month_dir, session.session_id
                            );
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[ERROR] Failed to list sessions for cleanup: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    format!("{:.1} {}", size, UNITS[unit_index])
}

fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_end_matches('.')
        .to_lowercase()
        .strip_prefix("www.")
        .unwrap_or(host.trim())
        .trim_end_matches('.')
        .to_string()
}

fn extract_host_from_url_str(url: &str) -> Option<String> {
    let u = url.trim();
    let after_scheme = u.split("://").nth(1).unwrap_or(u);
    let host_port = after_scheme.split('/').next().unwrap_or("").trim();
    if host_port.is_empty() {
        return None;
    }
    let host = host_port.split('@').last().unwrap_or(host_port);
    let host = host.split(':').next().unwrap_or(host);
    if host.contains('.') {
        Some(normalize_host(host))
    } else {
        None
    }
}

fn extract_first_host_from_instruction(instr: &str) -> Option<String> {
    // Very lightweight parsing: find the first token that looks like a host or URL.
    // This is intentionally generic (no domain allowlists).
    let cleaned = instr.replace(
        ['(', ')', '[', ']', '{', '}', '<', '>', ',', ';', '"', '\''],
        " ",
    );
    for raw in cleaned.split_whitespace() {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        if let Some(host) = extract_host_from_url_str(token) {
            return Some(host);
        }
        if token.contains('.') {
            // Handle bare host like "amazon.com" (strip any path suffix).
            let hostish = token.split('/').next().unwrap_or(token);
            if let Some(host) = extract_host_from_url_str(hostish) {
                return Some(host);
            }
        }
    }
    None
}

fn workflow_first_navigate_host(workflow: &rzn_core::dsl::Workflow) -> Option<String> {
    for seq in &workflow.browser_automation.sequences {
        for step in &seq.steps {
            if let StepKind::NavigateToUrl { url, .. } = &step.kind {
                if let Some(host) = extract_host_from_url_str(url) {
                    return Some(host);
                }
            }
        }
    }
    None
}

// Removed old handle_autonomous function - using handle_llm_autonomous

fn create_llm_client(
    config: &PlanConfig,
) -> Result<rzn_plan::LLMClient, Box<dyn std::error::Error>> {
    let engine_config = to_engine_plan_config(config);
    match rzn_plan::LLMClient::new(&engine_config) {
        Ok(client) => Ok(client),
        Err(e) => Err(Box::new(e)),
    }
}

async fn create_broker_client(
    config: &PlanConfig,
) -> Result<rzn_plan::broker_client::BrokerClient, Box<dyn std::error::Error>> {
    let transport = match config.runtime_transport.as_str() {
        "native" | "endpoint" | "auto" => rzn_plan::broker_client::Transport::Native,
        "tcp" => rzn_plan::broker_client::Transport::Tcp,
        "pipe" => rzn_plan::broker_client::Transport::Pipe,
        _ => rzn_plan::broker_client::Transport::Native,
    };

    let mut broker_client = rzn_plan::broker_client::BrokerClient::new(transport);
    broker_client
        .connect()
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    Ok(broker_client)
}

async fn handle_llm_autonomous(args: AutonomousArgs, config: PlanConfig) {
    if args.json {
        // Ensure libraries don't emit human-friendly stdout that would corrupt JSON output.
        std::env::set_var("RZN_JSON", "1");
    } else {
        println!("🤖 RZN LLM AUTONOMOUS MODE (CSP-Safe)");
        println!("   ├─ Instruction: {}", args.instruction);
        if let Some(u) = &args.url {
            println!("   ├─ Start URL: {}", u);
        }
        println!("   ├─ Max Steps: {}", args.max_steps);
        println!("   ├─ LLM Provider: {}", config.llm_provider);
        println!(
            "   └─ Static Actions: {}",
            if args.pure_llm { "DISABLED" } else { "ENABLED" }
        );
        println!();
    }

    // Try cached workflow first, if requested.
    // If `--url` is provided, we must respect the deterministic start context and skip cached flows
    // (cached workflows may navigate away and invalidate the test/fixture run).
    if args.prefer_cached && args.url.is_none() {
        let engine_config = to_engine_plan_config(&config);
        if let Ok(mut orchestrator) = rzn_plan::Orchestrator::new(engine_config.clone()).await {
            if let Ok(mut wm) =
                rzn_plan::workflow_manager::WorkflowManager::new(&config.workflows_dir)
            {
                if wm.initialize().await.is_ok() {
                    if let Ok(Some(wfid)) = wm.find_similar_workflow(&args.instruction).await {
                        // If the instruction mentions a specific host/domain, only run a cached workflow
                        // that targets that same host (generic safety check to avoid spurious matches).
                        if let Some(requested_host) =
                            extract_first_host_from_instruction(&args.instruction)
                        {
                            if let Ok(wf) = wm.load_workflow(&wfid).await {
                                if let Some(wf_host) = workflow_first_navigate_host(&wf) {
                                    if wf_host != requested_host {
                                        if args.json {
                                            eprintln!(
                                                "[INFO] Skipping cached workflow {} (targets {}) because instruction targets {}",
                                                wfid, wf_host, requested_host
                                            );
                                        } else {
                                            println!(
                                                "[INFO] Skipping cached workflow {} (targets {}) because instruction targets {}",
                                                wfid, wf_host, requested_host
                                            );
                                        }
                                        // Fall through to LLM
                                    } else {
                                        if args.json {
                                            eprintln!("[INFO] Running cached workflow: {}", wfid);
                                        } else {
                                            println!("⚡ Running cached workflow: {}", wfid);
                                        }
                                        let run_req = rzn_plan::RunRequest {
                                            workflow: wfid.clone(),
                                            parameters: HashMap::new(),
                                            auto_heal: false,
                                        };
                                        if let Ok(resp) = orchestrator.run(run_req).await {
                                            if resp.success {
                                                if args.json {
                                                    let out = json!({ "success": true, "cached": true, "steps_executed": resp.steps_executed, "data": resp.data });
                                                    println!(
                                                        "{}",
                                                        serde_json::to_string_pretty(&out).unwrap()
                                                    );
                                                } else {
                                                    println!(
                                                        "✅ Cached workflow succeeded ({} steps)",
                                                        resp.steps_executed
                                                    );
                                                    if let Some(d) = resp.data {
                                                        if let Ok(pretty) =
                                                            serde_json::to_string_pretty(&d)
                                                        {
                                                            println!("{}", pretty);
                                                        }
                                                    }
                                                }
                                                return;
                                            }
                                            if args.json {
                                                eprintln!("[INFO] Cached workflow did not succeed; falling back to LLM.");
                                            } else {
                                                println!("[INFO] Cached workflow did not succeed; falling back to LLM.");
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            if args.json {
                                eprintln!("[INFO] Running cached workflow: {}", wfid);
                            } else {
                                println!("⚡ Running cached workflow: {}", wfid);
                            }
                            let run_req = rzn_plan::RunRequest {
                                workflow: wfid.clone(),
                                parameters: HashMap::new(),
                                auto_heal: false,
                            };
                            if let Ok(resp) = orchestrator.run(run_req).await {
                                if resp.success {
                                    if args.json {
                                        let out = json!({ "success": true, "cached": true, "steps_executed": resp.steps_executed, "data": resp.data });
                                        println!("{}", serde_json::to_string_pretty(&out).unwrap());
                                    } else {
                                        println!(
                                            "✅ Cached workflow succeeded ({} steps)",
                                            resp.steps_executed
                                        );
                                        if let Some(d) = resp.data {
                                            if let Ok(pretty) = serde_json::to_string_pretty(&d) {
                                                println!("{}", pretty);
                                            }
                                        }
                                    }
                                    return;
                                }
                                if args.json {
                                    eprintln!("[INFO] Cached workflow did not succeed; falling back to LLM.");
                                } else {
                                    println!("[INFO] Cached workflow did not succeed; falling back to LLM.");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Verify API key is available (skip for dummy + local CLI providers)
    let provider_type = rzn_plan::llm_provider::ProviderType::from_str(&config.llm_provider);
    let requires_key = provider_type
        .as_ref()
        .map(|p| p.requires_api_key())
        .unwrap_or(true);
    if requires_key && config.llm_api_key.is_empty() && config.openai_api_key.is_empty() {
        eprintln!("[ERROR] API key required for this LLM provider");
        eprintln!("   Set OPENAI_API_KEY / GEMINI_API_KEY (or switch LLM_PROVIDER to dummy/claude-cli/gemini-cli/codex-cli)");
        process::exit(1);
    }

    // Create LLM and broker clients
    let llm_client = match create_llm_client(&config) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("[ERROR] Failed to create LLM client: {}", e);
            process::exit(1);
        }
    };

    let broker_client = match create_broker_client(&config).await {
        Ok(client) => client,
        Err(e) => {
            eprintln!("[ERROR] Failed to create broker client: {}", e);
            process::exit(1);
        }
    };

    // Create LLM autonomous planner (single implementation)
    use rzn_plan::llm_autonomous::{LLMAutonomousPlanner, LLMAutonomousRequest};

    let mut planner = if args.pure_llm {
        LLMAutonomousPlanner::new_with_options(
            llm_client,
            broker_client,
            rzn_plan::llm_autonomous::LLMAutonomousOptions {
                enable_macros: false,
            },
        )
    } else {
        LLMAutonomousPlanner::new(llm_client, broker_client)
    };

    // Build LLM autonomous request
    let request = LLMAutonomousRequest {
        instruction: args.instruction.clone(),
        start_url: args.url.clone(),
        max_steps: Some(args.max_steps as usize),
        timeout_seconds: Some(300), // 5 minute timeout
    };

    if !args.json {
        println!("🚀 Starting LLM autonomous execution...");
        println!();
    }

    // Execute autonomous instruction
    match planner.execute_autonomous(request).await {
        Ok(response) => {
            if args.json {
                let out = json!({
                    "success": response.success,
                    "steps_executed": response.steps_executed,
                    "result": response.result,
                    "extracted_data": response.extracted_data,
                    "error": response.error,
                });
                let pretty = serde_json::to_string_pretty(&out).unwrap_or_else(|_| out.to_string());
                println!("{}", pretty);
                return;
            }

            if response.success {
                println!("✅ LLM autonomous execution completed successfully!");
                println!("   Total steps: {}", response.steps_executed);

                if let Some(data) = &response.extracted_data {
                    println!("\n📊 Extracted Data:");
                    let want_md = std::env::var("RZN_OUTPUT").ok().map(|v| v.to_lowercase())
                        == Some("markdown".to_string());
                    if want_md {
                        if let Some(md) = result_formatter::format_markdown_results(data) {
                            println!("{}", md);
                        } else if let Ok(pretty) = serde_json::to_string_pretty(data) {
                            println!("{}", pretty);
                        }
                    } else if let Some(items) = data.as_array() {
                        let mut printed_md = false;
                        if let Some(first) = items.get(0).and_then(|v| v.as_array()) {
                            for obj in first {
                                if let Some(o) = obj.as_object() {
                                    let title =
                                        o.get("title").and_then(|v| v.as_str()).unwrap_or("");
                                    let href = o
                                        .get("href")
                                        .and_then(|v| v.as_str())
                                        .or_else(|| o.get("url").and_then(|v| v.as_str()))
                                        .unwrap_or("");
                                    let snippet = o
                                        .get("snippet")
                                        .or_else(|| o.get("description"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    if !title.is_empty() && !href.is_empty() {
                                        println!("- [{}]({}) — {}", title, href, snippet);
                                        printed_md = true;
                                    }
                                }
                            }
                        }
                        if !printed_md {
                            for item in items {
                                if let Some(obj) = item.as_object() {
                                    let title =
                                        obj.get("title").and_then(|v| v.as_str()).unwrap_or("");
                                    let href = obj
                                        .get("href")
                                        .and_then(|v| v.as_str())
                                        .or_else(|| obj.get("url").and_then(|v| v.as_str()))
                                        .unwrap_or("");
                                    let snippet = obj
                                        .get("snippet")
                                        .or_else(|| obj.get("description"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    if !title.is_empty() && !href.is_empty() {
                                        println!("- [{}]({}) — {}", title, href, snippet);
                                        printed_md = true;
                                    }
                                }
                            }
                        }
                        if !printed_md {
                            if let Ok(pretty) = serde_json::to_string_pretty(data) {
                                println!("{}", pretty);
                            }
                        }
                    } else if let Ok(pretty) = serde_json::to_string_pretty(data) {
                        println!("{}", pretty);
                    }
                }

                // Save deterministic workflow for reuse
                if args.save_workflow {
                    if let Some(workflow) = planner.export_workflow(&args.instruction) {
                        match rzn_plan::workflow_manager::WorkflowManager::new(
                            &config.workflows_dir,
                        ) {
                            Ok(mut wm) => {
                                let _ = wm.initialize().await;
                                match wm.save_workflow(&workflow).await {
                                    Ok(path) => println!("💾 Cached workflow saved: {}", path),
                                    Err(e) => eprintln!("[WARN] Failed to save workflow: {}", e),
                                }
                            }
                            Err(e) => eprintln!("[WARN] Could not create WorkflowManager: {}", e),
                        }
                    } else {
                        println!("[NOTE] Workflow not saved (no replayable steps recorded)");
                    }
                }
            } else {
                println!("❌ LLM autonomous execution failed");
                if let Some(error) = &response.error {
                    println!("   Error: {}", error);
                }
            }
        }
        Err(e) => {
            if args.json {
                let out = json!({
                    "success": false,
                    "error": e.to_string(),
                });
                let pretty = serde_json::to_string_pretty(&out).unwrap_or_else(|_| out.to_string());
                println!("{}", pretty);
            } else {
                eprintln!("❌ LLM autonomous execution error: {}", e);
            }
            process::exit(1);
        }
    }
}

async fn handle_workflow_commands(
    cmd: WorkflowCommands,
    config: PlanConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        WorkflowCommands::List(args) => handle_workflow_list(args, config).await,
        WorkflowCommands::Catalog(args) => handle_workflow_catalog(args).await,
        WorkflowCommands::Validate(args) => handle_workflow_validate(args).await,
        WorkflowCommands::Dirs => handle_workflow_dirs(),
        WorkflowCommands::Add(args) => handle_workflow_add(args).await,
        WorkflowCommands::Pull(args) => handle_workflow_pull(args).await,
        WorkflowCommands::Show(args) => handle_workflow_show(args, config).await,
        WorkflowCommands::Run(args) => handle_workflow_run(args, config).await,
        WorkflowCommands::New(args) => handle_workflow_new(args, config).await,
    }
}

async fn handle_workflow_list(
    args: WorkflowListArgs,
    _config: PlanConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    handle_workflow_catalog(args).await
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowHelpParamView {
    name: String,
    required: bool,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    shape: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    example: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    sensitive: bool,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowHelpExampleView {
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    command: String,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowHelpView {
    reference: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workflow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    legacy_alias: Option<String>,
    id: String,
    name: String,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_description: Option<String>,
    uses_current_tab: bool,
    parameters: Vec<WorkflowHelpParamView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    examples: Vec<WorkflowHelpExampleView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    returns: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct WorkflowHelpMetadata {
    summary: Option<String>,
    parameters: Vec<WorkflowHelpParamView>,
    examples: Vec<WorkflowHelpExampleView>,
    notes: Vec<String>,
    returns: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum WorkflowValidationLevel {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowValidationIssue {
    level: WorkflowValidationLevel,
    field: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowValidationReport {
    reference: String,
    path: String,
    ok: bool,
    error_count: usize,
    warning_count: usize,
    info_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    issues: Vec<WorkflowValidationIssue>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    wrote_help: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn load_workflow_value(reference: &str) -> anyhow::Result<(PathBuf, Value)> {
    let resolved =
        resolve_named_workflow_path(reference).unwrap_or_else(|_| PathBuf::from(reference));
    let content = fs::read_to_string(&resolved)
        .map_err(|e| anyhow::anyhow!("failed to read workflow {}: {}", resolved.display(), e))?;
    let value: Value = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("invalid workflow JSON in {}: {}", resolved.display(), e))?;
    Ok((resolved, value))
}

fn find_catalog_entry_for_path(path: &std::path::Path) -> Option<NamedWorkflowEntry> {
    let entries = list_named_workflows_with_query(&WorkflowCatalogQuery::default()).ok()?;
    let needle = path.to_string_lossy().to_string();
    entries.into_iter().find(|entry| entry.path == needle)
}

fn build_workflow_help_view(
    reference: &str,
    resolved_path: &std::path::Path,
    value: &Value,
    entry: Option<&NamedWorkflowEntry>,
) -> WorkflowHelpView {
    let help_meta = parse_workflow_help_metadata(value);
    let first_sequence = value
        .pointer("/browser_automation/sequences/0")
        .and_then(|seq| seq.as_object());
    let sequence_name = first_sequence
        .and_then(|seq| seq.get("name"))
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let sequence_description = first_sequence
        .and_then(|seq| seq.get("description"))
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let parameters = build_workflow_help_params(value, &help_meta);

    let system = entry
        .map(|entry| entry.system.clone())
        .or_else(|| read_string_field(value, &["system", "system_id"]));
    let workflow = entry.map(|entry| entry.workflow.clone()).or_else(|| {
        read_string_field(value, &["workflow"]).or_else(|| {
            read_string_field(value, &["id"]).and_then(|id| {
                id.split_once('/')
                    .map(|(_, workflow)| workflow.trim().to_string())
                    .filter(|workflow| !workflow.is_empty())
            })
        })
    });
    let description = help_meta
        .summary
        .clone()
        .or_else(|| read_string_field(value, &["description"]))
        .unwrap_or_else(|| "No description provided.".to_string());
    let uses_current_tab = value
        .pointer("/browser_automation/use_current_tab")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || value
            .pointer("/browser_automation/use_active_tab")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

    let mut notes = help_meta.notes.clone();
    if uses_current_tab {
        notes.push("Uses the current Chrome tab/session instead of opening a separate browser tab by default.".to_string());
    }
    if let Some(source) = entry.map(|entry| entry.source.as_str()) {
        notes.push(format!("Catalog source: {}", source));
    }
    notes.push(format!("Workflow file: {}", resolved_path.display()));

    let examples = if help_meta.examples.is_empty() {
        generate_workflow_examples(
            system.as_deref(),
            workflow.as_deref(),
            reference,
            &parameters,
        )
    } else {
        help_meta.examples.clone()
    };

    WorkflowHelpView {
        reference: reference.to_string(),
        path: resolved_path.display().to_string(),
        source: entry.map(|entry| entry.source.clone()),
        system,
        workflow,
        legacy_alias: entry.map(|entry| entry.legacy_alias.clone()),
        id: read_string_field(value, &["id"]).unwrap_or_else(|| reference.to_string()),
        name: read_string_field(value, &["name"]).unwrap_or_else(|| {
            entry
                .map(|entry| entry.workflow.clone())
                .unwrap_or_else(|| reference.to_string())
        }),
        description,
        version: read_string_field(value, &["version"]),
        updated: read_string_field(value, &["last_updated"]),
        sequence_name,
        sequence_description,
        uses_current_tab,
        parameters,
        examples,
        notes,
        returns: help_meta.returns,
    }
}

fn render_workflow_help_view(view: &WorkflowHelpView) -> String {
    let mut lines = vec![render_primary_heading(
        &view.reference,
        if view.name.trim().is_empty() {
            String::new()
        } else {
            format!(" — {}", view.name)
        },
    )];

    if !view.description.trim().is_empty() {
        lines.push(view.description.clone());
    }

    let mut meta = Vec::new();
    if let Some(source) = view.source.as_deref() {
        meta.push(format!("source: {}", source));
    }
    if let Some(version) = view.version.as_deref().filter(|v| !v.trim().is_empty()) {
        meta.push(format!("version: {}", version));
    }
    if let Some(updated) = view.updated.as_deref().filter(|v| !v.trim().is_empty()) {
        meta.push(format!("updated: {}", updated));
    }
    if view.uses_current_tab {
        meta.push("mode: current-tab".to_string());
    }
    if !meta.is_empty() {
        lines.push(render_meta_line(&format!("meta: {}", meta.join(" | "))));
    }

    if let Some(sequence_name) = view.sequence_name.as_deref() {
        let detail = view
            .sequence_description
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!(" — {}", value))
            .unwrap_or_default();
        lines.push(format!("sequence: {}{}", sequence_name, detail));
    }

    let run_command = build_run_command(
        view.system.as_deref(),
        view.workflow.as_deref(),
        &view.reference,
        &view.parameters,
        false,
    );
    lines.push(String::new());
    lines.push(render_section_heading("Run"));
    lines.push(indent_block(&run_command, 2));

    lines.push(String::new());
    lines.push(render_section_heading("Parameters"));
    if view.parameters.is_empty() {
        lines.push("  none".to_string());
    } else {
        let include_examples = view.parameters.iter().any(|param| {
            param
                .example
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
        });
        let rows: Vec<Vec<Cell>> = view
            .parameters
            .iter()
            .map(|param| {
                let mut row = vec![
                    styled_body_cell(&param.name),
                    styled_status_cell(if param.required {
                        "required"
                    } else {
                        "optional"
                    }),
                    styled_body_cell(param.shape.as_deref().unwrap_or("string")),
                    styled_body_cell(param.default_value.as_deref().unwrap_or("-")),
                    styled_body_cell(&param.description),
                ];
                if include_examples {
                    row.push(styled_body_cell(param.example.as_deref().unwrap_or("-")));
                }
                row
            })
            .collect();
        let mut headers = vec![
            styled_header_cell("parameter"),
            styled_header_cell("input"),
            styled_header_cell("shape"),
            styled_header_cell("default"),
            styled_header_cell("description"),
        ];
        if include_examples {
            headers.push(styled_header_cell("example"));
        }
        lines.push(render_cli_table(headers, rows, 2));
    }

    if !view.examples.is_empty() {
        lines.push(String::new());
        lines.push(render_section_heading("Examples"));
        let rows: Vec<Vec<Cell>> = view
            .examples
            .iter()
            .map(|example| {
                vec![
                    styled_body_cell(
                        example
                            .description
                            .as_deref()
                            .filter(|value| !value.trim().is_empty())
                            .unwrap_or("Example"),
                    ),
                    styled_body_cell(&example.command),
                ]
            })
            .collect();
        lines.push(render_cli_table(
            vec![styled_header_cell("purpose"), styled_header_cell("command")],
            rows,
            2,
        ));
    }

    if let Some(returns) = view
        .returns
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(String::new());
        lines.push(render_section_heading("Returns"));
        lines.push(format!("  {}", returns));
    }

    if !view.notes.is_empty() {
        lines.push(String::new());
        lines.push(render_section_heading("Notes"));
        for note in &view.notes {
            lines.push(format!("  - {}", note));
        }
    }

    lines.join("\n")
}

fn cli_plain_output_requested() -> bool {
    std::env::var_os("RZN_CLI_PLAIN").is_some()
}

fn cli_stdout_is_tty() -> bool {
    io::stdout().is_terminal()
}

fn cli_rich_output_enabled() -> bool {
    if cli_plain_output_requested() || !cli_stdout_is_tty() {
        return false;
    }

    std::env::var("TERM")
        .map(|term| term.trim() != "dumb")
        .unwrap_or(true)
}

fn cli_table_width(indent: usize) -> u16 {
    let width = terminal_size()
        .map(|(Width(width), _)| width as usize)
        .filter(|width| *width >= 60)
        .unwrap_or(100);
    width
        .saturating_sub(indent)
        .clamp(60, 140)
        .min(u16::MAX as usize) as u16
}

fn render_primary_heading(prefix: &str, suffix: String) -> String {
    if cli_rich_output_enabled() {
        format!(
            "{}{}",
            prefix.bold().bright_white(),
            suffix.bold().bright_white()
        )
    } else {
        format!("{}{}", prefix, suffix)
    }
}

fn render_section_heading(title: &str) -> String {
    if cli_rich_output_enabled() {
        title.bold().bright_cyan().to_string()
    } else {
        title.to_string()
    }
}

fn render_meta_line(text: &str) -> String {
    if cli_rich_output_enabled() {
        text.dimmed().to_string()
    } else {
        text.to_string()
    }
}

fn indent_block(block: &str, indent: usize) -> String {
    let prefix = " ".repeat(indent);
    block
        .lines()
        .map(|line| format!("{}{}", prefix, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn styled_header_cell(text: &str) -> Cell {
    let cell = Cell::new(text);
    if cli_rich_output_enabled() {
        cell.fg(Color::Cyan).add_attribute(Attribute::Bold)
    } else {
        cell
    }
}

fn styled_body_cell(text: &str) -> Cell {
    Cell::new(text)
}

fn styled_status_cell(text: &str) -> Cell {
    let cell = Cell::new(text);
    if !cli_rich_output_enabled() {
        return cell;
    }

    match text {
        "required" => cell.fg(Color::Yellow).add_attribute(Attribute::Bold),
        "optional" => cell.fg(Color::Green),
        "ERROR" => cell.fg(Color::Red).add_attribute(Attribute::Bold),
        "WARN" => cell.fg(Color::Yellow).add_attribute(Attribute::Bold),
        "INFO" => cell.fg(Color::Blue),
        "ok" => cell.fg(Color::Green).add_attribute(Attribute::Bold),
        "failed" => cell.fg(Color::Red).add_attribute(Attribute::Bold),
        other => Cell::new(other),
    }
}

fn render_cli_table(headers: Vec<Cell>, rows: Vec<Vec<Cell>>, indent: usize) -> String {
    let mut table = Table::new();
    if cli_rich_output_enabled() {
        table
            .load_preset(UTF8_FULL_CONDENSED)
            .apply_modifier(UTF8_ROUND_CORNERS);
    } else {
        table.load_preset(ASCII_FULL).force_no_tty();
    }
    table
        .set_content_arrangement(ContentArrangement::DynamicFullWidth)
        .set_width(cli_table_width(indent))
        .set_header(headers);
    for row in rows {
        table.add_row(row);
    }
    indent_block(&table.to_string(), indent)
}

fn parse_workflow_help_metadata(value: &Value) -> WorkflowHelpMetadata {
    let Some(help) = value.get("help").and_then(|v| v.as_object()) else {
        return WorkflowHelpMetadata::default();
    };

    let summary = help
        .get("summary")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let returns = help
        .get("returns")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let notes = help
        .get("notes")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let examples = help
        .get("examples")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_object())
                .filter_map(|item| {
                    let command = item
                        .get("command")
                        .and_then(|v| v.as_str())
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())?;
                    let description = item
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty());
                    Some(WorkflowHelpExampleView {
                        description,
                        command,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let parameters = help
        .get("parameters")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_object())
                .filter_map(|item| {
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())?;
                    let description = item
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                        .unwrap_or_else(|| infer_param_description(&name));
                    Some(WorkflowHelpParamView {
                        name,
                        required: item
                            .get("required")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                        description,
                        shape: item
                            .get("shape")
                            .and_then(|v| v.as_str())
                            .map(|v| v.trim().to_string())
                            .filter(|v| !v.is_empty()),
                        default_value: item
                            .get("default")
                            .and_then(|v| v.as_str())
                            .map(|v| v.trim().to_string())
                            .filter(|v| !v.is_empty()),
                        example: item
                            .get("example")
                            .and_then(|v| v.as_str())
                            .map(|v| v.trim().to_string())
                            .filter(|v| !v.is_empty()),
                        sensitive: item
                            .get("sensitive")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    WorkflowHelpMetadata {
        summary,
        parameters,
        examples,
        notes,
        returns,
    }
}

fn build_workflow_help_params(
    value: &Value,
    help_meta: &WorkflowHelpMetadata,
) -> Vec<WorkflowHelpParamView> {
    let required_variables = value
        .pointer("/browser_automation/sequences/0/required_variables")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut required_names = BTreeSet::new();
    let mut params = Vec::new();
    let mut seen = BTreeSet::new();

    for param in &help_meta.parameters {
        if param.required {
            required_names.insert(param.name.clone());
        }
    }

    for variable in &required_variables {
        if let Some(name) = variable.get("name").and_then(|v| v.as_str()) {
            required_names.insert(name.trim().to_string());
        }
    }

    for param in &help_meta.parameters {
        let mut merged = param.clone();
        if required_names.contains(&merged.name) {
            merged.required = true;
        }
        if merged.shape.is_none() {
            merged.shape = infer_param_shape(&merged.name);
        }
        if merged.example.is_none() {
            merged.example = Some(infer_param_example(&merged.name));
        }
        if seen.insert(merged.name.clone()) {
            params.push(merged);
        }
    }

    for variable in required_variables {
        let Some(name) = variable.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let name = name.trim().to_string();
        if name.is_empty() || !seen.insert(name.clone()) {
            continue;
        }
        let description = variable
            .get("description")
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| infer_param_description(&name));
        params.push(WorkflowHelpParamView {
            name: name.clone(),
            required: true,
            description,
            shape: infer_param_shape(&name),
            default_value: None,
            example: Some(infer_param_example(&name)),
            sensitive: variable
                .get("sensitive")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        });
    }

    let mut placeholders = BTreeSet::new();
    collect_placeholders(value, None, &mut placeholders);
    for name in placeholders {
        if !seen.insert(name.clone()) {
            continue;
        }
        let required = required_names.contains(&name);
        params.push(WorkflowHelpParamView {
            name: name.clone(),
            required,
            description: infer_param_description(&name),
            shape: infer_param_shape(&name),
            default_value: None,
            example: Some(infer_param_example(&name)),
            sensitive: infer_param_sensitive(&name),
        });
    }

    params.sort_by(|left, right| {
        right
            .required
            .cmp(&left.required)
            .then_with(|| left.name.cmp(&right.name))
    });
    params
}

fn collect_placeholders(value: &Value, parent_key: Option<&str>, out: &mut BTreeSet<String>) {
    match value {
        Value::String(text) => {
            if parent_key == Some("script") {
                return;
            }
            scan_placeholders_in_string(text, out);
        }
        Value::Array(items) => {
            for item in items {
                collect_placeholders(item, parent_key, out);
            }
        }
        Value::Object(map) => {
            for (key, child) in map {
                collect_placeholders(child, Some(key.as_str()), out);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn scan_placeholders_in_string(text: &str, out: &mut BTreeSet<String>) {
    let bytes = text.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() {
        if bytes[idx] != b'{' {
            idx += 1;
            continue;
        }
        let start = idx + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != b'}' {
            end += 1;
        }
        if end >= bytes.len() {
            break;
        }
        let candidate = &text[start..end];
        if is_valid_placeholder_name(candidate) {
            out.insert(candidate.to_string());
        }
        idx = end + 1;
    }
}

fn is_valid_placeholder_name(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn read_string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn infer_param_description(name: &str) -> String {
    match name {
        "search_query" | "query" => {
            "Plain-language query string typed into the site's search box.".to_string()
        }
        "message_text" => "Message or prompt text to send.".to_string(),
        "reply_text" => "Reply text to type or submit.".to_string(),
        "comment_text" => "Comment text to type or submit.".to_string(),
        "post_text" => "Body text for the post.".to_string(),
        "post_title" => "Title to use for the post.".to_string(),
        "post_url" | "item_url" | "thread_url" => "Target page URL for the workflow.".to_string(),
        "chat_id" => "Conversation id from the target app URL.".to_string(),
        "attachment_file_path" => "Absolute path to the local file to upload.".to_string(),
        "recipient" | "recipient_handle" | "handle" => {
            "Target username or handle, usually without the leading @.".to_string()
        }
        "since_date" => {
            "Inclusive start date for the query window in YYYY-MM-DD format.".to_string()
        }
        "until_date" => "Inclusive end date for the query window in YYYY-MM-DD format.".to_string(),
        "timeline_mode" => "Timeline variant to use, such as `top` or `live`.".to_string(),
        "model_slug" => "Model label to select before sending, if the app supports it.".to_string(),
        "model_effort" => {
            "Reasoning effort label to select before sending, if the app supports it.".to_string()
        }
        _ if name.ends_with("_url") => "URL value to pass into the workflow.".to_string(),
        _ if name.ends_with("_id") => "Identifier value used by the workflow.".to_string(),
        _ if name.ends_with("_path") => "Absolute local path used by the workflow.".to_string(),
        _ if name.ends_with("_date") => "Date value in YYYY-MM-DD format.".to_string(),
        _ if name.ends_with("_text") => "Text value used by the workflow.".to_string(),
        _ if name.ends_with("_count") || name.starts_with("max_") => {
            "Numeric limit used by the workflow.".to_string()
        }
        _ => format!("Value for `{}`.", name),
    }
}

fn infer_param_shape(name: &str) -> Option<String> {
    let shape = match name {
        "search_query" | "query" | "message_text" | "reply_text" | "comment_text" | "post_text"
        | "post_title" => "text",
        "chat_id" => "string id",
        "model_slug" | "model_effort" => "string",
        _ if name.ends_with("_url") => "url",
        _ if name.ends_with("_id") => "string id",
        _ if name.ends_with("_path") => "absolute path",
        _ if name.ends_with("_date") => "YYYY-MM-DD",
        _ if name.ends_with("_count") || name.starts_with("max_") => "integer",
        _ => "string",
    };
    Some(shape.to_string())
}

fn infer_param_example(name: &str) -> String {
    match name {
        "search_query" | "query" => "rust browser automation".to_string(),
        "message_text" => "Summarize the last three commits.".to_string(),
        "reply_text" => "Thanks — this is useful context.".to_string(),
        "comment_text" => "Interesting take. Here is the part I agree with.".to_string(),
        "post_text" => "Here is the body text.".to_string(),
        "post_title" => "Useful link worth sharing".to_string(),
        "post_url" | "item_url" | "thread_url" => "https://example.com/post".to_string(),
        "chat_id" => "01234567-89ab-cdef-0123-456789abcdef".to_string(),
        "attachment_file_path" => "/absolute/path/to/file.txt".to_string(),
        "recipient" | "recipient_handle" | "handle" => "example_user".to_string(),
        "model_slug" => "Pro".to_string(),
        "model_effort" => "Extended".to_string(),
        _ if name.ends_with("_url") => "https://example.com".to_string(),
        _ if name.ends_with("_id") => "1234567890".to_string(),
        _ if name.ends_with("_path") => "/absolute/path/to/file".to_string(),
        _ if name.ends_with("_date") => "2026-04-23".to_string(),
        _ if name.ends_with("_count") || name.starts_with("max_") => "10".to_string(),
        _ => format!("example_{}", name),
    }
}

fn infer_param_sensitive(name: &str) -> bool {
    matches!(
        name,
        "password" | "api_key" | "token" | "secret" | "session_token"
    ) || name.ends_with("_token")
        || name.ends_with("_secret")
        || name.ends_with("_password")
}

fn generate_workflow_examples(
    system: Option<&str>,
    workflow: Option<&str>,
    reference: &str,
    parameters: &[WorkflowHelpParamView],
) -> Vec<WorkflowHelpExampleView> {
    vec![WorkflowHelpExampleView {
        description: Some("Basic run with required parameters.".to_string()),
        command: build_run_command(system, workflow, reference, parameters, false),
    }]
}

fn build_run_command(
    system: Option<&str>,
    workflow: Option<&str>,
    reference: &str,
    parameters: &[WorkflowHelpParamView],
    include_optional: bool,
) -> String {
    let target = match (system, workflow) {
        (Some(system), Some(workflow)) => format!("{} {}", system, workflow),
        _ => reference.to_string(),
    };
    let mut command = format!("rzn-browser run {}", target);
    for param in parameters {
        if !param.required && !include_optional {
            continue;
        }
        let value = param
            .example
            .clone()
            .unwrap_or_else(|| infer_param_example(&param.name));
        command.push_str(&format!(" --param {}=\"{}\"", param.name, value));
    }
    command
}

fn ensure_run_parameters_present(
    workflow_ref: &str,
    resolved_path: &std::path::Path,
    params: &HashMap<String, String>,
) -> anyhow::Result<()> {
    let (_, value) = load_workflow_value(&resolved_path.display().to_string())?;
    let entry = find_catalog_entry_for_path(resolved_path);
    let view = build_workflow_help_view(workflow_ref, resolved_path, &value, entry.as_ref());
    let missing = view
        .parameters
        .iter()
        .filter(|param| param.required && !params.contains_key(&param.name))
        .map(|param| param.name.clone())
        .collect::<Vec<_>>();

    if missing.is_empty() {
        return Ok(());
    }

    anyhow::bail!(
        "missing required parameters: {}\n\n{}",
        missing.join(", "),
        render_workflow_help_view(&view)
    );
}

fn synthesize_workflow_help_value(
    reference: &str,
    resolved_path: &std::path::Path,
    value: &Value,
    entry: Option<&NamedWorkflowEntry>,
) -> Value {
    let existing = parse_workflow_help_metadata(value);
    let view = build_workflow_help_view(reference, resolved_path, value, entry);
    let summary = existing
        .summary
        .clone()
        .unwrap_or_else(|| view.description.clone());
    let examples = if existing.examples.is_empty() {
        view.examples.clone()
    } else {
        existing.examples.clone()
    };
    let notes = if existing.notes.is_empty() {
        view.notes
            .iter()
            .filter(|note| {
                !note.starts_with("Catalog source: ") && !note.starts_with("Workflow file: ")
            })
            .cloned()
            .collect::<Vec<_>>()
    } else {
        existing.notes.clone()
    };
    let returns = existing.returns.clone().or(view.returns.clone());

    json!({
        "summary": summary,
        "parameters": view.parameters.iter().map(|param| {
            let mut out = serde_json::Map::new();
            out.insert("name".to_string(), Value::String(param.name.clone()));
            out.insert("required".to_string(), Value::Bool(param.required));
            out.insert("description".to_string(), Value::String(param.description.clone()));
            if let Some(shape) = &param.shape {
                out.insert("shape".to_string(), Value::String(shape.clone()));
            }
            if let Some(default_value) = &param.default_value {
                out.insert("default".to_string(), Value::String(default_value.clone()));
            }
            if let Some(example) = &param.example {
                out.insert("example".to_string(), Value::String(example.clone()));
            }
            if param.sensitive {
                out.insert("sensitive".to_string(), Value::Bool(true));
            }
            Value::Object(out)
        }).collect::<Vec<_>>(),
        "examples": examples.iter().map(|example| {
            let mut out = serde_json::Map::new();
            if let Some(description) = &example.description {
                out.insert("description".to_string(), Value::String(description.clone()));
            }
            out.insert("command".to_string(), Value::String(example.command.clone()));
            Value::Object(out)
        }).collect::<Vec<_>>(),
        "returns": returns,
        "notes": notes
    })
}

fn write_workflow_help_block(
    reference: &str,
    resolved_path: &std::path::Path,
    value: &Value,
    entry: Option<&NamedWorkflowEntry>,
) -> anyhow::Result<Value> {
    let mut updated = value.clone();
    let help = synthesize_workflow_help_value(reference, resolved_path, value, entry);
    let Some(root) = updated.as_object_mut() else {
        anyhow::bail!("workflow root must be a JSON object");
    };
    root.insert("help".to_string(), help);
    fs::write(
        resolved_path,
        serde_json::to_string_pretty(&updated)
            .map_err(|e| anyhow::anyhow!("failed to serialize workflow JSON: {}", e))?,
    )
    .map_err(|e| {
        anyhow::anyhow!(
            "failed to write workflow {}: {}",
            resolved_path.display(),
            e
        )
    })?;
    Ok(updated)
}

fn validate_workflow_help_contract(
    reference: &str,
    resolved_path: &std::path::Path,
    value: &Value,
) -> WorkflowValidationReport {
    let mut issues = Vec::new();
    let description = read_string_field(value, &["description"]);
    let name = read_string_field(value, &["name"]);
    let help_meta = parse_workflow_help_metadata(value);
    let explicit_param_names = help_meta
        .parameters
        .iter()
        .map(|param| param.name.clone())
        .collect::<BTreeSet<_>>();
    let inferred_params = build_workflow_help_params(value, &help_meta);
    let required_names = value
        .pointer("/browser_automation/sequences/0/required_variables")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|variable| variable.get("name").and_then(|v| v.as_str()))
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect::<BTreeSet<_>>();

    if name.is_none() {
        issues.push(WorkflowValidationIssue {
            level: WorkflowValidationLevel::Error,
            field: "name".to_string(),
            message: "Missing top-level `name`.".to_string(),
            suggestion: Some("Add a short human-readable workflow name.".to_string()),
        });
    }
    if description.is_none() {
        issues.push(WorkflowValidationIssue {
            level: WorkflowValidationLevel::Error,
            field: "description".to_string(),
            message: "Missing top-level `description`.".to_string(),
            suggestion: Some("Add one sentence describing what the workflow does.".to_string()),
        });
    }
    if value.get("help").is_none() {
        issues.push(WorkflowValidationIssue {
            level: WorkflowValidationLevel::Error,
            field: "help".to_string(),
            message: "Missing top-level `help` block.".to_string(),
            suggestion: Some(
                "Run `rzn-browser workflow validate <path> --write-help` to scaffold it."
                    .to_string(),
            ),
        });
    }
    if help_meta.summary.is_none() {
        issues.push(WorkflowValidationIssue {
            level: WorkflowValidationLevel::Warning,
            field: "help.summary".to_string(),
            message: "Missing `help.summary`.".to_string(),
            suggestion: Some("Add a short CLI-facing summary for the workflow.".to_string()),
        });
    }
    if help_meta.examples.is_empty() {
        issues.push(WorkflowValidationIssue {
            level: WorkflowValidationLevel::Error,
            field: "help.examples".to_string(),
            message: "Missing `help.examples`.".to_string(),
            suggestion: Some("Add at least one runnable command example.".to_string()),
        });
    }

    for param in &inferred_params {
        if !explicit_param_names.contains(&param.name) {
            issues.push(WorkflowValidationIssue {
                level: WorkflowValidationLevel::Error,
                field: format!("help.parameters.{}", param.name),
                message: format!("Parameter `{}` is used by the workflow but not documented in `help.parameters`.", param.name),
                suggestion: Some(format!(
                    "Add a `help.parameters` entry for `{}` or run `rzn-browser workflow validate {} --write-help`.",
                    param.name, reference
                )),
            });
        }
    }

    for param in &help_meta.parameters {
        if param.required && !required_names.contains(&param.name) {
            issues.push(WorkflowValidationIssue {
                level: WorkflowValidationLevel::Warning,
                field: format!("help.parameters.{}", param.name),
                message: format!(
                    "`help.parameters` marks `{}` as required, but it is not declared in `required_variables`.",
                    param.name
                ),
                suggestion: Some("Either add it to `required_variables` or mark it optional in `help.parameters`.".to_string()),
            });
        }
        if param.description.trim().is_empty() {
            issues.push(WorkflowValidationIssue {
                level: WorkflowValidationLevel::Error,
                field: format!("help.parameters.{}", param.name),
                message: format!("Parameter `{}` is missing a description.", param.name),
                suggestion: Some(
                    "Add a short description that explains what the value means.".to_string(),
                ),
            });
        }
    }

    let error_count = issues
        .iter()
        .filter(|issue| matches!(issue.level, WorkflowValidationLevel::Error))
        .count();
    let warning_count = issues
        .iter()
        .filter(|issue| matches!(issue.level, WorkflowValidationLevel::Warning))
        .count();
    let info_count = issues
        .iter()
        .filter(|issue| matches!(issue.level, WorkflowValidationLevel::Info))
        .count();

    WorkflowValidationReport {
        reference: reference.to_string(),
        path: resolved_path.display().to_string(),
        ok: error_count == 0,
        error_count,
        warning_count,
        info_count,
        issues,
        wrote_help: false,
    }
}

fn render_workflow_validation_report(report: &WorkflowValidationReport) -> String {
    let mut lines = vec![render_primary_heading(
        "Workflow validation: ",
        if report.ok { "ok" } else { "failed" }.to_string(),
    )];
    lines.push(render_meta_line(&format!(
        "reference: {}",
        report.reference
    )));
    lines.push(render_meta_line(&format!("path: {}", report.path)));
    lines.push(render_meta_line(&format!(
        "issues: {} error(s), {} warning(s), {} info",
        report.error_count, report.warning_count, report.info_count
    )));
    if report.wrote_help {
        lines.push(render_meta_line(
            "help: wrote or refreshed top-level help metadata",
        ));
    }
    if report.issues.is_empty() {
        lines.push("No issues found.".to_string());
        return lines.join("\n");
    }

    lines.push(String::new());
    lines.push(render_section_heading("Issues"));
    let rows: Vec<Vec<Cell>> = report
        .issues
        .iter()
        .map(|issue| {
            let level = match issue.level {
                WorkflowValidationLevel::Error => "ERROR",
                WorkflowValidationLevel::Warning => "WARN",
                WorkflowValidationLevel::Info => "INFO",
            };
            vec![
                styled_status_cell(level),
                styled_body_cell(&issue.field),
                styled_body_cell(&issue.message),
                styled_body_cell(issue.suggestion.as_deref().unwrap_or("-")),
            ]
        })
        .collect();
    lines.push(render_cli_table(
        vec![
            styled_header_cell("level"),
            styled_header_cell("field"),
            styled_header_cell("message"),
            styled_header_cell("fix"),
        ],
        rows,
        2,
    ));
    lines.join("\n")
}

async fn handle_workflow_show(
    args: WorkflowShowArgs,
    config: PlanConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let workflow_ref = workflow_ref_value(&args.workflow_ref)?;
    let (resolved_path, value) = load_workflow_value(&workflow_ref)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    let _ = config;
    let entry = find_catalog_entry_for_path(&resolved_path);
    let view = build_workflow_help_view(&workflow_ref, &resolved_path, &value, entry.as_ref());
    println!("{}", render_workflow_help_view(&view));
    Ok(())
}

async fn handle_workflow_validate(
    args: WorkflowValidateArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let workflow_ref = workflow_ref_value(&args.workflow_ref)?;
    let (resolved_path, mut value) = load_workflow_value(&workflow_ref)?;
    let entry = find_catalog_entry_for_path(&resolved_path);

    if args.write_help {
        value = write_workflow_help_block(&workflow_ref, &resolved_path, &value, entry.as_ref())?;
    }

    let mut report = validate_workflow_help_contract(&workflow_ref, &resolved_path, &value);
    report.wrote_help = args.write_help;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", render_workflow_validation_report(&report));
    }

    if report.ok {
        return Ok(());
    }

    Err(Box::<dyn std::error::Error>::from(anyhow::anyhow!(
        "workflow validation failed with {} error(s)",
        report.error_count
    )))
}

async fn handle_workflow_run(
    args: WorkflowRunArgs,
    mut config: PlanConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let workflow_ref = workflow_ref_value(&args.workflow_ref)?;
    let auto_heal = !args.no_auto_heal;
    if auto_heal {
        if let Err(msg) = ensure_llm_ready_for_auto_heal(&config) {
            return Err(Box::<dyn std::error::Error>::from(msg));
        }
    } else {
        force_dummy_llm(&mut config);
    }

    let mut orch = Orchestrator::new(config.clone())
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    let mut params_map: HashMap<String, String> = HashMap::new();
    for (k, v) in args.params {
        params_map.insert(k, v);
    }
    params_map = normalize_run_params(params_map)?;

    // Built-in generator: "builtin/google-search"
    let wf_identifier = if workflow_ref == "builtin/google-search" {
        match generate_google_search_workflow(&params_map) {
            Ok(path) => path,
            Err(e) => {
                return Err(Box::<dyn std::error::Error>::from(format!(
                    "failed to generate builtin workflow: {}",
                    e
                )));
            }
        }
    } else {
        resolve_named_workflow_path(&workflow_ref)
            .unwrap_or_else(|_| PathBuf::from(workflow_ref.clone()))
            .to_string_lossy()
            .to_string()
    };

    let req = RunRequest {
        workflow: wf_identifier.clone(),
        parameters: params_map,
        auto_heal,
    };
    let resp = orch
        .run(req)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    if resp.success {
        println!(
            "✅ Workflow '{}' ran successfully ({} steps)",
            workflow_ref, resp.steps_executed
        );
        if let Some(data) = resp.data {
            println!("{}", serde_json::to_string_pretty(&data)?);
        }
    } else {
        println!("❌ Workflow '{}' failed", workflow_ref);
        let err = anyhow::anyhow!(
            "{}",
            resp.error
                .unwrap_or_else(|| "workflow execution failed".to_string())
        );
        let context = build_failure_context_from_error(
            &workflow_ref,
            std::path::Path::new(&wf_identifier),
            &err.to_string(),
        );
        eprintln!("\n{}", render_report_block(&context));
    }
    Ok(())
}

async fn handle_workflow_catalog(args: WorkflowListArgs) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(workflow_name) = args.workflow_name.as_deref() {
        let system = args.system.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "workflow help requires both a system and workflow name, for example `rzn-browser list chatgpt continue-chat-v1`"
            )
        })?;
        let workflow_ref = compose_workflow_reference(system, Some(workflow_name))?;
        let (resolved_path, value) = load_workflow_value(&workflow_ref)?;
        let entry = find_catalog_entry_for_path(&resolved_path);
        let view = build_workflow_help_view(&workflow_ref, &resolved_path, &value, entry.as_ref());
        if args.json {
            println!("{}", serde_json::to_string_pretty(&view)?);
        } else {
            println!("{}", render_workflow_help_view(&view));
        }
        return Ok(());
    }

    let entries = list_named_workflows_with_query(&WorkflowCatalogQuery {
        system_filter: args.system.clone(),
        source_filter: args.source.map(|source| source.as_str().to_string()),
        include_all_sources: args.all_sources,
    })?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }
    println!("{}", render_workflow_catalog(&entries, &args));
    Ok(())
}

fn render_workflow_catalog(entries: &[NamedWorkflowEntry], args: &WorkflowListArgs) -> String {
    if entries.is_empty() {
        return render_empty_workflow_catalog(args);
    }

    let mut by_system: BTreeMap<&str, Vec<&NamedWorkflowEntry>> = BTreeMap::new();
    for entry in entries {
        by_system.entry(&entry.system).or_default().push(entry);
    }

    let workflow_label = if entries.len() == 1 {
        "workflow"
    } else {
        "workflows"
    };
    let system_label = if by_system.len() == 1 {
        "system"
    } else {
        "systems"
    };
    let source_counts = format_source_counts(entries);
    let unique_sources: BTreeSet<&str> =
        entries.iter().map(|entry| entry.source.as_str()).collect();
    let show_source_column = args.all_sources || args.source.is_some() || unique_sources.len() > 1;
    let show_details_column = args.verbose
        || entries
            .iter()
            .any(|entry| !entry.overrides_sources.is_empty() || entry.shadowed_by_source.is_some());
    let catalog_entry_label = if args.all_sources || args.source.is_some() {
        "entries"
    } else {
        workflow_label
    };

    let mut lines = vec![render_primary_heading(
        "Installed workflows: ",
        format!(
            "{} {} across {} {} ({})",
            entries.len(),
            catalog_entry_label,
            by_system.len(),
            system_label,
            source_counts
        ),
    )];

    for (system, workflows) in by_system {
        let system_workflow_label =
            if workflows.len() == 1 && !(args.all_sources || args.source.is_some()) {
                "workflow"
            } else if workflows.len() == 1 {
                "entry"
            } else {
                if args.all_sources || args.source.is_some() {
                    "entries"
                } else {
                    "workflows"
                }
            };
        lines.push(String::new());
        lines.push(render_section_heading(&format!(
            "{} ({} {})",
            system,
            workflows.len(),
            system_workflow_label
        )));

        let rows: Vec<Vec<Cell>> = workflows
            .iter()
            .map(|entry| {
                let mut details = Vec::new();
                if !entry.overrides_sources.is_empty() {
                    details.push(format!("overrides {}", entry.overrides_sources.join(", ")));
                }
                if let Some(shadowed_by) = entry.shadowed_by_source.as_deref() {
                    details.push(format!("shadowed by {}", shadowed_by));
                }
                if args.verbose {
                    details.push(format!("id {}", entry.id));
                    details.push(format!("legacy {}", entry.legacy_alias));
                    details.push(format!("rel {}", entry.relative_path));
                    details.push(format!("path {}", entry.path));
                }
                let detail_text = if details.is_empty() {
                    "-".to_string()
                } else {
                    details.join(" | ")
                };
                let mut row = vec![
                    styled_body_cell(&entry.workflow),
                    styled_body_cell(entry.name.as_deref().unwrap_or("-")),
                    styled_body_cell(entry.description.as_deref().unwrap_or("-")),
                ];
                if show_source_column {
                    row.insert(1, styled_body_cell(&entry.source));
                }
                if show_details_column {
                    row.push(styled_body_cell(&detail_text));
                }
                row
            })
            .collect();
        let mut headers = vec![
            styled_header_cell("workflow"),
            styled_header_cell("name"),
            styled_header_cell("description"),
        ];
        if show_source_column {
            headers.insert(1, styled_header_cell("source"));
        }
        if show_details_column {
            headers.push(styled_header_cell("details"));
        }
        lines.push(render_cli_table(headers, rows, 2));
        if args.verbose {
            lines.push(render_meta_line(&format!(
                "Tip: run `rzn-browser list {} <workflow>` for per-workflow inputs and examples.",
                system
            )));
        }
    }

    lines.join("\n")
}

fn render_empty_workflow_catalog(args: &WorkflowListArgs) -> String {
    let mut filters = Vec::new();
    if let Some(system) = args.system.as_deref() {
        filters.push(format!("system '{}'", system));
    }
    if let Some(source) = args.source {
        filters.push(format!("source '{}'", source.as_str()));
    }

    if filters.is_empty() {
        "No installed named workflows found.".to_string()
    } else {
        format!(
            "No installed workflows found for {}.",
            filters.join(" and ")
        )
    }
}

fn format_source_counts(entries: &[NamedWorkflowEntry]) -> String {
    let ordered_sources = ["user", "builtin", "legacy"];
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for entry in entries {
        *counts.entry(entry.source.as_str()).or_insert(0) += 1;
    }

    let mut parts = Vec::new();
    for source in ordered_sources {
        if let Some(count) = counts.remove(source) {
            parts.push(format!("{} {}", count, source));
        }
    }
    for (source, count) in counts {
        parts.push(format!("{} {}", count, source));
    }
    parts.join(", ")
}

fn handle_workflow_dirs() -> Result<(), Box<dyn std::error::Error>> {
    let roots = workflow_roots();
    println!("Workflow directories:");
    println!("  runtime: {}", roots.runtime_dir.display());
    println!("  builtin: {}", roots.builtin_dir.display());
    println!("  user: {}", roots.user_dir.display());
    if let Some(legacy) = roots.legacy_user_dir {
        println!("  legacy: {}", legacy.display());
    }
    Ok(())
}

async fn handle_workflow_add(args: WorkflowAddArgs) -> Result<(), Box<dyn std::error::Error>> {
    let imported = import_user_workflows(
        &PathBuf::from(&args.source),
        args.system.as_deref(),
        args.workflow_name.as_deref(),
        args.force,
    )?;
    println!("Imported {} workflow file(s):", imported.len());
    for path in imported {
        println!("- {}", path.display());
    }
    Ok(())
}

async fn handle_workflow_pull(args: WorkflowPullArgs) -> Result<(), Box<dyn std::error::Error>> {
    let source_root = if let Some(repo_root) = args.repo_root {
        detect_catalog_source_root(&PathBuf::from(repo_root))?
    } else {
        let url = workflow_pull_url(&args);
        let temp_root = download_catalog_source(&url)?;
        detect_catalog_source_root(&temp_root)?
    };

    let summary = install_builtin_catalog_from_repo_root(&source_root)?;
    println!("Updated bundled workflow catalog:");
    println!("  builtin: {}", summary.builtin_dir);
    println!("  workflows: {}", summary.workflow_files);
    println!("  examples: {}", summary.example_files);
    Ok(())
}

fn workflow_ref_value(args: &WorkflowRefArgs) -> anyhow::Result<String> {
    compose_workflow_reference(&args.workflow_or_system, args.workflow_name.as_deref())
}

fn normalize_chatgpt_chat_id(value: &str) -> anyhow::Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("chat_id cannot be empty");
    }
    if !trimmed.contains("://") {
        return Ok(trimmed.to_string());
    }

    let parsed =
        Url::parse(trimmed).map_err(|_| anyhow::anyhow!("invalid chat_id URL '{}'", trimmed))?;
    let host = parsed
        .host_str()
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    if host != "chatgpt.com" && host != "chat.openai.com" {
        anyhow::bail!(
            "unsupported chat_id URL host '{}' (expected chatgpt.com or chat.openai.com)",
            host
        );
    }

    let segments: Vec<&str> = parsed
        .path_segments()
        .map(|parts| parts.filter(|part| !part.is_empty()).collect())
        .unwrap_or_default();
    match segments.as_slice() {
        ["c", chat_id] if !chat_id.trim().is_empty() => Ok(chat_id.trim().to_string()),
        _ => anyhow::bail!(
            "unsupported chat_id URL path '{}' (expected /c/<chat_id>)",
            parsed.path()
        ),
    }
}

fn normalize_run_params(
    mut params: HashMap<String, String>,
) -> anyhow::Result<HashMap<String, String>> {
    if let Some(raw_chat_id) = params.get("chat_id").cloned() {
        params.insert(
            "chat_id".to_string(),
            normalize_chatgpt_chat_id(&raw_chat_id)?,
        );
    }
    Ok(params)
}

fn workflow_pull_url(args: &WorkflowPullArgs) -> String {
    if let Some(url) = args.url.as_ref() {
        return url.clone();
    }
    if let Some(git_ref) = args.git_ref.as_ref() {
        return format!(
            "https://github.com/{}/archive/{}.tar.gz",
            args.repo.trim(),
            git_ref.trim()
        );
    }
    let repo = std::env::var("RZN_INSTALL_REPO").unwrap_or_else(|_| args.repo.clone());
    format!(
        "https://github.com/{}/releases/latest/download/rzn-browser-workflows.tar.gz",
        repo.trim()
    )
}

fn download_catalog_source(url: &str) -> anyhow::Result<PathBuf> {
    let temp_root =
        std::env::temp_dir().join(format!("rzn-workflow-pull-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_root)
        .map_err(|e| anyhow::anyhow!("create temp dir {}: {}", temp_root.display(), e))?;

    let archive_path = temp_root.join("catalog.tar.gz");
    run_checked_command(
        "curl",
        Command::new("curl")
            .arg("-fsSL")
            .arg(url)
            .arg("-o")
            .arg(&archive_path),
    )?;

    let extract_dir = temp_root.join("extract");
    fs::create_dir_all(&extract_dir)
        .map_err(|e| anyhow::anyhow!("create extract dir {}: {}", extract_dir.display(), e))?;

    run_checked_command(
        "tar",
        Command::new("tar")
            .arg("-xzf")
            .arg(&archive_path)
            .arg("-C")
            .arg(&extract_dir),
    )?;

    Ok(extract_dir)
}

fn run_checked_command(label: &str, command: &mut Command) -> anyhow::Result<()> {
    let output = command
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run {}: {}", label, e))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(anyhow::anyhow!(
        "{} failed with status {}.\nstdout:\n{}\nstderr:\n{}",
        label,
        output.status,
        stdout.trim(),
        stderr.trim()
    ))
}

fn resolve_named_workflow_path(reference: &str) -> anyhow::Result<PathBuf> {
    resolve_workflow_reference(reference)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_run_defaults_to_native_with_split_workflow_reference() {
        let cli = Cli::try_parse_from([
            "rzn-browser",
            "run",
            "google",
            "search",
            "--param",
            "search_query=rust",
        ])
        .expect("parse run command");

        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.via, RunVia::Native);
                assert_eq!(
                    workflow_ref_value(&args.workflow_ref).expect("workflow ref"),
                    "google/search"
                );
                assert_eq!(
                    args.params,
                    vec![("search_query".to_string(), "rust".to_string())]
                );
            }
            other => panic!("expected run command, got {:?}", other),
        }
    }

    #[test]
    fn parse_run_accepts_namespaced_reference_and_desktop_backend() {
        let cli = Cli::try_parse_from([
            "rzn-browser",
            "run",
            "google/search",
            "--via",
            "desktop",
            "--profile",
            "minimal",
        ])
        .expect("parse desktop run");

        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.via, RunVia::Desktop);
                assert_eq!(
                    workflow_ref_value(&args.workflow_ref).expect("workflow ref"),
                    "google/search"
                );
                assert_eq!(args.profile.as_deref(), Some("minimal"));
            }
            other => panic!("expected run command, got {:?}", other),
        }
    }

    #[test]
    fn parse_report_workflow_broken_command() {
        let cli = Cli::try_parse_from([
            "rzn-browser",
            "report",
            "workflow-broken",
            "--system",
            "google",
            "--workflow",
            "google/search-v1",
            "--version",
            "2026-04-24.1",
            "--step",
            "search_button",
            "--error",
            "button_not_found",
            "--app-version",
            "0.2.5",
            "--platform",
            "macos",
            "--note",
            "The page loaded, but the button never appeared.",
            "--dry-run",
        ])
        .expect("parse report command");

        match cli.command {
            Commands::Report(ReportCommands::WorkflowBroken(args)) => {
                assert_eq!(args.product, "rzn-browser");
                assert_eq!(args.flow_kind, "workflow");
                assert_eq!(args.system, "google");
                assert_eq!(args.workflow, "google/search-v1");
                assert_eq!(args.version, "2026-04-24.1");
                assert_eq!(args.step, "search_button");
                assert_eq!(args.error, "button_not_found");
                assert_eq!(args.app_version, "0.2.5");
                assert_eq!(args.platform, "macos");
                assert_eq!(
                    args.note.as_deref(),
                    Some("The page loaded, but the button never appeared.")
                );
                assert!(args.dry_run);
            }
            other => panic!("expected report command, got {:?}", other),
        }
    }

    #[test]
    fn workflow_report_payload_uses_only_explicit_args() {
        let args = WorkflowBrokenReportArgs {
            product: "rzn-browser".to_string(),
            flow_kind: "workflow".to_string(),
            system: "google".to_string(),
            workflow: "google/search-v1".to_string(),
            version: "2026-04-24.1".to_string(),
            step: "search_button".to_string(),
            error: "button_not_found".to_string(),
            app_version: "0.2.5".to_string(),
            platform: "macos".to_string(),
            note: Some("User-written note.".to_string()),
            dry_run: true,
        };

        let payload = build_report_body(WorkflowBrokenReportInput {
            product: args.product,
            flow_kind: args.flow_kind,
            system: args.system,
            workflow: args.workflow,
            version: args.version,
            step: args.step,
            error: args.error,
            app_version: args.app_version,
            platform: args.platform,
            note: args.note,
        })
        .expect("payload");
        let value = serde_json::to_value(payload).expect("json");
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["source"], "rzn-browser-cli");
        assert_eq!(value["submission_mode"], "manual_cli");
        assert_eq!(value["product"], "rzn-browser");
        assert_eq!(value["flow_kind"], "workflow");
        assert_eq!(value["surface"], "google");
        assert_eq!(value["flow"], "google/search-v1");
        assert_eq!(value["flow_version"], "2026-04-24.1");
        assert_eq!(value["failed_stage"], "search_button");
        assert_eq!(value["error"], "button_not_found");
        assert_eq!(value["app_version"], "0.2.5");
        assert_eq!(value["platform"], "macos");
        assert_eq!(value["note"], "User-written note.");

        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let forbidden = [
            "workflow_inputs",
            "search_terms",
            "prompts",
            "url",
            "urls",
            "dom",
            "accessibility_tree",
            "screenshots",
            "cookies",
            "local_storage",
            "session_storage",
            "logs",
            "stdout",
            "stderr",
            "run_id",
            "trace_id",
            "browser_history",
            "file_paths",
            "page_title",
            "page_text",
        ];
        for key in forbidden {
            assert!(
                !keys.contains(&key.to_string()),
                "forbidden key present: {key}"
            );
        }
    }

    #[test]
    fn failure_report_block_is_visible_and_private_data_free() {
        let context = workflow_failure_report::WorkflowFailureReportContext {
            product: "rzn-browser".to_string(),
            flow_kind: "workflow".to_string(),
            system: "google".to_string(),
            workflow: "google/search-v1".to_string(),
            version: "2026-04-24.1".to_string(),
            step: "search_button".to_string(),
            error: "button_not_found".to_string(),
            app_version: "0.2.5".to_string(),
            platform: "macos".to_string(),
        };

        let rendered = render_report_block(&context);
        assert!(rendered.contains("Reporting this helps us know what broke"));
        assert!(rendered.contains("This command sends exactly the visible fields in the command."));
        assert!(rendered.contains("rzn-browser report workflow-broken"));
        assert!(rendered.contains("--workflow google/search-v1"));
        assert!(rendered.contains("--error button_not_found"));
        assert!(rendered.contains("It does not read or send workflow inputs"));
        assert!(rendered.contains("DOM/accessibility trees"));
        assert!(rendered.contains("local storage, session storage"));
        assert!(rendered.contains("stdout/stderr, run_id/trace_id"));
        assert!(!rendered.contains("last-failure"));
        assert!(!rendered.contains("diagnostics"));
    }

    #[test]
    fn raw_private_error_normalizes_to_stable_code() {
        let raw = "step submit (click_element) failed: selector not found on https://example.com/search?q=private+term";
        let context = build_failure_context_from_error(
            "google/search-v1",
            std::path::Path::new("/tmp/google-search-v1.json"),
            raw,
        );
        assert_eq!(context.step, "submit");
        assert_eq!(context.error, "button_not_found");
    }

    #[test]
    fn parse_top_level_list_with_optional_system_filter() {
        let cli = Cli::try_parse_from([
            "rzn-browser",
            "list",
            "google",
            "--source",
            "builtin",
            "--all-sources",
            "--verbose",
        ])
        .expect("parse list");

        match cli.command {
            Commands::List(args) => {
                assert_eq!(args.system.as_deref(), Some("google"));
                assert_eq!(args.workflow_name, None);
                assert_eq!(args.source, Some(WorkflowSourceArg::Builtin));
                assert!(args.all_sources);
                assert!(args.verbose);
                assert!(!args.json);
            }
            other => panic!("expected list command, got {:?}", other),
        }
    }

    #[test]
    fn render_workflow_catalog_groups_workflows_by_system() {
        let args = WorkflowListArgs {
            system: None,
            workflow_name: None,
            source: None,
            all_sources: false,
            verbose: false,
            json: false,
        };
        let rendered = render_workflow_catalog(
            &[
                NamedWorkflowEntry {
                    id: "google/search".to_string(),
                    system: "google".to_string(),
                    workflow: "search".to_string(),
                    legacy_alias: "google-search".to_string(),
                    source: "builtin".to_string(),
                    path: "/tmp/google-search.json".to_string(),
                    relative_path: "google/google-search.json".to_string(),
                    name: Some("Google Search".to_string()),
                    description: Some("Search Google and return the results page.".to_string()),
                    effective: true,
                    shadowed_by_source: None,
                    overrides_sources: vec!["legacy".to_string()],
                },
                NamedWorkflowEntry {
                    id: "google/images".to_string(),
                    system: "google".to_string(),
                    workflow: "images".to_string(),
                    legacy_alias: "google-images".to_string(),
                    source: "user".to_string(),
                    path: "/tmp/google-images.json".to_string(),
                    relative_path: "google/google-images.json".to_string(),
                    name: None,
                    description: None,
                    effective: true,
                    shadowed_by_source: None,
                    overrides_sources: Vec::new(),
                },
                NamedWorkflowEntry {
                    id: "x/export-thread".to_string(),
                    system: "x".to_string(),
                    workflow: "export-thread".to_string(),
                    legacy_alias: "x-export-thread".to_string(),
                    source: "builtin".to_string(),
                    path: "/tmp/x-export-thread.json".to_string(),
                    relative_path: "x/x-export-thread.json".to_string(),
                    name: Some("Export Thread".to_string()),
                    description: None,
                    effective: true,
                    shadowed_by_source: None,
                    overrides_sources: Vec::new(),
                },
            ],
            &args,
        );

        assert!(rendered
            .contains("Installed workflows: 3 workflows across 2 systems (1 user, 2 builtin)"));
        assert!(rendered.contains("google (2 workflows)"));
        assert!(rendered.contains("workflow"));
        assert!(rendered.contains("source"));
        assert!(rendered.contains("search"));
        assert!(rendered.contains("builtin"));
        assert!(rendered.contains("Google Search"));
        assert!(rendered.contains("overrides legacy"));
        assert!(rendered.contains("description"));
        assert!(rendered.contains("images"));
        assert!(rendered.contains("user"));
        assert!(rendered.contains("x (1 workflow)"));
        assert!(rendered.contains("export-thread"));
        assert!(rendered.contains("Export Thread"));
    }

    #[test]
    fn render_workflow_catalog_verbose_marks_shadowed_entries() {
        let args = WorkflowListArgs {
            system: Some("google".to_string()),
            workflow_name: None,
            source: Some(WorkflowSourceArg::Builtin),
            all_sources: false,
            verbose: true,
            json: false,
        };
        let rendered = render_workflow_catalog(
            &[NamedWorkflowEntry {
                id: "google/search".to_string(),
                system: "google".to_string(),
                workflow: "search".to_string(),
                legacy_alias: "google-search".to_string(),
                source: "builtin".to_string(),
                path: "/tmp/google-search.json".to_string(),
                relative_path: "google/google-search.json".to_string(),
                name: Some("Google Search".to_string()),
                description: None,
                effective: false,
                shadowed_by_source: Some("user".to_string()),
                overrides_sources: Vec::new(),
            }],
            &args,
        );

        assert!(rendered.contains("Installed workflows: 1 entries across 1 system (1 builtin)"));
        assert!(rendered.contains("google (1 entry)"));
        assert!(rendered.contains("details"));
        assert!(rendered.contains("shadowed by user"));
        assert!(rendered.contains("id google/search"));
        assert!(rendered.contains("legacy google-search"));
    }

    #[test]
    fn parse_top_level_list_accepts_workflow_name_for_detail_view() {
        let cli = Cli::try_parse_from(["rzn-browser", "list", "chatgpt", "continue-chat-v1"])
            .expect("parse detail list");

        match cli.command {
            Commands::List(args) => {
                assert_eq!(args.system.as_deref(), Some("chatgpt"));
                assert_eq!(args.workflow_name.as_deref(), Some("continue-chat-v1"));
                assert!(!args.all_sources);
                assert!(!args.verbose);
                assert!(!args.json);
            }
            other => panic!("expected list command, got {:?}", other),
        }
    }

    #[test]
    fn parse_workflow_validate_with_write_help() {
        let cli = Cli::try_parse_from([
            "rzn-browser",
            "workflow",
            "validate",
            "chatgpt",
            "continue-chat-v1",
            "--write-help",
            "--json",
        ])
        .expect("parse workflow validate");

        match cli.command {
            Commands::Workflow(WorkflowCommands::Validate(args)) => {
                assert_eq!(args.workflow_ref.workflow_or_system, "chatgpt");
                assert_eq!(
                    args.workflow_ref.workflow_name.as_deref(),
                    Some("continue-chat-v1")
                );
                assert!(args.write_help);
                assert!(args.json);
            }
            other => panic!("expected workflow validate command, got {:?}", other),
        }
    }

    #[test]
    fn build_workflow_help_params_merges_required_declared_and_inferred_placeholders() {
        let workflow = json!({
            "id": "chatgpt/continue-chat-v1",
            "name": "ChatGPT: Continue Chat",
            "description": "Continue an existing chat.",
            "browser_automation": {
                "use_current_tab": true,
                "sequences": [{
                    "name": "chatgpt_continue_chat",
                    "description": "Open thread and send next prompt.",
                    "required_variables": [
                        {"name": "chat_id", "description": "Conversation id"},
                        {"name": "message_text", "description": "Prompt text"}
                    ],
                    "steps": [
                        {"type": "navigate_to_url", "url": "https://chatgpt.com/c/{chat_id}"},
                        {"type": "execute_javascript", "args": ["{chat_id}", "{message_text}", "{model_slug}|||{model_effort}"], "script": "const ignore = /^\\{[a-z]+\\}$/;"}
                    ]
                }]
            }
        });

        let params = build_workflow_help_params(&workflow, &WorkflowHelpMetadata::default());
        let names: Vec<&str> = params.iter().map(|param| param.name.as_str()).collect();

        assert_eq!(
            names,
            vec!["chat_id", "message_text", "model_effort", "model_slug"]
        );
        assert!(
            params
                .iter()
                .find(|param| param.name == "chat_id")
                .unwrap()
                .required
        );
        assert!(
            !params
                .iter()
                .find(|param| param.name == "model_slug")
                .unwrap()
                .required
        );
    }

    #[test]
    fn render_workflow_help_view_includes_run_command_and_parameter_table() {
        let view = WorkflowHelpView {
            reference: "chatgpt/continue-chat-v1".to_string(),
            path: "/tmp/chatgpt-continue-chat-v1.json".to_string(),
            source: Some("builtin".to_string()),
            system: Some("chatgpt".to_string()),
            workflow: Some("continue-chat-v1".to_string()),
            legacy_alias: Some("chatgpt-continue-chat-v1".to_string()),
            id: "chatgpt_continue_chat_v1".to_string(),
            name: "ChatGPT: Continue Chat".to_string(),
            description: "Continue an existing chat.".to_string(),
            version: Some("1.1.0".to_string()),
            updated: Some("2026-04-23T00:00:00Z".to_string()),
            sequence_name: Some("chatgpt_continue_chat".to_string()),
            sequence_description: Some("Open thread and send another prompt.".to_string()),
            uses_current_tab: true,
            parameters: vec![
                WorkflowHelpParamView {
                    name: "chat_id".to_string(),
                    required: true,
                    description: "Conversation id from the target app URL.".to_string(),
                    shape: Some("string id".to_string()),
                    default_value: None,
                    example: Some("01234567-89ab-cdef-0123-456789abcdef".to_string()),
                    sensitive: false,
                },
                WorkflowHelpParamView {
                    name: "message_text".to_string(),
                    required: true,
                    description: "Message or prompt text to send.".to_string(),
                    shape: Some("text".to_string()),
                    default_value: None,
                    example: Some("Summarize the last three commits.".to_string()),
                    sensitive: false,
                },
            ],
            examples: vec![WorkflowHelpExampleView {
                description: Some("Basic run with required parameters.".to_string()),
                command: "rzn-browser run chatgpt continue-chat-v1 --param chat_id=\"01234567-89ab-cdef-0123-456789abcdef\" --param message_text=\"Summarize the last three commits.\"".to_string(),
            }],
            notes: vec!["Uses the current Chrome tab/session instead of opening a separate browser tab by default.".to_string()],
            returns: Some("Returns the resolved chat id and post-send thread state.".to_string()),
        };

        let rendered = render_workflow_help_view(&view);
        assert!(rendered.contains("chatgpt/continue-chat-v1 — ChatGPT: Continue Chat"));
        assert!(rendered.contains("rzn-browser run chatgpt continue-chat-v1"));
        assert!(rendered.contains("Parameters"));
        assert!(rendered.contains("message_text"));
        assert!(rendered.contains("Returns"));
    }

    #[test]
    fn validate_workflow_help_contract_reports_missing_help_entries() {
        let workflow = json!({
            "id": "google/search",
            "name": "Google Search",
            "description": "Search Google.",
            "browser_automation": {
                "sequences": [{
                    "name": "search",
                    "description": "Run a search.",
                    "required_variables": [
                        {"name": "search_query", "description": "Query text"}
                    ],
                    "steps": [
                        {"type": "navigate_to_url", "url": "https://www.google.com/search?q={search_query}"}
                    ]
                }]
            }
        });

        let report = validate_workflow_help_contract(
            "google/search",
            std::path::Path::new("/tmp/google-search.json"),
            &workflow,
        );

        assert!(!report.ok);
        assert!(report.error_count >= 2);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.field == "help"
                && matches!(issue.level, WorkflowValidationLevel::Error)));
        assert!(report.issues.iter().any(|issue| {
            issue.field == "help.parameters.search_query"
                && matches!(issue.level, WorkflowValidationLevel::Error)
        }));
    }

    #[test]
    fn ensure_run_parameters_present_shows_help_for_missing_required_params() {
        let temp = std::env::temp_dir().join(format!("rzn_workflow_help_{}", uuid::Uuid::new_v4()));
        std::fs::write(
            &temp,
            serde_json::to_string_pretty(&json!({
                "id": "chatgpt/continue-chat-v1",
                "name": "ChatGPT: Continue Chat",
                "description": "Continue an existing chat.",
                "help": {
                    "summary": "Continue an existing chat and send another prompt.",
                    "parameters": [
                        {
                            "name": "chat_id",
                            "required": true,
                            "shape": "string id",
                            "description": "Conversation id from /c/<chat_id>",
                            "example": "01234567-89ab-cdef-0123-456789abcdef"
                        },
                        {
                            "name": "message_text",
                            "required": true,
                            "shape": "text",
                            "description": "Prompt text to send",
                            "example": "Turn that into a checklist."
                        }
                    ],
                    "examples": [
                        {
                            "description": "Basic run",
                            "command": "rzn-browser run chatgpt continue-chat-v1 --param chat_id=\"01234567-89ab-cdef-0123-456789abcdef\" --param message_text=\"Turn that into a checklist.\""
                        }
                    ]
                },
                "browser_automation": {
                    "sequences": [{
                        "name": "main",
                        "description": "Main sequence",
                        "required_variables": [
                            {"name": "chat_id", "description": "Conversation id"},
                            {"name": "message_text", "description": "Prompt text"}
                        ],
                        "steps": []
                    }]
                }
            }))
            .expect("serialize workflow"),
        )
        .expect("write workflow");

        let err = ensure_run_parameters_present(
            "chatgpt/continue-chat-v1",
            &temp,
            &HashMap::from([("chat_id".to_string(), "abc".to_string())]),
        )
        .expect_err("missing params should fail");

        let msg = err.to_string();
        assert!(msg.contains("missing required parameters: message_text"));
        assert!(msg.contains("Parameters"));
        assert!(
            msg.contains("rzn-browser run chatgpt/continue-chat-v1")
                || msg.contains("rzn-browser run chatgpt continue-chat-v1")
        );

        let _ = std::fs::remove_file(temp);
    }

    #[test]
    fn normalize_chat_id_accepts_bare_uuid() {
        let normalized =
            normalize_chatgpt_chat_id("69dcd746-60d4-83a2-a43d-3460f9c8adc4").expect("chat id");
        assert_eq!(normalized, "69dcd746-60d4-83a2-a43d-3460f9c8adc4");
    }

    #[test]
    fn normalize_chat_id_accepts_full_chatgpt_url() {
        let normalized = normalize_chatgpt_chat_id(
            "https://chatgpt.com/c/69dcd746-60d4-83a2-a43d-3460f9c8adc4?model=auto",
        )
        .expect("chat id from url");
        assert_eq!(normalized, "69dcd746-60d4-83a2-a43d-3460f9c8adc4");
    }

    #[test]
    fn normalize_run_params_rewrites_chat_id_url() {
        let params = HashMap::from([(
            "chat_id".to_string(),
            "https://chatgpt.com/c/69dcd746-60d4-83a2-a43d-3460f9c8adc4".to_string(),
        )]);
        let normalized = normalize_run_params(params).expect("normalized params");
        assert_eq!(
            normalized.get("chat_id").map(String::as_str),
            Some("69dcd746-60d4-83a2-a43d-3460f9c8adc4")
        );
    }

    #[test]
    fn desktop_surface_rejects_native_only_flags() {
        let err = validate_desktop_run_surface(
            "attach",
            DEFAULT_SNAPSHOT_MODE,
            None,
            &Vec::<String>::new(),
        )
        .expect_err("desktop should reject native-only mode");
        assert!(
            err.to_string()
                .contains("--mode only works with --via native"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn native_surface_rejects_desktop_only_flags() {
        let err =
            validate_native_run_surface(Some("minimal")).expect_err("native should reject profile");
        assert!(
            err.to_string()
                .contains("--profile only works with --via desktop"),
            "unexpected error: {}",
            err
        );
    }
}

async fn handle_workflow_new(
    _args: WorkflowNewArgs,
    mut config: PlanConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{stdin, stdout, Write};
    let mut wm = rzn_plan::workflow_manager::WorkflowManager::new(&config.workflows_dir)?;
    wm.initialize().await?;

    // Ask for name and variables (generic)
    print!("Workflow name [My Workflow]: ");
    stdout().flush().ok();
    let mut name = String::new();
    stdin().read_line(&mut name).ok();
    let name = if name.trim().is_empty() {
        "My Workflow".to_string()
    } else {
        name.trim().to_string()
    };

    print!("Required parameters (comma-separated) []: ");
    stdout().flush().ok();
    let mut req_line = String::new();
    stdin().read_line(&mut req_line).ok();
    let mut required_params: Vec<String> = req_line
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    print!("Optional parameters (comma-separated) []: ");
    stdout().flush().ok();
    let mut opt_line = String::new();
    stdin().read_line(&mut opt_line).ok();
    let mut optional_params: Vec<String> = opt_line
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    print!("Describe the goal of the task: ");
    stdout().flush().ok();
    let mut goal = String::new();
    stdin().read_line(&mut goal).ok();
    let goal = goal.trim().to_string();

    print!("Root domain(s) (comma-separated, optional): ");
    stdout().flush().ok();
    let mut root_line = String::new();
    stdin().read_line(&mut root_line).ok();
    let root_domains: Vec<String> = root_line
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let mut defaults: HashMap<String, String> = HashMap::new();
    for p in required_params.iter().chain(optional_params.iter()) {
        print!("Default for '{}'? (optional): ", p);
        stdout().flush().ok();
        let mut d = String::new();
        stdin().read_line(&mut d).ok();
        let d = d.trim().to_string();
        if !d.is_empty() {
            defaults.insert(p.clone(), d);
        }
    }

    print!("Is the outcome a list? (y/N): ");
    stdout().flush().ok();
    let mut list = String::new();
    stdin().read_line(&mut list).ok();
    let is_list = list.trim().to_lowercase() == "y";
    print!("Fields to extract (comma-separated) []: ");
    stdout().flush().ok();
    let mut fields_line = String::new();
    stdin().read_line(&mut fields_line).ok();
    let extract_fields: Vec<String> = fields_line
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Sanitize parameter names (letters, digits, underscore). Convert dashes to underscore, drop invalid-only names.
    let sanitize = |v: &mut Vec<String>| {
        v.iter_mut().for_each(|s| {
            let cleaned: String = s
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() {
                        c
                    } else if c == '-' {
                        '_'
                    } else {
                        ' '
                    }
                })
                .collect::<String>()
                .split_whitespace()
                .collect::<String>();
            *s = cleaned;
        });
        v.retain(|s| !s.is_empty());
    };
    sanitize(&mut required_params);
    sanitize(&mut optional_params);

    // Attempt a best-effort simulate-and-bind to learn a repeated list and attach an extraction step.
    // This keeps things site-agnostic by relying on observation (detectAutoList/observe) rather than hard-coded selectors.
    let mut extra_steps: Vec<rzn_core::Step> = Vec::new();
    let mut learned_item_selector: Option<String> = None;
    let mut used_url: Option<String> = None;

    if let Some(domain) = root_domains.first() {
        let start_url = if domain.starts_with("http") {
            domain.clone()
        } else {
            format!("https://{}", domain)
        };
        used_url = Some(start_url.clone());
        // Broker connection (best-effort). If it fails, we still write a seed workflow.
        if let Ok(mut broker) = create_broker_client(&config).await {
            // Navigate now (we also add this as the first workflow step below)
            let nav = rzn_core::Step {
                id: "nav_root".into(),
                name: format!("Navigate to {}", start_url),
                kind: rzn_core::StepKind::NavigateToUrl {
                    url: start_url.clone(),
                    wait: Some("domcontentloaded".into()),
                },
            };
            let _ = broker.execute_step(&nav).await;

            // If a default query exists, try a generic submit path (keeps selectors generic)
            let default_query = defaults.get("query").cloned();
            if let Some(q) = default_query {
                // Wait for a likely search field (generic patterns only)
                let wait = rzn_core::Step {
                id: "wait_search".into(),
                name: "Wait for a search field".into(),
                kind: rzn_core::StepKind::WaitForElement {
                    selector: "input[type='search'], input[name='q'], textarea[name='q'], input[aria-label*='Search' i], input[title*='Search' i]".into(),
                    frame_id: None,
                    condition: None,
                    timeout_ms: Some(12_000),
                },
            };
                let _ = broker.execute_step(&wait).await;

                // Fill the field (same generic selector set)
                let fill = rzn_core::Step {
                id: "fill_query".into(),
                name: "Fill query".into(),
                kind: rzn_core::StepKind::FillInputField {
                    selector: "input[type='search'], input[name='q'], textarea[name='q'], input[aria-label*='Search' i], input[title*='Search' i]".into(),
                    value: q,
                    frame_id: None,
                    clear_first: Some(true),
                    simulate_typing: Some(false),
                    delay_ms: None,
                    timeout_ms: Some(12_000),
                },
            };
                let _ = broker.execute_step(&fill).await;

                // Robust submit helper (raw extension action)
                let submit_raw = serde_json::json!({
                    "type": "submit_text_query",
                    "selector": "input[type='search'], input[name='q'], textarea[name='q'], input[aria-label*='Search' i], input[title*='Search' i]",
                    "press_enter_first": true,
                    "try_form_submit": true,
                    "timeoutMs": 8000
                });
                let _ = broker.execute_raw_step(submit_raw).await; // best-effort

                // Small wait for content to settle
                let _ = broker
                    .execute_step(&rzn_core::Step {
                        id: "settle".into(),
                        name: "Settle".into(),
                        kind: rzn_core::StepKind::WaitForTimeout { timeout_ms: 1200 },
                    })
                    .await;
            }

            // Discover a repeated list from observation (no site-specific CSS)
            let scope_selector = Some("#search, main, body");
            if let Ok(obs) = broker
                .observe("find search results", scope_selector, Some(8))
                .await
            {
                if let Some(best_sel) = obs
                    .get("result")
                    .and_then(|r| r.get("candidates"))
                    .and_then(|c| c.as_array())
                    .and_then(|a| a.get(0))
                    .and_then(|c| c.get("selector"))
                    .and_then(|s| s.as_str())
                {
                    learned_item_selector = Some(best_sel.to_string());
                }
            }

            // If observe path didn’t yield, try auto list detection
            if learned_item_selector.is_none() {
                if let Ok(auto) = broker.detect_auto_list(None).await {
                    if let Some(selector) = auto
                        .get("result")
                        .and_then(|r| r.get("containerSelector").or_else(|| r.get("itemSelector")))
                        .and_then(|s| s.as_str())
                    {
                        learned_item_selector = Some(selector.to_string());
                    }
                }
            }

            // If we learned an item selector, add a wait + extract step to the workflow we’ll write
            if let Some(item_sel) = learned_item_selector.clone() {
                // Create FieldSpec list from requested fields (generic mappings only)
                let mut fields_spec: Vec<rzn_core::FieldSpec> = Vec::new();
                let normalized_fields: Vec<String> = if extract_fields.is_empty() {
                    vec!["title".into(), "url".into()]
                } else {
                    extract_fields.clone()
                };
                for f in normalized_fields {
                    let fl = f.to_lowercase();
                    if fl == "title" {
                        fields_spec.push(rzn_core::FieldSpec {
                            name: f,
                            selector: "h1, h2, h3, a".into(),
                            attribute: None,
                            post_processing: vec![],
                        });
                    } else if fl == "url" {
                        fields_spec.push(rzn_core::FieldSpec {
                            name: f,
                            selector: "a".into(),
                            attribute: Some("href".into()),
                            post_processing: vec![],
                        });
                    } else if fl == "source_icon_url" {
                        // best-effort
                        fields_spec.push(rzn_core::FieldSpec {
                            name: f,
                            selector: "img, [role='img']".into(),
                            attribute: Some("src".into()),
                            post_processing: vec![],
                        });
                    } else if fl == "source" {
                        // best-effort (text within item)
                        fields_spec.push(rzn_core::FieldSpec {
                            name: f,
                            selector: "a, span, cite".into(),
                            attribute: None,
                            post_processing: vec![],
                        });
                    } else if fl == "meta_info" || fl == "snippet" {
                        fields_spec.push(rzn_core::FieldSpec {
                            name: f,
                            selector: "p, span".into(),
                            attribute: None,
                            post_processing: vec![],
                        });
                    } else {
                        // generic fallback
                        fields_spec.push(rzn_core::FieldSpec {
                            name: f,
                            selector: "*".into(),
                            attribute: None,
                            post_processing: vec![],
                        });
                    }
                }

                extra_steps.push(rzn_core::Step {
                    id: "wait_items".into(),
                    name: "Wait for list items".into(),
                    kind: rzn_core::StepKind::WaitForElement {
                        selector: item_sel.clone(),
                        frame_id: None,
                        condition: None,
                        timeout_ms: Some(12_000),
                    },
                });
                extra_steps.push(rzn_core::Step {
                    id: "extract".into(),
                    name: "Extract list items".into(),
                    kind: rzn_core::StepKind::ExtractStructuredData {
                        item_selector: item_sel,
                        limit: None,
                        fields: fields_spec,
                        frame_id: None,
                        extraction_type: None,
                    },
                });
            }
        } else {
            eprintln!(
                "[INFO] Skipping simulation: broker not available. Writing seed workflow only."
            );
        }
    }

    // Build and write the workflow with any extra learned steps
    let out_path = generate_generic_workflow(
        &name,
        &goal,
        &root_domains,
        &required_params,
        &optional_params,
        &extract_fields,
        is_list,
        Some(extra_steps),
    )?;
    let meta_path = format!("{}/{}.params.json", &config.workflows_dir, slugify(&name));
    let meta = json!({
        "workflow_file": out_path,
        "goal": goal,
        "root_domains": root_domains,
        "parameters": { "required": required_params, "optional": optional_params, "defaults": defaults },
        "outcome": { "type": if is_list { "list" } else { "single" }, "fields": extract_fields }
    });
    std::fs::create_dir_all(&config.workflows_dir).ok();
    std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;
    let out_path_buf = PathBuf::from(&out_path);
    let workflow_value = load_workflow_value(&out_path)?.1;
    let updated_value = write_workflow_help_block(&out_path, &out_path_buf, &workflow_value, None)?;
    let report = validate_workflow_help_contract(&out_path, &out_path_buf, &updated_value);
    println!("💾 Created workflow at {}", out_path);
    println!("📝 Parameters meta at {}", meta_path);
    println!(
        "{}",
        render_workflow_validation_report(&WorkflowValidationReport {
            wrote_help: true,
            ..report
        })
    );
    Ok(())
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() || ch == '-' || ch == '_' {
            out.push('-');
        }
    }
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_string()
}

fn generate_generic_workflow(
    name: &str,
    goal: &str,
    root_domains: &Vec<String>,
    required_params: &Vec<String>,
    optional_params: &Vec<String>,
    _extract_fields: &Vec<String>,
    _is_list: bool,
    extra_steps: Option<Vec<rzn_core::Step>>,
) -> Result<String, String> {
    use rzn_core::dsl::{BrowserAutomation, Sequence, Step, Variable, Workflow};
    use rzn_core::StepKind;
    let mut steps: Vec<Step> = Vec::new();
    if let Some(domain) = root_domains.first() {
        steps.push(Step {
            id: "nav_root".into(),
            name: format!("Navigate to {}", domain),
            kind: StepKind::NavigateToUrl {
                url: format!("https://{}", domain),
                wait: Some("domcontentloaded".into()),
            },
        });
    }
    if let Some(mut extras) = extra_steps {
        steps.append(&mut extras);
    }
    let mut req_vars: Vec<Variable> = Vec::new();
    for r in required_params {
        req_vars.push(Variable {
            name: r.clone(),
            description: "".into(),
            sensitive: Some(false),
        });
    }
    for o in optional_params {
        req_vars.push(Variable {
            name: o.clone(),
            description: "".into(),
            sensitive: Some(false),
        });
    }
    // Allow caller to append extra learned steps via a hidden global in this scope
    let seq = Sequence {
        name: "main".into(),
        description: goal.to_string(),
        required_variables: req_vars,
        steps,
    };
    let wf = Workflow {
        id: format!(
            "wf-{}-{}",
            slugify(name),
            uuid::Uuid::new_v4().to_string()[..8].to_string()
        ),
        name: name.to_string(),
        description: goal.to_string(),
        version: "1.0".into(),
        last_updated: chrono::Utc::now().to_rfc3339(),
        browser_automation: BrowserAutomation {
            sequences: vec![seq],
        },
    };
    let out_dir = &config_workflows_dir_or_default();
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let out_path = format!("{}/{}.json", out_dir, slugify(name));
    std::fs::write(
        &out_path,
        serde_json::to_string_pretty(&wf).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(out_path)
}

fn config_workflows_dir_or_default() -> String {
    default_user_workflows_dir().to_string_lossy().to_string()
}

fn collect_param_schema_interactive() -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    use std::io::{stdin, stdout, Write};
    let mut required: Vec<serde_json::Value> = Vec::new();
    let mut optional: Vec<serde_json::Value> = Vec::new();
    loop {
        print!("Add parameter? (y/N): ");
        let _ = stdout().flush();
        let mut ans = String::new();
        let _ = stdin().read_line(&mut ans);
        if ans.trim().to_lowercase() != "y" {
            break;
        }
        print!("  name: ");
        let _ = stdout().flush();
        let mut pname = String::new();
        let _ = stdin().read_line(&mut pname);
        let pname = pname.trim().to_string();
        if pname.is_empty() {
            continue;
        }
        print!("  description: ");
        let _ = stdout().flush();
        let mut pdesc = String::new();
        let _ = stdin().read_line(&mut pdesc);
        let pdesc = pdesc.trim().to_string();
        print!("  required? (Y/n): ");
        let _ = stdout().flush();
        let mut preq = String::new();
        let _ = stdin().read_line(&mut preq);
        let is_required = preq.trim().to_lowercase() != "n";
        print!("  default (optional): ");
        let _ = stdout().flush();
        let mut pdef = String::new();
        let _ = stdin().read_line(&mut pdef);
        let pdef = pdef.trim().to_string();
        let entry = if pdef.is_empty() {
            json!({"name": pname, "description": pdesc})
        } else {
            json!({"name": pname, "description": pdesc, "default": pdef})
        };
        if is_required {
            required.push(entry);
        } else {
            optional.push(entry);
        }
    }
    (required, optional)
}

fn generate_google_search_workflow(params: &HashMap<String, String>) -> Result<String, String> {
    use rzn_core::dsl::{BrowserAutomation, Sequence, Step, Variable, Workflow};
    use rzn_core::StepKind;

    let query = params
        .get("query")
        .cloned()
        .unwrap_or_else(|| "rust".to_string());
    let vertical = params
        .get("vertical")
        .cloned()
        .unwrap_or_else(|| "web".to_string());
    let limit = params
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(10);

    let mut steps: Vec<Step> = Vec::new();
    steps.push(Step {
        id: "nav_google".into(),
        name: "Navigate to Google".into(),
        kind: StepKind::NavigateToUrl {
            url: "https://www.google.com".into(),
            wait: Some("domcontentloaded".into()),
        },
    });
    steps.push(Step {
        id: "wait_box".into(),
        name: "Wait search box".into(),
        kind: StepKind::WaitForElement {
            selector: "input[name='q'], textarea[name='q']".into(),
            frame_id: None,
            condition: None,
            timeout_ms: Some(12000),
        },
    });
    steps.push(Step {
        id: "fill_q".into(),
        name: "Fill query".into(),
        kind: StepKind::FillInputField {
            selector: "input[name='q'], textarea[name='q']".into(),
            value: "{query}".into(),
            frame_id: None,
            clear_first: Some(true),
            simulate_typing: Some(false),
            delay_ms: None,
            timeout_ms: Some(12000),
        },
    });
    steps.push(Step {
        id: "press_enter".into(),
        name: "Press Enter".into(),
        kind: StepKind::PressSpecialKey {
            key: "Enter".into(),
            selector: None,
            frame_id: None,
            timeout_ms: Some(8000),
        },
    });
    steps.push(Step {
        id: "wait_results".into(),
        name: "Wait results".into(),
        kind: StepKind::WaitForElement {
            selector: "#search h3, .MjjYud h3, .g h3, h3".into(),
            frame_id: None,
            condition: None,
            timeout_ms: Some(12000),
        },
    });

    // Optional vertical tab click for images/news
    let vertical_lower = vertical.to_lowercase();
    if vertical_lower == "images" {
        steps.push(Step {
            id: "click_images".into(),
            name: "Open Images tab".into(),
            kind: StepKind::ClickElement {
                selector: "a[aria-label*='Images' i], a[href*='tbm=isch']".into(),
                frame_id: None,
                random_offset: Some(true),
                timeout_ms: Some(8000),
            },
        });
        steps.push(Step {
            id: "wait_images".into(),
            name: "Wait Images".into(),
            kind: StepKind::WaitForElement {
                selector: "a[aria-current='page'][aria-label*='Images' i], #islmp, div[data-ri]"
                    .into(),
                frame_id: None,
                condition: None,
                timeout_ms: Some(12000),
            },
        });
    } else if vertical_lower == "news" {
        steps.push(Step {
            id: "click_news".into(),
            name: "Open News tab".into(),
            kind: StepKind::ClickElement {
                selector: "a[aria-label*='News' i], a[href*='tbm=nws']".into(),
                frame_id: None,
                random_offset: Some(true),
                timeout_ms: Some(8000),
            },
        });
        steps.push(Step {
            id: "wait_news".into(),
            name: "Wait News".into(),
            kind: StepKind::WaitForElement {
                selector: "[data-hveid], .SoAPf, .dbsr".into(),
                frame_id: None,
                condition: None,
                timeout_ms: Some(12000),
            },
        });
    } else if vertical_lower == "scholar" {
        // Use Scholar UI directly for robustness
        steps.clear();
        steps.push(Step {
            id: "nav_scholar".into(),
            name: "Navigate to Google Scholar".into(),
            kind: StepKind::NavigateToUrl {
                url: "https://scholar.google.com".into(),
                wait: Some("domcontentloaded".into()),
            },
        });
        steps.push(Step {
            id: "wait_sbox".into(),
            name: "Wait search box".into(),
            kind: StepKind::WaitForElement {
                selector: "input[name='q']".into(),
                frame_id: None,
                condition: None,
                timeout_ms: Some(12000),
            },
        });
        steps.push(Step {
            id: "fill_s".into(),
            name: "Fill query".into(),
            kind: StepKind::FillInputField {
                selector: "input[name='q']".into(),
                value: query.clone(),
                frame_id: None,
                clear_first: Some(true),
                simulate_typing: Some(false),
                delay_ms: None,
                timeout_ms: Some(12000),
            },
        });
        steps.push(Step {
            id: "enter_s".into(),
            name: "Enter".into(),
            kind: StepKind::PressSpecialKey {
                key: "Enter".into(),
                selector: None,
                frame_id: None,
                timeout_ms: Some(8000),
            },
        });
        steps.push(Step {
            id: "wait_sresults".into(),
            name: "Wait results".into(),
            kind: StepKind::WaitForElement {
                selector: "#gs_res_ccl_mid, .gs_r".into(),
                frame_id: None,
                condition: None,
                timeout_ms: Some(12000),
            },
        });
    }

    // Extraction step (generic). We keep it simple: title + url + snippet for web/news/scholar; images may return titles only.
    let extraction_type = match vertical_lower.as_str() {
        "images" => "search_results",
        "news" => "search_results",
        "scholar" => "search_results",
        _ => "search_results",
    };

    // The extension understands extraction_type via enhanced handler; here we set a neutral step
    steps.push(Step {
        id: "extract".into(),
        name: format!("Extract top {} results", limit),
        kind: StepKind::ExtractStructuredData {
            item_selector: "#search .g, .g".into(),
            limit: Some(limit as u32),
            fields: vec![
                rzn_core::FieldSpec {
                    name: "title".into(),
                    selector: "h3".into(),
                    attribute: None,
                    post_processing: vec![],
                },
                rzn_core::FieldSpec {
                    name: "url".into(),
                    selector: "a".into(),
                    attribute: Some("href".into()),
                    post_processing: vec![],
                },
                rzn_core::FieldSpec {
                    name: "snippet".into(),
                    selector: ".VwiC3b, [data-sncf], .yXK7lf, .st".into(),
                    attribute: None,
                    post_processing: vec![],
                },
            ],
            frame_id: None,
            extraction_type: Some(extraction_type.to_string()),
        },
    });

    let seq = Sequence {
        name: "main".into(),
        description: "Google search with parameterized query and vertical".into(),
        required_variables: vec![Variable {
            name: "query".into(),
            description: "Search query".into(),
            sensitive: Some(false),
        }],
        steps,
    };
    let wf = Workflow {
        id: format!(
            "builtin-google-search-{}",
            uuid::Uuid::new_v4().to_string()[..8].to_string()
        ),
        name: "Google Search".into(),
        description: "Parameterized Google search workflow".into(),
        version: "1.0".into(),
        last_updated: chrono::Utc::now().to_rfc3339(),
        browser_automation: BrowserAutomation {
            sequences: vec![seq],
        },
    };

    // Write to a temp file and return its path
    let tmp = format!("/tmp/rzn_wf_google_{}.json", uuid::Uuid::new_v4());
    std::fs::write(
        &tmp,
        serde_json::to_string_pretty(&wf).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(tmp)
}

async fn handle_observe(
    args: ObserveArgs,
    config: PlanConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut broker_client = create_broker_client(&config).await?;
    // Ensure we have a tab and DOM
    let _ = broker_client.get_dom_snapshot().await;

    let resp = broker_client
        .observe(&args.instruction, args.scope.as_deref(), Some(args.max))
        .await?;

    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

async fn handle_action_surface_act(
    args: ActArgs,
    config: PlanConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let llm_client = create_llm_client(&config)?;
    let mut broker_client = create_broker_client(&config).await?;

    if let Some(url) = args.url.clone() {
        println!("🌐 Navigating to {}", url);
        let step = Step {
            id: "prefetch_nav".to_string(),
            name: format!("Navigate to {}", url),
            kind: StepKind::NavigateToUrl {
                url,
                wait: Some("domcontentloaded".to_string()),
            },
        };
        broker_client.execute_step(&step).await?;
    }

    let mut options = SurfaceActOptions::default();
    options.scope_selector = args.scope.clone();
    options.max_inventory = args.max_inventory;

    let execution =
        action_surface_execute_act(&llm_client, &mut broker_client, &args.instruction, options)
            .await?;

    if args.json {
        let out = json!({
            "step": execution.plan.step,
            "reason": execution.plan.reasoning,
            "plan": execution.plan.raw_plan,
            "inventory": execution.plan.inventory_excerpt,
            "result": execution.execution_result,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("🤖 Act Instruction: {}", args.instruction);
        if let Some(reason) = &execution.plan.reasoning {
            println!("   Reasoning: {}", reason);
        }
        println!("   Step: {:?}", execution.plan.step.kind);
        println!(
            "   Result: {}",
            serde_json::to_string_pretty(&execution.execution_result)?
        );
    }

    Ok(())
}

async fn handle_action_surface_extract(
    args: ExtractSchemaArgs,
    config: PlanConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let llm_client = create_llm_client(&config)?;
    let broker_client = create_broker_client(&config).await?;

    let fields_value: Value = serde_json::from_str(&args.fields)?;
    let fields_array = fields_value
        .as_array()
        .ok_or("fields must be a JSON array of objects")?;

    let mut fields: Vec<SurfaceExtractField> = Vec::new();
    for item in fields_array {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("field missing name")?
            .to_string();
        let attribute = item
            .get("attribute")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let optional = item
            .get("optional")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        fields.push(SurfaceExtractField {
            name,
            attribute,
            optional,
        });
    }

    let request = SurfaceExtractRequest {
        fields,
        limit: args.limit,
        scope_selector: args.scope.clone(),
    };

    let result = action_surface_extract(llm_client, broker_client, request).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&result.items)?);
    } else {
        println!(
            "📊 Extracted items: {}",
            serde_json::to_string_pretty(&result.items)?
        );
    }
    Ok(())
}

async fn handle_action_surface_observe(
    args: ObserveCompatArgs,
    config: PlanConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let llm_client = create_llm_client(&config)?;
    let mut broker_client = create_broker_client(&config).await?;

    let observe = action_surface_observe(
        &llm_client,
        &mut broker_client,
        &args.instruction,
        args.scope.clone(),
        args.max_inventory,
        args.max,
        Some(0.1),
    )
    .await?;

    let payload = json!({
        "actions": observe.actions,
        "inventory": observe.inventory_excerpt,
        "raw": observe.raw,
    });

    if args.json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("🔍 Observe instruction: {}", args.instruction);
        for (idx, action) in observe.actions.iter().enumerate() {
            println!(
                "{}: {} — method={} selector={:?} confidence={:?}",
                idx + 1,
                action.description,
                action.method,
                action.selector,
                action.confidence
            );
        }
        println!("\nInventory excerpt:\n{}", observe.inventory_excerpt);
    }

    Ok(())
}

async fn handle_nb(cmd: NbCommands) -> Result<(), Box<dyn std::error::Error>> {
    use rzn_core::StepKind;
    use rzn_plan::broker_client::{BrokerClient, Transport};

    match cmd {
        NbCommands::TopList { url, top } => {
            println!(" RZN NB: Top List Extractor");
            println!("   URL: {}", url);
            println!("   Top: {}", top);

            let mut broker = BrokerClient::new(Transport::Pipe);

            // Navigate
            let nav = rzn_core::Step {
                id: "nb_nav".to_string(),
                name: format!("Navigate to {}", url),
                kind: StepKind::NavigateToUrl {
                    url: url.clone(),
                    wait: Some("domcontentloaded".to_string()),
                },
            };
            broker.execute_step(&nav).await?;

            // Small settle wait
            let _ = broker
                .execute_step(&rzn_core::Step {
                    id: "nb_wait".to_string(),
                    name: "Wait".to_string(),
                    kind: StepKind::WaitForTimeout { timeout_ms: 900 },
                })
                .await;

            // Try detection a few times with small scrolls
            let mut results: Vec<serde_json::Value> = Vec::new();
            for attempt in 0..3usize {
                if let Ok(auto) = broker.detect_auto_list(None).await {
                    if let Some(obj) = auto.get("result").and_then(|v| v.as_object()) {
                        if let Some(items) = obj.get("items").and_then(|v| v.as_array()) {
                            for it in items.iter() {
                                let title = it.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                let url = it.get("href").and_then(|v| v.as_str()).unwrap_or("");
                                if !title.is_empty() || !url.is_empty() {
                                    results.push(serde_json::json!({ "title": title, "url": url }));
                                }
                                if results.len() >= top {
                                    break;
                                }
                            }
                        }
                    }
                }
                if results.len() >= top {
                    break;
                }
                // Nudge lazy loading
                let _ = broker
                    .execute_step(&rzn_core::Step {
                        id: format!("nb_wait_{}", attempt),
                        name: "Wait".to_string(),
                        kind: StepKind::WaitForTimeout { timeout_ms: 600 },
                    })
                    .await;
                let _ = broker
                    .execute_step(&rzn_core::Step {
                        id: format!("nb_scroll_{}", attempt),
                        name: "Scroll".to_string(),
                        kind: StepKind::ScrollWindowTo {
                            x: None,
                            y: Some(800),
                            direction: Some("down".to_string()),
                        },
                    })
                    .await;
            }

            if results.is_empty() {
                return Err("No repeated list items detected".into());
            }

            let mut out = results;
            out.truncate(top);
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({ "results": out }))?
            );
        }
    }

    Ok(())
}

async fn handle_quick_extract(
    args: QuickExtractArgs,
    config: PlanConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut broker = create_broker_client(&config).await?;

    // Normalize site
    let site = args.site.to_lowercase();
    if site != "google" && site != "amazon" {
        return Err(format!("Unsupported site: {} (supported: google, amazon)", site).into());
    }

    // Optional: enable CDP-first flags for this domain
    if args.cdp_first {
        let mut overrides = serde_json::Map::new();
        if site == "google" {
            overrides.insert("www.google.com".to_string(), json!({ "cdpEnable": true }));
            overrides.insert("google.com".to_string(), json!({ "cdpEnable": true }));
        } else if site == "amazon" {
            overrides.insert("www.amazon.com".to_string(), json!({ "cdpEnable": true }));
            overrides.insert("amazon.com".to_string(), json!({ "cdpEnable": true }));
        }
        let _ = broker.set_flags(Value::Object(overrides)).await; // best-effort
    }

    // Navigate to site
    let url = if site == "google" {
        "https://www.google.com"
    } else {
        "https://www.amazon.com"
    };
    let nav = Step {
        id: "nav".to_string(),
        name: format!("Navigate to {}", url),
        kind: StepKind::NavigateToUrl {
            url: url.to_string(),
            wait: Some("domcontentloaded".to_string()),
        },
    };
    let _ = broker.execute_step(&nav).await?;

    // Wait for search box, fill and submit
    if site == "google" {
        let wait = Step {
            id: "wait_q".to_string(),
            name: "Wait for search box".to_string(),
            kind: StepKind::WaitForElement {
                selector: "input[name='q'], textarea[name='q']".to_string(),
                frame_id: None,
                condition: None,
                timeout_ms: Some(12_000),
            },
        };
        let _ = broker.execute_step(&wait).await?;
        let fill = Step {
            id: "fill_q".to_string(),
            name: "Fill query".to_string(),
            kind: StepKind::FillInputField {
                selector: "input[name='q'], textarea[name='q']".to_string(),
                value: args.query.clone(),
                frame_id: None,
                clear_first: Some(true),
                simulate_typing: Some(false),
                delay_ms: None,
                timeout_ms: Some(12_000),
            },
        };
        let _ = broker.execute_step(&fill).await?;
        // Use generic submit action (robust across suggestion overlays/forms/buttons)
        let submit_raw = json!({
            "type": "submit_text_query",
            "selector": "input[name='q'], textarea[name='q']",
            "press_enter_first": true,
            "try_form_submit": true,
            "timeoutMs": 8000
        });
        let _ = broker.execute_raw_step(submit_raw).await; // best-effort; fallback waits below
                                                           // Wait for results (more permissive than #search to handle UI variants)
        let wait_res = Step {
            id: "wait_results".to_string(),
            name: "Wait results".to_string(),
            kind: StepKind::WaitForElement {
                selector: "#search h3, .MjjYud h3, .g h3, h3".to_string(),
                frame_id: None,
                condition: None,
                timeout_ms: Some(12_000),
            },
        };
        let wait_resp = broker.execute_step(&wait_res).await?;
        let mut waited_ok = wait_resp
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if !waited_ok {
            // Fallback 1: click the Google Search button
            let click_btn = Step { id: "click_search".to_string(), name: "Click Google Search".to_string(), kind: StepKind::ClickElement { selector: "input[name='btnK'], input[value='Google Search'], button[aria-label*='Search']".to_string(), frame_id: None, random_offset: Some(true), timeout_ms: Some(5_000) } };
            let _ = broker.execute_step(&click_btn).await?;
            let wait2 = Step {
                id: "wait_results2".to_string(),
                name: "Wait results (h3)".to_string(),
                kind: StepKind::WaitForElement {
                    selector: "#search h3, .MjjYud h3, .g h3, h3".to_string(),
                    frame_id: None,
                    condition: None,
                    timeout_ms: Some(10_000),
                },
            };
            let wait2_resp = broker.execute_step(&wait2).await?;
            waited_ok = wait2_resp
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
        }
        if !waited_ok {
            // Fallback 2: requestSubmit() on the form programmatically
            let js = r#"(function(){
              try {
                var el = document.querySelector("input[name='q'], textarea[name='q']");
                if (!el) return { ok: false, reason: 'no_input' };
                el.focus();
                var form = el.closest('form');
                if (form && form.requestSubmit) { form.requestSubmit(); return { ok: true, method: 'requestSubmit' }; }
                if (form && form.submit) { form.submit(); return { ok: true, method: 'submit' }; }
                // Fallback to clicking submit
                var btn = document.querySelector("input[value='Google Search'], button[aria-label*='Search']");
                if (btn) { btn.click(); return { ok: true, method: 'click' }; }
                return { ok: false, reason: 'no_method' };
              } catch(e){ return { ok: false, error: String(e) }; }
            })();"#;
            let raw = json!({ "type": "execute_javascript", "script": js });
            let _ = broker.execute_raw_step(raw).await?;
            let wait3 = Step {
                id: "wait_results3".to_string(),
                name: "Wait results (final)".to_string(),
                kind: StepKind::WaitForElement {
                    selector: "#search h3, .MjjYud h3, .g h3, h3".to_string(),
                    frame_id: None,
                    condition: None,
                    timeout_ms: Some(10_000),
                },
            };
            let _ = broker.execute_step(&wait3).await?;
        }

        // Helper to extract items array from extension response
        let mut extract_items = |resp: &Value| -> Vec<Value> {
            if let Some(arr) = resp.get("result").and_then(|v| v.as_array()) {
                return arr.clone();
            }
            if let Some(arr) = resp
                .get("result")
                .and_then(|v| v.get("results"))
                .and_then(|v| v.as_array())
            {
                return arr.clone();
            }
            if let Some(arr) = resp.get("results").and_then(|v| v.as_array()) {
                return arr.clone();
            }
            Vec::new()
        };

        // Observe → Extract via legacy array extractor (generic, no site profiles)
        let obs = broker
            .observe("find search results", Some("#search, main, body"), Some(6))
            .await?;
        let mut resp = json!({});
        let mut items: Vec<Value> = Vec::new();
        if let Some(best_sel) = obs
            .get("result")
            .and_then(|r| r.get("candidates"))
            .and_then(|c| c.as_array())
            .and_then(|a| a.get(0))
            .and_then(|c| c.get("selector"))
            .and_then(|s| s.as_str())
        {
            let legacy = json!({
                "type": "extract_structured_data",
                "force_legacy": true,
                "item_selector": best_sel,
                "fields": [
                    {"name": "title", "selector": "h3"},
                    {"name": "url", "selector": "a", "attribute": "href"},
                    {"name": "snippet", "selector": "p"}
                ]
            });
            resp = broker.execute_raw_step(legacy).await?;
            items = extract_items(&resp);
        }

        if items.len() > args.top {
            items.truncate(args.top);
        }
        let arr = Value::Array(items);
        // Pretty print if it looks like search results (title/url/snippet). If empty, print raw response too.
        let mut pretty = result_formatter::format_google_search_results(&arr);
        if arr.as_array().map(|a| a.is_empty()).unwrap_or(false) {
            pretty.push_str("\n\n[debug] Raw extension response:\n");
            pretty.push_str(
                &serde_json::to_string_pretty(&resp)
                    .unwrap_or_else(|_| "<serialize error>".to_string()),
            );
        }
        println!("{}", pretty);
    } else {
        // amazon
        let wait = Step {
            id: "wait_q".to_string(),
            name: "Wait for search box".to_string(),
            kind: StepKind::WaitForElement {
                selector: "#twotabsearchtextbox".to_string(),
                frame_id: None,
                condition: None,
                timeout_ms: Some(10_000),
            },
        };
        let _ = broker.execute_step(&wait).await?;
        let fill = Step {
            id: "fill_q".to_string(),
            name: "Fill query".to_string(),
            kind: StepKind::FillInputField {
                selector: "#twotabsearchtextbox".to_string(),
                value: args.query.clone(),
                frame_id: None,
                clear_first: Some(true),
                simulate_typing: Some(false),
                delay_ms: None,
                timeout_ms: Some(10_000),
            },
        };
        let _ = broker.execute_step(&fill).await?;
        // Click search button instead of Enter for Amazon
        let click = Step {
            id: "submit".to_string(),
            name: "Submit search".to_string(),
            kind: StepKind::ClickElement {
                selector: "#nav-search-submit-button".to_string(),
                frame_id: None,
                random_offset: Some(true),
                timeout_ms: Some(10_000),
            },
        };
        let _ = broker.execute_step(&click).await?;
        // Wait for results to load
        let wait_res = Step {
            id: "wait_results".to_string(),
            name: "Wait results".to_string(),
            kind: StepKind::WaitForElement {
                selector: ".s-result-item, [data-component-type='s-search-result']".to_string(),
                frame_id: None,
                condition: None,
                timeout_ms: Some(12_000),
            },
        };
        let _ = broker.execute_step(&wait_res).await?;

        // Extract product cards
        let fields = vec![
            FieldSpec {
                name: "title".to_string(),
                selector: "h2 a span, .s-title-instructions-style".to_string(),
                attribute: None,
                post_processing: vec![],
            },
            FieldSpec {
                name: "url".to_string(),
                selector: "h2 a".to_string(),
                attribute: Some("href".to_string()),
                post_processing: vec![],
            },
            FieldSpec {
                name: "price".to_string(),
                selector: ".a-price .a-offscreen, .a-price-whole".to_string(),
                attribute: None,
                post_processing: vec![],
            },
            FieldSpec {
                name: "rating".to_string(),
                selector: ".a-icon-alt, [aria-label*='out of']".to_string(),
                attribute: None,
                post_processing: vec![],
            },
        ];
        let extract = Step {
            id: "extract".to_string(),
            name: "Extract listings".to_string(),
            kind: StepKind::ExtractStructuredData {
                item_selector: ".s-result-item, [data-component-type='s-search-result']"
                    .to_string(),
                limit: None,
                fields,
                frame_id: None,
                extraction_type: None,
            },
        };
        let resp = broker.execute_step(&extract).await?;
        let mut items = resp
            .get("result")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if items.len() > args.top {
            items.truncate(args.top);
        }
        println!("{}", serde_json::to_string_pretty(&Value::Array(items))?);
    }

    Ok(())
}
