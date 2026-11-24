//! Agent interface. Capture the ordinary runner in a child process so its
//! logs and error messages cannot bypass the public result projection.

use crate::{workflow_ref_value, Commands, RunArgs, WorkflowCommands};
use serde_json::{json, Map, Value};
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

type Result<T> = std::result::Result<T, &'static str>;

pub async fn handle(command: &Commands) -> Value {
    match dispatch(command).await {
        Ok(value) => value,
        Err(code) => json!({"success": false, "status": code}),
    }
}

async fn dispatch(command: &Commands) -> Result<Value> {
    match command {
        Commands::Run(args) => {
            let workflow =
                workflow_ref_value(&args.workflow_ref).map_err(|_| "unsupported_agent_command")?;
            if workflow == "book" {
                return Ok(help());
            }
            if workflow.starts_with("book/") {
                check_workflow(&workflow)?;
                if !args
                    .params
                    .iter()
                    .any(|(key, _)| key == "query" || key == "id")
                {
                    return Ok(help());
                }
            }
            run(args, &workflow).await
        }
        Commands::Workflow(WorkflowCommands::Show(args)) => workflow_help(&args.workflow_ref),
        Commands::Workflow(WorkflowCommands::Inspect(args)) => workflow_help(&args.workflow_ref),
        Commands::Workflow(WorkflowCommands::List(args)) => {
            if let Some(name) = &args.workflow_name {
                let reference = crate::WorkflowRefArgs {
                    workflow_or_system: args.system.clone().ok_or("invalid_workflow")?,
                    workflow_name: Some(name.clone()),
                };
                return workflow_help(&reference);
            }
            if args.system.as_deref() == Some("book") {
                return Ok(help());
            }
            let entries = crate::list_named_workflows_with_query(&crate::WorkflowCatalogQuery {
                system_filter: args.system.clone(),
                source_filter: args.source.map(|source| source.as_str().to_string()),
                include_all_sources: args.all_sources,
            })
            .map_err(|_| "catalog_unavailable")?;
            let workflows: Vec<_> = entries.iter().map(|entry| {
                if entry.system == "book" {
                    json!({"id": entry.id, "name": "Local Book Search",
                        "description": "Search book metadata or retrieve a book by ID."})
                } else {
                    json!({"id": entry.id, "name": entry.name, "description": entry.description})
                }
            }).collect();
            Ok(json!({"success": true, "workflows": workflows}))
        }
        _ => Err("unsupported_agent_command"),
    }
}

fn workflow_help(reference: &crate::WorkflowRefArgs) -> Result<Value> {
    let workflow = workflow_ref_value(reference).map_err(|_| "invalid_workflow")?;
    if workflow == "book" || workflow.starts_with("book/") {
        if workflow != "book" {
            check_workflow(&workflow)?;
        }
        return Ok(help());
    }
    let (_, manifest) = crate::load_workflow_value(&workflow).map_err(|_| "invalid_workflow")?;
    if manifest["system"] == "book" {
        return Ok(help());
    }
    Ok(
        json!({"success": true, "id": manifest["id"], "name": manifest["name"],
        "summary": manifest["summary"], "parameters": manifest["params"],
        "help": manifest["help"], "output_schema": manifest.pointer("/result/output_schema"),
        "side_effects": manifest["side_effects"]}),
    )
}

fn check_workflow(workflow: &str) -> Result<()> {
    match workflow {
        "book/search" | "book/get" => Ok(()),
        _ => Err("unsupported_agent_command"),
    }
}

fn help() -> Value {
    json!({
        "success": true,
        "name": "Local Book Search",
        "description": "Search online book collections and save a matching book locally.",
        "commands": [
            "rzn-browser run book search --agent-output --param query=\"TITLE AUTHOR\"",
            "rzn-browser run book get --agent-output --param id=BOOK_ID --download-dir ./books"
        ],
        "parameters": {
            "query": "Search: title, author, or ISBN. Get also accepts a query for automatic selection.",
            "id": "Get: opaque ID returned by search. Expires after 24 hours; selects the exact file.",
            "format": "File extension. Get defaults to epub; search returns all formats unless set.",
            "limit": "Search only: maximum rows, 1–100 (default 25).",
            "language": "Get only: optional language preference.",
            "result_index": "Get only: matching candidate index, 0–99 (default 0).",
            "sort": "Publication year: year_desc (newest), year_asc (oldest), or relevance (default). Unknown years last.",
            "mirror_urls": "Comma-separated fallback source URLs, tried in order; up to three total attempts.",
            "max_attempts": "Maximum source attempts, 1–3 (default 3). Get advances slow partner servers on retry.",
            "download-dir": "Get only: directory for saved attachments. Without it, return book metadata.",
            "max-download-mib": "Maximum size per downloaded file in MiB (default 1024; 0 disables the cap)."
        },
        "notes": [
            "Search returns metadata and opaque IDs; get returns metadata or local file receipts without provider URLs or logs.",
            "Existing workflow commands retain their ordinary output when this flag is omitted.",
            "This output mode is not an access-control boundary."
        ]
    })
}

