use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as b64;
use base64::Engine;
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::write::FileOptions;

fn fixed_zip_date_time() -> zip::DateTime {
    zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0).unwrap_or_default()
}

#[derive(Parser, Debug)]
#[command(name = "rzn-plugin-devkit")]
#[command(
    about = "Build + verify signed RZN plugin bundles (plugin.json + plugin.sig + payload ZIP)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Generate a dev Ed25519 keypair (base64) under a directory
    Keygen(KeygenArgs),
    /// Build a signed plugin bundle ZIP from a config JSON
    Build(BuildArgs),
    /// Verify a bundle ZIP (signature + sha256 payload map)
    Verify(VerifyArgs),
}

#[derive(Parser, Debug)]
struct KeygenArgs {
    /// Output directory for ed25519.private + ed25519.public
    #[arg(long, default_value = ".secrets/plugin-signing")]
    out: String,
}

#[derive(Parser, Debug)]
struct BuildArgs {
    /// Path to plugin config JSON
    #[arg(long)]
    config: String,
    /// Target platform key (e.g. macos_universal, windows_x86_64)
    #[arg(long)]
    platform: String,
    /// Path to base64 Ed25519 secret key (32-byte seed or 64-byte nacl secretKey)
    #[arg(long)]
    key: String,
    /// Output directory (defaults to dist/plugins)
    #[arg(long, default_value = "dist/plugins")]
    out: String,
    /// Overwrite existing output directory for this plugin/version/platform
    #[arg(long, default_value_t = true)]
    clean: bool,
}

