# Diagram System — Design Brief

**Status:** Draft v0.1 — the foundation document. The rest of the work compiles to this.
**Owner:** RZN visuals
**Last updated:** 2026-04-20

This brief defines what we're building, why it works, and the decisions that everything downstream inherits. Implementation is deliberately deferred until this document is signed off — the cost of building on a wrong foundation is much higher than the cost of writing this carefully.

---

## 0. The thesis

> A diagram is a *cognitive prosthetic*. Its job is to offload the viewer's working memory so they can reason about a system spatially, instead of holding it as a list of facts.

Everything else in this brief follows from that thesis. If a design choice doesn't help the viewer build the right mental model in the first 5 seconds and refine it across the next 30, we don't make the choice — no matter how clever or beautiful.

---

## 1. Scope

### 1.1 What this system produces

Explanation-first technical diagrams that:

- Make the central idea legible in **5 seconds** (preattentive read — Gestalt grouping does the work)
- Reward **30 seconds** of attention with the next layer of detail (labels, captions, secondary structure)
- Stay coherent at three target densities — thumbnail (300 px wide), inline (800 px), full (1600 px)
- Render **deterministically** across SVG consumers (Chrome, Safari, librsvg, Inkscape, Figma import, slide tools)
- Are authored as **information**, not as visuals — the design system supplies all visual decisions

### 1.2 What this system does *not* produce

We commit to being bad at these so we can be excellent at the above.

- **Data visualizations** (charts, plots, statistical graphics) — different problem space, different tools (D3, Vega, Observable Plot)
- **Marketing illustration** (conceptual collages, hero art, mascot graphics) — design tools, not generative
- **Live / interactive diagrams** — different stack (React + canvas/WebGL)
- **Hand-sketched aesthetic** — the existing `roughjs-svg-diagrams` skill remains for that idiom; this system is the *editorial* sibling

### 1.3 Diagram kinds (the only five we promise to do well)

| Kind | Reading direction | Examples | Idiom signature |
|---|---|---|---|
| **System map** | Spatial, no fixed direction | Service topology, component overview | Containers grouping nodes, connectors as relationships |
| **Flow** | Left → right or top → bottom | Action escalation, request lifecycle | Linear sequence with optional branches |
| **Comparison** | Two parallel tracks | Workflow vs agent mode | Side-by-side, shared elements convergent |
| **Sequence** | Top → bottom across actors | API handshake, sync protocol | Vertical lifelines, time advances downward |
| **Hierarchy** | Top → bottom containment | Org chart, taxonomy | Tree, nested boxes |

Anything outside these five is either rejected or expressed as a combination of these (e.g. a "swimlane flow" is a *flow* with *hierarchy* groups).

---

## 2. The four pillars (literature-grounded principles)

Each pillar has a plain-language definition, the principle we commit to, the literature it rests on, and the concrete implication for the system.

### 2.1 Visual hierarchy — the eye's reading order encodes meaning

**Principle:** The viewer's eye lands on elements in a deterministic order. That order encodes meaning whether we want it to or not. Therefore the highest-importance information must occupy the strongest visual rank, and lower-importance must visibly recede.

**Literature:**
- Bertin, *Sémiologie graphique* (1967) — the seven visual variables: position, size, value, texture, color hue, orientation, shape. Each has a different power for encoding ordered, categorical, and quantitative information.
- Cleveland & McGill, *"Graphical Perception"* (1984) — the empirical ranking of which encodings the eye decodes most accurately: position on common scale > position on identical non-aligned scales > length > angle > area > volume > color saturation > color hue.
- Wertheimer / Koffka — Gestalt principles (proximity, similarity, continuity, closure, common fate, figure/ground). These determine what the viewer *groups* preattentively, before any conscious reading.

**Commitment:**
- Importance is encoded in this priority order: **position → size → value → color hue → texture → shape → orientation**
- Tokens encode discrete emphasis levels (`display`, `primary`, `secondary`, `tertiary`, `meta`) not arbitrary sizes
- The dominant reading path through any diagram is single. If two paths compete, one of them is wrong.

### 2.2 Scaling — readable at any size, on any device

