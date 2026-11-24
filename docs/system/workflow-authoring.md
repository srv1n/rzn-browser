---
title: "Workflow authoring"
subject: workflow-authoring
keywords: [workflow, JSON, catalog, authoring]
part_of: overview
kind: doc
lifecycle: current
code_conformance: unverified
read_when: "You need to add or change a workflow."
skip_when: "You need runtime or extension code. Open architecture."
---

# Workflow authoring

A workflow is one JSON file that describes a repeatable browser task. Start
from a nearby workflow and keep the task specific.

```mermaid
flowchart LR
  copy["copy a workflow for<br/>the same site"] --> edit["edit the manifest"]
  edit --> valid["validate --strict"]
  valid --> smoke["safe smoke run"]
  smoke --> submit["submit with proof"]
```

## Start with a nearby file

Put a production workflow in `workflows/<system>/`. Copy a workflow for the
same site and action shape. Keep selectors and site logic in JSON. Keep shared
engine code generic.

## Edit the manifest

Include:

- stable `id`, `name`, `system`, and `capability`;
- typed parameters in `params.properties`;
- ordered actions in `steps`;
- honest top-level and step side effects;
- an output selector and output schema;
- a `help` block with examples and notes.

Parameter kinds:

| Kind | Use | Validation |
| --- | --- | --- |
| `String` | Text input | Optional regex or non-empty rule |
| `Number` | Integer or float | Numeric bounds |
| `Boolean` | Flag | Boolean coercion |
| `Secret` | Credential input | Redacted in logs and telemetry |

Mark secret inputs as sensitive. Do not write secret values to the file or a
log.

### Browse like one person

The CLI queues every manifest run by its `system`. Runs for one site execute one
at a time across processes, and each starts as soon as the previous one ends.
Eight parallel `capterra search` calls run one after another, not at once.
CLI runs for different sites can overlap in separate session tabs. Claims using
an explicit `--tab-ref` share a lane and queue when the ref matches. Workflows
that use the active tab without an explicit ref, workflows that switch tabs,
and legacy in-process/fleet runs retain exclusive supervisor access. A
conflicting run waits with a `[queue]` message. `status.snapshot.active_runs` reports all active runs; `now_running`
keeps reporting the oldest for existing clients. When several runs are active,
`runs.cancel` requires a `run_id`.

