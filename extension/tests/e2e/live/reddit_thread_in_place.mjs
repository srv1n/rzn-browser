// Live check of reddit/thread's in-place read (BFL-T-0026): open one public thread logged out in a
// plain headless Chromium and run the workflow's read step on the page as loaded (no navigation,
// no scrolling, no request of its own). Prints the item. One page per run; keep it gentle.
//
//   node tests/e2e/live/reddit_thread_in_place.mjs https://www.reddit.com/r/<sub>/comments/<id>/
//
// The extension test profile (new-headless Chromium with the extension) is answered with Reddit's
// js_challenge and never reaches the thread, so this runs the read outside the extension.
import { chromium } from 'playwright';
import fs from 'node:fs';

const url = process.argv[2];
const workflow = JSON.parse(fs.readFileSync(new URL('../../../../workflows/reddit/reddit-thread.json', import.meta.url), 'utf8'));
const read = workflow.steps.find((s) => s.id === 's5').action.inputs.script;
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36' });
  await page.goto(url, { waitUntil: 'load', timeout: 60_000 });
  await page.waitForSelector(workflow.steps.find((s) => s.id === 's4').action.target.selector, { timeout: 30_000 });
  const before = await page.evaluate(() => [location.href, window.scrollY]);
  const item = await page.evaluate(async ([script, arg0]) => {
    const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
    return await new AsyncFunction('arg0', '__rzn_params', script)(arg0, { in_place: true });
  }, [read, url]);
  const after = await page.evaluate(() => [location.href, window.scrollY]);
  console.log(JSON.stringify({ before, after, item }, null, 1));
} finally {
  await browser.close();
}
