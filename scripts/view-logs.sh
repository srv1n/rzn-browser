#!/usr/bin/env bash
set -euo pipefail

LOG_FILE="${HOME}/rzn_build.log"

echo "RZN Logs (unified): $LOG_FILE"
echo "Examples:"
echo "  ./scripts/logger.sh follow"
echo "  ./scripts/logger.sh follow-json   # pretty (jq required)"
echo "  ./scripts/logger.sh show 300"
echo "  ./scripts/logger.sh clear"

