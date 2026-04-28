#!/usr/bin/env bash
set -euo pipefail

out_dir="docs/index"
mkdir -p "$out_dir"
out_file="$out_dir/INVARIANTS.md"

echo "# Streaming Invariants" > "$out_file"
echo >> "$out_file"

rg -n --hidden \
  -g '!target/**' -g '!node_modules/**' -g '!extension/dist*/**' \
  -e 'INVARIANT' -e 'Invariant' -e 'SAFETY' -e 'Safety' -e 'Do not change' -e 'Do not modify' -e 'Stable API' -e 'Assumption' \
  | sed 's/^/- /' >> "$out_file" || true

echo "Wrote $out_file"

