#!/bin/bash
set -euo pipefail

echo "===== RZN Browser Setup ====="

# This setup is intentionally "dev-first" to avoid macOS SIGKILL from heavy release builds.
# Override with RZN_SETUP_PROFILE=release if you need release binaries.
PROFILE="${RZN_SETUP_PROFILE:-debug}" # debug | release
SKIP_RUST="${RZN_SETUP_SKIP_RUST:-0}"
SKIP_EXT="${RZN_SETUP_SKIP_EXT:-0}"
SKIP_MANIFEST="${RZN_SETUP_SKIP_MANIFEST:-0}"
INSTALL_GLOBAL_BIN="${RZN_SETUP_INSTALL_GLOBAL_BIN:-0}"
GLOBAL_BIN_DIR="${RZN_SETUP_GLOBAL_BIN_DIR:-}"
GLOBAL_CLI_NAME="${RZN_SETUP_GLOBAL_CLI_NAME:-rzn-browser}"
INSTALL_CODEX_SKILL="${RZN_SETUP_INSTALL_CODEX_SKILL:-${RZN_INSTALL_CODEX_SKILL:-0}}"
CODEX_SKILL_SCOPE="${RZN_SETUP_CODEX_SKILL_SCOPE:-${RZN_INSTALL_CODEX_SKILL_SCOPE:-global}}"

# Use the desktop app's native-host name by default so the CLI/dev loop matches production wiring.
# Override with RZN_NATIVE_HOST_NAME if you intentionally want an isolated manifest.
HOST_NAME="${RZN_NATIVE_HOST_NAME:-com.rzn.browser.broker}"
MANIFEST_NAME="${HOST_NAME}.json"

# Deterministic extension ID derived from extension/src/manifest.base.json "key".
DEFAULT_EXTENSION_ID="bogjdnehdficgkhklinmnbgiiofbamji"
EXTENSION_ID="${RZN_CHROME_EXTENSION_ID:-$DEFAULT_EXTENSION_ID}"

path_contains_dir() {
  local dir="${1:-}"
  [[ -n "$dir" ]] && [[ ":$PATH:" == *":$dir:"* ]]
}

default_global_bin_dir() {
  if [[ -n "$GLOBAL_BIN_DIR" ]]; then
    echo "$GLOBAL_BIN_DIR"
  elif [[ "$OSTYPE" == "msys"* ]] || [[ "$OSTYPE" == "win32" ]]; then
    echo "$APPDATA\\RZN\\bin"
  elif path_contains_dir "$HOME/.local/bin"; then
    echo "$HOME/.local/bin"
  elif path_contains_dir "$HOME/bin"; then
    echo "$HOME/bin"
  else
    echo "$HOME/.local/bin"
  fi
}

install_bin_entry() {
  local src="${1:?missing src}"
  local dest="${2:?missing dest}"
  if ln -sfn "$src" "$dest" 2>/dev/null; then
    return 0
  fi
  install_file_atomic "$src" "$dest"
}