- Each run reuses the site's parked tab, which stays open between runs and
  comes to the front when the run starts. The exceptions are workflows with `requires_existing_session` (they bind the
  user's active tab) and workflows with `open_new_tab`, `switch_to_tab`, or
  `close_current_tab` steps.
- If a step throws an error that contains `site_challenge`, the site pauses for
  15 minutes. Later runs fail fast and do not reload the challenge page.
- The state lives in `<app base>/site-queue/<system>.json`. Delete that file to
  clear a pause early.

The extension applies two rules to every workflow, so a plain
`navigate_to_url` already browses like a person:

- When the tab already shows the same site, `navigate_to_url` pauses for about
  a second and navigates from inside the page (`location.assign`). The request
  carries the site's referrer, like a click, instead of looking like a fresh
  address-bar visit. The first visit to a site, a different site, or a
  fragment-only change uses a normal browser navigation.
- After `navigate_to_url` and `wait_for_navigation`, the extension checks the
  page for a bot check (Cloudflare, DataDome, PerimeterX, Amazon or Google
  captcha pages, and "verify you are human" walls). A check that clears itself
  within about 8 seconds passes. A wall that stays fails the step with
  `SITE_CHALLENGE` / `site_challenge`, which pauses the site.

The steps set the rest of the pace. Click a link on the current page when the
flow follows one. Scroll (`scroll_window_to`) and pause (`wait_for_timeout`)
before you extract from a page a person would read.

### Read what the page already loaded

A read workflow makes no request of its own: no `fetch`, `XMLHttpRequest`,
`navigator.sendBeacon`, `WebSocket` or `EventSource` in a script, no
`same_origin_request` step, no per-item lookups. Keep a copy of the responses
the site's own app downloads instead:

```json
{ "id": "capture", "action": { "kind": "capture_responses",
  "inputs": { "url_pattern": "/graphql/[^/?]+/UserTweets\\?", "methods": ["GET"] },
  "side_effects": ["browser_state"] } }
```

Put it before `navigate_to_url`; the tab then opens blank, starts capturing and
navigates, so the first batch is kept too. `url_pattern` is a JavaScript regular
expression tested against each response URL. Optional: `resource_types`
(default `["XHR", "Fetch"]`, `["*"]` for any), `methods`, `max_responses` (200),
`max_body_bytes` (8 MiB) and `max_total_bytes` (24 MiB). `{"stop": true}` ends a
capture early; otherwise it ends with the run. The extension reads bodies the
browser already received (CDP `Network.getResponseBody`), so nothing is sent.
If it cannot watch the tab, the step fails with `CAPTURE_ATTACH_FAILED`.

A later `execute_javascript` step with `"captured_responses": true` gets
`params.captured_responses`: `{ url_pattern, responses: [{ seq, url, method,
status, mime_type, resource_type, bytes, json | text }], duplicates, dropped,
detached }`, in arrival order, identical bodies once. Map those; read the DOM
only for what no response carried. `x_profile_posts` shows the pattern. The lint
`READ_WORKFLOWS_MAKING_REQUESTS_BASELINE` counts read workflows still making
their own requests; lower it in the change that moves one.

Keep timing out of page JavaScript. Chrome throttles timers in background tabs
to about once a minute, so `setTimeout` loops and `simulate_human_behavior`
stall there. Put pauses in `wait_for_timeout` steps, which the runner executes.
Wait for content with a `MutationObserver`. Give `wait_for_navigation` a
`url_pattern`, because without one it waits for the full load event, which slow
ads can delay past the step timeout. The G2 and Capterra packs show the pattern.

## Shipped packs

Production packs live in `workflows/<system>/`: search and research (Google,
Bing, YouTube, PubMed, arXiv, ScienceDirect, Goodreads, Local Book Search, Yahoo Finance),
commerce and reviews (Amazon, Etsy, G2, Capterra, App Store, Airbnb), ads and
market intelligence (Meta Ad Library, Google Ads Transparency, Apple Ads), AI
apps (ChatGPT, Claude), and social (X, Reddit, Hacker News, Instagram,
LinkedIn). Each pack keeps site logic in JSON and shares the generic engine.

## Declare the output contract

Every manifest sets `result.contract`:

- `rzn.items/v2` (a page) or `rzn.item/v2` (one item) for a read workflow whose output feeds an
  index. The shape is rzn-tools `rzn_tools_core::item`, vendored as `schema/rzn.item.v2.json`,
  `schema/rzn.items.v2.json` and `schema/rzn.describe.v2.json` (each names the rzn-tools commit and
  file hash it follows). Do the site mapping in the workflow's final step, usually
  `execute_javascript`; keep the engine generic.
- `none` for write, utility and not-yet-moved read workflows.

An item contract also needs:

- `workflows/<system>/samples/<file stem>.sample.json`:
  `{"sample_of": "<workflow id>", "source": "recorded" | "hand-built", "output": <result>}`. The
  output must pass the vendored schema and the rules a schema cannot express: full items carry
  `author`, `published_at` (RFC 3339), `published_in` and `blocks`; keys are `<site>:<kind>:<id>`;
  `listing` is `full` or `feed` (say `feed` unless the pages name every item in scope).
- `workflows/<system>/describe.json`: the family's `rzn.describe/v2`, naming workflows of that
  family as `list_tool` and `get_tool`.

`READ_WORKFLOWS_ON_NONE_BASELINE` in `crates/rzn_browser/src/workflow_contract_lint.rs` counts read
workflows (no `external_write`) still on `none`. Lower it in the change that moves one; never raise it.

## Validate

```sh
rzn-browser workflow validate workflows/<system>/<name>.json --strict --json
rzn-browser workflow inspect <system> <name>
rzn-browser workflow validate-catalog --strict --json
```

Validate the changed file directly. The catalog check is a second check. It can
choose an effective source and does not replace file review.

Use these commands when source selection is unclear:

```sh
rzn-browser workflow dirs
rzn-browser workflow list --all-sources
rzn-browser workflow inspect <system> <name>
```

## Smoke safely

Run a read-only workflow through the normal path:

```sh
rzn-browser run <system> <name> --param <name>=<value>
```

For a workflow that posts, sends, votes, follows, uploads, or deletes, stop at
the draft or review step unless the operator approved the final action. Read
[Security](security.md) for side-effect and approval rules.

## Submit

Include the workflow JSON, pack documentation when needed, a focused check or
smoke result, the browser and login state used, and the input class. Do not
include page content, credentials, or private account data.
