# Book recovery SOP

Run from the RZN Browser checkout. The tools are foreground operators, not a daemon. Use a separate evidence folder per list; preserve it for resumability. Set `RZN_BOOK_CLI` to choose a specific binary; otherwise helpers use the checkout’s debug binary when present, then the installed CLI. Helpers use checkout workflow data. A development checkout needs `make build-rust` after Rust changes; JSON-only changes need no compilation.

## 1. Inventory and define the request

Confirm the destination and inspect existing EPUB metadata or PDF title/copyright pages. Similar filenames may be a different author or edition. Record existing verified copies rather than downloading them again.

Save `requests.json` in the evidence folder:

```json
[
  {
    "key": "handley",
    "title": "Everybody Writes",
    "author": "Ann Handley",
    "query": "Everybody Writes Ann Handley",
    "expected_identifiers": ["9781119854319", "9781119854166", "9781119854326"],
    "filename": "Everybody Writes (2nd edition) - Ann Handley"
  }
]
```

Keys are unique short letters/digits/underscore/hyphen labels. `expected_identifiers` is optional and requires one exact ISBN match in EPUB identifier/source metadata or PDF front matter. Use it for requested editions. `filename` is an optional extension-free destination name. PDFs default to at least 20 pages; `min_pdf_pages` can express a reviewed exception. Length is a review-scan guard, not proof of identity.

## 2. Search and review

```bash
python3 scripts/book_recovery.py search EVIDENCE/requests.json EVIDENCE --sort year_desc
```

Defaults: LibGen first, Anna fallback, up to three attempts. Choose `--base-url`, `--mirror-urls`, `--max-attempts`, and `--limit` as needed. Search stops at the first valid result page; it does not merge collections or continue past a valid empty page. To check another source after an empty result, run an explicit search and retain its evidence.

Review `KEY.search.json`: title, author, language, edition, format, size. Prefer a reasonably sized complete copy. Newest publication year does not necessarily mean a new edition. Try title+author, author alone, and print/ebook ISBNs before marking a title unavailable. A metadata catalogue entry is not a downloadable copy.

Save `selections.json` with reviewed IDs:

```json
[{"key":"handley","id":"EXACT_ID_FROM_SEARCH"}]
```

IDs expire after 24 hours. Use `search --refresh` and review a new ID after expiry. The batch helper otherwise reuses recent successful search receipts.

## 3. Retrieve and resume

```bash
python3 scripts/book_recovery.py get EVIDENCE/selections.json EVIDENCE
```

The helper saves `KEY.get.json`, skips downloads only after checking their recorded paths, sizes and SHA-256, and rejects a second writer to the same evidence folder. A changed or missing saved file needs inspection; it is not silently redownloaded over an existing destination. Search/source options belong on the search that creates an ID; they cannot override an exact-ID get.

Exit status: 0 for successful processing, 1 for failures requiring review, 2 when browser attention is required. Individual results retain the detailed status. An empty successful search is still an empty search, not a completed book.

Keep one batch controller per book tab: competing batches queue page operations, add latency and increase verification failures. A slow transfer may continue independently after the browser workflow resolves its link. Check growing file size before calling it stalled. Ctrl-C stops the foreground controller; check for an in-flight child or browser download before restarting the same key.

For CAPTCHA, ask the user to complete the visible manual check and stop new attempts. Resume after their confirmation. If the runner’s cooldown remains active, wait for it to expire; preserve the queue’s existing tab binding. A browser challenge and a dead file are different outcomes.

## 4. Choose a faster mirror per file

Inspect the selected book’s actual detail page:

```bash
python3 scripts/book_mirror_inspect.py OBSERVED_DETAIL_URL EVIDENCE/detail.txt
```

From the returned links, choose up to five relevant mirrors. Save their observed URLs as `[{"name":"mirror label","url":"observed URL"}]`, then probe:

```bash
python3 scripts/book_mirror_benchmark.py EVIDENCE/probes.json EVIDENCE/probes-result.json
```

