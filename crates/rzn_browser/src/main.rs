#![allow(dead_code, unused_assignments, unused_imports, unused_variables)]
#![allow(
    clippy::field_reassign_with_default,
    clippy::large_enum_variant,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::used_underscore_binding,
    clippy::vec_init_then_push
)]

mod cloud;
mod mcp_browser;
mod native_runner;
mod result_formatter;
mod skill_installer;
mod supervisor;
mod supervisor_cloud;
mod workflow_catalog;
mod workflow_failure_report;
mod workflow_params;

use anyhow::Context;
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use cloud::{handle_cloud_commands, CloudCommands};
use colored::Colorize;
use comfy_table::{
    modifiers::UTF8_ROUND_CORNERS,
    presets::{ASCII_FULL, UTF8_FULL_CONDENSED},
    Attribute, Cell, Color, ContentArrangement, Table,
};
use mcp_browser::{run_browser_mcp_server, BrowserMcpArgs};
use native_runner::{SnapshotMode, SupervisorRunConfig};
use rzn_contracts::v2::{
    validate_manifest_value, ParamDefV2, ParamKindV2, WorkflowManifestV2, WORKFLOW_CONTRACT_VERSION,
};
use rzn_core::dsl::LogMessage;
use rzn_core::secure_files::{append_secret_file_capped, secure_dir};
use rzn_core::{FieldSpec, Step, StepKind};
use rzn_plan::action_surface::{
    execute_act as action_surface_execute_act, extract as action_surface_extract,
    observe as action_surface_observe, SurfaceActOptions, SurfaceExtractField,
    SurfaceExtractRequest,
};
use rzn_sdk::host::{Host as Orchestrator, HostConfig as PlanConfig, PlanRequest, RunRequest};
use rzn_sdk::native_host::{
    install_rzn_native_host_for_browser_with_origins, native_host_manifest_path_for_browser,
    normalize_extension_origin, normalize_extension_origins, read_manifest,
    resolve_native_host_executable_path, uninstall_rzn_native_host_for_browser, BrowserKind,
    NativeHostInstallReport, NativeHostUninstallReport, NativeMessagingHostManifest,
    RZN_DEV_EXTENSION_ID, RZN_DEV_EXTENSION_ORIGIN, RZN_NATIVE_HOST_NAME,
};
use serde::{Deserialize, Serialize};
use serde_json::{self, json, Value};
use sha2::{Digest, Sha256};
use skill_installer::{
    install_skill, parse_clients, remove_skill, skill_paths, update_skill, SkillInstallRequest,
    SkillInstallScope, SkillRemoveRequest, DEFAULT_SKILL_NAME,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use terminal_size::{terminal_size, Width};
use url::Url;
use workflow_catalog::{
    compose_workflow_reference, default_user_workflows_dir, detect_catalog_source_root,
    import_user_workflows, install_builtin_catalog_from_repo_root, list_capabilities_with_query,
    list_named_workflows_with_query, resolve_capability_route, resolve_workflow_reference,
    validate_catalog_manifests, workflow_roots, CapabilityCatalogEntry, CapabilityCatalogQuery,
    CatalogValidationReport, NamedWorkflowEntry, WorkflowCatalogQuery,
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

    /// Run a workflow through the local browser supervisor
    Run(RunArgs),

    /// Re-check the local browser supervisor and native-host bridge
    Heal(HealArgs),

    /// Run or inspect the durable local browser supervisor
    #[command(subcommand)]
    Supervisor(SupervisorCommands),

    /// Inspect connected browser targets and bridges
    #[command(subcommand)]
    Browser(BrowserCommands),

    /// Run MCP servers over stdio
    #[command(subcommand)]
    Mcp(McpCommands),

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

    /// Install, list, and uninstall browser native-host registrations
    #[command(name = "native-host", subcommand)]
    NativeHost(NativeHostCommands),
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

const DEFAULT_SNAPSHOT_MODE: &str = "on-error";

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

    #[command(flatten)]
    target: BrowserTargetArgs,

    /// Parameters for the workflow (format: --param key=value)
    #[arg(long = "param", value_parser = parse_key_val::<String, String>)]
    params: Vec<(String, String)>,

    /// Snapshot mode for runs: none | after-step | on-error
    #[arg(long, default_value = DEFAULT_SNAPSHOT_MODE)]
    snapshot: String,

    /// Override APP_BASE for supervisor socket/token/runtime files
    #[arg(long)]
    app_base: Option<String>,

    /// Write the workflow's final result (markdown if present, otherwise pretty JSON) to this file path
    #[arg(long = "output-file")]
    output_file: Option<PathBuf>,

    /// Download all asset_urls + external_links from the result into this directory (writes manifest.json)
    #[arg(long = "download-dir")]
    download_dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct HealArgs {
    /// Override APP_BASE for supervisor socket/token/runtime files
    #[arg(long)]
    app_base: Option<String>,

    /// Emit machine-readable JSON
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand, Debug)]
enum McpCommands {
    /// Expose browser automation worker tools over MCP stdio
    Browser(BrowserMcpArgs),
}

#[derive(Subcommand, Debug)]
enum SupervisorCommands {
    /// Run the local browser supervisor until interrupted
    Serve(SupervisorCommonArgs),

    /// Print supervisor runtime status
    Status(SupervisorCommonArgs),

    /// Start the supervisor if needed and print status
    #[command(name = "ensure-ready")]
    EnsureReady(SupervisorCommonArgs),

    /// Ask a running supervisor to shut down
    Shutdown(SupervisorCommonArgs),

    /// Call a supervisor method with JSON params
    Call(SupervisorCallArgs),
}

#[derive(Subcommand, Debug)]
enum BrowserCommands {
    /// List connected browser bridges and target identifiers
    #[command(visible_alias = "list")]
    Targets(BrowserTargetsArgs),
    /// Show the saved default browser target
    Default(BrowserDefaultArgs),
    /// Save the default browser target used when no explicit target flag is passed
    Set(BrowserSetArgs),
    /// Clear the saved default browser target
    Clear(BrowserClearArgs),
}

#[derive(Args, Debug, Clone)]
struct BrowserTargetsArgs {
    #[command(flatten)]
    common: SupervisorCommonArgs,
}

#[derive(Args, Debug, Clone)]
struct BrowserDefaultArgs {
    #[command(flatten)]
    common: SupervisorCommonArgs,
}

#[derive(Args, Debug, Clone)]
struct BrowserSetArgs {
    #[command(flatten)]
    common: SupervisorCommonArgs,

    #[command(flatten)]
    target: BrowserTargetArgs,

    /// Browser target kind shorthand, for example `rzn-browser browser set chromium`
    browser_name: Option<String>,
}

#[derive(Args, Debug, Clone)]
struct BrowserClearArgs {
    #[command(flatten)]
    common: SupervisorCommonArgs,
}

#[derive(Args, Debug, Clone)]
struct SupervisorCommonArgs {
    /// Override APP_BASE for supervisor socket/token/runtime files
    #[arg(long)]
    app_base: Option<String>,

    /// Emit machine-readable JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct SupervisorCallArgs {
    #[command(flatten)]
    common: SupervisorCommonArgs,

    #[command(flatten)]
    target: BrowserTargetArgs,

    /// Supervisor method, for example runtime.status or browser.snapshot
    method: String,

    /// JSON params object
    #[arg(long, default_value = "{}")]
    params: String,

    /// Composite browser tab reference, for example rzn://browser/<instance>/tab/123
    #[arg(long = "tab-ref")]
    tab_ref: Option<String>,

    /// Browser-local numeric tab ID. Must be scoped by browser, browser instance, bridge, or session when multiple browsers are connected.
    #[arg(long = "tab")]
    tab: Option<u64>,
}

#[derive(Args, Debug, Clone, Default)]
struct BrowserTargetArgs {
    /// Browser target kind. Examples: --browser edge, --browser chromium
    #[arg(long)]
    browser: Option<String>,

    /// Exact browser instance ID from `rzn-browser browser targets`
    #[arg(long = "browser-instance")]
    browser_instance_id: Option<String>,

    /// Exact supervisor bridge ID from `rzn-browser browser targets`
    #[arg(long = "bridge")]
    bridge_id: Option<String>,
}

#[derive(Subcommand, Debug)]
enum NativeHostCommands {
    /// Install native-host manifests for one or more browser targets
    Install(NativeHostInstallArgs),
    /// List native-host registration status for known browser targets
    List(NativeHostListArgs),
    /// Diagnose native-host installation and runtime connectivity
    Doctor(NativeHostDoctorArgs),
    /// Remove native-host registration for one or more browser targets
    Uninstall(NativeHostUninstallArgs),
}

#[derive(Args, Debug, Clone)]
struct NativeHostInstallArgs {
    /// Browser target(s): chrome, chromium, edge. Defaults to chrome, chromium, edge. Repeat or comma-separate.
    #[arg(long = "browser", value_delimiter = ',')]
    browsers: Vec<String>,

    /// Extension ID to allow. Repeat to allow multiple extension builds.
    #[arg(long = "extension-id")]
    extension_ids: Vec<String>,

    /// Full chrome-extension://<id>/ origin to allow. Repeat to allow multiple extension builds.
    #[arg(long = "extension-origin")]
    extension_origins: Vec<String>,

    /// Explicit rzn-native-host executable path. Defaults to SDK resolver.
    #[arg(long = "native-host-path")]
    native_host_path: Option<PathBuf>,

    /// Emit machine-readable JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct NativeHostListArgs {
    /// Browser target(s) to inspect. Defaults to chrome, chromium, edge. Repeat or comma-separate.
    #[arg(long = "browser", value_delimiter = ',')]
    browsers: Vec<String>,

    /// Emit machine-readable JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct NativeHostDoctorArgs {
    /// Browser target to diagnose: chrome, chromium, edge, etc.
    #[arg(long = "browser", required = true)]
    browser: String,

    /// Extension bundle directory to validate. Defaults to extension/dist/<target>.
    #[arg(long = "extension-dir")]
    extension_dir: Option<PathBuf>,

    /// Expected extension origin or 32-character extension ID. Defaults to the pinned dev ID.
    #[arg(long = "extension-origin")]
    extension_origin: Option<String>,

    /// Override APP_BASE for supervisor socket/token/runtime files
    #[arg(long)]
    app_base: Option<String>,

    /// Emit machine-readable JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct NativeHostUninstallArgs {
    /// Browser target(s): chrome, chromium, edge. Repeat or comma-separate.
    #[arg(long = "browser", value_delimiter = ',', required = true)]
    browsers: Vec<String>,

    /// Emit machine-readable JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct SupervisorRunArgs {
    #[command(flatten)]
    workflow_ref: WorkflowRefArgs,

    #[command(flatten)]
    target: BrowserTargetArgs,

    /// Parameters for the workflow (format: --param key=value)
    #[arg(long = "param", value_parser = parse_key_val::<String, String>)]
    params: Vec<(String, String)>,

    /// Snapshot mode for runs: none | after-step | on-error
    #[arg(long, default_value = DEFAULT_SNAPSHOT_MODE)]
    snapshot: String,

    /// Override APP_BASE for supervisor socket/token/runtime files
    #[arg(long)]
    app_base: Option<String>,

    /// Write the workflow's final result (markdown if present, otherwise pretty JSON) to this file path
    #[arg(long = "output-file")]
    output_file: Option<PathBuf>,

    /// Download all asset_urls + external_links from the result into this directory
    #[arg(long = "download-dir")]
    download_dir: Option<PathBuf>,
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
    /// Validate manifest catalog/capability routing
    #[command(name = "validate-catalog")]
    ValidateCatalog(CatalogValidateArgs),
    /// Inspect or resolve manifest-declared capabilities
    #[command(subcommand)]
    Capability(CapabilityCommands),
    /// Show workflow storage directories
    Dirs,
    /// Import a JSON workflow file or directory into the user catalog
    Add(WorkflowAddArgs),
    /// Refresh bundled workflows/examples from the repo or a release archive
    Pull(WorkflowPullArgs),
    /// Show a cached workflow by id or file path
    Show(WorkflowShowArgs),
    /// Inspect the workflow manifest: inputs, outputs, side effects, and runtime
    Inspect(WorkflowInspectArgs),
    /// Deprecated alias for `workflow inspect`
    #[command(hide = true)]
    Contract(WorkflowContractArgs),
    /// Run a cached workflow by id or file path
    Run(WorkflowRunArgs),
    /// Create a new workflow via interactive builder (or with a template)
    New(WorkflowNewArgs),
}

#[derive(Subcommand, Debug)]
enum CapabilityCommands {
    /// List manifest-declared capabilities
    List(CapabilityListArgs),
    /// Resolve an explicit system/capability pair to its workflow route
    Resolve(CapabilityResolveArgs),
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
struct CapabilityListArgs {
    /// Required explicit system namespace filter for launch-facing routes
    #[arg(long)]
    system: Option<String>,

    /// Limit the list to a single catalog source
    #[arg(long, value_enum)]
    source: Option<WorkflowSourceArg>,

    /// Emit JSON instead of table output
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct CapabilityResolveArgs {
    /// Explicit system namespace, for example chatgpt
    #[arg(long)]
    system: String,

    /// Manifest-declared capability id, for example assistant.conversation.read
    capability_id: String,

    /// Emit JSON instead of a route line
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct CatalogValidateArgs {
    /// Fail closed for launch contract validation
    #[arg(long)]
    strict: bool,

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
struct WorkflowInspectArgs {
    #[command(flatten)]
    workflow_ref: WorkflowRefArgs,
    /// Output JSON
    #[arg(long)]
    json: bool,
}

type WorkflowContractArgs = WorkflowInspectArgs;

#[derive(Args, Debug)]
struct WorkflowValidateArgs {
    #[command(flatten)]
    workflow_ref: WorkflowRefArgs,
    /// Write or refresh the top-level help block before validating
    #[arg(long)]
    write_help: bool,
    /// Require manifest capability routing and output-contract scaffolding
    #[arg(long)]
    strict: bool,
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
    /// Developer escape hatch for direct workflow-id/path execution
    #[arg(long)]
    allow_direct_workflow: bool,
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

    if let Ok(dir) = secure_dir("cli-debug") {
        let line = format!("{}\n", serde_json::to_string(&log_msg).unwrap_or_default());
        let _ = append_secret_file_capped(
            &dir.join("cli.debug.jsonl"),
            line.as_bytes(),
            10 * 1024 * 1024,
        );
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
        Commands::Heal(args) => {
            if let Err(err) = handle_heal(args).await {
                eprintln!("❌ heal failed: {}", err);
                process::exit(1);
            }
        }
        Commands::Supervisor(cmd) => {
            if let Err(err) = handle_supervisor_commands(cmd).await {
                eprintln!("❌ supervisor command failed: {}", err);
                process::exit(1);
            }
        }
        Commands::Browser(cmd) => {
            if let Err(err) = handle_browser_commands(cmd).await {
                eprintln!("❌ browser command failed: {}", err);
                process::exit(1);
            }
        }
        Commands::Mcp(cmd) => match cmd {
            McpCommands::Browser(args) => {
                if let Err(err) = run_browser_mcp_server(args).await {
                    eprintln!("❌ mcp browser failed: {}", err);
                    process::exit(1);
                }
            }
        },
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
        Commands::NativeHost(cmd) => {
            if let Err(e) = handle_native_host_commands(cmd).await {
                eprintln!("❌ native-host command failed: {}", e);
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

async fn handle_native_host_commands(cmd: NativeHostCommands) -> anyhow::Result<()> {
    match cmd {
        NativeHostCommands::Install(args) => handle_native_host_install(args),
        NativeHostCommands::List(args) => handle_native_host_list(args),
        NativeHostCommands::Doctor(args) => handle_native_host_doctor(args).await,
        NativeHostCommands::Uninstall(args) => handle_native_host_uninstall(args),
    }
}

#[derive(Debug, Serialize)]
struct NativeHostCommandFailure {
    browser: Option<String>,
    error_code: String,
    error: String,
}

#[derive(Debug, Serialize)]
struct NativeHostInstallOutput {
    success: bool,
    reports: Vec<NativeHostInstallReport>,
    failures: Vec<NativeHostCommandFailure>,
}

#[derive(Debug, Serialize)]
struct NativeHostUninstallOutput {
    success: bool,
    reports: Vec<NativeHostUninstallReport>,
    failures: Vec<NativeHostCommandFailure>,
}

#[derive(Debug, Serialize)]
struct NativeHostListOutput {
    success: bool,
    targets: Vec<NativeHostListStatus>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum DoctorCheckStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
struct NativeHostDoctorCheck {
    name: String,
    status: DoctorCheckStatus,
    message: String,
}

#[derive(Debug, Serialize)]
struct NativeHostDoctorOutput {
    success: bool,
    browser: String,
    extension_origin: String,
    manifest_path: Option<PathBuf>,
    load_unpacked_path: PathBuf,
    checks: Vec<NativeHostDoctorCheck>,
}

#[derive(Debug, Serialize)]
struct NativeHostListStatus {
    browser: BrowserKind,
    browser_slug: String,
    display_name: String,
    install_location: Option<PathBuf>,
    manifest_exists: bool,
    configured_path: Option<String>,
    allowed_origins: Vec<String>,
    error: Option<String>,
}

fn handle_native_host_install(args: NativeHostInstallArgs) -> anyhow::Result<()> {
    let browsers = parse_native_host_browsers(&args.browsers, true)?;
    let allowed_origins =
        native_host_allowed_origins(&args.extension_ids, &args.extension_origins)?;
    let native_host_path = match args.native_host_path {
        Some(path) => path,
        None => resolve_native_host_executable_path()
            .map_err(|err| anyhow::anyhow!("failed to resolve native-host executable: {err}"))?,
    };

    let mut reports = Vec::new();
    let mut failures = Vec::new();
    for browser in browsers {
        match install_rzn_native_host_for_browser_with_origins(
            browser,
            &native_host_path,
            allowed_origins.iter().map(String::as_str),
        ) {
            Ok(report) => reports.push(report),
            Err(err) => failures.push(NativeHostCommandFailure {
                browser: Some(browser.slug().to_string()),
                error_code: "NATIVE_HOST_INSTALL_FAILED".to_string(),
                error: err.to_string(),
            }),
        }
    }

    let output = NativeHostInstallOutput {
        success: failures.is_empty(),
        reports,
        failures,
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        render_native_host_install_output(&output);
    }

    if !output.success {
        anyhow::bail!("native-host install failed for one or more browser targets");
    }
    Ok(())
}

fn handle_native_host_list(args: NativeHostListArgs) -> anyhow::Result<()> {
    let browsers = parse_native_host_browsers(&args.browsers, true)?;
    let targets = browsers
        .into_iter()
        .map(native_host_list_status)
        .collect::<Vec<_>>();
    let output = NativeHostListOutput {
        success: targets.iter().all(|target| target.error.is_none()),
        targets,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        render_native_host_list_output(&output);
    }

    Ok(())
}

async fn handle_native_host_doctor(args: NativeHostDoctorArgs) -> anyhow::Result<()> {
    let browser = args
        .browser
        .parse::<BrowserKind>()
        .map_err(|err| anyhow::anyhow!("invalid browser target `{}`: {}", args.browser, err))?;
    let extension_origin = normalize_extension_origin(
        args.extension_origin
            .as_deref()
            .unwrap_or(RZN_DEV_EXTENSION_ORIGIN),
    )
    .map_err(|err| anyhow::anyhow!("invalid extension origin: {err}"))?;
    let config = supervisor::SupervisorConfig {
        app_base: args.app_base.as_ref().map(PathBuf::from),
    };
    let extension_bundle_path = args
        .extension_dir
        .unwrap_or_else(|| default_extension_bundle_path_for_browser(browser, &config));
    let output =
        build_native_host_doctor_report(browser, extension_origin, extension_bundle_path, &config)
            .await;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        render_native_host_doctor_output(&output);
    }
    if !output.success {
        anyhow::bail!("native-host doctor found failing checks");
    }
    Ok(())
}

async fn build_native_host_doctor_report(
    browser: BrowserKind,
    extension_origin: String,
    extension_bundle_path: PathBuf,
    config: &supervisor::SupervisorConfig,
) -> NativeHostDoctorOutput {
    let mut checks = Vec::new();
    let mut manifest_path = None;
    let mut manifest = None;

    match native_host_manifest_path_for_browser(browser) {
        Ok(path) => {
            manifest_path = Some(path.clone());
            if path.exists() {
                checks.push(doctor_check(
                    "manifest_registered",
                    DoctorCheckStatus::Pass,
                    format!("manifest exists at {}", path.display()),
                ));
                match read_manifest(&path) {
                    Ok(value) => {
                        checks.push(doctor_check(
                            "manifest_json",
                            DoctorCheckStatus::Pass,
                            "manifest JSON parsed".to_string(),
                        ));
                        manifest = Some(value);
                    }
                    Err(err) => checks.push(doctor_check(
                        "manifest_json",
                        DoctorCheckStatus::Fail,
                        format!("manifest JSON is invalid: {err}"),
                    )),
                }
            } else {
                checks.push(doctor_check(
                    "manifest_registered",
                    DoctorCheckStatus::Fail,
                    format!("manifest is missing at {}", path.display()),
                ));
            }
        }
        Err(err) => checks.push(doctor_check(
            "manifest_path",
            DoctorCheckStatus::Fail,
            format!("cannot resolve manifest path: {err}"),
        )),
    }

    if let Some(manifest) = manifest.as_ref() {
        checks.extend(native_host_doctor_manifest_checks(
            manifest,
            &extension_origin,
        ));
    }
    checks.extend(native_host_doctor_extension_bundle_checks(
        &extension_bundle_path,
        browser,
        &extension_origin,
    ));

    let paths = supervisor::SupervisorPaths::for_config(config);
    checks.push(doctor_check(
        "supervisor_socket",
        if paths.socket_path.exists() {
            DoctorCheckStatus::Pass
        } else {
            DoctorCheckStatus::Warn
        },
        if paths.socket_path.exists() {
            "supervisor socket file exists".to_string()
        } else {
            "supervisor socket file is not present".to_string()
        },
    ));
    checks.push(doctor_check(
        "supervisor_token",
        if paths.token_path.exists() {
            DoctorCheckStatus::Pass
        } else {
            DoctorCheckStatus::Warn
        },
        if paths.token_path.exists() {
            "supervisor token file exists; value is intentionally not printed".to_string()
        } else {
            "supervisor token file is not present".to_string()
        },
    ));

    match query_browser_targets_compat(config).await {
        Ok(targets) => {
            checks.extend(native_host_doctor_bridge_checks(
                &targets,
                browser,
                &extension_origin,
            ));
        }
        Err(err) => checks.push(doctor_check(
            "connected_bridge",
            DoctorCheckStatus::Warn,
            format!("could not query supervisor bridge inventory: {err}"),
        )),
    }

    let success = !checks
        .iter()
        .any(|check| check.status == DoctorCheckStatus::Fail);
    NativeHostDoctorOutput {
        success,
        browser: browser.slug().to_string(),
        extension_origin,
        manifest_path,
        load_unpacked_path: extension_bundle_path,
        checks,
    }
}

fn native_host_doctor_manifest_checks(
    manifest: &NativeMessagingHostManifest,
    extension_origin: &str,
) -> Vec<NativeHostDoctorCheck> {
    let mut checks = Vec::new();
    checks.push(doctor_check(
        "host_name",
        if manifest.name == RZN_NATIVE_HOST_NAME {
            DoctorCheckStatus::Pass
        } else {
            DoctorCheckStatus::Fail
        },
        format!("manifest host name is {}", manifest.name),
    ));
    checks.push(doctor_check(
        "allowed_origin",
        if manifest
            .allowed_origins
            .iter()
            .any(|origin| origin == extension_origin)
        {
            DoctorCheckStatus::Pass
        } else {
            DoctorCheckStatus::Fail
        },
        format!("requested origin {}", extension_origin),
    ));
    checks.extend(host_path_checks(Path::new(&manifest.path)));
    checks.extend(native_host_self_test_checks(Path::new(&manifest.path)));
    checks
}

fn native_host_doctor_extension_bundle_checks(
    bundle_path: &Path,
    browser: BrowserKind,
    extension_origin: &str,
) -> Vec<NativeHostDoctorCheck> {
    let mut checks = Vec::new();
    checks.push(doctor_check(
        "extension_bundle_directory",
        if bundle_path.is_dir() {
            DoctorCheckStatus::Pass
        } else {
            DoctorCheckStatus::Fail
        },
        format!("load unpacked path: {}", bundle_path.display()),
    ));

    let manifest_path = bundle_path.join("manifest.json");
    let manifest = match read_json_file(&manifest_path) {
        Ok(value) => {
            checks.push(doctor_check(
                "extension_bundle_manifest",
                DoctorCheckStatus::Pass,
                format!("manifest JSON parsed at {}", manifest_path.display()),
            ));
            Some(value)
        }
        Err(err) => {
            checks.push(doctor_check(
                "extension_bundle_manifest",
                DoctorCheckStatus::Fail,
                format!(
                    "manifest JSON missing or invalid at {}: {err}",
                    manifest_path.display()
                ),
            ));
            None
        }
    };

    if let Some(manifest) = manifest.as_ref() {
        checks.push(extension_bundle_manifest_key_check(
            manifest,
            extension_origin,
        ));
        checks.push(doctor_check(
            "extension_bundle_native_messaging_permission",
            if json_array_contains_string(manifest.get("permissions"), "nativeMessaging") {
                DoctorCheckStatus::Pass
            } else {
                DoctorCheckStatus::Fail
            },
            "manifest permissions include nativeMessaging".to_string(),
        ));
        checks.push(extension_bundle_icons_check(bundle_path, manifest));
        checks.push(extension_bundle_background_worker_check(
            bundle_path,
            manifest,
        ));
    }

    checks.push(extension_bundle_build_target_check(bundle_path, browser));
    checks
}

fn extension_bundle_manifest_key_check(
    manifest: &Value,
    extension_origin: &str,
) -> NativeHostDoctorCheck {
    let Some(key) = manifest.get("key").and_then(Value::as_str) else {
        return doctor_check(
            "extension_bundle_manifest_key",
            DoctorCheckStatus::Fail,
            "manifest key is missing; unpacked extension ID will not be pinned".to_string(),
        );
    };

    match extension_id_from_manifest_key(key) {
        Ok(extension_id) => {
            let derived_origin = format!("chrome-extension://{extension_id}/");
            let status =
                if extension_id == RZN_DEV_EXTENSION_ID && derived_origin == extension_origin {
                    DoctorCheckStatus::Pass
                } else {
                    DoctorCheckStatus::Fail
                };
            doctor_check(
                "extension_bundle_manifest_key",
                status,
                format!(
                    "manifest key derives extension id {extension_id}; expected {RZN_DEV_EXTENSION_ID}"
                ),
            )
        }
        Err(err) => doctor_check(
            "extension_bundle_manifest_key",
            DoctorCheckStatus::Fail,
            format!("manifest key could not be decoded: {err}"),
        ),
    }
}

fn extension_bundle_icons_check(bundle_path: &Path, manifest: &Value) -> NativeHostDoctorCheck {
    let mut missing = Vec::new();
    let icons = manifest.get("icons").and_then(Value::as_object);
    for size in ["16", "32", "48", "128"] {
        let icon_path = icons
            .and_then(|icons| icons.get(size))
            .and_then(Value::as_str);
        match icon_path {
            Some(path) if bundle_path.join(path).is_file() => {}
            Some(path) => missing.push(format!("{size}:{path}")),
            None => missing.push(format!("{size}:<missing>")),
        }
    }

    doctor_check(
        "extension_bundle_icons",
        if missing.is_empty() {
            DoctorCheckStatus::Pass
        } else {
            DoctorCheckStatus::Fail
        },
        if missing.is_empty() {
            "manifest icons exist for 16, 32, 48, and 128".to_string()
        } else {
            format!("missing icon file(s): {}", missing.join(", "))
        },
    )
}

fn extension_bundle_background_worker_check(
    bundle_path: &Path,
    manifest: &Value,
) -> NativeHostDoctorCheck {
    let worker = manifest
        .pointer("/background/service_worker")
        .and_then(Value::as_str);
    let ok = worker
        .map(|worker| bundle_path.join(worker).is_file())
        .unwrap_or(false);
    doctor_check(
        "extension_bundle_background_worker",
        if ok {
            DoctorCheckStatus::Pass
        } else {
            DoctorCheckStatus::Fail
        },
        match worker {
            Some(worker) => format!("background service worker {worker}"),
            None => "background service worker is missing".to_string(),
        },
    )
}

fn extension_bundle_build_target_check(
    bundle_path: &Path,
    browser: BrowserKind,
) -> NativeHostDoctorCheck {
    let expected_target = extension_bundle_target_for_browser(browser);
    let build_path = bundle_path.join("rzn-build.json");
    let value = match read_json_file(&build_path) {
        Ok(value) => value,
        Err(err) => {
            return doctor_check(
                "extension_bundle_rzn_build_target",
                DoctorCheckStatus::Fail,
                format!(
                    "rzn-build.json missing or invalid at {}: {err}",
                    build_path.display()
                ),
            )
        }
    };
    let actual_target = value
        .get("extension_target")
        .and_then(Value::as_str)
        .unwrap_or("<missing>");

    doctor_check(
        "extension_bundle_rzn_build_target",
        if actual_target == expected_target {
            DoctorCheckStatus::Pass
        } else {
            DoctorCheckStatus::Fail
        },
        format!("rzn-build extension_target={actual_target}; expected {expected_target}"),
    )
}

fn read_json_file(path: &Path) -> anyhow::Result<Value> {
    let contents = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&contents)?)
}

fn json_array_contains_string(value: Option<&Value>, expected: &str) -> bool {
    value
        .and_then(Value::as_array)
        .map(|values| values.iter().any(|value| value.as_str() == Some(expected)))
        .unwrap_or(false)
}

fn default_extension_bundle_path_for_browser(
    browser: BrowserKind,
    config: &supervisor::SupervisorConfig,
) -> PathBuf {
    let relative = PathBuf::from("extension")
        .join("dist")
        .join(extension_bundle_target_for_browser(browser));
    if relative.exists() {
        return relative;
    }

    let installed = supervisor::SupervisorPaths::for_config(config)
        .app_base
        .join("extension")
        .join("dist")
        .join(extension_bundle_target_for_browser(browser));
    if installed.exists() {
        return installed;
    }

    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn extension_bundle_target_for_browser(browser: BrowserKind) -> &'static str {
    match browser {
        BrowserKind::Chromium => "chromium",
        BrowserKind::Edge
        | BrowserKind::EdgeBeta
        | BrowserKind::EdgeDev
        | BrowserKind::EdgeCanary => "edge",
        BrowserKind::Chrome | BrowserKind::ChromeForTesting => "chrome",
    }
}

fn extension_id_from_manifest_key(key: &str) -> anyhow::Result<String> {
    let der = base64_decode_standard(key)?;
    let digest = Sha256::digest(&der);
    let mut extension_id = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        extension_id.push((b'a' + (byte >> 4)) as char);
        extension_id.push((b'a' + (byte & 0x0f)) as char);
    }
    Ok(extension_id)
}

fn base64_decode_standard(value: &str) -> anyhow::Result<Vec<u8>> {
    let clean = value
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    if clean.is_empty() || clean.len() % 4 != 0 {
        anyhow::bail!("invalid base64 length");
    }

    let mut out = Vec::with_capacity((clean.len() / 4) * 3);
    for (chunk_index, chunk) in clean.chunks(4).enumerate() {
        let last_chunk = chunk_index == (clean.len() / 4) - 1;
        let pad2 = chunk[2] == b'=';
        let pad3 = chunk[3] == b'=';
        if (pad2 || pad3) && !last_chunk {
            anyhow::bail!("base64 padding before final chunk");
        }
        if pad2 && !pad3 {
            anyhow::bail!("invalid base64 padding");
        }

        let a = base64_value(chunk[0])?;
        let b = base64_value(chunk[1])?;
        let c = if pad2 { 0 } else { base64_value(chunk[2])? };
        let d = if pad3 { 0 } else { base64_value(chunk[3])? };
        out.push((a << 2) | (b >> 4));
        if !pad2 {
            out.push((b << 4) | (c >> 2));
        }
        if !pad3 {
            out.push((c << 6) | d);
        }
    }
    Ok(out)
}

fn base64_value(byte: u8) -> anyhow::Result<u8> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => anyhow::bail!("invalid base64 byte"),
    }
}

fn native_host_doctor_bridge_checks(
    targets: &Value,
    browser: BrowserKind,
    extension_origin: &str,
) -> Vec<NativeHostDoctorCheck> {
    let expected_id = extension_id_from_origin(extension_origin).unwrap_or(extension_origin);
    let all_targets = targets
        .get("targets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let browser_targets = all_targets
        .iter()
        .filter(|target| target.get("browser").and_then(Value::as_str) == Some(browser.slug()))
        .collect::<Vec<_>>();
    let browser_present = !browser_targets.is_empty();
    let status_for_identity = |matched: bool| {
        if matched {
            DoctorCheckStatus::Pass
        } else if browser_present {
            DoctorCheckStatus::Fail
        } else {
            DoctorCheckStatus::Warn
        }
    };
    let caller_origin_matches = browser_targets.iter().any(|target| {
        target.get("caller_origin").and_then(Value::as_str) == Some(extension_origin)
            || target.get("extension_origin").and_then(Value::as_str) == Some(extension_origin)
    });
    let extension_id_matches = browser_targets
        .iter()
        .any(|target| target.get("extension_id").and_then(Value::as_str) == Some(expected_id));
    let instance_id_present = browser_targets.iter().any(|target| {
        target
            .get("browser_instance_id")
            .and_then(Value::as_str)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    });

    vec![
        doctor_check(
            "connected_bridge_present",
            if all_targets.is_empty() {
                DoctorCheckStatus::Warn
            } else {
                DoctorCheckStatus::Pass
            },
            if all_targets.is_empty() {
                "no browser bridge is currently connected".to_string()
            } else {
                format!("{} browser bridge(s) connected", all_targets.len())
            },
        ),
        doctor_check(
            "connected_bridge_browser_matches",
            if browser_present {
                DoctorCheckStatus::Pass
            } else {
                DoctorCheckStatus::Warn
            },
            if browser_present {
                format!("connected bridge matches browser {}", browser.slug())
            } else {
                format!(
                    "no connected bridge currently reports browser {}",
                    browser.slug()
                )
            },
        ),
        doctor_check(
            "connected_bridge_caller_origin_matches",
            status_for_identity(caller_origin_matches),
            format!("expected caller origin {}", extension_origin),
        ),
        doctor_check(
            "connected_bridge_extension_id_matches",
            status_for_identity(extension_id_matches),
            format!("expected extension id {}", expected_id),
        ),
        doctor_check(
            "connected_bridge_browser_instance_id_present",
            status_for_identity(instance_id_present),
            "connected bridge reports a browser instance id".to_string(),
        ),
    ]
}

fn extension_id_from_origin(origin: &str) -> Option<&str> {
    origin
        .strip_prefix("chrome-extension://")
        .and_then(|value| value.strip_suffix('/'))
}

fn host_path_checks(path: &Path) -> Vec<NativeHostDoctorCheck> {
    let mut checks = Vec::new();
    checks.push(doctor_check(
        "host_path_exists",
        if path.exists() {
            DoctorCheckStatus::Pass
        } else {
            DoctorCheckStatus::Fail
        },
        if path.exists() {
            format!("host path exists: {}", path.display())
        } else {
            format!("host path does not exist: {}", path.display())
        },
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let executable = path
            .metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
        checks.push(doctor_check(
            "host_path_executable",
            if executable {
                DoctorCheckStatus::Pass
            } else {
                DoctorCheckStatus::Fail
            },
            if executable {
                "host path has an executable bit".to_string()
            } else {
                "host path is not executable".to_string()
            },
        ));
    }
    checks
}

fn native_host_self_test_checks(path: &Path) -> Vec<NativeHostDoctorCheck> {
    if !path.exists() || !path.is_file() {
        return Vec::new();
    }

    match Command::new(path).arg("--self-test").output() {
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            vec![
                doctor_check(
                    "host_launch_self_test",
                    if output.status.success() {
                        DoctorCheckStatus::Pass
                    } else {
                        DoctorCheckStatus::Fail
                    },
                    if output.status.success() {
                        format!(
                            "native host self-test exited successfully: {}",
                            stderr.trim()
                        )
                    } else {
                        format!(
                            "native host self-test failed with status {:?}: {}",
                            output.status.code(),
                            stderr.trim()
                        )
                    },
                ),
                doctor_check(
                    "host_self_test_stdout_clean",
                    if output.stdout.is_empty() {
                        DoctorCheckStatus::Pass
                    } else {
                        DoctorCheckStatus::Fail
                    },
                    if output.stdout.is_empty() {
                        "self-test emitted no stdout bytes".to_string()
                    } else {
                        format!("self-test emitted {} stdout byte(s)", output.stdout.len())
                    },
                ),
            ]
        }
        Err(err) => vec![doctor_check(
            "host_launch_self_test",
            DoctorCheckStatus::Fail,
            format!("failed to launch native host self-test: {err}"),
        )],
    }
}

fn doctor_check(
    name: impl Into<String>,
    status: DoctorCheckStatus,
    message: impl Into<String>,
) -> NativeHostDoctorCheck {
    NativeHostDoctorCheck {
        name: name.into(),
        status,
        message: message.into(),
    }
}

fn handle_native_host_uninstall(args: NativeHostUninstallArgs) -> anyhow::Result<()> {
    let browsers = parse_native_host_browsers(&args.browsers, false)?;
    let mut reports = Vec::new();
    let mut failures = Vec::new();

    for browser in browsers {
        match uninstall_rzn_native_host_for_browser(browser) {
            Ok(report) => reports.push(report),
            Err(err) => failures.push(NativeHostCommandFailure {
                browser: Some(browser.slug().to_string()),
                error_code: "NATIVE_HOST_UNINSTALL_FAILED".to_string(),
                error: err.to_string(),
            }),
        }
    }

    let output = NativeHostUninstallOutput {
        success: failures.is_empty(),
        reports,
        failures,
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        render_native_host_uninstall_output(&output);
    }

    if !output.success {
        anyhow::bail!("native-host uninstall failed for one or more browser targets");
    }
    Ok(())
}

fn parse_native_host_browsers(
    raw: &[String],
    default_primary: bool,
) -> anyhow::Result<Vec<BrowserKind>> {
    if raw.is_empty() && default_primary {
        return Ok(primary_native_host_browsers());
    }

    let mut browsers = Vec::new();
    let mut seen = BTreeSet::new();
    for value in raw {
        let browser = value.parse::<BrowserKind>().map_err(|_| {
            let error = serde_json::json!({
                "error_code": "INVALID_BROWSER_TARGET",
                "error": format!("Invalid browser target `{value}`."),
                "valid_slugs": valid_native_host_browser_slugs(),
            });
            anyhow::anyhow!("{error}")
        })?;
        if seen.insert(browser.slug()) {
            browsers.push(browser);
        }
    }

    if browsers.is_empty() {
        let error = serde_json::json!({
            "error_code": "MISSING_BROWSER_TARGET",
            "error": "At least one --browser target is required.",
            "valid_slugs": valid_native_host_browser_slugs(),
        });
        anyhow::bail!("{error}");
    }

    Ok(browsers)
}

fn primary_native_host_browsers() -> Vec<BrowserKind> {
    vec![
        BrowserKind::Chrome,
        BrowserKind::Chromium,
        BrowserKind::Edge,
    ]
}

fn valid_native_host_browser_slugs() -> Vec<&'static str> {
    BrowserKind::ALL
        .iter()
        .map(|browser| browser.slug())
        .collect()
}

fn native_host_allowed_origins(
    extension_ids: &[String],
    extension_origins: &[String],
) -> anyhow::Result<Vec<String>> {
    let raw = extension_ids
        .iter()
        .chain(extension_origins.iter())
        .map(String::as_str)
        .collect::<Vec<_>>();
    if raw.is_empty() {
        return Ok(vec![RZN_DEV_EXTENSION_ORIGIN.to_string()]);
    }

    normalize_extension_origins(raw).map_err(|err| {
        let error = serde_json::json!({
            "error_code": "INVALID_EXTENSION_ORIGIN",
            "error": err.to_string(),
        });
        anyhow::anyhow!("{error}")
    })
}

fn native_host_list_status(browser: BrowserKind) -> NativeHostListStatus {
    let mut status = NativeHostListStatus {
        browser,
        browser_slug: browser.slug().to_string(),
        display_name: browser.display_name().to_string(),
        install_location: None,
        manifest_exists: false,
        configured_path: None,
        allowed_origins: Vec::new(),
        error: None,
    };

    let manifest_path = match native_host_manifest_path_for_browser(browser) {
        Ok(path) => path,
        Err(err) => {
            status.error = Some(err.to_string());
            return status;
        }
    };

    status.install_location = Some(manifest_path.clone());
    status.manifest_exists = manifest_path.exists();
    if !status.manifest_exists {
        return status;
    }

    match read_manifest(&manifest_path) {
        Ok(NativeMessagingHostManifest {
            path,
            allowed_origins,
            ..
        }) => {
            status.configured_path = Some(path);
            status.allowed_origins = allowed_origins;
        }
        Err(err) => {
            status.error = Some(err.to_string());
        }
    }

    status
}

fn render_native_host_install_output(output: &NativeHostInstallOutput) {
    if !output.reports.is_empty() {
        println!("Native host installed.");
        println!("Allowed origins:");
        if let Some(first_report) = output.reports.first() {
            for origin in &first_report.allowed_origins {
                println!("  - {origin}");
            }
            if first_report
                .allowed_origins
                .iter()
                .any(|origin| origin == RZN_DEV_EXTENSION_ORIGIN)
            {
                println!("Default dev extension ID: {RZN_DEV_EXTENSION_ID}");
            }
        }
        println!("Installed registrations:");
    }
    for report in &output.reports {
        let state = if report.changed {
            "installed"
        } else {
            "unchanged"
        };
        println!(
            "  {} {}: {}",
            report.browser.slug(),
            state,
            report.manifest_path.display()
        );
    }
    for failure in &output.failures {
        eprintln!(
            "{} failed: {}",
            failure.browser.as_deref().unwrap_or("native-host"),
            failure.error
        );
    }
}

fn render_native_host_uninstall_output(output: &NativeHostUninstallOutput) {
    for report in &output.reports {
        let state = if report.removed {
            "removed"
        } else {
            "not installed"
        };
        println!(
            "{} {}: {}",
            report.browser.slug(),
            state,
            report.manifest_path.display()
        );
    }
    for failure in &output.failures {
        eprintln!(
            "{} failed: {}",
            failure.browser.as_deref().unwrap_or("native-host"),
            failure.error
        );
    }
}

fn render_native_host_list_output(output: &NativeHostListOutput) {
    for target in &output.targets {
        let state = if target.manifest_exists {
            "installed"
        } else {
            "not installed"
        };
        println!(
            "{} ({}) - {}",
            target.browser_slug, target.display_name, state
        );
        if let Some(location) = target.install_location.as_ref() {
            println!("  manifest: {}", location.display());
        }
        if let Some(configured_path) = target.configured_path.as_ref() {
            println!("  native_host_path: {configured_path}");
        }
        if !target.allowed_origins.is_empty() {
            println!("  allowed_origins: {}", target.allowed_origins.join(", "));
        }
        if let Some(error) = target.error.as_ref() {
            println!("  error: {error}");
        }
    }
}

fn render_native_host_doctor_output(output: &NativeHostDoctorOutput) {
    println!(
        "native-host doctor: browser={} extension_origin={}",
        output.browser, output.extension_origin
    );
    if let Some(path) = output.manifest_path.as_ref() {
        println!("manifest: {}", path.display());
    }
    for check in &output.checks {
        let status = match check.status {
            DoctorCheckStatus::Pass => "pass",
            DoctorCheckStatus::Warn => "warn",
            DoctorCheckStatus::Fail => "fail",
        };
        println!("- {} {}: {}", status, check.name, check.message);
    }
}

async fn handle_supervisor_commands(cmd: SupervisorCommands) -> anyhow::Result<()> {
    match cmd {
        SupervisorCommands::Serve(args) => {
            let json_output = args.json;
            let config = supervisor_config_from_common(&args);
            let paths = supervisor::SupervisorPaths::for_config(&config);
            let report = supervisor::SupervisorServeReport {
                ok: true,
                protocol: supervisor::RZN_LOCAL_PROTOCOL_VERSION,
                pid: std::process::id(),
                app_base: paths.app_base.to_string_lossy().to_string(),
                socket_path: paths.socket_path.to_string_lossy().to_string(),
                token_path: paths.token_path.to_string_lossy().to_string(),
            };
            if json_output {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("[SUPERVISOR] starting");
                println!("   ├─ Protocol: {}", report.protocol);
                println!("   ├─ PID: {}", report.pid);
                println!("   ├─ App base: {}", report.app_base);
                println!("   ├─ Socket: {}", report.socket_path);
                println!("   └─ Token: {}", report.token_path);
            }
            supervisor::serve(config).await?;
        }
        SupervisorCommands::Status(args) => {
            let result = supervisor::call(
                supervisor_config_from_common(&args),
                "runtime.status",
                json!({}),
            )
            .await?;
            render_supervisor_json_result(&result, args.json)?;
        }
        SupervisorCommands::EnsureReady(args) => {
            let config = supervisor_config_from_common(&args);
            let _status = supervisor::ensure_running(config.clone()).await?;
            let result = supervisor::call(config, "runtime.ensure_ready", json!({})).await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("[SUPERVISOR] ready");
                render_supervisor_json_result(&result, false)?;
            }
        }
        SupervisorCommands::Shutdown(args) => {
            let result = supervisor::call(
                supervisor_config_from_common(&args),
                "runtime.shutdown",
                json!({}),
            )
            .await?;
            render_supervisor_json_result(&result, args.json)?;
        }
        SupervisorCommands::Call(args) => {
            let mut params: Value = serde_json::from_str(&args.params)
                .with_context(|| format!("Invalid JSON params: {}", args.params))?;
            apply_supervisor_call_target_flags(&mut params, &args)?;
            let result = supervisor::call(
                supervisor_config_from_common(&args.common),
                &args.method,
                params,
            )
            .await?;
            render_supervisor_json_result(&result, args.common.json)?;
        }
    }
    Ok(())
}

async fn handle_browser_commands(cmd: BrowserCommands) -> anyhow::Result<()> {
    match cmd {
        BrowserCommands::Targets(args) => {
            let config = supervisor_config_from_common(&args.common);
            let _ = supervisor::ensure_running(config.clone()).await?;
            let result = query_browser_targets_compat(&config).await?;
            let result = browser_targets_with_default(result, &config)?;
            render_supervisor_json_result(&result, args.common.json)?;
        }
        BrowserCommands::Default(args) => {
            let config = supervisor_config_from_common(&args.common);
            let output = browser_default_status(&config)?;
            if args.common.json {
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                render_browser_default_status(&output);
            }
        }
        BrowserCommands::Set(args) => {
            let config = supervisor_config_from_common(&args.common);
            let target = browser_set_target_value(&args)?;
            write_browser_default_target(&config, target.clone())?;
            let output = browser_default_status(&config)?;
            if args.common.json {
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Default browser target saved.");
                render_browser_default_status(&output);
            }
        }
        BrowserCommands::Clear(args) => {
            let config = supervisor_config_from_common(&args.common);
            clear_browser_default_target(&config)?;
            let output = browser_default_status(&config)?;
            if args.common.json {
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Default browser target cleared.");
                render_browser_default_status(&output);
            }
        }
    }
    Ok(())
}

async fn query_browser_targets_compat(
    config: &supervisor::SupervisorConfig,
) -> anyhow::Result<Value> {
    match supervisor::call(config.clone(), "browser.targets", json!({})).await {
        Ok(result) => Ok(result),
        Err(err) if supervisor_unknown_method_error(&err, "browser.targets") => {
            if let Ok(result) = supervisor::call(config.clone(), "runtime.bridges", json!({})).await
            {
                return Ok(result);
            }
            let readiness = supervisor::call(
                config.clone(),
                "runtime.ensure_ready",
                json!({
                    "bridge_wait_ms": 0,
                    "bridge_probe_timeout_ms": 2_000
                }),
            )
            .await
            .ok();
            let status = supervisor::call(config.clone(), "runtime.status", json!({}))
                .await
                .with_context(|| {
                    "browser.targets is unsupported by the running supervisor and runtime.status fallback failed"
                })?;
            Ok(browser_targets_from_runtime_status(
                status,
                readiness.as_ref(),
            ))
        }
        Err(err) => Err(err),
    }
}

fn supervisor_unknown_method_error(err: &anyhow::Error, method: &str) -> bool {
    let message = err.to_string();
    message.contains("Unknown supervisor method") && message.contains(method)
}

fn browser_targets_from_runtime_status(status: Value, readiness: Option<&Value>) -> Value {
    let health = status
        .pointer("/native_host_bridge/health")
        .cloned()
        .unwrap_or(Value::Null);
    let mut targets = Vec::new();
    let probe_target = readiness.and_then(browser_target_from_readiness_probe);

    if let Some(bridges) = health.get("bridges").and_then(Value::as_object) {
        let mut bridge_ids = bridges.keys().cloned().collect::<Vec<_>>();
        bridge_ids.sort();
        for bridge_id in bridge_ids {
            let Some(bridge_health) = bridges.get(&bridge_id) else {
                continue;
            };
            if bridge_health
                .get("connected")
                .and_then(Value::as_bool)
                .unwrap_or(true)
            {
                targets.push(browser_target_from_runtime_health(
                    Some(&bridge_id),
                    bridge_health,
                    probe_target.as_ref(),
                ));
            }
        }
    }

    if targets.is_empty()
        && status
            .pointer("/native_host_bridge/connected")
            .and_then(Value::as_bool)
            == Some(true)
    {
        targets.push(browser_target_from_runtime_health(
            None,
            &health,
            probe_target.as_ref(),
        ));
    }

    let target_count = targets.len();
    json!({
        "ok": true,
        "version": "rzn.runtime.bridges.compat.v1",
        "status": if target_count == 0 { "no_bridges_connected" } else { "connected" },
        "target_count": target_count,
        "bridge_count": target_count,
        "targets": targets.clone(),
        "bridges": targets,
        "compat_source": "runtime.status"
    })
}

fn browser_target_from_runtime_health(
    bridge_id: Option<&str>,
    health: &Value,
    probe_target: Option<&Value>,
) -> Value {
    let metadata = health
        .get("current_bridge_metadata")
        .unwrap_or(&Value::Null);
    let bridge_id = bridge_id
        .map(str::to_string)
        .or_else(|| probe_target.and_then(|target| target_string_field(target, "bridge_id")))
        .or_else(|| json_string_at_any(health, &["/current_bridge_id"]));
    let browser_instance_id = probe_target
        .and_then(|target| target_string_field(target, "browser_instance_id"))
        .or_else(|| json_string_at_any(metadata, &["/browser_instance_id"]));
    let browser = probe_target
        .and_then(|target| target_string_field(target, "browser"))
        .or_else(|| json_string_at_any(metadata, &["/extension_target", "/browser"]));
    let extension_target_hint = probe_target
        .and_then(|target| target_string_field(target, "extension_target_hint"))
        .or_else(|| json_string_at_any(metadata, &["/extension_target_hint"]));
    let extension_id = probe_target
        .and_then(|target| target_string_field(target, "extension_id"))
        .or_else(|| {
            json_string_at_any(
                metadata,
                &["/caller_extension_id", "/extension_reported_id"],
            )
        });
    let caller_origin = probe_target
        .and_then(|target| target_string_field(target, "caller_origin"))
        .or_else(|| {
            json_string_at_any(metadata, &["/caller_origin", "/extension_reported_origin"])
        });

    json!({
        "bridge_id": bridge_id.clone(),
        "supervisor_bridge_id": bridge_id,
        "browser_instance_id": browser_instance_id.clone(),
        "browser": browser.clone(),
        "extension_target": browser,
        "extension_target_hint": extension_target_hint,
        "extension_id": extension_id,
        "caller_origin": caller_origin,
        "last_ping_status": runtime_health_last_ping_status(health),
        "last_successful_ping_at_ms": health.get("last_successful_ping_at_ms").cloned().unwrap_or(Value::Null),
        "last_successful_ping_latency_ms": health.get("last_successful_ping_latency_ms").cloned().unwrap_or(Value::Null),
        "active_session_count": 0,
        "target_flags": {
            "bridge": bridge_id,
            "browser_instance": browser_instance_id,
            "browser": browser
        }
    })
}

fn browser_target_from_readiness_probe(readiness: &Value) -> Option<Value> {
    let response = readiness.pointer("/native_host_bridge/probe/response")?;
    let bridge_id = json_string_at_any(
        response,
        &[
            "/bridge_id",
            "/supervisor_bridge_id",
            "/resolved_browser_target/bridge_id",
            "/resolved_browser_target/supervisor_bridge_id",
            "/result/supervisor_bridge_id",
            "/result/result/supervisor_bridge_id",
        ],
    );
    let browser_instance_id = json_string_at_any(
        response,
        &[
            "/browser_instance_id",
            "/resolved_browser_target/browser_instance_id",
            "/result/browser_instance_id",
            "/result/result/browser_instance_id",
        ],
    );
    let browser = json_string_at_any(
        response,
        &[
            "/browser",
            "/extension_target",
            "/resolved_browser_target/browser",
            "/resolved_browser_target/extension_target",
            "/result/browser",
            "/result/extension_target",
            "/result/result/browser",
            "/result/result/extension_target",
        ],
    );
    let extension_target_hint = json_string_at_any(
        response,
        &[
            "/extension_target_hint",
            "/result/extension_target_hint",
            "/result/result/extension_target_hint",
        ],
    );
    let extension_id = json_string_at_any(
        response,
        &[
            "/extension_id",
            "/caller_extension_id",
            "/result/extension_id",
            "/result/caller_extension_id",
            "/result/result/extension_id",
            "/result/result/caller_extension_id",
        ],
    );
    let caller_origin = json_string_at_any(
        response,
        &[
            "/caller_origin",
            "/extension_origin",
            "/result/caller_origin",
            "/result/extension_origin",
            "/result/result/caller_origin",
            "/result/result/extension_origin",
        ],
    );

    if browser_instance_id.is_none()
        && browser.is_none()
        && extension_target_hint.is_none()
        && extension_id.is_none()
        && caller_origin.is_none()
    {
        return None;
    }

    Some(json!({
        "bridge_id": bridge_id.clone(),
        "supervisor_bridge_id": bridge_id,
        "browser_instance_id": browser_instance_id,
        "browser": browser.clone(),
        "extension_target": browser,
        "extension_target_hint": extension_target_hint,
        "extension_id": extension_id,
        "caller_origin": caller_origin
    }))
}

fn target_string_field(target: &Value, field: &str) -> Option<String> {
    target
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "<unknown>")
        .map(str::to_string)
}

fn json_string_at_any(value: &Value, pointers: &[&str]) -> Option<String> {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "<unknown>")
        .map(str::to_string)
}

fn runtime_health_last_ping_status(health: &Value) -> &'static str {
    if health
        .get("last_successful_ping_at_ms")
        .is_some_and(|value| !value.is_null())
    {
        "ok"
    } else if health
        .get("last_failure_at_ms")
        .is_some_and(|value| !value.is_null())
    {
        "failed"
    } else {
        "unknown"
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct BrowserDefaultConfig {
    version: String,
    target: Value,
}

fn browser_set_target_value(args: &BrowserSetArgs) -> anyhow::Result<Value> {
    if args.browser_name.is_some() && args.target.browser.is_some() {
        anyhow::bail!("pass either browser shorthand or --browser, not both");
    }
    let mut target = args.target.clone();
    if let Some(browser_name) = args.browser_name.as_ref() {
        target.browser = Some(browser_name.clone());
    }
    let value = browser_target_routing_value(&target)?;
    value.ok_or_else(|| {
        anyhow::anyhow!(
            "choose a default with `rzn-browser browser set chromium`, `--browser edge`, `--browser-instance <id>`, or `--bridge <id>`"
        )
    })
}

fn browser_default_config_path(config: &supervisor::SupervisorConfig) -> PathBuf {
    supervisor::SupervisorPaths::for_config(config)
        .app_base
        .join("config")
        .join("browser-default.json")
}

fn read_browser_default_target(
    config: &supervisor::SupervisorConfig,
) -> anyhow::Result<Option<Value>> {
    let path = browser_default_config_path(config);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    let config: BrowserDefaultConfig =
        serde_json::from_str(&contents).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(config.target))
}

fn write_browser_default_target(
    config: &supervisor::SupervisorConfig,
    target: Value,
) -> anyhow::Result<()> {
    let path = browser_default_config_path(config);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let payload = BrowserDefaultConfig {
        version: "rzn.browser-default.v1".to_string(),
        target,
    };
    fs::write(&path, serde_json::to_string_pretty(&payload)?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn clear_browser_default_target(config: &supervisor::SupervisorConfig) -> anyhow::Result<()> {
    let path = browser_default_config_path(config);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("remove {}", path.display())),
    }
}

fn browser_target_preference(default_target: Value) -> Value {
    json!({
        "preferred": default_target,
        "fallback": "single_connected"
    })
}

fn browser_target_routing_value_with_default(
    target: &BrowserTargetArgs,
    config: &supervisor::SupervisorConfig,
) -> anyhow::Result<Option<Value>> {
    if let Some(explicit) = browser_target_routing_value(target)? {
        return Ok(Some(explicit));
    }
    Ok(read_browser_default_target(config)?.map(browser_target_preference))
}

fn browser_default_status(config: &supervisor::SupervisorConfig) -> anyhow::Result<Value> {
    let path = browser_default_config_path(config);
    Ok(json!({
        "ok": true,
        "path": path,
        "default_target": read_browser_default_target(config)?,
        "set_examples": [
            "rzn-browser browser set chrome",
            "rzn-browser browser set edge",
            "rzn-browser browser set chromium",
            "rzn-browser browser set --browser-instance <browser_instance_id>",
            "rzn-browser browser set --bridge <bridge_id>"
        ]
    }))
}

fn browser_targets_with_default(
    mut targets: Value,
    config: &supervisor::SupervisorConfig,
) -> anyhow::Result<Value> {
    let default_target = read_browser_default_target(config)?;
    let available_browsers = targets
        .get("targets")
        .and_then(Value::as_array)
        .map(|items| {
            let mut browsers = items
                .iter()
                .filter_map(|target| {
                    target
                        .get("browser")
                        .or_else(|| target.get("extension_target"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect::<Vec<_>>();
            browsers.sort();
            browsers.dedup();
            browsers
        })
        .unwrap_or_default();
    if let Some(map) = targets.as_object_mut() {
        map.insert(
            "default_target".to_string(),
            default_target.unwrap_or(Value::Null),
        );
        map.insert(
            "default_target_path".to_string(),
            Value::String(browser_default_config_path(config).display().to_string()),
        );
        map.insert("available_browsers".to_string(), json!(available_browsers));
        map.insert(
            "set_examples".to_string(),
            json!([
                "rzn-browser browser set chrome",
                "rzn-browser browser set edge",
                "rzn-browser browser set chromium",
                "rzn-browser browser set --browser-instance <browser_instance_id>",
                "rzn-browser browser set --bridge <bridge_id>"
            ]),
        );
    }
    Ok(targets)
}

fn render_browser_default_status(value: &Value) {
    println!("Browser default");
    match value.get("default_target") {
        Some(Value::Null) | None => {
            println!("default: none");
            println!("set one with:");
            println!("  rzn-browser browser set chromium");
        }
        Some(target) => {
            println!("default: {}", browser_target_display(target));
        }
    }
    if let Some(path) = value.get("path").and_then(Value::as_str) {
        println!("path: {path}");
    }
}

fn browser_target_display(target: &Value) -> String {
    if let Some(browser) = target.get("browser").and_then(Value::as_str) {
        return format!("browser={browser}");
    }
    if let Some(browser_instance_id) = target.get("browser_instance_id").and_then(Value::as_str) {
        return format!("browser_instance_id={browser_instance_id}");
    }
    if let Some(bridge_id) = target
        .get("bridge_id")
        .or_else(|| target.get("supervisor_bridge_id"))
        .and_then(Value::as_str)
    {
        return format!("bridge_id={bridge_id}");
    }
    target.to_string()
}

fn apply_supervisor_call_target_flags(
    params: &mut Value,
    args: &SupervisorCallArgs,
) -> anyhow::Result<()> {
    let Some(map) = params.as_object_mut() else {
        anyhow::bail!("Supervisor call params must be a JSON object");
    };
    let config = supervisor_config_from_common(&args.common);
    if let Some(browser_target) = browser_target_routing_value_with_default(&args.target, &config)?
    {
        map.insert("browser_target".to_string(), browser_target);
    }
    if let Some(tab_ref) = args.tab_ref.as_ref() {
        map.insert("tab_ref".to_string(), Value::String(tab_ref.clone()));
    }
    if let Some(tab) = args.tab {
        map.insert("current_tab_id".to_string(), Value::Number(tab.into()));
        map.insert("tab_id".to_string(), Value::Number(tab.into()));
    }
    Ok(())
}

fn browser_target_routing_value(target: &BrowserTargetArgs) -> anyhow::Result<Option<Value>> {
    let specified = [
        target.bridge_id.as_ref().map(|_| "--bridge"),
        target
            .browser_instance_id
            .as_ref()
            .map(|_| "--browser-instance"),
        target.browser.as_ref().map(|_| "--browser"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if specified.len() > 1 {
        anyhow::bail!(
            "conflicting browser target flags: {}. Use one target flag; --bridge is the most specific selector, followed by --browser-instance, then --browser.",
            specified.join(", ")
        );
    }

    if let Some(bridge_id) = target.bridge_id.as_ref() {
        return Ok(Some(json!({ "bridge_id": bridge_id })));
    }
    if let Some(browser_instance_id) = target.browser_instance_id.as_ref() {
        return Ok(Some(json!({ "browser_instance_id": browser_instance_id })));
    }
    if let Some(browser) = target.browser.as_ref() {
        return Ok(Some(json!({ "browser": browser })));
    }
    Ok(None)
}

fn supervisor_config_from_common(args: &SupervisorCommonArgs) -> supervisor::SupervisorConfig {
    supervisor::SupervisorConfig {
        app_base: args.app_base.as_ref().map(PathBuf::from),
    }
}

fn render_supervisor_json_result(value: &Value, json_output: bool) -> anyhow::Result<()> {
    if json_output {
        println!("{}", serde_json::to_string_pretty(value)?);
        return Ok(());
    }
    if let Some(rendered) = result_formatter::format_browser_target_error(value) {
        println!("{rendered}");
        return Ok(());
    }
    if let Some(rendered) = result_formatter::format_browser_targets_result(value) {
        println!("{rendered}");
        return Ok(());
    }
    if let Some(rendered) = result_formatter::format_browser_tab_context_result(value) {
        println!("{rendered}");
        return Ok(());
    }
    println!("{}", serde_json::to_string_pretty(value)?);
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
        target,
        params,
        snapshot,
        app_base,
        output_file,
        download_dir,
    } = args;

    handle_supervisor_run(SupervisorRunArgs {
        workflow_ref,
        target,
        params,
        snapshot,
        app_base,
        output_file,
        download_dir,
    })
    .await
}

async fn handle_heal(args: HealArgs) -> anyhow::Result<()> {
    let json_output = args.json;
    let config = supervisor::SupervisorConfig {
        app_base: args.app_base.as_ref().map(PathBuf::from),
    };
    supervisor::ensure_running(config.clone()).await?;
    let report = supervisor::call(config, "runtime.heal", json!({})).await?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        render_supervisor_heal_report(&report);
    }
    Ok(())
}

fn render_supervisor_heal_report(report: &Value) {
    let ready = report
        .get("ready")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let bridge_connected = report
        .pointer("/readiness/native_host_bridge/connected")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let bridge_responsive = report
        .pointer("/readiness/native_host_bridge/responsive")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let post_probe_recovery_attempted = report
        .get("post_probe_recovery_attempted")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    println!("[HEAL] Browser supervisor runtime");
    println!(
        "   ├─ Supervisor: {}",
        if ready { "ready" } else { "reported issues" }
    );
    println!(
        "   ├─ Native-host bridge: {} / {}",
        if bridge_connected {
            "connected"
        } else {
            "not connected"
        },
        if bridge_responsive {
            "responsive"
        } else {
            "not responsive"
        }
    );
    println!(
        "   ├─ Post-probe recovery: {}",
        if post_probe_recovery_attempted {
            "attempted"
        } else {
            "not needed"
        }
    );
    println!(
        "   └─ Result: {}",
        if ready { "done" } else { "needs attention" }
    );
}

fn extract_output_overrides(
    params: &mut HashMap<String, String>,
    cli_output_file: Option<PathBuf>,
    cli_download_dir: Option<PathBuf>,
) -> (Option<PathBuf>, Option<PathBuf>) {
    let from_params = |params: &mut HashMap<String, String>, keys: &[&str]| -> Option<PathBuf> {
        for key in keys {
            if let Some(value) = params.remove(*key) {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return Some(PathBuf::from(trimmed));
                }
            }
        }
        None
    };
    let output_file =
        cli_output_file.or_else(|| from_params(params, &["output_file", "output-file"]));
    let download_dir =
        cli_download_dir.or_else(|| from_params(params, &["download_dir", "download-dir"]));
    (output_file, download_dir)
}

fn enforce_cli_output_side_effect_policy(
    workflow_path: &Path,
    output_file: Option<&Path>,
    download_dir: Option<&Path>,
) -> anyhow::Result<()> {
    let required = required_cli_output_side_effects(output_file, download_dir);
    if required.is_empty() {
        return Ok(());
    }

    let Some(declared) = declared_manifest_side_effects(workflow_path)? else {
        return Ok(());
    };

    let missing = required
        .iter()
        .filter(|effect| !declared.contains(effect.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }

    anyhow::bail!(
        "CLI output side-effect policy blocked {}: requested output post-processing requires [{}], but manifest declares [{}]",
        workflow_path.display(),
        missing.join(", "),
        declared.into_iter().collect::<Vec<_>>().join(", ")
    )
}

fn required_cli_output_side_effects(
    output_file: Option<&Path>,
    download_dir: Option<&Path>,
) -> BTreeSet<String> {
    let mut required = BTreeSet::new();
    if output_file.is_some() {
        required.insert("file_write".to_string());
    }
    if download_dir.is_some() {
        required.insert("download".to_string());
        required.insert("external_read".to_string());
        required.insert("file_write".to_string());
        required.insert("network_access".to_string());
    }
    required
}

fn declared_manifest_side_effects(
    workflow_path: &Path,
) -> anyhow::Result<Option<BTreeSet<String>>> {
    let reference = workflow_path.to_string_lossy();
    let (_contract_path, value) = load_workflow_contract_value(&reference)?;
    if value.get("schema_version").and_then(|value| value.as_str())
        != Some(WORKFLOW_CONTRACT_VERSION)
    {
        return Ok(None);
    }

    let manifest = validate_manifest_value(&value).map_err(|issues| {
        let messages = issues
            .into_iter()
            .map(|issue| {
                if issue.field.trim().is_empty() {
                    issue.message
                } else {
                    format!("{}: {}", issue.field, issue.message)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::anyhow!("invalid workflow manifest: {messages}")
    })?;

    Ok(Some(
        manifest
            .side_effects
            .iter()
            .map(|effect| effect.class.as_str().to_string())
            .collect(),
    ))
}

async fn process_workflow_output(
    payload: Option<&Value>,
    output_file: Option<&Path>,
    download_dir: Option<&Path>,
) -> anyhow::Result<()> {
    let output_payload = payload.map(workflow_output_payload);
    if let Some(path) = output_file {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let body = output_payload
            .map(workflow_output_body)
            .unwrap_or_else(|| "null".to_string());
        tokio::fs::write(path, body)
            .await
            .with_context(|| format!("write {}", path.display()))?;
        println!("[OK] Wrote output to {}", path.display());
    }

    if let Some(dir) = download_dir {
        let Some(payload) = output_payload else {
            eprintln!("[WARN] --download-dir requested but workflow returned no result payload");
            return Ok(());
        };
        download_payload_assets(payload, dir).await?;
    }

    Ok(())
}

fn workflow_output_payload(payload: &Value) -> &Value {
    if payload.get("version").and_then(|value| value.as_str()) == Some("rzn.run_result.v2") {
        return payload.get("output").unwrap_or(payload);
    }
    payload
}

fn workflow_output_body(payload: &Value) -> String {
    payload
        .get("markdown")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string())
        })
}

fn collect_string_array(payload: &Value, key: &str) -> Vec<String> {
    payload
        .get(key)
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Candidate objects that may carry the downloadable-asset arrays. The CLI run
/// result nests the workflow's selected output under `output` (and some
/// workflows under `output.result` / `result`), so search those levels too
/// rather than only the payload root.
fn asset_containers(payload: &Value) -> Vec<&Value> {
    let mut out = vec![payload];
    if let Some(output) = payload.get("output") {
        out.push(output);
        if let Some(result) = output.get("result") {
            out.push(result);
        }
    }
    if let Some(result) = payload.get("result") {
        out.push(result);
    }
    out
}

/// First non-empty string array found for `key` across the candidate containers.
fn collect_string_array_nested(payload: &Value, key: &str) -> Vec<String> {
    for container in asset_containers(payload) {
        let found = collect_string_array(container, key);
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

struct AttachmentAsset {
    url: String,
    filename: Option<String>,
}

/// Attachment downloads keyed by `attachment_urls`: each item is either a bare
/// URL string or `{ "url": ..., "filename"/"name": ... }`. De-duplicates by URL.
fn collect_attachment_assets(payload: &Value) -> Vec<AttachmentAsset> {
    for container in asset_containers(payload) {
        let Some(items) = container.get("attachment_urls").and_then(|v| v.as_array()) else {
            continue;
        };
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for item in items {
            let asset = if let Some(url) = item.as_str() {
                AttachmentAsset {
                    url: url.to_string(),
                    filename: None,
                }
            } else if let Some(obj) = item.as_object() {
                let Some(url) = obj.get("url").and_then(|u| u.as_str()) else {
                    continue;
                };
                let filename = obj
                    .get("filename")
                    .or_else(|| obj.get("name"))
                    .and_then(|f| f.as_str())
                    .map(str::to_string);
                AttachmentAsset {
                    url: url.to_string(),
                    filename,
                }
            } else {
                continue;
            };
            if seen.insert(asset.url.clone()) {
                out.push(asset);
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    Vec::new()
}

fn sanitize_filename_segment(input: &str, max_len: usize) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.len() > max_len {
        out.truncate(max_len);
    }
    if out.is_empty() {
        "asset".to_string()
    } else {
        out
    }
}

fn filename_from_url(url: &str, fallback_ext: &str) -> String {
    let parsed = Url::parse(url).ok();
    let stem = parsed
        .as_ref()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.next_back().map(str::to_string))
        })
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "asset".to_string());
    let host = parsed
        .as_ref()
        .map(|url| url.host_str().unwrap_or("").to_string())
        .unwrap_or_default();
    let combined = if host.is_empty() {
        stem
    } else {
        format!("{}_{}", host, stem)
    };
    let mut name = sanitize_filename_segment(&combined, 96);
    if !name.contains('.') && !fallback_ext.is_empty() {
        name.push('.');
        name.push_str(fallback_ext);
    }
    name
}

async fn download_payload_assets(payload: &Value, dir: &Path) -> anyhow::Result<()> {
    let images = collect_string_array_nested(payload, "image_urls");
    let videos = collect_string_array_nested(payload, "video_urls");
    let links = collect_string_array_nested(payload, "external_links");
    let attachments = collect_attachment_assets(payload);

    tokio::fs::create_dir_all(dir)
        .await
        .with_context(|| format!("create {}", dir.display()))?;

    let client = reqwest::Client::builder()
        .user_agent("rzn-browser workflow downloader")
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()
        .context("build workflow download HTTP client")?;

    let mut manifest = Vec::new();
    let mut downloaded = 0usize;
    for (kind, urls, fallback_ext) in [
        ("image", images, "jpg"),
        ("video", videos, "mp4"),
        ("page", links, "html"),
    ] {
        let subdir = dir.join(format!("{kind}s"));
        if !urls.is_empty() {
            tokio::fs::create_dir_all(&subdir)
                .await
                .with_context(|| format!("create {}", subdir.display()))?;
        }
        for (index, url) in urls.iter().enumerate() {
            let dest = subdir.join(format!(
                "{:03}_{}",
                index + 1,
                filename_from_url(url, fallback_ext)
            ));
            match download_one(&client, url, &dest).await {
                Ok(bytes) => {
                    downloaded += 1;
                    manifest.push(json!({
                        "kind": kind,
                        "url": url,
                        "path": dest.strip_prefix(dir).unwrap_or(&dest).to_string_lossy(),
                        "bytes": bytes
                    }));
                }
                Err(err) => {
                    eprintln!("[WARN] download failed for {}: {}", url, err);
                    manifest.push(json!({
                        "kind": kind,
                        "url": url,
                        "error": err.to_string()
                    }));
                }
            }
        }
    }

    if !attachments.is_empty() {
        let subdir = dir.join("attachments");
        tokio::fs::create_dir_all(&subdir)
            .await
            .with_context(|| format!("create {}", subdir.display()))?;
        for (index, asset) in attachments.iter().enumerate() {
            let name = asset
                .filename
                .as_deref()
                .map(|f| sanitize_filename_segment(f, 160))
                .unwrap_or_else(|| filename_from_url(&asset.url, "bin"));
            let dest = subdir.join(format!("{:03}_{}", index + 1, name));
            match download_one(&client, &asset.url, &dest).await {
                Ok(bytes) => {
                    downloaded += 1;
                    manifest.push(json!({
                        "kind": "attachment",
                        "url": asset.url,
                        "path": dest.strip_prefix(dir).unwrap_or(&dest).to_string_lossy(),
                        "bytes": bytes
                    }));
                }
                Err(err) => {
                    eprintln!("[WARN] download failed for {}: {}", asset.url, err);
                    manifest.push(json!({
                        "kind": "attachment",
                        "url": asset.url,
                        "error": err.to_string()
                    }));
                }
            }
        }
    }

    let manifest_path = dir.join("manifest.json");
    tokio::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&json!({
            "downloaded": downloaded,
            "items": manifest
        }))?,
    )
    .await
    .with_context(|| format!("write {}", manifest_path.display()))?;
    println!("[OK] Wrote download manifest: {}", manifest_path.display());
    Ok(())
}

async fn download_one(client: &reqwest::Client, url: &str, dest: &Path) -> anyhow::Result<u64> {
    let bytes = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("status {url}"))?
        .bytes()
        .await
        .with_context(|| format!("read body {url}"))?;
    let len = bytes.len() as u64;
    tokio::fs::write(dest, &bytes)
        .await
        .with_context(|| format!("write {}", dest.display()))?;
    Ok(len)
}

async fn handle_supervisor_run(args: SupervisorRunArgs) -> anyhow::Result<()> {
    let workflow_ref = workflow_ref_value(&args.workflow_ref)?;
    let resolved_workflow = match resolve_named_workflow_path(&workflow_ref) {
        Ok(path) => path,
        Err(err) => {
            if let Some(rendered) = render_run_system_discovery(&workflow_ref)? {
                println!("{}", rendered);
                return Ok(());
            }
            return Err(err);
        }
    };
    let mut params = normalize_run_params(args.params.into_iter().collect::<HashMap<_, _>>())?;
    let supervisor_config = supervisor::SupervisorConfig {
        app_base: args.app_base.as_ref().map(PathBuf::from),
    };
    let browser_target =
        browser_target_routing_value_with_default(&args.target, &supervisor_config)?;
    let (output_file, download_dir) =
        extract_output_overrides(&mut params, args.output_file, args.download_dir);
    if let Some(rendered) =
        render_run_missing_parameter_guidance(&workflow_ref, &resolved_workflow, &params)?
    {
        println!("{}", rendered);
        return Ok(());
    }
    enforce_cli_output_side_effect_policy(
        &resolved_workflow,
        output_file.as_deref(),
        download_dir.as_deref(),
    )?;

    println!("[LIST] RZN BROWSER RUN");
    println!("   ├─ Workflow: {}", workflow_ref);
    println!("   ├─ Resolved: {}", resolved_workflow.display());
    println!("   ├─ Backend: supervisor");
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

    let snapshot_mode = match args.snapshot.to_lowercase().as_str() {
        "after-step" | "after" => SnapshotMode::AfterStep,
        "none" => SnapshotMode::None,
        _ => SnapshotMode::OnError,
    };

    let config = SupervisorRunConfig {
        workflow_path: resolved_workflow.to_string_lossy().to_string(),
        params,
        snapshot_mode,
        app_base: args.app_base,
        browser_target,
    };

    match native_runner::run_supervisor_workflow(config).await {
        Ok(payload) => {
            process_workflow_output(
                payload.as_ref(),
                output_file.as_deref(),
                download_dir.as_deref(),
            )
            .await?;
            Ok(())
        }
        Err(err) => {
            if let Some(rendered) =
                render_run_invalid_parameter_guidance(&workflow_ref, &resolved_workflow, &err)?
            {
                println!("{}", rendered);
                return Ok(());
            }
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
    let host = host_port.split('@').next_back().unwrap_or(host_port);
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

    let engine_config = to_engine_plan_config(&config);
    let mut planner = if args.pure_llm {
        LLMAutonomousPlanner::new_with_options_and_config(
            llm_client,
            broker_client,
            rzn_plan::llm_autonomous::LLMAutonomousOptions {
                enable_macros: false,
            },
            &engine_config,
        )
    } else {
        LLMAutonomousPlanner::new_with_config(llm_client, broker_client, &engine_config)
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
                        if let Some(first) = items.first().and_then(|v| v.as_array()) {
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
        WorkflowCommands::ValidateCatalog(args) => handle_catalog_validate(args).await,
        WorkflowCommands::Capability(cmd) => handle_capability_commands(cmd).await,
        WorkflowCommands::Dirs => handle_workflow_dirs(),
        WorkflowCommands::Add(args) => handle_workflow_add(args).await,
        WorkflowCommands::Pull(args) => handle_workflow_pull(args).await,
        WorkflowCommands::Show(args) => handle_workflow_show(args, config).await,
        WorkflowCommands::Inspect(args) => handle_workflow_inspect(args).await,
        WorkflowCommands::Contract(args) => handle_workflow_contract(args).await,
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

#[derive(Debug, Clone, Serialize)]
struct WorkflowContractView {
    reference: String,
    path: String,
    schema_version: String,
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    system: String,
    capability: String,
    inputs: Vec<WorkflowContractInputView>,
    output: WorkflowContractOutputView,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    side_effects: Vec<WorkflowContractSideEffectView>,
    runtime: WorkflowContractRuntimeView,
    step_count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowContractInputView {
    name: String,
    kind: String,
    required: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    sensitive: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    enum_values: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowContractOutputView {
    #[serde(skip_serializing_if = "Option::is_none")]
    schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    returns: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selector_step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selector_path: Option<String>,
    prefer_downloads: bool,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowContractSideEffectView {
    class: String,
    idempotency: String,
    confirmation_required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowContractRuntimeView {
    actor: String,
    requires_existing_session: bool,
    requires_cdp: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workflow_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workflow_path: Option<String>,
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    strict: bool,
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

fn load_workflow_contract_value(reference: &str) -> anyhow::Result<(PathBuf, Value)> {
    if let Some(manifest_path) = find_manifest_path_for_reference(reference) {
        return read_contract_value_from_path(&manifest_path);
    }

    let (resolved, value) = load_workflow_value(reference)?;
    if value.get("schema_version").and_then(|value| value.as_str())
        == Some(WORKFLOW_CONTRACT_VERSION)
    {
        return Ok((resolved, value));
    }

    if let Some(manifest_path) = find_manifest_path_for_runtime_workflow(&resolved) {
        return read_contract_value_from_path(&manifest_path);
    }

    Ok((resolved, value))
}

fn read_contract_value_from_path(path: &std::path::Path) -> anyhow::Result<(PathBuf, Value)> {
    let content = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read manifest {}: {}", path.display(), e))?;
    let value: Value = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("invalid manifest JSON in {}: {}", path.display(), e))?;
    Ok((path.to_path_buf(), value))
}

fn find_manifest_path_for_reference(reference: &str) -> Option<PathBuf> {
    let normalized = normalize_contract_ref(reference);
    let (system, workflow) = normalized.split_once('/')?;
    let expected_capability = format!("{}.{}", system, workflow.replace('_', "."));
    let expected_dash = format!("{}-{}", system, workflow);
    let capabilities = list_capabilities_with_query(&CapabilityCatalogQuery::default()).ok()?;
    capabilities.into_iter().find_map(|entry| {
        let capability = entry.capability_id.replace('_', ".");
        let matches = entry.system == system
            && (capability == expected_capability
                || entry.workflow == workflow
                || entry.workflow == expected_dash
                || entry.workflow.ends_with(&format!("-{}", workflow)));
        matches.then(|| PathBuf::from(entry.manifest_path))
    })
}

fn normalize_contract_ref(reference: &str) -> String {
    reference
        .trim()
        .replace('\\', "/")
        .trim_matches('/')
        .split('/')
        .map(slugify_contract_component)
        .collect::<Vec<_>>()
        .join("/")
}

fn slugify_contract_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn find_manifest_path_for_runtime_workflow(runtime_path: &std::path::Path) -> Option<PathBuf> {
    let runtime = canonicalize_for_compare(runtime_path);
    let capabilities = list_capabilities_with_query(&CapabilityCatalogQuery::default()).ok()?;
    capabilities.into_iter().find_map(|entry| {
        let workflow_path = canonicalize_for_compare(Path::new(&entry.workflow_path));
        (workflow_path == runtime).then(|| PathBuf::from(entry.manifest_path))
    })
}

fn canonicalize_for_compare(path: &std::path::Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
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
    let uses_current_tab = uses_current_tab
        || value
            .pointer("/runtime/requires_existing_session")
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

fn build_workflow_contract_view(
    reference: &str,
    resolved_path: &std::path::Path,
    value: &Value,
    entry: Option<&NamedWorkflowEntry>,
) -> anyhow::Result<WorkflowContractView> {
    if value.get("schema_version").and_then(|value| value.as_str())
        == Some(WORKFLOW_CONTRACT_VERSION)
    {
        let manifest = validate_manifest_value(value).map_err(|issues| {
            let rendered = issues
                .into_iter()
                .map(|issue| format!("{}: {}", issue.field, issue.message))
                .collect::<Vec<_>>()
                .join("; ");
            anyhow::anyhow!("invalid manifest contract: {}", rendered)
        })?;
        return Ok(build_manifest_contract_view(
            reference,
            resolved_path,
            &manifest,
        ));
    }

    Ok(build_legacy_contract_view(
        reference,
        resolved_path,
        value,
        entry,
    ))
}

fn build_manifest_contract_view(
    reference: &str,
    resolved_path: &std::path::Path,
    manifest: &WorkflowManifestV2,
) -> WorkflowContractView {
    let inputs = manifest
        .params
        .properties
        .iter()
        .map(|(name, def)| WorkflowContractInputView {
            name: name.clone(),
            kind: serde_json::to_value(def.kind)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_else(|| format!("{:?}", def.kind).to_ascii_lowercase()),
            required: def.required,
            sensitive: def.sensitive,
            description: def.description.clone(),
            default: def.default.clone(),
            enum_values: def.enum_values.clone(),
        })
        .collect();
    let returns = manifest
        .help
        .as_ref()
        .map(|help| help.returns.clone())
        .unwrap_or_default();
    let output = WorkflowContractOutputView {
        schema: manifest.result.output_schema.clone(),
        returns,
        selector_step_id: manifest
            .result
            .output_selector
            .as_ref()
            .map(|selector| selector.step_id.clone()),
        selector_path: manifest
            .result
            .output_selector
            .as_ref()
            .and_then(|selector| selector.path.clone()),
        prefer_downloads: manifest.result.artifact_policy.prefer_downloads,
    };
    let side_effects = manifest
        .side_effects
        .iter()
        .map(|effect| WorkflowContractSideEffectView {
            class: effect.class.as_str().to_string(),
            idempotency: serde_json::to_value(&effect.idempotency)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_else(|| "safe_retry".to_string()),
            confirmation_required: effect.confirmation_required,
            scopes: effect.scopes.clone(),
        })
        .collect();
    let runtime = WorkflowContractRuntimeView {
        actor: serde_json::to_value(&manifest.runtime.actor)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "extension".to_string()),
        requires_existing_session: manifest.runtime.requires_existing_session,
        requires_cdp: manifest.runtime.requires_cdp,
        timeout_ms: manifest.runtime.timeout_ms,
        workflow_ref: manifest.runtime.workflow_ref.clone(),
        workflow_path: manifest.runtime.workflow_path.clone(),
    };

    WorkflowContractView {
        reference: reference.to_string(),
        path: resolved_path.display().to_string(),
        schema_version: WORKFLOW_CONTRACT_VERSION.to_string(),
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        summary: manifest.summary.clone(),
        description: manifest.description.clone(),
        version: Some(manifest.version.clone()),
        system: manifest.system.clone(),
        capability: manifest.capability.clone(),
        inputs,
        output,
        side_effects,
        runtime,
        step_count: manifest.steps.len(),
    }
}

fn build_legacy_contract_view(
    reference: &str,
    resolved_path: &std::path::Path,
    value: &Value,
    entry: Option<&NamedWorkflowEntry>,
) -> WorkflowContractView {
    let help = build_workflow_help_view(reference, resolved_path, value, entry);
    let inputs = help
        .parameters
        .iter()
        .map(|param| WorkflowContractInputView {
            name: param.name.clone(),
            kind: param.shape.clone().unwrap_or_else(|| "string".to_string()),
            required: param.required,
            sensitive: param.sensitive,
            description: Some(param.description.clone()),
            default: param
                .default_value
                .as_ref()
                .map(|value| Value::String(value.clone())),
            enum_values: Vec::new(),
        })
        .collect();
    let output = WorkflowContractOutputView {
        schema: infer_legacy_output_schema(value),
        returns: help.returns.clone().into_iter().collect(),
        selector_step_id: final_result_step_id(value),
        selector_path: None,
        prefer_downloads: false,
    };
    let step_count = value
        .pointer("/browser_automation/sequences/0/steps")
        .and_then(|value| value.as_array())
        .map(Vec::len)
        .unwrap_or(0);

    WorkflowContractView {
        reference: reference.to_string(),
        path: resolved_path.display().to_string(),
        schema_version: "legacy.workflow_json".to_string(),
        id: help.id,
        name: help.name,
        summary: Some(help.description.clone()),
        description: Some(help.description),
        version: help.version,
        system: help.system.unwrap_or_else(|| "unknown".to_string()),
        capability: help
            .workflow
            .map(|workflow| format!("{}.{}", "workflow", workflow))
            .unwrap_or_else(|| "workflow.legacy".to_string()),
        inputs,
        output,
        side_effects: Vec::new(),
        runtime: WorkflowContractRuntimeView {
            actor: "supervisor".to_string(),
            requires_existing_session: help.uses_current_tab,
            requires_cdp: false,
            timeout_ms: None,
            workflow_ref: None,
            workflow_path: Some(resolved_path.display().to_string()),
        },
        step_count,
    }
}

fn infer_legacy_output_schema(value: &Value) -> Option<Value> {
    let steps = value
        .pointer("/browser_automation/sequences/0/steps")
        .and_then(|value| value.as_array())?;
    let step = steps.iter().rev().find(|step| {
        matches!(
            step.get("type").and_then(|value| value.as_str()),
            Some("extract_structured_data")
                | Some("get_element_text")
                | Some("download_images")
                | Some("execute_javascript")
        )
    })?;
    match step.get("type").and_then(|value| value.as_str()) {
        Some("extract_structured_data") => Some(json!({
            "type": "array",
            "items": {
                "type": "object",
                "properties": extract_structured_field_schema(step)
            }
        })),
        Some("get_element_text") => Some(json!({ "type": "string" })),
        Some("download_images") => Some(json!({
            "type": "object",
            "properties": {
                "image_urls": { "type": "array", "items": { "type": "string", "format": "uri" } },
                "downloads": { "type": "array", "items": { "type": "object" } }
            }
        })),
        Some("execute_javascript") => Some(json!({ "type": "object" })),
        _ => None,
    }
}

fn extract_structured_field_schema(step: &Value) -> Value {
    let mut properties = serde_json::Map::new();
    if let Some(fields) = step.get("fields").and_then(|value| value.as_array()) {
        for field in fields {
            if let Some(name) = field.get("name").and_then(|value| value.as_str()) {
                properties.insert(name.to_string(), json!({ "type": "string" }));
            }
        }
    }
    Value::Object(properties)
}

fn final_result_step_id(value: &Value) -> Option<String> {
    value
        .pointer("/browser_automation/sequences/0/steps")
        .and_then(|value| value.as_array())
        .and_then(|steps| {
            steps.iter().rev().find_map(|step| {
                let result_type = matches!(
                    step.get("type").and_then(|value| value.as_str()),
                    Some("extract_structured_data")
                        | Some("get_element_text")
                        | Some("download_images")
                        | Some("execute_javascript")
                );
                result_type.then(|| {
                    step.get("id")
                        .and_then(|value| value.as_str())
                        .unwrap_or("step")
                        .to_string()
                })
            })
        })
}

fn render_workflow_contract_view(view: &WorkflowContractView) -> String {
    let mut lines = vec![render_primary_heading(
        &view.reference,
        format!(" — {}", view.name),
    )];
    lines.push(render_meta_line(&format!(
        "manifest: {} | id: {} | system: {} | capability: {} | version: {} | steps: {}",
        view.schema_version,
        view.id,
        view.system,
        view.capability,
        view.version.as_deref().unwrap_or("-"),
        view.step_count
    )));
    if let Some(summary) = view
        .summary
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(summary.to_string());
    }

    lines.push(String::new());
    lines.push(render_section_heading("Inputs"));
    if view.inputs.is_empty() {
        lines.push("  none".to_string());
    } else {
        let rows = view
            .inputs
            .iter()
            .map(|input| {
                vec![
                    styled_body_cell(&input.name),
                    styled_status_cell(if input.required {
                        "required"
                    } else {
                        "optional"
                    }),
                    styled_body_cell(&input.kind),
                    styled_body_cell(
                        input
                            .default
                            .as_ref()
                            .map(format_json_inline)
                            .as_deref()
                            .unwrap_or("-"),
                    ),
                    styled_body_cell(input.description.as_deref().unwrap_or("-")),
                ]
            })
            .collect();
        lines.push(render_cli_table(
            vec![
                styled_header_cell("name"),
                styled_header_cell("required"),
                styled_header_cell("type"),
                styled_header_cell("default"),
                styled_header_cell("description"),
            ],
            rows,
            2,
        ));
    }

    lines.push(String::new());
    lines.push(render_section_heading("Output"));
    if let Some(schema) = &view.output.schema {
        lines.push(format!("  schema: {}", format_json_inline(schema)));
    } else {
        lines.push("  schema: not declared".to_string());
    }
    for returns in &view.output.returns {
        lines.push(format!("  returns: {}", returns));
    }
    if let Some(step_id) = &view.output.selector_step_id {
        let suffix = view
            .output
            .selector_path
            .as_deref()
            .map(|path| format!(" path={}", path))
            .unwrap_or_default();
        lines.push(format!("  selector: step={}{}", step_id, suffix));
    }
    if view.output.prefer_downloads {
        lines.push("  artifacts: prefers downloads".to_string());
    }

    if !view.side_effects.is_empty() {
        lines.push(String::new());
        lines.push(render_section_heading("Side Effects"));
        let rows = view
            .side_effects
            .iter()
            .map(|effect| {
                vec![
                    styled_body_cell(&effect.class),
                    styled_body_cell(&effect.idempotency),
                    styled_body_cell(if effect.confirmation_required {
                        "yes"
                    } else {
                        "no"
                    }),
                    styled_body_cell(&effect.scopes.join(", ")),
                ]
            })
            .collect();
        lines.push(render_cli_table(
            vec![
                styled_header_cell("class"),
                styled_header_cell("idempotency"),
                styled_header_cell("confirm"),
                styled_header_cell("scopes"),
            ],
            rows,
            2,
        ));
    }

    lines.push(String::new());
    lines.push(render_section_heading("Runtime"));
    lines.push(format!(
        "  actor: {} | existing_session: {} | cdp: {} | timeout_ms: {}",
        view.runtime.actor,
        view.runtime.requires_existing_session,
        view.runtime.requires_cdp,
        view.runtime
            .timeout_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string())
    ));
    if let Some(workflow_ref) = &view.runtime.workflow_ref {
        lines.push(format!("  workflow_ref: {}", workflow_ref));
    }
    if let Some(workflow_path) = &view.runtime.workflow_path {
        lines.push(format!("  workflow_path: {}", workflow_path));
    }

    lines.join("\n")
}

fn format_json_inline(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
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
        .filter(|v| !v.is_empty())
        .or_else(|| {
            help.get("returns")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .filter(|value| !value.is_empty())
        });
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
        .or_else(|| {
            help.get("parameters")
                .and_then(|v| v.as_object())
                .map(|items| {
                    items
                        .iter()
                        .map(|(name, description)| WorkflowHelpParamView {
                            name: name.clone(),
                            required: false,
                            description: description
                                .as_str()
                                .map(|value| value.trim().to_string())
                                .filter(|value| !value.is_empty())
                                .unwrap_or_else(|| infer_param_description(name)),
                            shape: infer_param_shape(name),
                            default_value: None,
                            example: Some(infer_param_example(name)),
                            sensitive: infer_param_sensitive(name),
                        })
                        .collect::<Vec<_>>()
                })
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
    let manifest = if value.get("schema_version").and_then(|value| value.as_str())
        == Some(WORKFLOW_CONTRACT_VERSION)
    {
        validate_manifest_value(value).ok()
    } else {
        None
    };
    let required_variables = value
        .pointer("/browser_automation/sequences/0/required_variables")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut required_names = BTreeSet::new();
    let mut params = Vec::new();
    let mut indices = BTreeMap::new();

    if let Some(manifest) = manifest.as_ref() {
        for (name, def) in &manifest.params.properties {
            if def.required {
                required_names.insert(name.clone());
            }
        }
    }

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
        if !indices.contains_key(&merged.name) {
            indices.insert(merged.name.clone(), params.len());
            params.push(merged);
        }
    }

    if let Some(manifest) = manifest.as_ref() {
        for (name, def) in &manifest.params.properties {
            if let Some(idx) = indices.get(name).copied() {
                merge_manifest_help_param(&mut params[idx], def);
                params[idx].required = params[idx].required || required_names.contains(name);
            } else {
                let param = WorkflowHelpParamView {
                    name: name.clone(),
                    required: required_names.contains(name),
                    description: def
                        .description
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| infer_param_description(name)),
                    shape: Some(manifest_param_shape(name, def)),
                    default_value: def.default.as_ref().map(format_help_param_value),
                    example: Some(manifest_param_example(name, def)),
                    sensitive: def.sensitive,
                };
                indices.insert(name.clone(), params.len());
                params.push(param);
            }
        }
    }

    for variable in required_variables {
        let Some(name) = variable.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let name = name.trim().to_string();
        if name.is_empty() || indices.contains_key(&name) {
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
        indices.insert(name.clone(), params.len() - 1);
    }

    let mut placeholders = BTreeSet::new();
    collect_placeholders(value, None, &mut placeholders);
    for name in placeholders {
        if indices.contains_key(&name) {
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
        indices.insert(name.clone(), params.len() - 1);
    }

    params.sort_by(|left, right| {
        right
            .required
            .cmp(&left.required)
            .then_with(|| left.name.cmp(&right.name))
    });
    params
}

fn merge_manifest_help_param(param: &mut WorkflowHelpParamView, def: &ParamDefV2) {
    let inferred_description = infer_param_description(&param.name);
    if let Some(description) = def
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if param.description.trim().is_empty() || param.description == inferred_description {
            param.description = description.to_string();
        }
    }

    let inferred_shape = infer_param_shape(&param.name);
    let manifest_shape = manifest_param_shape(&param.name, def);
    if param.shape.is_none() || param.shape.as_deref() == inferred_shape.as_deref() {
        param.shape = Some(manifest_shape);
    }

    if param.default_value.is_none() {
        param.default_value = def.default.as_ref().map(format_help_param_value);
    }

    let inferred_example = infer_param_example(&param.name);
    if param.example.is_none() || param.example.as_deref() == Some(inferred_example.as_str()) {
        param.example = Some(manifest_param_example(&param.name, def));
    }

    param.required = param.required || def.required;
    param.sensitive = param.sensitive || def.sensitive;
}

fn manifest_param_shape(name: &str, def: &ParamDefV2) -> String {
    match def.kind {
        ParamKindV2::String => infer_param_shape(name).unwrap_or_else(|| "string".to_string()),
        ParamKindV2::Integer => "integer".to_string(),
        ParamKindV2::Number => "number".to_string(),
        ParamKindV2::Boolean => "boolean".to_string(),
        ParamKindV2::Object => "json object".to_string(),
        ParamKindV2::Array => "json array".to_string(),
    }
}

fn manifest_param_example(name: &str, def: &ParamDefV2) -> String {
    if let Some(value) = def.enum_values.first() {
        return format_help_param_value(value);
    }
    if let Some(value) = def.default.as_ref() {
        return format_help_param_value(value);
    }

    match def.kind {
        ParamKindV2::String => infer_param_example(name),
        ParamKindV2::Integer => "1".to_string(),
        ParamKindV2::Number => "1.0".to_string(),
        ParamKindV2::Boolean => "true".to_string(),
        ParamKindV2::Object => "{\"key\":\"value\"}".to_string(),
        ParamKindV2::Array if name.ends_with("_file_paths") || name.ends_with("_paths") => {
            "[\"/absolute/path/to/file.txt\"]".to_string()
        }
        ParamKindV2::Array => "[\"value\"]".to_string(),
    }
}

fn format_help_param_value(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format_json_inline(value))
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
        "model_slug" => "GPT-5.6 Sol".to_string(),
        "model_effort" => "Pro".to_string(),
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

#[cfg(test)]
fn ensure_run_parameters_present(
    workflow_ref: &str,
    resolved_path: &std::path::Path,
    params: &HashMap<String, String>,
) -> anyhow::Result<()> {
    let view = load_run_help_view(workflow_ref, resolved_path)?;
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

fn load_run_help_source(resolved_path: &std::path::Path) -> anyhow::Result<(PathBuf, Value)> {
    let (loaded_path, loaded_value) = load_workflow_value(&resolved_path.display().to_string())?;
    if loaded_value
        .get("schema_version")
        .and_then(|value| value.as_str())
        == Some(WORKFLOW_CONTRACT_VERSION)
    {
        return Ok((loaded_path, loaded_value));
    }

    if let Some(manifest_path) = find_manifest_path_for_runtime_workflow(&loaded_path) {
        if let Ok((path, value)) = read_contract_value_from_path(&manifest_path) {
            return Ok((path, value));
        }
    }

    Ok((loaded_path, loaded_value))
}

fn load_run_help_view(
    workflow_ref: &str,
    resolved_path: &std::path::Path,
) -> anyhow::Result<WorkflowHelpView> {
    let (help_path, help_value) = load_run_help_source(resolved_path)?;
    let entry = find_catalog_entry_for_path(&help_path).or_else(|| {
        (help_path != resolved_path)
            .then(|| find_catalog_entry_for_path(resolved_path))
            .flatten()
    });
    Ok(build_workflow_help_view(
        workflow_ref,
        &help_path,
        &help_value,
        entry.as_ref(),
    ))
}

fn render_run_system_discovery(reference: &str) -> anyhow::Result<Option<String>> {
    if reference.contains('/') || std::path::Path::new(reference).components().count() > 1 {
        return Ok(None);
    }

    let system = slugify(reference);
    if system.is_empty() {
        return Ok(None);
    }

    let entries = list_named_workflows_with_query(&WorkflowCatalogQuery {
        system_filter: Some(system.clone()),
        source_filter: None,
        include_all_sources: false,
    })?;
    if entries.is_empty() {
        return Ok(None);
    }

    let args = WorkflowListArgs {
        system: Some(system.clone()),
        workflow_name: None,
        source: None,
        all_sources: false,
        verbose: false,
        json: false,
    };
    let mut lines = vec![
        render_primary_heading("Workflow system: ", system.clone()),
        format!(
            "`{}` is a workflow system. Pick a runnable workflow below.",
            reference.trim()
        ),
        String::new(),
        render_workflow_catalog(&entries, &args),
        String::new(),
        render_meta_line(&format!(
            "Tip: run `rzn-browser list {} <workflow>` for inputs and examples.",
            system
        )),
    ];
    let example_entry = entries
        .iter()
        .find(|entry| entry.workflow == "send")
        .or_else(|| entries.iter().find(|entry| entry.workflow == "read"))
        .or_else(|| entries.first());
    if let Some(entry) = example_entry {
        lines.push(render_meta_line(&format!(
            "Example: rzn-browser run {} {}",
            entry.system, entry.workflow
        )));
    }

    Ok(Some(lines.join("\n")))
}

fn render_run_missing_parameter_guidance(
    workflow_ref: &str,
    resolved_path: &std::path::Path,
    params: &HashMap<String, String>,
) -> anyhow::Result<Option<String>> {
    let view = load_run_help_view(workflow_ref, resolved_path)?;
    let missing = view
        .parameters
        .iter()
        .filter(|param| param.required && !params.contains_key(&param.name))
        .map(|param| param.name.clone())
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(None);
    }

    Ok(Some(render_run_parameter_guidance(
        workflow_ref,
        &format!("Missing required parameters: {}.", missing.join(", ")),
        &view,
    )))
}

fn render_run_invalid_parameter_guidance(
    workflow_ref: &str,
    resolved_path: &std::path::Path,
    err: &anyhow::Error,
) -> anyhow::Result<Option<String>> {
    let err_text = err.to_string();
    let Some(details) = err_text
        .strip_prefix("Invalid workflow parameters:")
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    let view = load_run_help_view(workflow_ref, resolved_path)?;
    Ok(Some(render_run_parameter_guidance(
        workflow_ref,
        &format!("Workflow input is invalid: {}.", details),
        &view,
    )))
}

fn render_run_parameter_guidance(
    workflow_ref: &str,
    detail: &str,
    view: &WorkflowHelpView,
) -> String {
    [
        render_primary_heading("Workflow needs input: ", workflow_ref.to_string()),
        detail.to_string(),
        "Nothing ran. Use the workflow contract below to supply the right inputs.".to_string(),
        String::new(),
        render_workflow_help_view(view),
    ]
    .join("\n")
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
        strict: false,
    }
}

fn validate_workflow_strict_contract(
    reference: &str,
    resolved_path: &std::path::Path,
    value: &Value,
) -> WorkflowValidationReport {
    if value.get("schema_version").and_then(|value| value.as_str())
        == Some(WORKFLOW_CONTRACT_VERSION)
    {
        return validate_manifest_file(reference, resolved_path, value, true);
    }

    let mut report = validate_workflow_help_contract(reference, resolved_path, value);
    report.strict = true;

    let entry = find_catalog_entry_for_path(resolved_path);
    let Some(entry) = entry else {
        report.issues.push(WorkflowValidationIssue {
            level: WorkflowValidationLevel::Error,
            field: "catalog.route".to_string(),
            message: "Strict mode requires the workflow to be installed in the catalog."
                .to_string(),
            suggestion: Some(
                "Import/install the workflow, then declare it in a manifest capability."
                    .to_string(),
            ),
        });
        refresh_workflow_validation_counts(&mut report);
        return report;
    };

    let capabilities = list_capabilities_with_query(&CapabilityCatalogQuery {
        system_filter: Some(entry.system.clone()),
        source_filter: None,
    })
    .unwrap_or_default();
    let declared = capabilities.iter().any(|capability| {
        std::path::Path::new(&capability.workflow_path) == resolved_path
            || (capability.system == entry.system && capability.workflow == entry.workflow)
    });
    if !declared {
        report.issues.push(WorkflowValidationIssue {
            level: WorkflowValidationLevel::Error,
            field: "manifest.capabilities".to_string(),
            message: format!(
                "Workflow `{}` is not reachable through a manifest capability for explicit system `{}`.",
                entry.workflow, entry.system
            ),
            suggestion: Some("Add a manifest capability with `system_id`, `capability_id`, and `route.workflow`.".to_string()),
        });
    }

    if value.pointer("/output").is_none()
        && value.pointer("/result").is_none()
        && value.pointer("/contract/output").is_none()
        && value.pointer("/outputs").is_none()
    {
        report.issues.push(WorkflowValidationIssue {
            level: WorkflowValidationLevel::Error,
            field: "output".to_string(),
            message: "Strict mode requires an explicit output/result contract; final output must not be guessed from the last payload.".to_string(),
            suggestion: Some("Declare output selectors/results in the workflow contract before launch routing uses it.".to_string()),
        });
    }

    refresh_workflow_validation_counts(&mut report);
    report
}

fn validate_manifest_file(
    reference: &str,
    resolved_path: &std::path::Path,
    value: &Value,
    strict: bool,
) -> WorkflowValidationReport {
    let mut issues = Vec::new();
    let manifest = match validate_manifest_value(value) {
        Ok(manifest) => Some(manifest),
        Err(contract_issues) => {
            for issue in contract_issues {
                issues.push(WorkflowValidationIssue {
                    level: WorkflowValidationLevel::Error,
                    field: issue.field,
                    message: issue.message,
                    suggestion: None,
                });
            }
            None
        }
    };

    if let Some(manifest) = manifest.as_ref() {
        validate_manifest_runtime_link(resolved_path, manifest, &mut issues);
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
        strict,
    }
}

fn validate_manifest_runtime_link(
    manifest_path: &std::path::Path,
    manifest: &WorkflowManifestV2,
    issues: &mut Vec<WorkflowValidationIssue>,
) {
    let runtime_path = manifest_manifest_runtime_workflow_path(manifest_path, manifest);
    let Some(runtime_path) = runtime_path else {
        if manifest.steps.is_empty() {
            issues.push(WorkflowValidationIssue {
                level: WorkflowValidationLevel::Error,
                field: "runtime.workflow_ref".to_string(),
                message: "Manifest with empty steps[] must declare runtime.workflow_ref or runtime.workflow_path.".to_string(),
                suggestion: Some("Point the manifest at the runtime workflow until steps[] becomes authoritative.".to_string()),
            });
        }
        return;
    };

    if !runtime_path.is_file() {
        issues.push(WorkflowValidationIssue {
            level: WorkflowValidationLevel::Error,
            field: "runtime.workflow_ref".to_string(),
            message: format!("Runtime workflow does not exist at {}.", runtime_path.display()),
            suggestion: Some("Fix runtime.workflow_ref/runtime.workflow_path or install the referenced workflow.".to_string()),
        });
        return;
    }

    let Some(selector) = manifest.result.output_selector.as_ref() else {
        return;
    };
    let Ok(content) = fs::read_to_string(&runtime_path) else {
        return;
    };
    let Ok(runtime_value) = serde_json::from_str::<Value>(&content) else {
        return;
    };
    if !legacy_runtime_has_step(&runtime_value, &selector.step_id) {
        issues.push(WorkflowValidationIssue {
            level: WorkflowValidationLevel::Error,
            field: "result.output_selector.step_id".to_string(),
            message: format!(
                "Output selector references step `{}` which does not exist in {}.",
                selector.step_id,
                runtime_path.display()
            ),
            suggestion: Some(
                "Update result.output_selector.step_id or the runtime workflow step id."
                    .to_string(),
            ),
        });
    }
}

fn manifest_manifest_runtime_workflow_path(
    manifest_path: &std::path::Path,
    manifest: &WorkflowManifestV2,
) -> Option<PathBuf> {
    let workflows_root = workflow_root_for_manifest_path(manifest_path)?;
    manifest
        .runtime
        .workflow_ref
        .as_deref()
        .and_then(|workflow_ref| resolve_manifest_workflow_ref(&workflows_root, workflow_ref))
        .or_else(|| {
            manifest
                .runtime
                .workflow_path
                .as_deref()
                .map(|workflow_path| {
                    let path = PathBuf::from(workflow_path);
                    if path.is_absolute() {
                        path
                    } else {
                        workflows_root.join(path)
                    }
                })
        })
}

fn workflow_root_for_manifest_path(manifest_path: &std::path::Path) -> Option<PathBuf> {
    for ancestor in manifest_path.ancestors() {
        if ancestor.file_name().and_then(|value| value.to_str()) == Some("workflows") {
            return Some(ancestor.to_path_buf());
        }
    }
    manifest_path.parent().map(Path::to_path_buf)
}

fn resolve_manifest_workflow_ref(root: &std::path::Path, workflow_ref: &str) -> Option<PathBuf> {
    let normalized = workflow_ref
        .trim()
        .replace('\\', "/")
        .trim_matches('/')
        .to_ascii_lowercase();
    let parts = normalized.split('/').collect::<Vec<_>>();
    if parts.len() == 2 {
        let system = slugify_manifest_ref_part(parts[0]);
        let workflow = slugify_manifest_ref_part(parts[1]);
        if !system.is_empty() && !workflow.is_empty() {
            let candidates = [
                root.join(&system).join(format!("{workflow}.json")),
                root.join(&system).join(format!("{system}-{workflow}.json")),
                root.join(&system).join(format!("{system}_{workflow}.json")),
            ];
            for candidate in candidates {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
            return Some(root.join(&system).join(format!("{workflow}.json")));
        }
    }

    let path = PathBuf::from(workflow_ref);
    Some(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

fn slugify_manifest_ref_part(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn legacy_runtime_has_step(workflow: &Value, step_id: &str) -> bool {
    workflow
        .pointer("/browser_automation/sequences")
        .and_then(|value| value.as_array())
        .map(|sequences| {
            sequences.iter().any(|sequence| {
                sequence
                    .get("steps")
                    .and_then(|value| value.as_array())
                    .map(|steps| {
                        steps.iter().any(|step| {
                            step.get("id").and_then(|value| value.as_str()) == Some(step_id)
                        })
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn refresh_workflow_validation_counts(report: &mut WorkflowValidationReport) {
    report.error_count = report
        .issues
        .iter()
        .filter(|issue| matches!(issue.level, WorkflowValidationLevel::Error))
        .count();
    report.warning_count = report
        .issues
        .iter()
        .filter(|issue| matches!(issue.level, WorkflowValidationLevel::Warning))
        .count();
    report.info_count = report
        .issues
        .iter()
        .filter(|issue| matches!(issue.level, WorkflowValidationLevel::Info))
        .count();
    report.ok = report.error_count == 0;
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
    if report.strict {
        lines.push(render_meta_line("mode: strict manifest contract"));
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

async fn handle_workflow_inspect(
    args: WorkflowInspectArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let workflow_ref = workflow_ref_value(&args.workflow_ref)?;
    let (resolved_path, value) = load_workflow_contract_value(&workflow_ref)?;
    let entry = find_catalog_entry_for_path(&resolved_path);
    let view = build_workflow_contract_view(&workflow_ref, &resolved_path, &value, entry.as_ref())?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&view)?);
    } else {
        println!("{}", render_workflow_contract_view(&view));
    }
    Ok(())
}

async fn handle_workflow_contract(
    args: WorkflowContractArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    handle_workflow_inspect(args).await
}

async fn handle_workflow_validate(
    args: WorkflowValidateArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let workflow_ref = workflow_ref_value(&args.workflow_ref)?;
    let (resolved_path, mut value) = load_workflow_value(&workflow_ref)?;
    let entry = find_catalog_entry_for_path(&resolved_path);
    let is_manifest = value.get("schema_version").and_then(|value| value.as_str())
        == Some(WORKFLOW_CONTRACT_VERSION);

    if is_manifest && args.write_help {
        return Err(Box::<dyn std::error::Error>::from(anyhow::anyhow!(
            "--write-help is only supported for legacy workflow JSON; manifest help lives in the manifest contract"
        )));
    }

    if args.write_help {
        value = write_workflow_help_block(&workflow_ref, &resolved_path, &value, entry.as_ref())?;
    }

    let mut report = if is_manifest {
        validate_manifest_file(&workflow_ref, &resolved_path, &value, args.strict)
    } else if args.strict {
        validate_workflow_strict_contract(&workflow_ref, &resolved_path, &value)
    } else {
        validate_workflow_help_contract(&workflow_ref, &resolved_path, &value)
    };
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
    if !args.allow_direct_workflow {
        return Err(Box::<dyn std::error::Error>::from(anyhow::anyhow!(
            "`rzn-browser workflow run` is a direct workflow escape hatch. Re-run with --allow-direct-workflow, or use `rzn-browser workflow capability resolve --system <system> <capability_id>` and route through a manifest capability."
        )));
    }
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

async fn handle_capability_commands(
    cmd: CapabilityCommands,
) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        CapabilityCommands::List(args) => handle_capability_list(args).await,
        CapabilityCommands::Resolve(args) => handle_capability_resolve(args).await,
    }
}

async fn handle_capability_list(
    args: CapabilityListArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let entries = list_capabilities_with_query(&CapabilityCatalogQuery {
        system_filter: args.system.clone(),
        source_filter: args.source.map(|source| source.as_str().to_string()),
    })?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        println!(
            "{}",
            render_capability_catalog(&entries, args.system.as_deref())
        );
    }
    Ok(())
}

async fn handle_capability_resolve(
    args: CapabilityResolveArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let entry = resolve_capability_route(&args.system, &args.capability_id)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&entry)?);
    } else {
        println!("{} -> {}", entry.capability_id, entry.route);
        println!("workflow: {}", entry.workflow_path);
        println!("manifest: {}", entry.manifest_path);
    }
    Ok(())
}

async fn handle_catalog_validate(
    args: CatalogValidateArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let report = validate_catalog_manifests(args.strict)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", render_catalog_validation_report(&report));
    }
    if report.ok {
        return Ok(());
    }
    Err(Box::<dyn std::error::Error>::from(anyhow::anyhow!(
        "catalog validation failed with {} error(s)",
        report.error_count
    )))
}

fn render_capability_catalog(entries: &[CapabilityCatalogEntry], system: Option<&str>) -> String {
    if entries.is_empty() {
        return match system {
            Some(system) => format!(
                "No manifest-declared capabilities found for system '{}'.",
                system
            ),
            None => "No manifest-declared capabilities found.".to_string(),
        };
    }

    let mut by_system: BTreeMap<&str, Vec<&CapabilityCatalogEntry>> = BTreeMap::new();
    for entry in entries {
        by_system.entry(&entry.system).or_default().push(entry);
    }

    let mut lines = vec![render_primary_heading(
        "Manifest capabilities: ",
        format!(
            "{} capabilities across {} systems",
            entries.len(),
            by_system.len()
        ),
    )];
    for (system, capabilities) in by_system {
        lines.push(String::new());
        lines.push(render_section_heading(&format!(
            "{} ({} capabilities)",
            system,
            capabilities.len()
        )));
        let rows = capabilities
            .iter()
            .map(|entry| {
                let effects = if entry.effects.is_empty() {
                    "-".to_string()
                } else {
                    entry.effects.join(", ")
                };
                vec![
                    styled_body_cell(&entry.capability_id),
                    styled_body_cell(&entry.workflow),
                    styled_body_cell(&entry.source),
                    styled_body_cell(entry.description.as_deref().unwrap_or("-")),
                    styled_body_cell(&effects),
                ]
            })
            .collect();
        lines.push(render_cli_table(
            vec![
                styled_header_cell("capability"),
                styled_header_cell("workflow"),
                styled_header_cell("source"),
                styled_header_cell("description"),
                styled_header_cell("effects"),
            ],
            rows,
            2,
        ));
    }
    lines.join("\n")
}

fn render_catalog_validation_report(report: &CatalogValidationReport) -> String {
    let mut lines = vec![render_primary_heading(
        "Catalog validation: ",
        if report.ok { "ok" } else { "failed" }.to_string(),
    )];
    lines.push(render_meta_line(&format!(
        "mode: {}",
        if report.strict { "strict" } else { "compat" }
    )));
    lines.push(render_meta_line(&format!(
        "manifests: {}, capabilities: {}",
        report.manifest_count, report.capability_count
    )));
    lines.push(render_meta_line(&format!(
        "issues: {} error(s), {} warning(s)",
        report.error_count, report.warning_count
    )));
    if report.issues.is_empty() {
        lines.push("No issues found.".to_string());
        return lines.join("\n");
    }
    let rows = report
        .issues
        .iter()
        .map(|issue| {
            let level = match issue.level {
                workflow_catalog::CatalogValidationLevel::Error => "ERROR",
                workflow_catalog::CatalogValidationLevel::Warning => "WARN",
                workflow_catalog::CatalogValidationLevel::Info => "INFO",
            };
            vec![
                styled_status_cell(level),
                styled_body_cell(&issue.source),
                styled_body_cell(&issue.field),
                styled_body_cell(&issue.message),
                styled_body_cell(&issue.path),
            ]
        })
        .collect();
    lines.push(String::new());
    lines.push(render_section_heading("Issues"));
    lines.push(render_cli_table(
        vec![
            styled_header_cell("level"),
            styled_header_cell("source"),
            styled_header_cell("field"),
            styled_header_cell("message"),
            styled_header_cell("path"),
        ],
        rows,
        2,
    ));
    lines.join("\n")
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

async fn handle_workflow_new(
    _args: WorkflowNewArgs,
    config: PlanConfig,
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
                    .and_then(|a| a.first())
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
    root_domains: &[String],
    required_params: &[String],
    optional_params: &[String],
    _extract_fields: &[String],
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
            &uuid::Uuid::new_v4().to_string()[..8]
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
            &uuid::Uuid::new_v4().to_string()[..8]
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
        let extract_items = |resp: &Value| -> Vec<Value> {
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
            .and_then(|a| a.first())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{}_{}", prefix, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn parse_run_defaults_to_supervisor_with_split_workflow_reference() {
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
    fn parse_run_rejects_removed_backend_selection() {
        let err = Cli::try_parse_from(["rzn-browser", "run", "google/search", "--via", "desktop"])
            .expect_err("backend selection should be removed");

        assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn tab_ref_input_supervisor_call_accepts_tab_target_flags() {
        let cli = Cli::try_parse_from([
            "rzn-browser",
            "supervisor",
            "call",
            "browser.execute_step",
            "--params",
            r#"{"step":{"type":"click"}}"#,
            "--tab-ref",
            "rzn://browser/edge-instance/tab/123",
            "--browser",
            "edge",
        ])
        .expect("parse supervisor call tab target flags");

        match cli.command {
            Commands::Supervisor(SupervisorCommands::Call(args)) => {
                assert_eq!(
                    args.tab_ref.as_deref(),
                    Some("rzn://browser/edge-instance/tab/123")
                );
                assert_eq!(args.target.browser.as_deref(), Some("edge"));
            }
            other => panic!("expected supervisor call command, got {:?}", other),
        }
    }

    #[test]
    fn tab_ref_input_supervisor_call_tab_target_flags_merge_into_params() {
        let args = SupervisorCallArgs {
            common: SupervisorCommonArgs {
                app_base: None,
                json: false,
            },
            target: BrowserTargetArgs {
                bridge_id: Some("chrome-bridge".to_string()),
                ..BrowserTargetArgs::default()
            },
            method: "browser.execute_step".to_string(),
            params: "{}".to_string(),
            tab_ref: Some("rzn://browser/chrome-instance/tab/44".to_string()),
            tab: Some(44),
        };
        let mut params = json!({});
        apply_supervisor_call_target_flags(&mut params, &args).expect("merge target flags");

        assert_eq!(
            params.get("tab_ref").and_then(Value::as_str),
            Some("rzn://browser/chrome-instance/tab/44")
        );
        assert_eq!(
            params.get("current_tab_id").and_then(Value::as_u64),
            Some(44)
        );
        assert_eq!(
            params
                .pointer("/browser_target/bridge_id")
                .and_then(Value::as_str),
            Some("chrome-bridge")
        );
    }

    #[test]
    fn browser_targets_cli_parses_json_flag() {
        let cli = Cli::try_parse_from(["rzn-browser", "browser", "targets", "--json"])
            .expect("parse browser targets command");

        match cli.command {
            Commands::Browser(BrowserCommands::Targets(args)) => {
                assert!(args.common.json);
            }
            other => panic!("expected browser targets command, got {:?}", other),
        }
    }

    #[test]
    fn browser_targets_runtime_status_compat_maps_health_bridges() {
        let result = browser_targets_from_runtime_status(
            json!({
                "native_host_bridge": {
                    "connected": true,
                    "health": {
                        "bridges": {
                            "edge-bridge": {
                                "connected": true,
                                "current_bridge_id": "edge-bridge",
                                "current_bridge_metadata": {
                                    "browser_instance_id": "edge-instance",
                                    "extension_target": "edge",
                                    "caller_extension_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                                    "caller_origin": "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/"
                                },
                                "last_successful_ping_at_ms": 1234,
                                "last_successful_ping_latency_ms": 9
                            },
                            "stale-bridge": {
                                "connected": false,
                                "current_bridge_id": "stale-bridge"
                            }
                        }
                    }
                }
            }),
            None,
        );

        assert_eq!(
            result.get("compat_source").and_then(Value::as_str),
            Some("runtime.status")
        );
        assert_eq!(result.get("target_count").and_then(Value::as_u64), Some(1));
        let target = result
            .get("targets")
            .and_then(Value::as_array)
            .and_then(|targets| targets.first())
            .expect("compat target");
        assert_eq!(
            target.get("bridge_id").and_then(Value::as_str),
            Some("edge-bridge")
        );
        assert_eq!(
            target.get("browser_instance_id").and_then(Value::as_str),
            Some("edge-instance")
        );
        assert_eq!(target.get("browser").and_then(Value::as_str), Some("edge"));
        assert_eq!(
            target.get("last_ping_status").and_then(Value::as_str),
            Some("ok")
        );
    }

    #[test]
    fn browser_targets_runtime_status_compat_uses_readiness_probe_identity() {
        let readiness = json!({
            "native_host_bridge": {
                "probe": {
                    "response": {
                        "resolved_browser_target": {
                            "bridge_id": "native-host-1"
                        },
                        "result": {
                            "success": true,
                            "result": {
                                "browser_instance_id": "chromium-instance",
                                "extension_target": "chromium",
                                "extension_target_hint": "chromium-mv3",
                                "extension_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                                "extension_origin": "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/"
                            }
                        }
                    }
                }
            }
        });
        let result = browser_targets_from_runtime_status(
            json!({
                "native_host_bridge": {
                    "connected": true,
                    "health": {
                        "bridges": {
                            "native-host-1": {
                                "connected": true,
                                "current_bridge_id": "native-host-1",
                                "last_successful_ping_at_ms": 1234
                            }
                        }
                    }
                }
            }),
            Some(&readiness),
        );

        let target = result
            .get("targets")
            .and_then(Value::as_array)
            .and_then(|targets| targets.first())
            .expect("compat target");
        assert_eq!(
            target.get("bridge_id").and_then(Value::as_str),
            Some("native-host-1")
        );
        assert_eq!(
            target.get("browser_instance_id").and_then(Value::as_str),
            Some("chromium-instance")
        );
        assert_eq!(
            target.get("browser").and_then(Value::as_str),
            Some("chromium")
        );
        assert_eq!(
            target.get("extension_id").and_then(Value::as_str),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(
            target.get("caller_origin").and_then(Value::as_str),
            Some("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/")
        );
    }

    #[test]
    fn supervisor_unknown_method_error_matches_requested_method() {
        let err = anyhow::anyhow!(
            "Supervisor error: {{\"code\":-32000,\"message\":\"Unknown supervisor method: browser.targets\"}}"
        );

        assert!(supervisor_unknown_method_error(&err, "browser.targets"));
        assert!(!supervisor_unknown_method_error(&err, "runtime.status"));
    }

    #[test]
    fn browser_set_cli_accepts_browser_shorthand() {
        let cli = Cli::try_parse_from(["rzn-browser", "browser", "set", "chromium"])
            .expect("parse browser set shorthand");

        match cli.command {
            Commands::Browser(BrowserCommands::Set(args)) => {
                assert_eq!(args.browser_name.as_deref(), Some("chromium"));
                let target = browser_set_target_value(&args).expect("target value");
                assert_eq!(
                    target.get("browser").and_then(Value::as_str),
                    Some("chromium")
                );
            }
            other => panic!("expected browser set command, got {:?}", other),
        }
    }

    #[test]
    fn browser_default_config_roundtrips_target() {
        let app_base =
            std::env::temp_dir().join(format!("rzn-browser-default-test-{}", uuid::Uuid::new_v4()));
        let config = supervisor::SupervisorConfig {
            app_base: Some(app_base.clone()),
        };
        let target = json!({ "browser": "edge" });

        write_browser_default_target(&config, target.clone()).expect("write default");
        assert_eq!(
            read_browser_default_target(&config).expect("read default"),
            Some(target.clone())
        );
        assert_eq!(
            browser_target_routing_value_with_default(&BrowserTargetArgs::default(), &config)
                .expect("target with default"),
            Some(json!({
                "preferred": target,
                "fallback": "single_connected"
            }))
        );
        clear_browser_default_target(&config).expect("clear default");
        assert_eq!(
            read_browser_default_target(&config).expect("read cleared default"),
            None
        );
        let _ = fs::remove_dir_all(app_base);
    }

    #[test]
    fn explicit_browser_target_overrides_saved_default() {
        let app_base = std::env::temp_dir().join(format!(
            "rzn-browser-default-override-test-{}",
            uuid::Uuid::new_v4()
        ));
        let config = supervisor::SupervisorConfig {
            app_base: Some(app_base.clone()),
        };
        write_browser_default_target(&config, json!({ "browser": "chromium" }))
            .expect("write default");
        let explicit = BrowserTargetArgs {
            browser: Some("edge".to_string()),
            ..BrowserTargetArgs::default()
        };

        assert_eq!(
            browser_target_routing_value_with_default(&explicit, &config)
                .expect("explicit target wins"),
            Some(json!({ "browser": "edge" }))
        );
        let _ = fs::remove_dir_all(app_base);
    }

    #[test]
    fn browser_target_flags_run_accepts_browser_instance() {
        let cli = Cli::try_parse_from([
            "rzn-browser",
            "run",
            "google/search",
            "--browser-instance",
            "edge-instance",
        ])
        .expect("parse run target flags");

        match cli.command {
            Commands::Run(args) => {
                assert_eq!(
                    args.target.browser_instance_id.as_deref(),
                    Some("edge-instance")
                );
                let target =
                    browser_target_routing_value(&args.target).expect("browser target value");
                assert_eq!(
                    target
                        .as_ref()
                        .and_then(|value| value.get("browser_instance_id"))
                        .and_then(Value::as_str),
                    Some("edge-instance")
                );
            }
            other => panic!("expected run command, got {:?}", other),
        }
    }

    #[test]
    fn browser_target_flags_reject_conflicting_selectors() {
        let target = BrowserTargetArgs {
            browser: Some("edge".to_string()),
            bridge_id: Some("edge-bridge".to_string()),
            ..BrowserTargetArgs::default()
        };
        let err = browser_target_routing_value(&target).expect_err("conflict is rejected");
        assert!(err.to_string().contains("conflicting browser target flags"));
        assert!(err.to_string().contains("--bridge"));
    }

    #[test]
    fn one_browser_compat_release_notes_scope_multi_bridge_change() {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dist/release/release-notes.md");
        let notes = fs::read_to_string(path).expect("release notes readable");

        assert!(notes.contains("Existing one-browser workflows continue to run"));
        assert!(notes.contains("When multiple browser bridges are connected"));
        assert!(notes.contains("--browser-instance"));
    }

    #[test]
    fn native_host_doctor_checks_manifest_origin_and_host_path() {
        let manifest = NativeMessagingHostManifest {
            name: RZN_NATIVE_HOST_NAME.to_string(),
            description: "test".to_string(),
            path: "/definitely/missing/rzn-native-host".to_string(),
            manifest_type: "stdio".to_string(),
            allowed_origins: vec![
                "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/".to_string()
            ],
        };
        let checks = native_host_doctor_manifest_checks(
            &manifest,
            "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/",
        );

        assert!(checks
            .iter()
            .any(|check| check.name == "host_name" && check.status == DoctorCheckStatus::Pass));
        assert!(checks.iter().any(|check| {
            check.name == "allowed_origin" && check.status == DoctorCheckStatus::Fail
        }));
        assert!(checks.iter().any(|check| {
            check.name == "host_path_exists" && check.status == DoctorCheckStatus::Fail
        }));
    }

    fn dev_manifest_key() -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../extension/src/manifest.base.json");
        let value: Value =
            serde_json::from_str(&fs::read_to_string(path).expect("read manifest base"))
                .expect("parse manifest base");
        value
            .get("key")
            .and_then(Value::as_str)
            .expect("manifest base key")
            .to_string()
    }

    fn write_doctor_bundle(dir: &Path, target: &str) {
        fs::create_dir_all(dir.join("icons")).expect("create icons dir");
        for size in ["16", "32", "48", "128"] {
            fs::write(dir.join(format!("icons/brain-{size}.png")), b"icon").expect("write icon");
        }
        fs::write(dir.join("background.js"), b"// background").expect("write background");
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec_pretty(&json!({
                "manifest_version": 3,
                "key": dev_manifest_key(),
                "icons": {
                    "16": "icons/brain-16.png",
                    "32": "icons/brain-32.png",
                    "48": "icons/brain-48.png",
                    "128": "icons/brain-128.png"
                },
                "permissions": ["nativeMessaging"],
                "background": { "service_worker": "background.js" }
            }))
            .expect("serialize manifest"),
        )
        .expect("write manifest");
        fs::write(
            dir.join("rzn-build.json"),
            serde_json::to_vec_pretty(&json!({ "extension_target": target }))
                .expect("serialize build metadata"),
        )
        .expect("write build metadata");
    }

    #[test]
    fn native_host_doctor_extension_bundle_checks_cover_load_unpacked_basics() {
        let dir = temp_dir("doctor_bundle_ok");
        write_doctor_bundle(&dir, "chrome");

        let checks = native_host_doctor_extension_bundle_checks(
            &dir,
            BrowserKind::Chrome,
            RZN_DEV_EXTENSION_ORIGIN,
        );

        for name in [
            "extension_bundle_directory",
            "extension_bundle_manifest",
            "extension_bundle_manifest_key",
            "extension_bundle_native_messaging_permission",
            "extension_bundle_icons",
            "extension_bundle_background_worker",
            "extension_bundle_rzn_build_target",
        ] {
            assert!(
                checks
                    .iter()
                    .any(|check| check.name == name && check.status == DoctorCheckStatus::Pass),
                "expected passing check {name}, got {checks:?}"
            );
        }
    }

    #[test]
    fn native_host_doctor_extension_bundle_checks_reject_stale_target() {
        let dir = temp_dir("doctor_bundle_stale_target");
        write_doctor_bundle(&dir, "chrome");

        let checks = native_host_doctor_extension_bundle_checks(
            &dir,
            BrowserKind::Edge,
            RZN_DEV_EXTENSION_ORIGIN,
        );

        assert!(checks.iter().any(|check| {
            check.name == "extension_bundle_rzn_build_target"
                && check.status == DoctorCheckStatus::Fail
                && check.message.contains("expected edge")
        }));
    }

    #[test]
    fn native_host_doctor_output_omits_token_values() {
        let output = NativeHostDoctorOutput {
            success: true,
            browser: "edge".to_string(),
            extension_origin: "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/".to_string(),
            manifest_path: None,
            load_unpacked_path: PathBuf::from("/tmp/rzn-extension"),
            checks: vec![doctor_check(
                "supervisor_token",
                DoctorCheckStatus::Pass,
                "supervisor token file exists; value is intentionally not printed",
            )],
        };
        let rendered = serde_json::to_string(&output).expect("doctor output serializes");

        assert!(rendered.contains("supervisor_token"));
        assert!(!rendered.contains("RZN_SUPERVISOR_TOKEN="));
    }

    #[test]
    fn native_host_doctor_bridge_checks_require_expected_origin_and_id() {
        let checks = native_host_doctor_bridge_checks(
            &json!({
                "targets": [{
                    "browser": "edge",
                    "caller_origin": "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
                    "extension_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "browser_instance_id": "edge-instance"
                }]
            }),
            BrowserKind::Edge,
            "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/",
        );

        assert!(checks.iter().any(|check| {
            check.name == "connected_bridge_browser_matches"
                && check.status == DoctorCheckStatus::Pass
        }));
        assert!(checks.iter().any(|check| {
            check.name == "connected_bridge_caller_origin_matches"
                && check.status == DoctorCheckStatus::Fail
        }));
        assert!(checks.iter().any(|check| {
            check.name == "connected_bridge_extension_id_matches"
                && check.status == DoctorCheckStatus::Fail
        }));
        assert!(checks.iter().any(|check| {
            check.name == "connected_bridge_browser_instance_id_present"
                && check.status == DoctorCheckStatus::Pass
        }));
    }

    #[test]
    fn native_host_doctor_cli_parses_browser_origin_and_json() {
        let cli = Cli::try_parse_from([
            "rzn-browser",
            "native-host",
            "doctor",
            "--browser",
            "edge",
            "--extension-origin",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--json",
        ])
        .expect("parse native-host doctor");

        match cli.command {
            Commands::NativeHost(NativeHostCommands::Doctor(args)) => {
                assert_eq!(args.browser, "edge");
                assert_eq!(
                    args.extension_origin.as_deref(),
                    Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                );
                assert!(args.json);
            }
            other => panic!("expected native-host doctor command, got {:?}", other),
        }
    }

    #[test]
    fn native_host_doctor_cli_defaults_extension_origin() {
        let cli =
            Cli::try_parse_from(["rzn-browser", "native-host", "doctor", "--browser", "edge"])
                .expect("parse native-host doctor without explicit origin");

        match cli.command {
            Commands::NativeHost(NativeHostCommands::Doctor(args)) => {
                assert_eq!(args.browser, "edge");
                assert!(args.extension_origin.is_none());
            }
            other => panic!("expected native-host doctor command, got {:?}", other),
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
            "0.1.0",
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
                assert_eq!(args.app_version, "0.1.0");
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
    fn parse_native_host_install_accepts_comma_and_repeated_values() {
        let cli = Cli::try_parse_from([
            "rzn-browser",
            "native-host",
            "install",
            "--browser",
            "chrome,edge",
            "--browser",
            "chromium",
            "--extension-id",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--extension-origin",
            "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/",
            "--native-host-path",
            "/tmp/rzn-native-host",
            "--json",
        ])
        .expect("parse native-host install");

        match cli.command {
            Commands::NativeHost(NativeHostCommands::Install(args)) => {
                assert_eq!(args.browsers, vec!["chrome", "edge", "chromium"]);
                assert_eq!(args.extension_ids, vec!["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]);
                assert_eq!(
                    args.extension_origins,
                    vec!["chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/"]
                );
                assert_eq!(
                    args.native_host_path,
                    Some(PathBuf::from("/tmp/rzn-native-host"))
                );
                assert!(args.json);
            }
            other => panic!("expected native-host install command, got {:?}", other),
        }
    }

    #[test]
    fn parse_native_host_install_allows_default_primary_browsers() {
        let cli = Cli::try_parse_from(["rzn-browser", "native-host", "install"])
            .expect("parse native-host install without browser");

        match cli.command {
            Commands::NativeHost(NativeHostCommands::Install(args)) => {
                assert!(args.browsers.is_empty());
                let browsers = parse_native_host_browsers(&args.browsers, true)
                    .expect("default primary browsers");
                assert_eq!(
                    browsers,
                    vec![
                        BrowserKind::Chrome,
                        BrowserKind::Chromium,
                        BrowserKind::Edge
                    ]
                );
            }
            other => panic!("expected native-host install command, got {:?}", other),
        }
    }

    #[test]
    fn native_host_browser_parser_rejects_unknown_slug_with_valid_slugs() {
        let err =
            parse_native_host_browsers(&["safari".to_string()], false).expect_err("reject safari");
        let message = err.to_string();
        assert!(message.contains("INVALID_BROWSER_TARGET"));
        assert!(message.contains("chrome"));
        assert!(message.contains("chromium"));
        assert!(message.contains("edge"));
    }

    #[test]
    fn native_host_allowed_origins_rejects_invalid_before_install() {
        let err = native_host_allowed_origins(&["*".to_string()], &[])
            .expect_err("reject wildcard extension id");
        let message = err.to_string();
        assert!(message.contains("INVALID_EXTENSION_ORIGIN"));
        assert!(message.contains("wildcard"));
    }

    #[test]
    fn native_host_allowed_origins_defaults_to_dev_extension_origin() {
        let origins = native_host_allowed_origins(&[], &[]).expect("default dev origin");
        assert_eq!(origins, vec![RZN_DEV_EXTENSION_ORIGIN.to_string()]);
        assert_eq!(RZN_DEV_EXTENSION_ID, "bogjdnehdficgkhklinmnbgiiofbamji");
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
            app_version: "0.1.0".to_string(),
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
        assert_eq!(value["app_version"], "0.1.0");
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
            app_version: "0.1.0".to_string(),
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
                assert!(!args.strict);
                assert!(args.json);
            }
            other => panic!("expected workflow validate command, got {:?}", other),
        }
    }

    #[test]
    fn parse_workflow_validate_strict() {
        let cli = Cli::try_parse_from([
            "rzn-browser",
            "workflow",
            "validate",
            "chatgpt",
            "read",
            "--strict",
        ])
        .expect("parse strict workflow validate");

        match cli.command {
            Commands::Workflow(WorkflowCommands::Validate(args)) => {
                assert_eq!(args.workflow_ref.workflow_or_system, "chatgpt");
                assert_eq!(args.workflow_ref.workflow_name.as_deref(), Some("read"));
                assert!(args.strict);
            }
            other => panic!("expected workflow validate command, got {:?}", other),
        }
    }

    #[test]
    fn parse_workflow_inspect_json() {
        let cli = Cli::try_parse_from([
            "rzn-browser",
            "workflow",
            "inspect",
            "google",
            "search",
            "--json",
        ])
        .expect("parse workflow inspect");

        match cli.command {
            Commands::Workflow(WorkflowCommands::Inspect(args)) => {
                assert_eq!(args.workflow_ref.workflow_or_system, "google");
                assert_eq!(args.workflow_ref.workflow_name.as_deref(), Some("search"));
                assert!(args.json);
            }
            other => panic!("expected workflow inspect command, got {:?}", other),
        }
    }

    #[test]
    fn parse_workflow_contract_alias_json() {
        let cli = Cli::try_parse_from([
            "rzn-browser",
            "workflow",
            "contract",
            "google",
            "search",
            "--json",
        ])
        .expect("parse workflow contract alias");

        match cli.command {
            Commands::Workflow(WorkflowCommands::Contract(args)) => {
                assert_eq!(args.workflow_ref.workflow_or_system, "google");
                assert_eq!(args.workflow_ref.workflow_name.as_deref(), Some("search"));
                assert!(args.json);
            }
            other => panic!("expected workflow contract command, got {:?}", other),
        }
    }

    #[test]
    fn parse_capability_resolve_requires_explicit_system() {
        let cli = Cli::try_parse_from([
            "rzn-browser",
            "workflow",
            "capability",
            "resolve",
            "--system",
            "chatgpt",
            "assistant.conversation.read",
            "--json",
        ])
        .expect("parse capability resolve");

        match cli.command {
            Commands::Workflow(WorkflowCommands::Capability(CapabilityCommands::Resolve(args))) => {
                assert_eq!(args.system, "chatgpt");
                assert_eq!(args.capability_id, "assistant.conversation.read");
                assert!(args.json);
            }
            other => panic!("expected capability resolve command, got {:?}", other),
        }
    }

    #[test]
    fn parse_workflow_run_requires_direct_escape_hatch_flag() {
        let cli = Cli::try_parse_from([
            "rzn-browser",
            "workflow",
            "run",
            "google",
            "search",
            "--allow-direct-workflow",
        ])
        .expect("parse workflow run");

        match cli.command {
            Commands::Workflow(WorkflowCommands::Run(args)) => {
                assert_eq!(args.workflow_ref.workflow_or_system, "google");
                assert_eq!(args.workflow_ref.workflow_name.as_deref(), Some("search"));
                assert!(args.allow_direct_workflow);
            }
            other => panic!("expected workflow run command, got {:?}", other),
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
    fn build_workflow_help_params_reads_manifest_contract_inputs() {
        let workflow = json!({
            "schema_version": "rzn.workflow_manifest",
            "id": "chatgpt/send",
            "name": "ChatGPT: Send",
            "version": "1.0.2",
            "system": "chatgpt",
            "capability": "chatgpt.send",
            "params": {
                "properties": {
                    "attachment_file_paths": {
                        "kind": "array",
                        "description": "Absolute file paths to upload."
                    },
                    "message_text": {
                        "kind": "string",
                        "required": true,
                        "description": "Prompt text to send."
                    },
                    "tool": {
                        "kind": "string",
                        "enum_values": ["search", "none"]
                    }
                },
                "additional_params": false
            },
            "side_effects": [{ "class": "read_only" }],
            "runtime": { "actor": "supervisor" },
            "steps": [{
                "id": "send",
                "action": {
                    "kind": "extract_structured_data",
                    "side_effects": ["read_only"]
                }
            }],
            "result": {
                "output_selector": { "step_id": "send", "path": "$" }
            },
            "help": {
                "summary": "Send a message to ChatGPT.",
                "parameters": {
                    "attachment_file_paths": "Absolute file paths to upload.",
                    "message_text": "Prompt text to send.",
                    "tool": "Tool toggle."
                }
            }
        });

        let params =
            build_workflow_help_params(&workflow, &parse_workflow_help_metadata(&workflow));
        let message = params
            .iter()
            .find(|param| param.name == "message_text")
            .expect("message_text param");
        let attachments = params
            .iter()
            .find(|param| param.name == "attachment_file_paths")
            .expect("attachment_file_paths param");
        let tool = params
            .iter()
            .find(|param| param.name == "tool")
            .expect("tool param");

        assert!(message.required);
        assert_eq!(message.shape.as_deref(), Some("text"));
        assert_eq!(attachments.shape.as_deref(), Some("json array"));
        assert_eq!(
            attachments.example.as_deref(),
            Some("[\"/absolute/path/to/file.txt\"]")
        );
        assert_eq!(tool.example.as_deref(), Some("search"));
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
    fn render_run_system_discovery_uses_catalog_output() {
        let original_builtin = std::env::var("RZN_BUILTIN_WORKFLOWS_DIR").ok();
        let original_user = std::env::var("RZN_WORKFLOWS_DIR").ok();
        let original_runtime = std::env::var("RZN_RUNTIME_DIR").ok();
        let temp_runtime = temp_dir("rzn_run_discovery");
        let builtin_dir = temp_runtime.join("builtin");
        let user_dir = temp_runtime.join("user");
        std::fs::create_dir_all(builtin_dir.join("chatgpt")).expect("create builtin chatgpt dir");
        std::fs::create_dir_all(&user_dir).expect("create user dir");
        std::fs::write(
            builtin_dir.join("chatgpt").join("chatgpt_continue_chat_v1.json"),
            r#"{"id":"chatgpt/continue-chat-v1","name":"Continue Chat","description":"Continue chat","browser_automation":{"sequences":[{"name":"main","description":"Main","required_variables":[],"steps":[]}]}}"#,
        )
        .expect("write workflow");

        std::env::set_var("RZN_RUNTIME_DIR", &temp_runtime);
        std::env::set_var("RZN_BUILTIN_WORKFLOWS_DIR", &builtin_dir);
        std::env::set_var("RZN_WORKFLOWS_DIR", &user_dir);

        let rendered = render_run_system_discovery("chatgpt")
            .expect("render discovery")
            .expect("system discovery output");

        assert!(rendered.contains("Workflow system: chatgpt"));
        assert!(rendered.contains("Installed workflows"));
        assert!(rendered.contains("continue-chat-v1"));
        assert!(rendered.contains("rzn-browser list chatgpt <workflow>"));

        if let Some(value) = original_runtime {
            std::env::set_var("RZN_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("RZN_RUNTIME_DIR");
        }
        if let Some(value) = original_builtin {
            std::env::set_var("RZN_BUILTIN_WORKFLOWS_DIR", value);
        } else {
            std::env::remove_var("RZN_BUILTIN_WORKFLOWS_DIR");
        }
        if let Some(value) = original_user {
            std::env::set_var("RZN_WORKFLOWS_DIR", value);
        } else {
            std::env::remove_var("RZN_WORKFLOWS_DIR");
        }

        std::fs::remove_dir_all(temp_runtime).expect("cleanup runtime");
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
    fn strict_validate_accepts_manifest_file_through_contract_validator() {
        let root =
            std::env::temp_dir().join(format!("rzn_manifest_validate_{}", uuid::Uuid::new_v4()));
        let workflow_dir = root.join("workflows").join("x");
        std::fs::create_dir_all(&workflow_dir).unwrap();
        let manifest_path = workflow_dir.join("x_open.json");
        let manifest = json!({
            "schema_version": "rzn.workflow_manifest",
            "id": "x.open",
            "name": "Open X",
            "version": "0.1.0",
            "system": "x",
            "capability": "x.read.unified",
            "side_effects": [{ "class": "read_only" }],
            "runtime": { "actor": "supervisor" },
            "steps": [{
                "id": "extract",
                "action": {
                    "kind": "extract_structured_data",
                    "side_effects": ["read_only"]
                }
            }],
            "result": {
                "output_selector": { "step_id": "extract", "path": "$" }
            }
        });

        let report = validate_workflow_strict_contract("x/open", &manifest_path, &manifest);

        assert!(report.ok, "{:?}", report.issues);
        assert!(report.strict);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn workflow_output_payload_unwraps_run_result_v2_output() {
        let payload = json!({
            "version": "rzn.run_result.v2",
            "run_id": "run-1",
            "workflow_id": "x.open",
            "status": "succeeded",
            "output": { "markdown": "# done" }
        });

        assert_eq!(
            workflow_output_payload(&payload),
            &json!({ "markdown": "# done" })
        );
    }

    #[test]
    fn workflow_output_body_uses_unwrapped_run_result_payload() {
        let payload = json!({
            "version": "rzn.run_result.v2",
            "run_id": "run-1",
            "workflow_id": "x.open",
            "status": "succeeded",
            "output": { "items": [{ "title": "one" }] }
        });

        let body = workflow_output_body(workflow_output_payload(&payload));

        assert!(body.contains("\"items\""));
        assert!(!body.contains("rzn.run_result.v2"));
        assert!(!body.contains("\"run_id\""));
    }

    #[test]
    fn cli_output_side_effects_require_declared_file_write() {
        let root =
            std::env::temp_dir().join(format!("rzn_cli_output_policy_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let manifest_path = root.join("read.json");
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&json!({
                "schema_version": "rzn.workflow_manifest",
                "id": "fixture/read",
                "name": "Fixture Read",
                "version": "1.0.0",
                "system": "fixture",
                "capability": "fixture.read",
                "side_effects": [{ "class": "read_only" }],
                "runtime": { "actor": "supervisor" },
                "steps": [],
                "result": {}
            }))
            .unwrap(),
        )
        .unwrap();

        let err = enforce_cli_output_side_effect_policy(
            &manifest_path,
            Some(std::path::Path::new("/tmp/out.json")),
            None,
        )
        .unwrap_err();

        assert!(err.to_string().contains("file_write"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cli_download_side_effects_require_network_and_file_declarations() {
        let required =
            required_cli_output_side_effects(None, Some(std::path::Path::new("/tmp/assets")));

        assert!(required.contains("download"));
        assert!(required.contains("external_read"));
        assert!(required.contains("file_write"));
        assert!(required.contains("network_access"));
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
    fn parse_run_rejects_removed_endpoint_and_worker_flags() {
        for removed_flag in [
            "--endpoint-path",
            "--worker-cmd",
            "--worker-arg",
            "--profile",
        ] {
            let err =
                Cli::try_parse_from(["rzn-browser", "run", "google/search", removed_flag, "x"])
                    .expect_err("removed flag should be rejected");

            assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);
        }
    }
}
