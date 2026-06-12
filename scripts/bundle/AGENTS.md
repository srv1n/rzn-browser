# Bundle Agent Guide

This directory is a shareable RZN Browser bundle for macOS arm64.

Scope:
- This `AGENTS.md` applies to this folder and everything under it after the bundle is unpacked.

What is included:
- `bin/rzn-native-host`: native messaging host used by the Chrome extension
- `bin/rzn-browser`: standalone CLI for deterministic workflows and LLM-driven automation
- `extension/dist-chrome/`: unpacked Chrome extension to load in Chrome
- `workflows/`: shipped workflow library, including search, social, shopping, research, finance, and generated examples
- `examples/browser_automation/`: packaged example workflows installed into the builtin catalog
- `schema/`: workflow/action schemas

How to get started:
1. Read `README.md` in this folder first.
2. Assume the human wants to use the shipped workflows before inventing a new one.
3. Discover available examples by reading `workflows/README.md` and listing `workflows/*/*.json`.
4. Prefer deterministic workflow execution for known tasks:
   - `rzn-browser run <system> <workflow> --param key="value"`
5. Use LLM mode for open-ended tasks:
   - `rzn-browser llm-auto "<task>"`

Important runtime assumptions:
- Chrome must have loaded `extension/dist-chrome` as an unpacked extension.
- The native host must be installed with `./install-macos.sh`.
- If the native host is not connecting, reload the extension or restart Chrome.

Workflow guidance:
- Reuse the shipped workflows whenever a close match exists.
- Run `rzn-browser workflow list` or `rzn-browser workflow list examples` before inventing a new workflow.
- Search examples live under `workflows/google`, `workflows/bing`, `workflows/youtube`, and `workflows/finance`.
- Social/posting examples live under `workflows/reddit` and `workflows/hn`.
- Shopping/research examples live under `workflows/amazon`, `workflows/airbnb`, `workflows/appstore`, `workflows/g2`, `workflows/capterra`, `workflows/etsy`, `workflows/pubmed`, `workflows/sciencedirect`, and `workflows/arxiv`.
- Generated examples may exist under `workflows/generated`.
- Debug and smoke-test workflows exist under `workflows/tests`; avoid those unless the human is debugging.

Safety:
- Some shipped workflows perform real write actions, including posting comments on Reddit or Hacker News.
- Prefer draft variants when they exist.
- Tell the human before running a workflow that submits content, logs in, or changes state.

When adding or editing workflows:
- Keep JSON indentation at 2 spaces.
- Do not add site-specific hacks when a generic heuristic can work.
- Prefer production workflows under domain folders; keep temporary debugging flows under `workflows/tests`.
