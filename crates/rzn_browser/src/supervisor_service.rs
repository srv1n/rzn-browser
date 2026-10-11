#[cfg(target_os = "macos")]
use anyhow::{anyhow, Context};
use anyhow::{bail, Result};
#[cfg(target_os = "macos")]
use serde::Serialize;
#[cfg(target_os = "macos")]
use serde_json::json;
use serde_json::Value;
#[cfg(target_os = "macos")]
use std::path::Path;

#[cfg(target_os = "macos")]
const LAUNCH_AGENT_LABEL: &str = crate::supervisor::SUPERVISOR_LAUNCHD_LABEL;
#[cfg(target_os = "macos")]
const MARKER_KIND: &str = "launchd-agent";
#[cfg(target_os = "macos")]
const WATCH_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
#[cfg(target_os = "macos")]
const MAX_OWNER_LOG_BYTES: u64 = 10 * 1024 * 1024;

#[cfg(target_os = "macos")]
#[derive(Debug, Serialize)]
struct OwnerMarker {
    schema: u32,
    kind: String,
    label: String,
}

#[cfg(target_os = "macos")]
impl OwnerMarker {
    fn launchd() -> Self {
        Self {
            schema: 1,
            kind: MARKER_KIND.to_string(),
            label: LAUNCH_AGENT_LABEL.to_string(),
        }
    }
}

pub(crate) async fn install(
    config: crate::supervisor::SupervisorConfig,
    restart: bool,
) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        install_macos(config, restart).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = config;
        let _ = restart;
        bail!("the launchd runtime owner is available only on macOS")
    }
}

pub(crate) async fn uninstall(config: crate::supervisor::SupervisorConfig) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        uninstall_macos(config).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = config;
        bail!("the launchd runtime owner is available only on macOS")
    }
}

pub(crate) async fn status(config: crate::supervisor::SupervisorConfig) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        status_macos(config).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = config;
        bail!("the launchd runtime owner is available only on macOS")
    }
}

