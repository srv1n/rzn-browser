#!/bin/bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  run.sh --term "<search_term>" [--country "us"] [--show-log]
USAGE
}

if [ "${1:-}" = "" ] || [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
  usage
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
RUNNER="$ROOT_DIR/skills/amazon-appstore-workflows/scripts/run_workflow.sh"

if [ ! -x "$RUNNER" ]; then
  echo "Missing shared runner: $RUNNER" >&2
  exit 1
fi

exec "$RUNNER" appstore_search_snapshot "$@"
