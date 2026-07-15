//! ADI-T-0002: focused tests for the streaming asset downloader — size cap,
//! unreachable/failed URL handling (per-asset error, no panic/abort), sha256, and
//! skip-existing. No external network: a tiny one-shot local server serves bytes.

use std::time::Duration;

use rzn_browser::asset_download::{download_asset, hash_existing};

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

/// Unique temp dir path under the OS temp dir (no external crate needed).
fn tempdir_path(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("rzn_asset_test_{tag}_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&p);
    p
}
