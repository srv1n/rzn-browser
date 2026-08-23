# AI contribution policy

AI-assisted work is allowed.

What is not allowed is submitting code that the human contributor does not understand.

## Required when AI is used

- disclose that AI was used
- disclose the tool or tools used
- summarize the degree of assistance
- verify the final behavior manually when appropriate
- resolve bot or review comments intentionally, not mechanically

## Not required

- full transcripts for every change
- giant prompt dumps
- performative process theater

## Code file size

Keep code files at or below roughly 1,000 lines. Past that, humans and agents
lose grip on the file; prefer extracting a cohesive module.

Exceeding the limit is allowed as a deliberate decision with a stated reason
(generated code, a cohesive protocol or lookup table) recorded in the change or
task evidence. `make check` runs an advisory (non-fatal) scan that lists code
files over the limit that are not on the allowlist.

## Strong preference

Use structured summaries instead of transcript walls.

For non-trivial AI-assisted changes, prefer a Tusker explainer packet over raw transcripts:

```bash
tusker packet <TASK-ID> --for explainer --write
```

It should help the human explain the change in their own words; it is not proof by itself.
