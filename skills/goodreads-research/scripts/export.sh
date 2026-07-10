#!/bin/bash
# Export a harvested Goodreads DB (built by harvest.sh) into analysis-ready CSVs +
# a critical-weighted dataset.txt, and print a summary. Read-only on the DB.
#
#   export.sh --db /path/to/goodreads.db [--out DIR]
#
# Writes (into --out, default the DB's directory):
#   books.csv      one row per book, with pct_low_12star derived from the histogram
#   reviews.csv    one row per collected review (rating, likes, date, body, url)
#   dataset.txt    per-book critical-weighted review digest for opinion synthesis
set -uo pipefail

DB=""; OUT=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --db) DB="${2:-}"; shift 2;;
    --out) OUT="${2:-}"; shift 2;;
    -h|--help) echo "Usage: export.sh --db PATH [--out DIR]"; exit 0;;
    *) echo "Unknown option: $1" >&2; exit 1;;
  esac
done
[ -z "$DB" ] && { echo "Missing --db" >&2; exit 1; }
[ -f "$DB" ] || { echo "No such DB: $DB" >&2; exit 1; }
command -v sqlite3 >/dev/null || { echo "sqlite3 required" >&2; exit 1; }
[ -z "$OUT" ] && OUT="$(cd "$(dirname "$DB")" && pwd)"
mkdir -p "$OUT"

# ---- books.csv ------------------------------------------------------------
sqlite3 -cmd ".timeout 10000" -csv -header "$DB" "
SELECT
  shelf_rank,
  title, author,
  COALESCE(first_published, published) AS year,
  avg_rating, ratings_count, reviews_count,
  review_n AS reviews_collected,
  (SELECT ROUND(100.0*SUM(CASE WHEN json_extract(value,'\$.stars') IN ('1','2')
                THEN CAST(replace(json_extract(value,'\$.count'),',','') AS INTEGER) ELSE 0 END)
            / NULLIF(SUM(CAST(replace(json_extract(value,'\$.count'),',','') AS INTEGER)),0),1)
     FROM json_each(books.histogram)) AS pct_low_12star,
  genres, source, local_file, book_url
FROM books
ORDER BY (shelf_rank IS NULL), shelf_rank, ratings_count+0 DESC;" > "$OUT/books.csv"

# ---- reviews.csv ----------------------------------------------------------
sqlite3 -cmd ".timeout 10000" -csv -header "$DB" "
SELECT r.book_id,
       (SELECT title FROM books b WHERE b.book_id=r.book_id) AS title,
       r.rating, r.likes, r.review_date, r.review_author, r.review_url,
       replace(replace(r.body, char(13),' '), char(10),' ') AS body
FROM reviews r
ORDER BY r.book_id, CAST(r.rating AS INTEGER), CAST(r.likes AS INTEGER) DESC;" > "$OUT/reviews.csv"

# ---- dataset.txt (critical-weighted, for opinion synthesis) ---------------
DS="$OUT/dataset.txt"; : > "$DS"
sqlite3 -cmd ".timeout 10000" "$DB" "SELECT book_id FROM books WHERE reviews_fetched=1 AND review_n>0 ORDER BY (shelf_rank IS NULL), shelf_rank;" | while IFS= read -r bid; do
  [ -z "$bid" ] && continue
  {
    sqlite3 "$DB" "SELECT '### '||title||char(10)||'Author: '||COALESCE(author,'?')||' | Avg: '||COALESCE(avg_rating,'?')||' ('||COALESCE(ratings_count,'?')||' ratings) | year '||COALESCE(first_published,published,'?')||' | source '||source||char(10)||'Genres: '||COALESCE(genres,'[]')||char(10)||'Desc: '||substr(COALESCE(description,''),1,280) FROM books WHERE book_id='$bid';"
    echo "-- 1-2* CRITICAL --"
    sqlite3 "$DB" "SELECT '['||rating||'*] '||substr(body,1,520) FROM reviews WHERE book_id='$bid' AND rating IN ('1','2') ORDER BY CAST(likes AS INTEGER) DESC;"
    echo "-- 3* MIXED --"
    sqlite3 "$DB" "SELECT '[3*] '||substr(body,1,380) FROM reviews WHERE book_id='$bid' AND rating='3' ORDER BY CAST(likes AS INTEGER) DESC LIMIT 6;"
    echo "-- TOP POSITIVE (4-5*, by likes) --"
    sqlite3 "$DB" "SELECT '['||rating||'*] '||substr(body,1,320) FROM reviews WHERE book_id='$bid' AND rating IN ('4','5') ORDER BY CAST(likes AS INTEGER) DESC LIMIT 5;"
    printf '\n============\n\n'
  } >> "$DS"
done

echo "books.csv   : $(($(wc -l < "$OUT/books.csv")-1)) books"
echo "reviews.csv : $(($(wc -l < "$OUT/reviews.csv")-1)) reviews"
echo "dataset.txt : $(wc -c < "$DS") bytes"
echo "Out dir: $OUT"
sqlite3 -box "$DB" "SELECT source, COUNT(*) books, SUM(review_n) reviews FROM books GROUP BY source ORDER BY books DESC;"