#[cfg(target_os = "macos")]
async fn install_macos(
    config: crate::supervisor::SupervisorConfig,
    restart: bool,
) -> Result<Value> {
    use std::fs;

    let paths = crate::supervisor::SupervisorPaths::for_config(&config);
    anyhow::ensure!(
        paths.app_base.is_absolute(),
        "app base must be an absolute path"
    );
    let _transition = crate::supervisor::acquire_owner_transition_lock(&paths).await?;
    let marker_path = paths.owner_marker_path.clone();
    let plist_path = launch_agent_path()?;
    let app_base = paths.app_base.to_string_lossy().into_owned();
    let cli_path = installed_cli_path(&paths.app_base)?;
    let log_path = paths.app_base.join("logs/supervisor-owner.log");
    let desired_plist =
        render_launch_agent(&cli_path, &paths.app_base, &log_path, LAUNCH_AGENT_LABEL)?;

    let existing_plist = match fs::read(&plist_path) {
        Ok(bytes) => Some(bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).with_context(|| format!("read {}", plist_path.display())),
    };
    let marker_exists = crate::supervisor::managed_owner_label(&paths)?.is_some();
    let job_target = launchd_job_target(LAUNCH_AGENT_LABEL)?;
    let loaded = launchctl_loaded(&job_target)?;
    let same_plist = existing_plist.as_deref() == Some(desired_plist.as_bytes());

    if let Some(existing) = existing_plist.as_deref() {
        if !same_plist {
            let existing_args = parse_program_arguments(existing).with_context(|| {
                format!("inspect existing launch agent {}", plist_path.display())
            })?;
            ensure_label(existing, LAUNCH_AGENT_LABEL)?;
            ensure_same_app_base(&existing_args, &paths.app_base)?;
            if loaded && !restart {
                bail!(
                    "the loaded RZN runtime owner configuration changed; finish active workflows, then rerun `rzn-browser supervisor service install --restart`"
                );
            }
        }
    } else if loaded {
        bail!(
            "launchd label {LAUNCH_AGENT_LABEL} is already loaded without its managed plist; inspect it and unload it before installing RZN"
        );
    }

    let must_restart_loaded_owner = loaded && restart;
    if must_restart_loaded_owner {
        // Suppress watcher restarts before asking the supervisor to quiesce. If an active run
        // rejects the shutdown, restore recovery before returning the actionable error.
        request_owner_and_supervisor_stop_if_idle(&config, &paths).await?;
        launchctl_ok(&["bootout".to_string(), job_target.clone()])
            .context("unload the previous RZN runtime owner")?;
        wait_for_job_unloaded(&job_target).await?;
        wait_for_lock_release(&paths.owner_lock_path).await?;
        wait_for_lock_release(&paths.lock_path).await?;
    } else if !loaded
        && (crate::supervisor::supervisor_lock_is_held(&paths.lock_path)
            || crate::supervisor::owner_lock_is_held(&paths.owner_lock_path))
    {
        // First install or a service that was manually booted out: migrate a legacy direct
        // supervisor only after its own run gate confirms it is idle.
        if crate::supervisor::owner_lock_is_held(&paths.owner_lock_path) {
            request_owner_and_supervisor_stop_if_idle(&config, &paths).await?;
        } else {
            stop_supervisor_if_idle(&config, &paths).await?;
        }
        wait_for_lock_release(&paths.lock_path).await?;
    }
    if !loaded && crate::supervisor::owner_lock_is_held(&paths.owner_lock_path) {
        request_owner_stop(&paths)?;
        wait_for_lock_release(&paths.owner_lock_path).await?;
    }

    if !same_plist {
        prepare_owner_log(&log_path)?;
        write_atomic(&plist_path, desired_plist.as_bytes(), 0o600)?;
    } else if !loaded || must_restart_loaded_owner {
        prepare_owner_log(&log_path)?;
    }

    // Publish ownership before bootstrap so new native-host/CLI launches route to launchd.
    // The transition flock serializes this with every launcher check-and-spawn path.
    let marker_was_present = marker_exists;
    write_marker_atomic(&marker_path)?;

    if !loaded || must_restart_loaded_owner {
        if let Err(err) = launchctl_ok(&[
            "bootstrap".to_string(),
            launchd_domain()?,
            plist_path.to_string_lossy().into_owned(),
        ]) {
            if !launchctl_loaded(&job_target).unwrap_or(false) {
                if !marker_was_present {
                    let _ = fs::remove_file(&marker_path);
                }
                if existing_plist.is_none() {
                    let _ = fs::remove_file(&plist_path);
                }
            }
            return Err(err).context("load the RZN runtime owner with launchd");
        }
    }

    if let Err(ready_error) = wait_for_owner_ready(&config, &paths).await {
        if !marker_was_present && (!loaded || must_restart_loaded_owner) {
            if let Err(rollback_error) = rollback_new_owner(
                &paths,
                &job_target,
                &marker_path,
                &plist_path,
                existing_plist.as_deref(),
            )
            .await
            {
                return Err(ready_error).context(format!(
                    "owner readiness failed and rollback was incomplete: {rollback_error:#}; marker retained unless launchd unload was confirmed"
                ));
            }
        }
        return Err(ready_error);
    }
    Ok(json!({
        "ok": true,
        "installed": true,
        "changed": !loaded || must_restart_loaded_owner || !same_plist || !marker_exists,
        "label": LAUNCH_AGENT_LABEL,
        "app_base": app_base,
        "plist_path": plist_path,
        "marker_path": marker_path,
        "owner_log_path": log_path,
    }))
}

