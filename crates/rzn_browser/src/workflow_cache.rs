//! On-device workflow manifest cache (FLA-T-0004).
//!
//! Content-hash-addressed storage under the runtime dir with fetch-on-miss and
//! garbage collection. Jobs reference `(workflow_id, content_hash)`; the cache
//! guarantees the device runs exactly the bytes the server dispatched by
//! re-serializing every fetched manifest compactly and verifying its sha256
//! matches the requested hash before it is trusted or stored.
//!
//! Layout: `<root>/<workflow_id>/<content_hash>.json`, where `workflow_id` is
//! restricted to `[a-z0-9_-]` (defense-in-depth against path traversal) and
//! `content_hash` is a 64-char lowercase sha256 hex digest.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use serde_json::Value;
use sha2::{Digest, Sha256};

use rzn_contracts::fleet_v1::FleetWorkflowFetchResponseV1;

/// Content hash of a manifest: sha256 hex over the compact `serde_json`
/// serialization of the value with object keys recursively sorted (byte order).
///
/// This is the cross-repo integrity contract (backend FLT-T-0004). The value is
/// explicitly canonicalized — every object's keys sorted ascending by byte
/// order, recursing through nested objects and arrays — before serialization, so
/// the digest is independent of serde_json's `preserve_order` feature on either
/// side. (Without `preserve_order` a `Map` is a `BTreeMap` and already sorts;
/// with it a `Map` is an `IndexMap` that preserves insertion order. The backend
/// enables `preserve_order`, so relying on serde_json's own ordering would make
/// the two repos disagree — the explicit sort below is what keeps them equal.)
pub fn manifest_content_hash(value: &Value) -> String {
    // `to_vec` over a already-parsed `Value` is effectively infallible; degrade
    // to hashing empty bytes rather than panic to honor the no-panic constraint.
    let bytes = serde_json::to_vec(&canonicalize(value)).unwrap_or_default();
    hex::encode(Sha256::digest(&bytes))
}

/// Rebuild `value` with every object's keys sorted ascending by byte order,
/// recursing into nested objects and array elements. This is the canonical form
/// the content hash is taken over; making it explicit (rather than leaning on
/// serde_json's default `BTreeMap` ordering) keeps the digest stable across
/// repos regardless of whether either enables the `preserve_order` feature.
fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            // `String`'s `Ord` compares the underlying UTF-8 bytes, i.e. byte
            // order. Sorting keys before re-inserting normalizes any
            // insertion-order-preserving map (e.g. `preserve_order`'s IndexMap).
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                sorted.insert(key.clone(), canonicalize(&map[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

/// Errors surfaced by [`WorkflowCache`].
#[derive(Debug)]
pub enum CacheError {
    /// `workflow_id` contained characters outside `[a-z0-9_-]`.
    InvalidWorkflowId(String),
    /// `content_hash` was not a 64-char lowercase hex string.
    InvalidContentHash(String),
    /// The fetched manifest did not hash to the requested content hash. This is
    /// the fleet integrity boundary: such a manifest is never stored.
    HashMismatch {
        workflow_id: String,
        requested: String,
        computed: String,
    },
    /// The underlying [`ManifestFetcher`] failed to retrieve the manifest.
    Fetch(FetchError),
    /// A filesystem operation failed.
    Io(io::Error),
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheError::InvalidWorkflowId(id) => {
                write!(f, "invalid workflow_id (must match [a-z0-9_-]): {id:?}")
            }
            CacheError::InvalidContentHash(hash) => {
                write!(f, "invalid content_hash (must be 64-char lowercase hex): {hash:?}")
            }
            CacheError::HashMismatch {
                workflow_id,
                requested,
                computed,
            } => write!(
                f,
                "manifest hash mismatch for {workflow_id}: requested {requested}, computed {computed}"
            ),
            CacheError::Fetch(err) => write!(f, "manifest fetch failed: {err}"),
            CacheError::Io(err) => write!(f, "workflow cache io error: {err}"),
        }
    }
}

impl std::error::Error for CacheError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CacheError::Fetch(err) => Some(err),
            CacheError::Io(err) => Some(err),
            _ => None,
        }
    }
}

/// Failure to fetch a manifest from the backend (transport, status, or decode).
#[derive(Debug)]
pub struct FetchError {
    message: String,
}

impl FetchError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FetchError {}

