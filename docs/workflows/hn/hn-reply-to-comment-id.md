---
title: HN Reply To Comment ID
slug: /workflows/hn/hn-reply-to-comment-id/
sidebar:
  label: HN Reply
---

# hn-reply-to-comment-id

## JSON
- File: `workflows/hn/hn-reply-to-comment-id.json`
- Workflow id: `hn_reply_to_comment_id_v1`

## Input Parameters
- `comment_id`
- `comment_text`

## Behavior
- Opens the HN reply form.
- Types the reply.
- Pauses on approval before the final submit click.

## Run
- `make run W=workflows/hn/hn-reply-to-comment-id.json PARAMS='--param comment_id="12345678" --param comment_text="Reply text"'`
