#!/usr/bin/env bash
set -euo pipefail

DEPTH="${1:-${DEPTH:-3}}"
IGNORE=("target" "node_modules" "dist" "dist-chrome" ".git")

if command -v tree >/dev/null 2>&1; then
  IGN="$(printf "%s|" "${IGNORE[@]}")"; IGN="${IGN%|}"
  tree -a -L "$DEPTH" -I "$IGN" || true
  exit 0
fi

rg --files --hidden -g '!target/**' -g '!node_modules/**' -g '!extension/dist*/**' -g '!.git/**' \
  | awk -F'/' -v maxd="$DEPTH" '{
      d=NF-1; if(d>maxd) d=maxd; 
      indent=""; for(i=0;i<d;i++) indent=indent"  "; print indent$0
    }'