/// Statistics returned from a [`WorkflowCache::gc`] pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GcStats {
    /// Manifest files examined across every workflow_id subdirectory.
    pub scanned: usize,
    /// Manifest files deleted (over the keep-count or over the age limit).
    pub deleted: usize,
    /// Manifest files retained (including any whose deletion failed).
    pub kept: usize,
}

/// Retrieves workflow manifests from the backend on cache miss.
#[async_trait]
pub trait ManifestFetcher: Send + Sync {
    async fn fetch(
        &self,
        workflow_id: &str,
        content_hash: &str,
    ) -> Result<FleetWorkflowFetchResponseV1, FetchError>;
}

/// Content-hash-addressed on-device manifest cache.
pub struct WorkflowCache {
    root: PathBuf,
}

/// Uniquifies temp filenames for atomic writes within a single process.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

impl WorkflowCache {
    /// Create a cache rooted at `root` (typically `<runtime>/workflow_cache`).
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The cache root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve `(workflow_id, content_hash)` to a manifest value.
    ///
    /// Cache hit: read, verify integrity, touch mtime, return. Cache miss (or a
    /// corrupt cached file, which is evicted): fetch via `fetcher`, verify the
    /// served manifest hashes to `content_hash`, store atomically, return. A
    /// hash mismatch on fetch is a hard error and is never stored.
    pub async fn get(
        &self,
        workflow_id: &str,
        content_hash: &str,
        fetcher: &dyn ManifestFetcher,
    ) -> Result<Value, CacheError> {
        let id = sanitize_workflow_id(workflow_id)?;
        let hash = validate_content_hash(content_hash)?;
        let path = self.path_for(&id, &hash);

        // Cache hit: only trust a file whose parsed value still hashes to the
        // requested digest. Anything else (unreadable, unparseable, bit-rot,
        // tampered) is treated as corrupt: evict and refetch exactly once.
        if path.exists() {
            match read_verified(&path, &hash) {
                Ok(value) => {
                    touch_mtime(&path);
                    return Ok(value);
                }
                Err(_) => {
                    let _ = fs::remove_file(&path);
                }
            }
        }

        // Miss (or evicted corrupt): fetch, verify, store.
        let response = fetcher
            .fetch(workflow_id, content_hash)
            .await
            .map_err(CacheError::Fetch)?;
        let computed = manifest_content_hash(&response.manifest);
        if computed != hash {
            return Err(CacheError::HashMismatch {
                workflow_id: id,
                requested: hash,
                computed,
            });
        }
        self.store_atomic(&path, &response.manifest)?;
        Ok(response.manifest)
    }

    /// Garbage-collect cached manifests.
    ///
    /// For each workflow_id, keep the newest `keep_per_id` versions by mtime and
    /// delete the rest; additionally delete any version older than
    /// `max_age_days` regardless of the keep count (`max_age_days == 0` disables
    /// the age rule). Non-`.json` files and unreadable entries are ignored, so
    /// stray/temp files left by concurrent writers do not perturb GC.
    pub fn gc(&self, keep_per_id: usize, max_age_days: u64) -> GcStats {
        let mut stats = GcStats::default();
        let max_age = Duration::from_secs(max_age_days.saturating_mul(86_400));

        let id_entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            // No cache directory yet (or unreadable) -> nothing to collect.
            Err(_) => return stats,
        };

        for id_entry in id_entries.flatten() {
            let id_path = id_entry.path();
            if !id_path.is_dir() {
                continue;
            }

            let mut versions: Vec<(PathBuf, SystemTime)> = Vec::new();
            let version_entries = match fs::read_dir(&id_path) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in version_entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue; // ignore stray/temp files
                }
                let metadata = match entry.metadata() {
                    Ok(metadata) if metadata.is_file() => metadata,
                    _ => continue,
                };
                let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                versions.push((path, mtime));
            }

            stats.scanned += versions.len();
            // Newest first.
            versions.sort_by(|a, b| b.1.cmp(&a.1));

            for (index, (path, mtime)) in versions.iter().enumerate() {
                let over_count = index >= keep_per_id;
                let over_age =
                    max_age_days > 0 && mtime.elapsed().map(|age| age >= max_age).unwrap_or(false);
                if over_count || over_age {
                    if fs::remove_file(path).is_ok() {
                        stats.deleted += 1;
                    } else {
                        stats.kept += 1;
                    }
                } else {
                    stats.kept += 1;
                }
            }
        }

        stats
    }

    fn path_for(&self, sanitized_id: &str, hash: &str) -> PathBuf {
        self.root.join(sanitized_id).join(format!("{hash}.json"))
    }

    /// Write the canonical (keys-sorted, compact) serialization of `manifest` to
    /// `path` atomically (temp file in the same directory, then rename). Storing
    /// the canonical bytes keeps the on-disk file byte-identical to what the
    /// hit-path re-hash canonicalizes, so re-verification stays consistent.
    fn store_atomic(&self, path: &Path, manifest: &Value) -> Result<(), CacheError> {
        let parent = path.parent().ok_or_else(|| {
            CacheError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "manifest path has no parent directory",
            ))
        })?;
        fs::create_dir_all(parent).map_err(CacheError::Io)?;

        let bytes = serde_json::to_vec(&canonicalize(manifest))
            .map_err(|err| CacheError::Io(io::Error::new(io::ErrorKind::InvalidData, err)))?;

        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = parent.join(format!(".tmp-{}-{}.json", std::process::id(), seq));
        fs::write(&tmp, &bytes).map_err(CacheError::Io)?;
        match fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            Err(err) => {
                let _ = fs::remove_file(&tmp);
                Err(CacheError::Io(err))
            }
        }
    }
}

