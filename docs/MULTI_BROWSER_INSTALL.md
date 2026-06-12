# Multi-Browser Install and Targeting

This guide covers Chrome, Chromium, and Microsoft Edge. Firefox is not part of this runtime path.

## Chrome-Only Path

Existing Chrome-only users can keep the short path:

```sh
rzn-browser native-host install --browser chrome
```

By default this allows the pinned development extension origin:

```text
chrome-extension://bogjdnehdficgkhklinmnbgiiofbamji/
```

Then load the Chrome extension build:

| install type | unpacked extension path |
| --- | --- |
| release payload | `<runtime-root>/extension/dist/chrome` |
| source build | `extension/dist/chrome` |

Open `chrome://extensions`, enable Developer mode, click `Load unpacked`, and select the path above.

Check it:

```sh
rzn-browser native-host doctor --browser chrome

rzn-browser browser targets
```

Pick the default target used by no-flag commands:

```sh
rzn-browser browser set chrome
rzn-browser browser default
```

No-target commands also work without a saved default only when exactly one bridge is connected. With multiple connected browser bridges, save a default or pass `--browser`, `--browser-instance`, or `--bridge`; the runtime fails with `AMBIGUOUS_BROWSER_TARGET` instead of guessing.

```sh
rzn-browser run google search --param search_query="browser automation"
```

## Chrome + Edge Setup

Build or install all extension bundles, then load each browser-specific unpacked extension:

| browser | extension page | source build path |
| --- | --- | --- |
| Chrome | `chrome://extensions` | `extension/dist/chrome` |
| Edge | `edge://extensions` | `extension/dist/edge` |
| Chromium | `chrome://extensions` | `extension/dist/chromium` |

For local dev/unpacked builds, the shared manifest key pins the same extension ID across Chrome, Edge, and Chromium:

```text
bogjdnehdficgkhklinmnbgiiofbamji
```

Register the same native-host binary for all three browser targets:

```sh
rzn-browser native-host install --browser chrome,edge,chromium
```

One native-host binary is shared by multiple browser registrations. The per-browser registration is just a small native messaging manifest pointing Chrome, Chromium, or Edge at the same `rzn-native-host` executable with the allowed extension origins for that browser.

## Registration Locations

Use `rzn-browser native-host list --json` for exact paths on the current machine. The usual user-level locations are:

| OS | Chrome | Chromium | Edge |
| --- | --- | --- | --- |
| macOS | `~/Library/Application Support/Google/Chrome/NativeMessagingHosts` | `~/Library/Application Support/Chromium/NativeMessagingHosts` | `~/Library/Application Support/Microsoft Edge/NativeMessagingHosts` |
| Linux | `~/.config/google-chrome/NativeMessagingHosts` | `~/.config/chromium/NativeMessagingHosts` | `~/.config/microsoft-edge/NativeMessagingHosts` |
| Windows | `HKCU\Software\Google\Chrome\NativeMessagingHosts` | `HKCU\Software\Chromium\NativeMessagingHosts` | `HKCU\Software\Microsoft\Edge\NativeMessagingHosts` |

The manifest name is `com.rzn.browser.broker`.

## Target Selection

List connected bridges:

```sh
rzn-browser browser targets
rzn-browser browser list
rzn-browser browser targets --json
```

Typical compact output includes each bridge's browser target, browser instance ID, bridge ID, extension ID, active sessions, the saved default, and example `browser set` commands.

Set, inspect, or clear the default used when a command does not pass a target flag:

```sh
rzn-browser browser set chromium
rzn-browser browser set edge
rzn-browser browser set --browser-instance <browser-instance-id>
rzn-browser browser set --bridge <bridge-id>
rzn-browser browser default
rzn-browser browser clear
```

Execution target precedence is:

1. explicit `--bridge`
2. explicit `--browser-instance`
3. explicit `--browser`
4. saved default from `rzn-browser browser set ...`
5. fail with target choices when more than one connected bridge remains possible

So if the saved default is Chromium but Chromium is not connected, an untargeted command only proceeds when exactly one bridge is connected. Explicit selectors remain strict: `--browser edge` still fails if it matches multiple Edge bridges, and `--browser-instance` or `--bridge` is the durable fix for that case.

Use the broadest target that is still unambiguous:

```sh
# route by browser kind
rzn-browser run google search --browser edge --param search_query="browser automation"

# route by stable browser instance from `browser targets`
rzn-browser run google search \
  --browser-instance <browser-instance-id> \
  --param search_query="browser automation"

# route by exact live bridge when debugging
rzn-browser supervisor call browser.snapshot --bridge <bridge-id>
```

Tab-scoped calls can use composite tab refs returned by browser results:

```sh
rzn-browser supervisor call browser.snapshot \
  --tab-ref rzn://browser/<browser-instance-id>/tab/<tab-id>
```

## Doctor Commands

Run doctor once per browser:

```sh
rzn-browser native-host doctor --browser chrome

rzn-browser native-host doctor --browser edge --json
```

Doctor checks the native-host manifest location, manifest JSON, allowed origin, host executable path, native-host self-test, supervisor socket/token files, and currently connected bridge identity. It does not print token values.

## Troubleshooting

| symptom | likely cause | fix |
| --- | --- | --- |
| `wrong extension ID` or doctor reports `allowed_origin` failed | The loaded unpacked extension does not match the pinned dev key, or you are testing a store build. | For dev, reload the current `extension/dist/<browser>` bundle. For store builds, rerun `native-host install` with the explicit store `--extension-origin`. |
| missing allowed origin | The native-host manifest exists but does not include the loaded extension origin. | Reinstall with `rzn-browser native-host install --browser <target>` for dev, or pass every required store `--extension-origin`. |
| missing manifest | The browser-specific native messaging manifest was never written, or it was removed. | Run `rzn-browser native-host install --browser <chrome|chromium|edge>`, then rerun doctor. |
| no connected bridge | Browser has not launched the extension's native host, the extension is disabled, or the supervisor is not reachable. | Reload the extension, open a normal page, run `rzn-browser browser targets`, then run `native-host doctor` for that browser. |
| ambiguous browser target | An explicit selector matches more than one bridge, such as two connected profiles for the same browser kind. | Run `rzn-browser browser targets`, then add `--browser-instance` or `--bridge`. Prefer `--browser-instance` for durable scripts. |
| `Unknown supervisor method: browser.targets` | The CLI was upgraded while an older supervisor process was still running. | Newer CLIs probe the connected bridge and then fall back to `runtime.status` for listing. For the clean path, stop the old supervisor with `rzn-browser supervisor call runtime.shutdown`, then rerun `rzn-browser browser targets` so the new supervisor starts. |
| target row shows `<unknown>` identity fields | The older supervisor or loaded extension did not return browser identity in the readiness ping. | Reload the unpacked extension, then run `rzn-browser browser targets` again. If it still shows unknowns, run `rzn-browser supervisor call runtime.shutdown` and retry. |

## Extension ID Notes

Dev/unpacked builds use the pinned manifest key and therefore the deterministic ID `bogjdnehdficgkhklinmnbgiiofbamji`. Store-published Chrome and Edge IDs can differ; register those origins explicitly for release builds instead of assuming they match the dev ID.
