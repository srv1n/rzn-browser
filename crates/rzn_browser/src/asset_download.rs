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
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Download defaults; callers may opt into a different size limit.
pub const DEFAULT_ASSET_MAX_MIB: u64 = 1024;
pub const DEFAULT_ASSET_MAX_BYTES: u64 = DEFAULT_ASSET_MAX_MIB * 1024 * 1024;
pub const DEFAULT_ASSET_TIMEOUT_SECS: u64 = 30 * 60;

pub fn download_limit_bytes(mib: u64) -> anyhow::Result<u64> {
    if mib == 0 {
        return Ok(u64::MAX);
    }
    mib.checked_mul(1024 * 1024)
        .context("--max-download-mib is too large")
}

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
    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut total = 0u64;
    loop {
        let count = file
            .read(&mut buffer)
            .await
            .with_context(|| format!("read {}", path.display()))?;
        if count == 0 {
            break;
        }
        total += count as u64;
        hasher.update(&buffer[..count]);
    }
    Ok((total, format!("{:x}", hasher.finalize())))
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
    let result = async {
        let mut hasher = Sha256::new();
        let mut total: u64 = 0;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.with_context(|| format!("read body {url}"))?;
            total += chunk.len() as u64;
            if total > max_bytes {
                anyhow::bail!("asset exceeds cap {max_bytes} bytes");
            }
            hasher.update(&chunk);
            file.write_all(&chunk)
                .await
                .with_context(|| format!("write {}", dest.display()))?;
        }
        file.flush()
            .await
            .with_context(|| format!("flush {}", dest.display()))?;
        Ok(AssetOutcome {
            bytes: total,
            sha256: format!("{:x}", hasher.finalize()),
            skipped: false,
        })
    }
    .await;
    drop(file);
    if result.is_err() {
        let _ = tokio::fs::remove_file(dest).await;
    }
    result
}
