# RZN Workflows

This directory contains production-ready browser automation workflows organized by domain.

Installed runtime behavior:

- `make install` copies the shipped catalog into `~/Library/Application Support/RZN/workflows/builtin`
- `rzn-browser workflow pull` refreshes that builtin catalog from the latest release payload
- packaged examples from `examples/browser_automation/` install under the `examples/*` namespace
- preferred deterministic run surface is `rzn-browser run <system> <workflow>`
- `rzn-browser list <system>` keeps catalog output compact
- `rzn-browser list <system> <workflow>` shows detailed help for one workflow, including params and example commands
- `rzn-browser workflow validate <path-or-id>` checks that `required_variables`, placeholders, and `help` metadata line up
- `rzn-browser workflow validate <path-or-id> --write-help` scaffolds or refreshes the top-level `help` block

## Workflow Help Contract

For new workflows, do not ship only the steps. Ship the help too.

Minimum bar for new workflow JSON:

- top-level `name`
- top-level `description`
- sequence `required_variables`
- top-level `help.parameters`
- top-level `help.examples`

Recommended shape:

```json
{
  "name": "ChatGPT: Continue Chat",
  "description": "Open an existing chat and send another prompt.",
  "help": {
    "summary": "Continue an existing chat, send another message, and return the post-send state.",
    "parameters": [
      {
        "name": "chat_id",
        "required": true,
        "shape": "string id",
        "description": "Conversation id from /c/<chat_id>",
        "example": "01234567-89ab-cdef-0123-456789abcdef"
      },
      {
        "name": "message_text",
        "required": true,
        "shape": "text",
        "description": "Prompt text to send",
        "example": "Turn that into a checklist."
      }
    ],
    "examples": [
      {
        "description": "Basic run with required parameters",
        "command": "rzn-browser run chatgpt continue-chat-v1 --param chat_id=\"01234567-89ab-cdef-0123-456789abcdef\" --param message_text=\"Turn that into a checklist.\""
      }
    ],
    "returns": "Resolved chat id and immediate post-send thread state.",
    "notes": [
      "Uses the current authenticated ChatGPT tab."
    ]
  }
}
```

The CLI now infers missing parameter docs from `required_variables` and `{placeholders}` so old workflows still work. But inference is the fallback, not the standard.

When using `workflow new`, the CLI now writes a starter `help` block and runs validation immediately. You should still review the generated descriptions and examples before committing the workflow.

## 📁 Folder Structure

- `/workflows/google` - Google services (Search, Images)
- `/workflows/bing` - Bing services (Images)
- `/workflows/youtube` - YouTube workflows
- `/workflows/x` - X / Twitter workflows
- `/workflows/chatgpt` - ChatGPT web app workflows
- `/workflows/claude` - Claude web app workflows
- `/workflows/instagram` - Instagram profile/post asset workflows
- `/workflows/amazon` - Amazon workflows
- `/workflows/airbnb` - Airbnb workflows
- `/workflows/hn` - Hacker News workflows
- `/workflows/demos` - Demo and example workflows
- `/workflows/tests` - Test and debug workflows
- `/workflows/wip` - Work in progress workflows

## ✅ Working Workflows

### Google Search
- **File**: `google/google-search.json`
- **Description**: Search Google and extract organic search results
- **Parameters**: `search_query` (required)
- **Example**: `./target/release/rzn-browser run google search --param search_query="artificial intelligence"`
- **Output**: Extracts title, URL, and snippet for each search result

### YouTube Search
- **File**: `youtube/youtube-search.json`
- **Description**: Search YouTube and extract video results
- **Parameters**: `search_query` (required)
- **Example**: `./target/release/rzn-browser run workflows/youtube/youtube-search.json --param search_query="cat videos"`
- **Output**: Extracts video title, channel, views, upload date, and URL

### Reddit: Comment on First Post
- **File**: `reddit/reddit-first-post-comment.json`
- **Description**: Opens Reddit, opens the first post's comments page, and posts a comment
- **Parameters**: `comment_text` (required)
- **Example**: `./target/release/rzn-browser run workflows/reddit/reddit-first-post-comment.json --param comment_text="that is just great"`
- **Notes**: Requires you to be logged in; this performs a real write action (posting a comment).

### Reddit: Draft Comment on First Post (no submit)
- **File**: `reddit/reddit-first-post-draft-comment.json`
- **Description**: Opens Reddit, opens the first post's comments page, and types a draft comment (no submit)
- **Parameters**: `comment_text` (required)
- **Example**: `./target/release/rzn-browser run workflows/reddit/reddit-first-post-draft-comment.json --param comment_text="that is just great"`
- **Notes**: Leaves the draft in the comment box for you to review before submitting.