The probe samples at most 64 KiB with a 15-second socket timeout. Its `seconds` measures response/sample latency, not full-file throughput. Its `file` flag recognizes PDF/ZIP signatures; EPUB/container checks still follow. Test the actual GET/file endpoint after a detail-page success. Keep full-transfer time and size when choosing among working mirrors. Recheck failures per book: one server can serve one file quickly and fail another.

An HTTP 403 does not prove the ordinary browser session cannot download. Inspect the offered page and use its normal controls where available; stop an explicitly blocked route and do not bypass a verification wall. Anna “no waitlist” partners can have much lower transfer speed; a shorter wait does not imply a faster complete download. “Fast” Anna links may require membership and are not the default path.

### LibGen direct fallback

For a working observed `ads.php?md5=...` detail URL, create `mirror-selections.json` containing the reviewed full metadata:

```json
[{"key":"example","title":"Reviewed book","author":"Reviewed author","ext":"epub","md5":"EXACT_32_HEX_FILE_HASH","id":"OPTIONAL_SEARCH_ID"}]
```

```bash
python3 scripts/book_external_download.py EVIDENCE example OBSERVED_DETAIL_URL
```

The helper navigates the page, reads its actual GET link, uses the native downloader, and verifies the completed file’s MD5 before recording a receipt. No key or signed URL is guessed. Use `--selections PATH` for another reviewed selection file. A different edition or file needs an explicitly reviewed selection; required editions remain enforced at publishing.

### PW / browser-only download fallback

Inspect the offered PW book/download page. When its normal countdown enables the observed control, use the inspector’s optional selector argument:

```bash
python3 scripts/book_mirror_inspect.py OBSERVED_DOWNLOAD_URL EVIDENCE/pw.txt OBSERVED_DOWNLOAD_SELECTOR
```

The helper waits for the selector to be enabled, clicks it, and records the page. This does not itself produce a verified receipt. Watch the browser’s download folder until the temporary extension disappears; locate the completed file by its exact expected hash, not only its filename. Then adopt it:

```bash
python3 scripts/book_external_download.py EVIDENCE example --local-file COMPLETED_FILE --selections EVIDENCE/mirror-selections.json
```

The helper rejects unfinished download files and a wrong MD5. The standard book CLI has no PW search/parser adapter; this explicit operator fallback should be reported accurately.

## 5. Verify and publish

```bash
python3 scripts/book_recovery_publish.py EVIDENCE BOOKS_FOLDER
```

Publishing checks receipt size/SHA-256, EPUB ZIP CRC/container/OPF, or PDF signature/full text parsing/page count using `pdfinfo` and `pdftotext` (Poppler). EPUBs require only Python’s standard library. The optional `expected_identifiers` constraint checks the requested edition. Review extracted EPUB metadata and PDF title/copyright pages for title, author and edition; no automatic fuzzy match replaces that review.

The publisher preserves existing files, reuses an identical file and adds a hash suffix for a different file with the same requested name. It writes `verification.json` in evidence and a Markdown report in the destination. Use `--report-name NAME.md` to keep a stable report filename when resuming. Batch-specific edition notes, measured mirrors and identity checks belong in `EVIDENCE/batch-notes.md`, which is included in the report.

Record unresolved titles in `EVIDENCE/unavailable.json`, keyed by request key, with `status`, the searches actually checked, and an optional `purchase_url`. Only record an outcome supported by completed searches. Unresolved entries never override verified downloads. Add legitimate publisher/library/store access options; do not buy, subscribe or count a preview as completion without the appropriate authorization.

## Completion and workflow maintenance

Every request ends with a verified destination file or an explicit unresolved outcome. Report totals, edition differences and remaining blockers. Keep the evidence and preserve existing books.

When changing workflow data, reproduce the observed failure and use the focused checks:

```bash
make test-ext-unit ARGS='src/bookWorkflow.test.ts src/navigationReuse.test.ts'
python3 -m unittest discover -s scripts -p 'test_book_recovery.py'
```

For Rust output/retry changes, also use `make rust ARGS='test -p rzn-browser --bin rzn-browser agent_output::tests'`. Build native changes through `make`; keep site-specific markup in workflow data. An unrelated dirty checkout is not permission to stage or commit sibling work.
