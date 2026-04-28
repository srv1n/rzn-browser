#!/usr/bin/env bash
set -euo pipefail

LOG_FILE="${HOME}/rzn_build.log"

usage() {
  cat <<EOF
RZN Unified Logger

Usage:
  logger.sh follow            # tail -f unified log (all components)
  logger.sh follow-json       # tail -f, pretty JSON (requires jq)
  logger.sh show [N]          # show last N lines (default 200)
  logger.sh clear             # rotate unified log
  logger.sh where             # print log file locations

Files:
  Unified: $LOG_FILE  (extension + CLI + native host JSON lines)
EOF
}

cmd=${1:-help}
shift || true

case "$cmd" in
  follow)
    echo "Tailing unified log: $LOG_FILE"
    touch "$LOG_FILE"
    tail -F "$LOG_FILE"
    ;;
  follow-json)
    if ! command -v jq >/dev/null 2>&1; then
      echo "jq is required for follow-json. Install jq or use 'follow'." >&2
      exit 1
    fi
    echo "Tailing unified log (pretty JSON): $LOG_FILE"
    touch "$LOG_FILE"
    tail -F "$LOG_FILE" | jq -r 'select(.timestamp?) | "[\(.level)] \(.component) \(.timestamp) — \(.message) \(.data // {})"'
    ;;
  show)
    N=${1:-200}
    echo "Last $N lines of $LOG_FILE:"
    tail -n "$N" "$LOG_FILE" || true
    ;;
  clear)
    if [ -f "$LOG_FILE" ]; then
      mv "$LOG_FILE" "${LOG_FILE}.$(date +%Y%m%d_%H%M%S)" || true
    fi
    : > "$LOG_FILE"
    echo "Cleared $LOG_FILE"
    ;;
  where)
    echo "Unified log : $LOG_FILE"
    ;;
  *)
    usage
    ;;
esac
