# chatgpt: Projects post + retrieve — added & validated (2026-06-01)

Projects are gizmos with id prefix `g-p-`.

## Discovery (read-only)
- `/backend-api/gizmos/snorlax/sidebar` → projects (id, short_url, name).
- `/backend-api/gizmos/{g-p-id}/conversations` → that project's chats (id=chat_id, title, snippet, gizmo_id).
- Project page: `https://chatgpt.com/g/{short_url}/project` — loads the normal composer; sending scopes the chat to the project (URL → `/g/<short>/c/<id>`).

## Shipped
- New `workflows/chatgpt/chatgpt_projects.json` (`chatgpt/projects`): `mode=list` (projects only) + `mode=conversations`+`project_id` (chat list). list mode does NOT embed chat objects — the result bridge flattens depth-≥3 objects to `"[Object]"` (same quirk that hit read attachments); chat_ids come from `mode=conversations`.
- `chatgpt_send` `project_id` param: s3 routing opens the project page when set (chat_id still wins). s17 fixed `^\/c\/` → `\/c\/` (matches `/g/<short>/c/<id>`) and now also returns `project`.

## Validated (live)
- `chatgpt projects` → 5 projects with urls.
- `chatgpt projects mode=conversations project_id=<Rzn>` → 4 chats.
- `chatgpt send project_id=<Rzn> model_slug=Instant` → chat created at `/g/…-rzn/c/6a1d6959…`, reply `RZN-PROJ-OK-5573`, and the new chat appears in the project's conversation list (membership confirmed).

## Notes
- No engine rebuild this round (JSON-only). Earlier this session: read→zip attachments, send menu/Thinking fix, engine attachment-downloader (those needed the rebuild already installed).
- Retrieve-from-project = `chatgpt_projects mode=conversations` → `chatgpt_read` each chat_id.