**Principle:** The diagram works at the smallest size we'd ever publish it (a thumbnail) and the largest (full-screen on a 4K display). The lowest density is the floor, not an afterthought.

**Literature:**
- CSS Flexbox spec § "Flex layout algorithm" — the formal model of intrinsic vs available size, main vs cross axis, growth/shrink
- Material Design density guidance / Apple HIG — multi-density rendering practice
- The "1.5 px stroke problem" — odd-pixel strokes blur on raster export when the coordinate system isn't pixel-aligned. Cleveland & McGill's "graphical integrity" applies to rendering as well as encoding.

**Commitment:**
- Minimum text size is **11 px @ 1600 px canvas** (≈ 6.6 px @ thumbnail). Below that text is decorative, not informational.
- Minimum stroke width is **1.2 px**. Below that, anti-aliasing eats the line.
- All coordinates are integer multiples of the **8 px base unit** so pixel-snapped raster output is clean.
- Text and stroke widths scale *with the canvas*, not independently — a single `density` value defines the whole scale.

### 2.3 Rendering — same input, same output

**Principle:** The diagram is a build artifact, not a hand-crafted asset. Same input bytes produce same output bytes. This unlocks code review on visuals, regression tests, and trustable diffs.

**Literature:**
- SVG 1.1/2 spec — coordinate system, text rendering rules, pattern semantics
- Resvg vs librsvg vs Chrome behavioral differences — well-documented in the resvg test suite
- Font fallback / system-font-stack pitfalls — inevitable drift if we rely on system fonts

**Commitment:**
- **Bundled fonts only.** Inter + IBM Plex Mono (+ optional editorial serif) embedded or web-loaded, never relying on system stack
- All IDs in output SVG are **content-derived** (hash of input), not random or sequential, so output is reproducible
- All numeric output is rounded to **0.5 px** precision; coordinates that the engine produces are rounded to **1 px**
- We test rendering across resvg, Chrome, and Safari on every reference diagram. Disagreements are bugs in our SVG, not in the renderer.

### 2.4 Human understanding — the diagram teaches

**Principle:** A diagram succeeds when the viewer can reproduce the structure from memory after one viewing. It fails when they can describe what they saw but not how the parts relate.

**Literature:**
- Tamara Munzner, *Visualization Analysis & Design* (2014), esp. ch. 5 (Marks & Channels) — the modern textbook framework
- Colin Ware, *Information Visualization: Perception for Design* — preattentive processing (color, motion, orientation, size detected in <250 ms before conscious search)
- Barbara Tversky, *Mind in Motion* — diagrams as spatial scaffolds for non-spatial reasoning; the "spatial schema" concept
- Don Norman, *Design of Everyday Things* — affordances and signifiers; what does this arrow *invite* the viewer to think?
- Edward Tufte, *Visual Display of Quantitative Information* — data-ink ratio; every drop of ink should encode information
- Müller-Brockmann, *Grid Systems in Graphic Design* — the column grid as the substrate of compositional clarity

**Commitment:**
- **Chunk by Gestalt.** Group related nodes by proximity and a shared container; never rely on color alone to indicate grouping.
- **Limit working memory.** No more than **7 ± 2** visible primary nodes in a single region. If we have more, we add a containment level.
- **Single dominant reading path.** The diagram tells one story per view. Secondary structure (annotations, footnotes) is visibly secondary — same diagram, different rank.
- **Affordance-true arrows.** An arrowhead means "the thing at the head receives." A double-headed arrow means "two-way coupling." A dashed line means "structural relationship, no flow." We never use these inconsistently.
- **Earn every drop of ink.** Decorative strokes, gradients, drop-shadows — banned unless they encode information. Tufte's data-ink ratio applies to system diagrams as strictly as to charts.

---

## 3. Visual grammar — Bertin's variables, mapped

The discipline that prevents drift. Each visual variable is *reserved* for a specific meaning. Once reserved, that variable is unavailable for ad-hoc use.

