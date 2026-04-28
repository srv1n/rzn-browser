#!/usr/bin/env bash
set -euo pipefail

STRICT="${STRICT:-0}"

echo "Running schema DDL guard (outside migrations)…"

matches=$(rg -n --hidden \
  -g '!target/**' -g '!node_modules/**' -g '!extension/dist*/**' \
  -g '!**/migrations/**' -g '!crates/**/migrations/**' \
  -e '\bCREATE\s+TABLE\b' -e '\bALTER\s+TABLE\b' -e '\bDROP\s+TABLE\b' -e '\bCREATE\s+INDEX\b' -e '\bDROP\s+INDEX\b' \
  --type-add 'sql:*.sql' --type-add 'rust:*.rs' --type=sql --type=rust || true)

if [[ -n "$matches" ]]; then
  echo "Potential schema DDL found outside migrations:" >&2
  printf '%s\n' "$matches" >&2
  if [[ "$STRICT" == "1" ]]; then
    exit 3
  fi
else
  echo "No DDL outside migrations detected."
fi
