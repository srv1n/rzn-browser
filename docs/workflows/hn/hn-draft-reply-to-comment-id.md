---
title: HN Draft Reply To Comment ID
slug: /workflows/hn/hn-draft-reply-to-comment-id/
sidebar:
  label: HN Draft Reply
---

# hn-draft-reply-to-comment-id

## JSON
- File: `workflows/hn/hn-draft-reply-to-comment-id.json`
- Workflow id: `hn_draft_reply_to_comment_id_v1`

## Input Parameters
- `comment_id`
- `comment_text`

## Behavior
- Opens the HN reply form in a new tab.
- Types the reply draft.
- Stops before submit.

## Run
- `make run W=workflows/hn/hn-draft-reply-to-comment-id.json PARAMS='--param comment_id="12345678" --param comment_text="Draft only"'`
