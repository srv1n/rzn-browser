# RZN Browser — dossier
Last updated: 2026-11-28 (update this line every edit)

## One paragraph
RZN Browser is a local browser-automation runtime that runs reusable JSON workflows against a person's own signed-in browser. A CLI, MCP client, or coding agent can invoke catalogued tasks for search, research, shopping, ads, AI chat, and social sites; fixed workflows execute through a Rust supervisor, native-messaging host, and browser extension, while optional LLM planning handles tasks without a fixed workflow.

## Status
Shipped as public source and under active development. GitHub has two published releases; `v0.1.0` (2026-07-10) is latest, while the later `v0.1.1` tag failed to publish. Reproduce with `gh release list --repo srv1n/rzn-browser` and `gh run view 31154945644 --repo srv1n/rzn-browser`.

Source installers and extension builds cover Chrome, Edge, and Chromium on macOS, Linux, and Windows, loaded unpacked rather than through a browser store. Published release bundles install only the Chrome extension and Chrome native-host registration. The runtime is installed locally at `/Users/sarav/.local/bin/rzn-browser`; a 2026-08-17 check found its supervisor reachable with zero connected browser bridges. Other private deployment or distribution: UNKNOWN — ask Sarav.

Real users: UNKNOWN — ask Sarav.

## What it does (features, user-facing)
- Run packaged workflows by site and task name from a CLI, MCP client, or coding agent.
- Search and extract from web, maps, academic, video, finance, shopping, travel, review, and book sites.
- Use existing signed-in sessions for ChatGPT, Claude, LinkedIn, X, Reddit, Instagram, Hacker News, and Apple Ads.
- Send chats and attachments, export conversations, generate or download images, and read the reply tied to the exact submitted turn.
- Read social feeds, profiles, posts, comments, jobs, and messages; run declared write workflows for posts, replies, likes, follows, and DMs with confirmation.
- Collect advertising data from Meta Ad Library, Google Ads Transparency, and Apple Ads surfaces.
- Save results to files and download referenced assets with a manifest.
- Select a browser, instance, bridge, or exact tab; fail instead of guessing when the target is ambiguous.
- Create, import, inspect, validate, refresh, and run personal workflow JSON alongside the built-in catalog.
- Ask a supported LLM provider to plan an unfamiliar browser task, save a successful flow, and prefer the cached flow later.
- Install bundled Agent Skills, diagnose native-host wiring, and generate privacy-bounded broken-workflow reports.
- Enrol user-owned computers in a fleet and route work to the machine holding the required browser session.

## Who it's for
The documented audience is developers and coding-agent users automating repeatable tasks in their own signed-in browsers. Actual audience and named current users: UNKNOWN — ask Sarav.

## Numbers that are true
- 100 user-facing workflow JSON files across 25 site directories, plus 6 fixture/smoke files. Reproduce by counting `workflows/*/*.json`, excluding `workflows/fixtures/` and `workflows/_smoke/`, and their parent directories.
- 213 valid catalog manifests exposing 106 capabilities, with zero errors and zero warnings in strict validation on 2026-08-18. Reproduce with `make rust ARGS='run -p rzn-browser -- workflow validate-catalog --strict --json'`.
- 20 bundled skill entrypoints. Reproduce with `rg --files skills -g 'SKILL.md' | wc -l`.
- 7 Rust workspace crates. Reproduce with `rg --files crates -g 'Cargo.toml' | wc -l`.
- 111 extension unit tests in 29 files passed on 2026-08-18. Reproduce with `make test-ext-unit`.
- 2 published GitHub releases; newest is `v0.1.0` from 2026-07-10. Reproduce with `gh release list --repo srv1n/rzn-browser`.
- 1 GitHub star, 0 forks, and 0 watchers on 2026-08-17. Reproduce with `gh repo view srv1n/rzn-browser --json stargazerCount,forkCount,watchers`.
- Installs and downloads: UNKNOWN — ask Sarav.
- Revenue: UNKNOWN — ask Sarav.
- Successful real-world runs, retention, and active-user counts: UNKNOWN — ask Sarav.
- Measured speed, token savings, success rate, and detection rate: UNKNOWN — ask Sarav.

