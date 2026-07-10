# chatgpt_send broke on new nested model picker (dynamic trigger label)

**Signal:** ChatGPT shipped a two-axis nested composer picker. `chatgpt_send` (the "ChatGPT upload" flow downstream teams rely on) matched the model submenu trigger by literal text `"GPT-5.6 Sol"` and intelligence by literal `"Pro"`. ChatGPT renders the submenu trigger's label as the *currently-selected* model (e.g. "GPT-5.5"), and the top intelligence tier's label is model-dependent ("Pro" under Sol, "Pro Extended" under 5.5). So selection only worked when the account was already on Sol; any other last-used model threw `model_submenu_not_found` and killed the upload flow.

**Fix applied** (workflows/chatgpt/chatgpt_send.json, s6 + s12, uncommitted):
- Match the model submenu trigger structurally — the sole `[role=menu] [role=menuitem][aria-haspopup=menu]` — never by label. Select the stable radio text `GPT-5.6 Sol` inside the submenu.
- Keep model-first / intelligence-second ordering (the "Pro" tier label only appears once Sol is active).
- `realClick` now fires pointer hover events so the radix submenu opens reliably.
- Radios still commit only on isTrusted events → `click_element use_cdp:true` retained on the stamped radios.

**Validation:** DOM-verified end-to-end from a GPT-5.5 start using trusted clicks (CDP-equivalent), no message sent — s6 stamped Sol despite "GPT-5.5" trigger, commit flipped pill, s9 stamped "Pro", s12 reopened both menus → VERIFY_PASSED. Full CLI send-path (real s7 CDP click + attachment upload) NOT yet run live; worth one benign end-to-end send before relying on it downstream.

**Takeaway for future picker breaks:** never match ChatGPT menu items by labels that echo current state (the submenu trigger, effort pills). Match structurally (role + aria-haspopup) and assert against stable inner radio text. See memory `project_chatgpt_nested_picker_2026_07_10`.
