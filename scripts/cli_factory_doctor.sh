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
echo "Worker binary discovery:"
FOUND_WORKER=""
for CANDIDATE in \
  "${RZN_BROWSER_WORKER_CMD:-}" \
  "./target/debug/rzn-browser-worker" \
  "./target/release/rzn-browser-worker" \
  "$HOME/Library/Application Support/RZN/bin/rzn-browser-worker"
do
  if [[ -n "$CANDIDATE" && -x "$CANDIDATE" ]]; then
    FOUND_WORKER="$CANDIDATE"
    break
  fi
done

if [[ -n "$FOUND_WORKER" ]]; then
  echo "[OK] Found rzn-browser-worker: $FOUND_WORKER"
else
  echo "[WARN] rzn-browser-worker not found in repo target/ or install dir"
  echo "       Run: cargo build -p rzn-browser-worker"
fi

echo ""
echo "Bridge endpoints (desktop or spawned worker):"
APP_BASE="${APP_BASE:-$HOME/Library/Application Support/rzn_debug}"
ENDPOINT="$APP_BASE/secure/broker_endpoint_v1.json"
if [[ -f "$ENDPOINT" ]]; then
  echo "[OK] Found bridge endpoint: $ENDPOINT"
else
  echo "[WARN] Missing bridge endpoint: $ENDPOINT"
  echo "       Expected flow:"
  echo "       - Desktop app running, OR"
  echo "       - CLI spawn mode creates it under APP_BASE"
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
