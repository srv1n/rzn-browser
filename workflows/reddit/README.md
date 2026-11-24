# Reddit Workflows

Discover the installed Reddit routes and their effective source:

```bash
rzn-browser workflow list reddit
rzn-browser workflow list reddit --json
```

Inspect a route before running it:

```bash
rzn-browser workflow show reddit/profile-history
rzn-browser workflow inspect reddit profile-history --json
```

## Items for an index

`reddit/thread` returns one `rzn.item/v2`; `reddit/listing` and `reddit/search` return
`rzn.items/v2` pages (listing `feed`) of thread entries. Keys and fields are the rzn-tools Reddit
connector's (`reddit:thread:<id>`, `u/<name>`, `r/<subreddit>`, counts `score`/`upvote_ratio`/
`comments`), so a thread read here and through rzn-tools lands as one document. A thread with more
comments than `limit`, or with comments behind "load more", comes back `truncated` and is not
indexed. `describe.json` is the family's `rzn.describe/v2`; `samples/` holds the outputs the lint
checks (hand-built until recorded, see `scripts/workflow_samples/`).

## Profile history without OAuth

`reddit/profile-history` drives the rendered `www.reddit.com` profile feed
through the connected browser bridge. It does not use Reddit's API or `.json`
endpoints.

The normal public-profile call is:

```bash
rzn-browser run reddit profile-history --param username=example_user
```

Useful bounded variants:

```bash
rzn-browser run reddit profile-history --param username=example_user --param mode=comments --param limit=50
rzn-browser run reddit profile-history --param username=example_user --param mode=posts --param limit=50
```

The default accepts the browser's anonymous cookies when Reddit renders public
history without an account login. Such output reports `authenticated=false`
and `history_may_be_partial=true`. Require an account-authenticated session
only when the caller needs that stronger contract:

```bash
rzn-browser run reddit profile-history --param username=example_user --param require_login=true
```

If no browser bridge is connected, inspect and repair the local path first:

```bash
rzn-browser browser targets
rzn-browser heal --json
```

The command's standard missing-parameter help, typed defaults, enum validation,
examples, result schema, and side-effect declarations come from the workflow
manifest, so callers do not need to open the JSON file.
