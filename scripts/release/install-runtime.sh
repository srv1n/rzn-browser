#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

HOST_NAME=${RZN_NATIVE_HOST_NAME:-com.rzn.browser.broker}
EXTENSION_ID=${RZN_CHROME_EXTENSION_ID:-bogjdnehdficgkhklinmnbgiiofbamji}

path_contains_dir() {
  case ":$PATH:" in
    *":$1:"*) return 0 ;;
    *) return 1 ;;
  esac
}

default_global_bin_dir() {
  if [ -n "${RZN_SETUP_GLOBAL_BIN_DIR:-}" ]; then
    printf '%s\n' "$RZN_SETUP_GLOBAL_BIN_DIR"
  elif path_contains_dir "$HOME/.local/bin"; then
    printf '%s\n' "$HOME/.local/bin"
  elif path_contains_dir "$HOME/bin"; then
    printf '%s\n' "$HOME/bin"
  else
    printf '%s\n' "$HOME/.local/bin"
  fi
}

guarded_rm_rf() {
  guard_target=${1:-}
  guard_root=${2:-}
  guard_label=${3:-path}

  if [ -z "$guard_target" ] || [ -z "$guard_root" ]; then
    echo "[ERROR] Refusing to remove empty ${guard_label}." >&2
    return 1
  fi
  case "$guard_target" in
    /*) ;;
    *)
      echo "[ERROR] Refusing to remove non-absolute ${guard_label}: $guard_target" >&2
      return 1
      ;;
  esac
  case "$guard_root" in
    /*) ;;
    *)
      echo "[ERROR] Refusing to remove ${guard_label}; expected root is not absolute: $guard_root" >&2
      return 1
      ;;
  esac
  guard_target=${guard_target%/}
  guard_root=${guard_root%/}
  if [ -z "$guard_root" ] || [ "$guard_root" = "/" ]; then
    echo "[ERROR] Refusing to remove ${guard_label}; expected root is unsafe: $guard_root" >&2
    return 1
  fi
  case "$guard_target" in
    *"/../"*|*"/..")
      echo "[ERROR] Refusing to remove ${guard_label} containing '..': $guard_target" >&2
      return 1
      ;;
  esac
  case "$guard_target" in
    "$guard_root"/*) rm -rf "$guard_target" ;;
    *)
      echo "[ERROR] Refusing to remove ${guard_label} outside expected root: $guard_target" >&2
      return 1
      ;;
  esac
}

strip_launch_xattrs() {
  strip_path=$1
  if [ "${RZN_INSTALL_ARTIFACT_SHA256_VERIFIED:-0}" != "1" ]; then
    return 0
  fi
  if command -v xattr >/dev/null 2>&1; then
    xattr -d com.apple.provenance "$strip_path" 2>/dev/null || true
    xattr -d com.apple.quarantine "$strip_path" 2>/dev/null || true
  fi
}

repair_macos_signature() {
  repair_path=$1
  if [ "${RZN_INSTALL_ARTIFACT_SHA256_VERIFIED:-0}" != "1" ]; then
    return 0
  fi
  if [ "$(uname -s)" = "Darwin" ] && command -v codesign >/dev/null 2>&1; then
    if ! codesign --verify "$repair_path" >/dev/null 2>&1 || codesign -dvv "$repair_path" 2>&1 | grep -q "Signature=adhoc"; then
      codesign --force --sign - "$repair_path" >/dev/null
    fi
  fi
}

install_bin_entry() {
  src=$1
  dest=$2
  if ln -sfn "$src" "$dest" 2>/dev/null; then
    return 0
  fi
  install_file_atomic "$src" "$dest"
}

install_file_atomic() {
  src=$1
  dest=$2
  dest_dir=$(dirname "$dest")
  mkdir -p "$dest_dir"
  tmp=$(mktemp "$dest_dir/.tmp.$(basename "$dest").XXXXXX")
  cp -f "$src" "$tmp"
  chmod +x "$tmp" 2>/dev/null || true
  strip_launch_xattrs "$tmp"
  repair_macos_signature "$tmp"
  mv -f "$tmp" "$dest"
}

validate_extension_tree() {
  tree=$1
  for required in manifest.json rzn-build.json background.js contentScript.js pageBridge.js popup.html dashboard.html; do
    if [ ! -f "$tree/$required" ]; then
      echo "[ERROR] Incomplete extension bundle; missing $tree/$required" >&2
      return 1
    fi
  done
}

install_extension_tree() {
  source_tree=$1
  dest_tree=$2
  expected_root=$3
  label=$4
  parent=$(dirname "$dest_tree")

  validate_extension_tree "$source_tree"
  mkdir -p "$parent"
  staged=$(mktemp -d "$parent/.rzn-extension.new.XXXXXX")
  backup=$(mktemp -d "$parent/.rzn-extension.old.XXXXXX")
  rmdir "$backup"

  if ! cp -R "$source_tree/." "$staged/" || ! validate_extension_tree "$staged"; then
    guarded_rm_rf "$staged" "$expected_root" "$label staging directory"
    return 1
  fi

  had_previous=0
  if [ -e "$dest_tree" ]; then
    mv "$dest_tree" "$backup"
    had_previous=1
  fi

  if mv "$staged" "$dest_tree" \
    && cmp -s "$source_tree/manifest.json" "$dest_tree/manifest.json" \
    && cmp -s "$source_tree/rzn-build.json" "$dest_tree/rzn-build.json"; then
    if [ "$had_previous" = "1" ]; then
      guarded_rm_rf "$backup" "$expected_root" "$label previous directory"
    fi
    return 0
  fi

  if [ -e "$dest_tree" ]; then
    guarded_rm_rf "$dest_tree" "$expected_root" "$label failed directory"
  fi
  if [ "$had_previous" = "1" ]; then
    mv "$backup" "$dest_tree"
  fi
  echo "[ERROR] Failed to install verified $label; previous copy restored." >&2
  return 1
}

case "$(uname -s)" in
  Darwin)
    INSTALL_ROOT=${RZN_RUNTIME_DIR:-"$HOME/Library/Application Support/RZN"}
    CHROME_HOST_DIR=${RZN_BUNDLE_CHROME_HOST_DIR:-"$HOME/Library/Application Support/Google/Chrome/NativeMessagingHosts"}
    ;;
  Linux)
    INSTALL_ROOT=${RZN_RUNTIME_DIR:-"$HOME/.local/share/RZN"}
    CHROME_HOST_DIR=${RZN_BUNDLE_CHROME_HOST_DIR:-"$HOME/.config/google-chrome/NativeMessagingHosts"}
    ;;
  *)
    echo "[ERROR] Unsupported OS: $(uname -s)" >&2
    exit 1
    ;;
esac

BIN_DIR="$INSTALL_ROOT/bin"
EXT_DIR="$INSTALL_ROOT/extension/dist/chrome"
MANIFEST_PATH="$CHROME_HOST_DIR/$HOST_NAME.json"
GLOBAL_BIN_DIR=$(default_global_bin_dir)

for required in \
  "$SCRIPT_DIR/bin/rzn-browser" \
  "$SCRIPT_DIR/bin/rzn-native-host" \
  "$SCRIPT_DIR/extension/dist/chrome/manifest.json" \
  "$SCRIPT_DIR/extension/dist/chrome/rzn-build.json" \
  "$SCRIPT_DIR/extension/dist/chrome/dashboard.html"
do
  if [ ! -e "$required" ]; then
    echo "[ERROR] Missing packaged file: $required" >&2
    exit 1
  fi
done
validate_extension_tree "$SCRIPT_DIR/extension/dist/chrome"

mkdir -p "$BIN_DIR" "$CHROME_HOST_DIR" "$GLOBAL_BIN_DIR"

echo "[INFO] Installing binaries into: $BIN_DIR"
install_file_atomic "$SCRIPT_DIR/bin/rzn-browser" "$BIN_DIR/rzn-browser"
install_file_atomic "$SCRIPT_DIR/bin/rzn-native-host" "$BIN_DIR/rzn-native-host"

echo "[INFO] Installing stable extension copy into: $EXT_DIR"
install_extension_tree \
  "$SCRIPT_DIR/extension/dist/chrome" \
  "$EXT_DIR" \
  "$INSTALL_ROOT" \
  "runtime extension"

if [ "$(uname -s)" = "Darwin" ] && [ "$(basename "$INSTALL_ROOT")" = "RZN" ]; then
  LEGACY_ROOT="$(dirname "$INSTALL_ROOT")/rzn"
  LEGACY_EXT_DIR="$LEGACY_ROOT/extension/dist-chrome"
  if [ -f "$LEGACY_EXT_DIR/manifest.json" ] && ! [ "$LEGACY_EXT_DIR" -ef "$EXT_DIR" ]; then
    echo "[INFO] Refreshing legacy Chrome extension path still used by existing installations: $LEGACY_EXT_DIR"
    install_extension_tree \
      "$SCRIPT_DIR/extension/dist/chrome" \
      "$LEGACY_EXT_DIR" \
      "$LEGACY_ROOT" \
      "legacy runtime extension"
  fi
fi

if [ -d "$SCRIPT_DIR/skills" ]; then
  echo "[INFO] Installing bundled skills into: $INSTALL_ROOT/skills/builtin"
  guarded_rm_rf "$INSTALL_ROOT/skills/builtin" "$INSTALL_ROOT" "builtin skills"
  mkdir -p "$INSTALL_ROOT/skills/builtin"
  cp -R "$SCRIPT_DIR/skills/." "$INSTALL_ROOT/skills/builtin/"
fi

echo "[INFO] Refreshing bundled workflows/examples into: $INSTALL_ROOT/workflows/builtin"
RZN_RUNTIME_DIR="$INSTALL_ROOT" "$BIN_DIR/rzn-browser" workflow pull --repo-root "$SCRIPT_DIR"

install_bin_entry "$BIN_DIR/rzn-browser" "$GLOBAL_BIN_DIR/rzn-browser"
install_bin_entry "$BIN_DIR/rzn-native-host" "$GLOBAL_BIN_DIR/rzn-native-host"

echo "[INFO] Writing native messaging manifest: $MANIFEST_PATH"
cat > "$MANIFEST_PATH" <<EOF
{
  "name": "$HOST_NAME",
  "description": "RZN Browser Host",
  "path": "$BIN_DIR/rzn-native-host",
  "type": "stdio",
  "allowed_origins": [
    "chrome-extension://$EXTENSION_ID/"
  ]
}
EOF

echo ""
echo "[OK] Installed RZN Browser"
echo "  - runtime: $INSTALL_ROOT"
echo "  - cli: $GLOBAL_BIN_DIR/rzn-browser"
echo "  - native host: $GLOBAL_BIN_DIR/rzn-native-host"
echo "  - extension: $EXT_DIR"
echo ""
if ! path_contains_dir "$GLOBAL_BIN_DIR"; then
  echo "[WARN] $GLOBAL_BIN_DIR is not on PATH."
  echo "       Add: export PATH=\"$GLOBAL_BIN_DIR:\$PATH\""
  echo ""
fi
echo "Next:"
echo "1. Open chrome://extensions"
echo "2. Click Reload on RZN Browser Automation"
echo "3. If this is the first install, load unpacked from: $EXT_DIR"
echo "4. Run: rzn-browser supervisor ensure-ready"
