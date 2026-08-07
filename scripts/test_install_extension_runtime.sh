#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/rzn-extension-sync.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT INT TERM

SOURCE_DIST="$TMP_DIR/build/dist"
RUNTIME_ROOT="$TMP_DIR/runtime"
mkdir -p "$SOURCE_DIST/chrome" "$TMP_DIR/build/dist-chrome"
printf '{"build_signature":"fresh"}\n' > "$SOURCE_DIST/chrome/rzn-build.json"
printf '{"manifest_version":3}\n' > "$SOURCE_DIST/chrome/manifest.json"
printf '{"build_signature":"stale"}\n' > "$TMP_DIR/build/dist-chrome/rzn-build.json"

bash "$ROOT_DIR/scripts/install_extension_runtime.sh" "$SOURCE_DIST" "$RUNTIME_ROOT"

diff -rq "$SOURCE_DIST/chrome" "$RUNTIME_ROOT/extension/dist/chrome"
diff -rq "$SOURCE_DIST/chrome" "$RUNTIME_ROOT/extension/dist-chrome"
if rg -q 'stale' "$RUNTIME_ROOT/extension/dist-chrome/rzn-build.json"; then
  echo "[FAIL] legacy extension path used stale input" >&2
  exit 1
fi
echo "[PASS] legacy Chrome path is copied from freshly built dist/chrome"
