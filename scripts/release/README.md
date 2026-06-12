# RZN Browser Release Payload

This archive is a self-contained runtime payload for a single platform release.

## Contents

| Path | Purpose |
| --- | --- |
| `bin/rzn-browser*` | Main CLI/runtime entrypoint |
| `bin/rzn-native-host*` | Chrome native messaging host |
| `extension/dist-chrome/` | Unpacked extension bundle to load in Chrome |
| `workflows/` | Shipped workflow catalog |
| `examples/browser_automation/` | Packaged examples |
| `release-manifest.json` | Build metadata for this bundle |

## Install

- macOS / Linux:

```sh
./install.sh
```

- Windows:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1
```

The installer copies the runtime into a stable local directory, exposes `rzn-browser` on PATH,
installs the Chrome native-host registration, and refreshes the bundled workflow/example catalog.