| Variable | Bertin's strength for ordered data | Our reserved use | Forbidden uses |
|---|---|---|---|
| **Position** | Strongest | Topology — which group, which row, reading order | Decoration |
| **Size** | Very strong | Importance / emphasis level | Distinguishing categories |
| **Value** (lightness) | Strong | Emphasis within a category, or selected/active state | Distinguishing categories with similar emphasis |
| **Color hue** | Categorical only | Node-kind (max 4 hues per diagram) | Encoding magnitude |
| **Texture** | Categorical | Trust boundary / containment kind (hatch = sandboxed, dot-fill = component, plain = passive) | Decoration |
| **Orientation** | Weak | Arrow direction only | Distinguishing categories |
| **Shape** | Categorical | Node silhouette → semantic kind (capsule = component, card = artifact, container = grouping, badge = label) | Decoration |

This is the discipline that makes diagrams legible. There are no "let's use color here for variety" decisions.

---

## 4. Design system — tokens

All visual choices live in one place. Authoring uses semantic names; literal values appear nowhere in diagram source.

### 4.1 Spacing scale (base 8)

```
xxs   4 px    hairline
xs    8 px    intra-token gap
s    16 px    tight padding
m    24 px    default padding
l    32 px    container padding (the standard)
xl   48 px    section gap
xxl  64 px    column gutter (default)
xxxl 96 px    major break / poster margin
```

Every gap, padding, and stroke offset uses one of these eight values. No others.

### 4.2 Type scale (base 14, ratio 1.2 — minor third)

```
meta       11 px / 16 lh   IBM Plex Mono, weight 700, letter-spacing 0.15em
caption    12 px / 18 lh   Inter, weight 500
body       14 px / 22 lh   Inter, weight 400 (regular) / 600 (emphasis)
node-title 17 px / 22 lh   Inter, weight 700, letter-spacing -0.01em
section    20 px / 26 lh   Inter, weight 600
subhead    24 px / 30 lh   Inter, weight 500 (often italic in editorial idiom)
headline   32 px / 38 lh   Inter, weight 600, letter-spacing -0.02em
display    40 px / 46 lh   Inter, weight 600, letter-spacing -0.03em
```

Type pair: **Inter** (sans, primary) + **IBM Plex Mono** (mono, for meta and code). Editorial idiom may opt into a serif (Tiempos / Source Serif / Charter) for headline + subhead only.

Minimum legible text floor: **11 px**. Anything smaller is decorative and may not carry information.

### 4.3 Color tokens (semantic)

Light mode (v1). Dark mode tokens defined in parallel for v2.

```
surface       #FAFAF7   warm canvas (default)
surface-cool  #FFFFFF   cool canvas (alternate)
surface-2     #F4F1EA   sunken / container fill
surface-3     #E8E4D9   inset / depth-2

ink           #1A1A17   primary text & primary strokes
ink-2         #3A352D   secondary text (passes AA on surface)
ink-3         #6F6A60   tertiary / meta — restricted use; must pass AA on surface

boundary      #1A1A17 @ 1.0    primary node strokes & rules
boundary-2    #1A1A17 @ 0.25   secondary rules / dividers

accent        #3F6F1A   sage green — RZN-authored channels only
accent-tint   #F4FAEB   accent fills (10% saturation of accent)
accent-deep   #26500D   accent text on accent-tint

flow          #1A4596   blue — external runtime / target surface
flow-tint     #E8F0FE   flow fills

warn          #C8A83A   caution / escalation / fallback
warn-tint     #FEF7D6   warn fills

danger        #B43A2C   reserved — error / destructive (rare in product diagrams)
```

**Hard rule:** any single diagram uses at most **4 semantic-color slots** (one of which must be `ink` for text, leaving 3 for category encoding). More than that and the diagram has lost its dominant reading path.

### 4.4 Shape vocabulary

| Shape | Silhouette | Semantic | Notes |
|---|---|---|---|
| **capsule** | rounded rectangle with `r = h/2` | runtime component / agent | The default workhorse |
| **capsule-cylinder** | capsule + 2 vertical end-bars inset 18 px | executor / has internal state | Reads as "machine" |
| **card** | rounded rect, `r = 8` | passive artifact / external surface | Lower visual weight than capsule |
| **container** | rounded rect, `r = 16-20`, dashed or hatched fill | grouping / trust boundary | Holds other nodes |
| **badge** | small capsule, all-caps mono inside | label / tag | Used as a sticker, not as a node |
| **diamond** | square rotated 45° | decision / branch | Used sparingly, only in `flow` kind |

