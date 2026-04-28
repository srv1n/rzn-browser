# Minimal Tauri Integration (Desktop Bridge)

Goal: show how a Tauri backend can talk to the running RZN desktop bridge and drive the browser via the extension, without WebDriver.

## What you get
- A single Rust entry point you can copy into a Tauri command handler: connect → navigate → deterministic extract (validated plan).
- Works with the same transport as the CLI (`pipe` by default; `tcp` optionally).

## Quickstart (from this repo)
1) Ensure the extension + native host are installed and the runtime bridge can be reached.
2) Run the example:

```sh
RZN_TRANSPORT=pipe cargo run -p rzn_sdk --example desktop_bridge_sdk -- "https://example.com"
```

## Using inside a Tauri app
In your Tauri backend (Rust):
- Add `rzn_sdk` as a dependency (path dependency in your workspace or via git).
- Prefer `rzn_sdk::host::DesktopSession` (policy-gated deterministic surface) for a safer default embedding surface.
- For extraction, prefer `execute_extraction_plan` (validated, deterministic) over arbitrary JS.

### Policy gates
High-risk actions (uploads, cookie/localStorage mutation, arbitrary JS) are blocked or require explicit user confirmation.
- Dev escape hatch: set `RZN_POLICY_AUTO_APPROVE=1` to auto-approve confirmations (not recommended for production).

### Protocol examples (non-Rust callers)
The runtime bridge speaks **length-prefixed JSON** over either:
- TCP: `127.0.0.1:30123`
- Pipe: `rzn.sock` (see `rzn_plan::broker_client` for platform details)

Below are the JSON shapes you send **inside** the length-prefixed frames.

**1) Observe (execute_static)**
```json
{
  "action": "execute_static",
  "task_id": "obs-123",
  "data": {
    "cmd": "observe",
    "payload": { "instruction": "find product cards", "scope_selector": "#cards", "max_items": 10 }
  }
}
```

**2) Deterministic extraction (execute_extraction_plan)**
```json
{
  "action": "execute_static",
  "task_id": "explan-123",
  "data": {
    "cmd": "execute_extraction_plan",
    "payload": {
      "plan": {
        "version": 1,
        "mode": "list",
        "scope": { "css": "#cards" },
        "item_selector": ".card",
        "limit": 20,
        "fields": [
          { "name": "title", "selector": ".title" },
          { "name": "price", "selector": ".price", "optional": true }
        ]
      }
    }
  }
}
```

**3) Single action (execute_step)**
```json
{
  "action": "execute_static",
  "task_id": "step-123",
  "data": {
    "cmd": "execute_step",
    "payload": { "step": { "type": "click_element", "selector": "button[type='submit']" } }
  }
}
```

## Troubleshooting
- **“Bridge connection failed”**: open Chrome, enable the extension, and ensure the native messaging host is installed (Chrome launches the native host when the extension connects).
- **Nothing happens in the browser**: check the extension service worker logs in `chrome://extensions` → “Inspect views”.
- **Actions fail with `event.isTrusted`-style issues**: the extension should escalate to CDP for input when needed; if not, enable CDP in flags (see `extension/src/config/flags.ts`).
