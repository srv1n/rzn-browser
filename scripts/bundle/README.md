# RZN Browser Bundle (macOS arm64)

This bundle contains:

- `extension/dist-chrome/`: Chrome MV3 extension (load as unpacked)
- `bin/rzn-native-host`: native messaging host (installed by script)
- `bin/rzn-browser`: standalone CLI to run workflows
- `workflows/`: shipped workflow library, including search, social, shopping, research, finance, generated examples, and test/debug flows
- `examples/browser_automation/`: packaged example workflows that install into the builtin catalog
- `AGENTS.md`: local instructions for Codex, Claude Code, and similar coding agents

## Quick Start

1. (If macOS blocks execution) remove quarantine from the unzipped folder:

```bash
xattr -dr com.apple.quarantine .
```

2. Install binaries + native messaging manifest:

```bash
./install-macos.sh
```

3. Load the extension:

- Open `chrome://extensions`
- Enable **Developer mode**
- Click **Load unpacked**
- Select: `~/Library/Application Support/RZN/extension/dist-chrome`
- Confirm extension ID matches: `__RZN_EXTENSION_ID__`

4. Restart Chrome once (recommended).

5. Sanity check wiring:

```bash
./doctor-macos.sh
```

## Agent-Friendly Usage

If you are using Codex, Claude Code, or another agentic coding tool, start from this bundle folder. The bundle includes a local `AGENTS.md` with instructions telling the agent to:

- read this `README.md`
- inspect `workflows/README.md`
- prefer shipped workflows before inventing new ones
- use deterministic `run` commands for known tasks
- use `llm-auto` only for open-ended tasks

This keeps agents grounded in the actual bundle contents instead of guessing.

## Running Shipped Workflows

From this bundle folder:

```bash
rzn-browser workflow list google
```

If the native host is not connecting yet, reload the extension or restart Chrome.

The full top-level `workflows/` tree is bundled. That includes production workflows, generated workflows, and debug/test workflows that existed when the bundle was created.

Useful examples:

```bash
# Google search
rzn-browser run google search --param search_query="rust browser automation"

# Google News
rzn-browser run google news --param search_query="OpenAI"

# YouTube search
./bin/rzn-browser run workflows/youtube/youtube-search.json --param search_query="browser automation"

# Reddit draft comment on first post (safer than submit)
./bin/rzn-browser run workflows/reddit/reddit-first-post-draft-comment.json --param comment_text="Interesting thread."

# Reddit search/profile workflow
./bin/rzn-browser run workflows/reddit/reddit-search-with-profiles.json --param search_query="browser automation"

# Hacker News comment on the current top story (omit item_url to comment on the first front-page item; click Stop at the approval gate to dry-run)
./bin/rzn-browser run workflows/hn/hn-submit-comment.json --param comment_text="Interesting point."

# Amazon product search
./bin/rzn-browser run workflows/amazon/amazon-search.json --param search_query="mechanical keyboard"

# Airbnb search demo (built-in San Francisco example)
./bin/rzn-browser run workflows/airbnb/airbnb-search.json

# App Store search
rzn-browser run appstore search --param app_query="habit tracker"

# Refresh shipped workflows/examples later
rzn-browser workflow pull

# See packaged examples
rzn-browser workflow list examples
```

Additional shipped categories commonly available in this bundle:

- `workflows/google`
- `workflows/bing`
- `workflows/youtube`
- `workflows/reddit`
- `workflows/hn`
- `workflows/amazon`
- `workflows/airbnb`
- `workflows/appstore`
- `workflows/g2`
- `workflows/capterra`
- `workflows/etsy`
- `workflows/pubmed`
- `workflows/sciencedirect`
- `workflows/arxiv`
- `workflows/finance`
- `workflows/generated`
- `workflows/tests`

To inspect what is available:

```bash
find workflows -maxdepth 2 -type f -name '*.json' | sort
```

Read `workflows/README.md` for more examples and notes on write-capable workflows.

## LLM Mode

For tasks without a matching workflow:

```bash
./bin/rzn-browser llm-auto "Search Google for browser automation tools and summarize the first page"
```

Use workflow mode when there is already a good shipped example. It is more deterministic and easier for agents to reuse.

## Why You Might See Two Extensions

Chrome can show two copies if you loaded the extension twice (from two folders) or if you have a second RZN-related extension installed (e.g. desktop-app wiring).

If both are enabled at the same time, each can try to launch its own native runtime path and fight over the same local socket.

This bundle targets the deterministic unpacked ID `__RZN_EXTENSION_ID__` and the native host name `__RZN_NATIVE_HOST_NAME__`.
