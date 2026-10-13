# Supreme Court of India workflows

| Ref | What it does |
|---|---|
| `sci/judgements-open` | Opens the judgment-date search on sci.gov.in, fills a date window, returns the CAPTCHA image as a PNG data URL. |
| `sci/judgements-by-date` | In the tab `sci/judgements-open` left open: types `captcha_answer`, presses Search, reads every result page. |

The two runs share the site's parked tab. Nothing calls `admin-ajax.php` directly, and the page's
`sci_auto_submit` URL parameter is not used.

## Bulk listing

```sh
scripts/sci/sci-listing.sh window 2019-01-01 2019-01-31   # one window
scripts/sci/sci-listing.sh backfill                       # 2014-01-01 .. today, month windows, resumable
SCI_START=1950-01-01 SCI_END=2013-12-31 scripts/sci/sci-listing.sh backfill   # any range
scripts/sci/sci-listing.sh weekly                         # the last 14 days
```

`scripts/sci/solve-captcha.sh` reads each CAPTCHA with `claude -p --model haiku` (default) or
`codex exec -m gpt-6-luna` (`SCI_SOLVER=codex`). After three wrong answers on one window the driver stops and
sends a desktop notification. It also stops on any site message it does not recognise.

The site also answers "No records found" when it quietly refuses a search. A window that comes back empty is
searched once more before it is recorded empty. In weekly mode an empty window of a week or more stops the run.
In a backfill, `SCI_EMPTY_STREAK` (default 6) empty windows in a row trigger a control search of January 2019:
if it finds rows the empty windows stand, otherwise the run stops and requeues them. Only one driver runs at a
time; the weekly run waits up to 24 hours for a running backfill.

Output goes to `~/rzn-sc-listing/` (`SCI_LISTING_DIR`): `rows.jsonl`, `windows.jsonl` and
`pages/<from>_<to>_<page>.html`. `SCI_GAP` (default 30) sets the seconds between searches. A window that
paging could not finish in one run, or that reaches `SCI_MAX_ROWS`, is split in half and requeued.

## Step 0 findings (12 October 2026)

Probe: 01-01-2019 to 31-01-2019.

1. **Maximum range:** the form validator allows `to_date` at most 30 days after `from_date`. Calendar months fit.
2. **Paging:** January 2019 returned all 127 rows on one page with an empty `#paginationHtml`. Paged results
   use `#paginationHtml .pagination a[data-page-id]` plus a `data-nonce`. A server row cap is not known yet.
3. **New CAPTCHA:** each search needs one; the site clears the box and reloads the image after every response.
   Paging resubmits with the pagination nonce and needs none. The CAPTCHA is a sum or difference such as `4 + 4`.
4. **PDF link shape:** the Judgment cell lists the English PDF
   (`https://api.sci.gov.in/supremecourt/<year>/<diary>/<diary>_<year>_Judgement_<dd-Mon-yyyy>.pdf`), then
   regional PDFs under `supremecourt_vernacular/`, then INSC citation links to `api.sci.gov.in//pdfdate/index1.php`.
   `pdf_url` takes the English PDF, else the first regional one. Some rows are `_Order_` PDFs.
5. **Columns:** Serial Number, Diary Number, Case Number, Petitioner / Respondent (one cell, split on `VS`),
   Petitioner/Respondent Advocate, Bench, Judgment By, Judgment. The decision date is the date in Judgment.
6. The live form has no `optradio` language field. The page sends `language=en`.

The extension's result serializer cuts arrays to 50 items, so the reader returns rows as `rows_json`, a string.
