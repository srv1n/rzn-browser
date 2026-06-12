# Hacker News Workflows

Three workflows cover the full HN write surface. Each one drafts the form, hard-fails on duplicate submissions where applicable, pauses on an explicit operator approval gate, and only posts after the operator clicks Continue.

## Requirements
- You must already be logged in to Hacker News in the workflow tab/profile. None of these workflows handle login.
- Keep rate limits, dedupe, and title quality on the upstream caller's side. HN punishes lazy spam faster than the workflow runner will.

## Operator guidance
- To **dry-run** any workflow (fill the form but do not post), let it run to the approval gate and click **Stop** in the extension UI. The form stays filled in the browser tab so you can inspect copy or selectors. There are no separate "draft-only" workflows — the gate is the single point of control.
- For link posts, keep `post_text` short and factual. Do not dump sponsor copy, timestamp blocks, or every platform link into HN.
- Submit a canonical URL once. If the link may already be on HN, search before submitting.
- Comments and replies should add context, not just repeat the parent or the linked content.
- The comment and reply workflows hard-fail before drafting if the logged-in account already has the same normalised text on the page.

## Workflows

### `hn-submit-link-post.json`
Submit a link post. Approval-gated.
- **id**: `hn_submit_link_post_v1`
- **params**: `post_title` (required), `post_url` (required), `post_text` (required, pass `""` for none)
- **example**:
  ```
  rzn-browser run hn submit-link-post \
    --param post_title="Show HN: rzn-browser" \
    --param post_url="https://github.com/example/rzn-browser" \
    --param post_text=""
  ```

### `hn-submit-comment.json`
Post a root-level comment on a chosen HN item. If `item_url` is omitted, comments on whatever is currently the first item on the front page. Includes a dedupe guard against the logged-in user's own prior comments on the thread.
- **id**: `hn_submit_comment_v1`
- **params**: `comment_text` (required), `item_url` (optional)
- **examples**:
  ```
  rzn-browser run hn submit-comment \
    --param item_url="https://news.ycombinator.com/item?id=12345678" \
    --param comment_text="Section 4 misses cold-start cost — adding it flips the latency story."

  # First front-page item (no item_url):
  rzn-browser run hn submit-comment \
    --param comment_text="Counterpoint: the benchmark in Table 2 excludes warm-cache reads."
  ```

### `hn-submit-reply.json`
Reply to a specific comment by id. Approval-gated, with a dedupe guard against the logged-in user's own prior replies on the parent thread.
- **id**: `hn_submit_reply_v1`
- **params**: `comment_id` (required, numeric), `comment_text` (required)
- **example**:
  ```
  rzn-browser run hn submit-reply \
    --param comment_id="12345678" \
    --param comment_text="Agreed on cold-start; the gap is actually in Section 4 not Section 5."
  ```

## Implementation notes
- All three workflows use plain `execute_javascript` (`world: "main"`) for verification, dedupe checks, and (for `submit-comment`) item-URL resolution. Final submits use the standard non-CDP `click_element` path because HN forms are server-rendered submit controls.
- `submit-comment` resolves the target item entirely via JS: it lands on the front page first, then either redirects to the supplied `item_url` (after origin-validating it against `news.ycombinator.com`) or extracts the first `tr.athing` item link and redirects there. This avoids needing a separate "first item" workflow.
- The approval gate is `request_user_intervention` with `continue_on_timeout: false` and `timeout_ms: 2147483647`, so the workflow blocks indefinitely until the operator decides.
