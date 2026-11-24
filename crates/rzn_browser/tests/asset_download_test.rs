//! ADI-T-0002: focused tests for the streaming asset downloader — size cap,
//! unreachable/failed URL handling (per-asset error, no panic/abort), sha256, and
//! skip-existing. No external network: a tiny one-shot local server serves bytes.

use std::time::Duration;

use rzn_browser::asset_download::{
    download_asset, download_limit_bytes, hash_existing, DEFAULT_ASSET_MAX_BYTES,
};

/// Serve `body` once over HTTP on an ephemeral localhost port; returns the URL.
async fn serve_once(body: Vec<u8>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut sock, _)) = listener.accept().await {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = [0u8; 2048];
            let _ = sock.read(&mut buf).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(head.as_bytes()).await;
            let _ = sock.write_all(&body).await;
            let _ = sock.flush().await;
        }
    });
    format!("http://{addr}/asset.bin")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

/// A4: an unreachable URL returns Err (a per-asset error), it does not panic.
#[tokio::test]
async fn unreachable_url_errors_gracefully() {
    let dir = tempdir_path("unreachable");
    let dest = dir.join("x.bin");
    let res = download_asset(&client(), "http://127.0.0.1:1/nope", &dest, 1024).await;
    assert!(res.is_err(), "unreachable URL must return Err, not panic");
    assert!(!dest.exists(), "no file should be left behind on failure");
}

/// A4: a body larger than the cap is rejected.
#[tokio::test]
async fn oversized_asset_rejected() {
    let url = serve_once(vec![7u8; 5000]).await;
    let dir = tempdir_path("oversized");
    let dest = dir.join("big.bin");
    let res = download_asset(&client(), &url, &dest, 1000).await;
    assert!(
        res.is_err(),
        "5000-byte asset must be rejected under a 1000-byte cap"
    );
}

/// A2: a successful download records the right size and a sha256 that matches the
/// bytes on disk.
#[tokio::test]
async fn download_records_size_and_hash() {
    let body = b"the quick brown fox jumps over the lazy dog".to_vec();
    let url = serve_once(body.clone()).await;
    let dir = tempdir_path("ok");
    let dest = dir.join("good.bin");
    let outcome = download_asset(&client(), &url, &dest, 1_000_000)
        .await
        .expect("download should succeed");
    assert!(!outcome.skipped);
    assert_eq!(outcome.bytes, body.len() as u64);
    assert!(dest.exists(), "file must exist after download");
    let (disk_size, disk_hash) = hash_existing(&dest).await.unwrap();
    assert_eq!(disk_size, outcome.bytes);
    assert_eq!(
        disk_hash, outcome.sha256,
        "manifest sha256 must match file on disk"
    );
}

/// A3: an existing destination is skipped (idempotent re-run), hashing the file
/// already present rather than re-downloading.
#[tokio::test]
async fn existing_file_is_skipped() {
    let dir = tempdir_path("skip");
    let dest = dir.join("present.bin");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&dest, b"already here").unwrap();
    // Point at an unreachable URL: if it were fetched, this would error; skip wins.
    let outcome = download_asset(&client(), "http://127.0.0.1:1/nope", &dest, 1024)
        .await
        .expect("existing file should be skipped, not fetched");
    assert!(outcome.skipped, "existing file must be skipped");
    assert_eq!(outcome.bytes, "already here".len() as u64);
}

/// Generate a body incrementally; optional length also lets tests simulate truncation.
async fn serve_stream(size: usize, content_length: Option<usize>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 2048];
        let _ = socket.read(&mut request).await;
        let length = content_length
            .map(|size| format!("Content-Length: {size}\r\n"))
            .unwrap_or_default();
        let header = format!("HTTP/1.1 200 OK\r\n{length}Connection: close\r\n\r\n");
        if socket.write_all(header.as_bytes()).await.is_err() {
            return;
        }
        let buffer = vec![b'x'; 64 * 1024];
        let mut remaining = size;
        while remaining > 0 {
            let count = remaining.min(buffer.len());
            if socket.write_all(&buffer[..count]).await.is_err() {
                return;
            }
            remaining -= count;
        }
        let _ = socket.shutdown().await;
    });
    format!("http://{addr}/stream.bin")
}

#[test]
fn configurable_size_limit_supports_large_and_unlimited_downloads() {
    assert_eq!(DEFAULT_ASSET_MAX_BYTES, 1024 * 1024 * 1024);
    assert_eq!(
        download_limit_bytes(2048).unwrap(),
        2 * DEFAULT_ASSET_MAX_BYTES
    );
    assert_eq!(download_limit_bytes(0).unwrap(), u64::MAX);
    assert!(download_limit_bytes(u64::MAX).is_err());
}

#[tokio::test]
async fn default_limit_downloads_and_hashes_more_than_the_old_100_mib_cap() {
    use sha2::{Digest, Sha256};
    let size = 101 * 1024 * 1024;
    let url = serve_stream(size, Some(size)).await;
    let dir = tempdir_path("large-stream");
    let dest = dir.join("large.bin");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let outcome = download_asset(&client, &url, &dest, DEFAULT_ASSET_MAX_BYTES)
        .await
        .unwrap();
    assert_eq!(outcome.bytes, size as u64);
    let mut expected = Sha256::new();
    for _ in 0..(size / (64 * 1024)) {
        expected.update([b'x'; 64 * 1024]);
    }
    assert_eq!(outcome.sha256, format!("{:x}", expected.finalize()));
    assert_eq!(
        hash_existing(&dest).await.unwrap(),
        (outcome.bytes, outcome.sha256)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn unknown_length_still_enforces_the_cap_and_cleans_partial_files() {
    let url = serve_stream(5000, None).await;
    let dir = tempdir_path("stream-cap");
    let dest = dir.join("large.bin");
    let error = download_asset(&client(), &url, &dest, 1000)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("exceeds cap"));
    assert!(!dest.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn interrupted_download_does_not_leave_a_file_that_would_be_skipped_on_retry() {
    let url = serve_stream(100, Some(1000)).await;
    let dir = tempdir_path("stream-interrupted");
    let dest = dir.join("partial.bin");
    assert!(download_asset(&client(), &url, &dest, 1000).await.is_err());
    assert!(!dest.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

/// Unique temp dir path under the OS temp dir (no external crate needed).
fn tempdir_path(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("rzn_asset_test_{tag}_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&p);
    p
}