install_file_atomic() {
  local src="${1:?missing src}"
  local dest="${2:?missing dest}"
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

if [[ "$PROFILE" != "debug" && "$PROFILE" != "release" ]]; then
  echo "[ERROR] Invalid RZN_SETUP_PROFILE=$PROFILE (expected debug|release)"
  exit 1
fi

echo "Profile: $PROFILE"
echo "Host name: $HOST_NAME"
echo "Extension ID: $EXTENSION_ID"
echo "Install global bin links: $INSTALL_GLOBAL_BIN"
echo "Install Codex skill: $INSTALL_CODEX_SKILL"

if [[ "$OSTYPE" == "darwin"* ]]; then
  # macOS can SIGKILL heavy builds (OOM/resource pressure). Also, we've seen sporadic
  # dependency-ordering failures on highly parallel builds; default to 1 job for setup.
  export CARGO_BUILD_JOBS="${RZN_SETUP_JOBS:-${CARGO_BUILD_JOBS:-1}}"
fi

if [[ "$SKIP_RUST" != "1" ]]; then
  echo "Building Rust components (rzn-native-host + rzn-browser-worker + rzn-browser)..."
  build_rust() {
    if [[ "$PROFILE" == "release" ]]; then
      cargo build --release -p rzn-native-host -p rzn-browser-worker -p rzn-browser
    else
      cargo build -p rzn-native-host -p rzn-browser-worker -p rzn-browser
    fi
  }

  # Work around sporadic `E0463 can't find crate for phf` build failures by retrying
  # after a clean. This has been observed when build artifacts get into a bad state.
  tmp_log="$(mktemp -t rzn_setup_rust.XXXXXX.log)"
  set +e
  build_rust 2>&1 | tee "$tmp_log"
  rust_status=${PIPESTATUS[0]}
  set -e
  if [[ "$rust_status" != "0" ]]; then
    if grep -q "error\\[E0463\\]: can't find crate for \\`phf\\`" "$tmp_log" 2>/dev/null; then
      echo "[WARN] Rust build hit E0463(phf). Running cargo clean and retrying once..."
      cargo clean
      build_rust
    else
      echo "[ERROR] Rust build failed. Log: $tmp_log"
      exit "$rust_status"
    fi
  fi
else
  echo "[INFO] RZN_SETUP_SKIP_RUST=1: skipping Rust build"
fi

if [[ "$SKIP_EXT" != "1" ]]; then
  if [[ "${RZN_SETUP_FORCE_EXT_BUILD:-0}" != "1" ]] && [[ -f "extension/dist-chrome/manifest.json" ]]; then
    echo "[INFO] extension/dist-chrome already exists; skipping extension build"
    echo "       Set RZN_SETUP_FORCE_EXT_BUILD=1 to rebuild"
  else
    if ! command -v bun >/dev/null 2>&1; then
      if [[ -f "extension/dist-chrome/manifest.json" ]]; then
        echo "[WARN] bun is not installed; reusing existing extension/dist-chrome"
      else
        echo "[ERROR] bun is required to build the extension, and extension/dist-chrome is missing"
        exit 1
      fi
    else
    echo "Building Chrome extension..."
    cd extension
    if [ ! -d "node_modules" ]; then
      echo "Installing extension dependencies (bun install)..."
      bun install
    else
      echo "Dependencies already installed, skipping bun install."
    fi

    echo "Running bun build for extension..."
    bun run build
    cd ..
    echo "Generating dist manifests (build-ext.ts)..."
    bun scripts/build-ext.ts
    fi
  fi
else
  echo "[INFO] RZN_SETUP_SKIP_EXT=1: skipping extension build"
fi

if [[ "$SKIP_MANIFEST" == "1" ]]; then
  echo "[INFO] RZN_SETUP_SKIP_MANIFEST=1: skipping native host manifest install"
  echo "Done."
  exit 0
fi

# Determine correct Native Messaging Host directory based on OS
echo "Determining Native Messaging Host directory for your OS..."
HOST_DIR=""

if [[ "$OSTYPE" == "linux-gnu"* ]]; then
  # Linux
  HOST_DIR="$HOME/.config/google-chrome/NativeMessagingHosts"
  # Create directory if it doesn't exist
  mkdir -p "$HOST_DIR"
elif [[ "$OSTYPE" == "darwin"* ]]; then
  # macOS
  HOST_DIR="$HOME/Library/Application Support/Google/Chrome/NativeMessagingHosts"
  # Create directory if it doesn't exist
  mkdir -p "$HOST_DIR"
elif [[ "$OSTYPE" == "msys"* ]] || [[ "$OSTYPE" == "win32" ]]; then
  # Windows Chrome looks up the manifest path from the registry, so store the JSON in our runtime area.
  HOST_DIR="$APPDATA/RZN/native-hosts"
  mkdir -p "$HOST_DIR"
else
  echo "Unsupported OS: $OSTYPE"
  echo "Please manually install the Native Messaging Host manifest."
  exit 1
fi

# Get absolute path to native host executable
NATIVE_HOST_BUILT_PATH="$(pwd)/target/${PROFILE}/rzn-native-host"
WORKER_BUILT_PATH="$(pwd)/target/${PROFILE}/rzn-browser-worker"
CLI_BUILT_PATH="$(pwd)/target/${PROFILE}/rzn-browser"
if [[ "$OSTYPE" == "msys"* ]] || [[ "$OSTYPE" == "win32" ]]; then
  # Windows needs .exe extension
  NATIVE_HOST_BUILT_PATH="${NATIVE_HOST_BUILT_PATH}.exe"
  WORKER_BUILT_PATH="${WORKER_BUILT_PATH}.exe"
  CLI_BUILT_PATH="${CLI_BUILT_PATH}.exe"
fi

# Install the native host binary into a stable location outside the repo (recommended on macOS).
# This avoids issues when the repo lives under protected folders (e.g. Downloads) and Chrome
# launches the native host with restricted access.
NATIVE_HOST_INSTALL_DIR=""
if [[ "$OSTYPE" == "darwin"* ]]; then
  NATIVE_HOST_INSTALL_DIR="$HOME/Library/Application Support/RZN/bin"
elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
  NATIVE_HOST_INSTALL_DIR="$HOME/.local/share/rzn/bin"
elif [[ "$OSTYPE" == "msys"* ]] || [[ "$OSTYPE" == "win32" ]]; then
  NATIVE_HOST_INSTALL_DIR="$APPDATA\\RZN\\bin"
fi

NATIVE_HOST_PATH="$NATIVE_HOST_BUILT_PATH"
WORKER_PATH="$WORKER_BUILT_PATH"
CLI_PATH="$CLI_BUILT_PATH"
RUNTIME_ROOT_DIR=""
if [ -n "$NATIVE_HOST_INSTALL_DIR" ]; then
  echo "Installing native host binary to: $NATIVE_HOST_INSTALL_DIR"
  mkdir -p "$NATIVE_HOST_INSTALL_DIR"
  RUNTIME_ROOT_DIR="$(dirname "$NATIVE_HOST_INSTALL_DIR")"
  NATIVE_HOST_PATH="$NATIVE_HOST_INSTALL_DIR/$(basename "$NATIVE_HOST_BUILT_PATH")"
  install_file_atomic "$NATIVE_HOST_BUILT_PATH" "$NATIVE_HOST_PATH"
  if [[ "$OSTYPE" == "darwin"* && "$PROFILE" == "debug" ]]; then
    echo "[INFO] Leaving debug CLI/worker in the repo build dir on macOS."
    echo "       Copied debug binaries under ~/Library/Application Support/RZN/bin can be exec-killed by macOS."
    WORKER_PATH="$WORKER_BUILT_PATH"
    CLI_PATH="$CLI_BUILT_PATH"
  else
    WORKER_PATH="$NATIVE_HOST_INSTALL_DIR/$(basename "$WORKER_BUILT_PATH")"
    CLI_PATH="$NATIVE_HOST_INSTALL_DIR/$(basename "$CLI_BUILT_PATH")"
    install_file_atomic "$WORKER_BUILT_PATH" "$WORKER_PATH"
    install_file_atomic "$CLI_BUILT_PATH" "$CLI_PATH"
  fi
  # On macOS, strip provenance/quarantine attributes from the installed copy so Chrome can launch it reliably.
  if [[ "$OSTYPE" == "darwin"* ]]; then
    xattr -d com.apple.provenance "$NATIVE_HOST_PATH" 2>/dev/null || true
    xattr -d com.apple.quarantine "$NATIVE_HOST_PATH" 2>/dev/null || true
    if [[ "$WORKER_PATH" == "$NATIVE_HOST_INSTALL_DIR/"* ]]; then
      xattr -d com.apple.provenance "$WORKER_PATH" 2>/dev/null || true
      xattr -d com.apple.quarantine "$WORKER_PATH" 2>/dev/null || true
    fi
    if [[ "$CLI_PATH" == "$NATIVE_HOST_INSTALL_DIR/"* ]]; then
      xattr -d com.apple.provenance "$CLI_PATH" 2>/dev/null || true
      xattr -d com.apple.quarantine "$CLI_PATH" 2>/dev/null || true
    fi
  fi
fi

if [[ -n "$RUNTIME_ROOT_DIR" ]]; then
  BUILTIN_WORKFLOWS_DIR="$RUNTIME_ROOT_DIR/workflows/builtin"
  USER_WORKFLOWS_DIR="$RUNTIME_ROOT_DIR/workflows/user"
  INSTALLED_EXTENSION_DIR="$RUNTIME_ROOT_DIR/extension/dist-chrome"

  echo "Installing stable extension copy to: $INSTALLED_EXTENSION_DIR"
  rm -rf "$INSTALLED_EXTENSION_DIR"
  mkdir -p "$(dirname "$INSTALLED_EXTENSION_DIR")"
  cp -R "$(pwd)/extension/dist-chrome" "$INSTALLED_EXTENSION_DIR"

  echo "Installing bundled workflows and examples into: $BUILTIN_WORKFLOWS_DIR"
  RZN_RUNTIME_DIR="$RUNTIME_ROOT_DIR" "$CLI_PATH" workflow pull --repo-root "$(pwd)"
fi

GLOBAL_BIN_TARGET_DIR=""
if [[ "$INSTALL_GLOBAL_BIN" == "1" ]]; then
  GLOBAL_BIN_TARGET_DIR="$(default_global_bin_dir)"
  echo "Installing PATH-facing binaries to: $GLOBAL_BIN_TARGET_DIR"
  mkdir -p "$GLOBAL_BIN_TARGET_DIR"
  install_bin_entry "$CLI_PATH" "$GLOBAL_BIN_TARGET_DIR/rzn-browser"
  if [[ "$GLOBAL_CLI_NAME" != "rzn-browser" ]]; then
    install_bin_entry "$CLI_PATH" "$GLOBAL_BIN_TARGET_DIR/$GLOBAL_CLI_NAME"
  fi
  install_bin_entry "$WORKER_PATH" "$GLOBAL_BIN_TARGET_DIR/rzn-browser-worker"
  install_bin_entry "$NATIVE_HOST_PATH" "$GLOBAL_BIN_TARGET_DIR/rzn-native-host"
fi

# Create and install the manifest
echo "Creating Native Messaging Host manifest at $HOST_DIR/$MANIFEST_NAME..."
MANIFEST_HOST_PATH="$NATIVE_HOST_PATH"
REGISTRY_MANIFEST_PATH="$HOST_DIR/$MANIFEST_NAME"
if [[ "$OSTYPE" == "msys"* ]] || [[ "$OSTYPE" == "win32" ]]; then
  if command -v cygpath >/dev/null 2>&1; then
    MANIFEST_HOST_PATH="$(cygpath -w "$NATIVE_HOST_PATH")"
    REGISTRY_MANIFEST_PATH="$(cygpath -w "$HOST_DIR/$MANIFEST_NAME")"
  fi
fi
MANIFEST_HOST_PATH_JSON="${MANIFEST_HOST_PATH//\\/\\\\}"
# Run the native host binary directly, with no shell wrapper.
cat > "$HOST_DIR/$MANIFEST_NAME" << EOL
{
  "name": "$HOST_NAME",
  "description": "RZN Browser Host",
  "path": "$MANIFEST_HOST_PATH_JSON",
  "type": "stdio",
  "allowed_origins": [
    "chrome-extension://$EXTENSION_ID/"
  ]
}
EOL

if [[ "$OSTYPE" == "msys"* ]] || [[ "$OSTYPE" == "win32" ]]; then
  echo "Registering Windows native host manifest in HKCU..."
  reg.exe ADD "HKCU\\Software\\Google\\Chrome\\NativeMessagingHosts\\$HOST_NAME" /ve /t REG_SZ /d "$REGISTRY_MANIFEST_PATH" /f >/dev/null
fi

if [[ "$INSTALL_CODEX_SKILL" != "0" && -x "$(pwd)/scripts/install_rzn_workflow_builder_skill.sh" ]]; then
  echo "Installing Codex workflow-builder skill (${CODEX_SKILL_SCOPE})..."
  case "$CODEX_SKILL_SCOPE" in
    global|project)
      bash "$(pwd)/scripts/install_rzn_workflow_builder_skill.sh" "--${CODEX_SKILL_SCOPE}" --force
      ;;
    *)
      echo "[WARN] Unsupported RZN_SETUP_CODEX_SKILL_SCOPE=$CODEX_SKILL_SCOPE (expected global|project). Skipping skill install."
      ;;
  esac