#[cfg(target_os = "macos")]
async fn uninstall_macos(config: crate::supervisor::SupervisorConfig) -> Result<Value> {
    use std::fs;

    let paths = crate::supervisor::SupervisorPaths::for_config(&config);
    anyhow::ensure!(
        paths.app_base.is_absolute(),
        "app base must be an absolute path"
    );
    let _transition = crate::supervisor::acquire_owner_transition_lock(&paths).await?;
    let marker_path = paths.owner_marker_path.clone();
    let plist_path = launch_agent_path()?;
    let job_target = launchd_job_target(LAUNCH_AGENT_LABEL)?;
    let marker = crate::supervisor::managed_owner_label(&paths)?;
    let existing_plist = match fs::read(&plist_path) {
        Ok(bytes) => Some(bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).with_context(|| format!("read {}", plist_path.display())),
    };

    if let Some(bytes) = existing_plist.as_deref() {
        let args = parse_program_arguments(bytes)
            .with_context(|| format!("inspect existing launch agent {}", plist_path.display()))?;
        ensure_label(bytes, LAUNCH_AGENT_LABEL)?;
        ensure_same_app_base(&args, &paths.app_base)?;
    }
    if marker.is_none() && existing_plist.is_none() {
        return Ok(json!({
            "ok": true,
            "installed": false,
            "label": LAUNCH_AGENT_LABEL,
            "app_base": paths.app_base,
        }));
    }

    // Explicit uninstall is allowed to stop the service, but it still refuses to interrupt a
    // workflow. Keep the marker until launchd has booted the owner out.
    if launchctl_loaded(&job_target)? {
        request_owner_and_supervisor_stop_if_idle(&config, &paths).await?;
        launchctl_ok(&["bootout".to_string(), job_target.clone()])
            .context("unload the RZN runtime owner")?;
        wait_for_job_unloaded(&job_target).await?;
    } else if crate::supervisor::owner_lock_is_held(&paths.owner_lock_path) {
        request_owner_and_supervisor_stop_if_idle(&config, &paths).await?;
    } else {
        stop_supervisor_if_idle(&config, &paths).await?;
    }
    wait_for_lock_release(&paths.owner_lock_path).await?;
    wait_for_lock_release(&paths.lock_path).await?;

    if marker.is_some() {
        fs::remove_file(&marker_path)
            .with_context(|| format!("remove owner marker {}", marker_path.display()))?;
    }
    if existing_plist.is_some() {
        fs::remove_file(&plist_path)
            .with_context(|| format!("remove launch agent {}", plist_path.display()))?;
    }
    Ok(json!({
        "ok": true,
        "installed": false,
        "label": LAUNCH_AGENT_LABEL,
        "app_base": paths.app_base,
        "plist_path": plist_path,
    }))
}

#[cfg(target_os = "macos")]
async fn status_macos(config: crate::supervisor::SupervisorConfig) -> Result<Value> {
    let paths = crate::supervisor::SupervisorPaths::for_config(&config);
    let _transition = crate::supervisor::acquire_owner_transition_lock(&paths).await?;
    let marker_path = paths.owner_marker_path.clone();
    let marker = crate::supervisor::managed_owner_label(&paths)?;
    let plist_path = launch_agent_path()?;
    let job_target = launchd_job_target(LAUNCH_AGENT_LABEL)?;
    let loaded = launchctl_loaded(&job_target)?;
    let plist_exists = plist_path.is_file();
    let owner_lock_held = crate::supervisor::owner_lock_is_held(&paths.owner_lock_path);
    let supervisor_lock_held = crate::supervisor::supervisor_lock_is_held(&paths.lock_path);
    let supervisor_status = crate::supervisor::call(config, "runtime.status", json!({}))
        .await
        .ok();
    Ok(json!({
        "ok": true,
        "supported": true,
        "installed": marker.is_some(),
        "launchd_loaded": loaded,
        "plist_exists": plist_exists,
        "owner_lock_held": owner_lock_held,
        "supervisor_lock_held": supervisor_lock_held,
        "supervisor": supervisor_status,
        "label": LAUNCH_AGENT_LABEL,
        "app_base": paths.app_base,
        "plist_path": plist_path,
        "marker_path": marker_path,
        "owner_lock_path": paths.owner_lock_path,
        "owner_log_path": paths.app_base.join("logs/supervisor-owner.log"),
        "owner_state_path": paths.owner_state_path,
        "transition_lock_path": paths.owner_transition_lock_path,
    }))
}

