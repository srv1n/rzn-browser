# Bing Workflows

Read-only search workflows for Bing. All open a fresh tab so they can run in parallel.

## Pack

| Workflow | Purpose | Required params |
| --- | --- | --- |
| `bing_web_search` | Up to 10 organic web results (title, url, source, snippet). | `search_query` |
| `bing_news_search` | Up to 10 news cards (title, url, source, time, snippet). | `search_query` |
| `bing_videos_search` | Up to 20 video tiles (title, url, channel). | `search_query` |
| `bing_images_search` | Up to 50 image entries with high-res URL, thumbnail URL, source page, title. | `search_query` |
| `bing_images_download` | Up to 20 thumbnails streamed through Chrome into `~/Downloads/<folder>/`. | `search_query`, `download_folder` |

## Image pack split

`bing_images_search` vs `bing_images_download` are kept separate because they have fundamentally different output contracts:

- `bing_images_search` returns JSON with `high_res_url` (Bing's `m.murl`, the real original URL) and `thumbnail_url`. Pipe the URLs into `wget`/`curl` for full-resolution downloads — this is the high-quality path.
- `bing_images_download` writes files directly via Chrome's download manager. Thumbnails only (Chrome's cross-origin download is reliable for `th.bing.com` but not for arbitrary host origins). Useful when the authenticated browser session is the only path through a firewall/geo-block.

## Running

```bash
rzn-browser run bing web-search --param search_query="claude code release notes"
rzn-browser run bing news-search --param search_query="anthropic"
rzn-browser run bing videos-search --param search_query="claude code tutorial"
rzn-browser run bing images-search --param search_query="sunset photos"
rzn-browser run bing images-download --param search_query="cats" --param download_folder="cats"

# Pipeline for real high-res downloads:
rzn-browser run bing images-search --param search_query="sunset" 2>&1 \
  | sed -n '/^\[$/,/^\]$/p' \
  | jq -r '.[0].result.items[].high_res_url' \
  | head -10 \
  | xargs -I {} wget -P ~/Downloads/sunset "{}"
```

## Design rules

- No `_v1` suffix in filenames — `id` and `version` live inside the JSON.
- `browser_automation.use_current_tab: false` everywhere so parallel runs don't collide.
- Underscore naming (matches the rest of the repo).
- Every workflow ships a `help` block validated by `rzn-browser workflow validate`.

## Notes

- Bing's organic results (`li.b_algo`) automatically exclude `li.b_ad` sponsored listings.
- The `m` attribute on `a.iusc` is Bing's image metadata payload — `murl` is the real host URL, `turl` is Bing's thumbnail, `purl` is the hosting page, `t` is the title. Dimensions are not present in the `m` payload.
- `span[aria-label]` on news cards carries the human-readable published time.
