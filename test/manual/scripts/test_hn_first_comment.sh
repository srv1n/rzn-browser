#!/usr/bin/env bash
set -euo pipefail

# Minimal smoke test for the generic first-comment iterator macro.
# Requires extension + native host runtime (make build) and Chrome available.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$ROOT_DIR"

export LLM_PROVIDER=dummy
export RUST_LOG=info

echo "Running: rzn-browser llm-auto 'First comment of top 5 posts of hackernews'"
./target/release/rzn-browser llm-auto "First comment of top 5 posts of hackernews" --max-steps 5 || true
