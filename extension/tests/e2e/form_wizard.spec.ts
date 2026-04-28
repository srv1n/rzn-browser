import { test, expect, chromium } from '@playwright/test';
import http from 'http';
import path from 'path';
import fs from 'fs';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

function startServer(html: string): Promise<{ url: string; close: () => Promise<void> }> {
  return new Promise((resolve) => {
    const server = http.createServer((_, res) => {
      res.writeHead(200, { 'Content-Type': 'text/html' });
      res.end(html);
    });
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      const port = typeof address === 'object' && address ? address.port : 0;
      resolve({
        url: `http://127.0.0.1:${port}/`,
        close: () =>
          new Promise<void>((resolveClose) => server.close(() => resolveClose())),
      });
    });
  });
}

function readRepoFixture(relPath: string): string {
  // __dirname = extension/tests/e2e
  const repoRoot = path.resolve(__dirname, '../../..');
  return fs.readFileSync(path.join(repoRoot, relPath), 'utf-8');
}

test.describe('Form wizard fixture', () => {
  test('can complete wizard via extension actions (no final submit)', async () => {
    const extensionPath = path.resolve(__dirname, '../../dist-chrome');
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
    const srv = await startServer(readRepoFixture('test/fixtures/form_wizard.html'));
    await page.goto(srv.url);

    await page.waitForFunction(
      () => typeof (window as any).__rznExecuteStep === 'function',
      { timeout: 10000 }
    );
    await page.waitForFunction(
      () => (window as any).__rznWizard && typeof (window as any).__rznWizard.getStep === 'function',
      { timeout: 10000 }
    );

    const exec = async (step: any) => {
      const resp = await page.evaluate(async (s) => (window as any).__rznExecuteStep(s), step);
      expect(resp && resp.success).toBeTruthy();
      return resp;
    };

    // Step 1
    await exec({ type: 'fill_input_field', selector: '#email', value: 'rzn.tester@example.com', force_legacy: true });
    await exec({ type: 'fill_input_field', selector: '#password', value: 'rznpass12345', force_legacy: true });
    await expect(page.locator('#email')).toHaveValue('rzn.tester@example.com');
    await expect(page.locator('#password')).toHaveValue('rznpass12345');
    await exec({ type: 'click_element', selector: '#planPro', force_legacy: true });
    await expect(page.locator('#planPro')).toBeChecked();
    await exec({ type: 'click_element', selector: '#nextBtn', force_legacy: true });
    const afterStep1 = await page.evaluate(() => {
      const get = (id: string) => (document.getElementById(id) as HTMLInputElement | null)?.value || '';
      const plan = (document.querySelector("input[name='plan']:checked") as HTMLInputElement | null)?.value || '';
      const globalError = (document.getElementById('globalError')?.textContent || '').trim();
      return { step: (window as any).__rznWizard?.getStep?.(), email: get('email'), password: get('password'), plan, globalError };
    });
    expect(afterStep1.step, JSON.stringify(afterStep1)).toBe(2);
    await expect(page.locator('#step-2')).toBeVisible();

    // Step 2
    await exec({ type: 'fill_input_field', selector: '#firstName', value: 'Ada', force_legacy: true });
    await exec({ type: 'fill_input_field', selector: '#lastName', value: 'Lovelace', force_legacy: true });
    await exec({ type: 'fill_input_field', selector: '#zip', value: '94107', force_legacy: true });
    await exec({ type: 'click_element', selector: '#terms', force_legacy: true });
    await expect(page.locator('#firstName')).toHaveValue('Ada');
    await expect(page.locator('#lastName')).toHaveValue('Lovelace');
    await expect(page.locator('#zip')).toHaveValue('94107');
    await expect(page.locator('#terms')).toBeChecked();
    await exec({ type: 'click_element', selector: '#nextBtn', force_legacy: true });
    await expect
      .poll(async () => {
        return page.evaluate(() => (window as any).__rznWizard?.getStep?.() ?? 0);
      }, { timeout: 5000 })
      .toBe(3);
    const afterStep2 = await page.evaluate(() => {
      const get = (id: string) => (document.getElementById(id) as HTMLInputElement | null)?.value || '';
      const terms = (document.getElementById('terms') as HTMLInputElement | null)?.checked || false;
      const globalError = (document.getElementById('globalError')?.textContent || '').trim();
      return { step: (window as any).__rznWizard?.getStep?.(), firstName: get('firstName'), lastName: get('lastName'), zip: get('zip'), terms, globalError };
    });
    expect(afterStep2.globalError, JSON.stringify(afterStep2)).toBe('');
    await expect(page.locator('#step-3')).toBeVisible();

    // Step 3 (Review)
    await expect(page.locator('#step-3 h2')).toHaveText('Review');
    await expect(page.locator('#reviewBox')).toContainText('rzn.tester@example.com');
    await expect(page.locator('#reviewBox')).toContainText('pro');
    await expect(page.locator('#submitBtn')).toBeEnabled();

    await context.close();
    await srv.close();
  });
});