### Reddit: Comment on Specific Post URL
- **File**: `reddit/reddit-comment-on-post-url.json`
- **Description**: Navigate to a specific post URL and post a comment
- **Parameters**: `post_url` (required), `comment_text` (required)
- **Example**: `./target/release/rzn-browser run workflows/reddit/reddit-comment-on-post-url.json --param post_url="https://old.reddit.com/r/some_sub/comments/POSTID/some_title/" --param comment_text="that is just great"`
- **Notes**: Requires you to be logged in; this performs a real write action (posting a comment). Prefer `old.reddit.com` URLs for deterministic HTML.

### Reddit: Read Then Comment on Specific Post URL
- **File**: `reddit/reddit-read-then-comment-on-post-url.json`
- **Description**: Navigate to a specific post URL, scroll to read context, then post a comment
- **Parameters**: `post_url` (required), `comment_text` (required)
- **Example**: `./target/release/rzn-browser run workflows/reddit/reddit-read-then-comment-on-post-url.json --param post_url="https://old.reddit.com/r/some_sub/comments/POSTID/some_title/" --param comment_text="…"`
- **Notes**: Uses deterministic scroll + pauses to allow a short “read” phase before submitting.

### Reddit: Read Then Draft Comment on Specific Post URL (no submit)
- **File**: `reddit/reddit-read-then-draft-comment-on-post-url.json`
- **Description**: Open a specific post URL in a new tab, scroll to read context, then type a draft comment (no submit)
- **Parameters**: `post_url` (required), `comment_text` (required)
- **Example**: `./target/release/rzn-browser run workflows/reddit/reddit-read-then-draft-comment-on-post-url.json --param post_url="https://old.reddit.com/r/some_sub/comments/POSTID/some_title/" --param comment_text="…"`
- **Notes**: Opens a new tab and leaves the draft in the comment box for you to review before submitting.

### Reddit: Draft Chat Message to a User (no send)
- **File**: `reddit/reddit-draft-dm.json`
- **Description**: Open a Reddit user's profile on new Reddit, follow the full-page chat link, locate the composer through Reddit's shadow DOM, and type a draft message
- **Parameters**: `recipient` (required), `message_body` (required)
- **Example**: `./target/release/rzn-browser run workflows/reddit/reddit-draft-dm.json --via native --mode spawn --param recipient="jomreap" --param message_body="hello there"`
- **Notes**: Requires a logged-in Chrome session on new Reddit. The flow navigates from the profile's chat link to the full chat page and then types into the deep shadow-DOM composer. If Chrome redirects `www.reddit.com` to `old.reddit.com`, disable that redirect for this workflow because old Reddit does not support chat.

### Hacker News: Submit Link Post
- **File**: `hn/hn-submit-link-post.json`
- **Description**: Fill the HN submit form (title, URL, optional text), pause on an explicit approval gate, then submit. Click Stop at the gate to leave a draft instead of posting.
- **Parameters**: `post_title` (required), `post_url` (required), `post_text` (required; pass `""` for none)
- **Example**: `rzn-browser run hn submit-link-post --param post_title="Show HN: rzn-browser" --param post_url="https://github.com/example/rzn-browser" --param post_text=""`
- **Notes**: Requires you to be logged in. To dry-run the form fill without submitting, click Stop at the approval gate.

### Hacker News: Submit Root Comment
- **File**: `hn/hn-submit-comment.json`
- **Description**: Post a root-level comment on a chosen HN item. If `item_url` is omitted, comments on whatever is currently the first item on the front page. Approval-gated, with a dedupe guard against the logged-in user's own prior comments on the thread.
- **Parameters**: `comment_text` (required), `item_url` (optional; omit to comment on first front-page item)
- **Example**: `rzn-browser run hn submit-comment --param item_url="https://news.ycombinator.com/item?id=12345678" --param comment_text="Section 4 misses cold-start cost — adding it flips the latency story."`
- **Notes**: Requires you to be logged in. To dry-run, click Stop at the approval gate.

### Hacker News: Submit Reply to Comment
- **File**: `hn/hn-submit-reply.json`
- **Description**: Reply to a specific HN comment by id. Approval-gated, with a dedupe guard against the logged-in user's own prior replies on the parent thread.
- **Parameters**: `comment_id` (required, numeric), `comment_text` (required)
- **Example**: `rzn-browser run hn submit-reply --param comment_id="12345678" --param comment_text="Agreed on cold-start; the gap is in Section 4 not Section 5."`
- **Notes**: Requires you to be logged in. To dry-run, click Stop at the approval gate.