fi

echo "===== Setup Complete ====="
echo ""
echo "IMPORTANT: Next steps to complete setup:"
echo "1. Go to chrome://extensions/ in Chrome"
echo "2. Enable 'Developer mode' (toggle in top-right)"
echo "3. Click 'Load unpacked' and select the extension/dist-chrome directory"
echo "4. Verify the extension ID is: $EXTENSION_ID"
echo "5. Restart Chrome fully to ensure the Native Messaging Host is registered."
echo ""
if [[ "$INSTALL_GLOBAL_BIN" == "1" ]]; then
  echo "Installed CLI/runtime binaries:"
  if [[ "$GLOBAL_CLI_NAME" != "rzn-browser" ]]; then
    echo "  - $GLOBAL_BIN_TARGET_DIR/$GLOBAL_CLI_NAME"
  fi
  echo "  - $GLOBAL_BIN_TARGET_DIR/rzn-browser"
  echo "  - $GLOBAL_BIN_TARGET_DIR/rzn-browser-worker"
  echo "  - $GLOBAL_BIN_TARGET_DIR/rzn-native-host"
  if ! path_contains_dir "$GLOBAL_BIN_TARGET_DIR"; then
    echo "[WARN] $GLOBAL_BIN_TARGET_DIR is not currently on PATH."
    echo "       Add this to your shell profile:"
    echo "       export PATH=\"$GLOBAL_BIN_TARGET_DIR:\$PATH\""
    echo ""
  fi
