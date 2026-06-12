# Hacker News Workflows

## Overview
- Goal: provide a full Hacker News write pack with three core actions: submit a link post, add a root-level comment on an item page, and reply to a specific comment. Accidental-spam risk stays low through a single explicit approval gate per workflow; clicking Stop at the gate yields a draft-only outcome without needing a separate workflow.
- Constraints: the workflows must reuse the authenticated Chrome session, stay at the workflow layer instead of adding HN-specific engine logic, and avoid silently firing final submit clicks without a human approval step.

## Flow Diagrams
- End-to-end flow
```text
CLI / make run
  -> native runner
  -> browser worker
  -> Chrome extension
  -> HN page in authenticated Chrome session
  -> request_user_intervention gate for live writes
  -> final submit click only after Continue
```

- Internal write-lane shape
```text
submit/comment/reply target
  -> navigate/open target page
  -> wait for form fields
  -> fill title/url/text or comment textarea
  -> execute_javascript sanity check
  -> request_user_intervention(ask_user, continue_on_timeout=false)
  -> click submit
```

## Decision Record
- Chosen (2026-04-24): collapse to **one workflow per capability** (link-post, comment, reply). The previous draft + live-with-gate split duplicated 90% of every workflow; the gate's existing Stop button already produces an identical "draft" outcome. Per `AGENTS.md` rules 1–5 (consolidation, no debug-only tools), the duplicates were removed.
- Chosen (2026-04-24): merge "first front-page item" into the comment workflow as an optional `item_url`. Resolves via JS on the front page when the param is empty.
- Chosen: put the anti-spam protection in workflow JSON with `request_user_intervention` rather than relying on operator discipline alone. That is the only thing that actually prevents accidental posts.
- Chosen: keep selectors simple and HN-specific in the workflow pack. HN is stable enough that shared engine heuristics would just be unnecessary abstraction.
- Rejected: immediate-submit comment/reply flows. They were fast, but too easy to misuse.
- Rejected: stuffing long marketing blurbs into `post_text`. HN hates that, and it makes the automation look dumb.
- Rejected (2026-04-24): keeping standalone draft workflows. The approval gate already provides the same outcome; ship one workflow per capability.

## Architecture
- Modules:
  - `workflows/hn/hn-submit-link-post.json` (`hn_submit_link_post_v1`): approval-gated link submission.
  - `workflows/hn/hn-submit-comment.json` (`hn_submit_comment_v1`): approval-gated root comment with optional `item_url` (omit to comment on the first front-page item) and dedupe guard.
  - `workflows/hn/hn-submit-reply.json` (`hn_submit_reply_v1`): approval-gated reply to a comment id with dedupe guard.
  - `workflows/hn/README.md`: operator-facing pack guidance and example payloads.
  - `crates/rzn_plan/tests/hn_workflows_parse_test.rs`: parse coverage for the HN pack.
- Data contracts:
  - Link submission requires `post_title`, `post_url`, and `post_text` (the last may be an empty string).
  - Root comment requires `comment_text`; `item_url` is optional and falls back to the first front-page item.
  - Reply requires `comment_id` and `comment_text`.

## Implementation Notes
- Entry points: `rzn-browser run hn <submit-link-post|submit-comment|submit-reply> --param ...`.
- All workflows run an `execute_javascript` sanity check before the approval gate so we fail fast on empty draft fields instead of clicking submit into garbage.
- Comment + reply workflows additionally run a dedupe-guard JS step before fill: if the logged-in user already has the same normalised text on the parent thread, the workflow throws and exits before any further state change.
- `submit-comment` resolves the item URL via JS on the front page: if `item_url` is provided it origin-validates against `news.ycombinator.com` and redirects; otherwise it picks the first `tr.athing` item and redirects there.
- Approval gates use `approval_mode: "ask_user"` and `continue_on_timeout: false`; timing out kills the write instead of falling through. Click Stop at the gate for a draft-only outcome.
- The guidance text intentionally tells operators to avoid duplicate URLs, sponsor blocks, timestamp dumps, and low-context comments.

## Tasks & Status
- [x] Add approval-gated HN link submission workflow
- [x] Harden live HN root-comment workflow with approval gate + dedupe guard
- [x] Harden live HN reply workflow with approval gate + dedupe guard
- [x] Consolidate 7 → 3 workflows (drop draft-only variants; merge first-item into submit-comment via optional item_url) — 2026-04-24
- [x] Add `help` blocks (summary/parameters/examples/returns/notes) per `AGENTS.md` rule 11 — 2026-04-24
- [x] Add HN workflow docs and pack README updates
- [x] Add HN workflow parse test coverage
- [ ] Live-validate against a real logged-in HN session in Chrome

## What Works (Do Not Change)
- Keep final-submit protection in the workflow layer through `request_user_intervention`.
- Keep HN-specific selectors in the HN workflow pack rather than shared engine code.
- Keep the recommended payload style short and factual; long promotional body text is the wrong fit for HN.

## Tried & Didn’t Work
- Immediate submit after fill: faster, but too easy to weaponize into accidental spam.
- Treating HN like X and adding fancy current-tab composer logic: pointless overengineering for a plain server-rendered form.
- Using the raw user-provided long description as submission text: bloated, noisy, and exactly how to look like a bot.
