#!/bin/bash
set -euo pipefail

# Deterministic local E2E harness for `rzn-browser llm-auto` without hitting real websites.
#
# Requirements:
# - Chrome extension loaded (from `extension/dist-chrome/`)
# - Native host runtime running (spawned by Chrome via native messaging)
# - CLI built (`cargo build --release -p rzn-browser`)
#
# Notes:
# - Uses the local fixtures under `test/fixtures/ecommerce/`
# - Uses the deterministic macro (top-N links + comment extraction) in `LLMAutonomousPlanner`

PORT="${PORT:-7331}"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
FIXTURES_DIR="$ROOT_DIR/test/fixtures"

if [ ! -x "$ROOT_DIR/target/release/rzn-browser" ] || find "$ROOT_DIR/crates/rzn_browser" "$ROOT_DIR/crates/rzn_plan" "$ROOT_DIR/crates/rzn_core" -type f -newer "$ROOT_DIR/target/release/rzn-browser" | head -n 1 | grep -q .; then
  echo "[WARN] rzn-browser is missing or stale. Building rzn-browser (release)..."
  (cd "$ROOT_DIR" && cargo build --release -p rzn-browser)
fi

echo "[INFO] Starting local fixture server on http://127.0.0.1:$PORT ..."
pushd "$FIXTURES_DIR" >/dev/null
python3 -m http.server "$PORT" >/dev/null 2>&1 &
SERVER_PID=$!
popd >/dev/null

cleanup() {
  if kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

START_URL="http://127.0.0.1:$PORT/ecommerce/search.html"
INSTRUCTION="Open the top 3 links and extract the comments"

echo "[INFO] Running llm-auto against fixtures"
echo "  start_url: $START_URL"
echo "  instruction: $INSTRUCTION"
echo ""

RUST_LOG=info \
  RZN_TRANSPORT="${RZN_TRANSPORT:-pipe}" \
  LLM_PROVIDER="${LLM_PROVIDER:-dummy}" \
  "$ROOT_DIR/target/release/rzn-browser" llm-auto --json --url "$START_URL" "$INSTRUCTION"
