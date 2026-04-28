#!/bin/bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  run_workflow.sh amazon_search --query "<search_query>" [--show-log]
  run_workflow.sh amazon_product --product-url "<amazon_product_url>" [--show-log]
  run_workflow.sh appstore_search --query "<app_query>" [--show-log]
  run_workflow.sh appstore_details --app-id "<app_id>" [--show-log]
  run_workflow.sh g2_search --query "<search_query>" [--show-log]
  run_workflow.sh g2_product --product-url "<g2_product_url>" [--show-log]
  run_workflow.sh capterra_search --query "<search_query>" [--show-log]
  run_workflow.sh capterra_product --product-url "<capterra_product_url>" [--show-log]
  run_workflow.sh etsy_search --query "<search_query>" [--show-log]
  run_workflow.sh etsy_listing --listing-url "<etsy_listing_url>" [--show-log]
  run_workflow.sh apple_ads_keyword_recs --adam-id "<adam_id>" --adgroup-id "<adgroup_id>" --query "<keyword_seed>" [--storefront "us"] [--show-log]
  run_workflow.sh apple_ads_keyword_suggest --organization-id "<org_id>" --campaign-id "<campaign_id>" --adgroup-id "<adgroup_id>" --adam-id "<adam_id>" --query "<keyword_seed>" [--show-log]
  run_workflow.sh apple_ads_portal_report --report-type "<report_type>" --start-date "<YYYY-MM-DD>" --end-date "<YYYY-MM-DD>" [--organization-id "<org_id>"] [--campaign-id "<campaign_id>"] [--show-log]
  run_workflow.sh appstore_search_snapshot --term "<search_term>" [--country "us"] [--show-log]

Commands:
  amazon_search           Run workflows/amazon/amazon-search.json
  amazon_product          Run workflows/amazon/amazon-product-key-facts-reviews.json
  appstore_search         Run workflows/appstore/appstore-search.json
  appstore_details        Run workflows/appstore/appstore-app-details.json
  g2_search               Run workflows/g2/g2-search.json
  g2_product              Run workflows/g2/g2-product-details-reviews.json
  capterra_search         Run workflows/capterra/capterra-search.json
  capterra_product        Run workflows/capterra/capterra-product-details-reviews.json
  etsy_search             Run workflows/etsy/etsy-search.json
  etsy_listing            Run workflows/etsy/etsy-listing-details-reviews.json
  apple_ads_keyword_recs  Run workflows/generated/aso/apple-ads-keyword-recommendations.json
  apple_ads_keyword_suggest Run workflows/generated/aso/apple-ads-keyword-suggest-same-origin.json
  apple_ads_portal_report Run workflows/generated/aso/apple-ads-portal-report.json
  appstore_search_snapshot Run workflows/generated/aso/appstore-search-snapshot.json

Common options:
  --query <value>
  --product-url <value>
  --app-id <value>
  --listing-url <value>
  --adam-id <value>
  --adgroup-id <value>
  --storefront <value>
  --report-type <value>
  --start-date <value>
  --end-date <value>
  --organization-id <value>
  --campaign-id <value>
  --term <value>
  --country <value>
  --show-log               Print raw run log to stderr
USAGE
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

require_value() {
  local label="$1"
  local value="$2"
  if [ -z "$value" ]; then
    echo "Missing required value: $label" >&2
    usage
    exit 1
  fi
}

