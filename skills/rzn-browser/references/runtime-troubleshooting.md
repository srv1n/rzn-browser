# Runtime troubleshooting

Diagnose the failing layer before changing installation state.

```bash
rzn-browser workflow dirs
rzn-browser workflow list google search
rzn-browser browser targets --json
rzn-browser supervisor status --json
```

Catalog lookup establishes CLI/catalog availability, not browser connectivity.
Inspect target and supervisor output for the actual bridge failure.

## Extension or native-host connection

For a missing/disconnected extension, keep a normal Chrome window open and reload
RZN in `chrome://extensions`. If it is not loaded, use the installed extension:

- macOS: `~/Library/Application Support/RZN/extension/dist/chrome`
- Linux: `~/.local/share/RZN/extension/dist/chrome`
- Windows: `%LOCALAPPDATA%\RZN\extension\dist\chrome`

These are defaults; a custom installation may use a different path. Verify the
installed location before directing a reload. Recheck connectivity after repair.
Do not replace the user's profile to make a failed connection test pass.

In a source checkout, `make doctor` inspects wiring. Use `make install` only when
installation or repair is in scope; a failed workflow is not itself a reason to
reinstall. If manual browser action is required, give the exact action and resume
the requested work after it is complete.

## Provider failures

Inspect the selected provider and whether its required variables are set; never
print API keys or dump the environment. Real `llm-auto` needs a configured
provider. Dummy mode can check local plumbing but cannot validate real exploration.
A provider failure does not require reinstalling the browser.

## Bad results or ambiguous writes

Inspect parameters, the selected workflow file, and the returned error/fields.
Retry a read only when a corrected input or state makes a different result
plausible. Inspect page state before retrying a write that may have succeeded.
For authoring fixes, keep local validation and live browser evidence distinct.
