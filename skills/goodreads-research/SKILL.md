---
name: "goodreads-research"
description: "Topic-to-opinion Goodreads research: discover the top books in a field, read the full review spectrum (praise + criticism, incl. science-grounding critiques), then synthesize a critical opinion, a shortlist, and a spreadsheet. Use when asked to survey/compare/rank books or form a view on a subject area from reader reviews."
---

# Goodreads Research Skill

Umbrella pipeline over the five `goodreads/*` workflows. Given a topic, it produces a
ranked, opinion-bearing reading list grounded in what real reviewers say — including
the *critical* reviews, which is where the signal lives (weak evidence, dated/debunked
science, padding, value-system clashes).

This genre is **opinion-heavy with thin empirical bedrock**: most popular books are
anecdote-driven, lightly cited, and repetitive. The job is not "which is correct" but
"which value system + practical toolkit, knowing the science is soft" — so surface
disagreements honestly rather than averaging stars.

## Pipeline

1. **Discover** — `goodreads/shelf` (or `goodreads/search`) → top books in the field.
2. **Profile** — `goodreads/book` per candidate → metadata, rating histogram, genres.
3. **Read reviews** — `goodreads/reviews --coverage by_rating` per candidate → balanced
   sample across all five star buckets (captures criticism, not just praise).
4. **Synthesize** — read the collected reviews; write a per-book opinion, a shortlist,
   and a spreadsheet (this step is agent reasoning, not scripted).

Steps 1-3 are deterministic and scripted. Step 4 is the model's judgment.

## Step 1-3: collect (scripted)

```bash
./skills/goodreads-research/scripts/collect.sh --shelf "parenting" --top 6
# -> ./goodreads-research-out/parenting/{shelf.json,book_*.json,rev_*.json,urls.txt,dataset.txt}
```

`--shelf` is the Goodreads shelf slug (parenting, self-help, philosophy, psychology,
business, …). `--top` = how many books to study (default 6). `--out` overrides the
output dir. Read `dataset.txt` — it is critical-weighted (all 1-2★ reviews, a sample
of 3★, and the most-liked 4-5★ per book).

## Bulk / database mode (many books, persistent store)

For a large survey (100-150+ books) or to build a reusable, queryable store, use the
SQLite pipeline instead of `collect.sh`:

```bash
# discover top-N of a shelf + (optionally) resolve local book files, harvest by_rating
# reviews for every book into a SQLite DB. Resumable. Be gentle: 2 workers.
./skills/goodreads-research/scripts/harvest.sh \
  --shelf parenting --pages 3 --local-dir "/path/to/downloaded/books" \
  --out "/path/to/workspace" --workers 2 --delay 5

# export analysis-ready artifacts from the DB
./skills/goodreads-research/scripts/export.sh --db "/path/to/workspace/goodreads.db"
#   -> books.csv (incl. pct_low_12star controversy metric), reviews.csv, dataset.txt
```

Tables: `books`, `reviews`, `local_files`, `runs` (log). Re-running skips books already
harvested; a failed fetch is left pending for retry. `--local-dir` matches downloaded
book files to Goodreads editions (skips summary/study-guide knockoffs, prefers the
most-rated edition). Keep `--workers` at 2-3 — Goodreads throttles under heavy load.
Then synthesize from `dataset.txt` (or query the DB) per the rubric below.

## Step 4: synthesize (agent)

Read `dataset.txt`, then write two artifacts in the output dir:

**`opinion_report.md`** — a markdown report with:
- A **cross-cutting findings** section first: the criticisms that recur across *all*
  the books (this is usually the most valuable output). Always check for and call out:
  weak/absent citations ("where's the data?"), **outdated or debunked science**
  (e.g. left/right-brain, "lizard brain", pop-neuroscience), padding/repetition, and
  the dominant **value-system fault lines** (e.g. gentle vs. discipline, secular vs.
  religious) that drive 1-2★ reviews on principle rather than execution.
- A **per-book verdict** (2-4 sentences each): what it's good for, its main weakness,
  and how trustworthy its evidence base is.
- A **shortlist** of 2-3 with rationale, and an explicit skip/skim list.
- A **method caveat**: by_rating is a ~10-per-star sample (result bridge caps arrays
  at ~50), so it surfaces dominant themes reliably but is not a census.

**`books.csv`** — one row per book with columns:
`rank,title,author,year,avg_rating,ratings_count,pct_1_2_star,evidence_grounding,science_caveat,top_praise,top_criticism,verdict,shortlist`
- `evidence_grounding`: Weak / Moderate / Strong — your judgment from the reviews +
  what kind of author/source it is (clinician/researcher vs. journalist/blogger).
- `science_caveat`: the specific scientific/empirical problem if any, else "none".
- `top_praise` / `top_criticism`: the single most-recurring point on each side.
- `shortlist`: Y/N.

Be even-handed: weight recurring, specific criticisms over one-off rants, and note
when low-star reviews reflect a values disagreement rather than a quality problem.

## Scope

- Read-only. No login required (by_rating clicks public histogram buckets).
- **EPUB / book-file acquisition is out of scope** — Goodreads hosts no files; it only
  links to retailers/libraries. Do not attempt to source book files here.
- Personalized account recommendations (`/recommendations`) require login; use
  `goodreads/similar` for logged-out, book-seeded recommendations instead.

## Example output

A reference run for the "parenting" shelf (report + CSV) is in this skill's
`examples/` directory.
