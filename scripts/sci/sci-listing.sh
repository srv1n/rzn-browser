#!/usr/bin/env bash
# Collect the Supreme Court judgment-date listing with sci/judgements-open + sci/judgements-by-date.
# solve-captcha.sh reads each CAPTCHA with a model CLI (SCI_SOLVER=claude|codex). After three wrong
# answers on one window the script stops and notifies.
#
#   scripts/sci/sci-listing.sh backfill            # SCI_START (2014-01-01) .. today, month windows
#   scripts/sci/sci-listing.sh weekly              # the last 14 days
#   scripts/sci/sci-listing.sh window FROM TO      # one window, YYYY-MM-DD (the Step 0 probe)
#
# Output in $SCI_LISTING_DIR (default ~/rzn-sc-listing): rows.jsonl, windows.jsonl,
# pages/<from>_<to>_<page>.html. Each mode keeps its queue on disk and resumes at the
# first unfinished window; a window already in windows.jsonl is never run again.
set -euo pipefail

OUT="${SCI_LISTING_DIR:-$HOME/rzn-sc-listing}"
START="${SCI_START:-2014-01-01}"
MAX_ROWS="${SCI_MAX_ROWS:-0}" # row count that signals a truncated window; 0 = no known cap
GAP="${SCI_GAP:-30}"          # seconds between searches
HERE="$(cd "$(dirname "$0")" && pwd)"
WF_OPEN="$HERE/../../workflows/sci/sci-judgements-open.json"
WF_SEARCH="$HERE/../../workflows/sci/sci-judgements-by-date.json"
mkdir -p "$OUT/pages"
touch "$OUT/rows.jsonl" "$OUT/windows.jsonl"

notify() {
  osascript -e "display notification \"$2\" with title \"$1\" sound name \"Glass\"" >/dev/null 2>&1 || true
  echo "[$(date +%H:%M:%S)] $1: $2" >&2
}
epoch() { TZ=UTC date -j -f "%Y-%m-%d %H:%M:%S" "$1 00:00:00" +%s; }
ymd() { TZ=UTC date -j -r "$1" +%Y-%m-%d; }
plus_days() { ymd $(($(epoch "$1") + $2 * 86400)); }
dmy() { TZ=UTC date -j -r "$(epoch "$1")" +%d-%m-%Y; }
done_already() { grep -qF "\"window\":\"$1..$2\"" "$OUT/windows.jsonl"; }
# insc: the English "YYYY INSC N" in the row. pdf_language: "en", or the regional PDF's language code.
ENRICH='. + {insc: (first(.listed_text | capture("(?<i>[0-9]{4} INSC [0-9]+) ?\\(English\\)").i) // ""),
  pdf_language: (if .pdf_url == "" then ""
    elif (.pdf_url | test("supremecourt_vernacular")) then
      (.pdf_url | capture("_(?<l>[A-Z]+)\\.pdf").l // "") as $l
      | {HIN:"hi",BEN:"bn",ASM:"as",TEL:"te",TAM:"ta",KAN:"kn",MAL:"ml",MAR:"mr",GUJ:"gu",ORI:"or",ODI:"or",PUN:"pa",PAN:"pa",URD:"ur",NEP:"ne",KHA:"kha",GAR:"grt"}[$l] // ($l | ascii_downcase)
    else "en" end)}'
SHIP="${SCI_SHIP-srv@192.168.2.226:rzn-producers/listing/rows.jsonl}" # empty = do not ship

rzn() {
  if ! rzn-browser run "$@" >"$OUT/last-run.log" 2>&1; then
    notify "SC listing stopped" "A run failed. See $OUT/last-run.log"
    exit 1
  fi
}