### 4.5 Edge vocabulary

| Edge kind | Stroke | Color | Arrow | Use |
|---|---|---|---|---|
| **structural** | 1.0 px, dashed (4 4) | `boundary-2` | none | Defines topology / containment relationships |
| **flow** | 1.5 px, solid | `ink` | single arrow at head | Directional data or control |
| **feedback** | 1.5 px, solid | `accent` | single arrow at head | Emphasized loop / RZN-authored channel |
| **escalation** | 1.5 px, solid | `warn` | single arrow at head | Fallback / caution path |
| **bidirectional** | 1.5 px, solid | `ink` | arrows at both ends | Two-way coupling |

Routing default is **orthogonal**. Smooth Bézier is opt-in per-diagram for editorial idiom.

---

## 5. Information model

The author writes only the data. No coordinates, no colors, no sizes.

```ts
type Diagram = {
  meta: {
    slug: string
    title: string
    subtitle?: string
    kind: 'system-map' | 'flow' | 'comparison' | 'sequence' | 'hierarchy'
    density?: 'thumbnail' | 'inline' | 'full'   // default 'inline'
    idiom?: 'system' | 'editorial'              // default 'system'
  }
  groups?: Group[]
  nodes: Node[]
  edges: Edge[]
  annotations?: Annotation[]
}

type Node = {
  id: string
  label: string                 // primary text
  caption?: string              // secondary text (one line)
  kind: NodeKind                // → shape, color, texture
  group?: string                // group id; nests inside group
  emphasis?: 'primary' | 'secondary' | 'tertiary'  // default 'secondary'
  hint?: string                 // routing hint, e.g. 'left-of:other-node-id'
}

type Edge = {
  id?: string                   // auto-generated if omitted
  from: string                  // node id (anchor resolved by router)
  to: string
  kind: EdgeKind                // → stroke, color, arrow
  label?: string                // optional inline label
}

type Group = {
  id: string
  label?: string
  kind: 'lane' | 'boundary' | 'cluster'
  members: string[]             // node ids
}

type Annotation = {
  text: string
  attach: { kind: 'node' | 'edge', id: string, side?: 'top' | 'bottom' | 'left' | 'right' }
}

type NodeKind =
  | 'runtime-component'    // capsule, ink + accent-tint or surface-2
  | 'executor'             // capsule-cylinder
  | 'passive-artifact'     // card
  | 'external-surface'     // capsule, flow palette
  | 'actor'                // card with person glyph
  | 'data'                 // card with data glyph
  | 'decision'             // diamond
  | 'annotation'           // badge

type EdgeKind = 'structural' | 'flow' | 'feedback' | 'escalation' | 'bidirectional'
```

**Authoring discipline:** if you find yourself wanting a property that's not in this schema (e.g. `node.color`, `node.x`), the design system is missing a semantic — extend the *kind* or *emphasis* enum, not the schema's expressivity.

---

## 6. Layout invariants

- **Grid:** 8 px base unit. Every coordinate `x`, `y`, `w`, `h` produced by the engine is a multiple of 8.
- **Columns:** 12-column grid for 1600 px canvas (80 px outer margin, 32 px gutter, 96 px column). Smaller canvases scale proportionally.
- **Rows:** baseline grid of 8 px. Type baselines snap to it.
- **Padding:** every container has the same inner padding on all four sides (default `l` = 32 px). No exceptions for "this side has more headroom."
- **Pill stacking:** pills inside a container are evenly distributed along the cross axis with `gap = m` (24 px) between siblings. The first and last pill are inset by container padding (`l` = 32 px).
- **Edge endpoints:** edges enter or exit a node *only* at one of the four named anchors (`top-mid`, `right-mid`, `bottom-mid`, `left-mid`) — never from interior, never within 8 px of a corner.
- **Curves:** if `idiom: 'editorial'` opts into smooth routing, control points are computed from anchor normals (tangent at the entry/exit). Hand-authored control points are forbidden.
- **Z-order:** containers → groups → edges → nodes → labels-on-edges (with backplates) → annotations → meta/footer. Renderer enforces.

---

## 7. Validation rules

