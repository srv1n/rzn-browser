# Action Surface Quickstart

These snippets mirror the public "act / observe / extract" surface using our new CLI commands.

## 1. Act (single action)

```sh
# Click the Sign In button on example.com
RZN_TRANSPORT=pipe ./target/release/rzn-browser act "Click the Sign In button" --url https://example.com --json
```

## 2. Observe (action candidates)

```sh
RZN_TRANSPORT=pipe ./target/release/rzn-browser observe-llm "actions to sign in" --max 5 --json
```

## 3. Extract structured data

```sh
FIELDS='[{"name":"title"},{"name":"url","attribute":"href"}]'
RZN_TRANSPORT=pipe ./target/release/rzn-browser extract-schema --fields "$FIELDS" --limit 5 --json
```

> Tip: set `LLM_PROVIDER=dummy` for deterministic demos without API keys.
