#!/usr/bin/env bash
set -euo pipefail

out_dir="docs/index"
mkdir -p "$out_dir"
out_file="$out_dir/REDUCERS_INDEX.md"

echo "# Reducers Index" > "$out_file"
echo >> "$out_file"

rg -n --hidden -g 'extension/src/**' -g '!extension/dist*/**' -g '!node_modules/**' \
  'createSlice\(|builder\.addCase|switch\s*\(\s*action\.type' \
  | while IFS=: read -r file line text; do
      echo "- $file:$line: $(echo "$text" | sed 's/^\s*//')" >> "$out_file"
    done

echo "Wrote $out_file"

