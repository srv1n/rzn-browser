# Icons — Diagram System v1

A typed primitive in the schema. Not freeform images.

Three kinds, one schema, one render path. Curated libraries only — no escape hatch to ad-hoc SVG or PNG.

---

## 1 · The schema

```ts
type IconRef =
  | { kind: 'system'; name: SystemIconName; size?: 16 | 20 | 24 | 32 }
  | { kind: 'brand';  name: BrandIconName;  size?: 20 | 24 | 32 | 40 }
  | { kind: 'glyph';  name: GlyphName;      size?: 12 | 16 | 20 }

type SystemIconName =
  | 'user' | 'users' | 'cpu' | 'cog' | 'terminal'
  | 'code' | 'share-2' | 'puzzle' | 'globe' | 'monitor'
  | 'smartphone' | 'cloud' | 'server' | 'database' | 'pointer'
  | 'eye' | 'camera' | 'box' | 'lock' | 'key'
  | 'file' | 'folder' | 'zap' | 'layers' | 'arrow-right'

type BrandIconName =
  | 'chrome' | 'firefox' | 'safari' | 'edge'
  | 'github' | 'openai' | 'anthropic'
  | 'slack' | 'discord' | 'apple'

type GlyphName =
  // domain-specific RZN glyphs (defined per-diagram, never freeform)
  | 'rzn-monogram' | 'cursor-target' | 'browser-chrome-bar'
```

Every icon used in a diagram must resolve to one of these typed names. There is no `iconUrl` or `customSvg` field.

**Why typed:** prevents drift, guarantees deterministic render, lets the layout engine reserve correct intrinsic sizes during the measure pass.

---

## 2 · The libraries

