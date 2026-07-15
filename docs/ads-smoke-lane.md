# Ads packs smoke lane (selector-drift detection)

The ads-intelligence packs (`google_ads_transparency/search`,
`meta_ad_library/search`) scrape live third-party sites. When Google or Meta
change their markup, a pack can silently start returning empty or half-populated
manifests. The smoke lane catches that: it validates a produced manifest against
the shared schema (`schema/ads-manifest-v1.json`) **and** a per-source baseline of
the fields most likely to break, and fails loudly — naming the offending field —
when they drift.

## The checker

`ads-smoke` (crate `rzn_core`, `cargo build -p rzn_core --bin ads-smoke`) reads a
manifest from a file or stdin and exits:

- **0** — schema-valid, non-empty, and every baseline field cleared the 80%
  coverage bar.
- **non-zero** — empty result, schema-invalid, or a baseline field degraded; the
  report names each problem.

Baseline fields (beyond `id`, required on every ad):

| source | baseline fields |
|---|---|
| `google_ads_transparency` | `media_type`, `first_shown` |
| `meta_ad_library` | `media_type`, `started_at` |

```
ads-smoke <manifest.json>        # from a file
… | ads-smoke -                  # from a pipe (tolerates the run banner)
```

## Live run — one command

```bash
make ads-smoke
```

Runs each pack at `cap=5` and pipes its output through `ads-smoke`. Non-zero exit
if any pack is empty/invalid/degraded. Needs Chrome + the RZN extension.

Individually:

```bash
rzn-browser run meta_ad_library search --param query=shoes --param country=US --param cap=5 2>&1 | ads-smoke -
rzn-browser run google_ads_transparency search --param advertiser=Nike --param cap=5 2>&1 | ads-smoke -
```

## Offline checks (CI, no browser)

The schema contract and the drift-report logic run without any live site:

```bash
cargo test -p rzn_core --test ads_smoke_test --test ads_manifest_contract
```

## Suggested cadence

- **Daily** (scheduled) `make ads-smoke` to catch drift early.
- **Before a release** that touches the ads packs.
- The offline tests run on **every CI build** (deterministic, no browser).