Every rule **fails the build**. There are no warnings — a diagram either passes all checks and exports, or it doesn't export.

### 7.1 Mechanical
- No node bounding box overlaps another node bounding box (same level)
- No edge endpoint inside any node's interior
- No edge endpoint floating > 0 px from its declared anchor
- No edge segment crosses an unrelated node's bounding box
- All elements within canvas bounds
- All coordinates are integer multiples of 8

### 7.2 Typographic
- Every text string fits its container (computed from font metrics, not estimated)
- Every text passes WCAG AA contrast against its actual background (4.5:1 for body, 3:1 for ≥ 18 px or ≥ 14 px bold)
- No text bounding box overlaps unrelated text
- No text overlaps an edge unless a backplate is present and covers the intersection

### 7.3 Semantic
- Every edge `from`/`to` resolves to a defined node id
- Every group `members` entry resolves to a defined node id
- ≤ 4 semantic color slots used in a single diagram
- ≤ 7 primary-emphasis nodes in any one visual region (Gestalt limit)
- Visual hierarchy matches information hierarchy: any node with `emphasis: 'primary'` is at least one size step larger than any sibling with lower emphasis
- Exactly one dominant reading path detectable (heuristic: a node graph traversal from the visually-heaviest entry node reaches all primary nodes without ambiguity)

### 7.4 Determinism
- All output IDs are content-derived (hash of input subset), not random or counter-based
- All font specs reference bundled fonts only (no system-font fallback)
- All numeric output rounded to 0.5 px precision

---

## 8. Architecture — six layers, typed contracts

```
┌──────────────────────────────────────────────────────────┐
│  Author writes:  diagram.ts  or  diagram.json           │
│                       │                                  │
│                       ▼   Diagram (information model)    │
│  ┌────────────────────────────────────────────────────┐  │
│  │ 1. Design System Resolver                          │  │
│  │    Maps kind → shape, color, texture, type spec    │  │
│  └────────────────────────────────────────────────────┘  │
│                       │                                  │
│                       ▼   StyledScene (intent + style)   │
│  ┌────────────────────────────────────────────────────┐  │
│  │ 2. Layout Engine — Pass 1: Measure                 │  │
│  │    Bottom-up. Text → node intrinsic w,h →          │  │
│  │    container intrinsic w,h.                        │  │
│  └────────────────────────────────────────────────────┘  │
│                       │                                  │
│                       ▼   MeasuredScene (sizes only)     │
│  ┌────────────────────────────────────────────────────┐  │
│  │ 3. Layout Engine — Pass 2: Place                   │  │
│  │    Top-down. Containers claim grid slots; children │  │
│  │    distributed inside content box.                 │  │
│  └────────────────────────────────────────────────────┘  │
│                       │                                  │
│                       ▼   PlacedScene (nodes have x,y)   │
│  ┌────────────────────────────────────────────────────┐  │
│  │ 4. Layout Engine — Pass 3: Route                   │  │
│  │    Edges resolved against final coordinates.       │  │
│  │    Channel-based orthogonal or tangent Bézier.     │  │
│  └────────────────────────────────────────────────────┘  │
│                       │                                  │
│                       ▼   RoutedScene (edges have paths) │
│  ┌────────────────────────────────────────────────────┐  │
│  │ 5. Validator                                       │  │
│  │    Mechanical + typographic + semantic + determ.   │  │
│  │    Pass → continue. Fail → throw with diagnostics. │  │
│  └────────────────────────────────────────────────────┘  │
│                       │                                  │
│                       ▼   ValidatedScene                 │
│  ┌────────────────────────────────────────────────────┐  │
│  │ 6. Renderer                                        │  │
│  │    Scene → SVG (deterministic IDs, pixel-snapped)  │  │
│  │    SVG → PNG/PDF via resvg                         │  │
│  └────────────────────────────────────────────────────┘  │
│                       │                                  │
│                       ▼   svg + png + pdf                │
└──────────────────────────────────────────────────────────┘
```

Each arrow is a typed contract. Each layer is unit-testable in isolation. Each layer can be replaced (e.g. swap our hand-rolled placer for `dagre`) without touching the others.

### 8.1 Layer responsibilities, in one sentence each

