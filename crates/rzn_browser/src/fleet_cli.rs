//! FLA-T-0005: operator-facing `rzn-browser fleet {enroll,status,disable}` CLI.
//!
//! These commands write and read `fleet_config.json` (the device identity the
//! supervisor's fleet poll loop, FLA-T-0003, consumes). Persistence mirrors the
//! `cloud pair` precedent (`cloud.rs`): version-tagged config written 0600 via the
//! `rzn_core::secure_files` helpers, atomically replaced through a temp+rename.
//!
//! Config shape (shared contract with FLA-T-0003):
//! ```json
//! {
//!   "version": "rzn.fleet.device_config.v1",
//!   "server_url": "https://fleet.example.com",
//!   "device_id": "dev_42",
//!   "device_token": "fld_...",
//!   "tenant_id": "tnt_1",
//!   "poll_interval_seconds": 30
//! }
//! ```
//! `poll_interval_seconds` is omitted when the server did not pin one (FLA-T-0003
//! then falls back to its own default). The device token is never printed or logged.

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use rzn_contracts::fleet_v1::{
    FleetDeviceSelfV1, FleetDeviceStatusV1, FleetEnrollRequestV1, FleetEnrollResponseV1,
    FleetErrorV1,
};
use rzn_core::runtime_paths::{default_app_base_dir, env_trimmed};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use url::Url;

/// Version tag pinned into `fleet_config.json`.
const FLEET_DEVICE_CONFIG_VERSION: &str = "rzn.fleet.device_config.v1";
/// Basename of the device config file in the runtime dir.
const FLEET_CONFIG_FILENAME: &str = "fleet_config.json";
/// Env override for the config path (primarily for tests / advanced operators).
const FLEET_CONFIG_PATH_ENV: &str = "RZN_FLEET_CONFIG_PATH";

const FLEET_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const FLEET_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Cap on best-effort local supervisor RPC probes so the CLI never hangs.
const SUPERVISOR_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------
// CLI surface
// ---------------------------------------------------------------------------

#[derive(Subcommand, Debug)]
pub enum FleetCommands {
    /// Enroll this device into a fleet using a one-time code from the dashboard.
    Enroll(FleetEnrollArgs),
    /// Show this device's fleet enrollment and status.
    Status(FleetStatusArgs),
    /// Remove this device's local fleet enrollment.
    Disable(FleetDisableArgs),
}

#[derive(Args, Debug)]
pub struct FleetEnrollArgs {
    /// Fleet control-plane base URL (https, or http only for loopback).
    #[arg(long)]
    pub server: String,
    /// One-time enrollment code copied from the fleet dashboard.
    #[arg(long)]
    pub code: String,
    /// Device name to register (defaults to this machine's hostname).
    #[arg(long)]
    pub name: Option<String>,
    /// Replace an existing enrollment on this device.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct FleetStatusArgs {
    /// Emit a single machine-readable JSON object.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct FleetDisableArgs {
    /// Skip the interactive confirmation prompt.
    #[arg(long)]
    pub yes: bool,
}

pub async fn handle_fleet_commands(cmd: FleetCommands) -> Result<()> {
    match cmd {
        FleetCommands::Enroll(args) => run_enroll(args).await,
        FleetCommands::Status(args) => run_status(args).await,
        FleetCommands::Disable(args) => run_disable(args).await,
    }
}

// ---------------------------------------------------------------------------
// Device config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct FleetDeviceConfig {
    version: String,
    server_url: String,
    device_id: String,
    device_token: String,
    tenant_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    poll_interval_seconds: Option<u64>,
}

/// Resolve the on-disk path of `fleet_config.json`.
///
/// `RZN_FLEET_CONFIG_PATH` wins when set (tests, advanced operators); otherwise
/// the file lives directly in the runtime dir (`default_app_base_dir()`), the
/// same base the supervisor resolves — so FLA-T-0003 reads the identical path.
fn fleet_config_path() -> PathBuf {
    if let Some(path) = env_trimmed(FLEET_CONFIG_PATH_ENV) {
        return PathBuf::from(path);
    }
    default_app_base_dir().join(FLEET_CONFIG_FILENAME)
}

