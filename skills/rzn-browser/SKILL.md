---
name: rzn-browser
description: Browser automation for data extraction, form filling, search, posting, messaging, and signed-in web workflows. Use direct CLI commands to drive the browser through the native host bridge, or run premade workflows like Google Search, Reddit post/comment flows, PubMed search/extract, ChatGPT/Claude chats, X threads, Instagram assets, Amazon/G2/Capterra reviews, and App Store lookup.
---

# RZN Browser

Use direct CLI commands to automate the user's browser through the native host bridge. This skill is both a **use skill** and a **workflow-build skill**:

- Use mode: run existing workflows or `llm-auto` to complete browser tasks.
- Build mode: create, edit, validate, or promote workflow JSON when a task should become repeatable.

Prefer direct CLI commands. Do not write Python helper scripts or wrapper scripts for normal use.

## Use This When

- The user asks to automate, inspect, extract data, fill forms, search, post, message, or operate a signed-in website.
- A premade workflow might already cover the task.
- The task is fuzzy enough for `llm-auto`, but still belongs in the browser.
- A successful fuzzy run should become a reusable workflow.
- The user asks to build, debug, validate, or contribute a workflow.

## Do Not Use This When

- A normal answer, API call, file edit, or local code change solves the task without browser automation.
- The user needs a test suite, remote browser infrastructure, or large-scale scraping.
- The browser/native-host bridge is unavailable.
- The task would post, send, buy, like, submit, delete, or otherwise mutate state without explicit approval or a draft/review mode.

## Good Premade Workflow Examples

- Search/research: Google Search, Google Scholar, Google Maps, PubMed search/extract, arXiv, ScienceDirect.
- Posting and communities: Reddit post/comment/message flows, Hacker News submit/comment/reply, X post/thread/DM flows.
- AI apps: ChatGPT send/export/download flows, Claude send/recent/export flows.
- Shopping and reviews: Amazon product/search, G2 reviews, Capterra reviews, Etsy listings.
- App and social data: App Store lookup/search, Instagram profile/post asset extraction.

## First Move

1. Inspect the catalog before improvising:

```bash
rzn-browser list
rzn-browser list <system>
rzn-browser list <system> <workflow>
```

2. If a listed workflow fits, run it:

```bash
rzn-browser run <system> <workflow> --param key="value"
```

3. If the task is ambiguous or no workflow fits, use `llm-auto`:

```bash
rzn-browser llm-auto "Do the browser task in plain language" --max-steps 20
```

4. If the task is to create or improve a durable workflow, read [references/workflow-authoring.md](references/workflow-authoring.md).

Read [references/cli-cheatsheet.md](references/cli-cheatsheet.md) for exact command forms and flags.

## Choice Rule

| Situation | Use | Why |
| --- | --- | --- |
| Known repeatable task with matching workflow | `rzn-browser run` | Deterministic, documented params, stable output |
| Need to discover available capability | `rzn-browser list` then detailed help | The CLI already knows params, examples, and notes |
| Fuzzy browser task with no known workflow | `rzn-browser llm-auto` | Lets the planner observe and adapt |
| Fuzzy task that should become repeatable | `llm-auto --save-workflow true --name ...`, then validate | Discovery first, deterministic reuse after |
| Build or edit workflow JSON | `workflow-authoring.md` + `rzn-browser workflow validate` | Keeps reusable workflows stable |
| Authenticated app flow | Existing Chrome session plus workflow/`llm-auto` | RZN is for the user's real browser profile |
| Write action: post, send, like, buy, submit | Draft/review workflow or explicit user approval | Blind writes are how agents get spicy in the bad way |

## Runtime Contract

- Use the user's existing browser session with the extension and native host.
- Do not launch isolated profiles, temporary browser apps, or a fresh browser unless the user explicitly asks.
- If native host or extension wiring is broken, stop and tell the user what to reload instead of silently switching tools.
- Use `rzn-browser` on PATH. In a source checkout, `./target/release/rzn-browser` is acceptable only when the installed binary is missing or the task needs local uninstalled changes.
- `llm-auto` needs real provider env vars (`OPENAI_*` or `GEMINI_*`) unless using `LLM_PROVIDER=dummy` for local deterministic smoke checks.
- For machine-readable output, prefer `--json` where the command supports it.

Read [references/runtime-troubleshooting.md](references/runtime-troubleshooting.md) if a command cannot connect to Chrome or the native host.

## Workflow Flow

```text
User task
  |
  v
Check catalog: rzn-browser list [system] [workflow]
  |
  +-- matching workflow --> rzn-browser run ... --param ...
  |
  +-- no match / fuzzy --> rzn-browser llm-auto "..."
                              |
                              +-- worth reusing --> save workflow, validate, rerun
```

## Working With Workflows

Use workflow ids in the form `<system> <workflow>` or `system/workflow`. Prefer installed ids over repo-relative JSON paths once a workflow is part of the catalog.

Common inspection and validation:

```bash
rzn-browser workflow show <system> <workflow> --json
rzn-browser workflow validate <system> <workflow>
rzn-browser workflow dirs
```

If editing or creating workflow JSON, read [references/workflow-authoring.md](references/workflow-authoring.md) first. Existing workflows must keep site-specific details in workflow data/docs, not in shared engine code.

## llm-auto Defaults

Use `llm-auto` for goals, not for known fixed workflows:

```bash
rzn-browser llm-auto "Search Google for browser automation tools and extract the first page" --json
```

Useful flags:

- `--url <url>` to start somewhere deterministic.
- `--context "..."` to provide constraints or known state.
- `--constraint "..."` to make rules explicit.
- `--max-steps <n>` to bound execution.
- `--prefer-cached false` when you do not want cached workflows tried first.
- `--save-workflow true --name <name>` when a successful run should become reusable.
- `--pure-llm` only when deterministic macros are getting in the way.

## Reporting Results

Report:

- command used
- whether it used a workflow or `llm-auto`
- important output rows or extracted fields
- any browser-side action left for user review
- connection/runtime failures with the exact next manual step

Do not paste huge raw JSON unless the user asks. Summarize the high-signal fields and mention how to rerun the command.
