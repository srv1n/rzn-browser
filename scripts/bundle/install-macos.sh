#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

HOST_NAME="__RZN_NATIVE_HOST_NAME__"
EXTENSION_ID="__RZN_EXTENSION_ID__"

INSTALL_ROOT="${RZN_BUNDLE_INSTALL_ROOT:-${HOME}/Library/Application Support/RZN}"
BIN_DIR="${INSTALL_ROOT}/bin"
EXT_DIR="${INSTALL_ROOT}/extension/dist-chrome"
CHROME_HOST_DIR="${RZN_BUNDLE_CHROME_HOST_DIR:-${HOME}/Library/Application Support/Google/Chrome/NativeMessagingHosts}"
MANIFEST_PATH="${CHROME_HOST_DIR}/${HOST_NAME}.json"
GLOBAL_BIN_DIR="${RZN_BUNDLE_GLOBAL_BIN_DIR:-${HOME}/.local/bin}"

install_bin_entry() {
  local src="$1"
  local dest="$2"
  if ln -sfn "$src" "$dest" 2>/dev/null; then
    return 0
  fi
  install_file_atomic "$src" "$dest"
}

install_file_atomic() {
  local src="$1"
  local dest="$2"
  local dest_dir
  local tmp
  dest_dir="$(dirname "$dest")"
  mkdir -p "$dest_dir"
  tmp="$(mktemp "$dest_dir/.tmp.$(basename "$dest").XXXXXX")"
  cp -f "$src" "$tmp"
  chmod +x "$tmp" 2>/dev/null || true
  if command -v xattr >/dev/null 2>&1; then
    xattr -d com.apple.provenance "$tmp" 2>/dev/null || true
    xattr -d com.apple.quarantine "$tmp" 2>/dev/null || true
  fi
  mv -f "$tmp" "$dest"
}

mkdir -p "$BIN_DIR" "$CHROME_HOST_DIR" "$GLOBAL_BIN_DIR"

for required in \
  "${SCRIPT_DIR}/bin/rzn-browser" \
  "${SCRIPT_DIR}/bin/rzn-browser-worker" \
  "${SCRIPT_DIR}/bin/rzn-native-host" \
  "${SCRIPT_DIR}/extension/dist-chrome/manifest.json"
do
  if [[ ! -e "$required" ]]; then
    echo "[ERROR] Missing bundle file: ${required#${SCRIPT_DIR}/}"
    exit 1
  fi
done

echo "[INFO] Installing binaries to: ${BIN_DIR}"
install_file_atomic "${SCRIPT_DIR}/bin/rzn-browser" "${BIN_DIR}/rzn-browser"
install_file_atomic "${SCRIPT_DIR}/bin/rzn-browser-worker" "${BIN_DIR}/rzn-browser-worker"
install_file_atomic "${SCRIPT_DIR}/bin/rzn-native-host" "${BIN_DIR}/rzn-native-host"

echo "[INFO] Installing stable extension copy to: ${EXT_DIR}"
rm -rf "${EXT_DIR}"
mkdir -p "$(dirname "${EXT_DIR}")"
cp -R "${SCRIPT_DIR}/extension/dist-chrome" "${EXT_DIR}"

if [[ -d "${SCRIPT_DIR}/skills" ]]; then
  echo "[INFO] Installing bundled skills to: ${INSTALL_ROOT}/skills/builtin"
  rm -rf "${INSTALL_ROOT}/skills/builtin"
  mkdir -p "${INSTALL_ROOT}/skills/builtin"
  cp -R "${SCRIPT_DIR}/skills/." "${INSTALL_ROOT}/skills/builtin/"
fi

echo "[INFO] Refreshing bundled workflows/examples"
RZN_RUNTIME_DIR="${INSTALL_ROOT}" "${BIN_DIR}/rzn-browser" workflow pull --repo-root "${SCRIPT_DIR}"

install_bin_entry "${BIN_DIR}/rzn-browser" "${GLOBAL_BIN_DIR}/rzn-browser"
install_bin_entry "${BIN_DIR}/rzn-browser-worker" "${GLOBAL_BIN_DIR}/rzn-browser-worker"
install_bin_entry "${BIN_DIR}/rzn-native-host" "${GLOBAL_BIN_DIR}/rzn-native-host"

echo "[INFO] Writing native messaging manifest: ${MANIFEST_PATH}"
cat > "${MANIFEST_PATH}" << EOF
{
  "name": "${HOST_NAME}",
  "description": "RZN Browser Host",
  "path": "${BIN_DIR}/rzn-native-host",
  "type": "stdio",
  "allowed_origins": [
    "chrome-extension://${EXTENSION_ID}/"
  ]
}
EOF

echo ""
echo "[OK] Installed:"
echo "  - rzn-browser: ${BIN_DIR}/rzn-browser"
echo "  - rzn-browser-worker: ${BIN_DIR}/rzn-browser-worker"
echo "  - rzn-native-host: ${BIN_DIR}/rzn-native-host"
echo "  - PATH links: ${GLOBAL_BIN_DIR}"
echo "  - extension: ${EXT_DIR}"
echo "  - manifest: ${MANIFEST_PATH}"
echo ""
echo "Next:"
echo "1) Load extension unpacked from: ${EXT_DIR}"
echo "2) Confirm extension ID: ${EXTENSION_ID}"
echo "3) Restart Chrome (recommended)"
echo "4) Run: ${SCRIPT_DIR}/doctor-macos.sh"
