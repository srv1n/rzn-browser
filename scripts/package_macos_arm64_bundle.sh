#!/bin/bash
set -euo pipefail

# Build + package a shareable macOS arm64 bundle containing:
# - rzn-browser + rzn-native-host
# - unpacked Chrome extension (dist-chrome)
# - workflows + packaged examples
# - install/doctor scripts + README

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

usage() {
  cat <<'EOF'
Usage: scripts/package_macos_arm64_bundle.sh [--force-ext] [--skip-rust] [--skip-ext] [--out-name NAME]

Environment overrides:
  RZN_BUNDLE_HOST_NAME           Native host name (default: com.rzn.browser.broker)
  RZN_BUNDLE_EXTENSION_ID        Allowed extension ID (default: bogjdnehdficgkhklinmnbgiiofbamji)
  RZN_BUNDLE_FORCE_EXT_BUILD=1   Force rebuild extension even if dist-chrome exists
  RZN_BUNDLE_SKIP_RUST=1         Skip cargo build
  RZN_BUNDLE_SKIP_EXT=1          Skip extension build

Output:
  dist/<NAME>.zip
EOF
}

FORCE_EXT=0
SKIP_RUST="${RZN_BUNDLE_SKIP_RUST:-0}"
SKIP_EXT="${RZN_BUNDLE_SKIP_EXT:-0}"
OUT_NAME=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    --force-ext) FORCE_EXT=1; shift ;;
    --skip-rust) SKIP_RUST=1; shift ;;
    --skip-ext) SKIP_EXT=1; shift ;;
    --out-name)
      OUT_NAME="${2:-}"
      shift 2
      ;;
    *)
      echo "[ERROR] Unknown arg: $1"
      usage
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "[WARN] Not macOS (uname -s=$(uname -s)); continuing anyway."
fi
if [[ "$(uname -m)" != "arm64" ]]; then
  echo "[WARN] Not arm64 (uname -m=$(uname -m)); bundle may not run on Apple Silicon."
fi

# Keep these defaults in sync with setup.sh.
HOST_NAME="${RZN_BUNDLE_HOST_NAME:-com.rzn.browser.broker}"
EXTENSION_ID="${RZN_BUNDLE_EXTENSION_ID:-bogjdnehdficgkhklinmnbgiiofbamji}"

# Best-effort bundle naming: include git SHA if available, otherwise date only.
GIT_SHA=""
if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || true)"
fi
STAMP="$(date +"%Y%m%d_%H%M%S")"
if [[ -z "$OUT_NAME" ]]; then
  if [[ -n "$GIT_SHA" ]]; then
    OUT_NAME="rzn_macos_arm64_${STAMP}_${GIT_SHA}"
  else
    OUT_NAME="rzn_macos_arm64_${STAMP}"
  fi
fi

echo "[INFO] Host name    : $HOST_NAME"
echo "[INFO] Extension ID : $EXTENSION_ID"
echo "[INFO] Output name  : $OUT_NAME"

if [[ "$SKIP_RUST" != "1" ]]; then
  echo "[INFO] Building Rust (release): rzn-browser + rzn-native-host"
  cargo build --release -p rzn-browser -p rzn-native-host
else
  echo "[INFO] Skipping Rust build (RZN_BUNDLE_SKIP_RUST=1)"
fi

if [[ "$SKIP_EXT" != "1" ]]; then
  NEED_EXT_BUILD=0
  if [[ "${RZN_BUNDLE_FORCE_EXT_BUILD:-0}" == "1" || "$FORCE_EXT" == "1" ]]; then
    NEED_EXT_BUILD=1
  elif [[ ! -f "extension/dist-chrome/manifest.json" ]]; then
    NEED_EXT_BUILD=1
  fi

  if [[ "$NEED_EXT_BUILD" == "1" ]]; then
    echo "[INFO] Building extension (dist-chrome)"
    pushd extension >/dev/null
    if [[ ! -d "node_modules" ]]; then
      bun install
    fi
    bun run build
    popd >/dev/null
    bun scripts/build-ext.ts
  else
    echo "[INFO] extension/dist-chrome exists; skipping extension build (use --force-ext to rebuild)"
  fi
