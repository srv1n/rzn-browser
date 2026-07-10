#!/bin/bash
# Collect the raw data for a topic-to-opinion Goodreads study:
#   shelf top-N  ->  per-book details + by_rating reviews  ->  critical-weighted dataset.
# Synthesis (opinion + CSV) is done by the agent reading dataset.txt (see SKILL.md).
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  collect.sh --shelf "<slug>" [--top N] [--out DIR]

  --shelf   Goodreads shelf/genre slug (e.g. parenting, self-help, philosophy). Required.
  --top     Number of top books to study (default 6).
  --out     Output directory (default ./goodreads-research-out/<shelf>).

Produces, under --out:
  shelf.json, book_<i>.json, rev_<i>.json   raw workflow output
  urls.txt                                  the studied book URLs
  dataset.txt                               critical-weighted, agent-readable summary
EOF
}

SHELF=""; TOP=6; OUT=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --shelf) SHELF="${2:-}"; shift 2;;
    --top) TOP="${2:-}"; shift 2;;
    --out) OUT="${2:-}"; shift 2;;
    -h|--help) usage; exit 0;;
    *) echo "Unknown option: $1" >&2; usage; exit 1;;
  esac
done

if [ -z "$SHELF" ]; then echo "Missing --shelf" >&2; usage; exit 1; fi
command -v jq >/dev/null 2>&1 || { echo "jq required" >&2; exit 1; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
CLI="${RZN_BROWSER_CLI:-rzn-browser}"
[ -z "$OUT" ] && OUT="$ROOT_DIR/goodreads-research-out/$SHELF"
mkdir -p "$OUT"

run() { "$CLI" run "$@" 2>/dev/null; }       # prints run JSON to stdout
res() { sed -n '/^{/,$p' "$1" | jq "$2" 2>/dev/null; }   # .output.result.<...>

echo "[shelf] $SHELF (top $TOP) -> $OUT"
run goodreads shelf --param shelf="$SHELF" --param max_pages=1 > "$OUT/shelf.json"
grep -oE 'https://www\.goodreads\.com/book/show/[^"]+' "$OUT/shelf.json" | awk '!seen[$0]++' | head -n "$TOP" > "$OUT/urls.txt"
echo "[shelf] collected $(wc -l < "$OUT/urls.txt" | tr -d ' ') book urls"

i=0
while IFS= read -r u; do
  i=$((i+1))
  echo "[book $i/$TOP] $u"
  run goodreads book --param book_url="$u" > "$OUT/book_$i.json"
  echo "[reviews $i/$TOP] by_rating"
  run goodreads reviews --param book_url="$u" --param coverage=by_rating > "$OUT/rev_$i.json"
done < "$OUT/urls.txt"

# Build critical-weighted, agent-readable dataset.
DS="$OUT/dataset.txt"; : > "$DS"
j=0
while [ "$j" -lt "$i" ]; do
  j=$((j+1)); bf="$OUT/book_$j.json"; rf="$OUT/rev_$j.json"
  res "$bf" '.output.result | "### "+(.book.title)+"\nAuthor: "+(.book.author)+" | Avg: "+(.book.avg_rating)+" ("+(.book.ratings_count)+" ratings) | pub "+(.book.first_published)+"\nGenres: "+((.genres//[])|join(", "))+"\nHistogram: "+((.rating_breakdown//[])|map(.stars+"*="+.percent)|join(" "))+"\nDesc: "+((.book.description//"")[0:280])' >> "$DS"
  res "$rf" '.output.result | "Reviews seen: "+(.reviews_loaded|tostring)+" | sample: "+((.reviews_returned // (.reviews|length))|tostring)+" | total text reviews: "+(.total_count|tostring)' >> "$DS"
  echo "-- 1-2* CRITICAL --" >> "$DS"
  res "$rf" '[.output.result.reviews[]|select(.rating=="1" or .rating=="2")]|.[]|"["+.rating+"*] "+((.body)[0:520])' >> "$DS"
  echo "-- 3* MIXED --" >> "$DS"
  res "$rf" '[.output.result.reviews[]|select(.rating=="3")]|.[0:6][]|"[3*] "+((.body)[0:380])' >> "$DS"
  echo "-- TOP POSITIVE (4-5*, by likes) --" >> "$DS"
  res "$rf" '[.output.result.reviews[]|select(.rating=="4" or .rating=="5")]|sort_by(.likes|tonumber? // 0)|reverse|.[0:5][]|"["+.rating+"*] "+((.body)[0:320])' >> "$DS"
  printf '\n============\n\n' >> "$DS"
done

echo "DONE shelf=$SHELF books=$i"
echo "Dataset: $DS"
echo "Next: read dataset.txt and synthesize per SKILL.md (opinion_report.md + books.csv)."
