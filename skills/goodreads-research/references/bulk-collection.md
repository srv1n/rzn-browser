# Bulk collection

Use for a large or resumable survey, or to match user-supplied local book files.
Requires Bash 4+ for local-file matching, `rzn-browser`, its connected browser,
`jq`, and `sqlite3`. From this skill directory:

```bash
bash scripts/harvest.sh --shelf parenting --pages 3 \
  --out "/path/to/research" --workers 2 --delay 5
bash scripts/export.sh --db "/path/to/research/goodreads.db" \
  --out "/path/to/research/export"
```

Harvest writes `books`, `reviews`, `local_files`, and `runs` tables, plus raw
JSON and logs. Reuse the DB to resume; fetched records are skipped. Inspect
`reviews_fetched`, `review_n`, and logs for gaps rather than equating process
completion with coverage. Keep concurrency low; reduce it or increase the delay
when the site throttles.

Add `--local-dir "/path/to/books"` only for user-selected files. Matching is
heuristic: verify title, author, and edition before treating it as the same book.
The script does not acquire or read the books themselves.

Export writes `books.csv`, `reviews.csv`, and a critical-weighted `dataset.txt`;
it overwrites those names in its output directory. Preserve the raw export and
write synthesis separately. `pct_low_12star` comes from the rating histogram, not
the balanced review sample. Raw reviews retain full text and source URLs.
