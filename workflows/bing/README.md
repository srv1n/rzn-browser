# Bing Workflows

Read-only search workflows for Bing. All open a fresh tab so they can run in parallel.

## Pack

| Workflow | Purpose | Required params |
| --- | --- | --- |
| `bing_web_search` | Up to 10 organic web results (title, url, source, snippet). | `search_query` |
| `bing_news_search` | Up to 10 news cards (title, url, source, time, snippet). | `search_query` |
| `bing_videos_search` | Up to 20 video tiles (title, url, channel). | `search_query` |
| `bing_images_search` | Up to 50 image entries with high-res URL, thumbnail URL, source page, title. | `search_query` |
| `bing_images_download` | Up to `count` (default 100) full-size original URLs; `--download-dir` saves the files. | `search_query` |

## Image pack split

- `bing_images_search` returns metadata for the first ~50 results without scrolling.
- `bing_images_download` scrolls the grid until `count` results load and returns their original URLs (Bing's `m.murl`) as `image_urls`. It applies Bing's Extra-large size filter by default (`size=wallpaper|large|any`) and can drop anything below `min_width`/`min_height` pixels. With `--download-dir`, the CLI fetches each original into `<dir>/images/` and writes `manifest.json`. Hosts that block hotlinking are logged and skipped, so expect a few fewer files than `count`.

## Running

```bash
rzn-browser run bing web-search --param search_query="claude code release notes"
rzn-browser run bing news-search --param search_query="anthropic"
rzn-browser run bing videos-search --param search_query="claude code tutorial"
rzn-browser run bing images-search --param search_query="sunset photos"
rzn-browser run bing images-download --param search_query="cats" --param count=30 --download-dir ~/Pictures/cats

# 100 full-size originals into ~/Pictures/sunset/images/
rzn-browser run bing images-download --param search_query="sunset" --download-dir ~/Pictures/sunset
```

## Design rules

- Dedicated workflow tabs by default, so parallel runs do not collide.
- Underscore naming (matches the rest of the repo).
- Every workflow ships a `help` block validated by `rzn-browser workflow validate`.

## Notes

- Bing's organic results (`li.b_algo`) automatically exclude `li.b_ad` sponsored listings.
- The `m` attribute on `a.iusc` is Bing's image metadata payload — `murl` is the real host URL, `turl` is Bing's thumbnail, `purl` is the hosting page, `t` is the title. Dimensions are not present in the `m` payload.
- `span[aria-label]` on news cards carries the human-readable published time.