async fn run(args: &RunArgs, workflow: &str) -> Result<Value> {
    if !matches!(workflow, "book/search" | "book/get") {
        return run_once(args, workflow, None).await;
    }
    let params = if let Some((_, id)) = args.params.iter().find(|(key, _)| key == "id") {
        // Validate the caller before loading the saved source configuration.
        if workflow != "book/get" || args.params.len() != 1 {
            return Err("invalid_book_id_parameters");
        }
        let record = load_book(&book_cache(args)?, id)?;
        serde_json::from_value(record["params"].clone()).map_err(|_| "invalid_book_id")?
    } else {
        args.params.clone()
    };
    let (_, manifest) = crate::load_workflow_value(workflow).map_err(|_| "invalid_workflow")?;
    let attempts = book_attempts(&params, &manifest, workflow)?;
    let mut last_error = "no_download_link";
    for (base, index) in attempts {
        match run_once(args, workflow, Some((&base, index))).await {
            Ok(value) => return Ok(value),
            Err(code) => {
                last_error = code;
                // A verification wall, invalid input, or local publish error needs
                // attention, not another network request. Retry only source failures.
                if !matches!(
                    code,
                    "no_download_link"
                        | "download_failed"
                        | "timeout"
                        | "unsupported_source_layout"
                        | "no_format_match"
                        | "book_unavailable_search_again"
                        | "source_unavailable"
                ) {
                    return Err(code);
                }
            }
        }
    }
    Err(last_error)
}

fn book_attempts(
    params: &[(String, String)],
    manifest: &Value,
    workflow: &str,
) -> Result<Vec<(String, usize)>> {
    let param = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    let max = param("max_attempts")
        .unwrap_or("3")
        .parse::<usize>()
        .map_err(|_| "invalid_parameters")?;
    if !(1..=3).contains(&max) {
        return Err("invalid_parameters");
    }
    let index = param("mirror_index")
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| "invalid_parameters")?;
    if index > 15 {
        return Err("invalid_parameters");
    }
    let base = param("base_url")
        .or_else(|| {
            manifest
                .pointer("/params/properties/base_url/default")
                .and_then(Value::as_str)
        })
        .ok_or("invalid_workflow")?;
    let mut sources = vec![base.to_string()];
    for source in param("mirror_urls")
        .or_else(|| {
            manifest
                .pointer("/params/properties/mirror_urls/default")
                .and_then(Value::as_str)
        })
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !sources.iter().any(|s| s == source) {
            sources.push(source.to_string());
        }
    }
    for source in &sources {
        let url = reqwest::Url::parse(source).map_err(|_| "invalid_parameters")?;
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("invalid_parameters");
        }
    }
    // Try configured sources first, then another partner on the last source.
    let count = if workflow == "book/search" {
        sources.len().min(max)
    } else {
        max
    };
    Ok((0..count)
        .map(|attempt| {
            (
                sources[attempt.min(sources.len() - 1)].clone(),
                (index + attempt.saturating_sub(sources.len() - 1)).min(15),
            )
        })
        .collect())
}

