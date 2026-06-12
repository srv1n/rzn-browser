#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

fail() {
  echo "[FAIL] $*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 is required"
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
    "$expected_root"/rzn-install-ps1-verification.*) rm -rf "$target" ;;
    *) fail "refusing to remove $label outside expected root: $target" ;;
  esac
}

write_fake_windows_payload() {
  local payload_root="$1"
  mkdir -p "$payload_root"
  cat > "$payload_root/install.ps1" <<'PAYLOAD'
$ErrorActionPreference = "Stop"

if ($env:RZN_INSTALL_ARTIFACT_SHA256_VERIFIED -ne "1") {
  throw "missing checksum verification marker"
}
if ([string]::IsNullOrWhiteSpace($env:RZN_INSTALL_ARTIFACT_SHA256)) {
  throw "missing verified checksum value"
}
if ([string]::IsNullOrWhiteSpace($env:RZN_TEST_INSTALL_MARKER)) {
  throw "missing RZN_TEST_INSTALL_MARKER"
}

Set-Content -LiteralPath $env:RZN_TEST_INSTALL_MARKER -Value $env:RZN_INSTALL_ARTIFACT_SHA256 -NoNewline
PAYLOAD
}

zip_payload() {
  local source_dir="$1"
  local artifact="$2"
  python3 - "$source_dir" "$artifact" <<'PY'
import os
import sys
import zipfile

source_dir, artifact = sys.argv[1:3]
with zipfile.ZipFile(artifact, "w", compression=zipfile.ZIP_DEFLATED) as archive:
    for root, _, files in os.walk(source_dir):
        for name in files:
            path = os.path.join(root, name)
            archive.write(path, os.path.relpath(path, source_dir))
PY
}

pick_free_port() {
  python3 - <<'PY'
import socket

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
}

run_bootstrap_install_ps1() {
  local pwsh="$1"
  local artifact_name="$2"
  local artifact_url="$3"
  local sidecar_url="$4"
  local marker="$5"
  local stderr_path="$6"

  RZN_INSTALL_ARTIFACT="$artifact_name" \
    RZN_INSTALL_URL="$artifact_url" \
    RZN_INSTALL_SHA256_URL="$sidecar_url" \
    RZN_TEST_INSTALL_MARKER="$marker" \
    "$pwsh" -NoLogo -NoProfile -File "$ROOT_DIR/install.ps1" >/dev/null 2>"$stderr_path"
}

require_cmd python3
require_cmd curl

tmp_root="${TMPDIR:-/tmp}"
tmp_root="${tmp_root%/}"
[[ "$tmp_root" == /* ]] || fail "TMPDIR must be absolute"
tmp_dir="$(mktemp -d "$tmp_root/rzn-install-ps1-verification.XXXXXX")"
server_pid=""
trap '[[ -n "$server_pid" ]] && kill "$server_pid" >/dev/null 2>&1 || true; safe_rm_rf "$tmp_dir" "$tmp_root" "test workdir"' EXIT INT TERM

pwsh="${RZN_PWSH:-}"
if [[ -z "$pwsh" ]]; then
  if command -v pwsh >/dev/null 2>&1; then
    pwsh="$(command -v pwsh)"
  elif command -v powershell >/dev/null 2>&1; then
    pwsh="$(command -v powershell)"
  elif [[ -n "${RZN_PWSH_DOWNLOAD_URL:-}" ]]; then
    require_cmd tar
    pwsh_dir="$tmp_dir/pwsh-portable"
    mkdir -p "$pwsh_dir"
    curl -fsSL "$RZN_PWSH_DOWNLOAD_URL" -o "$tmp_dir/pwsh.tar.gz"
    tar -xzf "$tmp_dir/pwsh.tar.gz" -C "$pwsh_dir"
    pwsh="$pwsh_dir/pwsh"
    chmod +x "$pwsh"
  else
    fail "pwsh or powershell is required; set RZN_PWSH or opt in with RZN_PWSH_DOWNLOAD_URL"
  fi
fi
[[ -x "$pwsh" ]] || fail "PowerShell binary is not executable: $pwsh"

server_root="$tmp_dir/server"
payload_dir="$tmp_dir/payload"
mkdir -p "$server_root" "$payload_dir"

artifact_name="rzn-browser-windows-x64.zip"
artifact="$server_root/$artifact_name"
sidecar="$artifact.sha256"
write_fake_windows_payload "$payload_dir/rzn-browser-test"
zip_payload "$payload_dir" "$artifact"
printf '%s  %s\n' "$(sha256_file "$artifact")" "$artifact_name" > "$sidecar"

port="$(pick_free_port)"
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$server_root" >/dev/null 2>"$tmp_dir/http.err" &
server_pid="$!"

for _ in {1..50}; do
  if curl -fsS "http://127.0.0.1:$port/$artifact_name.sha256" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done
curl -fsS "http://127.0.0.1:$port/$artifact_name.sha256" >/dev/null || fail "fixture HTTP server did not start"

artifact_url="http://127.0.0.1:$port/$artifact_name"
sidecar_url="$artifact_url.sha256"

marker="$tmp_dir/installed-ok"
run_bootstrap_install_ps1 "$pwsh" "$artifact_name" "$artifact_url" "$sidecar_url" "$marker" "$tmp_dir/success.err"
[[ -s "$marker" ]] || fail "verified artifact did not run packaged PowerShell installer"

rm -f "$marker"
if run_bootstrap_install_ps1 "$pwsh" "$artifact_name" "$artifact_url" "http://127.0.0.1:$port/missing.sha256" "$marker" "$tmp_dir/missing.err"; then
  fail "install.ps1 succeeded with missing sha256 sidecar"
fi
[[ ! -e "$marker" ]] || fail "PowerShell payload ran despite missing sidecar"

printf '%064d  %s\n' 0 "$artifact_name" > "$sidecar"
if run_bootstrap_install_ps1 "$pwsh" "$artifact_name" "$artifact_url" "$sidecar_url" "$marker" "$tmp_dir/mismatch.err"; then
  fail "install.ps1 succeeded with mismatched sha256 sidecar"
fi
[[ ! -e "$marker" ]] || fail "PowerShell payload ran despite checksum mismatch"

echo "[OK] install.ps1 verification tests passed"
