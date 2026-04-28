# Draft notes — upstream `roughjs-svg-diagrams` skill

Status: **draft / scratch**. Do not propagate yet. The downstream team will compose the formal note once we settle the visual direction.

These are observations from rendering the RZN browser-automation visuals (4 diagrams × several aesthetic variants). The current skill is great for the *rough hand-sketched system map* it was designed for, but several of the variants we've explored had to be hand-written as raw SVG because the spec format does not express them. Capturing those gaps here so we can decide what's worth adding upstream.

---

## 1. Fill types beyond solid color

**What we needed:** dot-fill, diagonal hatch, dashed-stroke containers — to convey "agent / executor", "sandboxed / scoped", and "open boundary" semantics without leaning on color.

**Today:** `node.fill` is a single CSS color string.

**Suggested shape (sketch only):**

```json
"fill": { "type": "dotfill", "color": "#1a1a17", "spacing": 8, "radius": 0.9 }
"fill": { "type": "hatch", "angle": 45, "spacing": 7, "color": "#1a1a17", "opacity": 0.35 }
"fill": "#f4faeb"   // string form still accepted, treated as solid
```

Implementation note: emit a `<pattern>` into `<defs>` per unique fill descriptor; reference via `url(#...)`. Patterns are deterministic so they don't break the seed-stable rendering contract.

---

## 2. Capsule-cylinder node shape

**What we needed:** pill-shaped nodes with vertical end-bars (the "cylinder seen from the side" silhouette in OpenAI/Codex diagrams). Used heavily for "runtime component" or "agent" boxes.

**Today:** Only rounded-rectangle nodes via `r` corner radius.

**Suggested shape:**

```json
{ "shape": "capsule",         "w": 200, "h": 80 }
{ "shape": "capsule-cylinder", "w": 200, "h": 80, "endBarInset": 18 }
```

`capsule` = rectangle with `r = h/2`. `capsule-cylinder` = capsule plus two vertical strokes inset from the rounded ends.

---

## 3. Curved (Bézier) edges

**What we needed:** smooth S-curves between nodes for the editorial / OpenAI aesthetic. Orthogonal routing reads "system diagram"; smooth curves read "narrative flow".

**Today:** `edge.points` is a polyline; segments are straight.

**Suggested shape:**

```json
{
  "from": "user_task",        // anchor name, see §6
  "to":   "rzn_runtime.left",
  "style": "smooth",          // "smooth" | "ortho" | "straight" (default)
  "tension": 0.5
}
```

`smooth` would emit a single cubic Bézier whose control points are derived from the source/target anchor tangents — caller doesn't hand-author control points.

---

## 4. Container with title-tab

**What we needed:** a labeled container where the title sits in a small tab clipped to the container's top edge (like the "RZN RUNTIME · LOCAL" label on the hatched box). Currently we paint the container, then paint a separate text+rect on top, then have to manually break the container's stroke under the tab.

**Suggested shape:**

```json
{
  "id": "rzn_lane",
  "shape": "container",
  "x": 380, "y": 310, "w": 700, "h": 320,
  "fill": { "type": "hatch", "angle": 45 },
  "stroke": "#1a1a17",
  "title": { "text": "RZN RUNTIME · LOCAL", "tab": true, "side": "top-left" }
}
```

Renderer breaks the container border under the tab automatically.

---

## 5. Multi-tier node labels

**What we needed:** capsule pills with two lines — a primary label (sentence-case or all-caps) and a secondary subtitle in a different font/weight/color. Today this works via the `label: ["line1", "line2"]` array, but both lines share the same `font` / `fontSize` / `weight` / `textColor`.

**Suggested shape:**

```json
"label": [
  { "text": "RZN RUNTIME", "size": 17, "weight": 700 },
  { "text": "planner · worker · broker", "size": 11, "weight": 500, "color": "#3f6f1a", "font": "mono" }
]
```

Backwards compatible: bare strings still get the node-level defaults.

---

## 6. Named anchor points on nodes

**What we needed:** `from: "rzn_runtime.right"` instead of `points: [[1030, 594], ...]`. The validator already complains about endpoints that miss a node by a few pixels — naming the anchor would make it impossible to author that bug.

**Suggested shape:**

```json
"edge": { "from": "user_task.right", "to": "rzn_runtime.top", "style": "smooth" }
```

Anchors: `top | right | bottom | left | top-left | top-right | bottom-left | bottom-right | center`. Renderer resolves to coordinates at draw time.

This is the single highest-leverage change — about half the bugs we hit in this batch were "edge endpoint is 4px inside the node" or "edge approaches the wrong face".

---

## 7. Stage / lane labels above containers

The numbered or all-caps zone labels above a container (e.g. `USER · RZN RUNTIME · LOCAL · GOOGLE CHROME · TARGET`) recur in every editorial-style diagram we've produced. Worth a first-class primitive:

```json
"stages": [
  { "x": 160,  "label": "USER" },
  { "x": 500,  "label": "RZN RUNTIME · LOCAL" },
  { "x": 1130, "label": "GOOGLE CHROME" },
  { "x": 1410, "label": "TARGET" }
]
```

Renderer draws them in the standard mono/all-caps/letter-spaced style above a horizontal rule.

---

## 8. Animation hints (optional, lower priority)

The animated SVG variant uses SMIL `animate` and `animateMotion`. The current spec format has no notion of motion. If we ever want this generated rather than hand-written:

```json
"animations": [
  { "type": "marching-ants", "edges": ["e1", "e2", "e3"], "period": "2s" },
  { "type": "packet", "path": "user_task -> target_web_app", "duration": "6s", "color": "#3f6f1a" }
]
```

Not urgent — animated diagrams are a small minority and the SMIL output is fairly readable when hand-authored.

---

## 9. Validator additions implied by the above

If we add the features above, the validator should also learn:

- Reject `from`/`to` that name a node id that doesn't exist.
- Reject anchor names that aren't in the allowed set.
- For `style: "smooth"` edges, verify the *generated* Bézier doesn't cut through unrelated nodes (current check only walks polyline segments).
- For `capsule-cylinder` nodes, verify `endBarInset < w/2`.
- For `pattern` fills, verify `spacing > 0` and `radius > 0`.

---

## Open questions for the upstream team

1. Are patterned fills (dotfill, hatch) something the skill should own, or should we keep them as a separate "editorial" theme layer?
2. Should curved edges share the same `points` shape as polyline edges (with a `style` flag), or be a distinct edge type? The former is friendlier; the latter is cleaner.
3. The named-anchor change (§6) is technically breaking if anyone is relying on the current `points`-only contract. Do we deprecate or version?
4. Several of these features (capsule, dotfill, hatched-container) are clearly "OpenAI / editorial" idioms. Are we building one skill that covers both styles, or splitting `roughjs-svg-diagrams` into a `system-map` skill + an `editorial-diagrams` skill?

---

*Notes captured while iterating on `docs/visuals/alt/01-product-overview-*.svg`. Will be pruned and rewritten as a formal upstream note once the team agrees on direction.*
