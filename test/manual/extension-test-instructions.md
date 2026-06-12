# Testing RZN Extension Features

## Quick Test Steps

### 1. Reload Extension
1. Open Chrome
2. Go to `chrome://extensions/`
3. Find "RZN Browser Automation"
4. Click the reload button (circular arrow icon)

### 2. Test Basic Extension Loading
1. Open new tab
2. Navigate to: `file:///Users/sarav/Downloads/side/rzn/rzn-browser/test/manual/browser/test_extension_basic.html`
3. Click "Test Chrome Runtime" button
4. Click "Send Test Message" button
5. You should see green checkmarks if working

### 3. Test Google Site Profile Feature
1. Open new tab
2. Navigate to: `https://www.google.com`
3. Open another tab with: `file:///Users/sarav/Downloads/side/rzn/rzn-browser/test/manual/browser/test_google_direct.html`
4. Click "Check Site Profile Config" button - should show Google profile
5. Click "Check Feature Flags" button - should show active flags
6. Go back to Google tab
7. Return to test page and click "Test Google Search & Extract"
8. Follow the prompts

### 4. Test Flight Recorder (Debugging Feature)
1. On any webpage, press `Ctrl+Shift+E`
2. Should trigger download of debugging data as JSON file
3. File contains:
   - All actions performed
   - Errors encountered
   - Performance metrics
   - DOM snapshots

## What Each Feature Does

### Site Profiles
- **Purpose**: Bypass LLM for known sites (Google, YouTube, Amazon, Reddit)
- **How it works**: Uses pre-defined selectors instead of AI extraction
- **Benefit**: Faster, more reliable, deterministic results

### Feature Flags
- **Purpose**: Control feature behavior per domain
- **Includes**: Circuit breaker, CDP enable/disable, batch actions
- **Location**: Check console for `[Flags] Resolved for domain`

### Flight Recorder
- **Purpose**: Debug automation failures
- **Captures**: Actions, errors, metrics, DOM changes
- **Export**: Ctrl+Shift+E downloads session data

## Check Console Logs

Open DevTools (F12) and look for:
- `[RZN]` - Extension logs
- `[RZN:CS]` - Content script logs
- `[RZN:SITE_PROFILE]` - Site profile usage
- `[Flags]` - Feature flag resolution

## If Extension Not Loading

1. Check `chrome://extensions/` for errors
2. Make sure "Developer mode" is ON (top right)
3. Click "Load unpacked" and select `/Users/sarav/Downloads/side/rzn/rzn-browser/extension/dist-chrome`
4. If already loaded, just click reload button

## Verify Logs

Check aggregated logs:
```bash
tail -f ~/rzn_build.log | jq .
```

Look for:
- `"component": "ext"` - Extension logs
- `"level": "error"` - Any errors
- `"action": "extension_log"` - Background script activity
