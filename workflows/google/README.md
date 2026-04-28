# Google Workflows

Browser-driven workflows for Google services. Run via:

```
rzn-browser run google <workflow> --param key=value [--param key=value]
```

## Naming convention

- File on disk: `google-<resource>.json` (kebab-case).
- Catalog reference (auto-derived from the filename): `google/<resource>`.
- Internal `id` field: `google_<resource>_v<N>` (snake_case, version pinned in id; bump `version` field for changes — never rename the file with a `-v2` suffix).

## Workflows

| Workflow             | Required params                  | Optional params                                                       | Output shape                                                                                |
| -------------------- | -------------------------------- | --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `google/search`      | `search_query`                   | `vertical` = `web` (default) \| `news` \| `books`                     | `[{ title, url, snippet, source?, age? }]` (source/age populated for news only)             |
| `google/images`      | `search_query`                   | —                                                                     | `[{ alt_text, thumbnail_src }]` (full URLs hidden behind click — see notes)                |
| `google/scholar`     | `search_query`                   | —                                                                     | `[{ title, url, authors, snippet, cited_by_url }]`                                          |
| `google/maps`        | `query`                          | —                                                                     | `[{ name, rating, reviews, price_range, category, address, description, status, url }]` + screenshot |
| `google/maps-directions` | `origin`, `destination`      | `mode` = `driving` (default) \| `walking` \| `bicycling` \| `transit` | `summary { distance, duration }` + `steps[{ step_index, instruction, distance }]` + screenshot |
| `google/weather`     | `location`                       | —                                                                     | `{ location, time, condition, temperature, precipitation, humidity, wind }`                 |
| `google/finance`     | `query` (`TICKER:EXCHANGE`)      | —                                                                     | `quote {…}` + `stats[{ label, value }]` + screenshot                                        |
| `google/translate`   | `text`, `to_language`            | —                                                                     | `{ translated_text, search_text }`                                                          |
| `google/flights`     | `origin`, `destination`          | —                                                                     | `{ search_text }` (best-effort)                                                             |
| `google/hotels`      | `search_query`                   | —                                                                     | `{ search_text }` (best-effort)                                                             |
| `google/trends`      | `search_query`                   | —                                                                     | `{ text }` (best-effort, no chart numbers)                                                  |
| `google/lens`        | `image_source` (URL)             | `mode` = `text` (default) \| `items`                                  | `{ mode, text }` or `{ mode, items[…] }`                                                    |

## Parameter aliases

The orchestrator recognizes the alias group `search_query` ↔ `query` ↔ `q`. If a workflow declares any one of those, callers may pass any other.

```
rzn-browser run google search --param search_query="rust"
rzn-browser run google search --param query="rust"
rzn-browser run google search --param q="rust"
```

## Screenshots

`google/maps`, `google/maps-directions`, and `google/finance` save a viewport PNG to:

```
test-results/workflow-artifacts/<workflow-stem>/
```

If you see `Unknown action type: take_screenshot`, rebuild + reload the extension:

```
make build-ext
make reload-ext
```

## Verticals folded into `google/search`

The `vertical` enum on `google/search` covers what used to be separate workflows (`google-news`, `google-books`):

```
rzn-browser run google search --param search_query="AI regulation" --param vertical=news
rzn-browser run google search --param search_query="category theory" --param vertical=books
```

Shopping (`google-shopping` previously) is **not** folded in. Google moved shopping to a React widget surface (`udm=28`) without clean per-product anchors; the old `tbm=shop` extractor no longer works. Building a usable shopping workflow needs its own dedicated effort.

`google/images` stays as its own workflow because the image-grid output shape (URLs + alt text) genuinely differs from organic title/url/snippet results.

## Modes folded into `google/lens`

`google/lens` accepts `mode=text` (default — visible page text) or `mode=items` (enumerated clickable result targets). The previous `google-lens-detect` workflow has been folded into this enum.
