# chatgpt: attachment download + Thinking effort — both fixed & validated (2026-06-01)

Follow-up to the model-menu fix. User asked to fix everything, prefer CSP over CDP.

## 1) chatgpt_read — retrieve ALL attachments (was: only first landed)
Chrome's "multiple automatic downloads" block dropped every anchor-click download after the first.
- Tried CLI `--download-dir` (reqwest): ChatGPT file URLs (`/backend-api/estuary/content?...&sig=`) are **cookie-bound**, reqwest is cookieless → HTTP error.
- Tried extension `chrome.downloads`: unreachable — workflow `execute_javascript` is forced to MAIN world (no `chrome.runtime`).
- **Shipped:** s2 `fetch(url,{credentials:'include'})` each attachment (cookie auth, same-origin), builds a STORE-method ZIP in pure JS, triggers ONE anchor download `chatgpt-attachments-<chat_id>.zip`. One download ⇒ no gate. Pure CSP, no CDP.
- Validated on chat `6a1d2eb8-…`: 4 files, `unzip -t` clean, 1.21MB. Payload adds `attachment_urls` + `attachments_zip`.

## 2) chatgpt_send — Thinking effort on the new flat menu
New menu rows: Instant / Medium / High / Extra High / Pro Extended (model+effort = one row).
- s6 builds a combined `desiredLabel` (Thinking + {light/standard→Medium, extended→High, heavy→Extra High}; new labels accepted directly) and stamps that row; s9/s10 effort-submenu logic neutered; s12 verifies checked row == desiredLabel.
- Validated: `model_slug=Thinking model_effort=Heavy` → committed `gpt-5-5-thinking`, replied "VALIDATED". Instant (`gpt-5-5`) + default Pro (Pro Extended) correct by construction.

## Engine change (general, reusable)
`crates/rzn_browser/src/main.rs` `download_payload_assets`: nested asset lookup (root/output/output.result/result) + `attachment_urls` object arrays → `attachments/` with real filenames. NOTE reqwest is cookieless (cookieless/pre-signed URLs only). Rebuilt release; `~/.local/bin/rzn-browser` symlinks to it.

## Discipline
Per user feedback: batched all edits offline (one DOM probe drove the whole menu fix), only 4 live ChatGPT runs total this turn (1 read + send-Thinking + its readback + the earlier failed reqwest attempt). No spamming.

## Still open
- `chatgpt_images_download` uses the same per-image anchor-click → also gate-limited for >1 image. Apply the same zip-or-extension approach if multi-image saving matters.
- Projects (post/retrieve into a ChatGPT Project) still unbuilt — deferred by user.
