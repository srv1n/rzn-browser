---
title: HN Read Then Draft Comment
slug: /workflows/hn/hn-read-then-draft-comment-on-item-url/
sidebar:
  label: HN Draft Comment
---

# hn-read-then-draft-comment-on-item-url

## JSON
- File: `workflows/hn/hn-read-then-draft-comment-on-item-url.json`
- Workflow id: `hn_read_then_draft_comment_on_item_url_v1`

## Input Parameters
- `item_url`
- `comment_text`

## Behavior
- Opens an HN item page in a new tab.
- Scrolls for context.
- Types a root-comment draft and stops before submit.

## Run
- `make run W=workflows/hn/hn-read-then-draft-comment-on-item-url.json PARAMS='--param item_url="https://news.ycombinator.com/item?id=123" --param comment_text="Draft only"'`
