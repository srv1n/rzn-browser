# Workflow Authoring

Use this reference when creating, editing, or validating workflow JSONs.

## Start Here

1. Run `./skills/rzn-workflow-builder/scripts/ensure-runtime.sh`.
2. Inspect the closest shipped workflow under `workflows/<system>/`.
3. Choose deterministic editing or `llm-auto` discovery.

Production workflows use one file: the normal workflow JSON path under
`workflows/<system>/`. The JSON body must declare
`"schema_version": "rzn.workflow_manifest"`. Do not create production
`*.manifest.json` or `*.manifest.v2.json` sidecars, and do not maintain V1/V2
pairs.

Treat `rzn-browser workflow inspect <system> <workflow> --json` as the
handoff contract. If another agent cannot discover the params, types,
side-effects, runtime, and output shape from inspect output, the workflow is
not ready.

## Most Useful Commands

```bash
# inspect shipped packs
rzn-browser list
rzn-browser list google

# inspect the manifest contract agents will call
rzn-browser workflow inspect google search
rzn-browser workflow inspect google search --json

# validate one workflow and the production catalog
rzn-browser workflow validate workflows/google/google-search.json --strict --json
rzn-browser workflow validate-catalog --strict --json

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

### Authenticated dedicated-tab flows

Most authenticated workflows should still open a dedicated workflow tab while
reusing the operator's Chrome profile. That keeps runs parallel-safe without
asking the agent to steal the active tab.

Good examples:

- `workflows/chatgpt/chatgpt_send.json`
- `workflows/x/x_open_inbox.json`
- `workflows/instagram/instagram-profile-recent-posts.json`

Use `runtime.requires_existing_session: true` only when the workflow genuinely
continues an already-open browser state. Do not add active-tab fields to
production workflow JSON.

### Real write flows

For send/post/reply/submit actions:

- make the draft state explicit
- assert the control is actionable before the final click
- add `request_user_intervention`
- mention clearly that the workflow performs a real write

Good examples:

- `workflows/x/x_reply_post.json`
- `workflows/hn/hn-submit-link-post.json`

## Dedicated Tab vs Existing Session

Prefer dedicated tabs when:

- the workflow is batch-oriented
- it should not steal the operator's active tab
- it can run safely in isolation

Set `runtime.requires_existing_session: true` only when:

- the site only behaves correctly in the already-open browser session
- the workflow is review-style
- the user is already on the target surface and wants the agent to continue there

Do not add `use_current_tab`, `use_active_tab`, or `current_tab_id` to
production workflow JSON. Those are legacy/manual-debug concepts, not the
authoring standard.

## Validation Loop

Use the smallest loop possible while preserving the manifest gate:

1. validate the file with `rzn-browser workflow validate <path> --strict --json`
2. inspect the contract with `rzn-browser workflow inspect <system> <workflow> --json`
3. run a safe smoke through the normal route
4. inspect the failure
5. patch one thing
6. rerun validation, inspect, and smoke

Useful runtime notes:

- `rzn-browser list google` checks that the CLI is installed and the catalog resolves.
- `make doctor` checks native-host wiring and manifest state.
- `~/rzn_build.log` is the main unified log file.

## Manifest Handoff Gate

Before handing a workflow to another agent or team, verify:

| Check | Required result |
|---|---|
| Canonical file | The production path is still `workflows/<system>/<workflow>.json`; there is no production sidecar. |
| Schema | The file body has `schema_version: "rzn.workflow_manifest"`. |
| Inputs | `workflow inspect` shows complete required/optional params, types, defaults, enums, and sensitivity. |
| Outputs | `workflow inspect` shows the output selector and schema. |
| Effects | Read, external read, network access, browser state, download, file write, auth, external write, and destructive effects are declared honestly. |
| Validation | Per-file strict validation and `workflow validate-catalog --strict --json` pass. |
| Smoke | A safe normal-route smoke has evidence. Mutating flows stop before final submit/send/post unless explicitly approved. |

Param type sanity:

- scalar choices and ids are `string`
- counters are `integer`
- toggles are `boolean`
- structured bags are `object`
- true lists are `array`

The CLI accepts array params as JSON arrays, comma-separated strings, or a
single value, but the manifest type should still reflect the real data model.

## Promotion Rule

Do not promote a generated workflow just because it ran once.

Promote it only when:

- the outcome is clear
- params are named cleanly
- steps are deterministic enough to rerun
- the workflow belongs to a stable pack
- inspect output is enough for an agent to call the workflow without reading the file
- strict validation and safe smoke have both passed
