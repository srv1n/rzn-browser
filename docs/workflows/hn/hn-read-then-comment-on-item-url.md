---
title: HN Read Then Comment
slug: /workflows/hn/hn-read-then-comment-on-item-url/
sidebar:
  label: HN Comment
---

# hn-read-then-comment-on-item-url

## JSON
- File: `workflows/hn/hn-read-then-comment-on-item-url.json`
- Workflow id: `hn_read_then_comment_on_item_url_v1`

## Input Parameters
- `item_url`
- `comment_text`

## Behavior
- Opens an HN item page.
- Scrolls for context.
- Types a root comment, pauses on approval, then submits only if continued.

## Run
- `make run W=workflows/hn/hn-read-then-comment-on-item-url.json PARAMS='--param item_url="https://news.ycombinator.com/item?id=123" --param comment_text="Reply text"'`