async fn run_once(args: &RunArgs, workflow: &str, attempt: Option<(&str, usize)>) -> Result<Value> {
    // Do not let the child write an unfiltered result outside its private scratch directory.
    if args.output_file.is_some()
        || args.params.iter().any(|(key, _)| {
            matches!(
                key.as_str(),
                "output_file" | "output-file" | "download_dir" | "download-dir"
            )
        })
    {
        return Err("use_download_dir_or_redirect_json_stdout");
    }
    if workflow == "book/search" && args.download_dir.is_some() {
        return Err("download_dir_requires_book_get");
    }

    let (_, manifest) = crate::load_workflow_value(workflow).map_err(|_| "invalid_workflow")?;
    let canonical = manifest["id"].as_str().ok_or("invalid_workflow")?;
    let is_book = manifest["system"] == "book" || canonical.starts_with("book/");
    if is_book {
        check_workflow(canonical)?;
    }
    let mut params = args.params.clone();
    let requested_id = params
        .iter()
        .find(|(key, _)| key == "id")
        .map(|(_, value)| value.clone());
    let mut expected_md5 = None;
    if is_book {
        if params.iter().any(|(key, _)| key == "selected_md5") {
            return Err("use_book_id");
        }
        if let Some(id) = &requested_id {
            if canonical != "book/get" || params.len() != 1 {
                return Err("invalid_book_id_parameters");
            }
            let record = load_book(&book_cache(args)?, id)?;
            params =
                serde_json::from_value(record["params"].clone()).map_err(|_| "invalid_book_id")?;
            expected_md5 = record["md5"].as_str().map(str::to_string);
            params.push((
                "selected_md5".into(),
                expected_md5.clone().ok_or("invalid_book_id")?,
            ));
        }
    }
    if let Some((base, index)) = attempt {
        params.retain(|(key, _)| key != "base_url" && key != "mirror_index");
        params.push(("base_url".into(), base.into()));
        params.push(("mirror_index".into(), index.to_string()));
    }
    let scratch = Scratch::new()?;
    let mut child = Command::new(std::env::current_exe().map_err(|_| "runner_unavailable")?);
    child.args(["run", workflow]);
    for (key, value) in &params {
        child.arg("--param").arg(format!("{key}={value}"));
    }
    for (flag, value) in [
        ("--browser", args.target.browser.as_ref()),
        (
            "--browser-instance",
            args.target.browser_instance_id.as_ref(),
        ),
        ("--bridge", args.target.bridge_id.as_ref()),
        ("--tab-ref", args.tab_ref.as_ref()),
        ("--app-base", args.app_base.as_ref()),
    ] {
        if let Some(value) = value {
            child.arg(flag).arg(value);
        }
    }
    if args.keep_tab_open {
        child.arg("--keep-tab-open");
    }
    child.args(["--snapshot", &args.snapshot]);
    child
        .arg("--max-download-mib")
        .arg(args.max_download_mib.to_string());
    if args.download_dir.is_some() {
        child.arg("--download-dir").arg(if is_book {
            &scratch.0
        } else {
            args.download_dir.as_ref().unwrap()
        });
    }
    child.env("RZN_OUTPUT", "json").kill_on_drop(true);
    let timeout = run_timeout(&manifest, args.download_dir.is_some());
    let output = tokio::time::timeout(timeout, child.output())
        .await
        .map_err(|_| "timeout")?
        .map_err(|_| "runner_unavailable")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(failure_code(&format!("{stdout}\n{stderr}")));
    }
    let payload = parse_result(&stdout, canonical)?;
    let mut result = if is_book {
        let mut result = project_result(&payload, canonical)?;
        if let Some(md5) = &expected_md5 {
            if payload.pointer("/chosen/md5").and_then(Value::as_str) != Some(md5.as_str()) {
                return Err("book_unavailable_search_again");
            }
        }
        if canonical == "book/search" {
            let cache = book_cache(args)?;
            let mut selection = params.clone();
            selection.retain(|(key, _)| {
                matches!(
                    key.as_str(),
                    "query" | "base_url" | "mirror_urls" | "max_attempts" | "sort"
                )
            });
            if !selection.iter().any(|(key, _)| key == "base_url") {
                if let Some(base) = manifest
                    .pointer("/params/properties/base_url/default")
                    .and_then(Value::as_str)
                {
                    selection.push(("base_url".into(), base.into()));
                }
            }
            for (book, row) in result["results"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .zip(payload["results"].as_array().unwrap())
            {
                if let Some(md5) = row["md5"].as_str().filter(|md5| valid_md5(md5)) {
                    let mut selection = selection.clone();
                    selection.push((
                        "format".into(),
                        row["ext"].as_str().ok_or("invalid_result")?.into(),
                    ));
                    book["id"] = json!(save_book(&cache, &selection, md5)?);
                } else {
                    book["downloadable"] = json!(false);
                }
            }
        } else if let Some(id) = &requested_id {
            result["book"]["id"] = json!(id);
        }
        result
    } else {
        json!({"success": true, "status": "ok", "result": payload})
    };
    if let Some(destination) = &args.download_dir {
        result["files"] = if is_book {
            json!([publish_download(&scratch.0, destination).await?])
        } else {
            let manifest: Value = serde_json::from_slice(
                &fs::read(destination.join("manifest.json")).map_err(|_| "download_failed")?,
            )
            .map_err(|_| "download_failed")?;
            let items = manifest["items"].as_array().ok_or("download_failed")?;
            if items.is_empty() || items.iter().any(|item| item.get("error").is_some()) {
                return Err("download_failed");
            }
            json!(items)
        };
        result["status"] = json!("downloaded");
    }
    Ok(result)
}

