# Website Visuals v1

Renderer-backed website assets for `rzn-browser`, authored with the `website-visuals` skill.

This pack is intentionally separate from `docs/visuals/`, which holds hand-authored technical diagrams. These assets are editorial website surfaces: share card, comparison matrix, and decision/comparison explainers.

## Asset Set

| asset id | kind | point |
| --- | --- | --- |
| `rzn_browser_local_chrome_og_v1` | `og_card` | The product thesis in one frame |
| `rzn_browser_compete_matrix_v1` | `compare_matrix` | Where RZN fits against adjacent tools |
| `rzn_browser_choose_rzn_decision_v1` | `decision_diagram` | When RZN is the right answer |
| `rzn_browser_modes_matrix_v1` | `compare_matrix` | Workflow mode vs agent mode on the same runtime |

## Render

```bash
python3 /Users/sarav/Downloads/play/editops-skills/skills/website-visuals/bin/website-visuals render-batch \
  --manifest /Users/sarav/Downloads/side/rzn/rzn-browser/docs/visuals/website-v1/manifests/website_v1.batch.json \
  --out /Users/sarav/Downloads/side/rzn/rzn-browser/docs/visuals/website-v1/renders
```

## Source Notes

- Narrative source: `README.md`
- Product framing: `docs/README_VISUAL_BRIEFS.md`
- These are not UI screenshots. No screenshot-board asset was added because the repo does not currently include a strong product UI capture worth annotating.
