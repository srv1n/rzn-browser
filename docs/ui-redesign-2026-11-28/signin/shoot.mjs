// Screenshot every HTML file in a folder at a desktop size: node shoot.mjs <html dir> <png dir>
import { chromium } from '@playwright/test';
import fs from 'node:fs'; import path from 'node:path';
const [dir, outDir] = process.argv.slice(2); fs.mkdirSync(outDir, { recursive: true });
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1100, height: 760 } });
for (const file of fs.readdirSync(dir).filter(f => f.endsWith('.html'))) {
  await page.goto('file://' + path.resolve(dir, file));
  await page.evaluate(() => document.activeElement && document.activeElement.blur());
  await page.screenshot({ path: path.join(outDir, file.replace(/\.html$/, '.png')) });
}
await browser.close();
