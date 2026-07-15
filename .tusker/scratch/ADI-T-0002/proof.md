# ADI-T-0002 proof capsule — Meta creative asset capture

Capture is `meta_ad_library/search` + `--download-dir`: the pack emits attachment_urls
(video src for video ads, creative image for image ads); the CLI streams each with a
size cap, records path+bytes+sha256, and skips already-downloaded assets.

## A1 (>=90% of video/image ads -> local assets)
- Whatnot (video-heavy advertiser), cap=12 --download-dir -> downloaded 12/12 (100%),
  0 errors; every ad yielded a local asset file on disk.
- Nike cap=8 --download-dir -> 8/8.

## A2 (manifest assets[].path exist; hashes + sizes recorded)
- Run 1 manifest: all 8 items have path + bytes + sha256; all 8 files exist on disk.
  Sample: attachments/001_1869276447125570.jpg bytes=13537 sha256=22731b8d...
- Unit: cargo test -p rzn-browser --test asset_download_test::download_records_size_and_hash
  (sha256 matches bytes on disk).

## A3 (re-run skips already-downloaded)
- Run 2 (same --download-dir) -> downloaded=0, skipped=8 (manifest.skipped=8).
- Unit: existing_file_is_skipped (skipped=true, hashes existing, no fetch).

## A4 (size caps + timeouts; failing asset -> per-asset error, not run abort)
- cargo test -p rzn-browser --test asset_download_test -> 4 passed:
  oversized_asset_rejected (5000 bytes vs 1000 cap -> Err),
  unreachable_url_errors_gracefully (Err, no panic, no partial file).
- Downloader loop records {kind,url,error} per failing asset and continues (no abort);
  100 MiB default cap via streaming; reqwest client 60s timeout.

## Artifacts
- workflows/meta_ad_library/search.json (emits attachment_urls; gradual-scroll video upgrade)
- crates/rzn_browser/src/asset_download.rs (stream + sha256 + size cap + skip-existing)
- crates/rzn_browser/src/main.rs (download loop records sha256 + skipped; manifest adds skipped count)
- crates/rzn_browser/tests/asset_download_test.rs (4 focused tests)

## Note
Engine changes are built in target/debug/rzn-browser; run `make install` to use --download-dir
with hashing/skip/size-cap from the installed `rzn-browser`.