else
  echo "[INFO] Skipping extension build (RZN_BUNDLE_SKIP_EXT=1)"
fi

if [[ ! -x "target/release/rzn-native-host" ]]; then
  echo "[ERROR] Missing built native host at target/release/rzn-native-host"
  exit 1
fi
if [[ ! -x "target/release/rzn-browser" ]]; then
  echo "[ERROR] Missing built CLI at target/release/rzn-browser"
  exit 1
fi
if [[ ! -f "extension/dist-chrome/manifest.json" ]]; then
  echo "[ERROR] Missing extension build at extension/dist-chrome/manifest.json"
  exit 1
fi

DIST_DIR="dist"
STAGE_DIR="${DIST_DIR}/${OUT_NAME}"
ZIP_PATH="${DIST_DIR}/${OUT_NAME}.zip"

rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR"

echo "[INFO] Staging bundle at: $STAGE_DIR"
mkdir -p "${STAGE_DIR}/bin"
cp -f target/release/rzn-native-host "${STAGE_DIR}/bin/rzn-native-host"
cp -f target/release/rzn-browser "${STAGE_DIR}/bin/rzn-browser"
chmod +x "${STAGE_DIR}/bin/rzn-native-host" "${STAGE_DIR}/bin/rzn-browser" || true

mkdir -p "${STAGE_DIR}/extension"
cp -R "extension/dist-chrome" "${STAGE_DIR}/extension/dist-chrome"

cp -R "workflows" "${STAGE_DIR}/workflows"
cp -R "skills" "${STAGE_DIR}/skills"
mkdir -p "${STAGE_DIR}/examples"
cp -R "examples/browser_automation" "${STAGE_DIR}/examples/browser_automation"
cp -R "schema" "${STAGE_DIR}/schema"
if [[ -f ".env.example" ]]; then
  cp -f ".env.example" "${STAGE_DIR}/.env.example"
fi

# Copy bundle templates and substitute placeholders.
cp -f "scripts/bundle/install-macos.sh" "${STAGE_DIR}/install-macos.sh"
cp -f "scripts/bundle/doctor-macos.sh" "${STAGE_DIR}/doctor-macos.sh"
cp -f "scripts/bundle/README.md" "${STAGE_DIR}/README.md"
cp -f "scripts/bundle/AGENTS.md" "${STAGE_DIR}/AGENTS.md"
chmod +x "${STAGE_DIR}/install-macos.sh" "${STAGE_DIR}/doctor-macos.sh" || true

python3 - <<PY
import pathlib

stage = pathlib.Path("$STAGE_DIR")
repls = {
  "__RZN_NATIVE_HOST_NAME__": "$HOST_NAME",
  "__RZN_EXTENSION_ID__": "$EXTENSION_ID",
}
for rel in ["install-macos.sh", "doctor-macos.sh", "README.md"]:
  p = stage / rel
  s = p.read_text()
  for k, v in repls.items():
    s = s.replace(k, v)
  p.write_text(s)
PY

# Include a small provenance file for debugging.
cat > "${STAGE_DIR}/BUNDLE_INFO.txt" <<EOF
name=${OUT_NAME}
created=${STAMP}
git_sha=${GIT_SHA}
host_name=${HOST_NAME}
extension_id=${EXTENSION_ID}
EOF

rm -f "$ZIP_PATH"
echo "[INFO] Creating zip: $ZIP_PATH"
(
  cd "$DIST_DIR"
  # zip preserves executable bits; keep it simple for sharing.
  zip -qr "${OUT_NAME}.zip" "${OUT_NAME}"
)

echo "[OK] Bundle created: $ZIP_PATH"
echo "[INFO] Next: share the zip, then your friend runs install-macos.sh and loads extension/dist-chrome"
