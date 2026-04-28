#!/usr/bin/env bash
set -euo pipefail

if [ $# -lt 1 ]; then
  echo "Usage: $0 /path/to/rznapp"
  exit 1
fi

SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/schema/actions-v1.json"
DEST_ROOT="$1"
DEST_DIR="$DEST_ROOT/schema"
DEST="$DEST_DIR/actions-v1.json"

mkdir -p "$DEST_DIR"
cp "$SRC" "$DEST"

echo "Copied actions schema to: $DEST"