### X: Top Posts From User
- **File**: `x/x-search-top-from-user.json`
- **Description**: Reuse the current logged-in X tab/profile, open `from:<handle>` Top search, and extract up to 20 posts
- **Parameters**: `handle` (required, no leading `@`)
- **Example**: `rzn-browser run x search-top-from-user --param handle="felixrieseberg"`
- **Notes**: Intentionally uses `use_current_tab` so the workflow rides on the browser's existing authenticated X session.

### X: Profile Posts
- **File**: `x/x-profile-posts.json`
- **Description**: Reuse the current logged-in X tab/profile, open a profile page, and extract up to 20 posts
- **Parameters**: `handle` (required, no leading `@`)
- **Example**: `rzn-browser run x profile-posts --param handle="felixrieseberg"`
- **Notes**: DOM-first extraction only; no dependency on private X API calls.

### X: Session Cookies Debug
- **File**: `x/x-session-cookies-debug.json`
- **Description**: Show the subset of x.com cookies visible to page JavaScript in the current browser session
- **Parameters**: none
- **Example**: `rzn-browser run x session-cookies-debug`
- **Notes**: Useful for understanding `document.cookie`, but it will not reveal HttpOnly auth cookies.

### X: Search User Posts In Window
- **File**: `x/x-search-user-window.json`
- **Description**: Search x.com for a handle within a `since` / `until` date window and extract candidate posts for later thread expansion
- **Parameters**: `handle`, `since_date`, `until_date`, `timeline_mode`
- **Example**: `rzn-browser run x search-user-window --param handle="felixrieseberg" --param since_date="2026-03-10" --param until_date="2026-03-18" --param timeline_mode="live"`
- **Notes**: This is the discovery half of the thread-export flow.

### X: Expand Thread From Post URL
- **File**: `x/x-thread-from-post-url.json`
- **Description**: Open one X post URL in a new tab and extract the same-author thread plus scoped assets and hrefs
- **Parameters**: `post_url`, `handle`
- **Example**: `rzn-browser run x thread-from-post-url --param post_url="https://x.com/felixrieseberg/status/123" --param handle="felixrieseberg"`
- **Notes**: Returns a combined thread payload, which the export wrapper turns into Markdown and optional downloads.

### Instagram: Export Recent Assets
- **Files**: `instagram/instagram-profile-recent-posts.json`, `instagram/instagram-post-extract.json`
- **Description**: Discover recent Instagram post/reel URLs for an account and expand one explicit post into ID/stats/comments plus image/video asset URLs. Both flows are invoked directly through the `rzn-browser` binary.
- **Parameters**:
  - Workflow discovery: `handle`, `target_count`, `max_scrolls`, `max_idle_scrolls`
  - Workflow read: `post_url`
- **Examples**:
  ```bash
  rzn-browser run instagram profile-recent-posts --param handle="timbersfc" --param target_count="36" --param max_scrolls="8" --param max_idle_scrolls="3"
  rzn-browser run instagram post-extract --param post_url="https://www.instagram.com/p/CuE2WN3LM7r/"
  ```
- **Output**: CLI returns candidate URLs or one structured post payload, including image/video asset URLs the caller can fetch.

### X: Draft Reply To Post URL
- **File**: `x/x-draft-reply-to-post-url.json`
- **Description**: Open a post in a new tab, type a reply draft, and stop before sending
- **Parameters**: `post_url`, `reply_text`
- **Example**: `rzn-browser run x draft-reply-to-post-url --param post_url="https://x.com/felixrieseberg/status/123" --param reply_text="Draft only"`
- **Notes**: Uses a manual-review pause and never presses send.

### X: Draft DM
- **File**: `x/x-draft-dm.json`
- **Description**: Open a recipient profile in a new tab, launch the DM composer, type a draft message, and stop before sending
- **Parameters**: `recipient_handle`, `message_text`
- **Example**: `rzn-browser run x draft-dm --param recipient_handle="felixrieseberg" --param message_text="Draft only"`
- **Notes**: Draft-only flow; does not submit the DM.

