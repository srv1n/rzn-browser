#!/usr/bin/env bash
set -euo pipefail

# Manual smoke test for the LLM autonomous planner on a hard multi-step form wizard.
#
# Prereqs:
# - Extension built + loaded in Chrome (native host runtime running)
# - LLM provider configured:
#   - OpenAI API: export OPENAI_API_KEY=... and optionally OPENAI_MODEL_PLANNING=...
#   - Or CLI providers (after implementation): export LLM_PROVIDER=claude-cli|gemini-cli|codex-cli
#
# This script serves `test/fixtures/form_wizard.html` over HTTP and asks the planner to
# fill valid values and stop at the Review step (do not press Submit).

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
FIXTURE_DIR="$ROOT_DIR/test/fixtures"
PORT="${PORT:-41733}"

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

echo "[INFO] Serving fixture from: $FIXTURE_DIR on http://127.0.0.1:${PORT}/form_wizard.html"
python3 -m http.server "$PORT" --bind 127.0.0.1 --directory "$FIXTURE_DIR" >/dev/null 2>&1 &
SERVER_PID="$!"
sleep 0.4

URL="http://127.0.0.1:${PORT}/form_wizard.html"

INSTRUCTION=$(
  cat <<EOF
Go to ${URL} and complete the multi-step Signup Wizard.

Fill in valid values so all validation errors disappear and you reach the Review step.
Do NOT click the final Submit button.

Use these values:
- Email: rzn.tester@example.com
- Password: rznpass12345
- Plan: Pro
- First name: Ada
- Last name: Lovelace
- ZIP: 94107
- Accept terms: checked

When you are on the Review step and the summary shows the values, call complete.
EOF
)

echo "[INFO] Running rzn-browser llm-auto..."
cd "$ROOT_DIR"

# NOTE: deterministic workflows now run through `rzn-browser run`; for llm-auto use the built binary if available.
# If you haven't built yet, run `cargo build --release` and use `./target/release/rzn-browser`.

if [[ -x "./target/release/rzn-browser" ]]; then
  ./target/release/rzn-browser llm-auto "$INSTRUCTION" --max-steps 30
else
  cargo run -q -p rzn-browser -- llm-auto "$INSTRUCTION" --max-steps 30
fi