fn run_timeout(manifest: &Value, downloads: bool) -> Duration {
    let workflow_ms = manifest
        .pointer("/runtime/timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(270_000)
        .saturating_add(30_000)
        .max(300_000);
    let download_ms = if downloads {
        rzn_browser::asset_download::DEFAULT_ASSET_TIMEOUT_SECS * 1000
    } else {
        0
    };
    Duration::from_millis(workflow_ms.saturating_add(download_ms))
}

fn failure_code(log: &str) -> &'static str {
    let lower = log.to_ascii_lowercase();
    if lower.contains("site_challenge") {
        "human_action_required"
    } else if lower.contains("open the existing") || lower.contains("no_bridges_connected") {
        "browser_unavailable"
    } else if lower.contains("timeout") || lower.contains("timed out") {
        "timeout"
    } else if lower.contains("navigation_error")
        || lower.contains("net::err_")
        || lower.contains("content_script_error")
    {
        "source_unavailable"
    } else {
        "workflow_failed"
    }
}

fn parse_result(log: &str, workflow: &str) -> Result<Value> {
    // Read the last complete frame: earlier progress may contain JSON-shaped page text.
    let mut frame = None;
    for (start, _) in log.match_indices('{') {
        if start != 0 && log.as_bytes()[start - 1] != b'\n' {
            continue;
        }
        let mut stream = serde_json::Deserializer::from_str(&log[start..]).into_iter::<Value>();
        if let Some(Ok(value)) = stream.next() {
            if value["version"] == "rzn.run_result" && value["workflow_id"] == workflow {
                frame = Some(value);
            }
        }
    }
    let value = frame.ok_or("invalid_result")?;
    if value["status"] != "succeeded" {
        return Err("workflow_failed");
    }
    let output = value.get("output").ok_or("invalid_result")?;
    Ok(output.get("result").unwrap_or(output).clone())
}

const BOOK_TTL: Duration = Duration::from_secs(24 * 60 * 60);

fn book_cache(args: &RunArgs) -> Result<PathBuf> {
    let path = if let Some(base) = &args.app_base {
        PathBuf::from(base).join("secure/book-results")
    } else {
        rzn_core::secure_files::secure_dir("book-results").map_err(|_| "cache_unavailable")?
    };
    rzn_core::secure_files::ensure_private_dir(&path).map_err(|_| "cache_unavailable")?;
    rzn_core::secure_files::cleanup_secure_artifacts(&path, BOOK_TTL, 2000)
        .map_err(|_| "cache_unavailable")?;
    Ok(path)
}

fn valid_md5(md5: &str) -> bool {
    md5.len() == 32 && md5.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn save_book(cache: &Path, params: &[(String, String)], md5: &str) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "cache_unavailable")?
        .as_secs();
    let record = json!({"created": created, "params": params, "md5": md5});
    rzn_core::secure_files::write_secret_file(
        &cache.join(format!("{id}.json")),
        record.to_string(),
    )
    .map_err(|_| "cache_unavailable")?;
    Ok(id)
}