#[derive(Parser, Debug)]
struct VerifyArgs {
    /// Path to bundle ZIP
    #[arg(long)]
    zip: String,
    /// Path to base64 Ed25519 public key
    #[arg(long)]
    public: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BundleConfig {
    id: String,
    version: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    min_host_version: Option<String>,
    #[serde(default)]
    mcp_protocol_version: Option<String>,
    #[serde(default)]
    requires_entitlement: bool,
    #[serde(default)]
    required_product_ids: Vec<String>,
    #[serde(default)]
    platforms: Vec<String>,
    #[serde(default)]
    workers: Vec<WorkerConfig>,
    #[serde(default)]
    resources: Vec<ResourceConfig>,
    #[serde(default)]
    payloads: Vec<PayloadConfig>,
    #[serde(default)]
    shared_payloads: Vec<PayloadConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkerConfig {
    id: String,
    #[serde(default = "default_worker_kind")]
    kind: String,
    #[serde(default)]
    auto_start: bool,
    #[serde(default)]
    tools_namespace: Option<String>,
    entrypoints: BTreeMap<String, String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

fn default_worker_kind() -> String {
    "mcp_stdio".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ResourceConfig {
    Path(String),
    Spec(ResourceSpecConfig),
}

#[derive(Debug, Clone, Deserialize)]
struct ResourceSpecConfig {
    path: String,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct PayloadConfig {
    source: String,
    dest: String,
    #[serde(default)]
    platforms: Option<Vec<String>>,
    #[serde(default)]
    mode: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
struct PayloadFile {
    source: PathBuf,
    mode: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PluginManifestV1 {
    v: u32,
    id: String,
    version: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_host_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mcp_protocol_version: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    requires_entitlement: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    required_product_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    workers: Vec<WorkerSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    resources: Vec<ResourceSpec>,
    sha256: BTreeMap<String, String>,
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkerSpec {
    id: String,
    kind: String,
    #[serde(default, skip_serializing_if = "is_false")]
    auto_start: bool,
    entrypoint: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    env: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools_namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResourceSpec {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Keygen(args) => cmd_keygen(args),
        Commands::Build(args) => cmd_build(args),
        Commands::Verify(args) => cmd_verify(args),
    }
}

fn cmd_keygen(args: KeygenArgs) -> Result<()> {
    let out_dir = PathBuf::from(args.out).expanduser();
    fs::create_dir_all(&out_dir).context("create out dir")?;
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let priv_seed_b64 = b64.encode(signing_key.to_bytes());
    let pub_b64 = b64.encode(verifying_key.to_bytes());

    let priv_path = out_dir.join("ed25519.private");
    let pub_path = out_dir.join("ed25519.public");
    fs::write(&priv_path, format!("{}\n", priv_seed_b64)).context("write private key")?;
    fs::write(&pub_path, format!("{}\n", pub_b64)).context("write public key")?;

    println!("wrote {}", priv_path.display());
    println!("wrote {}", pub_path.display());
    Ok(())
}

fn cmd_build(args: BuildArgs) -> Result<()> {
    let config_path = PathBuf::from(args.config).expanduser();
    let platform = args.platform.trim().to_string();
    let key_path = PathBuf::from(args.key).expanduser();
    let out_dir = PathBuf::from(args.out).expanduser();

    let config: BundleConfig =
        serde_json::from_slice(&fs::read(&config_path).context("read config")?)
            .context("parse config JSON")?;
    validate_platform(&config, &platform)?;

    let payloads = collect_payloads(&config, &platform)?;
    let manifest = build_manifest(&config, &platform, &payloads)?;
    let manifest_bytes = serialize_manifest_bytes(&manifest)?;

    let signing_key = read_signing_key_b64(&key_path)?;
    let sig_b64 = sign_manifest_b64(&signing_key, &manifest_bytes);

    let staging = out_dir
        .join(&config.id)
        .join(&config.version)
        .join(&platform);
    if args.clean && staging.exists() {
        fs::remove_dir_all(&staging).context("clean staging dir")?;
    }
    fs::create_dir_all(&staging).context("create staging dir")?;

    let plugin_json_path = staging.join("plugin.json");
    let plugin_sig_path = staging.join("plugin.sig");
    fs::write(&plugin_json_path, &manifest_bytes).context("write plugin.json")?;
    fs::write(&plugin_sig_path, format!("{}\n", sig_b64)).context("write plugin.sig")?;

    let zip_name = format!("{}-{}-{}.zip", config.id, config.version, platform);
    let zip_path = staging.join(&zip_name);
    write_bundle_zip(&zip_path, &manifest_bytes, sig_b64.as_bytes(), &payloads)
        .context("write bundle zip")?;

    let zip_sha = sha256_hex(&fs::read(&zip_path).context("read zip")?);
    let sha_path = staging.join(format!("{}.sha256", zip_name));
    fs::write(&sha_path, format!("{}\n", zip_sha)).context("write zip sha256 file")?;

    println!("built {}", zip_path.display());
    println!("sha256 {}", zip_sha);
    Ok(())
}

fn cmd_verify(args: VerifyArgs) -> Result<()> {
    let zip_path = PathBuf::from(args.zip).expanduser();
    let public_key_path = PathBuf::from(args.public).expanduser();
    verify_bundle_zip(&zip_path, &public_key_path)?;
    println!("ok {}", zip_path.display());
    Ok(())
}

fn validate_platform(config: &BundleConfig, platform: &str) -> Result<()> {
    if !config.platforms.is_empty() && !config.platforms.iter().any(|p| p == platform) {
        bail!("platform {} not in config platforms", platform);
    }
    Ok(())
}

fn collect_payloads(
    config: &BundleConfig,
    platform: &str,
) -> Result<BTreeMap<String, PayloadFile>> {
    let mut out: BTreeMap<String, PayloadFile> = BTreeMap::new();
    for item in config.payloads.iter().chain(config.shared_payloads.iter()) {
        if let Some(platforms) = &item.platforms {
            if !platforms.iter().any(|p| p == platform) {
                continue;
            }
        }
        let source_raw = expand_env(&item.source)?;
        let source_path = PathBuf::from(source_raw)
            .expanduser()
            .canonicalize()
            .with_context(|| format!("resolve payload source {}", item.source))?;

        let dest_root = normalize_rel_path(&item.dest)?;
        if dest_root.is_empty() {
            bail!("payload dest missing for source {}", source_path.display());
        }
        let base_mode = parse_mode(&item.mode).context("parse payload mode")?;

        if source_path.is_dir() {
            // Used to validate that symlink targets do not escape the payload tree.
            let payload_root = source_path.clone();
            for entry in WalkDir::new(&source_path)
                .follow_links(false)
                .sort_by_file_name()
            {
                let entry = entry?;

                // Allow symlink *files* (common in python bundles). We embed the file bytes
                // of the resolved target, not symlink metadata, because the desktop host's
                // hardened ZIP extractor rejects symlinks.
                let ft = entry.file_type();
                if !(ft.is_file() || ft.is_symlink()) {
                    continue;
                }

                let rel = entry
                    .path()
                    .strip_prefix(&source_path)
                    .context("strip dir prefix")?;
                let rel = rel.to_string_lossy().replace('\\', "/");
                let dest =
                    normalize_rel_path(&format!("{}/{}", dest_root.trim_end_matches('/'), rel))?;

                let resolved_source = if ft.is_symlink() || is_symlink(&entry.path())? {
                    let real = entry
                        .path()
                        .canonicalize()
                        .with_context(|| format!("resolve symlink {}", entry.path().display()))?;
                    if !real.starts_with(&payload_root) {
                        bail!(
                            "symlink escapes payload root: {} -> {}",
                            entry.path().display(),
                            real.display()
                        );
                    }
                    if !real.is_file() {
                        bail!(
                            "symlink target is not a file: {} -> {}",
                            entry.path().display(),
                            real.display()
                        );
                    }
                    real
                } else {
                    entry.path().to_path_buf()
                };

                let mode = if base_mode != 0o644 {
                    base_mode
                } else {
                    file_mode(&resolved_source).unwrap_or(0o644)
                };

                insert_payload(&mut out, resolved_source, dest, mode)?;
            }
        } else {
            let resolved_source = if is_symlink(&source_path)? {
                let real = source_path
                    .canonicalize()
                    .with_context(|| format!("resolve symlink {}", source_path.display()))?;
                if !real.is_file() {
                    bail!(
                        "symlink payload target is not a file: {} -> {}",
                        source_path.display(),
                        real.display()
                    );
                }
                real
            } else {
                source_path
            };
            insert_payload(&mut out, resolved_source, dest_root, base_mode)?;
        }
    }
    Ok(out)
}

fn insert_payload(
    map: &mut BTreeMap<String, PayloadFile>,
    source: PathBuf,
    dest: String,
    mode: u32,
) -> Result<()> {
    if !is_safe_relative_path(&dest) {
        bail!("unsafe payload dest path {}", dest);
    }
    if map.contains_key(&dest) {
        bail!("duplicate payload path {}", dest);
    }
    map.insert(dest.clone(), PayloadFile { source, mode });
    Ok(())
}

fn build_manifest(
    config: &BundleConfig,
    platform: &str,
    payloads: &BTreeMap<String, PayloadFile>,
) -> Result<PluginManifestV1> {
    let mut sha256: BTreeMap<String, String> = BTreeMap::new();
    for (dest, payload) in payloads {
        let bytes = fs::read(&payload.source)
            .with_context(|| format!("read payload {}", payload.source.display()))?;
        sha256.insert(dest.clone(), sha256_hex(&bytes));
    }

    // Validate resource paths are present in payload map (either exact file or as a dir prefix).
    for res in &config.resources {
        let path = match res {
            ResourceConfig::Path(p) => p.clone(),
            ResourceConfig::Spec(spec) => spec.path.clone(),
        };
        let path = normalize_rel_path(&path)?;
        if path.is_empty() || !is_safe_relative_path(&path) {
            bail!("unsafe resource path {}", path);
        }
        if sha256.contains_key(&path) {
            continue;
        }
        let prefix = format!("{}/", path.trim_end_matches('/'));
        if !sha256.keys().any(|k| k.starts_with(&prefix)) {
            bail!("resource path missing from payloads: {}", path);
        }
    }

    let mut workers_out: Vec<WorkerSpec> = Vec::new();
    for w in &config.workers {
        let entry = w.entrypoints.get(platform).ok_or_else(|| {
            anyhow!(
                "missing entrypoint for worker {} platform {}",
                w.id,
                platform
            )
        })?;
        let entry = normalize_rel_path(entry)?;
        if entry.is_empty() || !is_safe_relative_path(&entry) {
            bail!("unsafe worker entrypoint {}", entry);
        }
        if !sha256.contains_key(&entry) {
            bail!("sha256 missing for worker entrypoint {}", entry);
        }
        let mut entrypoint = BTreeMap::new();
        entrypoint.insert(platform.to_string(), entry);
        workers_out.push(WorkerSpec {
            id: w.id.clone(),
            kind: w.kind.clone(),
            auto_start: w.auto_start,
            entrypoint,
            args: w.args.clone(),
            env: w.env.clone(),
            tools_namespace: w.tools_namespace.clone(),
        });
    }

    let mut resources_out: Vec<ResourceSpec> = Vec::new();
    for res in &config.resources {
        match res {
            ResourceConfig::Path(p) => resources_out.push(ResourceSpec {
                path: normalize_rel_path(p)?,
                sha256: None,
            }),
            ResourceConfig::Spec(spec) => resources_out.push(ResourceSpec {
                path: normalize_rel_path(&spec.path)?,
                sha256: spec.sha256.clone(),
            }),
        }
    }

    let manifest = PluginManifestV1 {
        v: 1,
        id: config.id.clone(),
        version: config.version.clone(),
        name: config.name.clone(),
        description: config.description.clone(),
        min_host_version: config.min_host_version.clone(),
        mcp_protocol_version: config.mcp_protocol_version.clone(),
        requires_entitlement: config.requires_entitlement,
        required_product_ids: config.required_product_ids.clone(),
        workers: workers_out,
        resources: resources_out,
        sha256,
    };
    validate_manifest_basic(&manifest)?;
    Ok(manifest)
}

fn validate_manifest_basic(manifest: &PluginManifestV1) -> Result<()> {
    if manifest.v != 1 {
        bail!("unsupported manifest version {}", manifest.v);
    }
    if !is_valid_id(&manifest.id) {
        bail!("invalid id");
    }
    if !is_valid_version(&manifest.version) {
        bail!("invalid version");
    }
    if manifest.name.trim().is_empty() {
        bail!("name is required");
    }
    if manifest.sha256.is_empty() {
        bail!("sha256 map is required");
    }
    if manifest.sha256.contains_key("plugin.json") {
        bail!("sha256 map must exclude plugin.json");
    }
    if manifest.sha256.contains_key("plugin.sig") {
        bail!("sha256 map must exclude plugin.sig");
    }
    if manifest.requires_entitlement && manifest.required_product_ids.is_empty() {
        bail!("required_product_ids is required when requires_entitlement=true");
    }
    for worker in &manifest.workers {
        if worker.id.trim().is_empty() || !is_valid_id(&worker.id) {
            bail!("worker id is required");
        }
        if worker.entrypoint.is_empty() {
            bail!("worker entrypoint is required");
        }
        for path in worker.entrypoint.values() {
            if !is_safe_relative_path(path) {
                bail!("unsafe entrypoint path {}", path);
            }
            if !manifest.sha256.contains_key(path) {
                bail!("sha256 missing for entrypoint {}", path);
            }
        }
    }
    for res in &manifest.resources {
        if !is_safe_relative_path(&res.path) {
            bail!("unsafe resource path {}", res.path);
        }
        if let Some(hash) = &res.sha256 {
            let map_hash = manifest
                .sha256
                .get(&res.path)
                .ok_or_else(|| anyhow!("sha256 missing for resource {}", res.path))?;
            if map_hash != hash {
                bail!("sha256 mismatch for resource {}", res.path);
            }
        } else if !manifest.sha256.contains_key(&res.path) {
            let prefix = format!("{}/", res.path.trim_end_matches('/'));
            if !manifest.sha256.keys().any(|k| k.starts_with(&prefix)) {
                bail!("sha256 missing for resource {}", res.path);
            }
        }
    }
    for key in manifest.sha256.keys() {
        if !is_safe_relative_path(key) {
            bail!("unsafe sha256 path {}", key);
        }
    }
    Ok(())
}

fn is_valid_id(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 {
        return false;
    }
    value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn is_valid_version(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 {
        return false;
    }
    value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '+')
}

fn is_safe_relative_path(path: &str) -> bool {
    if path.is_empty() || path.contains('\\') {
        return false;
    }
    let p = Path::new(path);
    if p.is_absolute() {
        return false;
    }
    for comp in p.components() {
        match comp {
            std::path::Component::ParentDir => return false,
            std::path::Component::Prefix(_) => return false,
            _ => {}
        }
    }
    true
}

fn normalize_rel_path(raw: &str) -> Result<String> {
    let mut v = raw.trim().replace('\\', "/");
    while v.starts_with("./") {
        v = v.trim_start_matches("./").to_string();
    }
    v = v.trim_start_matches('/').to_string();
    Ok(v)
}

fn expand_env(input: &str) -> Result<String> {
    if !input.contains('$') {
        return Ok(input.to_string());
    }
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('{') => {
                let _ = chars.next();
                let mut name = String::new();
                while let Some(ch) = chars.next() {
                    if ch == '}' {
                        break;
                    }
                    name.push(ch);
                }
                if name.is_empty() {
                    bail!("empty env var in {}", input);
                }
                let val = std::env::var(&name)
                    .with_context(|| format!("unresolved env in path: {}", input))?;
                out.push_str(&val);
            }
            Some(ch) if ch.is_ascii_alphanumeric() || ch == '_' => {
                let mut name = String::new();
                while let Some(ch2) = chars.peek().copied() {
                    if ch2.is_ascii_alphanumeric() || ch2 == '_' {
                        name.push(ch2);
                        let _ = chars.next();
                    } else {
                        break;
                    }
                }
                let val = std::env::var(&name)
                    .with_context(|| format!("unresolved env in path: {}", input))?;
                out.push_str(&val);
            }
            Some('$') => {
                let _ = chars.next();
                out.push('$');
            }
            _ => bail!("unresolved env in path: {}", input),
        }
    }
    if out.contains('$') {
        bail!("unresolved env in path: {}", input);
    }
    Ok(out)
}

fn parse_mode(raw: &Option<serde_json::Value>) -> Result<u32> {
    let Some(v) = raw else {
        return Ok(0o644);
    };
    if let Some(n) = v.as_u64() {
        return Ok(n as u32);
    }
    if let Some(s) = v.as_str() {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Ok(0o644);
        }
        return u32::from_str_radix(trimmed, 8).context("parse octal mode");
    }
    Ok(0o644)
}

fn file_mode(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .ok()
            .map(|m| m.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

fn is_symlink(path: &Path) -> Result<bool> {
    Ok(fs::symlink_metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .file_type()
        .is_symlink())
}

fn serialize_manifest_bytes(manifest: &PluginManifestV1) -> Result<Vec<u8>> {
    let v = serde_json::to_value(manifest).context("serialize plugin.json to value")?;
    let v = canonicalize_json_value(v);
    let mut out = serde_json::to_vec(&v).context("serialize plugin.json")?;
    out.push(b'\n');
    Ok(out)
}

fn canonicalize_json_value(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(map) => {
            let mut entries: Vec<(String, JsonValue)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = serde_json::Map::new();
            for (k, v) in entries {
                out.insert(k, canonicalize_json_value(v));
            }
            JsonValue::Object(out)
        }
        JsonValue::Array(arr) => {
            JsonValue::Array(arr.into_iter().map(canonicalize_json_value).collect())
        }
        other => other,
    }
}

fn read_signing_key_b64(path: &Path) -> Result<SigningKey> {
    let s = fs::read_to_string(path).with_context(|| format!("read key {}", path.display()))?;
    let bytes = b64
        .decode(s.trim())
        .with_context(|| format!("base64 decode key {}", path.display()))?;
    match bytes.len() {
        32 => Ok(SigningKey::from_bytes(&bytes.try_into().unwrap())),
        64 => {
            let seed: [u8; 32] = bytes[0..32].try_into().unwrap();
            Ok(SigningKey::from_bytes(&seed))
        }
        other => bail!("invalid Ed25519 key length: {}", other),
    }
}

fn read_public_key_b64(path: &Path) -> Result<VerifyingKey> {
    let s =
        fs::read_to_string(path).with_context(|| format!("read public key {}", path.display()))?;
    let bytes = b64
        .decode(s.trim())
        .with_context(|| format!("base64 decode public key {}", path.display()))?;
    if bytes.len() != 32 {
        bail!("invalid Ed25519 public key length: {}", bytes.len());
    }
    let pk: [u8; 32] = bytes.try_into().unwrap();
    Ok(VerifyingKey::from_bytes(&pk)?)
}

fn sign_manifest_b64(signing_key: &SigningKey, message: &[u8]) -> String {
    let sig: Signature = signing_key.sign(message);
    b64.encode(sig.to_bytes())
}

fn write_bundle_zip(
    zip_path: &Path,
    plugin_json_bytes: &[u8],
    plugin_sig_b64_bytes: &[u8],
    payloads: &BTreeMap<String, PayloadFile>,
) -> Result<()> {
    let file =
        fs::File::create(zip_path).with_context(|| format!("create zip {}", zip_path.display()))?;
    let mut zw = zip::ZipWriter::new(file);

    let fixed_dt = fixed_zip_date_time();
    let opts = FileOptions::<()>::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(fixed_dt)
        .unix_permissions(0o644);
    zw.start_file("plugin.json", opts)?;
    zw.write_all(plugin_json_bytes)?;

    zw.start_file("plugin.sig", opts)?;
    zw.write_all(plugin_sig_b64_bytes)?;
    if !plugin_sig_b64_bytes.ends_with(b"\n") {
        zw.write_all(b"\n")?;
    }

    for (dest, payload) in payloads {
        let opts = FileOptions::<()>::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(fixed_dt)
            .unix_permissions(payload.mode);
        zw.start_file(dest, opts)?;
        let bytes = fs::read(&payload.source)
            .with_context(|| format!("read payload {}", payload.source.display()))?;
        zw.write_all(&bytes)?;
    }
    zw.finish()?;
    Ok(())
}

fn verify_bundle_zip(zip_path: &Path, public_key_path: &Path) -> Result<()> {
    let pk = read_public_key_b64(public_key_path)?;
    let file =
        fs::File::open(zip_path).with_context(|| format!("open zip {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("read zip archive")?;

    let mut manifest_bytes: Option<Vec<u8>> = None;
    let mut sig_b64: Option<String> = None;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        match entry.name() {
            "plugin.json" => {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                manifest_bytes = Some(buf);
            }
            "plugin.sig" => {
                let mut s = String::new();
                entry.read_to_string(&mut s)?;
                sig_b64 = Some(s.trim().to_string());
            }
            _ => {}
        }
    }
    let manifest_bytes = manifest_bytes.ok_or_else(|| anyhow!("plugin.json missing"))?;
    let sig_b64 = sig_b64.ok_or_else(|| anyhow!("plugin.sig missing"))?;
    let sig_bytes = b64
        .decode(sig_b64.trim())
        .context("base64 decode signature")?;
    if sig_bytes.len() != 64 {
        bail!("invalid Ed25519 signature length: {}", sig_bytes.len());
    }
    let sig_arr: [u8; 64] = sig_bytes.try_into().unwrap();
    let sig = Signature::from_bytes(&sig_arr);
    pk.verify_strict(&manifest_bytes, &sig)
        .context("signature verification failed")?;

    let manifest: PluginManifestV1 =
        serde_json::from_slice(&manifest_bytes).context("parse plugin.json")?;
    validate_manifest_basic(&manifest)?;

    // Verify sha256 map against ZIP payload bytes.
    let allow: BTreeSet<String> = manifest.sha256.keys().cloned().collect();
    for key in &allow {
        let mut entry = archive
            .by_name(key)
            .with_context(|| format!("missing payload file in zip: {}", key))?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        let actual = sha256_hex(&buf);
        let expected = manifest
            .sha256
            .get(key)
            .ok_or_else(|| anyhow!("sha256 missing for {}", key))?
            .to_lowercase();
        if actual != expected {
            bail!(
                "sha256 mismatch for {}: expected {}, got {}",
                key,
                expected,
                actual
            );
        }
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

trait ExpandUser {
    fn expanduser(self) -> Self;
}

impl ExpandUser for PathBuf {
    fn expanduser(self) -> Self {
        let s = self.to_string_lossy().to_string();
        PathBuf::from(expanduser_str(&s))
    }
}

impl ExpandUser for &Path {
    fn expanduser(self) -> Self {
        self
    }
}

fn expanduser_str(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs_home() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    if s == "~" {
        if let Some(home) = dirs_home() {
            return home.to_string_lossy().to_string();
        }
    }
    s.to_string()
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Result<Self> {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), unique));
            fs::create_dir_all(&path)?;
            Ok(Self { path })
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_file(path: &Path, contents: &[u8]) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents)?;
        Ok(())
    }

    #[test]
    fn directory_resources_and_examples_are_hashed_into_manifest() -> Result<()> {
        let temp = TempDirGuard::new("rzn-plugin-devkit-test")?;

        let worker_bin = temp.path.join("artifacts/rzn-browser-worker");
        let native_host_bin = temp.path.join("artifacts/rzn-native-host");
        let system_meta = temp
            .path
            .join("resources/systems/browser_automation/system.metadata.yaml");
        let example_workflow = temp
            .path
            .join("examples/browser_automation/open_page_get_title.json");

        write_file(&worker_bin, b"worker-binary")?;
        write_file(&native_host_bin, b"native-host-binary")?;
        write_file(
            &system_meta,
            b"version: 1\nsystem:\n  id: browser_automation\n",
        )?;
        write_file(&example_workflow, br#"{"id":"open_page_get_title"}"#)?;

        let config = BundleConfig {
            id: "rzn-browser".to_string(),
            version: "0.1.0".to_string(),
            name: "Browser Tools".to_string(),
            description: Some("Browser automation worker + native host".to_string()),
            min_host_version: None,
            mcp_protocol_version: Some("2025-06-18".to_string()),
            requires_entitlement: false,
            required_product_ids: Vec::new(),
            platforms: vec!["macos_universal".to_string()],
            workers: vec![WorkerConfig {
                id: "worker".to_string(),
                kind: "mcp_stdio".to_string(),
                auto_start: false,
                tools_namespace: Some("plugin:rzn-browser".to_string()),
                entrypoints: BTreeMap::from([(
                    "macos_universal".to_string(),
                    "bin/macos/universal/rzn-browser-worker".to_string(),
                )]),
                args: Vec::new(),
                env: BTreeMap::new(),
            }],
            resources: vec![ResourceConfig::Path(
                "resources/systems/browser_automation".to_string(),
            )],
            payloads: vec![
                PayloadConfig {
                    source: worker_bin.display().to_string(),
                    dest: "bin/macos/universal/rzn-browser-worker".to_string(),
                    platforms: Some(vec!["macos_universal".to_string()]),
                    mode: Some(JsonValue::String("755".to_string())),
                },
                PayloadConfig {
                    source: native_host_bin.display().to_string(),
                    dest: "bin/macos/universal/rzn-native-host".to_string(),
                    platforms: Some(vec!["macos_universal".to_string()]),
                    mode: Some(JsonValue::String("755".to_string())),
                },
            ],
            shared_payloads: vec![
                PayloadConfig {
                    source: temp
                        .path
                        .join("resources/systems/browser_automation")
                        .display()
                        .to_string(),
                    dest: "resources/systems/browser_automation".to_string(),
                    platforms: None,
                    mode: None,
                },
                PayloadConfig {
                    source: temp
                        .path
                        .join("examples/browser_automation")
                        .display()
                        .to_string(),
                    dest: "examples/browser_automation".to_string(),
                    platforms: None,
                    mode: None,
                },
            ],
        };

        let payloads = collect_payloads(&config, "macos_universal")?;
        assert!(payloads.contains_key("bin/macos/universal/rzn-browser-worker"));
        assert!(payloads.contains_key("bin/macos/universal/rzn-native-host"));
        assert!(payloads.contains_key("resources/systems/browser_automation/system.metadata.yaml"));
        assert!(payloads.contains_key("examples/browser_automation/open_page_get_title.json"));

        let manifest = build_manifest(&config, "macos_universal", &payloads)?;
        assert_eq!(manifest.resources.len(), 1);
        assert_eq!(
            manifest.resources[0].path,
            "resources/systems/browser_automation"
        );
        assert!(manifest
            .sha256
            .contains_key("resources/systems/browser_automation/system.metadata.yaml"));
        assert!(manifest
            .sha256
            .contains_key("examples/browser_automation/open_page_get_title.json"));

        Ok(())
    }
}
