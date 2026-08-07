#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" != "2" ]]; then
  echo "usage: $0 <built-extension-dist> <runtime-root>" >&2
  exit 64
fi

SOURCE_DIST="$1"
RUNTIME_ROOT="$2"

if [[ "$SOURCE_DIST" != /* || "$RUNTIME_ROOT" != /* ]]; then
  echo "[ERROR] extension source and runtime root must be absolute paths" >&2
  exit 64
fi
if [[ ! -f "$SOURCE_DIST/chrome/manifest.json" ]]; then
  echo "[ERROR] missing freshly built Chrome extension: $SOURCE_DIST/chrome/manifest.json" >&2
  exit 1
fi

SOURCE_DIST="$(cd "$SOURCE_DIST" && pwd -P)"
mkdir -p "$RUNTIME_ROOT"
RUNTIME_ROOT="$(cd "$RUNTIME_ROOT" && pwd -P)"
if [[ "$RUNTIME_ROOT" == "/" || "$RUNTIME_ROOT" == *"/../"* || "$RUNTIME_ROOT" == *"/.." ]]; then
  echo "[ERROR] refusing unsafe runtime root: $RUNTIME_ROOT" >&2
  exit 64
fi

EXTENSION_ROOT="$RUNTIME_ROOT/extension"
case "$SOURCE_DIST" in
  "$EXTENSION_ROOT"|"$EXTENSION_ROOT"/*)
    echo "[ERROR] extension source must not be inside the installed extension root" >&2
    exit 64
    ;;
esac

rm -rf "$EXTENSION_ROOT"
mkdir -p "$EXTENSION_ROOT"
cp -R "$SOURCE_DIST" "$EXTENSION_ROOT/dist"
# Chrome's legacy unpacked path must be a second copy of this build, never a
# repository dist-chrome directory that may predate the build above.
cp -R "$SOURCE_DIST/chrome" "$EXTENSION_ROOT/dist-chrome"
