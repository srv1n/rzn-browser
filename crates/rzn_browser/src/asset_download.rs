//! Streaming asset downloader for workflow `--download-dir` capture (ADI-T-0002).
//!
//! Downloads a URL to a destination file while (1) enforcing a hard size cap so a
//! runaway asset can't fill the disk, (2) computing a sha256 of the bytes, and
//! (3) skipping the fetch entirely when the destination already exists (idempotent
//! re-runs). A failing asset returns `Err` to the caller, which records a per-asset
//! error and moves on rather than aborting the whole capture.

use std::path::Path;

use anyhow::Context;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

/// Default per-asset size cap (100 MiB). Large enough for ad videos, small enough
/// to stop a pathological URL from filling the disk.
pub const DEFAULT_ASSET_MAX_BYTES: u64 = 100 * 1024 * 1024;

/// The result of downloading (or skipping) one asset.
#[derive(Debug, Clone)]
pub struct AssetOutcome {
    pub bytes: u64,
    pub sha256: String,
    /// True when the destination already existed and the fetch was skipped.
    pub skipped: bool,
}

/// Compute `(size, sha256_hex)` of an existing file.
pub async fn hash_existing(path: &Path) -> anyhow::Result<(u64, String)> {
    let data = tokio::fs::read(path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Ok((data.len() as u64, format!("{:x}", hasher.finalize())))
}

/// Download `url` into `dest`, streaming with a hard `max_bytes` cap and computing
/// a sha256. If `dest` already exists, the download is skipped and the existing
/// file is hashed instead (so a re-run is idempotent). Errors (unreachable URL,
/// non-2xx status, oversized body) are returned to the caller, not fatal.
pub async fn download_asset(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    max_bytes: u64,
) -> anyhow::Result<AssetOutcome> {
    if tokio::fs::try_exists(dest).await.unwrap_or(false) {
        let (bytes, sha256) = hash_existing(dest).await?;
        return Ok(AssetOutcome {
            bytes,
            sha256,
            skipped: true,
        });
    }

    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("status {url}"))?;

    // Reject early when the server advertises a size over the cap.
    if let Some(len) = resp.content_length() {
        if len > max_bytes {
            anyhow::bail!("asset content-length {len} exceeds cap {max_bytes} bytes");
        }
    }

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let mut file = tokio::fs::File::create(dest)
        .await
        .with_context(|| format!("create {}", dest.display()))?;
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("read body {url}"))?;
        total += chunk.len() as u64;
        if total > max_bytes {
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            anyhow::bail!("asset exceeds cap {max_bytes} bytes");
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .with_context(|| format!("write {}", dest.display()))?;
    }
    file.flush().await.ok();
    Ok(AssetOutcome {
        bytes: total,
        sha256: format!("{:x}", hasher.finalize()),
        skipped: false,
    })
}
