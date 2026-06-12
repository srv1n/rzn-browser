---
title: HN Submit Link Post
slug: /workflows/hn/hn-submit-link-post/
sidebar:
  label: HN Submit
---

# hn-submit-link-post

## JSON
- File: `workflows/hn/hn-submit-link-post.json`
- Workflow id: `hn_submit_link_post_v1`

## Input Parameters
- `post_title`
- `post_url`
- `post_text`

## Behavior
- Opens the HN submit page.
- Fills the title, URL, and text fields.
- Pauses on an approval gate before the final submit click.

## Run
- `make run W=workflows/hn/hn-submit-link-post.json PARAMS='--param post_title="Example title" --param post_url="https://example.com" --param post_text=""'`