/// Write the config atomically with 0600 perms (temp file + rename).
fn write_fleet_config(path: &Path, config: &FleetDeviceConfig) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(config).context("serialize fleet config")?;
    if let Some(parent) = path.parent() {
        rzn_core::secure_files::ensure_private_dir(parent)
            .with_context(|| format!("prepare config dir {}", parent.display()))?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(FLEET_CONFIG_FILENAME);
    let tmp = path.with_file_name(format!("{file_name}.tmp"));
    rzn_core::secure_files::write_secret_file(&tmp, &bytes)
        .with_context(|| format!("write temp config {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("atomically replace {}", path.display()))?;
    rzn_core::secure_files::set_secret_file_permissions(path)
        .with_context(|| format!("secure config perms {}", path.display()))?;
    Ok(())
}

/// Read the config, returning `None` when no enrollment exists.
fn read_fleet_config(path: &Path) -> Result<Option<FleetDeviceConfig>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let config = serde_json::from_slice(&bytes)
                .with_context(|| format!("parse fleet config {}", path.display()))?;
            Ok(Some(config))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("read fleet config {}", path.display())),
    }
}

/// Delete the config; returns whether a file was actually removed (idempotent).
fn delete_fleet_config(path: &Path) -> Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("remove fleet config {}", path.display())),
    }
}

pub(crate) async fn enroll_from_rpc(server: &str, code: &str) -> Result<Value> {
    let server_url = normalize_fleet_server_url(server)?;
    let request = FleetEnrollRequestV1 {
        code: code.to_string(),
        device_name: default_device_name(),
        platform: device_platform(),
        capabilities: device_capabilities(),
        cli_version: cli_version(),
        extension_version: detect_extension_version().await,
    };
    let config = perform_enroll(
        &fleet_http_client()?,
        &server_url,
        &request,
        &fleet_config_path(),
        false,
    )
    .await?;
    Ok(
        json!({"ok":true,"device_id":config.device_id,"tenant_id":config.tenant_id,"server_url":config.server_url}),
    )
}

pub(crate) fn unenroll_from_rpc() -> Result<Value> {
    let removed = delete_fleet_config(&fleet_config_path())?;
    Ok(json!({"ok":true,"removed":removed}))
}

// ---------------------------------------------------------------------------
// Enroll
// ---------------------------------------------------------------------------

async fn run_enroll(args: FleetEnrollArgs) -> Result<()> {
    let server_url = normalize_fleet_server_url(&args.server)?;
    let device_name = args.name.clone().unwrap_or_else(default_device_name);
    let request = FleetEnrollRequestV1 {
        code: args.code.clone(),
        device_name,
        platform: device_platform(),
        capabilities: device_capabilities(),
        cli_version: cli_version(),
        extension_version: detect_extension_version().await,
    };
    let config_path = fleet_config_path();
    let config = perform_enroll(
        &fleet_http_client()?,
        &server_url,
        &request,
        &config_path,
        args.force,
    )
    .await?;

    // Never print the device token.
    println!("device_id={}", config.device_id);
    println!("tenant_id={}", config.tenant_id);
    println!("server={}", config.server_url);
    println!("config written to {}", config_path.display());
    println!("supervisor will start polling within a minute; restart it if it is already running");
    Ok(())
}