| Kind | Source | License | Style |
|---|---|---|---|
| **system** | [Lucide](https://lucide.dev/) (Feather fork) | MIT | 24×24 viewBox, 1.5 stroke, round caps + joins, `fill: none` |
| **brand** | [Simple Icons](https://simpleicons.org/) | CC0 1.0 | 24×24 viewBox, single-path filled, official brand color allowed |
| **glyph** | RZN-authored | proprietary | 24×24 viewBox, hand-built for one specific use |

Brand icons that use **multi-color** (e.g. Chrome trefoil) are reconstructed manually per the brand's official palette — Simple Icons monochrome version is *not* used in branded contexts.

---

## 3 · Sizing rules

Sizes snap to the type scale. No arbitrary px values.

| Size | Where |
|---|---|
| **12 px** | inline with body text (`size-body 14`); rare; only `glyph` kind |
| **16 px** | inline with caption (`size-caption 12`); secondary marks |
| **20 px** | minimum for brand glyph in clear-space; in-text bullets |
| **24 px** | default; pairs with `size-h3 20` and below |
| **32 px** | large icon in component title position; pairs with `size-h2 28` |
| **40 px** | brand glyph in identity contexts only (logos, headers) |

Snap to the 8 px grid. An icon centered at `(cx, cy)` must have integer coords; SVG output of `25.5 7.999` is a layout bug.

---

## 4 · Color rules

| Kind | Default fill / stroke | Override |
|---|---|---|
| **system** | inherits parent `color` token (usually `ink #1A1A17`) | Yes — semantic color tokens only (`ink`, `accent`, `flow`, `escalate`) |
| **brand** | official brand color, mandatory | No — never recolor a brand mark |
| **glyph** | inherits parent `color` token | Yes |

System icons use `stroke` (not `fill`) — this is the Lucide / Feather convention. Default stroke is 1.5 px at 24×24 viewport. Scaling preserves visual weight because Lucide is designed for it.

A brand icon dropped into a non-branded context **always reads as a brand reference**. Don't decorate. If you want "a generic browser," use the system `globe` icon, not the Chrome glyph.

---

## 5 · Placement rules

Inside a node, an icon takes one of three positions:

```
┌─────────────────────┐    ┌─────────────────────┐    ┌─────────────────────┐
│        ⚙              │    │  ⚙   Title          │    │  Title              │
│      Title             │    │      caption       │    │  caption            │
│      caption           │    │                     │    │                  ⚙ │
└─────────────────────┘    └─────────────────────┘    └─────────────────────┘
   (a) icon-top              (b) icon-left              (c) icon-corner
```

- **(a) icon-top** — primary identity. Use when the icon *is* the component's identifying feature (Planner, Worker, Chrome). Default for cards and capsule-cylinders ≥128 px tall.
- **(b) icon-left** — co-equal with title. Use in compact pills (h ≤ 64) where vertical space won't fit a stack.
- **(c) icon-corner** — affordance / hint. Use sparingly, only when a node is identified primarily by its title and the icon is a meta-mark (e.g. a small lock indicating restricted access).

Pick one per diagram. Do not mix (a) and (b) in the same diagram — it reads as inconsistent type rather than meaningful contrast.

---

## 6 · Inlining rule

All icons are **inlined as SVG paths** in the output document.

- No `<image href="…">`. No `<use>` from external sprite. No PNG fallback.
- This guarantees: zero network requests, zero CORS issues, deterministic byte-for-byte renders, scales cleanly to any zoom, recolorable via `currentColor`.
- The build emits the icon path verbatim from the curated library. No optimization, no minification (those introduce non-determinism).

The renderer reads `IconRef` and emits:

```svg
<g transform="translate(cx-12, cy-12)" fill="none"
   stroke="{token}" stroke-width="1.5"
   stroke-linecap="round" stroke-linejoin="round">
  {{ paths from library }}
</g>
```

The `translate(cx-12, cy-12)` shifts the 24×24 icon viewport so its center sits at `(cx, cy)`.

---

## 7 · Adding to the library

Adding an icon = adding an entry to the type `SystemIconName` (or `BrandIconName`, `GlyphName`) and committing the path verbatim into `src/icons/<kind>/<name>.svg`.

It is **not** "find a nice icon online and use it." Three constraints:

1. **Source must be in the curated set.** Lucide for system, Simple Icons for brand, RZN-authored for glyph. Other libraries (Heroicons, Phosphor, Material) are not in v1.
2. **Path must be hand-verified.** Open it in `02-icons.svg`'s grid and confirm it renders at 24, then resize to 16 — does the stroke still read? If a Lucide icon has too much detail at 16 px, drop it.
3. **No new size tokens.** If your icon needs to be 28 px, you're using the wrong size — pick 24 or 32.

A new brand icon also needs the brand's official color hex documented in the entry, even if the diagram uses a monochrome rendering.

---

## 8 · Policy for branded marks

- Brand glyphs appear only in **branded contexts** — i.e. when the node's title literally is the product name ("Google Chrome", "GitHub", "Slack").
- A node titled "Browser" gets the system `globe` icon, not the Chrome glyph.
- Brand glyph minimum size: **20 px**. Below that, brand recognition collapses and it reads as a colored blob.
- Brand glyph clear-space: 1× the glyph diameter on all sides. No text or stroke may enter this zone.
- Multi-color brand glyphs are not recolored. A grayscale Chrome glyph violates Google brand guidance and weakens the diagram's claim that "this is your real Chrome."

---

## 9 · The reference swatch

`02-icons.svg` is the source of truth. It includes:

- The 25 system icons currently in `SystemIconName`.
- The 10 brand icons currently in `BrandIconName`.
- A size ladder showing the same icon at 16 / 20 / 24 / 32 px.
- Three in-context examples showing icon-in-capsule, icon-in-card, and brand-icon-in-branded-capsule.

When this file disagrees with `ICONS.md`, the file wins — paths are authoritative, prose is documentation.

---

## 10 · Open questions for v2

- Iconography for **state** (loading, error, success, blocked) — currently we use color (escalation slot). Should there be a small typed badge instead?
- Animated icons for the animated diagram variant — same path, animated stroke-dashoffset? Out of scope for v1.
- Light-mode vs dark-mode brand icons — Simple Icons offers both; we currently render only light-mode. Decide when we add dark-mode diagrams.
- Internationalization — the `arrow-right` icon implies LTR reading order. For RTL exports, swap to `arrow-left` automatically? Defer.
