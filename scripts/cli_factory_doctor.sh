#!/bin/bash
set -euo pipefail

HOST_NAME="${RZN_NATIVE_HOST_NAME:-com.rzn.browser.broker}"
MANIFEST_NAME="${HOST_NAME}.json"
EXTENSION_ID="${RZN_CHROME_EXTENSION_ID:-bogjdnehdficgkhklinmnbgiiofbamji}"

if [[ "$OSTYPE" == "darwin"* ]]; then
  HOST_DIR="$HOME/Library/Application Support/Google/Chrome/NativeMessagingHosts"
elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
  HOST_DIR="$HOME/.config/google-chrome/NativeMessagingHosts"
elif [[ "$OSTYPE" == "msys"* ]] || [[ "$OSTYPE" == "win32" ]]; then
  HOST_DIR="$APPDATA/Google/Chrome/NativeMessagingHosts"
else
  echo "[ERROR] Unsupported OS: $OSTYPE"
  exit 1
fi

echo "RZN Browser Factory Doctor"
echo "Host: $HOST_NAME"
echo "Manifest: $HOST_DIR/$MANIFEST_NAME"
echo "Expected extension ID: $EXTENSION_ID"
echo ""

if [[ ! -f "$HOST_DIR/$MANIFEST_NAME" ]]; then
  echo "[FAIL] Native host manifest missing: $HOST_DIR/$MANIFEST_NAME"
  echo "       Run: ./setup.sh (or set RZN_SETUP_SKIP_EXT=1 if extension already built)"
  exit 1
fi

echo "[OK] Found native host manifest"
export HOST_DIR
export MANIFEST_NAME
python3 - <<'PY'
import json, os, sys
p = os.path.expanduser(os.environ["HOST_DIR"] + "/" + os.environ["MANIFEST_NAME"])
d = json.load(open(p))
print("name:", d.get("name"))
print("path:", d.get("path"))
print("allowed_origins:", d.get("allowed_origins"))
PY

NATIVE_HOST_PATH="$(python3 - <<'PY'
import json, os
p = os.path.expanduser(os.environ["HOST_DIR"] + "/" + os.environ["MANIFEST_NAME"])
try:
    d = json.load(open(p))
    print((d.get("path") or "").strip())
except Exception:
    print("")
PY
)"
if [[ -n "$NATIVE_HOST_PATH" && -x "$NATIVE_HOST_PATH" ]]; then
  echo "[OK] Native host binary is executable: $NATIVE_HOST_PATH"
else
  echo "[WARN] Native host binary is missing or not executable: ${NATIVE_HOST_PATH:-<empty>}"
fi

echo ""
echo "Supervisor binary discovery:"
FOUND_BROWSER=""
for CANDIDATE in \
  "${RZN_BROWSER_CMD:-}" \
  "./target/debug/rzn-browser" \
  "./target/release/rzn-browser" \
  "$HOME/Library/Application Support/RZN/bin/rzn-browser"
do
  if [[ -n "$CANDIDATE" && -x "$CANDIDATE" ]]; then
    FOUND_BROWSER="$CANDIDATE"
    break
  fi
done

if [[ -n "$FOUND_BROWSER" ]]; then
  echo "[OK] Found rzn-browser: $FOUND_BROWSER"
else
  echo "[WARN] rzn-browser not found in repo target/ or install dir"
  echo "       Run: cargo build -p rzn-browser"
fi

echo ""
echo "Supervisor runtime files:"
APP_BASE="${APP_BASE:-$HOME/Library/Application Support/RZN}"
SOCKET="$APP_BASE/run/supervisor.sock"
TOKEN="$APP_BASE/secure/supervisor.token"
if [[ -S "$SOCKET" && -f "$TOKEN" ]]; then
  echo "[OK] Supervisor socket/token present under: $APP_BASE"
else
  echo "[INFO] Supervisor socket/token not present under: $APP_BASE"
  echo "       Expected flow:"
  echo "       - Run: rzn-browser supervisor ensure-ready"
  echo "       - Open Chrome with the RZN extension enabled so the native host can connect"
fi

echo ""
echo "Desktop app wiring check (should remain intact):"
for DESKTOP_MANIFEST in \
  "$HOST_DIR/com.rzn.browser.broker.json"
do
  if [[ -f "$DESKTOP_MANIFEST" ]]; then
    echo "[OK] Desktop native host manifest exists: $DESKTOP_MANIFEST"
  else
    echo "[INFO] No desktop native host manifest at: $DESKTOP_MANIFEST"
  fi
done
