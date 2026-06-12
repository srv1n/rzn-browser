#!/bin/bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  run.sh --adam-id "<adam_id>" --adgroup-id "<adgroup_id>" --query "<keyword_seed>" [--storefront "us"] [--show-log]
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

exec "$RUNNER" apple_ads_keyword_recs "$@"