if [ "${1:-}" = "" ] || [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
  usage
  exit 0
fi

require_cmd jq

COMMAND="$1"
shift

SHOW_LOG=0
WORKFLOW=""

QUERY_VALUE=""
PRODUCT_URL=""
APP_ID=""
LISTING_URL=""
ADAM_ID=""
ADGROUP_ID=""
STOREFRONT=""
REPORT_TYPE=""
START_DATE=""
END_DATE=""
ORGANIZATION_ID=""
CAMPAIGN_ID=""
TERM_VALUE=""
COUNTRY=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --query)
      QUERY_VALUE="${2:-}"
      shift 2
      ;;
    --product-url)
      PRODUCT_URL="${2:-}"
      shift 2
      ;;
    --app-id)
      APP_ID="${2:-}"
      shift 2
      ;;
    --listing-url)
      LISTING_URL="${2:-}"
      shift 2
      ;;
    --adam-id)
      ADAM_ID="${2:-}"
      shift 2
      ;;
    --adgroup-id)
      ADGROUP_ID="${2:-}"
      shift 2
      ;;
    --storefront)
      STOREFRONT="${2:-}"
      shift 2
      ;;
    --report-type)
      REPORT_TYPE="${2:-}"
      shift 2
      ;;
    --start-date)
      START_DATE="${2:-}"
      shift 2
      ;;
    --end-date)
      END_DATE="${2:-}"
      shift 2
      ;;
    --organization-id)
      ORGANIZATION_ID="${2:-}"
      shift 2
      ;;
    --campaign-id)
      CAMPAIGN_ID="${2:-}"
      shift 2
      ;;
    --term)
      TERM_VALUE="${2:-}"
      shift 2
      ;;
    --country)
      COUNTRY="${2:-}"
      shift 2
      ;;
    --show-log)
      SHOW_LOG=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage
      exit 1
      ;;
  esac
done

PARAM_KEYS=()
PARAM_VALUES=()

add_param() {
  PARAM_KEYS+=("$1")
  PARAM_VALUES+=("$2")
}

case "$COMMAND" in
  amazon_search)
    WORKFLOW="workflows/amazon/amazon-search.json"
    require_value "--query" "$QUERY_VALUE"
    add_param "search_query" "$QUERY_VALUE"
    ;;
  amazon_product)
    WORKFLOW="workflows/amazon/amazon-product-key-facts-reviews.json"
    require_value "--product-url" "$PRODUCT_URL"
    add_param "product_url" "$PRODUCT_URL"
    ;;
  appstore_search)
    WORKFLOW="workflows/appstore/appstore-search.json"
    require_value "--query" "$QUERY_VALUE"
    add_param "app_query" "$QUERY_VALUE"
    ;;
  appstore_details)
    WORKFLOW="workflows/appstore/appstore-app-details.json"
    require_value "--app-id" "$APP_ID"
    add_param "app_id" "$APP_ID"
    ;;
  g2_search)
    WORKFLOW="workflows/g2/g2-search.json"
    require_value "--query" "$QUERY_VALUE"
    add_param "search_query" "$QUERY_VALUE"
    ;;
  g2_product)
    WORKFLOW="workflows/g2/g2-product-details-reviews.json"
    require_value "--product-url" "$PRODUCT_URL"
    add_param "product_url" "$PRODUCT_URL"
    ;;
  capterra_search)
    WORKFLOW="workflows/capterra/capterra-search.json"
    require_value "--query" "$QUERY_VALUE"
    add_param "search_query" "$QUERY_VALUE"
    ;;
  capterra_product)
    WORKFLOW="workflows/capterra/capterra-product-details-reviews.json"
    require_value "--product-url" "$PRODUCT_URL"
    add_param "product_url" "$PRODUCT_URL"
    ;;
  etsy_search)
    WORKFLOW="workflows/etsy/etsy-search.json"
    require_value "--query" "$QUERY_VALUE"
    add_param "search_query" "$QUERY_VALUE"
    ;;
  etsy_listing)
    WORKFLOW="workflows/etsy/etsy-listing-details-reviews.json"
    require_value "--listing-url" "$LISTING_URL"
    add_param "listing_url" "$LISTING_URL"
    ;;
  apple_ads_keyword_recs)
    WORKFLOW="workflows/generated/aso/apple-ads-keyword-recommendations.json"
    require_value "--adam-id" "$ADAM_ID"
    require_value "--adgroup-id" "$ADGROUP_ID"
    require_value "--query" "$QUERY_VALUE"
    if [ -z "$STOREFRONT" ]; then
      STOREFRONT="us"
    fi
    add_param "adam_id" "$ADAM_ID"
    add_param "adgroup_id" "$ADGROUP_ID"
    add_param "query" "$QUERY_VALUE"
    add_param "storefront" "$STOREFRONT"
    ;;
  apple_ads_keyword_suggest)
    WORKFLOW="workflows/generated/aso/apple-ads-keyword-suggest-same-origin.json"
    require_value "--organization-id" "$ORGANIZATION_ID"
    require_value "--campaign-id" "$CAMPAIGN_ID"
    require_value "--adgroup-id" "$ADGROUP_ID"
    require_value "--adam-id" "$ADAM_ID"
    require_value "--query" "$QUERY_VALUE"
    add_param "org_id" "$ORGANIZATION_ID"
    add_param "campaign_id" "$CAMPAIGN_ID"
    add_param "adgroup_id" "$ADGROUP_ID"
    add_param "adam_id" "$ADAM_ID"
    add_param "query" "$QUERY_VALUE"
    ;;
  apple_ads_portal_report)
    WORKFLOW="workflows/generated/aso/apple-ads-portal-report.json"
    require_value "--report-type" "$REPORT_TYPE"
    require_value "--start-date" "$START_DATE"
    require_value "--end-date" "$END_DATE"
    add_param "report_type" "$REPORT_TYPE"
    add_param "start_date" "$START_DATE"
    add_param "end_date" "$END_DATE"
    if [ -n "$ORGANIZATION_ID" ]; then
      add_param "organization_id" "$ORGANIZATION_ID"
    fi
    if [ -n "$CAMPAIGN_ID" ]; then
      add_param "campaign_id" "$CAMPAIGN_ID"
    fi
    ;;
  appstore_search_snapshot)
    WORKFLOW="workflows/generated/aso/appstore-search-snapshot.json"
    if [ -z "$TERM_VALUE" ]; then
      TERM_VALUE="$QUERY_VALUE"
    fi
    require_value "--term" "$TERM_VALUE"
    if [ -z "$COUNTRY" ]; then
      COUNTRY="us"
    fi
    add_param "term" "$TERM_VALUE"
    add_param "country" "$COUNTRY"
    ;;
  *)
    echo "Unknown command: $COMMAND" >&2
    usage
    exit 1
    ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
