#!/usr/bin/env bash
# Cross-repo fleet smoke against the real backend and laptop supervisor.
# Operator auth uses a tenant API key seeded into the disposable SQLite DB. This
# exercises the normal AuthMiddleware API-key path; it is not an auth bypass.
set -euo pipefail

usage() {
  echo "usage: $0 [--with-browser]" >&2
  exit 2
}

WITH_BROWSER=0
case "${1:-}" in
  "") ;;
  --with-browser) WITH_BROWSER=1 ;;
  *) usage ;;
esac
[[ $# -le 1 ]] || usage

for tool in cargo curl jq python3; do
  command -v "$tool" >/dev/null || { echo "missing prerequisite: $tool" >&2; exit 1; }
done

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
BACKEND_DIR=${BACKEND_DIR:-"$REPO_ROOT/../backend"}
[[ -f "$BACKEND_DIR/Cargo.toml" ]] || { echo "backend not found: $BACKEND_DIR" >&2; exit 1; }
for task in FLT-T-000{1..8}; do
  task_file="$BACKEND_DIR/.tusker/work/tasks/$task.md"
  [[ -f "$task_file" ]] || { echo "missing backend task contract: $task" >&2; exit 1; }
  task_status=$(awk -F'"' '/^status:/{print $2; exit}' "$task_file")
  case "$task_status" in
    review|done) ;;
    *) echo "backend prerequisite $task is $task_status (need review/done)" >&2; exit 1 ;;
  esac
done

# Every mutable path is below ROOT. Supplying RZN_FLEET_SMOKE_ROOT lets Tier 2
# launch Chrome/native-host against the same known app base; ROOT is still erased.
ROOT=${RZN_FLEET_SMOKE_ROOT:-$(mktemp -d "${TMPDIR:-/tmp}/rzn-fleet-smoke.XXXXXX")}
mkdir -p "$ROOT/home" "$ROOT/tmp"
APP_BASE="$ROOT/app"
DB_PATH="$ROOT/fleet-smoke.db"
CONFIG_PATH="$ROOT/fleet_config.json"
BACKEND_LOG="$ROOT/backend.log"
SUPERVISOR_LOG="$ROOT/supervisor.log"
PORT=${PORT:-$((RANDOM + 20000))}
BASE_URL="http://127.0.0.1:$PORT"
BACKEND_PID=""
SUPERVISOR_PID=""

