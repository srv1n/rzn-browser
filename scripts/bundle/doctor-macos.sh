#!/bin/bash
set -euo pipefail

HOST_NAME="__RZN_NATIVE_HOST_NAME__"
EXTENSION_ID="__RZN_EXTENSION_ID__"

INSTALL_ROOT="${RZN_BUNDLE_INSTALL_ROOT:-${HOME}/Library/Application Support/RZN}"
BIN_DIR="${INSTALL_ROOT}/bin"
NATIVE_HOST_PATH="${BIN_DIR}/rzn-native-host"
CLI_PATH="${BIN_DIR}/rzn-browser"
EXT_DIR="${INSTALL_ROOT}/extension/dist-chrome"

CHROME_HOST_DIR="${RZN_BUNDLE_CHROME_HOST_DIR:-${HOME}/Library/Application Support/Google/Chrome/NativeMessagingHosts}"
MANIFEST_PATH="${CHROME_HOST_DIR}/${HOST_NAME}.json"

echo "RZN Bundle Doctor (macOS)"
echo "Host name     : ${HOST_NAME}"
echo "Expected ext  : ${EXTENSION_ID}"
echo "Manifest path : ${MANIFEST_PATH}"
echo ""

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "[FAIL] Not macOS (uname -s=$(uname -s))"
  exit 1
fi

if [[ "$(uname -m)" != "arm64" ]]; then
  echo "[WARN] This bundle is built for arm64; uname -m=$(uname -m)"
fi

if [[ ! -x "${NATIVE_HOST_PATH}" ]]; then
  echo "[FAIL] Missing installed native host binary: ${NATIVE_HOST_PATH}"
  echo "       Run: ./install-macos.sh"
  exit 1
fi
echo "[OK] Native host exists : ${NATIVE_HOST_PATH}"

if [[ ! -x "${CLI_PATH}" ]]; then
  echo "[FAIL] Missing installed CLI binary: ${CLI_PATH}"
  echo "       Run: ./install-macos.sh"
  exit 1
fi
echo "[OK] CLI binary exists   : ${CLI_PATH}"

if [[ ! -f "${EXT_DIR}/manifest.json" ]]; then
  echo "[FAIL] Missing installed extension directory: ${EXT_DIR}"
  echo "       Run: ./install-macos.sh"
  exit 1
fi
echo "[OK] Extension copy exists: ${EXT_DIR}"

if [[ ! -f "${MANIFEST_PATH}" ]]; then
  echo "[FAIL] Native host manifest missing: ${MANIFEST_PATH}"
  echo "       Run: ./install-macos.sh"
  exit 1
fi

echo "[OK] Found native host manifest"
export MANIFEST_PATH
python3 - <<'PY'
import json, os, sys
p = os.environ["MANIFEST_PATH"]
d = json.load(open(p))
print("name          :", d.get("name"))
print("path          :", d.get("path"))
print("allowed_origins:", d.get("allowed_origins"))
PY

echo ""
if [[ -S "/tmp/rzn.sock" ]]; then
  echo "[OK] /tmp/rzn.sock exists"
  python3 - <<'PY'
import socket
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(0.2)
try:
  s.connect("/tmp/rzn.sock")
  print("[OK] /tmp/rzn.sock is connectable")
except Exception as e:
  print("[WARN] /tmp/rzn.sock exists but connect failed:", e)
finally:
  try: s.close()
  except Exception: pass
PY
else
  echo "[WARN] /tmp/rzn.sock missing"
  echo "       Chrome launches the native host when the extension connects."
  echo "       Actions: open Chrome, reload the extension, or restart Chrome."
fi

echo ""
echo "[INFO] If Chrome shows two RZN extensions, remove/disable the one whose ID is not ${EXTENSION_ID} for this bundle."
