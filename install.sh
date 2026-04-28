#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd 2>/dev/null || pwd)

detect_platform_slug() {
  os=$(uname -s)
  arch=$(uname -m)

  case "$os" in
    Darwin) os_slug="macos" ;;
    Linux) os_slug="linux" ;;
    *)
      echo "[ERROR] Unsupported OS: $os" >&2
      exit 1
      ;;
  esac

  case "$arch" in
    arm64|aarch64) arch_slug="arm64" ;;
    x86_64|amd64) arch_slug="x64" ;;
    *)
      echo "[ERROR] Unsupported architecture: $arch" >&2
      exit 1
      ;;
  esac

  printf '%s-%s\n' "$os_slug" "$arch_slug"
}

bootstrap_install() {
  if ! command -v curl >/dev/null 2>&1; then
    echo "[ERROR] curl is required for bootstrap installs." >&2
    exit 1
  fi
  if ! command -v tar >/dev/null 2>&1; then
    echo "[ERROR] tar is required for bootstrap installs." >&2
    exit 1
  fi

  platform_slug=$(detect_platform_slug)
  repo=${RZN_INSTALL_REPO:-srv1n/rzn-browser}
  artifact_name=${RZN_INSTALL_ARTIFACT:-rzn-browser-${platform_slug}.tar.gz}
  base_url=${RZN_INSTALL_BASE_URL:-https://github.com/${repo}/releases/latest/download}
  artifact_url=${RZN_INSTALL_URL:-${base_url}/${artifact_name}}

  workdir=$(mktemp -d "${TMPDIR:-/tmp}/rzn-install.XXXXXX")
  trap 'rm -rf "$workdir"' EXIT INT TERM

  echo "[INFO] Downloading ${artifact_url}"
  curl -fsSL "$artifact_url" -o "$workdir/$artifact_name"

  mkdir -p "$workdir/extract"
  tar -xzf "$workdir/$artifact_name" -C "$workdir/extract"

  extracted_root=""
  for candidate in "$workdir/extract"/*; do
    if [ -d "$candidate" ] && [ -f "$candidate/install.sh" ]; then
      extracted_root="$candidate"
      break
    fi
  done

  if [ -z "$extracted_root" ]; then
    echo "[ERROR] Release artifact did not contain an install.sh payload." >&2
    exit 1
  fi

  echo "[INFO] Running packaged installer from ${extracted_root}"
  sh "$extracted_root/install.sh"
}

if [ -f "$SCRIPT_DIR/setup.sh" ] && [ -f "$SCRIPT_DIR/Cargo.toml" ]; then
  export RZN_SETUP_PROFILE="${RZN_SETUP_PROFILE:-release}"
  export RZN_SETUP_FORCE_EXT_BUILD="${RZN_SETUP_FORCE_EXT_BUILD:-1}"
  export RZN_SETUP_INSTALL_GLOBAL_BIN="${RZN_SETUP_INSTALL_GLOBAL_BIN:-1}"
  exec bash "$SCRIPT_DIR/setup.sh"
fi

bootstrap_install