#[cfg(target_os = "macos")]
async fn stop_supervisor_if_idle(
    config: &crate::supervisor::SupervisorConfig,
    paths: &crate::supervisor::SupervisorPaths,
) -> Result<bool> {
    if !crate::supervisor::supervisor_lock_is_held(&paths.lock_path) {
        return Ok(false);
    }
    if crate::supervisor::call(config.clone(), "runtime.status", json!({}))
        .await
        .is_err()
    {
        bail!(
            "supervisor lock is held but the running supervisor cannot authenticate; no runtime owner changes were made. Stop it cleanly after confirming no workflow is active, then rerun `rzn-browser supervisor service install`"
        );
    }
    let result = match crate::supervisor::call(
        config.clone(),
        "runtime.shutdown_if_idle",
        json!({}),
    )
    .await
    {
        Ok(result) => result,
        Err(err) if err.to_string().contains("cannot shut down while workflow") => {
            bail!(
                "runtime owner migration deferred because a workflow is active: {err}. Let it finish, then rerun `rzn-browser supervisor service install`"
            )
        }
        Err(err) if err.to_string().contains("Unknown supervisor method") => {
            bail!(
                "the installed supervisor does not support safe idle migration; update it or stop it cleanly after confirming no workflow is active, then rerun `rzn-browser supervisor service install`: {err}"
            )
        }
        Err(err) => return Err(err).context("safely stop the idle supervisor for owner migration"),
    };
    if result.get("shutdown").and_then(Value::as_bool) != Some(true) {
        bail!(
            "runtime owner migration deferred because a workflow is active; let it finish, then rerun `rzn-browser supervisor service install`"
        );
    }
    wait_for_lock_release(&paths.lock_path).await?;
    Ok(true)
}