## Tech shape (short)
- Rust workspace: CLI/supervisor, native host, contracts, planner, SDK, and plugin devkit.
- TypeScript MV3 extension built with Bun/Vite; JSON workflow manifests; shell/Python release tooling.
- CLI, MCP, cloud, and fleet calls route through a local authenticated supervisor; native messaging connects the extension through a thin Rust transport host.
- Workflows declare parameters, runtime needs, outputs, and side effects; external writes require confirmation and are not automatically retried.
- DOM/page actions are the normal path; browser-debugger CDP is a bounded fallback.
- Fixed workflows execute without a model call; OpenAI-compatible, Gemini, Anthropic, Groq, and dummy planning paths exist.
- User workflows can shadow built-ins; capability routing and strict catalog validation avoid filename guessing.
- Runtime and extension are AGPL-3.0-only; workflows, skills, and schemas are MIT.

## Recent changes (rolling, newest first, keep last ~10)
Reproduce dates and summaries with `git log --date=short --format='%ad %s'`; these are source-history changes, not proof of deployment.
- 2026-11-28: end-user UI redesign. The extension is named "RZN" (was "RZN Browser Automation"). Popup shows only: who is signed in, this site's capture switch (writes `capture_sites`), where pages go, and "Open RZN"; signed-out, helper-not-answering and not-a-website states are one short message and one action. The options page opens on Account ("Sign in to RZN" / "Signed in as", a "This browser" row with Sign out) and Settings (sites and index); device facts, server URL, pushed source, pause, notifications, history and the Runs, Workflows and Logs pages sit behind one collapsed Advanced section. Plain white look in one font, Geist (OFL, 3 woff2 files, 103 KB), near-black primary buttons; Sarav rejected the first warm-paper, serif-italic version the same day as too close to hex.tech. Errors never show raw RPC or JSON: a helper too old for a method reads "RZN needs an update", anything else one plain sentence, detail in the console. UI and copy only; no background or protocol change. Screens: `docs/ui-redesign-2026-11-28/`, produced by `extension/tests/e2e/ui_screens.spec.ts` with the helper's answers stubbed in the page.
- 2026-11-28: sign in with an RZN account (BFL-T-0012). The dashboard's Fleet page is now Account: "Sign in to RZN" opens the server's sign-in page in a tab as OAuth client `rzn-browser` with PKCE (server built in as `https://app.rzn.ai`, Settings override `server_url`); the helper trades the code, mints and redeems a one-time enrollment code itself, and keeps the session's refresh token in `account.json` (0600). The device belongs to the user and works for every tenant the user is a member of. Capture pushes go to `capture_index`/`capture_source` from Settings with the signed-in user's token; `capture.json` is gone. Account shows who, this browser, recent jobs and sign out; a browser revoked in the app is told and offers sign-in again. Typed enrollment codes remain only through `rzn-browser fleet enroll` for headless machines.
- 2026-11-27: capture of opened pages (BFL-T-0025). Per-site switch in the dashboard settings (`capture_sites`, default none); a page of a switched-on site is read in place by the new `web/page` workflow (no navigation, scroll or request) and the supervisor pushes items in batches (25 items or 60 s) to a pushed source of the owner's private index named in `<app base>/capture.json`; one activity line per page read and batch sent in `capture/activity.jsonl`. Proved in a headless Chromium on the MacBook Air: a voicebraindump.com page reached a test index on build1. Stubs: target and token are a hand-written file; every site reads with `web/page` (X/LinkedIn/Reddit readers need page-load network capture); an unsent batch is lost on a supervisor restart.
- 2026-11-28: capture reads X, LinkedIn and Reddit with their site readers (BFL-T-0026). An X post page goes to `x/open`, a LinkedIn post page to `linkedin/open`, a Reddit thread (www or old) to `reddit/thread`, under the keys those readers use in jobs (`x:post:<first post id>`, `linkedin:post:<id>`, `reddit:thread:<id>`); other pages of those three sites (feeds, profiles, search) are not read; every other site still reads with `web/page`. A read in place runs only the workflow's read-only steps (no navigation, popup dismissal, response capture or scrolling) and tells the script `in_place`, so LinkedIn clicks no "more". `x/open` reads the post page's cards when nothing was captured (no edit marker that way); `reddit/thread` also reads www.reddit.com's rendered thread (shreddit attributes: depth, score, is_op, parent). A `truncated` item (Reddit comments the page did not render, LinkedIn text behind "more", an X article) is logged and not pushed, because the push door refuses a batch holding one. Proved with saved fixture pages in a headless Chromium on the MacBook Air; no live X or LinkedIn account exists. Live Reddit, logged out: Reddit answers the extension test profile with a js_challenge that never reaches the thread; in a plain headless Chromium the read gave the right thread item but Reddit had rendered 0 of 11 comments, so it was held back as truncated. LinkedIn reads still use CDP eval, which attaches the debugger to the owner's tab.
- 2026-08-14: ChatGPT reads were bound to the exact submitted user turn, including concurrent-turn handling.
- 2026-08-13: ChatGPT send results began returning a boundary used to identify the matching response.
- 2026-08-07: OpenAI-compatible base URL overrides were added for local or alternative model endpoints.
- 2026-08-07: A Reddit profile-history workflow pack was added.
- 2026-08-07: Supervisor workflow failures began returning actionable report context.
- 2026-08-07: Workflow tabs were retained across extension service-worker discard and the dashboard refreshed after reconnect.
- 2026-08-07: ChatGPT model selection followed the nested picker into its Advanced panel.
- 2026-07-15: Fleet coordination and advertising workflows were integrated.
- 2026-07-10: ChatGPT's changed model picker was repaired and workflow/skill assets were bundled for release.

