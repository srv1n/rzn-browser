---
title: HN Draft Submit Link Post
slug: /workflows/hn/hn-draft-submit-link-post/
sidebar:
  label: HN Draft Submit
---

# hn-draft-submit-link-post

## JSON
- File: `workflows/hn/hn-draft-submit-link-post.json`
- Workflow id: `hn_draft_submit_link_post_v1`

## Input Parameters
- `post_title`
- `post_url`
- `post_text`

## Behavior
- Opens the HN submit page in a new tab.
- Fills the title, URL, and text fields.
- Stops before pressing submit.

## Run
- `make run W=workflows/hn/hn-draft-submit-link-post.json PARAMS='--param post_title="Example title" --param post_url="https://example.com" --param post_text=""'`
