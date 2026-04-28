# LLM Scoped Workflow

Use these single-path commands to map the repo, query high-signal areas, and keep changes inside a safe shortlist.

- make scope: builds docs/index/ with TREE, HOTSPOTS, CONTEXT_SNIPPETS, REDUCERS_INDEX, INVARIANTS, SUMMARY.
- make scope-q Q="...": fast ripgrep over scoped files from docs/context/llm_scope.yml.
- make sg-guards: schema DDL guard outside migrations (STRICT=1 to fail).
- Scoped agent flow:
  - make agent-run M="Fix X in Y" [S=1]
  - edit
  - make agent-validate OUT=docs/index/agent_runs/<timestamp> STRICT=1

Notes
- Adjust include/exclude globs in docs/context/llm_scope.yml if needed.
- ast-grep is optional; if installed, `make sg-find-stream` yields AST-precise streaming finds.
- Universal Ctags is optional.
