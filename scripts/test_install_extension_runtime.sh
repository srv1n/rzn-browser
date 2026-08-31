#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/rzn-extension-sync.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT INT TERM

SOURCE_DIST="$TMP_DIR/build/dist"
RUNTIME_ROOT="$TMP_DIR/case/RZN"
LEGACY_EXTENSION_ROOT="$TMP_DIR/case/rzn/extension/dist-chrome"
mkdir -p "$SOURCE_DIST/chrome"
mkdir -p "$LEGACY_EXTENSION_ROOT"
printf '{"build_signature":"fresh"}\n' > "$SOURCE_DIST/chrome/rzn-build.json"
printf '{"manifest_version":3}\n' > "$SOURCE_DIST/chrome/manifest.json"
printf 'dashboard\n' > "$SOURCE_DIST/chrome/dashboard.html"
printf '{"manifest_version":3,"version":"old"}\n' > "$LEGACY_EXTENSION_ROOT/manifest.json"

bash "$ROOT_DIR/scripts/install_extension_runtime.sh" "$SOURCE_DIST" "$RUNTIME_ROOT"

diff -rq "$SOURCE_DIST/chrome" "$RUNTIME_ROOT/extension/dist/chrome"
diff -rq "$SOURCE_DIST/chrome" "$LEGACY_EXTENSION_ROOT"

printf 'stale\n' > "$RUNTIME_ROOT/extension/dist/chrome/stale.js"
printf '{"build_signature":"newer"}\n' > "$SOURCE_DIST/chrome/rzn-build.json"
bash "$ROOT_DIR/scripts/install_extension_runtime.sh" "$SOURCE_DIST" "$RUNTIME_ROOT"

diff -rq "$SOURCE_DIST/chrome" "$RUNTIME_ROOT/extension/dist/chrome"
diff -rq "$SOURCE_DIST/chrome" "$LEGACY_EXTENSION_ROOT"
[[ ! -e "$RUNTIME_ROOT/extension/dist/chrome/stale.js" ]]
echo "[PASS] canonical Chrome path is copied from freshly built dist/chrome"
