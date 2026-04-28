# Workflow Authoring

Use this reference when creating, editing, or validating workflow JSONs.

## Start Here

1. Run `./skills/rzn-workflow-builder/scripts/ensure-runtime.sh`.
2. Inspect the closest shipped workflow under `workflows/<system>/`.
3. Choose deterministic editing or `llm-auto` discovery.

## Most Useful Commands

```bash
# inspect shipped packs
rzn-browser list
rzn-browser list google

# run a built-in workflow
rzn-browser run google search --param search_query="browser automation"

# run a workflow by file path while iterating
rzn-browser run workflows/google/google-search.json --param search_query="browser automation"

# import a finished local JSON into the user catalog
rzn-browser workflow add /abs/path/to/workflow.json --system custom --name my-flow

# rerun an imported workflow by id
rzn-browser run custom my-flow
```

## Discovery Loop With `llm-auto`

Use this when the flow is not yet deterministic.

```bash
./skills/rzn-workflow-builder/scripts/discover-workflow.sh "Search Google for browser automation and extract the top results"
```

Notes:

- The helper defaults to `LLM_PROVIDER=dummy` unless the environment already sets a provider.
- Saved flows land in `workflows/generated/` by default.
- Treat saved workflows as drafts. Clean them before promoting them into a real pack.

## Pattern Selection

### Read-only extractors

Use this shape for search, extraction, research, and catalog flows:

- navigate
- wait for surface
- extract structured data
- return compact JSON

Good examples:

- `workflows/google/google-search.json`
- `workflows/amazon/amazon-search.json`
- `workflows/pubmed/pubmed-search.json`

### Current-tab signed-in flows

Use `use_current_tab: true` when the active authenticated tab is the whole point.

Good examples:

- `workflows/chatgpt/chatgpt_new_chat_send_v1.json`
- `workflows/x/x_open_inbox_v1.json`
- `workflows/instagram/instagram-profile-recent-posts.json`

Use this only when the workflow genuinely needs the live signed-in tab. Do not force current-tab for generic public extraction.

### Real write flows

For send/post/reply/submit actions:

- make the draft state explicit
- assert the control is actionable before the final click
- add `request_user_intervention`
- mention clearly that the workflow performs a real write

Good examples:

- `workflows/x/x_reply_post_v1.json`
- `workflows/hn/hn-submit-link-post.json`

## Current Tab vs Dedicated Tab

Prefer dedicated tabs when:

- the workflow is batch-oriented
- it should not steal the operator's active tab
- it can run safely in isolation

Prefer `use_current_tab: true` when:

- the site only behaves correctly in the live authenticated tab
- the workflow is review-style
- the user is already on the target surface and wants the agent to continue there

## Validation Loop

Use the smallest loop possible:

1. run the workflow
2. inspect the failure
3. patch one thing
4. rerun

Useful runtime notes:

- `rzn-browser list google` checks that the CLI is installed and the catalog resolves.
- `make doctor` checks native-host wiring and manifest state.
- `~/rzn_build.log` is the main unified log file.

## Promotion Rule

Do not promote a generated workflow just because it ran once.

Promote it only when:

- the outcome is clear
- params are named cleanly
- steps are deterministic enough to rerun
- the workflow belongs to a stable pack
