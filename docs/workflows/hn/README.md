---
title: Hacker News Workflows
slug: /workflows/hn/
sidebar:
  label: Hacker News
---

# Hacker News Workflows

The HN pack is built around one simple rule: draft first, then make a conscious choice to post.

## Workflow Pack

| Workflow | Purpose | Key Params |
| --- | --- | --- |
| `hn-draft-submit-link-post.json` | Fill the HN submit form and stop before posting. | `post_title`, `post_url`, `post_text` |
| `hn-submit-link-post.json` | Fill the HN submit form, pause for approval, then submit. | `post_title`, `post_url`, `post_text` |
| `hn-read-then-draft-comment-on-item-url.json` | Open an item page, scroll for context, and leave a root-comment draft. | `item_url`, `comment_text` |
| `hn-read-then-comment-on-item-url.json` | Open an item page, scroll for context, pause for approval, then submit a root comment. | `item_url`, `comment_text` |
| `hn-draft-reply-to-comment-id.json` | Open the reply form for one comment id and leave a draft. | `comment_id`, `comment_text` |
| `hn-reply-to-comment-id.json` | Open the reply form for one comment id, pause for approval, then submit. | `comment_id`, `comment_text` |
| `hn-first-item-draft-comment.json` | Low-risk sandbox: draft a comment on the first front-page item without sending. | `comment_text` |

## Operator Notes

- These workflows assume the active Chrome profile is already authenticated to `news.ycombinator.com`.
- Every real write workflow pauses on `request_user_intervention` with `approval_mode: "ask_user"` and `continue_on_timeout: false`.
- Keep titles literal and body text short. HN is allergic to marketing fluff for good reason.
- Do not paste sponsor blocks, timestamps, or a pile of podcast links into `post_text`.
- The live comment and reply workflows now scan the visible thread and fail before submit if the same account already posted the same normalized text.

## Example

Reasonable submission payload for the Jensen Huang interview:

- `post_title`: `Dwarkesh Patel interviews Jensen Huang on TPUs, supply chains, China, and hyperscalers`
- `post_url`: `https://www.youtube.com/watch?v=Hrbq66XqtCo`
- `post_text`: `Dwarkesh interviews Jensen on TPU competition, Nvidia's supply-chain moat, China chip policy, why Nvidia has not become a hyperscaler, and how it thinks about investments.`

Draft it first:

```bash
make run W=workflows/hn/hn-draft-submit-link-post.json PARAMS='--param post_title="Dwarkesh Patel interviews Jensen Huang on TPUs, supply chains, China, and hyperscalers" --param post_url="https://www.youtube.com/watch?v=Hrbq66XqtCo" --param post_text="Dwarkesh interviews Jensen on TPU competition, Nvidia'\''s supply-chain moat, China chip policy, why Nvidia has not become a hyperscaler, and how it thinks about investments."'
```
