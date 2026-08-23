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
