#!/usr/bin/env bash
set -eo pipefail

Q="${1:-${Q:-}}"
if [[ -z "$Q" ]]; then
  echo "Usage: make scope-q Q=\"search terms\"" >&2
  exit 1
fi

SCOPE_FILE="docs/context/llm_scope.yml"

read_globs() {
  local section="$1"; shift || true
  awk -v sec="$section" '
    BEGIN{p=0}
    /^\s*#/ {next}
    $0 ~ "^"sec":" {p=1; next}
    p==1 && /^\s*-[[:space:]]/ {gsub("^- ", "", $0); gsub(/^\s+|\s+$/, "", $0); print $0}
    p==1 && NF==0 {p=0}
  ' "$SCOPE_FILE" | sed '/^$/d'
}

if [[ ! -f "$SCOPE_FILE" ]]; then
  echo "docs/context/llm_scope.yml not found; run make scope to initialize." >&2
  exit 1
fi

args=(-n --hidden)
while IFS= read -r line; do [[ -n "$line" ]] && args+=( -g "$line" ); done < <(read_globs include)
while IFS= read -r line; do [[ -n "$line" ]] && args+=( -g "!$line" ); done < <(read_globs exclude)

rg "${args[@]}" -S "$Q" || true
