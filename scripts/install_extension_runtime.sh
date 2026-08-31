#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" != "2" ]]; then
  echo "usage: $0 <built-extension-dist> <runtime-root>" >&2
  exit 64
fi

SOURCE_DIST="$1"
RUNTIME_ROOT="$2"
RUNTIME_ROOT_NAME="$(basename "$RUNTIME_ROOT")"

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
EXTENSION_DIST_ROOT="$EXTENSION_ROOT/dist"
case "$SOURCE_DIST" in
  "$EXTENSION_ROOT"|"$EXTENSION_ROOT"/*)
    echo "[ERROR] extension source must not be inside the installed extension root" >&2
    exit 64
    ;;
esac

install_tree() {
  local source_tree="$1"
  local dest_tree="$2"
  local manifest_rel="$3"
  local parent staged backup had_previous
  parent="$(dirname "$dest_tree")"
  mkdir -p "$parent"
  staged="$(mktemp -d "$parent/.rzn-extension.new.XXXXXX")"
  backup="$(mktemp -d "$parent/.rzn-extension.old.XXXXXX")"
  rmdir "$backup"

  if ! cp -R "$source_tree/." "$staged/" || [[ ! -f "$staged/$manifest_rel" ]]; then
    echo "[ERROR] staged Chrome extension is incomplete" >&2
    rm -rf "$staged"
    return 1
  fi

  had_previous=0
  if [[ -e "$dest_tree" ]]; then
    mv "$dest_tree" "$backup"
    had_previous=1
  fi

  if mv "$staged" "$dest_tree" \
    && cmp -s "$source_tree/$manifest_rel" "$dest_tree/$manifest_rel"; then
    if [[ "$had_previous" == "1" ]]; then
      rm -rf "$backup"
    fi
    return 0
  fi

  rm -rf "$dest_tree"
  if [[ "$had_previous" == "1" ]]; then
    mv "$backup" "$dest_tree"
  fi
  echo "[ERROR] extension install failed; previous copy restored" >&2
  return 1
}

install_tree "$SOURCE_DIST" "$EXTENSION_DIST_ROOT" "chrome/manifest.json"

if [[ "$(uname -s)" == "Darwin" && "$RUNTIME_ROOT_NAME" == "RZN" ]]; then
  LEGACY_ROOT="$(dirname "$RUNTIME_ROOT")/rzn"
  LEGACY_EXTENSION_ROOT="$LEGACY_ROOT/extension/dist-chrome"
  if [[ -f "$LEGACY_EXTENSION_ROOT/manifest.json" ]]; then
    echo "[INFO] Refreshing legacy Chrome extension path: $LEGACY_EXTENSION_ROOT"
    install_tree "$SOURCE_DIST/chrome" "$LEGACY_EXTENSION_ROOT" "manifest.json"
  fi
fi
