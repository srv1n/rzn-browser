#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/package-code-for-architect.sh [--output-dir DIR] [--dry-run]

Creates a dated ZIP containing only lean source/config files for code review:
  - Rust workspace: Cargo metadata, crates/**/*.rs, crates/**/*.toml
  - Browser extension: source, tests, manifests, HTML/CSS, TS/JS config
  - Source-bound assets: Rust prompt markdown and JSON schemas

Excluded by construction: docs, non-source markdown, target, dist outputs,
node_modules, planning/history folders, secrets, and other generated artifacts.
EOF
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_dir="${repo_root}/artifacts"
dry_run=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output-dir)
      [[ $# -ge 2 ]] || { echo "missing value for --output-dir" >&2; exit 2; }
      output_dir="$2"
      shift 2
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

command -v zip >/dev/null 2>&1 || {
  echo "zip is required but was not found on PATH" >&2
  exit 1
}

mkdir -p "$output_dir"
output_dir="$(cd "$output_dir" && pwd)"

date_stamp="$(date +%Y-%m-%d)"
base_name="rzn-browser-code-${date_stamp}"
zip_path="${output_dir}/${base_name}.zip"
suffix=2
while [[ -e "$zip_path" ]]; do
  zip_path="${output_dir}/${base_name}-${suffix}.zip"
  suffix=$((suffix + 1))
done

tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/rzn-browser-code.XXXXXX")"
manifest="${tmp_root}/manifest.txt"
staging="${tmp_root}/payload"
mkdir -p "$staging"

cleanup() {
  rm -rf "$tmp_root"
}
trap cleanup EXIT

cd "$repo_root"

{
  [[ -f Cargo.toml ]] && printf '%s\n' Cargo.toml
  [[ -f Cargo.lock ]] && printf '%s\n' Cargo.lock

  find crates \
    -type f \
    \( \
      -name '*.rs' -o \
      -name '*.toml' -o \
      -path '*/src/prompts/*.md' \
    \) \
    -print

  find extension \
    \( \
      -path 'extension/.build' -o \
      -path 'extension/.pw-user-data*' -o \
      -path 'extension/coverage' -o \
      -path 'extension/dist' -o \
      -path 'extension/dist-*' -o \
      -path 'extension/node_modules' -o \
      -path 'extension/test-results' \
    \) -prune -o \
    -type f \
    \( \
      -name '*.css' -o \
      -name '*.html' -o \
      -name '*.js' -o \
      -name '*.jsx' -o \
      -name '*.mjs' -o \
      -name '*.cjs' -o \
      -name '*.ts' -o \
      -name '*.tsx' -o \
      -name '*.json' -o \
      -name 'package.json' -o \
      -name 'tsconfig.json' -o \
      -name 'tsconfig.*.json' \
    \) \
    -print

  find schema \
    -type f \
    -name '*.json' \
    -print
} | sed 's#^\./##' | sort -u > "$manifest"

file_count="$(wc -l < "$manifest" | tr -d ' ')"
if [[ "$file_count" -eq 0 ]]; then
  echo "no matching source files found" >&2
  exit 1
fi

if [[ "$dry_run" -eq 1 ]]; then
  echo "would package ${file_count} files into:"
  echo "$zip_path"
  echo
  sed -n '1,240p' "$manifest"
  if [[ "$file_count" -gt 240 ]]; then
    echo "... (${file_count} total)"
  fi
  exit 0
fi

while IFS= read -r rel_path; do
  mkdir -p "${staging}/$(dirname "$rel_path")"
  cp -p "$rel_path" "${staging}/${rel_path}"
done < "$manifest"

(
  cd "$staging"
  zip -q -r -D "$zip_path" .
)

zip_size="$(du -h "$zip_path" | awk '{print $1}')"
echo "created: $zip_path"
echo "files:   $file_count"
echo "size:    $zip_size"
