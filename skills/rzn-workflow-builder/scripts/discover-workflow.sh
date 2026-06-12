#!/usr/bin/env bash
set -euo pipefail

show_help() {
  cat <<'EOF'
Usage: discover-workflow.sh "<goal>"

Run the RZN llm-auto workflow-factory loop and save the resulting workflow JSON.

Environment overrides:
  LLM_PROVIDER        default: dummy
  MAX_STEPS           default: 20
  RZN_WORKFLOWS_DIR   default: <repo>/workflows/generated
EOF
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" || $# -lt 1 ]]; then
  show_help
  exit $(( $# < 1 ? 1 : 0 ))
fi

GOAL="$1"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

"${SCRIPT_DIR}/ensure-runtime.sh"

cd "${REPO_ROOT}"

export LLM_PROVIDER="${LLM_PROVIDER:-dummy}"
export MAX_STEPS="${MAX_STEPS:-20}"
export RZN_WORKFLOWS_DIR="${RZN_WORKFLOWS_DIR:-${REPO_ROOT}/workflows/generated}"

mkdir -p "${RZN_WORKFLOWS_DIR}"

echo "[INFO] Saving generated workflows into: ${RZN_WORKFLOWS_DIR}"
echo "[INFO] Provider: ${LLM_PROVIDER}"

if command -v rzn-browser >/dev/null 2>&1; then
  exec rzn-browser llm-auto "${GOAL}" --max-steps "${MAX_STEPS}" --save-workflow
fi

if [[ -x "./target/release/rzn-browser" ]]; then
  exec ./target/release/rzn-browser llm-auto "${GOAL}" --max-steps "${MAX_STEPS}" --save-workflow
fi

exec cargo run -p rzn-browser -- llm-auto "${GOAL}" --max-steps "${MAX_STEPS}" --save-workflow
