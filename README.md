# RZN Browser

[![CI](https://github.com/srv1n/rzn-browser/actions/workflows/ci.yml/badge.svg)](https://github.com/srv1n/rzn-browser/actions/workflows/ci.yml)

**Zero-token browser automation.**

- Google, Maps, Scholar, and Trends searches
- G2, Capterra, Amazon, and App Store reviews
- Goodreads books, ratings, and review histograms
- LinkedIn feed, profiles, people search, and jobs
- Reddit threads, X posts, Hacker News
- Your own ChatGPT and Claude chat logs
- Meta Ad Library and Google Ads Transparency

Claude Code, Codex, Gemini CLI, OpenCode, or any agent that can run a CLI drives these as one-command, pre-built workflows. Zero extra tokens; the page is never re-derived.

- ~100 workflows across 25 sites, replayed deterministically in seconds
- Your signed-in Chrome: real sessions, real logins
- Chrome extension first; short CDP fallback
- An LLM figures a site out once (`llm-auto`); replays never need a model

**Not for** mass scraping or anonymous crawling. Personal automation: one signed-in person's repeatable tasks.

<p>
  <img src="docs/visuals/01-product-overview.png" alt="RZN Browser product overview showing the CLI, runtime, native host, Chrome extension, and target web app" width="100%">
</p>

## Workflow Packs

RZN ships with roughly 100 built-in workflows across 25 sites. For sites that need authentication, assume you are already signed in and use your normal browser session.

### Search & Research

| Icon | System | Capabilities |
| --- | --- | --- |
| <img alt="Google" src="https://www.google.com/s2/favicons?sz=64&domain=google.com" width="24" height="24"> | Google | Search, Maps, Maps directions, Scholar, Images, Lens, Trends, Flights, Hotels, Weather, Finance, Translate |
| <img alt="Bing" src="https://www.google.com/s2/favicons?sz=64&domain=bing.com" width="24" height="24"> | Bing | Web, news, video, and image search, plus bulk image download |
| <img alt="YouTube" src="https://www.google.com/s2/favicons?sz=64&domain=youtube.com" width="24" height="24"> | YouTube | Search, channel browsing, playlist contents, and watch-page metadata with recommendations and comments |
| <img alt="PubMed" src="https://www.google.com/s2/favicons?sz=64&domain=pubmed.ncbi.nlm.nih.gov" width="24" height="24"> | PubMed | Search and paper extraction |
| <img alt="arXiv" src="https://www.google.com/s2/favicons?sz=64&domain=arxiv.org" width="24" height="24"> | arXiv | Search and preprint extraction |
| <img alt="ScienceDirect" src="https://www.google.com/s2/favicons?sz=64&domain=sciencedirect.com" width="24" height="24"> | ScienceDirect | Search and paper access workflows |
| <img alt="Goodreads" src="https://www.google.com/s2/favicons?sz=64&domain=goodreads.com" width="24" height="24"> | Goodreads | Book search, book detail with rating histogram, shelf ranking, reviews, and similar-book lists |
| <img alt="Libgen" src="https://www.google.com/s2/favicons?sz=64&domain=libgen.li" width="24" height="24"> | Libgen | Mirror search and direct download resolution |
| <img alt="Yahoo Finance" src="https://www.google.com/s2/favicons?sz=64&domain=finance.yahoo.com" width="24" height="24"> | Yahoo Finance | Quote lookup for stocks, ETFs, indices, crypto, forex, and futures, plus ticker news |

### Shopping, Travel & Reviews

| Icon | System | Capabilities |
| --- | --- | --- |
| <img alt="Amazon" src="https://www.google.com/s2/favicons?sz=64&domain=amazon.com" width="24" height="24"> | Amazon | Product search, key facts, and review extraction |
| <img alt="Etsy" src="https://www.google.com/s2/favicons?sz=64&domain=etsy.com" width="24" height="24"> | Etsy | Listing search and review extraction |
| <img alt="G2" src="https://www.google.com/s2/favicons?sz=64&domain=g2.com" width="24" height="24"> | G2 | Product search, details, and review extraction |
| <img alt="Capterra" src="https://www.google.com/s2/favicons?sz=64&domain=capterra.com" width="24" height="24"> | Capterra | Product search, details, and review extraction |
| <img alt="App Store" src="https://www.google.com/s2/favicons?sz=64&domain=apps.apple.com" width="24" height="24"> | App Store | App search and app details |
| <img alt="Airbnb" src="https://www.google.com/s2/favicons?sz=64&domain=airbnb.com" width="24" height="24"> | Airbnb | Search workflows |

### Ads & Market Intelligence

| Icon | System | Capabilities |
| --- | --- | --- |
| <img alt="Meta Ad Library" src="https://www.google.com/s2/favicons?sz=64&domain=facebook.com" width="24" height="24"> | Meta Ad Library | Search live ads by keyword or by advertiser |
| <img alt="Google Ads Transparency" src="https://www.google.com/s2/favicons?sz=64&domain=google.com" width="24" height="24"> | Google Ads Transparency | List an advertiser's ads from the Transparency Center |
| <img alt="Apple Ads" src="https://www.google.com/s2/favicons?sz=64&domain=ads.apple.com" width="24" height="24"> | Apple Ads | Keyword suggestions with popularity signals, recommendation cards, and campaign/ad-group reports for the signed-in org |

Meta Ad Library and Google Ads Transparency emit a shared ad manifest shape, so results from both can be compared without per-pack conversion.

### AI Apps

| Icon | System | Capabilities |
| --- | --- | --- |
| <img alt="ChatGPT" src="https://www.google.com/s2/favicons?sz=64&domain=chatgpt.com" width="24" height="24"> | ChatGPT | Send to a new or existing chat with attachments, model and effort selection, and a tool toggle; read transcripts, list recent chats, browse Projects, generate and download images, and resolve artifact URLs |
| <img alt="Claude" src="https://www.google.com/s2/favicons?sz=64&domain=claude.ai" width="24" height="24"> | Claude | Send with attachments and model selection, list recent chats, and export a full thread |

### Social & Communities

| Icon | System | Capabilities |
| --- | --- | --- |
| <img alt="X" src="https://www.google.com/s2/favicons?sz=64&domain=x.com" width="24" height="24"> | X | Home timeline and profile digests, post search, unified read of any post/article/thread as markdown, plus posts, replies, likes, and DMs behind approval gates |
| <img alt="Reddit" src="https://www.google.com/s2/favicons?sz=64&domain=reddit.com" width="24" height="24"> | Reddit | Search, profile history, message inbox, submit in any post kind, comment, reply, vote, flair lookup, and DM drafts |
| <img alt="Hacker News" src="https://www.google.com/s2/favicons?sz=64&domain=news.ycombinator.com" width="24" height="24"> | Hacker News | Submit, comment, reply, and draft-first write flows |
| <img alt="Instagram" src="https://www.google.com/s2/favicons?sz=64&domain=instagram.com" width="24" height="24"> | Instagram | Search accounts/hashtags/places, profile post discovery, post extraction with media and comments, plus follow, like, comment, and DM |
| <img alt="LinkedIn" src="https://www.google.com/s2/favicons?sz=64&domain=linkedin.com" width="24" height="24"> | LinkedIn | Read-only feed digest, people/post/company search, profile read, post permalink with comments, job search, job detail, and your own jobs |

## Best Fit

- Local browser automation from the CLI or MCP
- Built-in workflow packs instead of starting from a blank SDK
- Repeatable browser tasks that you want to save and rerun
- LLM-driven browser tasks when the flow is still ambiguous
- Signed-in product surfaces like ChatGPT, Claude, X, Reddit, LinkedIn, and Instagram
- Ad and market research across Meta, Google, and Apple ad surfaces

## Not For

- Rented cloud browsers, or anything that runs without your own signed-in browser
- Mainstream test automation
- Large-scale anonymous scraping
- Teams looking for a general-purpose browser SDK first

`fleet` and `cloud` do not contradict this. Both coordinate machines you already own — enrolling your own laptops so work can be dispatched to whichever one holds the right session. Every action still runs in a real browser you are signed into.

## How It Works

<p>
  <img src="docs/visuals/02-runtime-architecture.png" alt="RZN Browser runtime architecture showing local entry points, native host, Chrome extension, DOM path, and short CDP fallback path" width="100%">
</p>

- First path: use the extension and page actions.
- Second path: use CDP for the cases the first path cannot handle.

<p>
  <img src="docs/visuals/03-action-escalation.png" alt="Action escalation ladder: same-origin DOM actions first, scripted event fallback second, short CDP attach last" width="100%">
</p>

- Use fixed workflows when the task is known.
- Use `llm-auto` when the task is still fuzzy.

<p>
  <img src="docs/visuals/04-workflow-vs-agent.png" alt="Workflow mode and agent mode converging on the same browser execution stack" width="100%">
</p>

## Get Started in 10 Seconds

1. Install the native binaries:

```sh
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/srv1n/rzn-browser/main/install.sh | sh

# Windows PowerShell
irm https://raw.githubusercontent.com/srv1n/rzn-browser/main/install.ps1 | iex
```

2. Install the browser extension:

- Open `chrome://extensions`
- Enable `Developer mode`
- Click `Load unpacked`
- Pick the installed `extension/dist/chrome` directory for Chrome, or the matching `extension/dist/<browser>` directory for Edge/Chromium

3. Optional but recommended: install the broad Agent Skill so LLMs can use workflows and `llm-auto` cleanly:

```sh
# Global install, linked into Codex, Claude Code, Gemini CLI, and generic Agent Skills paths.
rzn-browser skill install --global

# Or project-local install for the current repo.
rzn-browser skill install --project
```

For workflow authoring specifically, the narrower builder skill is still available:

```sh
bash scripts/install_rzn_workflow_builder_skill.sh --global
```

If you just want to prove it works, run:

```sh
rzn-browser list google
rzn-browser run google search --param search_query="browser automation"
```

## Prereqs

- Google Chrome
- A normal Chrome profile
- Developer mode enabled in `chrome://extensions` so you can load the unpacked extension
- `OPENAI_*` or `GEMINI_*` env vars only if you want `llm-auto`

You do not need:

- Playwright
- Selenium
- A separate CDP install
- A Chrome `--remote-debugging-port` setup
- Rust, Bun, or a local build toolchain unless you are building from source

## Repo Layout

If you are browsing the repository instead of just installing the product, this is the map:

| path | purpose |
| --- | --- |
| `crates/` | Rust runtime crates for the CLI, native host, worker, planner, and supporting systems |
| `extension/` | Chrome extension source, tests, and built browser payloads |
| `workflows/` | Shipped workflow catalog and workflow-specific docs/readmes |
| `scripts/` | Developer tooling, release scripts, guards, and agent utilities |
| `resources/` | Runtime metadata and connector assets such as browser system metadata and social card catalogs |
| `examples/` | Non-core examples, reference integrations, and experimental helpers |
| `bindings/` | Placeholder surface for external language bindings |
| `schema/` | Canonical JSON schemas used across runtime and extension codegen |
| `skills/` | Reusable workflow-oriented skill packs and wrappers |
| `docs/` | Architecture notes, feature scratchpads, visual briefs, and workflow docs |
| `test/` + `tests/` | Manual harnesses, fixtures, and automated tests |

## Installation

Install from the latest GitHub release first. Source installs belong in developer setup, not the first-run path.

```sh
# latest GitHub release on macOS/Linux
curl -fsSL https://raw.githubusercontent.com/srv1n/rzn-browser/main/install.sh | sh

# latest GitHub release on Windows
irm https://raw.githubusercontent.com/srv1n/rzn-browser/main/install.ps1 | iex
```

The installer drops the runtime into a stable local directory, installs the native host, refreshes the built-in workflow catalog, and exposes the CLI binaries on your machine.

To move an existing install to the latest release — binaries, extension, native host, and the bundled workflow catalog in one step:

```sh
rzn-browser update           # install the latest release
rzn-browser update --check   # only report installed vs available
```

`update` runs the same packaged installer the curl one-liner does, after verifying the release artifact's sha256. Workflows you added yourself (`workflow add`) live in the user catalog and are never overwritten by an update.

| OS | runtime root | what lands there |
| --- | --- | --- |
| macOS | `~/Library/Application Support/RZN` | `bin/`, `extension/dist/{chrome,edge,chromium}/`, `workflows/` |
| Linux | `~/.local/share/RZN` | `bin/`, `extension/dist/{chrome,edge,chromium}/`, `workflows/` |
| Windows | `%LOCALAPPDATA%\\RZN` | `bin\\`, `extension\\dist\\{chrome,edge,chromium}\\`, `workflows\\` |

- Runtime binaries installed by the bundle: `rzn-browser`, `rzn-native-host`
- On macOS/Linux the installer also places PATH-facing binaries in `~/.local/bin` or `~/bin` when available.
- On Windows it installs into `%LOCALAPPDATA%\\RZN\\bin` and adds that directory to the user PATH.
- The installer writes Chrome, Edge, and Chromium native messaging registrations for you. To inspect or manage
  registrations yourself, use `rzn-browser native-host list --json`,
  `rzn-browser native-host install --browser chrome,edge,chromium`,
  and `rzn-browser native-host uninstall --browser edge`.
- Chrome already includes CDP support. You are not installing CDP separately.

## Browser Setup

This is the one manual browser step per browser.

1. Open `chrome://extensions`.
2. Enable Developer mode.
3. Click `Load unpacked`.
4. Pick the installed `extension/dist/chrome` directory.

Use these stable paths:

- macOS Chrome: `~/Library/Application Support/RZN/extension/dist/chrome`
- macOS Edge: `~/Library/Application Support/RZN/extension/dist/edge`
- macOS Chromium: `~/Library/Application Support/RZN/extension/dist/chromium`
- Linux: `~/.local/share/RZN/extension/dist/<chrome|edge|chromium>`
- Windows: `%LOCALAPPDATA%\\RZN\\extension\\dist\\<chrome|edge|chromium>`

The expected dev extension ID is `bogjdnehdficgkhklinmnbgiiofbamji`. After the first load, restart the browser once.

Use `rzn-browser browser targets` to list connected Chrome, Edge, and Chromium bridges. No-flag commands use the only connected bridge, or your saved default from `rzn-browser browser set chrome`, `rzn-browser browser set chromium`, or `rzn-browser browser set edge`. If multiple bridges are connected and no saved or explicit target resolves, the command fails with target choices instead of guessing. Explicit `--bridge`, `--browser-instance`, and `--browser` flags still override the saved default.

For Chrome + Edge or Chromium setup, target flags, default selection, doctor output, and native-host troubleshooting, see [Multi-Browser Install and Targeting](docs/MULTI_BROWSER_INSTALL.md).

## First Run

You can call shipped workflows by namespace instead of file path:

- `rzn-browser run google search`
- `rzn-browser run google/search`

Both resolve through the installed workflow catalog.

Example first run:

```sh
rzn-browser list google
rzn-browser run google search --param search_query="browser automation"
```

That should open Google in your existing Chrome session, run the packaged search workflow, and return extracted results.

If you want the same runtime to go autonomous instead of following a fixed workflow:

```sh
rzn-browser llm-auto "Search Google for browser automation and extract the top results"
```

### Run Options Worth Knowing

```sh
# write the result to a file instead of stdout
rzn-browser run x open --param url="https://x.com/..." --output-file thread.md

# download every asset the result references, with a manifest.json alongside
rzn-browser run libgen download --param query="..." --download-dir ./books

# leave the tab open and reuse that exact tab on the next run
rzn-browser run chatgpt send --param message_text="..." --keep-tab-open
rzn-browser run chatgpt read --tab-ref "rzn://browser/<instance>/tab/<id>" --param chat_id="..."

# capture a snapshot when a step fails
rzn-browser run google search --param search_query="..." --snapshot on-error
```

A workflow must declare `file_write` in its side effects to use `--output-file` or `--download-dir`; the CLI refuses the post-processing otherwise rather than writing behind the manifest's back.

## Beyond Workflows

The same runtime exposes a few surfaces that are not fixed workflows:

| command | what it does |
| --- | --- |
| `rzn-browser llm-auto "<task>"` | LLM drives the browser when the flow is still ambiguous |
| `rzn-browser observe` | returns candidate selectors for a page, no LLM required |
| `rzn-browser quick-extract` | fast code-first extraction for common sites |
| `rzn-browser act` / `extract-schema` | single planned action, or structured extraction from a DOM inventory |
| `rzn-browser mcp` | serve the runtime to an MCP-based agent over stdio |
| `rzn-browser fleet enroll` | join this machine to a fleet of your own devices |
| `rzn-browser report workflow-broken` | report a workflow whose selectors have drifted |

The extension also ships a dashboard for runs, logs, workflows, fleet status, and settings — useful when you want to watch a run rather than read its JSON.

## Workflow Catalog

Every shipped system lives under `workflows/<system>/`, and the packs above are the human-readable index. For the authoritative list on your machine — including anything you have imported yourself — ask the CLI rather than a table that can drift:

```sh
rzn-browser list                 # every system
rzn-browser list linkedin        # one system's workflows
rzn-browser list chatgpt send    # full help for one workflow
```

Useful commands:

- `rzn-browser list`
- `rzn-browser list google`
- `rzn-browser list chatgpt continue-chat-v1`
- `rzn-browser list --source builtin`
- `rzn-browser list google --all-sources`
- `rzn-browser list --source user -v`
- `rzn-browser workflow list`
- `rzn-browser workflow list google`
- `rzn-browser workflow show chatgpt send`
- `rzn-browser workflow validate workflows/chatgpt/chatgpt_send.json`
- `rzn-browser workflow validate workflows/chatgpt/chatgpt_send.json --write-help`
- `rzn-browser workflow pull`
- `rzn-browser workflow add ~/Downloads/my-flow.json --system custom --name my-flow`

`rzn-browser list` is the short top-level alias for catalog inspection. `workflow list` still works.

Use progressive disclosure:

- `rzn-browser list <system>` keeps the output compact.
- `rzn-browser list <system> <workflow>` shows the detailed help view for one workflow: what it does, required and optional params, and a runnable example command.
- `rzn-browser workflow show <system> <workflow>` shows the same detailed help view.
- `rzn-browser workflow validate <path-or-id>` checks that the workflow and its help metadata are in sync.
- `rzn-browser workflow validate <path-or-id> --write-help` scaffolds or refreshes the top-level `help` block before validating.

List output defaults to the effective workflow per id. Use the extra flags when you need to inspect collisions or origins:

- `--source user|builtin|legacy`: only show entries from that catalog source
- `--all-sources`: include shadowed entries instead of only the winning one
- `-v`, `--verbose`: show ids, legacy aliases, relative paths, and resolved file paths

## Create Your Own Workflows

The fastest path is:

1. Start from an existing workflow that is close to what you want.
2. Run it against your real browser session.
3. Edit the JSON until the flow is stable.
4. Import it into your local catalog.

Useful commands:

```sh
# inspect the shipped packs
rzn-browser list
rzn-browser list google
rzn-browser list chatgpt send
rzn-browser workflow validate workflows/chatgpt/chatgpt_send.json

# run a built-in workflow
rzn-browser run google search --param search_query="browser automation"

# import your own workflow into the local user catalog
rzn-browser workflow add ~/Downloads/my-flow.json --system custom --name my-flow

# run your imported workflow
rzn-browser run custom my-flow
```

Recommended local loop:

- Start from a nearby JSON in `workflows/<system>/`.
- Keep site-specific selectors and page logic inside the workflow JSON, not in shared engine code.
- Prefer the installed workflow ids like `google search` over repo-relative file paths once the flow is good enough to reuse.
- Add a `help` block for every new workflow so the CLI can explain params and examples without guessing.
- Run `rzn-browser workflow validate <path> --write-help` while authoring. It fills in the boring param docs and then tells you what still needs a human pass.
- If you want the agent to discover a flow first, use `llm-auto` and save the resulting workflow JSON, then clean it up into a deterministic workflow.

Useful references:

- `workflows/README.md` for the shipped catalog layout
- `docs/BROWSER_DEV_LOOP.md` for the workflow factory / save-workflow dev loop
- `docs/workflows/README.md` for the docs structure used by built-in packs
- `rzn-browser skill install --global` if you want an Agent Skill for direct workflow and `llm-auto` use
- `rzn-browser skill update --global` to refresh installed skill links after a release or git checkout update
- `rzn-browser skill remove --global` to remove the managed skill and its symlinks
- `scripts/install_rzn_workflow_builder_skill.sh` if you want to install the narrower workflow-builder skill

## Submit Workflows Back To The Repo

If you want to contribute a workflow pack back upstream, follow this shape:

1. Add the workflow JSON under `workflows/<system>/`.
2. Add or update the docs under `docs/workflows/<system>/`.
3. Keep shared engine code generic. Site-specific selectors belong in the workflow pack.
4. Add a clear example command with required `--param` values.
5. Mention whether the flow is read-only, draft-only, or performs a real write action.

Good submissions usually include:

- One canonical workflow filename per workflow
- A short system README in `docs/workflows/<system>/README.md`
- One markdown doc per workflow under `docs/workflows/<system>/`
- Parameters, output shape, and a runnable example
- A focused validation path, parse test, or smoke workflow when the change is non-trivial

The easiest way to get a workflow accepted is to keep the value obvious:

- one system
- one concrete user outcome
- deterministic steps
- no site-specific hacks in shared runtime code

## Example Flows

- Search and extract results from Google, Bing, or YouTube.
- Reuse a logged-in session on X, Reddit, ChatGPT, LinkedIn, Instagram, or Hacker News.
- Pull an advertiser's live creatives from the Meta Ad Library and Google Ads Transparency Center into one comparable shape.
- Hand a task to ChatGPT with attachments and a chosen model and effort, then read the reply back.
- Interact with UI inside embedded cross-origin frames without swapping tools.
- Draft comments or messages for review before submit.
- Mix deterministic workflows with agent-style tasks in the same browser runtime.

## Integrations

- `Chrome + native messaging host`: local bridge into the browser without launching a separate automation browser
- `Workflow catalog`: built-in flows ship with the runtime; your own JSON flows live alongside them
- `LLM providers`: use `OPENAI_*` or `GEMINI_*` env vars for `llm-auto`, or `LLM_PROVIDER=dummy` for deterministic local runs
- `RZN desktop/runtime`: the standalone CLI can coexist with the same browser-side connection instead of stealing it

## For Design

If the downstream team is turning this into visuals, use [docs/README_VISUAL_BRIEFS.md](docs/README_VISUAL_BRIEFS.md). It includes copy-pasteable write-ups for the product story, runtime architecture, action escalation, and workflow-versus-agent flow.

## Build From Source

```sh
cargo build --release -p rzn-browser -p rzn-native-host
cd extension && bun install && bun run build
```

## Developer Setup

If you are working from the repo instead of trying the product, use:

```sh
make install
```

That builds the local runtime, installs the native host, copies the stable extension payload, and refreshes the bundled workflow catalog.

If you are building from source, you need:

- Rust toolchain / Cargo
- Bun for the extension build
- Google Chrome for loading the unpacked extension

For deeper setup and architecture details, see [docs/BROWSER_DEV_LOOP.md](docs/BROWSER_DEV_LOOP.md), [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), and [docs/REPO_MAP.md](docs/REPO_MAP.md).

## License

Two licenses, split by what the code is for:

- **Runtime** (`crates/`, `extension/`) — GNU AGPLv3 (`AGPL-3.0-only`). See [LICENSE](LICENSE).
- **Workflow catalog** (`workflows/`), **agent skills** (`skills/`), and **schemas** (`schema/`) — MIT. See [workflows/LICENSE](workflows/LICENSE), [skills/LICENSE](skills/LICENSE), [schema/LICENSE](schema/LICENSE).

Workflows and skills exist to be copied, remixed, and redistributed, so they carry a license that gets out of the way.