#[cfg(target_os = "macos")]
async fn request_owner_and_supervisor_stop_if_idle(
    config: &crate::supervisor::SupervisorConfig,
    paths: &crate::supervisor::SupervisorPaths,
) -> Result<()> {
    request_owner_stop(paths)?;
    if let Err(error) = stop_supervisor_if_idle(config, paths).await {
        crate::supervisor::clear_owner_stop_request(paths);
        return Err(error);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn request_owner_stop(paths: &crate::supervisor::SupervisorPaths) -> Result<()> {
    if !crate::supervisor::owner_lock_is_held(&paths.owner_lock_path) {
        return Ok(());
    }
    let bytes = match std::fs::read(&paths.owner_state_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("read supervisor owner state before unload"),
    };
    let state: Value =
        serde_json::from_slice(&bytes).context("parse supervisor owner state before unload")?;
    let Some(child_pid) = state
        .get("child_pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
    else {
        return Ok(());
    };
    crate::supervisor::request_managed_owner_stop(paths, child_pid)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn owner_child_pid(path: &Path) -> Option<u32> {
    let bytes = std::fs::read(path).ok()?;
    let state: Value = serde_json::from_slice(&bytes).ok()?;
    state
        .get("child_pid")?
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
}

#[cfg(target_os = "macos")]
async fn wait_for_owner_ready(
    config: &crate::supervisor::SupervisorConfig,
    paths: &crate::supervisor::SupervisorPaths,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + WATCH_READY_TIMEOUT;
    let mut last_error = None;
    while tokio::time::Instant::now() < deadline {
        if let Some(child_pid) = owner_child_pid(&paths.owner_state_path) {
            match crate::supervisor::call(config.clone(), "runtime.status", json!({})).await {
                Ok(status)
                    if status.get("pid").and_then(Value::as_u64) == Some(u64::from(child_pid)) =>
                {
                    return Ok(())
                }
                Ok(status) => {
                    last_error = Some(format!(
                        "owner child pid {child_pid} does not match runtime status pid {:?}",
                        status.get("pid")
                    ))
                }
                Err(err) => last_error = Some(err.to_string()),
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    bail!(
        "launchd loaded the RZN owner but its supervisor did not become ready within {}s: {}. Inspect `rzn-browser supervisor service status` and Console logs",
        WATCH_READY_TIMEOUT.as_secs(),
        last_error.unwrap_or_else(|| "owner lock was not acquired".to_string())
    )
}

#[cfg(target_os = "macos")]
async fn wait_for_lock_release(path: &Path) -> Result<()> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        if !crate::supervisor::supervisor_lock_is_held(path) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    bail!(
        "timed out waiting for runtime lock to release: {}",
        path.display()
    )
}

#[cfg(target_os = "macos")]
async fn wait_for_job_unloaded(job_target: &str) -> Result<()> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        if !launchctl_loaded(job_target)? {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    bail!("timed out waiting for launchd to unload {job_target}")
}

#[cfg(target_os = "macos")]
async fn rollback_new_owner(
    paths: &crate::supervisor::SupervisorPaths,
    job_target: &str,
    marker_path: &Path,
    plist_path: &Path,
    previous_plist: Option<&[u8]>,
) -> Result<()> {
    use std::fs;

    if launchctl_loaded(job_target)? {
        launchctl_ok(&["bootout".to_string(), job_target.to_string()])
            .context("unload the unready RZN runtime owner")?;
        wait_for_job_unloaded(job_target).await?;
    }
    wait_for_lock_release(&paths.owner_lock_path).await?;

    match previous_plist {
        Some(bytes) => write_atomic(plist_path, bytes, 0o600)?,
        None => match fs::remove_file(plist_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("remove new launch agent {}", plist_path.display()))
            }
        },
    }
    match fs::remove_file(marker_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("remove new owner marker {}", marker_path.display()))
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn write_marker_atomic(path: &Path) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(&OwnerMarker::launchd())?;
    write_atomic(path, &bytes, 0o600)
}

#[cfg(target_os = "macos")]
fn prepare_owner_log(path: &Path) -> Result<()> {
    use std::fs::{self, OpenOptions, Permissions};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("owner log path has no parent: {}", path.display()))?;
    if let Ok(metadata) = fs::symlink_metadata(parent) {
        anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "refusing symlinked owner log directory {}",
            parent.display()
        );
    }
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    fs::set_permissions(parent, Permissions::from_mode(0o700))
        .with_context(|| format!("secure owner log directory {}", parent.display()))?;

    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            anyhow::ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "refusing non-file or symlinked owner log {}",
                path.display()
            );
            if metadata.len() > MAX_OWNER_LOG_BYTES {
                let previous = path.with_file_name("supervisor-owner.log.previous");
                match fs::remove_file(&previous) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("remove previous owner log {}", previous.display())
                        })
                    }
                }
                fs::rename(path, &previous)
                    .with_context(|| format!("rotate oversized owner log {}", path.display()))?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| format!("inspect {}", path.display())),
    }

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("open owner log {}", path.display()))?;
    file.set_permissions(Permissions::from_mode(0o600))
        .with_context(|| format!("secure owner log {}", path.display()))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temp)
        .with_context(|| format!("create {}", temp.display()))?;
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path).with_context(|| format!("replace {}", path.display()))?;
        if let Ok(dir) = OpenOptions::new().read(true).open(parent) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(target_os = "macos")]
fn launch_agent_path() -> Result<std::path::PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| anyhow!("could not determine the current user's home directory"))?;
    anyhow::ensure!(
        home.is_absolute(),
        "home directory must be an absolute path"
    );
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LAUNCH_AGENT_LABEL}.plist")))
}