/// Core enroll: guard existing identity, POST the code, persist the config.
///
/// Split from `run_enroll` so tests can drive it against a mock server without
/// constructing the whole CLI arg surface or touching the local supervisor.
async fn perform_enroll(
    client: &reqwest::Client,
    server_url: &str,
    request: &FleetEnrollRequestV1,
    config_path: &Path,
    force: bool,
) -> Result<FleetDeviceConfig> {
    if !force && config_path.exists() {
        bail!(
            "already enrolled ({}); re-run with --force to replace the existing device identity",
            config_path.display()
        );
    }

    let url = format!("{server_url}/v1/fleet/enroll");
    let response = client
        .post(&url)
        .json(request)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("enrollment failed: {}", describe_http_error(status, &body));
    }

    let enroll: FleetEnrollResponseV1 =
        serde_json::from_str(&body).context("decode enroll response")?;
    let config = FleetDeviceConfig {
        version: FLEET_DEVICE_CONFIG_VERSION.to_string(),
        server_url: server_url.to_string(),
        device_id: enroll.device_id.clone(),
        device_token: enroll.device_token.clone(),
        tenant_id: enroll.tenant_id.clone(),
        poll_interval_seconds: (enroll.poll_interval_seconds > 0)
            .then_some(enroll.poll_interval_seconds),
    };
    write_fleet_config(config_path, &config)?;
    Ok(config)
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// Machine-readable status report. Never carries the device token.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct FleetStatusReport {
    enrolled: bool,
    device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    tenant_id: String,
    server_url: String,
    server_reachable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_seen_at_ms: Option<i64>,
    loop_state: Value,
}

async fn run_status(args: FleetStatusArgs) -> Result<()> {
    let config_path = fleet_config_path();
    let Some(config) = read_fleet_config(&config_path)? else {
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({ "enrolled": false }))?
            );
        } else {
            println!("not enrolled (no {} found)", config_path.display());
        }
        return Ok(());
    };

    let client = fleet_http_client()?;
    let self_result = fetch_device_self(&client, &config.server_url, &config.device_token).await;
    let server_reachable = self_result.is_ok();
    if let Err(err) = &self_result {
        if !args.json {
            eprintln!("warning: could not reach fleet server: {err:#}");
        }
    }
    let device_self = self_result.ok();

    let loop_state = probe_fleet_loop_state().await;
    let report = build_status_report(&config, device_self.as_ref(), server_reachable, loop_state);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        render_status_human(&report);
    }
    // Graceful degradation: status always exits 0.
    Ok(())
}

async fn fetch_device_self(
    client: &reqwest::Client,
    server_url: &str,
    device_token: &str,
) -> Result<FleetDeviceSelfV1> {
    let url = format!("{server_url}/v1/fleet/device/self");
    let response = client
        .get(&url)
        .bearer_auth(device_token)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!(
            "device self lookup failed: {}",
            describe_http_error(status, &body)
        );
    }
    serde_json::from_str(&body).context("decode device self response")
}

fn build_status_report(
    config: &FleetDeviceConfig,
    device_self: Option<&FleetDeviceSelfV1>,
    server_reachable: bool,
    loop_state: Value,
) -> FleetStatusReport {
    FleetStatusReport {
        enrolled: true,
        device_id: config.device_id.clone(),
        name: device_self.map(|view| view.name.clone()),
        tenant_id: config.tenant_id.clone(),
        server_url: config.server_url.clone(),
        server_reachable,
        device_status: device_self.map(|view| device_status_str(&view.status).to_string()),
        last_seen_at_ms: device_self.and_then(|view| view.last_seen_at_ms),
        loop_state,
    }
}

