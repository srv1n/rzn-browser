# Browser Automation Examples

These example workflows are packaged into the signed `rzn-browser` bundle so the browser system
has curated quick starts instead of only raw MCP tools.

Included examples:

- `open_page_get_title.json`: simplest proof of life for the extension/native-host bridge
- `search_google.json`: safe read-only Google search extraction demo
- `extract_first_table.json`: structured table extraction demo
- `packaged_google_search_v2.json`: production-style packaged workflow example copied from the
  main `workflows/` catalog

These files are installed into the builtin runtime catalog under:

```text
~/Library/Application Support/RZN/workflows/builtin/examples/browser_automation/
```

and show up in the CLI as:

```text
rzn-browser workflow list examples
```
