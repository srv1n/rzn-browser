# Instagram Asset Export

## Overview
- Goal: Given an Instagram account and an optional date window, discover recent profile posts, open each post in the current Chrome session, step through the visible viewer, and persist the highest-quality assets plus post metadata into a resumable local cache.
- Constraints: Reuse the existing authenticated Chrome session; avoid engine-level site special-casing; keep looping/date filtering in a wrapper script because the workflow DSL does not natively aggregate nested per-post runs.

## Flow Diagrams
- End-to-end flow
```text
instagram export helper
  -> start_app.sh
  -> rzn-browser desktop-run
  -> extension + broker
  -> instagram.com profile grid
  -> progressive profile scrolling
  -> per-post viewer expansion
  -> index.json + per-post cache + run manifest
```

- Internal flow
```text
profile workflow
  -> recent post/reel URLs
  -> exporter loop
      -> post-assets workflow per URL
      -> timestamp filter
      -> skip if index says complete
      -> best-effort download + sidecars
  -> output/instagram/<handle>/
      -> index.json
      -> <yyyy-mm-dd>_<post_id_or_shortcode>_*.jpg|.mp4|.json|.txt
      -> run_<since>_to_<until>.json|.md
```

## Decision Record
- Split the feature into two deterministic workflows plus a repo-local wrapper, mirroring the existing X export pattern.
- Keep all Instagram-specific behavior in workflow data and wrapper logic rather than adding code-path heuristics inside the shared automation engine.
- Default the window to the last 10 days because that matches the user’s ask and bounds the number of posts that need to be reopened.
- Persist an account-level index so later runs can skip completed posts without inferring resume state from folder names alone.

## Architecture
- Workflows (invoked directly via the `rzn-browser` binary)
  - `workflows/instagram/instagram-profile-recent-posts.json`: profile discovery and candidate URL extraction.
    - Run: `rzn-browser run instagram profile-recent-posts --param handle="<handle>" --param target_count="36"`
  - `workflows/instagram/instagram-post-extract.json`: per-post viewer stepping plus timestamp/media/comment/stat extraction with hydration + DOM fallbacks.
    - Run: `rzn-browser run instagram post-extract --param post_url="https://www.instagram.com/p/<shortcode>/"`
- Output contract
  - Each workflow returns one structured JSON payload (candidate URLs, or per-post media/comments/stats with image/video asset URLs).
  - Date-window filtering, resume state, on-disk asset downloads, and run manifests are caller responsibilities — there is no built-in Python wrapper anymore. Operators driving recurring exports compose these calls in their own runner.

## Implementation Notes
- The profile workflow collects `/p/`, `/reel/`, and `/tv/` links from the visible grid and keeps scrolling until it reaches the requested candidate count or repeated idle scrolls indicate nothing new is rendering.
- The post workflow steps through the visible viewer when a next control is present, while preferring hydrated media candidates to choose the largest known image/video URL.
- The exporter stops early after several consecutive older posts because Instagram profile order is newest-first.
- Asset downloads are best-effort direct HTTP fetches with a browser-like user agent and Instagram referer.
- Each completed post writes both a machine-readable JSON sidecar and a text summary containing caption, stats, comments, and local file paths.
- The browser hover primitive now performs a JS-only cursor-approach sequence across `elementsFromPoint(...)` targets instead of dispatching one isolated event on the final element; this is a preparatory step for reading Instagram grid overlays before adding native mouse movement.

## Tasks & Status
- [x] Add profile discovery workflow for recent Instagram post URLs.
- [x] Add per-post media extraction workflow.
- [x] Add local exporter/downloader wrapper with a default 10-day window.
- [x] Add persistent `index.json` resume state and per-post sidecars.
- [x] Add operator-friendly filters for posts, likes, image assets, video assets, and progressive scroll depth.
- [x] Add workflow docs and this feature scratchpad.
- [x] Strengthen JS-only hover simulation so Instagram-style grid overlays have a better chance to render before native hover support exists.
- [ ] Validate comment extraction against more live Instagram accounts and tighten fallbacks if the visible comment rail shifts.

## What Works (Do Not Change)
- The general pattern of deterministic workflow primitives plus a local wrapper for nested iteration.
- Reuse of the active Chrome session through `start_app.sh` and desktop broker wiring.
- Output layout rooted under a caller-supplied base directory with one stable handle cache and resumable post directories.

## Tried & Didn’t Work
- Relying only on `download_images`: too narrow because Instagram posts can mix images and videos.
- Trying to express the full account crawl directly inside one workflow: the current DSL does not provide the nested loop/aggregation needed for clean date filtering and per-post downloads.
- Using only date-window folders as state: too weak for resume because the exporter needs a stable notion of “already completed” per post, not per run.