## Deliberate exclusions
- Documented scope is one person's repeatable work in their own signed-in sessions, not anonymous mass scraping.
- Cloud and fleet coordination route work to user-owned machines; the repo does not provide a rented remote-browser service.
- The product is workflow-first, not a general browser-testing framework or general-purpose browser SDK.
- Browser-store distribution is not present; extension installation is manual and unpacked.
- Supported extension targets are Chrome, Edge, and Chromium.
- No Playwright, Selenium, or remote-debugging-port dependency is required by the installed runtime; development E2E tests use Playwright.
- Founder-level refusals and their reasons beyond these source-backed boundaries: UNKNOWN — ask Sarav.

## Open questions / embarrassments
- Why it was built, the triggering moment, current user feedback, the proudest part, and knowingly embarrassing unfinished work: UNKNOWN — ask Sarav.
- Current checkout validation on 2026-08-18: workspace check, Rust tests, rustfmt, all three supported extension builds, 111 extension unit tests, schema parity, strict catalog validation, ChatGPT contract scripts, setup sync, and Unix installer verification passed. Clippy fails on one `unnecessary_sort_by` warning in `crates/rzn_core/src/secure_files.rs:131`; PowerShell installer verification was not run because PowerShell is absent.
- `make test-dom-units` references deleted `crates/rzn_plan/tests/dom_integration_test.rs`; `make test-basic` and `make test-google` reference workflow files that do not exist.
- CI and release workflows do not install required `sccache`; latest `main` CI and the `v0.1.1` release run fail on that boundary. The dirty CI change does not add it.
- README says release installers deliver and register Chrome, Edge, and Chromium, but release staging and packaged installers contain only `extension/dist-chrome` and Chrome registration. Source `setup.sh` does support all three.
- README's “replayed deterministically in seconds” and “Get Started in 10 Seconds” lack checked-in timing receipts. Fixed workflows are model-free, but the “zero-token” phrase has no token telemetry. CDP docs' latency, success, detection-risk, and stealth claims lack benchmarks.
- Architecture and feature docs mix current behavior with stale paths, illustrative code, proposals, and old filenames; `docs/ARCHITECTURE.md` still names deleted extension files. `bindings/node` is an empty placeholder.
- A feature note says an external user ran version `0.2.5`, while the workspace and newest published release are `0.1.0`; this historical claim is not reproducible as current usage.
- The current uncommitted cleanup touches 56 tracked files with 497 insertions and 14,339 deletions; this dossier is the only additional untracked file created here. Reproduce with `git diff --numstat`, `git status --short`, and `git diff --check` (which is clean).
- Workflow selectors depend on third-party DOMs and can drift; current live success rate: UNKNOWN — ask Sarav.
