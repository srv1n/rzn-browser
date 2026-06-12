import { chromium, test, expect } from '@playwright/test';
import path from 'path';
import { fileURLToPath } from 'url';
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

test.describe('Extension content script', () => {
  test('injects captureEnhancedDOMSnapshot on pages', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist/chrome');
    const userDataDir = path.resolve(__dirname, '../../.pw-user-data');

    const context = await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: (process.env.RZN_PW_CHANNEL || (process.env.RZN_PW_HEADFUL === '1' ? undefined : 'chromium')) as any,
      args: [
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
    });

    const page = await context.newPage();
    await page.goto('https://example.com');

    // Wait for content script to expose capture function
    await page.waitForFunction(() => typeof (window as any).captureEnhancedDOMSnapshot === 'function', { timeout: 5000 });
    const hasCapture = await page.evaluate(() => typeof (window as any).captureEnhancedDOMSnapshot === 'function');
    expect(hasCapture).toBeTruthy();

    // Call the exposed function to get a small snapshot
    const snapshot = await page.evaluate(() => (window as any).captureEnhancedDOMSnapshot({ maxElements: 20 }));
    expect(snapshot).toBeTruthy();
    expect(typeof snapshot.hash).toBe('string');
    expect(Array.isArray(snapshot.elements)).toBeTruthy();

    await context.close();
  });
});