1. **Design System Resolver:** turns semantic intent into concrete style tokens. No layout knowledge.
2. **Measure:** computes intrinsic `(w, h)` for every node from text metrics + style. No placement knowledge.
3. **Place:** assigns `(x, y)` to every node by resolving grid columns and container content boxes. No edge knowledge.
4. **Route:** turns each edge `from → to` into an explicit polyline or curve, anchored to node midpoints, avoiding obstacles via channel routing.
5. **Validator:** enforces every rule in §7, with actionable diagnostics on failure.
6. **Renderer:** emits deterministic SVG and rasters. No semantic decisions.

---

## 9. Success criteria — how we know this is working

These are the objective tests. The system is "done" (v1) when all seven pass.

1. **Authoring speed:** a new diagram from scratch (information model only) takes a domain expert <30 minutes.
2. **Determinism:** the same `.diagram.json` rendered twice produces byte-identical SVG. Verified in CI.
3. **Accessibility:** every reference diagram passes WCAG AA contrast at every density tested. Verified by validator.
4. **Comprehension:** in a 5-second exposure test, a non-author can correctly identify the diagram's central claim. (Tested informally on 3 colleagues per reference diagram.)
5. **Reproducibility:** a non-author can reproduce the topology (nodes + connections) from memory after one 30-second viewing.
6. **Restyle locality:** changing any visual property across all diagrams (e.g. accent color, font pair) requires editing only `tokens.ts`. Zero diagram source changes.
7. **Reference suite passes clean:** the 8 reference diagrams render with zero validator errors and zero manual coordinate tuning.

---

## 10. Phased plan

| Phase | Deliverable | Effort | Done = |
|---|---|---|---|
| **0. Brief sign-off** | This document approved | 0 | We agree on every commitment in §1–§9 |
| **1. Tokens** | `tokens.ts` + `DESIGN_SYSTEM.md` documenting every token's rationale | ~3 days | Every value in §4 typed and traceable to a literature source |
| **2. Information model** | `schema.ts` + Zod runtime validator + 5 reference diagrams authored as pure information | ~2 days | Schema validates; reference diagrams live as `.diagram.json` |
| **3. Design System Resolver** | Layer 1 of architecture | ~2 days | `Diagram → StyledScene` pure function, fully tested |
| **4. Measure pass** | Layer 2: text metrics, node intrinsic sizing | ~3 days | `StyledScene → MeasuredScene`, pixel-accurate against rendered text |
| **5. Place pass** | Layer 3: grid placer | ~4 days | `MeasuredScene → PlacedScene` for `system-map` and `flow` kinds |
| **6. Route pass** | Layer 4: orthogonal channel router + tangent Bézier | ~5 days | `PlacedScene → RoutedScene`; passes obstacle-avoidance tests |
| **7. Validator** | Layer 5: all rules in §7 | ~3 days | Failing inputs produce actionable diagnostics |
| **8. Renderer** | Layer 6: SVG + PNG + PDF, deterministic | ~3 days | Byte-identical outputs verified in CI |
| **9. Reference suite** | Rebuild the 4 RZN diagrams + 4 more covering each kind | ~5 days | All 8 pass validator with zero manual tuning |
| **10. Upstream story** | Replace `UPSTREAM_SKILL_NOTES.md` draft with the formal note | ~1 day | Note ready to send |

**~31 working days.** ~6 weeks calendar at sustainable pace, ~4 weeks if focused. Phases 0–2 (~5 days) are the highest-leverage and gate everything else.

---

## 11. Open questions (decide later, not now)

These are deliberately not decided in v0.1. Each will be resolved when the prior layers force the answer.

- **Authoring format** — pure JSON vs typed TS DSL vs a mini YAML sublanguage? *Lean: typed TS, with JSON as the serialized form.*
- **Hand-rolled grid placer vs lift `dagre`** — decide after building the trivial cases. *Lean: hand-rolled for the 5 declared kinds; dagre as escape hatch for arbitrary topologies.*
- **Smooth-curve routing under obstacle constraints** — tangent-derived control points are simple but ugly when nodes are crowded. May need libavoid-style avoidance. *Defer to phase 6.*
- **Multi-page / poster-format diagrams** — out of scope for v1.
- **Animation** — out of scope for v1; tokens + scene model designed to support it cleanly in v2 (animate on rank, not on coordinates).
- **Dark mode** — not v1, but tokens are structured (`ink`, `surface`, `boundary`, etc.) so only `tokens-dark.ts` needs to exist.
- **Accessibility beyond contrast** — semantic SVG (`<title>`, `<desc>`, `aria-label`), keyboard navigation, screen-reader narration. *Defer to v2.*
- **Internationalization** — non-Latin script support, RTL reading direction. *Defer to v2; design system avoids LTR-only assumptions where cheap.*

