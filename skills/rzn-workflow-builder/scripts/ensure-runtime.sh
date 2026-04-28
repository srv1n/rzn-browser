#!/usr/bin/env bash
set -euo pipefail

show_help() {
  cat <<'EOF'
Usage: ensure-runtime.sh

Probe the local RZN Browser runtime first. If the CLI/catalog probe fails,
run `make install` from the repo root and then `make doctor`.
EOF
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  show_help
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

cd "${REPO_ROOT}"

echo "[INFO] Probing local RZN runtime..."
if command -v rzn-browser >/dev/null 2>&1; then
  if rzn-browser list google >/dev/null 2>&1; then
    echo "[OK] rzn-browser is installed and the workflow catalog resolves."
    echo "[INFO] Running doctor check..."
    make doctor
    exit 0
  fi
fi

echo "[WARN] RZN runtime probe failed. Running make install..."
make install

echo "[INFO] Running doctor check..."
if make doctor; then
  echo "[OK] Runtime install and doctor check completed."
  exit 0
fi

cat <<'EOF'
[FAIL] Install completed but doctor still failed.

Manual steps:
1. Open chrome://extensions
2. Enable Developer mode
3. Load unpacked from the stable extension copy
   macOS: ~/Library/Application Support/RZN/extension/dist-chrome
   Linux: ~/.local/share/RZN/extension/dist-chrome
4. Keep a Chrome window open
5. Rerun: make doctor
EOF

exit 1
