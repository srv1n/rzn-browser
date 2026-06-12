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

guarded_rm_rf() {
  guard_target=${1:-}
  guard_root=${2:-}
  guard_label=${3:-path}

  if [ -z "$guard_target" ] || [ -z "$guard_root" ]; then
    echo "[ERROR] Refusing to remove empty ${guard_label}." >&2
    return 1
  fi
  case "$guard_target" in
    /*) ;;
    *)
      echo "[ERROR] Refusing to remove non-absolute ${guard_label}: $guard_target" >&2
      return 1
      ;;
  esac
  case "$guard_root" in
    /*) ;;
    *)
      echo "[ERROR] Refusing to remove ${guard_label}; expected root is not absolute: $guard_root" >&2
      return 1
      ;;
  esac
  guard_target=${guard_target%/}
  guard_root=${guard_root%/}
  if [ -z "$guard_root" ] || [ "$guard_root" = "/" ]; then
    echo "[ERROR] Refusing to remove ${guard_label}; expected root is unsafe: $guard_root" >&2
    return 1
  fi
  case "$guard_target" in
    *"/../"*|*"/..")
      echo "[ERROR] Refusing to remove ${guard_label} containing '..': $guard_target" >&2
      return 1
      ;;
  esac
  case "$guard_target" in
    "$guard_root"/rzn-install.*) rm -rf "$guard_target" ;;
    *)
      echo "[ERROR] Refusing to remove ${guard_label} outside expected root: $guard_target" >&2
      return 1
      ;;
  esac
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 -r "$1" | awk '{print $1}'
  else
    echo "[ERROR] sha256sum, shasum, or openssl is required to verify release artifacts." >&2
    exit 1
  fi
}

verify_sha256_sidecar() {
  verify_artifact=$1
  verify_sidecar=$2
  verify_artifact_name=$3

  if [ ! -s "$verify_sidecar" ]; then
    echo "[ERROR] Missing or empty sha256 sidecar: $verify_sidecar" >&2
    exit 1
  fi

  expected_sha256=$(awk 'NR == 1 {print $1}' "$verify_sidecar")
  sidecar_artifact=$(awk 'NR == 1 {print $2}' "$verify_sidecar" | sed 's/^\*//')
  if ! printf '%s\n' "$expected_sha256" | grep -Eq '^[0-9a-fA-F]{64}$'; then
    echo "[ERROR] Invalid sha256 sidecar format: $verify_sidecar" >&2
    exit 1
  fi
  if [ -n "$sidecar_artifact" ] && [ "${sidecar_artifact##*/}" != "$verify_artifact_name" ]; then
    echo "[ERROR] sha256 sidecar is for ${sidecar_artifact##*/}, not $verify_artifact_name." >&2
    exit 1
  fi

  actual_sha256=$(sha256_file "$verify_artifact")
  expected_sha256_lc=$(printf '%s' "$expected_sha256" | tr 'A-F' 'a-f')
  actual_sha256_lc=$(printf '%s' "$actual_sha256" | tr 'A-F' 'a-f')
  if [ "$actual_sha256_lc" != "$expected_sha256_lc" ]; then
    echo "[ERROR] Checksum mismatch for $verify_artifact_name." >&2
    echo "        expected: $expected_sha256_lc" >&2
    echo "        actual:   $actual_sha256_lc" >&2
    exit 1
  fi

  printf '%s\n' "$actual_sha256_lc"
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
  case "$artifact_name" in
    ""|*/*)
      echo "[ERROR] RZN_INSTALL_ARTIFACT must be a file name, got: $artifact_name" >&2
      exit 1
      ;;
  esac
  version=${RZN_INSTALL_VERSION:-}
  case "$version" in
    "") release_path="releases/latest/download" ;;
    *[!A-Za-z0-9._-]*)
      echo "[ERROR] RZN_INSTALL_VERSION must be a release tag, got: $version" >&2
      exit 1
      ;;
    *) release_path="releases/download/${version}" ;;
  esac
  base_url=${RZN_INSTALL_BASE_URL:-https://github.com/${repo}/${release_path}}
  artifact_url=${RZN_INSTALL_URL:-${base_url}/${artifact_name}}
  sha256_url=${RZN_INSTALL_SHA256_URL:-${artifact_url}.sha256}

  temp_root=${TMPDIR:-/tmp}
  temp_root=${temp_root%/}
  case "$temp_root" in
    /*) ;;
    *)
      echo "[ERROR] TMPDIR must be absolute for bootstrap installs: $temp_root" >&2
      exit 1
      ;;
  esac
  workdir=$(mktemp -d "$temp_root/rzn-install.XXXXXX")
  trap 'guarded_rm_rf "$workdir" "$temp_root" "rzn-install workdir"' EXIT INT TERM
  artifact_path="$workdir/$artifact_name"
  sha256_path="$artifact_path.sha256"

  echo "[INFO] Downloading ${artifact_url}"
  resolved_url=$(curl -fsSL -w '%{url_effective}' "$artifact_url" -o "$artifact_path")
  if [ -n "$resolved_url" ] && [ "$resolved_url" != "$artifact_url" ]; then
    echo "[INFO] Resolved release artifact: $resolved_url"
  fi

  echo "[INFO] Downloading ${sha256_url}"
  if ! curl -fsSL "$sha256_url" -o "$sha256_path"; then
    echo "[ERROR] Missing sha256 sidecar: $sha256_url" >&2
    exit 1
  fi

  verified_sha256=$(verify_sha256_sidecar "$artifact_path" "$sha256_path" "$artifact_name")
  echo "[INFO] Verified sha256: $verified_sha256"

  mkdir -p "$workdir/extract"
  tar -xzf "$artifact_path" -C "$workdir/extract"

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
  RZN_INSTALL_ARTIFACT_SHA256_VERIFIED=1 RZN_INSTALL_ARTIFACT_SHA256="$verified_sha256" sh "$extracted_root/install.sh"
}

if [ -f "$SCRIPT_DIR/setup.sh" ] && [ -f "$SCRIPT_DIR/Cargo.toml" ]; then
  export RZN_SETUP_PROFILE="${RZN_SETUP_PROFILE:-release}"
  export RZN_SETUP_FORCE_EXT_BUILD="${RZN_SETUP_FORCE_EXT_BUILD:-1}"
  export RZN_SETUP_INSTALL_GLOBAL_BIN="${RZN_SETUP_INSTALL_GLOBAL_BIN:-1}"
  exec bash "$SCRIPT_DIR/setup.sh"
fi

bootstrap_install