#[cfg(target_os = "macos")]
fn installed_cli_path(app_base: &Path) -> Result<std::path::PathBuf> {
    let candidate = app_base.join("bin/rzn-browser");
    if candidate.is_file() {
        return candidate
            .canonicalize()
            .with_context(|| format!("resolve installed CLI {}", candidate.display()));
    }
    std::env::current_exe()
        .context("resolve the running rzn-browser executable for launchd")?
        .canonicalize()
        .context("canonicalize the running rzn-browser executable")
}

#[cfg(target_os = "macos")]
fn render_launch_agent(
    cli: &Path,
    app_base: &Path,
    log_path: &Path,
    label: &str,
) -> Result<String> {
    let arguments = [
        cli.to_str()
            .ok_or_else(|| anyhow!("CLI path is not valid UTF-8"))?,
        "supervisor",
        "watch",
        "--app-base",
        app_base
            .to_str()
            .ok_or_else(|| anyhow!("app base path is not valid UTF-8"))?,
        "--json",
    ];
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    xml.push_str("  <key>Label</key>\n  <string>");
    xml.push_str(&xml_escape(label)?);
    xml.push_str("</string>\n  <key>ProgramArguments</key>\n  <array>\n");
    for argument in arguments {
        xml.push_str("    <string>");
        xml.push_str(&xml_escape(argument)?);
        xml.push_str("</string>\n");
    }
    xml.push_str(
        "  </array>\n  <key>EnvironmentVariables</key>\n  <dict>\n    <key>RUST_LOG</key>\n    <string>warn</string>\n  </dict>\n  <key>StandardErrorPath</key>\n  <string>",
    );
    xml.push_str(&xml_escape(
        log_path
            .to_str()
            .ok_or_else(|| anyhow!("owner log path is not valid UTF-8"))?,
    )?);
    xml.push_str(
        "</string>\n  <key>RunAtLoad</key>\n  <true/>\n  <key>KeepAlive</key>\n  <true/>\n</dict>\n</plist>\n",
    );
    Ok(xml)
}

#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> Result<String> {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            '\n' => escaped.push_str("&#10;"),
            '\r' => escaped.push_str("&#13;"),
            '\t' => escaped.push_str("&#9;"),
            ch if ch.is_control() => {
                bail!("path contains a control character unsupported by launchd plists")
            }
            ch => escaped.push(ch),
        }
    }
    Ok(escaped)
}

#[cfg(target_os = "macos")]
fn parse_program_arguments(bytes: &[u8]) -> Result<Vec<String>> {
    let plist = std::str::from_utf8(bytes).context("launchd plist is not UTF-8")?;
    let key = "<key>ProgramArguments</key>";
    let start = plist
        .find(key)
        .ok_or_else(|| anyhow!("launchd plist has no ProgramArguments"))?
        + key.len();
    let body = plist[start..]
        .split_once("<array>")
        .ok_or_else(|| anyhow!("launchd ProgramArguments is not an array"))?
        .1
        .split_once("</array>")
        .ok_or_else(|| anyhow!("launchd ProgramArguments array is unterminated"))?
        .0;
    let mut rest = body.trim();
    let mut arguments = Vec::new();
    while !rest.is_empty() {
        rest = rest
            .strip_prefix("<string>")
            .ok_or_else(|| anyhow!("unexpected content in launchd ProgramArguments"))?;
        let (value, tail) = rest
            .split_once("</string>")
            .ok_or_else(|| anyhow!("unterminated string in launchd ProgramArguments"))?;
        arguments.push(xml_unescape(value)?);
        rest = tail.trim();
    }
    Ok(arguments)
}