CLI_OVERRIDE="${RZN_BROWSER_CLI:-}"
CLI_PROFILE="${RZN_CLI_PROFILE:-debug}"

if [ -n "$CLI_OVERRIDE" ]; then
  CLI_CMD=("$CLI_OVERRIDE")
elif [ -x "$ROOT_DIR/target/$CLI_PROFILE/rzn-browser" ]; then
  CLI_CMD=("$ROOT_DIR/target/$CLI_PROFILE/rzn-browser")
else
  CLI_CMD=(cargo run -p rzn-browser --)
fi

RUN_LOG="$(mktemp "${TMPDIR:-/tmp}/rzn_skill_run.XXXXXX")"
PAYLOAD_FILE="$(mktemp "${TMPDIR:-/tmp}/rzn_skill_payload.XXXXXX")"
PARAMS_FILE="$(mktemp "${TMPDIR:-/tmp}/rzn_skill_params.XXXXXX")"
CANDIDATE_FILE="$(mktemp "${TMPDIR:-/tmp}/rzn_skill_candidate.XXXXXX")"
trap 'rm -f "$RUN_LOG" "$PAYLOAD_FILE" "$PARAMS_FILE" "$CANDIDATE_FILE"' EXIT

echo "{}" >"$PARAMS_FILE"
PARAM_ARGS=()

idx=0
while [ "$idx" -lt "${#PARAM_KEYS[@]}" ]; do
  key="${PARAM_KEYS[$idx]}"
  value="${PARAM_VALUES[$idx]}"
  PARAM_ARGS+=("--param" "${key}=${value}")
  tmp_file="$(mktemp "${TMPDIR:-/tmp}/rzn_skill_params_update.XXXXXX")"
  jq --arg k "$key" --arg v "$value" '. + {($k): $v}' "$PARAMS_FILE" >"$tmp_file"
  mv "$tmp_file" "$PARAMS_FILE"
  idx=$((idx + 1))
done

STARTED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
set +e
RZN_RESTART_NATIVE_HOST="${RZN_RESTART_NATIVE_HOST:-1}" \
"${CLI_CMD[@]}" run "$WORKFLOW" "${PARAM_ARGS[@]}" >"$RUN_LOG" 2>&1
EXIT_CODE=$?
set -e
FINISHED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

