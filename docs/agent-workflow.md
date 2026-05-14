# Agent workflow

The goal is not to make contribution harder. The goal is to make sloppy contribution harder.

## Principles

- Problems should be legible before implementation starts.
- Review should start with evidence, not archaeology.
- AI is allowed, but slop is not.
- Contributors must understand their own changes.
- Raw transcripts are optional appendix material, not required reading.

## Expected flow

```text
idea / report
   ↓
structured issue
   ↓
approval or maintainer signal
   ↓
implementation
   ↓
PR summary + evidence
   ↓
review
```

## What we want from contributors

Before opening a PR, contributors should be able to explain:

- what changed
- why it changed
- how it was tested
- what the risk is
- what a reviewer should focus on

If AI was used, disclose it. If the contributor cannot explain the final behavior without leaning on the tool, the work is not ready.

## What maintainers should enforce

- keep new features or architecture discussion out of surprise PRs
- reject refactor-only churn unless requested
- ask for evidence on user-visible changes
- keep the bar proportional to risk
