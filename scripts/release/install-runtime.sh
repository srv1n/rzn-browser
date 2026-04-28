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
  if command -v xattr >/dev/null 2>&1; then
    xattr -d com.apple.provenance "$tmp" 2>/dev/null || true
    xattr -d com.apple.quarantine "$tmp" 2>/dev/null || true
  fi
  mv -f "$tmp" "$dest"
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
EXT_DIR="$INSTALL_ROOT/extension/dist-chrome"
MANIFEST_PATH="$CHROME_HOST_DIR/$HOST_NAME.json"
GLOBAL_BIN_DIR=$(default_global_bin_dir)

for required in "$SCRIPT_DIR/bin/rzn-browser" "$SCRIPT_DIR/bin/rzn-browser-worker" "$SCRIPT_DIR/bin/rzn-native-host" "$SCRIPT_DIR/extension/dist-chrome/manifest.json"; do
  if [ ! -e "$required" ]; then
    echo "[ERROR] Missing packaged file: $required" >&2
    exit 1
  fi
done

mkdir -p "$BIN_DIR" "$CHROME_HOST_DIR" "$GLOBAL_BIN_DIR"

echo "[INFO] Installing binaries into: $BIN_DIR"
install_file_atomic "$SCRIPT_DIR/bin/rzn-browser" "$BIN_DIR/rzn-browser"
install_file_atomic "$SCRIPT_DIR/bin/rzn-browser-worker" "$BIN_DIR/rzn-browser-worker"
install_file_atomic "$SCRIPT_DIR/bin/rzn-native-host" "$BIN_DIR/rzn-native-host"

echo "[INFO] Installing stable extension copy into: $EXT_DIR"
rm -rf "$EXT_DIR"
mkdir -p "$(dirname "$EXT_DIR")"
cp -R "$SCRIPT_DIR/extension/dist-chrome" "$EXT_DIR"

if [ -d "$SCRIPT_DIR/skills" ]; then
  echo "[INFO] Installing bundled skills into: $INSTALL_ROOT/skills/builtin"
  rm -rf "$INSTALL_ROOT/skills/builtin"
  mkdir -p "$INSTALL_ROOT/skills/builtin"
  cp -R "$SCRIPT_DIR/skills/." "$INSTALL_ROOT/skills/builtin/"
fi

echo "[INFO] Refreshing bundled workflows/examples into: $INSTALL_ROOT/workflows/builtin"
RZN_RUNTIME_DIR="$INSTALL_ROOT" "$BIN_DIR/rzn-browser" workflow pull --repo-root "$SCRIPT_DIR"

install_bin_entry "$BIN_DIR/rzn-browser" "$GLOBAL_BIN_DIR/rzn-browser"
install_bin_entry "$BIN_DIR/rzn-browser-worker" "$GLOBAL_BIN_DIR/rzn-browser-worker"
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
echo "  - worker: $GLOBAL_BIN_DIR/rzn-browser-worker"
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
echo "2. Enable Developer mode"
echo "3. Load unpacked from: $EXT_DIR"
echo "4. Restart Chrome once"
echo "5. Run: rzn-browser workflow list google"
