#!/bin/bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  run.sh --listing-url "<etsy_listing_url>" [--show-log]
EOF
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

exec "$RUNNER" etsy_listing "$@"