### ChatGPT: New Chat And Send
- **File**: `chatgpt/chatgpt_new_chat_send_v1.json`
- **Description**: Open ChatGPT in the current tab, normalize to a fresh chat, default to `Pro` with `Extended` effort unless overridden, send the first prompt, and return the new `chat_id`
- **Parameters**: `message_text` (required), `model_slug` (optional), `model_effort` (optional)
- **Example**: `rzn-browser run chatgpt new-chat-send-v1 --param message_text="Summarize the last three commits" --param model_slug="GPT-5"`
- **Notes**: Uses the currently authenticated Chrome session and defaults send flows to `Pro -> Extended`.

### ChatGPT: New Chat Send With Attachment
- **File**: `chatgpt/chatgpt_new_chat_send_attachment_v1.json`
- **Description**: Open ChatGPT in the current tab, normalize to a fresh chat, upload a local file or image, send the first prompt, and return the new `chat_id`
- **Parameters**: `message_text` (required), `attachment_file_path` (required), `model_slug` (optional), `model_effort` (optional)
- **Example**: `rzn-browser run chatgpt new-chat-send-attachment-v1 --param message_text="Describe this uploaded image in one sentence" --param attachment_file_path="/abs/path/to/image.png" --param model_slug="Pro" --param model_effort="Extended"`
- **Notes**: Uses the generic `upload_file` action after a ChatGPT-specific file-input prep step.

### ChatGPT: Send Current Composer
- **File**: `chatgpt/chatgpt_send_current_composer_v1.json`
- **Description**: Reuse the current ChatGPT tab without navigating, optionally verify an already attached file or image, and send from the visible composer
- **Parameters**: `message_text` (required), `attachment_file_path` (optional)
- **Example**: `rzn-browser run chatgpt send-current-composer-v1 --param message_text="Continue from the current draft"`
- **Notes**: Useful when a prior workflow or operator action already prepared the composer state.

### ChatGPT: Get Latest Response
- **File**: `chatgpt/chatgpt_get_response_v1.json`
- **Description**: Open an existing chat by `chat_id`, wait for the latest assistant response to stabilize, and return it with compact thread metadata
- **Parameters**: `chat_id` (required)
- **Example**: `rzn-browser run chatgpt get-response-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef"`
- **Notes**: Polls the visible DOM until the latest assistant turn stops changing.

### ChatGPT: Continue Chat
- **File**: `chatgpt/chatgpt_continue_chat_v1.json`
- **Description**: Open an existing chat, default to `Pro` with `Extended` effort unless overridden, send another prompt, and return the post-send state
- **Parameters**: `chat_id` (required), `message_text` (required), `model_slug` (optional), `model_effort` (optional)
- **Example**: `rzn-browser run chatgpt continue-chat-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param message_text="Now turn that into a checklist"`
- **Notes**: Stays in the same thread and does not require any engine-level ChatGPT support.

### ChatGPT: Export Visible Chat
- **File**: `chatgpt/chatgpt_export_chat_v1.json`
- **Description**: Open an existing chat by `chat_id` and extract the visible transcript
- **Parameters**: `chat_id` (required)
- **Example**: `rzn-browser run chatgpt export-chat-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef"`
- **Notes**: Returns structured role/content turns from the visible DOM only.

### ChatGPT Images: New Generation
- **File**: `chatgpt/chatgpt_images_new_generation_v1.json`
- **Description**: Open `chatgpt.com/images`, send an image-generation prompt, default to 2 variations unless already specified, and return the resulting `chat_id`
- **Parameters**: `message_text` (required), `variation_count` (optional, default: 2)
- **Example**: `rzn-browser run chatgpt images-new-generation-v1 --param message_text="A cinematic portrait of a fox astronaut" --param variation_count="2"`
- **Notes**: Intended for downstream polling via `chatgpt_images_get_latest_v1.json` or the local wrapper script.

### ChatGPT Images: New Generation With Attachment
- **File**: `chatgpt/chatgpt_images_new_generation_attachment_v1.json`
- **Description**: Open `chatgpt.com/images`, upload a local image or file, send an image-generation prompt, default to 2 variations unless already specified, and return the resulting `chat_id`
- **Parameters**: `message_text` (required), `attachment_file_path` (required), `variation_count` (optional, default: 2)
- **Example**: `rzn-browser run chatgpt images-new-generation-attachment-v1 --param message_text="Turn this into a retro travel poster" --param attachment_file_path="/abs/path/to/reference.png" --param variation_count="2"`
- **Notes**: Uses the generic `upload_file` action after a ChatGPT Images-specific file-input prep step.

