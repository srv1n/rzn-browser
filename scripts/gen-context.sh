#!/usr/bin/env bash
set -euo pipefail

out_dir="docs/index"
mkdir -p "$out_dir"
out_file="$out_dir/CONTEXT_SNIPPETS.md"

emit_section() {
  local title="$1"; shift
  echo "## $title" >> "$out_file"
  if "$@"; then :; else echo "(none)" >> "$out_file"; fi
  echo >> "$out_file"
}

write_matches() {
  local pattern="$1"; shift
  local globs=()
  while [[ $# -gt 0 ]]; do globs+=( -g "$1" ); shift; done
  rg -n --hidden "$pattern" "${globs[@]}" -g '!target/**' -g '!node_modules/**' -g '!extension/dist*/**' || true
}

snippet_context() {
  local pattern="$1"; shift
  local globs=()
  while [[ $# -gt 0 ]]; do globs+=( -g "$1" ); shift; done
  rg -n --hidden "$pattern" "${globs[@]}" -g '!target/**' -g '!node_modules/**' -g '!extension/dist*/**' \
    | while IFS=: read -r file line _; do
        echo "--- $file:$line"
        start=$((line-3)); end=$((line+3)); if (( start < 1 )); then start=1; fi
        sed -n "${start},${end}p" "$file" | sed 's/^/    /'
      done
}

echo "# Context Snippets" > "$out_file"
echo >> "$out_file"

emit_section "Listeners (TS)" snippet_context \
  'chrome\.runtime\.onMessage|browser\.runtime\.onMessage|window\.addEventListener\([^)]*message' \
  'extension/src/**'

emit_section "Emitters (TS)" snippet_context \
  'chrome\.(runtime|tabs)\.sendMessage|port\.postMessage|window\.postMessage' \
  'extension/src/**'

emit_section "Invokes (TS)" snippet_context \
  '__rznExecuteStep|captureEnhancedDOMSnapshot|chrome\.scripting\.executeScript' \
  'extension/src/**'

emit_section "Store/Dispatch (TS)" write_matches \
  'dispatch\(|createSlice\(|builder\.addCase' \
  'extension/src/**'

emit_section "Runtime Transport (Rust)" snippet_context \
  'serde_json::to_string|serde_json::from_str|mpsc|tokio|send|recv|write_all|read_to_end' \
  'crates/**/src/**'

echo "Wrote $out_file"