fn render_status_human(report: &FleetStatusReport) {
    let name = report.name.as_deref().unwrap_or("-");
    println!("Fleet device {} ({})", report.device_id, name);
    println!("  tenant:    {}", report.tenant_id);
    println!("  server:    {}", report.server_url);
    println!(
        "  reachable: {}",
        if report.server_reachable { "yes" } else { "no" }
    );
    println!(
        "  status:    {}",
        report.device_status.as_deref().unwrap_or("unknown")
    );
    println!(
        "  last seen: {}",
        report
            .last_seen_at_ms
            .map(|ms| ms.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    let loop_summary = report
        .loop_state
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            report
                .loop_state
                .get("state")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| report.loop_state.to_string());
    println!("  loop:      {loop_summary}");
}

// ---------------------------------------------------------------------------
// Disable
// ---------------------------------------------------------------------------

async fn run_disable(args: FleetDisableArgs) -> Result<()> {
    let config_path = fleet_config_path();
    if !config_path.exists() {
        println!("not enrolled (nothing to disable)");
        return Ok(());
    }
    if !args.yes && !confirm("Disable fleet enrollment on this device and delete its config?")? {
        println!("aborted");
        return Ok(());
    }

    // Best effort: tell a running supervisor to stop the poll loop.
    notify_supervisor_disable().await;

    let removed = delete_fleet_config(&config_path)?;
    if removed {
        println!("fleet enrollment removed ({})", config_path.display());
    } else {
        println!("not enrolled (nothing to disable)");
    }
    println!(
        "note: this opts out locally only; an operator must revoke this device server-side in the dashboard/API"
    );
    Ok(())
}

fn confirm(prompt: &str) -> Result<bool> {
    use std::io::Write;
    print!("{prompt} [y/N] ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("read confirmation from stdin")?;
    let answer = line.trim().to_ascii_lowercase();
    Ok(answer == "y" || answer == "yes")
}

// ---------------------------------------------------------------------------
// Local supervisor RPC (best effort — FLA-T-0003 provides the methods)
// ---------------------------------------------------------------------------

/// Query the supervisor's `fleet.status` RPC. Any failure (supervisor down or the
/// method not yet registered) degrades to an `available: false` marker so the CLI
/// keeps working before FLA-T-0003 lands.
async fn probe_fleet_loop_state() -> Value {
    let config = crate::supervisor::SupervisorConfig { app_base: None };
    let future = crate::supervisor::call(config, "fleet.status", json!({}));
    match tokio::time::timeout(SUPERVISOR_PROBE_TIMEOUT, future).await {
        Ok(Ok(value)) => value,
        Ok(Err(err)) => json!({ "available": false, "reason": describe_rpc_error(&err) }),
        Err(_) => json!({ "available": false, "reason": "supervisor did not respond" }),
    }
}

async fn notify_supervisor_disable() {
    let config = crate::supervisor::SupervisorConfig { app_base: None };
    let future = crate::supervisor::call(config, "fleet.disable", json!({}));
    let _ = tokio::time::timeout(SUPERVISOR_PROBE_TIMEOUT, future).await;
}

/// Best-effort extension version the supervisor last saw; empty when unavailable.
async fn detect_extension_version() -> String {
    let config = crate::supervisor::SupervisorConfig { app_base: None };
    let future = crate::supervisor::call(config, "runtime.status", json!({}));
    match tokio::time::timeout(SUPERVISOR_PROBE_TIMEOUT, future).await {
        Ok(Ok(value)) => find_string_field(&value, "extension_version").unwrap_or_default(),
        _ => String::new(),
    }
}

fn describe_rpc_error(err: &anyhow::Error) -> String {
    let text = err.to_string().to_ascii_lowercase();
    if text.contains("-32601")
        || text.contains("method not found")
        || text.contains("unknown method")
    {
        "loop state unavailable (supervisor lacks fleet mode)".to_string()
    } else {
        "loop state unavailable (supervisor not running)".to_string()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fleet_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(FLEET_HTTP_CONNECT_TIMEOUT)
        .timeout(FLEET_HTTP_REQUEST_TIMEOUT)
        .build()
        .context("build fleet HTTP client")
}

/// Validate + normalize the fleet server URL: https required except for loopback
/// (mirrors the cloud transport rule in `cloud.rs`).
fn normalize_fleet_server_url(server: &str) -> Result<String> {
    let mut url =
        Url::parse(server.trim()).with_context(|| format!("invalid --server URL {server}"))?;
    match url.scheme() {
        "https" => {}
        "http" if is_loopback_url(&url) => {}
        "http" => bail!("--server must use https unless it targets loopback (got {server})"),
        other => bail!("--server has unsupported scheme {other}"),
    }
    if url.path() == "/" {
        url.set_path("");
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn is_loopback_url(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .map(|addr| addr.is_loopback())
            .unwrap_or(false)
}

fn describe_http_error(status: reqwest::StatusCode, body: &str) -> String {
    if let Ok(err) = serde_json::from_str::<FleetErrorV1>(body) {
        return format!("{} ({})", err.message, err.code);
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        format!("server returned HTTP {status}")
    } else {
        format!("server returned HTTP {status}: {trimmed}")
    }
}

fn device_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn cli_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Capabilities advertised at enroll. `fleet_poll_v1` marks fleet-protocol support;
/// the rest describe this device's actor surface.
fn device_capabilities() -> Vec<String> {
    vec![
        "fleet_poll_v1".to_string(),
        "extension_actor".to_string(),
        "workflow_runner_v2".to_string(),
    ]
}

fn default_device_name() -> String {
    os_hostname().unwrap_or_else(|| format!("rzn-{}", std::env::consts::OS))
}

#[cfg(unix)]
fn os_hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let name = String::from_utf8_lossy(&buf[..end]).trim().to_string();
    (!name.is_empty()).then_some(name)
}

#[cfg(not(unix))]
fn os_hostname() -> Option<String> {
    env_trimmed("COMPUTERNAME")
}

fn device_status_str(status: &FleetDeviceStatusV1) -> &'static str {
    match status {
        FleetDeviceStatusV1::Active => "active",
        FleetDeviceStatusV1::Dormant => "dormant",
        FleetDeviceStatusV1::Revoked => "revoked",
    }
}

fn find_string_field(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(found)) = map.get(key) {
                if !found.is_empty() {
                    return Some(found.clone());
                }
            }
            map.values()
                .find_map(|nested| find_string_field(nested, key))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|nested| find_string_field(nested, key)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("rzn-fleet-cli-{tag}-{unique}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn sample_config() -> FleetDeviceConfig {
        FleetDeviceConfig {
            version: FLEET_DEVICE_CONFIG_VERSION.to_string(),
            server_url: "https://fleet.example.com".to_string(),
            device_id: "dev_42".to_string(),
            device_token: "fld_secret_value".to_string(),
            tenant_id: "tnt_1".to_string(),
            poll_interval_seconds: Some(30),
        }
    }

    async fn spawn_mock(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{addr}")
    }

    #[cfg(unix)]
    fn file_mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    // A1: config round-trip, 0600 asserted.
    #[test]
    fn config_round_trips_and_is_0600() {
        let dir = temp_dir("round-trip");
        let path = dir.join(FLEET_CONFIG_FILENAME);
        let config = sample_config();

        write_fleet_config(&path, &config).expect("write config");
        let loaded = read_fleet_config(&path)
            .expect("read config")
            .expect("present");
        assert_eq!(loaded, config);

        #[cfg(unix)]
        assert_eq!(file_mode(&path), 0o600, "fleet config must be 0600");

        // Omitted poll interval stays omitted on the wire.
        let mut no_interval = config.clone();
        no_interval.poll_interval_seconds = None;
        let text = serde_json::to_string(&no_interval).unwrap();
        assert!(
            !text.contains("poll_interval_seconds"),
            "None interval must be skipped: {text}"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    // A1: enroll happy path writes a valid 0600 config; token stored, not returned in a print.
    #[tokio::test]
    async fn enroll_happy_path_writes_config() {
        async fn enroll_ok() -> Json<FleetEnrollResponseV1> {
            Json(FleetEnrollResponseV1 {
                device_id: "dev_enrolled".to_string(),
                device_token: "fld_top_secret".to_string(),
                tenant_id: "tnt_9".to_string(),
                poll_interval_seconds: 45,
                server_time_ms: 1_720_000_000_000,
            })
        }
        let base = spawn_mock(Router::new().route("/v1/fleet/enroll", post(enroll_ok))).await;

        let dir = temp_dir("enroll-ok");
        let path = dir.join(FLEET_CONFIG_FILENAME);
        let request = FleetEnrollRequestV1 {
            code: "otc_abc".to_string(),
            device_name: "unit-test-device".to_string(),
            platform: device_platform(),
            capabilities: device_capabilities(),
            cli_version: cli_version(),
            extension_version: String::new(),
        };

        let config = perform_enroll(&fleet_http_client().unwrap(), &base, &request, &path, false)
            .await
            .expect("enroll succeeds");

        assert_eq!(config.device_id, "dev_enrolled");
        assert_eq!(config.device_token, "fld_top_secret");
        assert_eq!(config.tenant_id, "tnt_9");
        assert_eq!(config.poll_interval_seconds, Some(45));
        assert_eq!(config.version, FLEET_DEVICE_CONFIG_VERSION);

        let on_disk = read_fleet_config(&path).unwrap().unwrap();
        assert_eq!(on_disk, config);
        #[cfg(unix)]
        assert_eq!(file_mode(&path), 0o600);

        let _ = std::fs::remove_dir_all(dir);
    }

    // A1: invalid/expired code surfaces the server's actionable message, non-zero.
    #[tokio::test]
    async fn enroll_rejects_invalid_code() {
        async fn enroll_bad() -> (StatusCode, Json<FleetErrorV1>) {
            (
                StatusCode::BAD_REQUEST,
                Json(FleetErrorV1 {
                    code: rzn_contracts::fleet_v1::error_codes::ENROLLMENT_CODE_INVALID.to_string(),
                    message: "enrollment code invalid or expired".to_string(),
                }),
            )
        }
        let base = spawn_mock(Router::new().route("/v1/fleet/enroll", post(enroll_bad))).await;

        let dir = temp_dir("enroll-bad");
        let path = dir.join(FLEET_CONFIG_FILENAME);
        let request = FleetEnrollRequestV1 {
            code: "otc_bad".to_string(),
            device_name: "unit-test-device".to_string(),
            platform: device_platform(),
            capabilities: device_capabilities(),
            cli_version: cli_version(),
            extension_version: String::new(),
        };

        let err = perform_enroll(&fleet_http_client().unwrap(), &base, &request, &path, false)
            .await
            .expect_err("invalid code must error");
        let text = err.to_string();
        assert!(
            text.contains("enrollment code invalid or expired"),
            "got: {text}"
        );
        assert!(text.contains("enrollment_code_invalid"), "got: {text}");
        assert!(!path.exists(), "no config should be written on failure");

        let _ = std::fs::remove_dir_all(dir);
    }

    // A1: an existing enrollment requires --force to overwrite.
    #[tokio::test]
    async fn enroll_requires_force_to_overwrite() {
        async fn enroll_ok() -> Json<FleetEnrollResponseV1> {
            Json(FleetEnrollResponseV1 {
                device_id: "dev_second".to_string(),
                device_token: "fld_second".to_string(),
                tenant_id: "tnt_2".to_string(),
                poll_interval_seconds: 0,
                server_time_ms: 0,
            })
        }
        let base = spawn_mock(Router::new().route("/v1/fleet/enroll", post(enroll_ok))).await;

        let dir = temp_dir("enroll-force");
        let path = dir.join(FLEET_CONFIG_FILENAME);
        write_fleet_config(&path, &sample_config()).unwrap();

        let request = FleetEnrollRequestV1 {
            code: "otc_new".to_string(),
            device_name: "unit-test-device".to_string(),
            platform: device_platform(),
            capabilities: device_capabilities(),
            cli_version: cli_version(),
            extension_version: String::new(),
        };

        let err = perform_enroll(&fleet_http_client().unwrap(), &base, &request, &path, false)
            .await
            .expect_err("existing config blocks enroll without --force");
        assert!(err.to_string().contains("--force"), "got: {err}");
        // Original identity untouched.
        assert_eq!(
            read_fleet_config(&path).unwrap().unwrap().device_id,
            "dev_42"
        );

        // With --force the identity is replaced.
        let config = perform_enroll(&fleet_http_client().unwrap(), &base, &request, &path, true)
            .await
            .expect("force overwrites");
        assert_eq!(config.device_id, "dev_second");
        assert_eq!(config.poll_interval_seconds, None, "0 interval → omitted");
        assert_eq!(
            read_fleet_config(&path).unwrap().unwrap().device_id,
            "dev_second"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    // A2: status --json shape from a fixture config + mocked self endpoint.
    #[tokio::test]
    async fn status_json_shape_from_fixture_and_mock() {
        async fn device_self() -> Json<FleetDeviceSelfV1> {
            Json(FleetDeviceSelfV1 {
                device_id: "dev_42".to_string(),
                name: "sarav-mbp".to_string(),
                status: FleetDeviceStatusV1::Active,
                tenant_id: "tnt_1".to_string(),
                last_seen_at_ms: Some(1_720_000_000_000),
            })
        }
        let base = spawn_mock(Router::new().route("/v1/fleet/device/self", get(device_self))).await;

        let mut config = sample_config();
        config.server_url = base.clone();

        let view = fetch_device_self(&fleet_http_client().unwrap(), &base, &config.device_token)
            .await
            .expect("self lookup");
        let report = build_status_report(
            &config,
            Some(&view),
            true,
            json!({ "available": false, "reason": "loop state unavailable (supervisor not running)" }),
        );

        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["enrolled"], json!(true));
        assert_eq!(value["device_id"], json!("dev_42"));
        assert_eq!(value["name"], json!("sarav-mbp"));
        assert_eq!(value["tenant_id"], json!("tnt_1"));
        assert_eq!(value["server_reachable"], json!(true));
        assert_eq!(value["device_status"], json!("active"));
        assert_eq!(value["last_seen_at_ms"], json!(1_720_000_000_000i64));
        assert!(value.get("loop_state").is_some());
        // The token must never appear in the status report.
        assert!(
            !serde_json::to_string(&value)
                .unwrap()
                .contains("fld_secret_value"),
            "device token leaked into status report"
        );

        let _ = base;
    }

    // A2: server unreachable degrades gracefully (no self data, still reports config).
    #[test]
    fn status_degrades_when_server_unreachable() {
        let config = sample_config();
        let report = build_status_report(
            &config,
            None,
            false,
            json!({ "available": false, "reason": "supervisor did not respond" }),
        );
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["enrolled"], json!(true));
        assert_eq!(value["server_reachable"], json!(false));
        assert_eq!(value["device_id"], json!("dev_42"));
        assert_eq!(value["server_url"], json!("https://fleet.example.com"));
        // Absent self fields are omitted, not null.
        assert!(value.get("device_status").is_none());
        assert!(value.get("name").is_none());
        assert!(value.get("last_seen_at_ms").is_none());
    }

    // A3: disable removes the config and is idempotent when not enrolled.
    #[test]
    fn disable_deletes_config_and_is_idempotent() {
        let dir = temp_dir("disable");
        let path = dir.join(FLEET_CONFIG_FILENAME);
        write_fleet_config(&path, &sample_config()).unwrap();

        assert!(
            delete_fleet_config(&path).unwrap(),
            "first delete removes the file"
        );
        assert!(!path.exists());
        assert!(
            !delete_fleet_config(&path).unwrap(),
            "second delete is a no-op (idempotent)"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    // A1/A4: server URL validation matches the cloud loopback-vs-https rule.
    #[test]
    fn server_url_validation_rules() {
        assert_eq!(
            normalize_fleet_server_url("https://fleet.example.com/").unwrap(),
            "https://fleet.example.com"
        );
        assert_eq!(
            normalize_fleet_server_url("http://127.0.0.1:8787").unwrap(),
            "http://127.0.0.1:8787"
        );
        assert_eq!(
            normalize_fleet_server_url("http://localhost:9000").unwrap(),
            "http://localhost:9000"
        );
        assert!(normalize_fleet_server_url("http://fleet.example.com").is_err());
        assert!(normalize_fleet_server_url("ftp://fleet.example.com").is_err());
        assert!(normalize_fleet_server_url("not a url").is_err());
    }

    #[test]
    fn finds_nested_string_field() {
        let value = json!({ "a": { "b": { "extension_version": "1.2.3" } } });
        assert_eq!(
            find_string_field(&value, "extension_version"),
            Some("1.2.3".to_string())
        );
        assert_eq!(
            find_string_field(&json!({ "x": 1 }), "extension_version"),
            None
        );
    }
}
