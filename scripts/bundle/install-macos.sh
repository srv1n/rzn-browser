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

guarded_rm_rf() {
  local target="${1:-}"
  local expected_root="${2:-}"
  local label="${3:-path}"

  if [[ -z "$target" || -z "$expected_root" ]]; then
    echo "[ERROR] Refusing to remove empty ${label}." >&2
    return 1
  fi
  if [[ "$target" != /* ]]; then
    echo "[ERROR] Refusing to remove non-absolute ${label}: ${target}" >&2
    return 1
  fi
  if [[ "$expected_root" != /* ]]; then
    echo "[ERROR] Refusing to remove ${label}; expected root is not absolute: ${expected_root}" >&2
    return 1
  fi

  target="${target%/}"
  expected_root="${expected_root%/}"
  if [[ -z "$expected_root" || "$expected_root" == "/" ]]; then
    echo "[ERROR] Refusing to remove ${label}; expected root is unsafe: ${expected_root}" >&2
    return 1
  fi
  case "$target" in
    *"/../"*|*"/..")
      echo "[ERROR] Refusing to remove ${label} containing '..': ${target}" >&2
      return 1
      ;;
  esac
  case "$target" in
    "$expected_root"/*) rm -rf "$target" ;;
    *)
      echo "[ERROR] Refusing to remove ${label} outside expected root: ${target}" >&2
      return 1
      ;;
  esac
}

strip_launch_xattrs() {
  local path="${1:?missing path}"
  if [[ "${RZN_INSTALL_ARTIFACT_SHA256_VERIFIED:-0}" != "1" ]]; then
    return 0
  fi
  if command -v xattr >/dev/null 2>&1; then
    xattr -d com.apple.provenance "$path" 2>/dev/null || true
    xattr -d com.apple.quarantine "$path" 2>/dev/null || true
  fi
}

repair_macos_signature() {
  local path="${1:?missing path}"
  if [[ "${RZN_INSTALL_ARTIFACT_SHA256_VERIFIED:-0}" != "1" ]]; then
    return 0
  fi
  if command -v codesign >/dev/null 2>&1; then
    if ! codesign --verify "$path" >/dev/null 2>&1 || codesign -dvv "$path" 2>&1 | grep -q "Signature=adhoc"; then
      codesign --force --sign - "$path" >/dev/null
    fi
  fi
}

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
  strip_launch_xattrs "$tmp"
  repair_macos_signature "$tmp"
  mv -f "$tmp" "$dest"
}

mkdir -p "$BIN_DIR" "$CHROME_HOST_DIR" "$GLOBAL_BIN_DIR"

for required in \
  "${SCRIPT_DIR}/bin/rzn-browser" \
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
install_file_atomic "${SCRIPT_DIR}/bin/rzn-native-host" "${BIN_DIR}/rzn-native-host"

echo "[INFO] Installing stable extension copy to: ${EXT_DIR}"
guarded_rm_rf "${EXT_DIR}" "${INSTALL_ROOT}" "runtime extension"
mkdir -p "$(dirname "${EXT_DIR}")"
cp -R "${SCRIPT_DIR}/extension/dist-chrome" "${EXT_DIR}"

if [[ -d "${SCRIPT_DIR}/skills" ]]; then
  echo "[INFO] Installing bundled skills to: ${INSTALL_ROOT}/skills/builtin"
  guarded_rm_rf "${INSTALL_ROOT}/skills/builtin" "${INSTALL_ROOT}" "builtin skills"
  mkdir -p "${INSTALL_ROOT}/skills/builtin"
  cp -R "${SCRIPT_DIR}/skills/." "${INSTALL_ROOT}/skills/builtin/"
fi

echo "[INFO] Refreshing bundled workflows/examples"
RZN_RUNTIME_DIR="${INSTALL_ROOT}" "${BIN_DIR}/rzn-browser" workflow pull --repo-root "${SCRIPT_DIR}"

install_bin_entry "${BIN_DIR}/rzn-browser" "${GLOBAL_BIN_DIR}/rzn-browser"
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