/// Read a cached manifest and confirm its parsed value still hashes to
/// `expected_hash`. Any failure (io, parse, mismatch) is reported so the caller
/// can evict and refetch.
fn read_verified(path: &Path, expected_hash: &str) -> Result<Value, CacheError> {
    let bytes = fs::read(path).map_err(CacheError::Io)?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|err| CacheError::Io(io::Error::new(io::ErrorKind::InvalidData, err)))?;
    if manifest_content_hash(&value) == expected_hash {
        Ok(value)
    } else {
        Err(CacheError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "cached manifest hash does not match its filename",
        )))
    }
}

/// Validate that `id` is a safe cache subdirectory name. Rejects (rather than
/// rewrites) anything outside `[a-z0-9_-]`, so `../`, `/`, `.`, and uppercase
/// are all refused.
fn sanitize_workflow_id(id: &str) -> Result<String, CacheError> {
    let ok = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if ok {
        Ok(id.to_string())
    } else {
        Err(CacheError::InvalidWorkflowId(id.to_string()))
    }
}

/// Validate that `hash` is a 64-char lowercase sha256 hex digest.
fn validate_content_hash(hash: &str) -> Result<String, CacheError> {
    let ok = hash.len() == 64
        && hash
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    if ok {
        Ok(hash.to_string())
    } else {
        Err(CacheError::InvalidContentHash(hash.to_string()))
    }
}

/// Best-effort bump of a file's mtime to now so GC keep-newest reflects usage.
/// Failure is non-fatal (GC ordering is merely slightly stale).
fn touch_mtime(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        if let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) {
            // Null `times` sets both atime and mtime to the current time.
            unsafe {
                libc::utimes(c_path.as_ptr(), std::ptr::null());
            }
        }
    }
    #[cfg(not(unix))]
    {
        // No portable stdlib mtime setter; rewrite the same bytes to bump mtime.
        if let Ok(bytes) = fs::read(path) {
            let _ = fs::write(path, bytes);
        }
    }
}

/// Real [`ManifestFetcher`]: `GET {server}/v1/fleet/workflows/{id}/{hash}` with
/// a device-token bearer credential, decoding [`FleetWorkflowFetchResponseV1`].
pub struct HttpManifestFetcher {
    client: reqwest::Client,
    server_url: String,
    device_token: String,
}

impl HttpManifestFetcher {
    /// Build a fetcher against `server_url` (base, e.g. `https://cloud.example`)
    /// authenticating with `device_token`.
    pub fn new(server_url: impl Into<String>, device_token: impl Into<String>) -> Self {
        Self::with_client(reqwest::Client::new(), server_url, device_token)
    }

    /// As [`HttpManifestFetcher::new`] but with a caller-provided client (shared
    /// pools, custom timeouts).
    pub fn with_client(
        client: reqwest::Client,
        server_url: impl Into<String>,
        device_token: impl Into<String>,
    ) -> Self {
        let server_url = server_url.into().trim_end_matches('/').to_string();
        Self {
            client,
            server_url,
            device_token: device_token.into(),
        }
    }
}