if [ "$SHOW_LOG" = "1" ]; then
  cat "$RUN_LOG" >&2
fi

echo "null" >"$PAYLOAD_FILE"
JSON_START_LINES="$(grep -nE '^[[:space:]]*[\[{][[:space:]]*$' "$RUN_LOG" | cut -d: -f1 || true)"

if [ -n "$JSON_START_LINES" ]; then
  for line in $(echo "$JSON_START_LINES" | awk '{a[NR]=$1} END { for (i=NR; i>=1; i--) print a[i] }'); do
    tail -n "+$line" "$RUN_LOG" >"$CANDIDATE_FILE"
    if jq empty "$CANDIDATE_FILE" >/dev/null 2>&1; then
      cp "$CANDIDATE_FILE" "$PAYLOAD_FILE"
      break
    fi
  done
fi

ERROR_LINE="$(grep -E "\[ERROR\]|❌|error:" "$RUN_LOG" | tail -n 1 || true)"

jq -n \
  --arg command "$COMMAND" \
  --arg workflow "$WORKFLOW" \
  --arg started_at "$STARTED_AT" \
  --arg finished_at "$FINISHED_AT" \
  --argjson exit_code "$EXIT_CODE" \
  --arg error_line "$ERROR_LINE" \
  --slurpfile params "$PARAMS_FILE" \
  --slurpfile data "$PAYLOAD_FILE" \
  '
  def normalize_appstore_snapshot($payload):
    if ($payload | type) == "array" then
      ($payload | map(
        if (type == "object") then
          . + {
            app_id: (
              (
                (.app_url // .app_id // "")
                | capture("id(?<id>[0-9]+)").id?
              ) // .app_id
            )
          }
        else
          .
        end
      ))
    else
      $payload
    end;

  def row_count_of($x):
    if ($x | type) == "array" then
      ($x | length)
    elif ($x | type) == "object" then
      if (($x.rows // null) | type) == "array" then ($x.rows | length)
      elif (($x.recommendations // null) | type) == "array" then ($x.recommendations | length)
      elif (($x.results // null) | type) == "array" then ($x.results | length)
      elif (($x.data // null) | type) == "array" then ($x.data | length)
      elif (($x.result // null) | type) == "array" then ($x.result | length)
      elif (($x.result // null) | type) == "object" then
        if (($x.result.rows // null) | type) == "array" then ($x.result.rows | length)
        elif (($x.result.recommendations // null) | type) == "array" then ($x.result.recommendations | length)
        elif (($x.result.data // null) | type) == "array" then ($x.result.data | length)
        else (([$x.result | to_entries[] | select(.value | type == "array") | (.value | length)] | first) // 0) end
      else (([$x | to_entries[] | select(.value | type == "array") | (.value | length)] | first) // 0) end
    else
      0
    end;
  def payload_failed($x):
    if ($x | type) != "object" then
      false
    elif (($x.success // true) == false) then
      true
    elif (($x.error_code // null) != null) then
      true
    elif (($x.status // "") == "error") then
      true
    else
      false
    end;

  def payload_error_text($x):
    if ($x | type) != "object" then
      null
    else
      ($x.error_msg // ($x.error | tostring)? // ($x.error_code | tostring)? // null)
    end;
  ($data[0]
    | if $command == "appstore_search_snapshot" then normalize_appstore_snapshot(.) else . end
  ) as $payload
  | (payload_failed($payload)) as $payload_failed
  | {
    command: $command,
    workflow: $workflow,
    params: $params[0],
    success: (($exit_code == 0) and ($payload_failed | not)),
    exit_code: $exit_code,
    started_at: $started_at,
    finished_at: $finished_at,
    error: (
      if $exit_code != 0 then
        (if $error_line == "" then null else $error_line end)
      elif $payload_failed then
        payload_error_text($payload)
      else
        null
      end
    ),
    row_count: row_count_of($payload),
    data: $payload
  }'

if [ "$EXIT_CODE" -ne 0 ]; then
  exit "$EXIT_CODE"
fi