# Run one window, solving the CAPTCHA with a model. Returns 0 when saved, 2 when it must be split.
run_window() {
  local from=$1 to=$2 res status try answer
  res="$(mktemp)"
  for try in 1 2 3; do
    rzn "$WF_OPEN" --param from_date="$(dmy "$from")" --param to_date="$(dmy "$to")" --output-file "$res"
    jq -r '.result.image' "$res" | sed 's#^data:image/png;base64,##' | base64 -D >"$OUT/captcha.png"
    if ! answer="$("$HERE/solve-captcha.sh" "$OUT/captcha.png")"; then
      notify "SC listing stopped" "The solver failed on $from to $to."
      exit 1
    fi
    rzn "$WF_SEARCH" --param captcha_answer="$answer" --output-file "$res"
    status="$(jq -r '.result.status' "$res")"
    case "$status" in
      ok | empty) break ;;
      captcha_wrong) echo "[$(date +%H:%M:%S)] $from..$to: answer $answer rejected (try $try)" >&2; sleep "$GAP" ;;
      *) notify "SC listing stopped" "$from to $to: $(jq -r '.result.message' "$res")"; exit 1 ;;
    esac
  done
  if [[ "$status" == captcha_wrong ]]; then
    notify "SC listing stopped" "Three wrong CAPTCHA answers on $from to $to. See $OUT/captcha.png"
    exit 1
  fi
  if [[ "$(jq '(.result.rows_json | fromjson | length) == .result.row_count and (.result.pages_html | length) == .result.page_count' "$res")" != true ]]; then
    notify "SC listing stopped" "$from to $to: the result lost rows or pages in transfer."
    exit 1
  fi

  local rows complete
  rows="$(jq '.result.row_count' "$res")"
  complete="$(jq '.result.complete' "$res")"
  if [[ "$complete" != true ]] || ((MAX_ROWS > 0 && rows >= MAX_ROWS)); then
    rm -f "$res"
    return 2
  fi

  local at n i
  at="$(TZ=Asia/Kolkata date +%Y-%m-%dT%H:%M:%S+05:30)"
  n="$(jq '.result.pages_html | length' "$res")"
  for ((i = 0; i < n; i++)); do
    jq -r ".result.pages_html[$i]" "$res" >"$OUT/pages/${from}_${to}_$((i + 1)).html"
  done
  # ponytail: a crash between these two appends duplicates the window's rows on rerun; consumers dedupe on pdf_url.
  jq -c --arg w "$from..$to" --arg at "$at" '.result.rows_json | fromjson | .[]
    | {pdf_url, petitioner, respondent, case_number, diary_number, decision_date, listed_text, window: $w, captured_at: $at}
    | '"$ENRICH" \
    "$res" >>"$OUT/rows.jsonl"
  jq -c --arg w "$from..$to" --arg at "$at" \
    '{window: $w, status: .result.status, row_count: .result.row_count, page_count: .result.page_count, finished_at: $at}' \
    "$res" >>"$OUT/windows.jsonl"
  echo "[$(date +%H:%M:%S)] $from..$to: $rows rows saved" >&2
  # The producer dedupes, so ship the whole file; a failed rsync is caught up after the next window.
  if [[ -n "$SHIP" ]] && ! rsync -a "$OUT/rows.jsonl" "$SHIP" >>"$OUT/rsync.log" 2>&1; then
    echo "[$(date +%H:%M:%S)] rsync failed; see $OUT/rsync.log" >&2
  fi
  rm -f "$res"
}

# Drain a queue file of "FROM TO" lines, oldest first.
drain() {
  local q=$1 from to rc mid
  while [[ -s "$q" ]]; do
    read -r from to <"$q"
    if ! done_already "$from" "$to"; then
      rc=0
      run_window "$from" "$to" || rc=$?
      if ((rc == 2)); then
        if [[ "$from" == "$to" ]]; then
          notify "SC listing stopped" "$from may be truncated and cannot be split further."
          exit 1
        fi
        mid="$(plus_days "$from" $((($(epoch "$to") - $(epoch "$from")) / 86400 / 2)))"
        { echo "$from $mid"; echo "$(plus_days "$mid" 1) $to"; tail -n +2 "$q"; } >"$q.tmp" && mv "$q.tmp" "$q"
        echo "[$(date +%H:%M:%S)] $from..$to may be truncated; split in two" >&2
        sleep "$GAP"
        continue
      fi
      sleep "$GAP"
    fi
    tail -n +2 "$q" >"$q.tmp" && mv "$q.tmp" "$q"
  done
}

today="$(date +%Y-%m-%d)"
case "${1:-}" in
  backfill)
    q="$OUT/queue.txt"
    if [[ ! -e "$q" ]]; then
      d="$START"
      while [[ ! "$d" > "$today" ]]; do
        end="$(date -j -v+1m -v-1d -f %Y-%m-%d "$d" +%Y-%m-%d)"
        [[ "$end" > "$today" ]] && end="$today"
        echo "$d $end"
        d="$(plus_days "$end" 1)"
      done >"$q"
    fi
    ;;
  weekly)
    q="$OUT/weekly-queue.txt"
    [[ -s "$q" ]] || echo "$(plus_days "$today" -13) $today" >"$q"
    ;;
  window)
    [[ $# -eq 3 ]] || { echo "usage: $0 window FROM TO" >&2; exit 64; }
    q="$OUT/adhoc-queue.txt"
    echo "$2 $3" >"$q"
    ;;
  *)
    sed -n '2,8p' "$0" >&2
    exit 64
    ;;
esac
drain "$q"
notify "SC listing" "Queue finished."
