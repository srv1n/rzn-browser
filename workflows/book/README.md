# Local Book Search

Search LibGen or Anna’s Archive by title, author, or ISBN and retrieve a matching file.

For agent callers, add `--agent-output` to keep provider settings, URLs and
workflow logs out of the command's output:

```bash
rzn-browser run book search --agent-output --param query="Pride and Prejudice Austen" --param format=epub
rzn-browser run book get --agent-output --param id="<ID from search>" --download-dir ~/Books/austen
rzn-browser workflow show book search --agent-output
```

This mode returns JSON with book metadata and an opaque `id` for each retrievable
result. Pass that ID to `book get` to select the exact file. IDs last 24 hours on
the same machine/app base; expired or missing IDs require a new search. An ID
cannot be combined with other `--param` overrides; set source and retry options
on the search that creates it. If the selected file
has disappeared from the results, retrieval fails rather than substituting a
different book. Rows without a retrievable file have `downloadable: false` and
no ID. Query-based `get` remains available for automatic selection.

After a download, the result includes a local file receipt (`path`, `bytes`,
`sha256`). Search never returns download paths, provider URLs, file hashes, or
raw progress. It also supports `workflow inspect` and `workflow list book`.
The same flag works for all other workflow systems, preserving their final
result links. Normal workflow output remains available when the flag is omitted.

Downloads run in a private temporary directory. Only the verified attachment is
copied to the requested directory; the source-bearing manifest stays private
and is removed when the command exits normally. Existing destination files are
never overwritten. Use a new directory if the status is `destination_exists`.
Without `--download-dir`, `get` returns metadata with status `resolved`.

The per-file download cap defaults to 1 GiB. Use `--max-download-mib 2048` for
2 GiB or `--max-download-mib 0` for no size cap. An explicit cap violation
returns `download_too_large`. Downloads stream to disk and have a 30-minute
HTTP timeout; the agent wrapper allows this extra time as well.

`--output-file` and output overrides inside `--param` are not supported in this
mode; redirect stdout to save the filtered JSON instead.

Failures return `success: false` and a stable status such as
`browser_unavailable`, `human_action_required`, `no_format_match`, or
`download_failed`, with a nonzero exit code. No raw error text is forwarded.
Use the ordinary command for operator diagnostics. A run is awaited for at least
five minutes (or the declared workflow timeout plus 30 seconds, if longer);
download runs receive an additional 30 minutes. This is an output format, not an
access-control boundary: someone with filesystem or browser access can still
inspect the source and internal
run history. Private ID records retain the query, source configuration and exact
file selector under `secure/book-results` in the app data directory (directory
mode 0700, files 0600). Expired entries are removed on the next cache operation;
the cache retains at most approximately 2,000 recent records. Downloaded book
contents are not filtered.

| Command | What it does |
|---|---|
| `book search` | List matching editions, formats, languages, and download metadata. |
| `book get` | Find a matching edition and resolve its download URL. |

```bash
rzn-browser run book search --param query="Brave New World Huxley" --param format=epub
rzn-browser run book get --param query="Brave New World Huxley" --download-dir ~/Books/bnw
```

`book get` selects a match and returns `download_url` and `attachment_urls`.
With `--download-dir`, the CLI saves the file to `<folder>/attachments/` and
writes a manifest. Without it, the command returns metadata without saving files.

Parameters:

- `query`: title, author, or ISBN (required).
- `format`: file extension; `book get` defaults to `epub`.
- `language`: optional language preference for `book get`.
- `result_index`: matching candidate to retrieve, starting at zero.
- `limit`: maximum search results (default 25, maximum 100).
- `base_url`: first book source URL with a trailing slash (default LibGen).
- `sort`: `year_desc` (newest publication first), `year_asc` (oldest first), or
  `relevance` (default). Unknown years come last. Anna’s Archive sorts on the
  server; LibGen sorts the loaded page before applying `limit`.
- `mirror_urls`: comma-separated fallback source URLs for `--agent-output`.
  Defaults to Anna’s Archive; set an empty value to disable source fallback.
- `max_attempts`: total attempt limit, 1–3 (default 3), with `--agent-output`.
- `mirror_index`: slow partner index, starting at zero after prioritizing offered “no waitlist”
  links; useful for explicit
  retries without `--agent-output`.

```bash
rzn-browser run book search --agent-output --param query="On Directing Film David Mamet" --param sort=year_desc
rzn-browser run book get --agent-output --param id="<ID from search>" --download-dir ~/Books/mamet
```

Agent output tries the first source, then configured fallback sources. If attempts
remain, retrieval advances to another slow partner on the last source. With an
Anna’s Archive base URL and no other sources, it tries up to three slow partners, prioritizing links marked “no waitlist”. Search stops on the first successfully parsed result page; it does not
merge results from several sources. An empty, valid result page is a successful
search. A cached ID keeps the chosen file hash fixed across every attempt.

Anna’s Archive retrieval opens the book’s detail page, chooses a slow partner
link (never a fast-download link), and checks the page until “Download now” appears. The runner checks for the file link four times, 15 seconds apart, and stops
as soon as it appears. These checks survive page redirects; they do not assume
the countdown always takes 20–30 seconds. LibGen opens its own download
page and reads the GET link. Both paths use browser navigation and the existing
session rather than background HTML fetches.

Automatic browser checks may clear during the existing navigation grace period.
A persistent verification wall, including the DDoS-Guard manual-check page,
returns `human_action_required`; complete it in
the browser and rerun. Verification, invalid inputs, file size caps, and local
write conflicts are not retried. Network failures, missing links, unsupported
layouts, wait timeouts, and invalid downloaded HTML can try another source.
The default plan is at most three attempts, not three retries per source.

Without `--agent-output`, use `base_url` and `mirror_index` to retry explicitly;
the ordinary runner performs one workflow attempt.

Search returns an empty results array when the file counter is zero, and
`layout_ok: false` for an unsupported source layout.
Retrieval returns `found: false` with a stable status when it cannot resolve a file. Retry with another source,
format, or candidate as appropriate. Resolve and download on the same machine;
download links can expire quickly.

## Recovering a batch

Use the [Book Recovery skill](../../skills/book-recovery/SKILL.md) and its
[recovery SOP](../../skills/book-recovery/references/SOP.md) for resumable
batches, per-file mirror checks, browser-only downloads and edition verification.
The helper reports come from the actual request list; batch-specific findings
belong in evidence, rather than being hardcoded in the tools.

PW is an explicit browser fallback in that SOP, not a source parser supported
by `book search`. A fast HTML page or successful button click does not establish
that a complete file was saved. Manual DDoS-Guard checks stop retrieval at the
homepage, results, detail and final download phases.

Focused regression checks (including the migrated standalone book fixtures):

```bash
make test-ext-unit ARGS='src/bookWorkflow.test.ts src/navigationReuse.test.ts'
python3 -m unittest discover -s scripts -p 'test_book_recovery.py'
```