#[cfg(target_os = "macos")]
fn ensure_label(bytes: &[u8], expected: &str) -> Result<()> {
    let plist = std::str::from_utf8(bytes).context("launchd plist is not UTF-8")?;
    let key = "<key>Label</key>";
    let start = plist
        .find(key)
        .ok_or_else(|| anyhow!("launchd plist has no Label"))?
        + key.len();
    let body = plist[start..].trim_start();
    let value = body
        .strip_prefix("<string>")
        .and_then(|value| value.split_once("</string>"))
        .ok_or_else(|| anyhow!("launchd Label is not a string"))?
        .0;
    let actual = xml_unescape(value)?;
    if actual != expected {
        bail!("launchd plist label {actual:?} does not match {expected:?}");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn xml_unescape(value: &str) -> Result<String> {
    let mut decoded = String::with_capacity(value.len());
    let mut rest = value;
    while let Some((prefix, entity_rest)) = rest.split_once('&') {
        decoded.push_str(prefix);
        let (entity, tail) = entity_rest
            .split_once(';')
            .ok_or_else(|| anyhow!("invalid XML entity in launchd plist"))?;
        match entity {
            "amp" => decoded.push('&'),
            "lt" => decoded.push('<'),
            "gt" => decoded.push('>'),
            "quot" => decoded.push('"'),
            "apos" => decoded.push('\''),
            "#9" => decoded.push('\t'),
            "#10" => decoded.push('\n'),
            "#13" => decoded.push('\r'),
            _ => bail!("unsupported XML entity in launchd plist: &{entity};"),
        }
        rest = tail;
    }
    decoded.push_str(rest);
    Ok(decoded)
}

#[cfg(target_os = "macos")]
fn ensure_same_app_base(arguments: &[String], app_base: &Path) -> Result<()> {
    let old_app_base = arguments
        .windows(2)
        .find(|pair| pair[0] == "--app-base")
        .map(|pair| Path::new(&pair[1]))
        .ok_or_else(|| anyhow!("existing launch agent has no --app-base argument"))?;
    if old_app_base != app_base {
        bail!(
            "launchd label {LAUNCH_AGENT_LABEL} already belongs to app base {}; uninstall that runtime owner before installing {}",
            old_app_base.display(),
            app_base.display()
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn launchd_domain() -> Result<String> {
    let uid = unsafe { libc::getuid() };
    Ok(format!("gui/{uid}"))
}

#[cfg(target_os = "macos")]
fn launchd_job_target(label: &str) -> Result<String> {
    Ok(format!("{}/{}", launchd_domain()?, label))
}

#[cfg(target_os = "macos")]
fn launchctl_loaded(job_target: &str) -> Result<bool> {
    use std::process::Command;

    let output = Command::new("/bin/launchctl")
        .args(["print", job_target])
        .output()
        .context("run launchctl print")?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("Could not find service") {
        return Ok(false);
    }
    bail!(
        "launchctl print {job_target} failed ({}): {}",
        output.status,
        stderr.trim()
    )
}

#[cfg(target_os = "macos")]
fn launchctl_ok(arguments: &[String]) -> Result<()> {
    use std::process::Command;

    let output = Command::new("/bin/launchctl")
        .args(arguments)
        .output()
        .context("run launchctl")?;
    if !output.status.success() {
        bail!(
            "launchctl {} failed ({}): {}",
            arguments.first().map(String::as_str).unwrap_or("command"),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn owner_marker_schema_is_stable() {
        let marker = OwnerMarker::launchd();
        assert_eq!(marker.schema, 1);
        assert_eq!(marker.kind, "launchd-agent");
        assert_eq!(marker.label, "com.rzn.browser.runtime-owner");
        let encoded = serde_json::to_value(marker).expect("serialize marker");
        assert_eq!(encoded["schema"], 1);
        assert_eq!(encoded["kind"], "launchd-agent");
        assert_eq!(encoded["label"], "com.rzn.browser.runtime-owner");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn launchd_plist_escapes_paths_and_keeps_the_watch_foreground() {
        let app_base = Path::new("/tmp/RZN & <user> \"test\"");
        let cli = Path::new("/tmp/RZN & <user> \"test\"/bin/rzn-browser");
        let log = app_base.join("logs/supervisor-owner.log");
        let plist =
            render_launch_agent(cli, app_base, &log, LAUNCH_AGENT_LABEL).expect("render plist");
        assert!(plist.contains(
            "<string>/tmp/RZN &amp; &lt;user&gt; &quot;test&quot;/bin/rzn-browser</string>"
        ));
        assert!(plist.contains("<string>supervisor</string>\n    <string>watch</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(plist.contains("<key>KeepAlive</key>\n  <true/>"));
        assert!(plist.contains("<key>StandardErrorPath</key>"));
        assert!(plist.contains("<key>RUST_LOG</key>\n    <string>warn</string>"));
        assert!(plist.contains("logs/supervisor-owner.log</string>"));
        assert!(plist.contains(
            "<string>/tmp/RZN &amp; &lt;user&gt; &quot;test&quot;/logs/supervisor-owner.log</string>"
        ));
        let args = parse_program_arguments(plist.as_bytes()).expect("parse generated args");
        assert_eq!(
            args[1..],
            [
                "supervisor",
                "watch",
                "--app-base",
                app_base.to_str().unwrap(),
                "--json"
            ]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn launchd_plist_rejects_unsupported_control_characters() {
        let error = render_launch_agent(
            Path::new("/tmp/rzn\u{1}browser"),
            Path::new("/tmp/app"),
            Path::new("/tmp/app/logs/supervisor-owner.log"),
            LAUNCH_AGENT_LABEL,
        )
        .expect_err("control characters are invalid in plist strings");
        assert!(error.to_string().contains("control character"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn existing_launchd_label_and_app_base_are_checked() {
        let app_base = Path::new("/tmp/rzn");
        let plist = render_launch_agent(
            Path::new("/tmp/rzn/bin/rzn-browser"),
            app_base,
            Path::new("/tmp/rzn/logs/supervisor-owner.log"),
            LAUNCH_AGENT_LABEL,
        )
        .expect("render plist");
        ensure_label(plist.as_bytes(), LAUNCH_AGENT_LABEL).expect("matching label");
        assert!(ensure_label(plist.as_bytes(), "com.other.runtime").is_err());
        let args = parse_program_arguments(plist.as_bytes()).expect("parse generated args");
        ensure_same_app_base(&args, app_base).expect("matching app base");
        assert!(ensure_same_app_base(&args, Path::new("/tmp/other")).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn owner_log_is_private_and_rotates_when_oversized() {
        use std::os::unix::fs::PermissionsExt;

        let app_base = std::env::temp_dir().join(format!("rzn-owner-log-{}", uuid::Uuid::new_v4()));
        let log = app_base.join("logs/supervisor-owner.log");
        std::fs::create_dir_all(log.parent().unwrap()).expect("create fixture log directory");
        std::fs::write(&log, vec![b'x'; MAX_OWNER_LOG_BYTES as usize + 1])
            .expect("write oversized fixture log");

        prepare_owner_log(&log).expect("prepare private owner log");

        let current = std::fs::metadata(&log).expect("current owner log");
        let directory = std::fs::metadata(log.parent().unwrap()).expect("owner log directory");
        let previous = log.with_file_name("supervisor-owner.log.previous");
        assert_eq!(current.len(), 0);
        assert_eq!(current.permissions().mode() & 0o777, 0o600);
        assert_eq!(directory.permissions().mode() & 0o777, 0o700);
        assert_eq!(
            std::fs::metadata(previous)
                .expect("rotated owner log")
                .len(),
            MAX_OWNER_LOG_BYTES + 1
        );
        std::fs::remove_dir_all(app_base).expect("remove fixture app base");
    }
}