fn load_book(cache: &Path, id: &str) -> Result<Value> {
    let parsed = uuid::Uuid::parse_str(id).map_err(|_| "invalid_book_id")?;
    if parsed.to_string() != id {
        return Err("invalid_book_id");
    }
    let record: Value = serde_json::from_slice(
        &fs::read(cache.join(format!("{id}.json"))).map_err(|_| "book_id_expired_search_again")?,
    )
    .map_err(|_| "invalid_book_id")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "cache_unavailable")?
        .as_secs();
    let age = now
        .checked_sub(record["created"].as_u64().ok_or("invalid_book_id")?)
        .ok_or("invalid_book_id")?;
    if age >= BOOK_TTL.as_secs() {
        return Err("book_id_expired_search_again");
    }
    if !record["md5"].as_str().is_some_and(valid_md5) {
        return Err("invalid_book_id");
    }
    Ok(record)
}

fn book_metadata(value: &Value) -> Value {
    let mut book = Map::new();
    for field in [
        "title",
        "author",
        "publisher",
        "year",
        "language",
        "pages",
        "size",
        "ext",
    ] {
        if let Some(text) = value.get(field).and_then(Value::as_str) {
            book.insert(field.into(), json!(text));
        }
    }
    Value::Object(book)
}

fn project_result(value: &Value, workflow: &str) -> Result<Value> {
    if workflow == "book/search" {
        if value["layout_ok"] != true {
            return Err("unsupported_source_layout");
        }
        let rows = value["results"].as_array().ok_or("invalid_result")?;
        let books: Vec<_> = rows.iter().map(book_metadata).collect();
        Ok(json!({"success": true, "status": "ok", "count": books.len(), "results": books}))
    } else {
        if value["found"] != true {
            return Err(match value["status"].as_str() {
                Some("no_format_match") => "no_format_match",
                Some("download_wait_timeout") => "timeout",
                Some("no_table") => "unsupported_source_layout",
                _ => "no_download_link",
            });
        }
        if !value["chosen"].is_object() {
            return Err("invalid_result");
        }
        Ok(json!({"success": true, "status": "resolved", "book": book_metadata(&value["chosen"])}))
    }
}

