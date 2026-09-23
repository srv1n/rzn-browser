---
name: goodreads-research
description: Compare books on a topic using Goodreads metadata and reader reviews, producing a reasoned shortlist and research artifacts.
---

# Goodreads Research

Build a recommendation grounded in the reader's goal and the reviews actually
collected. For a single lookup, use the relevant Goodreads workflow directly.

## Collect the needed evidence

Requires `rzn-browser`, its connected Chrome extension, and `jq`. Commands below
are relative to this skill directory. Choose a fresh output directory to preserve
previous research.

```bash
bash scripts/collect.sh --shelf "parenting" --top 6 --out "/path/to/research"
```

`--shelf` is a Goodreads shelf slug; `--top` defaults to six candidates from the
first shelf page. The script writes `shelf.json`, `book_*.json`, `rev_*.json`,
`urls.txt`, and `dataset.txt`. For a specified book list, call `goodreads/book`
and `goodreads/reviews` with `coverage=by_rating` for those books instead.

Read [bulk collection](references/bulk-collection.md) for a resumable SQLite
survey or matching existing local book files. Small comparisons do not need a DB.

## Interpret the evidence

`dataset.txt` favors critical reviews and truncates excerpts. Consult raw reviews
for context and source links before attributing a claim. Coverage by rating is a
sample across star buckets, not the population's rating distribution; use the
book's rating histogram for percentages. Report missing buckets and failed books.

Separate recurring praise, specific criticism, and differences in reader values.
Do not assume a genre has weak evidence or infer scientific validity from stars,
reviewer sentiment, or an author's job title. Attribute claims about weak or
debunked science to reviewers unless checked against relevant primary sources.

## Deliver

Unless the user requests a different format, write:

- `opinion_report.md`: comparison criteria, recurring themes, per-book strengths
  and weaknesses, a justified shortlist, source links, and coverage limitations.
- `books.csv`: title, author, book URL, rating/count, recurring praise/criticism,
  verdict, and shortlist status. Keep unavailable facts empty or explicitly unknown;
  use the user's requested columns if supplied.

Check artifacts against the evidence and account for every requested candidate.
The [parenting example](examples/parenting-opinion_report.md) illustrates a report,
not conclusions to reuse for another topic.

These workflows read public Goodreads pages and write local research files.
Acquiring book files and changing account recommendations are outside this skill.