fi
echo "Installed workflow directories:"
echo "  - builtin: $BUILTIN_WORKFLOWS_DIR"
echo "  - user: $USER_WORKFLOWS_DIR"
echo "Installed extension directory:"
echo "  - $INSTALLED_EXTENSION_DIR"
echo "To run workflows (unified desktop wiring):"
echo "1. Open Chrome. The extension will connect to '$HOST_NAME', causing Chrome to launch rzn-native-host."
echo "2. Run (preferred verb is 'run'; wrapper auto mode prefers desktop if running and otherwise spawns a worker from $WORKER_PATH):"
if [[ "$INSTALL_GLOBAL_BIN" == "1" && "$GLOBAL_CLI_NAME" != "rzn-browser" ]]; then
  echo "   $GLOBAL_CLI_NAME run google search --param search_query=\"rust lang\""
  echo "   # or"
fi
echo "   rzn-browser run google search --param search_query=\"rust lang\""
echo "   # explicit desktop transport: rzn-browser run google search --via desktop --param search_query=\"rust lang\""
echo "   # umbrella CLI wrapper: rzn browser run google search --param search_query=\"rust lang\""
echo "3. Installed CLI path:"
echo "   $CLI_PATH"
echo "4. List installed workflows:"
echo "   rzn-browser workflow list google"
echo "5. Refresh bundled workflows/examples later:"
echo "   rzn-browser workflow pull"
echo "6. Import your own workflow JSON:"
echo "   rzn-browser workflow add /path/to/my-workflow.json --system google --name my-flow"
echo "7. Extension folder to load in Chrome:"
echo "   $INSTALLED_EXTENSION_DIR"
echo "8. Optional: install the bundled Codex workflow-builder skill:"
echo "   bash scripts/install_rzn_workflow_builder_skill.sh --global"
echo "   # or"
echo "   bash scripts/install_rzn_workflow_builder_skill.sh --project"
echo "   # or install it during setup/install:"
echo "   RZN_INSTALL_CODEX_SKILL=1 RZN_INSTALL_CODEX_SKILL_SCOPE=global bash ./install.sh"
echo ""
