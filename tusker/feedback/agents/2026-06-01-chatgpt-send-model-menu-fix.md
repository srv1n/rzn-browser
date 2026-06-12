# chatgpt_send model-select broke on new flat menu — fixed + validated (2026-06-01)

## Signal
`rzn-browser run chatgpt send --param model_slug=Instant` failed at s12:
`model_selection_verify_failed: wanted Instant; got ?`

## Root cause
ChatGPT changed the composer model picker:
- `[role=menuitemradio]` rows no longer have `data-testid="model-switcher-gpt-*"` (now `null`).
- Menu is now a flat combined-label list: `Instant / Medium / High / Extra High / Pro Extended` (no Model•Effort split, no effort submenu).

s6 (pick) / s9 (effort) / s12 (verify) all filtered items by the dead testid → selection silently no-op'd, verify hard-failed.

## Fix (workflows/chatgpt/chatgpt_send.json, 3 lines)
- Item filter `data-testid startsWith model-switcher-gpt-` → `filter(isVisible)` (s6/s9/s12).
- s12 verify made tolerant of combined labels (exact OR first-token OR substring; effort matched inside label).

## Validated (single live run each — did not spam)
- Read: chat `6a1d2eb8-…` → 1 real user turn + attachment list retrieved OK.
- Send: Instant new-chat + sample.txt upload → assistant read file back verbatim (magic tokens ZEBRA-7741-QUOKKA / 84219 present). model_slug returned `gpt-5-5` (= Instant).

## Open follow-ups (NOT done)
1. **chatgpt_read multi-download**: only the FIRST attachment lands (Chrome multi-auto-download gate on page anchor-clicks). 4 triggered, only the 957KB zip saved; 3 PNGs lost. Move to `chrome.downloads.download` (background) or sequence with completion waits.
2. **Thinking effort remap**: new effort names Medium/High/Extra High (≠ old Light/Standard/Extended/Heavy); s9/s10 effort-submenu logic is obsolete (model+effort is one row). `model_slug=Thinking + model_effort` needs a combined-label map.
3. **Projects** post/retrieve: not attempted (deferred by request).
