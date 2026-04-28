#!/usr/bin/env bash
set -eo pipefail

OUT_DIR="${OUT:-docs/index/agent_runs/latest}"
STRICT="${STRICT:-0}"
SHORTLIST="$OUT_DIR/shortlist.txt"

if [[ ! -f "$SHORTLIST" ]]; then
  echo "Shortlist not found at $SHORTLIST" >&2
  exit 1
fi

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "Not a git repository; cannot validate touched files." >&2
  exit 1
fi

changed=$(git status --porcelain | awk '{print $2}' | sort -u)
violations_count=0
if [[ -n "$changed" ]]; then
  while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    if ! grep -Fx -- "$f" "$SHORTLIST" >/dev/null 2>&1 && ! grep -Fx -- "./$f" "$SHORTLIST" >/dev/null 2>&1; then
      echo " - $f" >&2
      violations_count=$((violations_count+1))
    fi
  done <<< "$changed"
fi

if (( violations_count > 0 )); then
  echo "Files outside shortlist:" >&2
  if [[ "$STRICT" == "1" ]]; then
    exit 2
  fi
fi

make sg-guards STRICT="$STRICT"
echo "Validation complete."
