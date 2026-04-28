#!/usr/bin/env bash
set -eo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
OUT_ROOT="${OUT:-docs/index/agent_runs}"
TS=$(date +%Y%m%d_%H%M%S)
RUN_DIR="$OUT_ROOT/$TS"
SCOPE_FILE="docs/context/llm_scope.yml"
MSG="${M:-}"
SPARSE="${S:-0}"

mkdir -p "$RUN_DIR"

if [[ -z "$MSG" ]]; then
  echo "Set M=\"<task message>\" to describe the change." >&2
fi

echo "$MSG" > "$RUN_DIR/MESSAGE.txt"

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

ensure_scope_defaults() {
  if [[ ! -f "$SCOPE_FILE" ]]; then
    mkdir -p "docs/context"
    cat > "$SCOPE_FILE" << 'YAML'
include:
  - extension/src/**
  - crates/**/src/**
  - workflows/**
  - docs/**/*.md
exclude:
  - target/**
  - node_modules/**
  - extension/dist*/**
  - logs/**
YAML
  fi
}

ensure_scope_defaults

RG_ARGS=()
while IFS= read -r line; do [[ -n "$line" ]] && RG_ARGS+=( -g "$line" ); done < <(read_globs include)
while IFS= read -r line; do [[ -n "$line" ]] && RG_ARGS+=( -g "!$line" ); done < <(read_globs exclude)

SHORTLIST="$RUN_DIR/shortlist.txt"
rg --files --hidden "${RG_ARGS[@]}" | sort -u > "$SHORTLIST"

if [[ -s docs/index/HOTSPOTS.rg ]]; then
  cp docs/index/HOTSPOTS.rg "$RUN_DIR/HOTSPOTS.rg" || true
fi

PROMPT="$RUN_DIR/prompt.txt"
{
  echo "Scoped Agent Run"
  echo "Message: $MSG"
  echo "Run Dir: $RUN_DIR"
  echo "Shortlist Count: $(wc -l < "$SHORTLIST" | tr -d ' ')"
  echo
  echo "Map: docs/REPO_MAP.md"
  echo "Workflow: docs/LLM_SCOPED_WORKFLOW.md"
  echo "Tree: docs/index/TREE.md"
  echo
  echo "Guidance: Limit edits to files in shortlist.txt."
  echo "Use make agent-validate OUT=$RUN_DIR STRICT=1 before committing."
} > "$PROMPT"

if [[ "$SPARSE" == "1" ]]; then
  if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "Skipping sparse-checkout (not a git repo)." >&2
  else
    if [[ -n "$(git status --porcelain)" ]]; then
      echo "Working tree not clean; skipping sparse-checkout." >&2
    else
      git sparse-checkout init --cone >/dev/null 2>&1 || true
      git sparse-checkout set $(cat "$SHORTLIST") >/dev/null 2>&1 || true
      echo "Applied sparse-checkout to shortlist files." >&2
    fi
  fi
fi

echo "Wrote $RUN_DIR"
ln -sfn "$(cd "$RUN_DIR" && pwd)" "$OUT_ROOT/latest" 2>/dev/null || true