### ChatGPT Images: Get Latest Result
- **File**: `chatgpt/chatgpt_images_get_latest_v1.json`
- **Description**: Open an existing image-generation chat by `chat_id`, inspect the latest assistant turn for image URLs, and report whether the requested set is ready
- **Parameters**: `chat_id` (required), `expected_image_count` (optional, default: 2)
- **Example**: `rzn-browser run chatgpt images-get-latest-v1 --param chat_id="01234567-89ab-cdef-0123-456789abcdef" --param expected_image_count="2"`
- **Notes**: Returns readiness, image metadata, and the extracted image URLs from the latest assistant turn.

### ChatGPT Images: Generate And Download
- **File**: `chatgpt/chatgpt_images_generate_and_download_v1.json`
- **Description**: Start a ChatGPT Images job, wait for the rendered result to stabilize in the same tab, and trigger browser downloads into a named Downloads subfolder
- **Parameters**: `message_text` (required), `download_folder` (required), `variation_count` (optional)
- **Example**: `rzn-browser run chatgpt images-generate-and-download-v1 --param message_text="A cinematic portrait of a fox astronaut" --param download_folder="fox_astronaut" --param variation_count="2"`
- **Notes**: Caller-chosen cwd-relative output paths and explicit file names still belong to helper-layer tooling, not the raw workflow surface.

### Bing Images - Thumbnails
- **File**: `bing/bing-images-download.json`
- **Description**: Search Bing Images and download thumbnail results
- **Parameters**: 
  - `search_query` (required) - What to search for
  - `download_folder` (required) - Folder name for downloads
  - `limit` (optional, default: 20) - Maximum number of images
- **Example**: 
  ```bash
  ./target/release/rzn-browser run workflows/bing/bing-images-download.json \
    --param search_query="puppies" \
    --param download_folder="cute_puppies"
  ```
- **Output**: Downloads thumbnail images to `~/Downloads/{download_folder}/`

### Bing Images - Extract High-Res URLs
- **File**: `bing/bing-images-metadata.json`
- **Description**: Extract metadata containing high-resolution image URLs
- **Parameters**: `search_query` (required)
- **Example**: 
  ```bash
  # Extract URLs and download with wget
  ./target/release/rzn-browser run workflows/bing/bing-images-metadata.json \
    --param search_query="4k wallpapers" 2>&1 | \
    sed -n '/^\[$/,/^\]$/p' | \
    jq -r '.[].metadata | fromjson | .murl' 2>/dev/null | \
    xargs -I {} wget -P ~/Downloads/wallpapers_4k "{}"
  ```
- **Output**: Original high-resolution images from source websites

### Bing Images - Simple
- **File**: `bing/bing-images-simple.json`
- **Description**: Simple Bing Images search with auto-generated folder name
- **Parameters**: `search_query` (required)
- **Example**: `./target/release/rzn-browser run workflows/bing/bing-images-simple.json --param search_query="cats"`
- **Output**: Downloads to `~/Downloads/bing_{search_query}/`

### Demo: Unsplash Download
- **File**: `demos/image-download-demo.json`
- **Description**: Download images from Unsplash (demonstrates working image downloads)
- **Parameters**: `search_query` (required)
- **Example**: `./target/release/rzn-browser run workflows/demos/image-download-demo.json --param search_query="nature"`
- **Output**: Downloads images to `~/Downloads/unsplash_{search_query}/`

## 🚧 Work in Progress

### Google Images
- **Status**: Thumbnails extract but downloads need work
- **Issue**: Images use lazy loading with base64 placeholders
- **Location**: `wip/google-images-*.json`
- **Note**: Google requires clicking on images to get high-res URLs

### Execute JavaScript Action
- **Status**: Handler implemented but CSP restrictions on many sites
- **Issue**: Content Security Policy blocks script execution
- **Alternative**: Use extract_structured_data for most tasks

## 🔧 Common Issues

### Images Not Downloading
Chrome downloads images to your default Downloads folder:
- **macOS/Linux**: `~/Downloads/{folder_name}/`
- **Windows**: `%USERPROFILE%\Downloads\{folder_name}\`
- Chrome may show a popup asking to "Allow multiple downloads" - click Allow
- Images must have absolute URLs (not base64 or relative paths)

### 4MB Message Limit
For large data extraction, we've implemented OPFS (Origin Private File System) support to avoid the 4MB Chrome extension message size limit.

## 🚀 Running Workflows

All workflows can be run with:
```bash
./target/release/rzn-browser run workflows/{domain}/{workflow-name}.json --param key="value"
```

Deterministic workflows run through `rzn-browser run ...` without requiring an LLM provider.