---

## 12. Glossary

- **Anchor** — one of the four named midpoints on a node's bounding box (`top-mid`, `right-mid`, `bottom-mid`, `left-mid`) where edges may attach
- **Channel** — a guaranteed-empty horizontal or vertical strip on the grid, used for edge routing
- **Density** — the canvas size class: `thumbnail` (300 px), `inline` (800 px), `full` (1600 px). Determines stroke widths and text sizes via a single multiplier.
- **Idiom** — the visual register: `system` (technical, geometric, mostly orthogonal) or `editorial` (literary, smoother curves, optional serif)
- **Kind** — for nodes, the semantic category that resolves to shape + color + texture; for diagrams, one of the five top-level types
- **Scene** — the in-memory representation of a diagram at any layer (`StyledScene`, `MeasuredScene`, `PlacedScene`, `RoutedScene`, `ValidatedScene`)
- **Slot** — a named position in the grid that a node can occupy (e.g. `runtime-column`, `chrome-column`)
- **Token** — a named design value (`spacing.l`, `color.accent`, `type.node-title`)
- **Visual variable** (Bertin) — one of position, size, value, color hue, texture, orientation, shape

---

## 13. References

### Primary literature
- Bertin, J. *Sémiologie graphique* / *Semiology of Graphics* (1967)
- Cleveland, W. S., & McGill, R. *"Graphical Perception: Theory, Experimentation, and Application to the Development of Graphical Methods"* (JASA 1984)
- Munzner, T. *Visualization Analysis & Design* (2014), esp. ch. 5 (Marks & Channels)
- Tufte, E. *The Visual Display of Quantitative Information* (1983)
- Tversky, B. *Mind in Motion: How Action Shapes Thought* (2019)
- Ware, C. *Information Visualization: Perception for Design* (3rd ed., 2012)
- Norman, D. *The Design of Everyday Things* (rev. ed., 2013)
- Müller-Brockmann, J. *Grid Systems in Graphic Design* (1981)
- Bringhurst, R. *The Elements of Typographic Style* (4th ed.)

### Layout & graph drawing
- Sugiyama, K., Tagawa, S., & Toda, M. *"Methods for Visual Understanding of Hierarchical System Structures"* (IEEE SMC 1981)
- Gansner, E. R., Koutsofios, E., North, S. C., & Vo, K-P. *"A Technique for Drawing Directed Graphs"* (IEEE TSE 1993) — the GraphViz `dot` paper
- Wybrow, M., Marriott, K., & Stuckey, P. J. *"Orthogonal Connector Routing"* (Graph Drawing 2010)
- Badros, G., & Borning, A. *"The Cassowary Linear Arithmetic Constraint Solving Algorithm"* (1999)

### Implementation references
- Yoga (Facebook) — `yoga-layout` on npm
- dagre — Pradeep Chetal, JS port of `dot`
- ELK / elkjs — Eclipse Layout Kernel
- libavoid — Adaptagrams, orthogonal routing
- W3C Flexbox spec — `https://www.w3.org/TR/css-flexbox-1/`
- SVG 2 spec — `https://www.w3.org/TR/SVG2/`

### Adjacent reading (deferred but on the path)
- Wong, D. M. *The Wall Street Journal Guide to Information Graphics* (2010)
- Kosslyn, S. *Graph Design for the Eye and Mind* (2006)
- Few, S. *Show Me the Numbers* (2nd ed., 2012)
- The Penrose project (Stanford) — *"Penrose: From Mathematical Notation to Beautiful Diagrams"* (SIGGRAPH 2020)

---

*End of brief. Sign-off required before Phase 1 begins.*
