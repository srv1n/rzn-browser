#!/bin/bash
# Harvest the top books of a Goodreads shelf (+ optional local book files) into a
# SQLite database, with deep by_rating reviews for every book. Parallel browser
# workers; resumable (re-run skips books whose reviews are already fetched).
#
#   harvest.sh --shelf parenting --pages 3 \
#              --local-dir "/path/to/downloaded/books" \
#              --db "/path/to/goodreads.db" --workers 4
#
# Output DB tables: books, reviews, local_files, runs (log). Raw workflow JSON is
# kept under <db_dir>/raw for traceability. Synthesis (opinion/ranking) is a
# separate agent step (see SKILL.md) that reads this DB.
set -uo pipefail   # NOT -e: one bad book must not abort the whole harvest

CLI="${RZN_BROWSER_CLI:-rzn-browser}"
SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"

# ---- result helpers -------------------------------------------------------
res() { sed -n '/^{/,$p' "$1" 2>/dev/null; }   # strip rzn-browser pretty-tree prefix
slug() { sed -E 's#.*/book/show/([^/?#]+).*#\1#' <<<"$1"; }

# ===========================================================================
# WORKER MODE: harvest one book. Re-invoked by xargs; reads DB/RAW from env.
# ===========================================================================
work_one() {
  local url="$1" id bres rres merged title
  id="$(slug "$url")"
  bres="$RAW/book/$id.json"; rres="$RAW/reviews/$id.json"; merged="$RAW/reviews/$id.merged.json"

  sleep "0.$((RANDOM%9))"   # tiny stagger so concurrent workers don't fire in lockstep
  "$CLI" run goodreads book    --param book_url="$url"                          2>/dev/null | res /dev/stdin > "$bres"
  sleep "$(( 2 + RANDOM%4 ))"
  "$CLI" run goodreads reviews --param book_url="$url" --param coverage=by_rating 2>/dev/null | res /dev/stdin > "$rres"

  # pull just the workflow result object out of each run envelope (fall back to {})
  jq '.output.result // {}' "$bres" > "$bres.r" 2>/dev/null; [ -s "$bres.r" ] || echo '{}' > "$bres.r"; mv "$bres.r" "$bres"
  jq '.output.result // {}' "$rres" > "$rres.r" 2>/dev/null; [ -s "$rres.r" ] || echo '{}' > "$rres.r"; mv "$rres.r" "$rres"

  title="$(jq -r '.book.title // ""' "$bres" 2>/dev/null)"; title="${title//\'/}"

  # merge first-page + by_rating reviews, drop empties, dedupe
  jq -s '((.[0].reviews // []) + (.[1].reviews // []))
         | map(select(.body and ((.body|length)>0)))
         | unique_by(.review_url // (.body[0:80]))' "$bres" "$rres" > "$merged" 2>/dev/null
  [ -s "$merged" ] || echo '[]' > "$merged"
  local nrev st; nrev="$(jq 'length' "$merged" 2>/dev/null || echo 0)"
  st="ok"; [ "${nrev:-0}" -gt 0 ] || st="empty"

  sqlite3 -cmd ".timeout 20000" "$DB" "
UPDATE books SET
  title           = COALESCE(NULLIF(json_extract(readfile('$bres'),'\$.book.title'),''), title),
  author          = COALESCE(NULLIF(json_extract(readfile('$bres'),'\$.book.author'),''), author),
  avg_rating      = COALESCE(NULLIF(json_extract(readfile('$bres'),'\$.book.avg_rating'),''), avg_rating),
  ratings_count   = COALESCE(NULLIF(json_extract(readfile('$bres'),'\$.book.ratings_count'),''), ratings_count),
  reviews_count   = json_extract(readfile('$bres'),'\$.book.reviews_count'),
  pages           = json_extract(readfile('$bres'),'\$.book.pages'),
  format          = json_extract(readfile('$bres'),'\$.book.format'),
  isbn            = json_extract(readfile('$bres'),'\$.book.isbn'),
  language        = json_extract(readfile('$bres'),'\$.book.language'),
  first_published = json_extract(readfile('$bres'),'\$.book.first_published'),
  description     = json_extract(readfile('$bres'),'\$.book.description'),
  cover_image     = json_extract(readfile('$bres'),'\$.book.cover_image'),
  genres          = json_extract(readfile('$bres'),'\$.genres'),
  histogram       = json_extract(readfile('$bres'),'\$.rating_breakdown'),
  similar_urls    = json_extract(readfile('$bres'),'\$.similar_urls'),
  reviews_fetched = CASE WHEN (length('$title')>0 OR $nrev>0) THEN 1 ELSE 0 END,
  review_n        = $nrev,
  fetched_at      = datetime('now')
WHERE book_id='$id';
DELETE FROM reviews WHERE book_id='$id';
INSERT INTO reviews(book_id, book_url, rating, review_author, review_date, likes, body, review_url)
 SELECT '$id', '$url',
        json_extract(value,'\$.rating'),  json_extract(value,'\$.author'),
        json_extract(value,'\$.date'),    json_extract(value,'\$.likes'),
        json_extract(value,'\$.body'),    json_extract(value,'\$.review_url')
 FROM json_each(readfile('$merged'));
INSERT INTO runs(ts,phase,book_id,status,detail)
 VALUES (datetime('now'),'harvest','$id','$st','$nrev reviews | '||replace('$title','''',''));
" 2>>"$RAW/sqlite.err"

  printf '[harvest] %-22s reviews=%-3s %s\n' "$id" "$nrev" "${title:0:48}"
  sleep "$(( ${DELAY:-5} + RANDOM%4 ))"   # pace between books (be gentle on Goodreads)
}

if [ "${1:-}" = "__work" ]; then
  work_one "$2"
  exit 0
fi

# ===========================================================================
# MAIN MODE
# ===========================================================================
usage() {
  cat <<'EOF'
Usage:
  harvest.sh --shelf <slug> [--pages N] [--local-dir DIR] [--db PATH] [--workers N] [--out DIR]

  --shelf      Goodreads shelf slug (parenting, self-help, ...). Required unless --local-dir only.
  --pages      Shelf pages to collect, 50 books/page (default 3 = top 150).
  --local-dir  Also resolve + harvest the book files in this directory.
  --db         SQLite database path (default <out>/goodreads.db).
  --out        Workspace dir for db + raw JSON + logs (default ./goodreads-research-out/<shelf>).
  --workers    Parallel browser workers (default 2). Keep this low (2-3): Goodreads
               throttles / shows CAPTCHAs under heavy concurrent load.
  --delay      Base seconds each worker sleeps between books, plus 0-3s jitter
               (default 5). Raise it to be gentler.
  --skip-shelf Skip shelf discovery (only refresh/harvest what is already in the DB + local).
EOF
}

SHELF=""; PAGES=3; LOCAL_DIR=""; DB=""; OUT=""; WORKERS=2; DELAY=5; SKIP_SHELF=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --shelf) SHELF="${2:-}"; shift 2;;
    --pages) PAGES="${2:-}"; shift 2;;
    --local-dir) LOCAL_DIR="${2:-}"; shift 2;;
    --db) DB="${2:-}"; shift 2;;
    --out) OUT="${2:-}"; shift 2;;
    --workers) WORKERS="${2:-}"; shift 2;;
    --delay) DELAY="${2:-}"; shift 2;;
    --skip-shelf) SKIP_SHELF=1; shift;;
    -h|--help) usage; exit 0;;
    *) echo "Unknown option: $1" >&2; usage; exit 1;;
  esac
done

command -v jq >/dev/null      || { echo "jq required" >&2; exit 1; }
command -v sqlite3 >/dev/null  || { echo "sqlite3 required" >&2; exit 1; }
[ -z "$SHELF" ] && [ -z "$LOCAL_DIR" ] && { echo "Need --shelf or --local-dir" >&2; usage; exit 1; }

[ -z "$OUT" ] && OUT="$(pwd)/goodreads-research-out/${SHELF:-local}"
[ -z "$DB" ] && DB="$OUT/goodreads.db"
RAW="$OUT/raw"
mkdir -p "$RAW/book" "$RAW/reviews" "$OUT/log"
LOG="$OUT/log/harvest.log"
export CLI RAW DB SELF DELAY
log() { echo "$(date +%H:%M:%S) $*" | tee -a "$LOG"; }

# ---- schema ---------------------------------------------------------------
sqlite3 "$DB" <<'SQL'
PRAGMA journal_mode=WAL;
CREATE TABLE IF NOT EXISTS books(
  book_id TEXT PRIMARY KEY, book_url TEXT, title TEXT, author TEXT,
  avg_rating TEXT, ratings_count TEXT, published TEXT, first_published TEXT,
  reviews_count TEXT, pages TEXT, format TEXT, isbn TEXT, language TEXT,
  description TEXT, cover_image TEXT, genres TEXT, histogram TEXT, similar_urls TEXT,
  shelf TEXT, shelf_rank INTEGER, source TEXT DEFAULT 'shelf', local_file TEXT,
  reviews_fetched INTEGER DEFAULT 0, review_n INTEGER DEFAULT 0, fetched_at TEXT);
CREATE TABLE IF NOT EXISTS reviews(
  id INTEGER PRIMARY KEY AUTOINCREMENT, book_id TEXT, book_url TEXT,
  rating TEXT, review_author TEXT, review_date TEXT, likes TEXT, body TEXT, review_url TEXT);
CREATE TABLE IF NOT EXISTS local_files(
  filename TEXT, query TEXT, resolved_book_id TEXT, resolved_title TEXT, bytes INTEGER);
CREATE TABLE IF NOT EXISTS runs(
  ts TEXT, phase TEXT, book_id TEXT, status TEXT, detail TEXT);
CREATE INDEX IF NOT EXISTS idx_reviews_book ON reviews(book_id);
CREATE INDEX IF NOT EXISTS idx_books_src ON books(source);
SQL
log "DB ready: $DB (workers=$WORKERS)"

# ---- stage 1: shelf discovery --------------------------------------------
if [ -n "$SHELF" ] && [ "$SKIP_SHELF" -eq 0 ]; then
  for p in $(seq 1 "$PAGES"); do
    pf="$RAW/shelf_p$p.json"
    "$CLI" run goodreads shelf --param shelf="$SHELF" --param max_pages=1 --param start_page="$p" 2>/dev/null | res /dev/stdin > "$pf"
    base=$(( (p-1)*50 ))
    jq --argjson base "$base" --arg shelf "$SHELF" '
      (.output.result.books // [])
      | to_entries
      | map(.value + {
          book_id:    (.value.book_url | capture("/book/show/(?<s>[^/?#]+)").s),
          shelf_rank: ($base + .key + 1),
          shelf:      $shelf })' "$pf" > "$pf.aug" 2>/dev/null
    n=$(jq 'length' "$pf.aug" 2>/dev/null || echo 0)
    sqlite3 -cmd ".timeout 20000" "$DB" "
INSERT INTO books(book_id,book_url,title,author,avg_rating,ratings_count,published,shelf,shelf_rank,source)
 SELECT json_extract(value,'\$.book_id'), json_extract(value,'\$.book_url'),
        json_extract(value,'\$.title'),   json_extract(value,'\$.author'),
        json_extract(value,'\$.avg_rating'), json_extract(value,'\$.ratings_count'),
        json_extract(value,'\$.published'),  json_extract(value,'\$.shelf'),
        json_extract(value,'\$.shelf_rank'), 'shelf'
 FROM json_each(readfile('$pf.aug'))
 WHERE true
 ON CONFLICT(book_id) DO UPDATE SET
   shelf_rank=excluded.shelf_rank, shelf=excluded.shelf,
   source=CASE WHEN books.source='local' THEN 'both' ELSE books.source END;"
    log "shelf $SHELF page $p: +$n books"
    sleep "$(( 2 + RANDOM%2 ))"
  done
fi

# ---- stage 2: resolve local files ----------------------------------------
if [ -n "$LOCAL_DIR" ]; then
  log "resolving local files in $LOCAL_DIR"
  # one search per distinct cleaned query; record every matching filename
  declare -A seen_q
  while IFS= read -r f; do
    [ -z "$f" ] && continue
    fname="$(basename "$f")"; fname="${fname//\'/}"
    bytes="$(stat -f%z "$f" 2>/dev/null || echo 0)"
    q="$(printf '%s' "$fname" \
        | sed -E 's/\.(pdf|epub|mobi|azw3)(\.part)?$//I' \
        | sed -E 's/libgen[^ ]*//Ig' \
        | sed -E 's/\([^)]*\)//g; s/\{[^}]*\}//g; s/\[[^]]*\]//g' \
        | sed -E 's/[_]+/ /g; s/[—–]/ /g' \
        | sed -E 's/[[:space:]]*-[[:space:]]*$//' \
        | sed -E 's/  +/ /g; s/^[[:space:]]+|[[:space:]]+$//g')"
    q="${q//\'/}"; q="${q// - / }"   # flatten author/title dash
    # cap to first 12 words: long author+title+subtitle queries return nothing or rank summaries first
    q="$(printf '%s' "$q" | awk '{n=(NF>12?12:NF); for(i=1;i<=n;i++) printf (i>1?" ":"")$i}')"
    [ -z "$q" ] && continue
    key="$(echo "$q" | tr 'A-Z' 'a-z')"
    if [ -n "${seen_q[$key]:-}" ]; then
      # extra file for an already-resolved book: just log filename
      sqlite3 "$DB" "INSERT INTO local_files(filename,query,bytes) VALUES (replace('$fname','''',''),replace('$q','''',''),$bytes);"
      continue
    fi
    seen_q[$key]=1
    sf="$RAW/search_$(echo "$key" | tr -c 'a-z0-9' '_' | cut -c1-40).json"
    "$CLI" run goodreads search --param search_query="$q" 2>/dev/null | res /dev/stdin > "$sf"
    sleep "$(( 3 + RANDOM%3 ))"
    # pick the canonical book: drop summary/study-guide knockoffs, then prefer the
    # most-rated match (the real book dwarfs summaries in ratings_count).
    pick="$(jq -r '
      ([.output.result.results[]?
         | select(((.title//"")|ascii_downcase)
             | test("summary|analysis|workbook|study guide|guide to|joosr|conversation starters|key takeaways|quicklet|sidekick|cliffsnotes|instaread|sparknotes";"i") | not)]
      ) as $clean
      | (if ($clean|length)>0 then $clean else (.output.result.results // []) end)
      | sort_by(((.ratings_count//"0")|tostring|gsub(",";"")|(tonumber? // 0)))
      | reverse | (.[0] // {})
      | ((.book_url // "")+"\t"+(.title // ""))' "$sf" 2>/dev/null)"
    rurl="${pick%%$'\t'*}"
    rtitle="${pick#*$'\t'}"; rtitle="${rtitle//\'/}"
    rid=""; [ -n "$rurl" ] && rid="$(slug "$rurl")"
    sqlite3 "$DB" "INSERT INTO local_files(filename,query,resolved_book_id,resolved_title,bytes)
      VALUES (replace('$fname','''',''),replace('$q','''',''),'$rid',replace('$rtitle','''',''),$bytes);"
    if [ -n "$rid" ]; then
      sqlite3 -cmd ".timeout 20000" "$DB" "
INSERT INTO books(book_id,book_url,title,source,local_file)
 VALUES ('$rid','$rurl',replace('$rtitle','''',''),'local',replace('$fname','''',''))
 ON CONFLICT(book_id) DO UPDATE SET
   local_file=replace('$fname','''',''),
   source=CASE WHEN books.source='shelf' THEN 'both' ELSE 'local' END;"
      log "local: \"$q\" -> $rid ($rtitle)"
    else
      log "local: \"$q\" -> NO MATCH"
    fi
  done < <(find "$LOCAL_DIR" -maxdepth 1 -type f \( -iname '*.pdf' -o -iname '*.epub' -o -iname '*.mobi' -o -iname '*.azw3' -o -iname '*.part' \) | sort)
fi

# ---- stage 3: parallel harvest -------------------------------------------
WL="$OUT/worklist.txt"
sqlite3 "$DB" "SELECT book_url FROM books WHERE reviews_fetched=0 AND book_url IS NOT NULL ORDER BY shelf_rank;" > "$WL"
TOTAL=$(wc -l < "$WL" | tr -d ' ')
log "harvesting $TOTAL books with $WORKERS workers"
if [ "$TOTAL" -gt 0 ]; then
  xargs -P "$WORKERS" -I@ "$SELF" __work "@" < "$WL"
fi

# ---- summary --------------------------------------------------------------
log "DONE"
sqlite3 -box "$DB" "SELECT
  (SELECT COUNT(*) FROM books) AS books,
  (SELECT COUNT(*) FROM books WHERE reviews_fetched=1) AS harvested,
  (SELECT COUNT(*) FROM books WHERE source IN('local','both')) AS local_linked,
  (SELECT COUNT(*) FROM reviews) AS reviews;"
echo "DB: $DB"
echo "Next: synthesize from this DB per SKILL.md, or query directly."
