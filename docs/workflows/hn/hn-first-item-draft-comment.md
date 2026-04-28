---
title: HN First Item Draft Comment
slug: /workflows/hn/hn-first-item-draft-comment/
sidebar:
  label: HN First Item Draft
---

# hn-first-item-draft-comment

## JSON
- File: `workflows/hn/hn-first-item-draft-comment.json`
- Workflow id: `hn_first_item_draft_comment_v1`

## Input Parameters
- `comment_text`

## Behavior
- Opens the HN front page.
- Opens the first visible item discussion.
- Types a draft comment without submitting it.

## Run
- `make run W=workflows/hn/hn-first-item-draft-comment.json PARAMS='--param comment_text="Draft only"'`
