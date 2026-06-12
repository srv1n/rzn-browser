# CLI Cheatsheet

Use this when you need exact RZN Browser command forms.

## Catalog

```bash
rzn-browser list
rzn-browser list <system>
rzn-browser list <system> <workflow>
rzn-browser list --source builtin
rzn-browser list --source user -v
rzn-browser list <system> --all-sources
rzn-browser list <system> --json
```

`rzn-browser list <system> <workflow>` is the fastest way to get params, examples, returns, notes, and the source workflow file.

## Run Deterministic Workflows

```bash
rzn-browser run <system> <workflow> --param key="value"
rzn-browser run <system>/<workflow> --param key="value"
rzn-browser run /absolute/path/to/workflow.json --param key="value"
```

Examples:

```bash
rzn-browser run google search --param search_query="browser automation"
rzn-browser run google search --param query="AI regulation" --param vertical=news
rzn-browser run chatgpt recent-chats-v1 --param limit="10" --param days="7"
rzn-browser run claude recent-chats --param limit="5"
rzn-browser run x search-posts --param handle="openai"
```

Runtime flags:

```bash
rzn-browser run <system> <workflow> --snapshot on-error
```

`rzn-browser run` uses the local supervisor. The old native/desktop worker backends have been removed.

## Show And Validate Workflows

```bash
rzn-browser workflow show <system> <workflow>
rzn-browser workflow show <system> <workflow> --json
rzn-browser workflow validate <system> <workflow>
rzn-browser workflow validate /path/to/workflow.json --write-help
rzn-browser workflow dirs
rzn-browser workflow pull
```

`--write-help` is for authoring. Review its output before treating the workflow as done.

## User Catalog

Import a local workflow:

```bash
rzn-browser workflow add ~/Downloads/my-flow.json --system custom --name my-flow
rzn-browser run custom my-flow
```

Overwrite intentionally:

```bash
rzn-browser workflow add ~/Downloads/my-flow.json --system custom --name my-flow --force
```

## llm-auto

```bash
rzn-browser llm-auto "Natural language browser task"
rzn-browser llm-auto "Natural language browser task" --json
rzn-browser llm-auto "Natural language browser task" --url "https://example.com" --max-steps 12
rzn-browser llm-auto "Natural language browser task" --context "Use the signed-in account already open in Chrome"
rzn-browser llm-auto "Natural language browser task" --constraint "Do not submit forms or send messages"
rzn-browser llm-auto "Natural language browser task" --save-workflow true --name "system-task-name"
```

Use `--prefer-cached false` when testing raw autonomous behavior:

```bash
rzn-browser llm-auto "Find the first visible pricing plan" --prefer-cached false
```

Use dummy mode only for smoke checks that should avoid provider calls:

```bash
LLM_PROVIDER=dummy rzn-browser llm-auto "Search Google for OpenAI" --max-steps 10
```

## Logs

```bash
make logs-show
make logs-follow
make logs-clear
```

Direct log file:

```bash
tail -n 200 ~/rzn_build.log
```
