#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

fail() {
  echo "[FAIL] $*" >&2
  exit 1
}

safe_rm_rf() {
  local target="${1:-}"
  local expected_root="${2:-}"
  local label="${3:-path}"

  if [[ -z "$target" || -z "$expected_root" ]]; then
    fail "refusing to remove empty $label"
  fi
  if [[ "$target" != /* || "$expected_root" != /* ]]; then
    fail "refusing to remove non-absolute $label"
  fi
  target="${target%/}"
  expected_root="${expected_root%/}"
  if [[ -z "$expected_root" || "$expected_root" == "/" ]]; then
    fail "refusing to remove $label under unsafe root"
  fi
  case "$target" in
    *"/../"*|*"/..") fail "refusing to remove $label containing '..'" ;;
  esac
  case "$target" in
    "$expected_root"/rzn-install-verification.*) rm -rf "$target" ;;
    *) fail "refusing to remove $label outside expected root: $target" ;;
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
    fail "sha256sum, shasum, or openssl is required"
  fi
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 is required"
}

write_fake_runtime_payload() {
  local payload_root="$1"
  mkdir -p "$payload_root"
  cat > "$payload_root/install.sh" <<'PAYLOAD'
#!/bin/sh
set -eu
if [ "${RZN_INSTALL_ARTIFACT_SHA256_VERIFIED:-0}" != "1" ]; then
  echo "missing checksum verification marker" >&2
  exit 64
fi
if [ -z "${RZN_INSTALL_ARTIFACT_SHA256:-}" ]; then
  echo "missing verified checksum value" >&2
  exit 65
fi
printf '%s\n' "$RZN_INSTALL_ARTIFACT_SHA256" > "$RZN_TEST_INSTALL_MARKER"
PAYLOAD
  chmod +x "$payload_root/install.sh"
}

write_fake_runtime_bundle() {
  local bundle_root="$1"
  mkdir -p "$bundle_root/bin" "$bundle_root/extension/dist-chrome"
  cp "$ROOT_DIR/scripts/release/install-runtime.sh" "$bundle_root/install.sh"
  chmod +x "$bundle_root/install.sh"
  cat > "$bundle_root/bin/rzn-browser" <<'BROWSER'
#!/bin/sh
set -eu
if [ "${1:-}" = "workflow" ] && [ "${2:-}" = "pull" ]; then
  exit 0
fi
exit 0
BROWSER
  cat > "$bundle_root/bin/rzn-native-host" <<'HOST'
#!/bin/sh
exit 0
HOST
  chmod +x "$bundle_root/bin/rzn-browser" "$bundle_root/bin/rzn-native-host"
  printf '{"manifest_version":3}\n' > "$bundle_root/extension/dist-chrome/manifest.json"
}

run_bootstrap_install() {
  local install_script="$1"
  local artifact="$2"
  local sidecar="$3"
  local marker="$4"
  local stderr_path="$5"

  RZN_INSTALL_ARTIFACT="$(basename "$artifact")" \
    RZN_INSTALL_URL="file://$artifact" \
    RZN_INSTALL_SHA256_URL="file://$sidecar" \
    RZN_TEST_INSTALL_MARKER="$marker" \
    sh "$install_script" >/dev/null 2>"$stderr_path"
}

run_runtime_install() {
  local bundle_root="$1"
  local install_root="$2"
  local global_bin="$3"
  local xattr_log="$4"
  local codesign_log="$5"
  local verified="$6"

  local fakebin
  fakebin="$(dirname "$xattr_log")/fakebin"
  mkdir -p "$fakebin"
  cat > "$fakebin/uname" <<'UNAME'
#!/bin/sh
if [ "${1:-}" = "-s" ]; then
  echo Darwin
else
  /usr/bin/uname "$@"
fi
UNAME
  cat > "$fakebin/xattr" <<'XATTR'
#!/bin/sh
printf '%s\n' "$*" >> "${RZN_XATTR_LOG:?missing RZN_XATTR_LOG}"
XATTR
  cat > "$fakebin/codesign" <<'CODESIGN'
#!/bin/sh
printf '%s\n' "$*" >> "${RZN_CODESIGN_LOG:?missing RZN_CODESIGN_LOG}"
case "${1:-}" in
  --verify) exit 1 ;;
  -dvv) printf '%s\n' "Signature=adhoc" >&2; exit 0 ;;
  --force) exit 0 ;;
  *) exit 0 ;;
esac
CODESIGN
  chmod +x "$fakebin/uname" "$fakebin/xattr" "$fakebin/codesign"

  if [[ "$verified" == "1" ]]; then
    RZN_INSTALL_ARTIFACT_SHA256_VERIFIED=1 \
      RZN_XATTR_LOG="$xattr_log" \
      RZN_CODESIGN_LOG="$codesign_log" \
      RZN_RUNTIME_DIR="$install_root" \
      RZN_SETUP_GLOBAL_BIN_DIR="$global_bin" \
      HOME="$(dirname "$install_root")/home" \
      PATH="$fakebin:$PATH" \
      sh "$bundle_root/install.sh" >/dev/null
  else
    RZN_XATTR_LOG="$xattr_log" \
      RZN_CODESIGN_LOG="$codesign_log" \
      RZN_RUNTIME_DIR="$install_root" \
      RZN_SETUP_GLOBAL_BIN_DIR="$global_bin" \
      HOME="$(dirname "$install_root")/home" \
      PATH="$fakebin:$PATH" \
      sh "$bundle_root/install.sh" >/dev/null
  fi
}

run_bundle_macos_install() {
  local bundle_root="$1"
  local install_root="$2"
  local global_bin="$3"
  local host_dir="$4"
  local xattr_log="$5"
  local codesign_log="$6"
  local verified="$7"

  local fakebin
  fakebin="$(dirname "$xattr_log")/fakebin"
  mkdir -p "$fakebin"
  cat > "$fakebin/xattr" <<'XATTR'
#!/bin/sh
printf '%s\n' "$*" >> "${RZN_XATTR_LOG:?missing RZN_XATTR_LOG}"
XATTR
  cat > "$fakebin/codesign" <<'CODESIGN'
#!/bin/sh
printf '%s\n' "$*" >> "${RZN_CODESIGN_LOG:?missing RZN_CODESIGN_LOG}"
case "${1:-}" in
  --verify) exit 1 ;;
  -dvv) printf '%s\n' "Signature=adhoc" >&2; exit 0 ;;
  --force) exit 0 ;;
  *) exit 0 ;;
esac
CODESIGN
  chmod +x "$fakebin/xattr" "$fakebin/codesign"

  if [[ "$verified" == "1" ]]; then
    RZN_INSTALL_ARTIFACT_SHA256_VERIFIED=1 \
      RZN_XATTR_LOG="$xattr_log" \
      RZN_CODESIGN_LOG="$codesign_log" \
      RZN_BUNDLE_INSTALL_ROOT="$install_root" \
      RZN_BUNDLE_GLOBAL_BIN_DIR="$global_bin" \
      RZN_BUNDLE_CHROME_HOST_DIR="$host_dir" \
      HOME="$(dirname "$install_root")/home" \
      PATH="$fakebin:$PATH" \
      bash "$bundle_root/install-macos.sh" >/dev/null
  else
    RZN_XATTR_LOG="$xattr_log" \
      RZN_CODESIGN_LOG="$codesign_log" \
      RZN_BUNDLE_INSTALL_ROOT="$install_root" \
      RZN_BUNDLE_GLOBAL_BIN_DIR="$global_bin" \
      RZN_BUNDLE_CHROME_HOST_DIR="$host_dir" \
      HOME="$(dirname "$install_root")/home" \
      PATH="$fakebin:$PATH" \
      bash "$bundle_root/install-macos.sh" >/dev/null
  fi
}

require_cmd curl
require_cmd tar
require_cmd awk

tmp_root="${TMPDIR:-/tmp}"
tmp_root="${tmp_root%/}"
[[ "$tmp_root" == /* ]] || fail "TMPDIR must be absolute"
tmp_dir="$(mktemp -d "$tmp_root/rzn-install-verification.XXXXXX")"
trap 'safe_rm_rf "$tmp_dir" "$tmp_root" "test workdir"' EXIT INT TERM

bootstrap_dir="$tmp_dir/bootstrap"
payload_dir="$tmp_dir/payload"
mkdir -p "$bootstrap_dir" "$payload_dir"
cp "$ROOT_DIR/install.sh" "$bootstrap_dir/install.sh"

artifact="$tmp_dir/rzn-browser-test.tar.gz"
sidecar="$artifact.sha256"
write_fake_runtime_payload "$payload_dir/rzn-browser-test"
tar -czf "$artifact" -C "$payload_dir" "rzn-browser-test"
printf '%s  %s\n' "$(sha256_file "$artifact")" "$(basename "$artifact")" > "$sidecar"

marker="$tmp_dir/installed-ok"
run_bootstrap_install "$bootstrap_dir/install.sh" "$artifact" "$sidecar" "$marker" "$tmp_dir/success.err"
[[ -s "$marker" ]] || fail "verified artifact did not run packaged installer"

rm -f "$marker"
if run_bootstrap_install "$bootstrap_dir/install.sh" "$artifact" "$tmp_dir/missing.sha256" "$marker" "$tmp_dir/missing.err"; then
  fail "install succeeded with missing sha256 sidecar"
fi
[[ ! -e "$marker" ]] || fail "packaged installer ran despite missing sidecar"

printf '%064d  %s\n' 0 "$(basename "$artifact")" > "$sidecar"
if run_bootstrap_install "$bootstrap_dir/install.sh" "$artifact" "$sidecar" "$marker" "$tmp_dir/mismatch.err"; then
  fail "install succeeded with mismatched sha256 sidecar"
fi
[[ ! -e "$marker" ]] || fail "packaged installer ran despite checksum mismatch"

bundle_root="$tmp_dir/runtime-bundle"
write_fake_runtime_bundle "$bundle_root"

unverified_xattr_log="$tmp_dir/unverified-xattr.log"
unverified_codesign_log="$tmp_dir/unverified-codesign.log"
run_runtime_install "$bundle_root" "$tmp_dir/runtime-unverified" "$tmp_dir/global-unverified" "$unverified_xattr_log" "$unverified_codesign_log" 0
[[ ! -s "$unverified_xattr_log" ]] || fail "xattr was called without checksum verification"
[[ ! -s "$unverified_codesign_log" ]] || fail "codesign was called without checksum verification"

verified_xattr_log="$tmp_dir/verified-xattr.log"
verified_codesign_log="$tmp_dir/verified-codesign.log"
run_runtime_install "$bundle_root" "$tmp_dir/runtime-verified" "$tmp_dir/global-verified" "$verified_xattr_log" "$verified_codesign_log" 1
[[ -s "$verified_xattr_log" ]] || fail "xattr was not called after checksum verification"
grep -q -- "--force" "$verified_codesign_log" || fail "codesign repair was not called after checksum verification"

bundle_macos_root="$tmp_dir/bundle-macos"
write_fake_runtime_bundle "$bundle_macos_root"
cp "$ROOT_DIR/scripts/bundle/install-macos.sh" "$bundle_macos_root/install-macos.sh"
chmod +x "$bundle_macos_root/install-macos.sh"

bundle_unverified_xattr_log="$tmp_dir/bundle-unverified-xattr.log"
bundle_unverified_codesign_log="$tmp_dir/bundle-unverified-codesign.log"
run_bundle_macos_install "$bundle_macos_root" "$tmp_dir/bundle-runtime-unverified" "$tmp_dir/bundle-global-unverified" "$tmp_dir/bundle-host-unverified" "$bundle_unverified_xattr_log" "$bundle_unverified_codesign_log" 0
[[ ! -s "$bundle_unverified_xattr_log" ]] || fail "bundle xattr was called without checksum verification"
[[ ! -s "$bundle_unverified_codesign_log" ]] || fail "bundle codesign was called without checksum verification"

bundle_verified_xattr_log="$tmp_dir/bundle-verified-xattr.log"
bundle_verified_codesign_log="$tmp_dir/bundle-verified-codesign.log"
run_bundle_macos_install "$bundle_macos_root" "$tmp_dir/bundle-runtime-verified" "$tmp_dir/bundle-global-verified" "$tmp_dir/bundle-host-verified" "$bundle_verified_xattr_log" "$bundle_verified_codesign_log" 1
[[ -s "$bundle_verified_xattr_log" ]] || fail "bundle xattr was not called after checksum verification"
grep -q -- "--force" "$bundle_verified_codesign_log" || fail "bundle codesign repair was not called after checksum verification"

echo "[OK] install verification tests passed"
