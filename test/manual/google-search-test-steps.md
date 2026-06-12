# Google Search Test - Quick Steps

## Prerequisites
1. Make sure Chrome is open
2. Extension is loaded (check `chrome://extensions/`)
3. Keep Chrome window in focus

## Test Steps

### Step 1: Open a new tab in Chrome
- Press `Cmd+T` (Mac) or `Ctrl+T` (Windows/Linux)
- This creates a fresh tab for the workflow

### Step 2: Keep the tab active
- **IMPORTANT**: Don't switch tabs or minimize Chrome
- The extension needs an active tab to work

### Step 3: Run the workflow
Open Terminal and run:
```bash
cd /Users/sarav/Downloads/side/rzn/rzn-browser
cargo run -p rzn-browser -- run workflows/google/google-search.json --param search_query="OpenAI GPT news"
```

### Step 4: Watch the automation
- The browser will navigate to Google
- Type the search query
- Extract results

## If you get "Could not establish connection" error:

1. **Reload the extension**:
   - Go to `chrome://extensions/`
   - Click reload button on RZN extension

2. **Check for active tab**:
   - Make sure you have at least one tab open
   - The tab should be active (not in background)

3. **Try a simpler test**:
   Navigate to Google manually first:
   ```bash
   # First, go to https://www.google.com in Chrome
   # Then run:
   cargo run -p rzn-browser -- run workflows/google/google-search.json --param search_query="test"
   ```

## Check logs for debugging:
```bash
# See what's happening
tail -f ~/rzn_build.log | jq . | grep -E "(error|ERROR|success)"
```

## Expected Behavior with Site Profiles:
When the workflow runs on Google, you should see in the console:
- `[RZN] Using site profile extraction for google.com`
- `[Flags] Resolved for google.com`
- Results will be extracted using predefined selectors (not LLM)

## Site Profile Benefits:
- ✅ No LLM calls needed for Google
- ✅ Faster extraction
- ✅ Deterministic selectors
- ✅ Works even if OpenAI is down