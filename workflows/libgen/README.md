# Libgen Workflows

Browser workflows for finding and downloading books from a **Libgen
(libgen.li-family)** mirror. The base mirror URL is a parameter because Libgen
domains rotate — point `base_url` at whichever mirror is up.

| Ref | What it does | Key output |
|---|---|---|
| `libgen/search` | Drives the homepage search box and reads the results table. Pure read. | `results: [{ title, author, publisher, year, language, pages, size, ext, md5, edition_url, file_url }]` |
| `libgen/download` | Searches, picks the best match in `format` (default epub), resolves the file's real download URL from the mirror's `ads.php` page, and emits `attachment_urls`. | `found, status, chosen{…,md5}, download_url, attachment_urls:[{url,filename}]` |

## Typical flow

```bash
# 1. (optional) Inspect candidates and their formats/md5s.
rzn-browser run libgen search --param query="Brave New World Huxley" --param format=epub

# 2. Download the epub straight into a folder.
rzn-browser run libgen download \
  --param query="Brave New World Huxley" \
  --download-dir ~/Books/bnw
# -> file lands in ~/Books/bnw/attachments/<title>.epub  (+ manifest.json)
```

`libgen/download` is the one-shot path: give it a book name, it searches,
selects, resolves, and (with `--download-dir`) saves the file. Use
`libgen/search` first only when you want to eyeball candidates or grab a
specific `result_index`.

## Parameters

`libgen/download`:

- `query` (required) — title and/or author. The default form searches all
  columns (title, author, series, year, publisher, isbn).
- `base_url` (default `https://libgen.li/`) — the mirror to use. Override when
  the domain rotates, e.g. `https://libgen.gs/`, `https://libgen.la/`. Keep the
  trailing slash.
- `format` (default `epub`) — exact match against the Ext. column (epub, pdf,
  mobi, azw3, djvu, …).
- `language` (optional) — e.g. `English`; restricts selection when at least one
  matching-format row has that language.
- `result_index` (default `0`) — pick the Nth matching candidate.

## How the download actually happens

The workflow does **not** download the file itself. It resolves the file's
direct URL and returns it under `attachment_urls`. The CLI's `--download-dir`
flag then fetches each `attachment_urls` entry into
`<download-dir>/attachments/` and writes a `manifest.json`. Without
`--download-dir` the workflow just prints `download_url` / `attachment_urls`
and downloads nothing.

Resolution uses a **same-origin `fetch`** of the mirror's own
`ads.php?md5=<md5>` page (with session cookies) to read a fresh `get.php` link
with a valid key. The CLI then fetches that link from the same machine. Libgen
download keys are tied to the requesting IP and a short time window, so the
resolve → download must run back-to-back on one machine (which is exactly how
`--download-dir` runs it).

## Flagging failures

`libgen/download` returns `found: false` with a `status` instead of throwing,
so a caller can branch and retry:

| status | meaning | suggested retry |
|---|---|---|
| `no_table` | The page had no `#tablelibgen` — `base_url` is probably not a libgen.li-family mirror. | Try another `base_url`. |
| `no_format_match` | No row in the requested `format` (carries `available_formats`). | Retry with a `format` from `available_formats`. |
| `no_download_link` | Found the book but the mirror page exposed no `get.php` link. | Try another `base_url` or `result_index`. |

`libgen/search` returns `layout_ok: false` for the same unrecognized-layout
case.

## Implementation notes

- Results come from the server-rendered `table#tablelibgen`; columns are
  Title/Series · Author(s) · Publisher · Year · Language · Pages · Size · Ext. ·
  Mirrors. `md5` is parsed from the row's native libgen.li mirror link.
- Search drives the real homepage form (type into `input[name='req']`, submit)
  rather than hitting `index.php?req=` directly.
- Targets the **libgen.li family** (libgen.li / .gs / .la / .vg / .pm), which
  share the `ads.php` + `get.php` download path. The **libgen.rs / .is** family
  uses a different page structure and is not supported here.
- No CDP required; all JS runs in the page main world.
