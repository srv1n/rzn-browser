#!/usr/bin/env bash
set -euo pipefail

out_dir="docs/index"
mkdir -p "$out_dir"

summary="$out_dir/SUMMARY.md"

echo "# Scope Summary" > "$summary"
echo >> "$summary"
echo "- Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$summary"
echo "- Tree: docs/index/TREE.md" >> "$summary"
echo "- Hotspots: docs/index/HOTSPOTS.rg" >> "$summary"
echo "- Context: docs/index/CONTEXT_SNIPPETS.md" >> "$summary"
echo "- Reducers Index: docs/index/REDUCERS_INDEX.md" >> "$summary"
echo "- Invariants: docs/index/INVARIANTS.md" >> "$summary"
echo >> "$summary"
echo "Use make scope-q Q=\"...\" for quick lookups." >> "$summary"

echo "Wrote $summary"