#[async_trait]
impl ManifestFetcher for HttpManifestFetcher {
    async fn fetch(
        &self,
        workflow_id: &str,
        content_hash: &str,
    ) -> Result<FleetWorkflowFetchResponseV1, FetchError> {
        let url = format!(
            "{}/v1/fleet/workflows/{}/{}",
            self.server_url, workflow_id, content_hash
        );
        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.device_token)
            .send()
            .await
            .map_err(|err| FetchError::new(format!("request to {url} failed: {err}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(FetchError::new(format!(
                "unexpected status {status} fetching {url}"
            )));
        }
        response
            .json::<FleetWorkflowFetchResponseV1>()
            .await
            .map_err(|err| FetchError::new(format!("failed to decode manifest from {url}: {err}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Mock fetcher that counts calls and always serves a fixed manifest,
    /// echoing the requested id/hash into the response envelope.
    struct MockFetcher {
        manifest: Value,
        calls: Arc<AtomicUsize>,
    }

    impl MockFetcher {
        fn new(manifest: Value) -> (Self, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    manifest,
                    calls: calls.clone(),
                },
                calls,
            )
        }
    }

    #[async_trait]
    impl ManifestFetcher for MockFetcher {
        async fn fetch(
            &self,
            workflow_id: &str,
            content_hash: &str,
        ) -> Result<FleetWorkflowFetchResponseV1, FetchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(FleetWorkflowFetchResponseV1 {
                workflow_id: workflow_id.to_string(),
                content_hash: content_hash.to_string(),
                manifest: self.manifest.clone(),
            })
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock drift")
            .as_nanos();
        std::env::temp_dir().join(format!("rzn-workflow-cache-{name}-{unique}"))
    }

    /// Return a valid-but-different 64-char hex hash (flip the first nibble).
    fn other_valid_hash(hash: &str) -> String {
        let mut chars: Vec<char> = hash.chars().collect();
        chars[0] = if chars[0] == 'a' { 'b' } else { 'a' };
        chars.into_iter().collect()
    }

    #[cfg(unix)]
    fn set_mtime_secs_ago(path: &Path, secs_ago: u64) {
        use std::os::unix::ffi::OsStrExt;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock drift")
            .as_secs() as i64;
        let target = now - secs_ago as i64;
        let times = [
            libc::timeval {
                tv_sec: target as libc::time_t,
                tv_usec: 0,
            },
            libc::timeval {
                tv_sec: target as libc::time_t,
                tv_usec: 0,
            },
        ];
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        unsafe {
            libc::utimes(c_path.as_ptr(), times.as_ptr());
        }
    }

    // A4: the shared algorithm is pinned against an out-of-band sha256 of a
    // known compact JSON literal. The sha256 of the sorted compact form
    // `{"a":1,"b":[2,3],"z":"x"}` was computed independently:
    //   printf '%s' '{"a":1,"b":[2,3],"z":"x"}' | shasum -a 256
    // Keys are supplied here out of order to also prove sort+compaction.
    #[test]
    fn manifest_content_hash_matches_pinned_fixture() {
        const PINNED: &str = "f140e20a25515cdf131ffbf57b439d8e7ee0e577af99ae7d78ef4b98aec2c08b";
        let value: Value = serde_json::from_str(r#"{"z":"x","a":1,"b":[2,3]}"#).unwrap();
        assert_eq!(manifest_content_hash(&value), PINNED);
        // Length/charset are the on-disk filename contract.
        assert_eq!(PINNED.len(), 64);
        assert!(validate_content_hash(&manifest_content_hash(&value)).is_ok());
    }

    // A4b: logically equal manifests hash identically regardless of the order
    // their object keys were inserted, including nested objects.
    //
    // Why the explicit sort in `canonicalize` is load-bearing: on THIS workspace
    // serde_json's `Map` is a `BTreeMap`, so key order is already discarded when
    // the maps below are built — this assertion would pass even without any
    // canonicalization here. The sibling backend, however, builds serde_json
    // with `preserve_order` (Map = IndexMap), where the differing insertion
    // orders below WOULD survive into the serialization and produce diverging
    // digests. The recursive key-sort is what makes the two repos agree
    // (cross-repo contract, backend FLT-T-0004); this test guards against a
    // regression that drops the sort or flips `preserve_order` on here.
    #[test]
    fn manifest_content_hash_is_key_order_independent() {
        use serde_json::Map;

        // Same logical manifest, keys inserted in opposite orders at both levels.
        let mut inner_a = Map::new();
        inner_a.insert("y".to_string(), Value::from(2));
        inner_a.insert("x".to_string(), Value::from(1));
        let mut outer_a = Map::new();
        outer_a.insert("z".to_string(), Value::from("last"));
        outer_a.insert("nested".to_string(), Value::Object(inner_a));
        outer_a.insert("a".to_string(), Value::from(0));

        let mut inner_b = Map::new();
        inner_b.insert("x".to_string(), Value::from(1));
        inner_b.insert("y".to_string(), Value::from(2));
        let mut outer_b = Map::new();
        outer_b.insert("a".to_string(), Value::from(0));
        outer_b.insert("nested".to_string(), Value::Object(inner_b));
        outer_b.insert("z".to_string(), Value::from("last"));

        let va = Value::Object(outer_a);
        let vb = Value::Object(outer_b);

        assert_eq!(
            manifest_content_hash(&va),
            manifest_content_hash(&vb),
            "key insertion order must not affect the content hash"
        );

        // And both equal the digest of the sorted-literal parse, pinning that
        // canonicalization is order-normalizing rather than merely coincidental.
        let sorted: Value =
            serde_json::from_str(r#"{"a":0,"nested":{"x":1,"y":2},"z":"last"}"#).unwrap();
        assert_eq!(manifest_content_hash(&va), manifest_content_hash(&sorted));
    }

    // A1: miss -> fetch -> store -> hit, and the hit path performs zero fetches.
    #[tokio::test]
    async fn miss_fetches_then_hit_serves_without_fetch() {
        let root = temp_root("hit");
        let cache = WorkflowCache::new(root.clone());
        let manifest = serde_json::json!({ "steps": [1, 2, 3], "name": "demo" });
        let hash = manifest_content_hash(&manifest);
        let (fetcher, calls) = MockFetcher::new(manifest.clone());

        let first = cache.get("wf_demo", &hash, &fetcher).await.unwrap();
        assert_eq!(first, manifest);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "miss should fetch once");
        assert!(cache.path_for("wf_demo", &hash).exists(), "stored on disk");

        let second = cache.get("wf_demo", &hash, &fetcher).await.unwrap();
        assert_eq!(second, manifest);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "hit path must not call the fetcher"
        );

        let _ = fs::remove_dir_all(root);
    }

    // A2: a served manifest that does not hash to the requested value is a hard
    // error and is never written to disk.
    #[tokio::test]
    async fn hash_mismatch_errors_and_stores_nothing() {
        let root = temp_root("mismatch");
        let cache = WorkflowCache::new(root.clone());
        let manifest = serde_json::json!({ "name": "real" });
        let real_hash = manifest_content_hash(&manifest);
        let wrong_hash = other_valid_hash(&real_hash);
        let (fetcher, calls) = MockFetcher::new(manifest);

        let result = cache.get("wf_demo", &wrong_hash, &fetcher).await;
        assert!(
            matches!(result, Err(CacheError::HashMismatch { .. })),
            "expected HashMismatch, got {result:?}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            !cache.path_for("wf_demo", &wrong_hash).exists(),
            "mismatched manifest must not be stored"
        );

        let _ = fs::remove_dir_all(root);
    }

    // A2: a corrupt cached file is evicted and refetched exactly once.
    #[tokio::test]
    async fn corrupt_cache_file_is_evicted_and_refetched_once() {
        let root = temp_root("corrupt");
        let cache = WorkflowCache::new(root.clone());
        let manifest = serde_json::json!({ "name": "demo", "v": 7 });
        let hash = manifest_content_hash(&manifest);
        let (fetcher, calls) = MockFetcher::new(manifest.clone());

        // Prime the cache.
        cache.get("wf_demo", &hash, &fetcher).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Corrupt the stored file in place.
        let path = cache.path_for("wf_demo", &hash);
        fs::write(&path, b"} not valid json {{{").unwrap();

        // Next get detects corruption, evicts, and refetches (exactly once more).
        let recovered = cache.get("wf_demo", &hash, &fetcher).await.unwrap();
        assert_eq!(recovered, manifest);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "corrupt hit should trigger exactly one refetch"
        );
        // And the freshly restored file is a valid hit again (no further fetch).
        cache.get("wf_demo", &hash, &fetcher).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        let _ = fs::remove_dir_all(root);
    }

    // A3: GC keeps newest N per id, deletes older + over-age versions, and
    // ignores stray files.
    #[cfg(unix)]
    #[tokio::test]
    async fn gc_keeps_newest_and_respects_age_and_ignores_strays() {
        let root = temp_root("gc");
        let cache = WorkflowCache::new(root.clone());

        // Four fresh versions of wf_a with staggered mtimes (1h..4h old).
        let mut a_paths = Vec::new();
        for i in 0..4u64 {
            let manifest = serde_json::json!({ "wf": "a", "rev": i });
            let hash = manifest_content_hash(&manifest);
            let (fetcher, _) = MockFetcher::new(manifest);
            cache.get("wf_a", &hash, &fetcher).await.unwrap();
            let path = cache.path_for("wf_a", &hash);
            set_mtime_secs_ago(&path, (i + 1) * 3600);
            a_paths.push((path, i));
        }

        // One over-age (40-day-old) version of wf_b, alone in its dir.
        let b_manifest = serde_json::json!({ "wf": "b" });
        let b_hash = manifest_content_hash(&b_manifest);
        let (b_fetcher, _) = MockFetcher::new(b_manifest);
        cache.get("wf_b", &b_hash, &b_fetcher).await.unwrap();
        let b_path = cache.path_for("wf_b", &b_hash);
        set_mtime_secs_ago(&b_path, 40 * 86_400);

        // A stray non-json file in wf_a's dir must be ignored by GC.
        let stray = root.join("wf_a").join("leftover.tmp");
        fs::write(&stray, b"junk").unwrap();

        let stats = cache.gc(2, 30);

        // wf_a: keep the 2 newest (rev 0 @1h, rev 1 @2h), delete rev 2/3.
        assert!(a_paths[0].0.exists(), "newest wf_a kept");
        assert!(a_paths[1].0.exists(), "2nd newest wf_a kept");
        assert!(!a_paths[2].0.exists(), "3rd newest wf_a deleted");
        assert!(!a_paths[3].0.exists(), "oldest wf_a deleted");
        // wf_b: within keep count but over 30 days -> deleted.
        assert!(!b_path.exists(), "over-age wf_b deleted despite keep count");
        // Stray untouched, and not counted as a manifest.
        assert!(stray.exists(), "stray non-json file left alone");

        assert_eq!(stats.scanned, 5, "4 wf_a + 1 wf_b json files scanned");
        assert_eq!(stats.deleted, 3, "2 over-count wf_a + 1 over-age wf_b");
        assert_eq!(stats.kept, 2);

        let _ = fs::remove_dir_all(root);
    }

    // GC on an empty/absent cache is a no-op, not a panic.
    #[test]
    fn gc_on_missing_root_is_noop() {
        let root = temp_root("gc-empty");
        let cache = WorkflowCache::new(root.clone());
        assert_eq!(cache.gc(3, 30), GcStats::default());
    }

    // Sanitization rejects traversal/invalid ids before any fetch occurs.
    #[tokio::test]
    async fn invalid_workflow_ids_are_rejected_without_fetching() {
        let root = temp_root("sanitize");
        let cache = WorkflowCache::new(root.clone());
        let manifest = serde_json::json!({ "name": "demo" });
        let hash = manifest_content_hash(&manifest);

        for bad in ["../evil", "a/b", "UPPER", "", "with.dot", "sp ace"] {
            let (fetcher, calls) = MockFetcher::new(manifest.clone());
            let result = cache.get(bad, &hash, &fetcher).await;
            assert!(
                matches!(result, Err(CacheError::InvalidWorkflowId(_))),
                "id {bad:?} should be rejected, got {result:?}"
            );
            assert_eq!(
                calls.load(Ordering::SeqCst),
                0,
                "rejected id {bad:?} must not fetch"
            );
        }

        let _ = fs::remove_dir_all(root);
    }

    // Malformed content hashes are rejected before any fetch occurs.
    #[tokio::test]
    async fn invalid_content_hashes_are_rejected_without_fetching() {
        let root = temp_root("hash-validate");
        let cache = WorkflowCache::new(root.clone());
        let manifest = serde_json::json!({ "name": "demo" });
        let (fetcher, calls) = MockFetcher::new(manifest);

        // Too short, non-hex, uppercase, and traversal-ish.
        for bad in ["abc", "../../etc/passwd", &"Z".repeat(64), &"g".repeat(64)] {
            let result = cache.get("wf_demo", bad, &fetcher).await;
            assert!(
                matches!(result, Err(CacheError::InvalidContentHash(_))),
                "hash {bad:?} should be rejected, got {result:?}"
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let _ = fs::remove_dir_all(root);
    }
}
