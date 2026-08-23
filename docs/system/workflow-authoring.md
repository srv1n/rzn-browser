---
title: "Workflow authoring"
subject: workflow-authoring
keywords: [workflow, JSON, catalog, authoring]
part_of: overview
read_when: "You need to add or change a workflow."
skip_when: "You need runtime or extension code. Open architecture."
---

# Workflow authoring

A workflow is one JSON file that describes a repeatable browser task. Start
from a nearby workflow and keep the task specific.

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

Mark secret inputs as sensitive. Do not write secret values to the file or a
log.

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
the draft or review step unless the operator approved the final action.

## Submit

Include the workflow JSON, pack documentation when needed, a focused check or
smoke result, the browser and login state used, and the input class. Do not
include page content, credentials, or private account data.