cleanup() {
  local rc=$?
  trap - EXIT INT TERM
  if [[ $rc -ne 0 ]]; then
    echo "fleet smoke failed; backend tail:" >&2
    tail -20 "$BACKEND_LOG" 2>/dev/null >&2 || true
    echo "fleet smoke failed; supervisor tail:" >&2
    tail -20 "$SUPERVISOR_LOG" 2>/dev/null >&2 || true
  fi
  [[ -n "$SUPERVISOR_PID" ]] && kill "$SUPERVISOR_PID" 2>/dev/null || true
  [[ -n "$BACKEND_PID" ]] && kill "$BACKEND_PID" 2>/dev/null || true
  [[ -n "$SUPERVISOR_PID" ]] && wait "$SUPERVISOR_PID" 2>/dev/null || true
  [[ -n "$BACKEND_PID" ]] && wait "$BACKEND_PID" 2>/dev/null || true
  rm -rf "$ROOT"
  exit "$rc"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Refuse a caller-selected or unlucky random port that is already occupied.
python3 - "$PORT" <<'PY'
import socket, sys
s = socket.socket()
try:
    s.bind(("127.0.0.1", int(sys.argv[1])))
finally:
    s.close()
PY

echo "[build] laptop release binaries"
(cd "$REPO_ROOT" && cargo build --release -p rzn-browser -p rzn-native-host)
echo "[build] backend with browser-fleet"
(cd "$BACKEND_DIR" && RZN_CARGO_LOCK_POLL_SECONDS=${RZN_CARGO_LOCK_POLL_SECONDS:-0.25} \
  scripts/cargo_shared.sh build --features browser-fleet)

# Resolve binaries even when the caller supplies a relative CARGO_TARGET_DIR.
TARGET_DIR=${CARGO_TARGET_DIR:-target}
[[ "$TARGET_DIR" = /* ]] || TARGET_DIR="$REPO_ROOT/$TARGET_DIR"
BROWSER_BIN="$TARGET_DIR/release/rzn-browser"
BACKEND_TARGET_DIR=${RZN_CARGO_TARGET_DIR:-"$BACKEND_DIR/target/backend"}
[[ "$BACKEND_TARGET_DIR" = /* ]] || BACKEND_TARGET_DIR="$BACKEND_DIR/$BACKEND_TARGET_DIR"
BACKEND_BIN="$BACKEND_TARGET_DIR/debug/rznbackend"

export RZN_SUPERVISOR_APP_BASE="$APP_BASE"
export RZN_APP_BASE_DIR="$APP_BASE"
export RZN_RUNTIME_DIR="$ROOT/runtime"
export RZN_FLEET_CONFIG_PATH="$CONFIG_PATH"
export RZN_FLEET_POLL_INTERVAL_MS=200
export RZN_FLEET_DISABLE_JITTER=1

echo "[backend] start $BASE_URL"
(cd "$ROOT" && exec env HOME="$ROOT/home" TMPDIR="$ROOT/tmp" DATABASE_URL="sqlite:$DB_PATH" \
  ENVIRONMENT=development MIGRATE_ON_BOOT=true PORT="$PORT" "$BACKEND_BIN") \
  >"$BACKEND_LOG" 2>&1 &
BACKEND_PID=$!
for _ in {1..180}; do
  curl -fsS "$BASE_URL/livez" >/dev/null 2>&1 && break
  kill -0 "$BACKEND_PID" 2>/dev/null || { echo "backend exited during startup" >&2; exit 1; }
  sleep 1
done
curl -fsS "$BASE_URL/livez" >/dev/null || { echo "backend /livez timed out" >&2; exit 1; }

# Seed a normal tenant API key into only the disposable database. The backend
# verifies its prefix + base64 SHA-256 through the production middleware.
OPERATOR_KEY="fleet-smoke-${RANDOM}-${RANDOM}-${RANDOM}"
python3 - "$DB_PATH" "$OPERATOR_KEY" <<'PY'
import base64, hashlib, sqlite3, sys
db, key = sys.argv[1:]
con = sqlite3.connect(db, timeout=10)
con.execute("INSERT INTO api_keys (id,tenant_id,name,key_hash,key_prefix,scopes,created_by) VALUES (?,?,?,?,?,?,?)",
            ("fleet-smoke-key", "platform", "Fleet smoke", base64.b64encode(hashlib.sha256(key.encode()).digest()).decode(), key[:8], '["*"]', "fleet-smoke"))
con.commit()
PY

api() {
  curl --silent --show-error --fail-with-body \
    -H "x-rzn-api-key: $OPERATOR_KEY" -H "content-type: application/json" "$@"
}

echo "[fleet] mint code and enroll isolated device"
CODE=$(api -X POST "$BASE_URL/v1/fleet/enrollment-codes" -d '{"ttl_seconds":600}' | jq -er .code)
"$BROWSER_BIN" fleet enroll --server "$BASE_URL" --code "$CODE" --name smoke-device >/dev/null
DEVICE_ID=$(jq -er .device_id "$CONFIG_PATH")

# Enrollment must precede startup: the supervisor loads fleet_config.json once.
(cd "$ROOT" && exec env HOME="$ROOT/home" TMPDIR="$ROOT/tmp" \
  "$BROWSER_BIN" supervisor serve --app-base "$APP_BASE") >"$SUPERVISOR_LOG" 2>&1 &
SUPERVISOR_PID=$!
for _ in {1..40}; do
  "$BROWSER_BIN" supervisor status --app-base "$APP_BASE" --json >/dev/null 2>&1 && break
  kill -0 "$SUPERVISOR_PID" 2>/dev/null || { echo "supervisor exited during startup" >&2; exit 1; }
  sleep 0.25
done
"$BROWSER_BIN" supervisor status --app-base "$APP_BASE" --json >/dev/null

if [[ $WITH_BROWSER -eq 1 ]]; then
  MANIFEST="$REPO_ROOT/workflows/google/google-search.json"
  PARAMS='{"search_query":"rzn fleet laptop smoke"}'
  EXPECTED=succeeded
else
  MANIFEST="$REPO_ROOT/workflows/_smoke/fleet-tier1.json"
  PARAMS='{}'
  EXPECTED=failed
fi
WORKFLOW_ID=$(jq -er .id "$MANIFEST")
echo "[fleet] publish $WORKFLOW_ID and submit job"
api -X POST "$BASE_URL/v1/fleet/workflows" --data-binary "@$MANIFEST" >/dev/null
JOB_BODY=$(jq -cn --arg id "$WORKFLOW_ID" --argjson params "$PARAMS" \
  '{workflow_id:$id,params:$params,target:{kind:"any"},execution_deadline_seconds:120}')
JOB_ID=$(api -X POST "$BASE_URL/v1/fleet/jobs" -d "$JOB_BODY" | jq -er .id)

JOB_JSON=""
STATUS=""
DEADLINE=$((SECONDS + 180))
while (( SECONDS < DEADLINE )); do
  JOB_JSON=$(api "$BASE_URL/v1/fleet/jobs/$JOB_ID")
  STATUS=$(jq -r .status <<<"$JOB_JSON")
  case "$STATUS" in succeeded|failed|cancelled|timed_out|lost|expired) break ;; esac
  sleep 0.5
done
[[ "$STATUS" == "$EXPECTED" ]] || { echo "expected $EXPECTED, got ${STATUS:-timeout}" >&2; exit 1; }
jq -e --arg expected "$EXPECTED" '.result.status == $expected and .result.result != null' \
  <<<"$JOB_JSON" >/dev/null
if [[ $WITH_BROWSER -eq 1 ]]; then
  jq -e '.result.result.output as $o | $o != null and $o != "" and $o != [] and $o != {}' \
    <<<"$JOB_JSON" >/dev/null
fi

# Heartbeat proof plus the laptop-local journal/status observation requested by
# the contract. Allow a short race between backend commit and journal "posted".
NOW_MS=$(python3 -c 'import time; print(int(time.time()*1000))')
DEVICES=$(api "$BASE_URL/v1/fleet/devices")
jq -e --arg id "$DEVICE_ID" --argjson cutoff "$((NOW_MS - 30000))" \
  'any(.[]; .id == $id and .last_seen_at != null and .last_seen_at >= $cutoff)' \
  <<<"$DEVICES" >/dev/null
for _ in {1..40}; do
  LOCAL_STATUS=$("$BROWSER_BIN" fleet status --json)
  jq -e --arg id "$JOB_ID" '.server_reachable and any(.loop_state.journal_tail[]?; .job_id == $id and .state == "posted")' \
    <<<"$LOCAL_STATUS" >/dev/null && break
  sleep 0.25
done
jq -e --arg id "$JOB_ID" '.server_reachable and any(.loop_state.journal_tail[]?; .job_id == $id and .state == "posted")' \
  <<<"$LOCAL_STATUS" >/dev/null

echo "[pass] job=$JOB_ID status=$STATUS device=$DEVICE_ID heartbeat=recent journal=posted"
jq '{id,status,result}' <<<"$JOB_JSON"