async fn publish_download(scratch: &Path, destination: &Path) -> Result<Value> {
    let manifest: Value = serde_json::from_slice(
        &fs::read(scratch.join("manifest.json")).map_err(|_| "download_failed")?,
    )
    .map_err(|_| "download_failed")?;
    let items = manifest["items"].as_array().ok_or("download_failed")?;
    if items.len() != 1 || items[0]["kind"] != "attachment" {
        return Err("download_failed");
    }
    if let Some(error) = items[0].get("error") {
        return Err(
            if error
                .as_str()
                .is_some_and(|error| error.contains("exceeds cap"))
            {
                "download_too_large"
            } else {
                "download_failed"
            },
        );
    }
    let item = &items[0];
    let source = scratch
        .join(item["path"].as_str().ok_or("download_failed")?)
        .canonicalize()
        .map_err(|_| "download_failed")?;
    let private_root = scratch.canonicalize().map_err(|_| "download_failed")?;
    if !source.starts_with(private_root) || !source.is_file() {
        return Err("download_failed");
    }
    let (bytes, hash) = rzn_browser::asset_download::hash_existing(&source)
        .await
        .map_err(|_| "download_failed")?;
    if bytes == 0 || item["bytes"] != bytes || item["sha256"] != hash {
        return Err("download_failed");
    }
    let mut prefix = [0; 512];
    let len = fs::File::open(&source)
        .and_then(|mut file| file.read(&mut prefix))
        .map_err(|_| "download_failed")?;
    let text = String::from_utf8_lossy(&prefix[..len])
        .trim_start()
        .to_ascii_lowercase();
    if text.starts_with("<!doctype html") || text.starts_with("<html") {
        return Err("download_failed");
    }
    let folder = destination.join("attachments");
    fs::create_dir_all(&folder).map_err(|_| "file_write_failed")?;
    let target = folder.join(source.file_name().ok_or("download_failed")?);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::AlreadyExists {
                "destination_exists"
            } else {
                "file_write_failed"
            }
        })?;
    let copied = (|| -> std::io::Result<()> {
        std::io::copy(&mut fs::File::open(&source)?, &mut file)?;
        file.sync_all()
    })();
    if copied.is_err() {
        drop(file);
        let _ = fs::remove_file(&target);
        return Err("file_write_failed");
    }
    let path = target.canonicalize().map_err(|_| "file_write_failed")?;
    Ok(json!({"path": path, "bytes": bytes, "sha256": hash}))
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("rzn-book-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).map_err(|_| "file_write_failed")?;
        // Scratch holds the downloader's unfiltered manifest, never the public receipt.
        if rzn_core::secure_files::ensure_private_dir(&path).is_err() {
            let _ = fs::remove_dir(&path);
            return Err("file_write_failed");
        }
        Ok(Self(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn book_retries_visit_sources_then_advance_partner_with_a_hard_cap() {
        let manifest = json!({"params": {"properties": {
            "base_url": {"default": "https://first.example/"},
            "mirror_urls": {"default": "https://second.example/"}
        }}});
        assert_eq!(
            book_attempts(&[], &manifest, "book/get").unwrap(),
            vec![
                ("https://first.example/".into(), 0),
                ("https://second.example/".into(), 0),
                ("https://second.example/".into(), 1),
            ]
        );
        assert_eq!(
            book_attempts(&[], &manifest, "book/search").unwrap().len(),
            2
        );
        let one = vec![
            ("mirror_urls".into(), "".into()),
            ("mirror_index".into(), "2".into()),
        ];
        assert_eq!(
            book_attempts(&one, &manifest, "book/get").unwrap(),
            vec![
                ("https://first.example/".into(), 2),
                ("https://first.example/".into(), 3),
                ("https://first.example/".into(), 4),
            ]
        );
        for (key, value) in [
            ("max_attempts", "4"),
            ("max_attempts", "0"),
            ("mirror_urls", "file:///etc/passwd"),
            ("mirror_index", "16"),
        ] {
            assert_eq!(
                book_attempts(&[(key.into(), value.into())], &manifest, "book/get").unwrap_err(),
                "invalid_parameters"
            );
        }
        assert_eq!(
            failure_code("SITE_CHALLENGE navigation_error"),
            "human_action_required"
        );
        assert_eq!(
            failure_code("navigation_error: net::ERR_NAME_NOT_RESOLVED"),
            "source_unavailable"
        );
        assert_eq!(
            failure_code("CONTENT_SCRIPT_ERROR: Content script did not respond to ping"),
            "source_unavailable"
        );
    }

    #[tokio::test]
    async fn flag_help_and_missing_query_do_not_expose_provider_configuration() {
        for args in [
            vec!["run", "book", "search", "--agent-output"],
            vec![
                "--agent-output",
                "workflow",
                "show",
                "book",
                "get",
                "--json",
            ],
            vec!["workflow", "inspect", "book/search", "--agent-output"],
            vec!["workflow", "list", "book", "--agent-output"],
        ] {
            let cli =
                crate::Cli::try_parse_from(std::iter::once("rzn-browser").chain(args)).unwrap();
            assert!(cli.agent_output);
            let output = handle(&cli.command).await;
            assert_eq!(output["success"], true);
            let text = output.to_string();
            for private in ["base_url", "libgen", "https://", "ads.php", "steps"] {
                assert!(!text.contains(private), "{private}: {text}");
            }
        }
        assert!(
            !crate::Cli::try_parse_from(["rzn-browser", "run", "book", "search"])
                .unwrap()
                .agent_output
        );
    }

    #[tokio::test]
    async fn unsupported_commands_and_raw_output_overrides_fail_before_execution() {
        for (args, status) in [
            (
                vec!["browser", "targets", "--agent-output"],
                "unsupported_agent_command",
            ),
            (
                vec![
                    "run",
                    "book",
                    "get",
                    "--agent-output",
                    "--param",
                    "query=Book",
                    "--output-file",
                    "raw.json",
                ],
                "use_download_dir_or_redirect_json_stdout",
            ),
            (
                vec![
                    "run",
                    "book",
                    "get",
                    "--agent-output",
                    "--param",
                    "query=Book",
                    "--param",
                    "download_dir=raw",
                ],
                "use_download_dir_or_redirect_json_stdout",
            ),
            (
                vec![
                    "run",
                    "book",
                    "search",
                    "--agent-output",
                    "--param",
                    "query=Book",
                    "--download-dir",
                    "raw",
                ],
                "download_dir_requires_book_get",
            ),
        ] {
            let cli =
                crate::Cli::try_parse_from(std::iter::once("rzn-browser").chain(args)).unwrap();
            assert_eq!(
                handle(&cli.command).await,
                json!({"success": false, "status": status})
            );
        }
    }

    #[test]
    fn extraction_ignores_progress_and_unknown_provider_fields() {
        let payload = json!({"version": "rzn.run_result", "workflow_id": "book/search",
            "status": "succeeded", "output": {"result": {"layout_ok": true,
            "results": [{"title": "Book", "ext": "epub", "edition_url": "https://provider.test/edition",
                "md5": "secret", "new_provider_field": {"url": "https://provider.test"}}]}}});
        let log = format!("[STEP] {{not JSON}}\n{}\n[CLOSE] closing", payload);
        let selected = parse_result(&log, "book/search").unwrap();
        let projected = project_result(&selected, "book/search").unwrap();
        assert_eq!(
            projected["results"],
            json!([{"title": "Book", "ext": "epub"}])
        );
        assert!(!projected.to_string().contains("provider"));
        assert!(parse_result(&log, "book/get").is_err());
        assert!(parse_result("help text only", "book/search").is_err());
    }

    #[test]
    fn failures_keep_actionable_status_without_raw_errors() {
        assert_eq!(
            failure_code("SITE_CHALLENGE https://provider.test"),
            "human_action_required"
        );
        assert_eq!(
            failure_code("Open the existing Chrome profile"),
            "browser_unavailable"
        );
        assert_eq!(
            failure_code("timeout fetching https://provider.test"),
            "timeout"
        );
        assert_eq!(
            failure_code("GET https://provider.test failed"),
            "workflow_failed"
        );
        assert_eq!(
            project_result(
                &json!({"found": false, "status": "no_format_match",
            "error": "https://provider.test"}),
                "book/get"
            )
            .unwrap_err(),
            "no_format_match"
        );
        assert!(project_result(&json!({"layout_ok": false}), "book/search").is_err());
        let output = project_result(
            &json!({"found": true, "chosen": {"title": "Book"},
            "download_url": "https://provider.test", "attachment_urls": ["https://provider.test"]}),
            "book/get",
        )
        .unwrap();
        assert_eq!(
            output,
            json!({"success": true, "status": "resolved", "book": {"title": "Book"}})
        );
    }

    #[test]
    fn generic_results_keep_links_but_drop_navigation_progress() {
        let frame = json!({"version": "rzn.run_result", "workflow_id": "google/search",
            "status": "succeeded", "output": {"result": {"results": [
                {"title": "Example", "url": "https://example.org/"}]},
                "tab_id": 42, "url": "https://google.com/search?q=private"}});
        let log = format!("[STEP] result.url=https://google.com/\n{frame}\n[CLOSE] done");
        let result = parse_result(&log, "google/search").unwrap();
        assert_eq!(result["results"][0]["url"], "https://example.org/");
        assert!(!result.to_string().contains("google.com"));
        let later = frame.to_string().replace("Example", "Final");
        assert_eq!(
            parse_result(&format!("{log}\n{later}"), "google/search").unwrap()["results"][0]
                ["title"],
            "Final"
        );
        let mut array = frame;
        array["output"] = json!([{"url": "https://example.org/"}]);
        assert!(parse_result(&array.to_string(), "google/search")
            .unwrap()
            .is_array());
        array["status"] = json!("failed");
        assert_eq!(
            parse_result(&array.to_string(), "google/search").unwrap_err(),
            "workflow_failed"
        );
    }

    #[test]
    fn book_ids_are_private_expire_and_reject_path_traversal() {
        let scratch = Scratch::new().unwrap();
        let md5 = "51860b9694a6d153c4c305dcf3b9e10d";
        let params = vec![
            ("query".into(), "Book".into()),
            ("base_url".into(), "https://provider.test/".into()),
        ];
        let id = save_book(&scratch.0, &params, md5).unwrap();
        assert_ne!(id, md5);
        let mut record = load_book(&scratch.0, &id).unwrap();
        assert_eq!(record["md5"], md5);
        assert_eq!(
            serde_json::from_value::<Vec<(String, String)>>(record["params"].clone()).unwrap(),
            params
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(scratch.0.join(format!("{id}.json")))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert_eq!(
            load_book(&scratch.0, "../../private").unwrap_err(),
            "invalid_book_id"
        );
        assert_eq!(
            load_book(&scratch.0, &uuid::Uuid::new_v4().to_string()).unwrap_err(),
            "book_id_expired_search_again"
        );
        record["created"] = json!(0);
        fs::write(scratch.0.join(format!("{id}.json")), record.to_string()).unwrap();
        assert_eq!(
            load_book(&scratch.0, &id).unwrap_err(),
            "book_id_expired_search_again"
        );
    }

    #[tokio::test]
    async fn generic_help_and_catalog_keep_book_configuration_private() {
        for args in [
            vec!["workflow", "show", "google/search", "--agent-output"],
            vec!["workflow", "list", "--agent-output"],
        ] {
            let cli =
                crate::Cli::try_parse_from(std::iter::once("rzn-browser").chain(args)).unwrap();
            let result = handle(&cli.command).await;
            assert_eq!(result["success"], true, "{result}");
            assert!(!result.to_string().contains("libgen"));
        }
    }

    #[test]
    fn download_limits_and_agent_timeout_accept_large_files() {
        for (arguments, expected) in [
            (vec![], 1024),
            (vec!["--max-download-mib", "2048"], 2048),
            (vec!["--max-download-mib", "0"], 0),
        ] {
            let cli = crate::Cli::try_parse_from(
                ["rzn-browser", "run", "book/get", "--agent-output"]
                    .into_iter()
                    .chain(arguments),
            )
            .unwrap();
            let Commands::Run(args) = cli.command else {
                panic!("expected run")
            };
            assert_eq!(args.max_download_mib, expected);
        }
        assert!(crate::Cli::try_parse_from([
            "rzn-browser",
            "run",
            "book/get",
            "--max-download-mib",
            "18446744073709551615"
        ])
        .is_err());
        assert_eq!(run_timeout(&json!({}), false), Duration::from_secs(300));
        assert_eq!(run_timeout(&json!({}), true), Duration::from_secs(2100));
        assert_eq!(
            run_timeout(&json!({"runtime": {"timeout_ms": 420000}}), true),
            Duration::from_secs(2250)
        );
    }

    #[tokio::test]
    async fn receipt_verifies_bytes_and_never_publishes_raw_manifest_or_overwrites() {
        let private = Scratch::new().unwrap();
        let public = Scratch::new().unwrap();
        fs::create_dir(private.0.join("attachments")).unwrap();
        let file = private.0.join("attachments/book.epub");
        fs::write(&file, b"fixture book").unwrap();
        let (bytes, hash) = rzn_browser::asset_download::hash_existing(&file)
            .await
            .unwrap();
        let mut manifest = json!({"items": [{"kind": "attachment", "path": "attachments/book.epub",
            "url": "https://provider.test", "bytes": bytes, "sha256": hash}]});
        fs::write(private.0.join("manifest.json"), manifest.to_string()).unwrap();
        let receipt = publish_download(&private.0, &public.0).await.unwrap();
        assert_eq!(
            fs::read(receipt["path"].as_str().unwrap()).unwrap(),
            b"fixture book"
        );
        assert!(!receipt.to_string().contains("provider"));
        assert!(!public.0.join("manifest.json").exists());
        assert_eq!(
            publish_download(&private.0, &public.0).await.unwrap_err(),
            "destination_exists"
        );
        fs::write(&file, b"<!DOCTYPE html><html>source error</html>").unwrap();
        let (bytes, hash) = rzn_browser::asset_download::hash_existing(&file)
            .await
            .unwrap();
        manifest["items"][0]["bytes"] = json!(bytes);
        manifest["items"][0]["sha256"] = json!(hash);
        fs::write(private.0.join("manifest.json"), manifest.to_string()).unwrap();
        assert_eq!(
            publish_download(&private.0, &public.0).await.unwrap_err(),
            "download_failed"
        );
        fs::write(&file, b"corrupt").unwrap();
        assert_eq!(
            publish_download(&private.0, &public.0).await.unwrap_err(),
            "download_failed"
        );
        manifest["items"][0]["path"] = json!(receipt["path"]);
        fs::write(private.0.join("manifest.json"), manifest.to_string()).unwrap();
        assert_eq!(
            publish_download(&private.0, &public.0).await.unwrap_err(),
            "download_failed"
        );
        fs::write(
            private.0.join("manifest.json"),
            r#"{"items":[{"kind":"attachment","error":"https://provider.test"}]}"#,
        )
        .unwrap();
        assert_eq!(
            publish_download(&private.0, &public.0).await.unwrap_err(),
            "download_failed"
        );
    }
}
