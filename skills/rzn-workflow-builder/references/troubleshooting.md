# Troubleshooting

Use this only after a runtime or validation failure. An edit does not need a
healthy live browser until its runtime check.

```bash
rzn-browser workflow dirs
rzn-browser workflow list google search
rzn-browser browser targets --json
rzn-browser supervisor status --json
```

Catalog success verifies discovery; target/supervisor output establishes bridge
state. In a source checkout, `make doctor` checks local wiring. Installation via
`make install` is appropriate when setup or repair is part of the request, not
as a reflex after any failed command.

For an extension connection failure, keep Chrome open and reload RZN from
`chrome://extensions`. Default unpacked paths are:

- macOS: `~/Library/Application Support/RZN/extension/dist/chrome`
- Linux: `~/.local/share/RZN/extension/dist/chrome`
- Windows: `%LOCALAPPDATA%\RZN\extension\dist\chrome`

Verify custom install paths before asking for a reload. Preserve the user's
profile; a new profile does not test the requested session.

For a provider failure, check the selected provider and presence of its required
configuration without printing secrets. Dummy mode cannot prove live discovery.
If a command rejects an option, consult its `--help`; older discovery helpers
may assume options that the current CLI no longer supports.

For a workflow error, inspect the exact file and failing step, correct the
parameter/selector/state issue, and rerun the affected check. Inspect page state
before repeating an ambiguous write. Report offline and live evidence separately.
