# Instagram Follow Workflow

## Overview
- Goal: Build a deterministic workflow that reuses the logged-in Chrome Instagram session, opens a fixed starter list of Portland-oriented accounts, and follows each account if it is not already followed.
- Constraints: The system must use the extension/native-host path instead of Playwright, keep Instagram-specific targeting inside workflow JSON rather than shared engine code, tolerate Instagram's SPA hydration and modal interruptions, and avoid duplicate follow attempts when an account is already followed or requested.

## Flow Diagrams
- End-to-end flow
```text
CLI factory
  -> native host
  -> Chrome extension background
  -> content script
  -> instagram.com profile pages
  <- content script
  <- extension background
  <- native host
  <- CLI
```

- Internal flow
```text
for each handle in starter pack
  -> navigate_to_url(profile)
  -> wait for hydration
  -> dismiss incidental popups
  -> inspect visible CTA buttons
  -> if CTA is "Follow", click it
  -> if CTA is "Following" / "Requested", skip
  -> return per-handle status payload
```

## Decision Record
- The workflow is profile-by-profile rather than API-driven because internal Instagram follow endpoints are more brittle and harder to validate safely than clicking the visible CTA inside the authenticated session.
- Instagram-specific selectors stay in the workflow JSON. No site-specific engine heuristics are added.
- A probe workflow is used first because Instagram's button copy and DOM structure vary by account state, login state, and experiments.
- The workflow should treat `Following` and `Requested` as success states. Re-clicking those buttons would create accidental unfollow or cancellation behavior.

## Architecture
- Modules:
  - `/Users/sarav/Downloads/side/rzn/rzn-browser/workflows/generated/instagram-follow-probe.json`: Probe flow for live CTA inspection.
  - `/Users/sarav/Downloads/side/rzn/rzn-browser/workflows/generated/instagram-follow-portland-starter.json`: Initial direct-profile batch workflow, kept as a deterministic baseline.
  - `/Users/sarav/Downloads/side/rzn/rzn-browser/workflows/generated/instagram-follow-one-via-search.json`: Parameterized search-first workflow that opens Instagram home, searches a handle, clicks the matching row, and follows the profile if needed.
  - `/Users/sarav/Downloads/side/rzn/rzn-browser/workflows/generated/instagram-search-probe.json`: Probe flow for the Instagram search rail.
  - `/Users/sarav/Downloads/side/rzn/rzn-browser/workflows/generated/instagram-search-handle-probe.json`: Probe flow for typed search-result behavior.
- Data contracts:
  - Workflow step results are JSON strings emitted from `execute_javascript` with `{ handle, status, clicked, button_text, url }`.
  - Status values are expected to be `followed`, `already_following`, `requested`, `not_found`, or `blocked`.

## Implementation Notes
- Entry points:
  - `make run W=workflows/generated/instagram-follow-probe.json PARAMS='--param handle="..."'`
  - `make run W=workflows/generated/instagram-follow-portland-starter.json`
- Key calls and event flow:
  - The search-first workflow opens `instagram.com`, expands the native search rail, types the handle with paced input events, then clicks the matching visible row from the left search pane.
  - Follow targeting uses visible CTA text and aria-label matching so the workflow survives class-name churn.
  - A post-click stabilization pause is required because Instagram often updates CTA state asynchronously after the click.
- Error handling & retries:
  - Login walls or unavailable profiles should return explicit statuses instead of silently succeeding.
  - Popups are dismissed opportunistically before CTA lookup.
  - Per-profile follow logic should be idempotent: if the button already reads `Following` or `Requested`, the workflow reports that and moves on.

## Tasks & Status
- [x] Read repo guidance and confirm the native workflow-factory path
- [x] Add a live Instagram probe workflow
- [ ] Probe the live Instagram profile DOM in the logged-in Chrome session
- [x] Build the final 20-account Portland starter workflow
- [x] Build the search-first single-account workflow requested after live feedback
- [ ] Execute the workflow and verify per-account results

## What Works (Do Not Change)
- Keep the automation on the existing Chrome session with the installed extension and native host.
- Keep Instagram-specific handling in workflow data, not in shared Rust or extension code.
- Preserve idempotent follow behavior by treating already-followed states as no-ops.

## Tried & Didn’t Work
- None yet. This doc will be updated with any rejected approaches during live validation.
