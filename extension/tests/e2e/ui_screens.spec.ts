// The end-user screens (popup, Account, Settings) in each state, in a headless Chromium with the
// extension loaded unpacked. The helper is replaced by a fixed answer per method inside the page,
// so every state renders without a supervisor; capture_pages and account_signin cover the real one.
// Checks plain words, keyboard use and the collapsed Advanced section. With RZN_UI_SHOTS=<dir> it
// also writes one screenshot per state there.
//
//   bun run build:chromium && RZN_UI_SHOTS=../docs/ui-redesign-2026-11-28 bun x playwright test tests/e2e/ui_screens.spec.ts
import { chromium, test, expect, type BrowserContext, type Page } from '@playwright/test';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const extensionDir = path.resolve(__dirname, '../../dist/chromium');
const shots = process.env.RZN_UI_SHOTS ? path.resolve(process.cwd(), process.env.RZN_UI_SHOTS) : '';
const banned = /fleet|automation|supervisor|workflow|\bjobs?\b|worker|tenant|enrol/i;

type Answers = Record<string, unknown> & { unreachable?: boolean; tabUrl?: string; tabIcon?: string; errors?: Record<string, string> };
const snapshot = { supervisor_version: '0.1.6', native_host_connected: true, extension_connected: true, paused: false, now_running: null, fleet: null, recent_runs: [], flagged_workflows: 0 };
const settings = { run_retention_count: 500, run_retention_days: 30, notifications_enabled: true, notify_on: 'failures_only', fleet_keep_window_on_failure: false, capture_sites: ['lambda.ai'], capture_index: 'reading', capture_source: 'capture', server_url: '' };
const signedIn = { signed_in: true, enrolled: true, user_email: 'sam@example.com', user_id: 'u1', device_id: 'b7f1c2e4-1d2a-4c55-9e0e-6f1f0c9a77d3', device_name: 'Sam’s MacBook Air', server_url: 'https://app.rzn.ai', fleet: { state: 'polling', last_poll_ms: Date.now() - 42_000 } };
const signedOut = { signed_in: false, enrolled: false };
// The tab's own icon, as Chrome reports it in favIconUrl.
const tabIcon = `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" rx="3" fill="#111"/><path d="M5 4h6M8 4v8" stroke="#fff" stroke-width="2"/></svg>')}`;
const answers = (over: Answers = {}): Answers => ({ tabIcon, 'status.snapshot': snapshot, 'account.status': signedIn, 'settings.get': settings, tabUrl: 'https://www.nytimes.com/2026/11/28/technology/ai-search.html', ...over });

/** Replace the helper inside the page: every supervisor_rpc gets the fixed answer for its method. */
function mock(page: Page, a: Answers) {
  return page.addInitScript((a: any) => {
    const state = structuredClone(a); (window as any).__rznCalls = [];
    const runtime: any = chrome.runtime;
    runtime.sendMessage = async (message: any) => {
      if (message?.cmd !== 'supervisor_rpc') return { success: true };
      const { method, params } = message.payload; (window as any).__rznCalls.push({ method, params });
      if (state.unreachable) return { success: false, transport_error: true, error: 'native host disconnected' };
      if (state.errors?.[method]) return { success: false, error: state.errors[method] };
      if (method === 'settings.set') { state['settings.get'] = { ...state['settings.get'], ...params.patch }; return { success: true, result: state['settings.get'] }; }
      if (method in state) return { success: true, result: state[method] };
      return { success: true, result: { ok: true } };
    };
    (chrome.tabs as any).query = async () => [{ id: 1, url: state.tabUrl, favIconUrl: state.tabIcon }];
  }, a);
}

let context: BrowserContext; let id = '';
test.beforeAll(async () => {
  context = await chromium.launchPersistentContext(fs.mkdtempSync(path.join(os.tmpdir(), 'rzn-ui-')), {
    headless: process.env.RZN_PW_HEADFUL !== '1', channel: 'chromium', ignoreDefaultArgs: ['--disable-extensions'],
    args: [`--disable-extensions-except=${extensionDir}`, `--load-extension=${extensionDir}`],
  });
  const worker = context.serviceWorkers()[0] ?? (await context.waitForEvent('serviceworker', { timeout: 10_000 }));
  id = new URL(worker.url()).host;
  if (shots) fs.mkdirSync(shots, { recursive: true });
});
test.afterAll(async () => { await context?.close(); });

async function open(file: string, a: Answers, size: { width: number; height: number }): Promise<Page> {
  const page = await context.newPage(); await page.setViewportSize(size); await mock(page, a);
  await page.goto(`chrome-extension://${id}/${file}`);
  return page;
}
const popup = (a: Answers) => open('popup.html', a, { width: 360, height: 560 });
const dashboard = (hash: string, a: Answers) => open(`dashboard.html#${hash}`, a, { width: 1280, height: 800 });
async function shot(page: Page, name: string, element = false) {
  await page.evaluate(() => document.fonts.ready);
  // Geist, plus Geist Mono for the small section labels; upright only, no serif or italics.
  const fonts = await page.evaluate(() => [...new Set([...document.fonts].map(f => `${f.family.replace(/"/g, '')} ${f.style}`))]);
  expect(fonts.sort()).toEqual(['Geist Mono normal', 'Geist normal']);
  const loaded = await page.evaluate(() => [...document.fonts].some(f => f.status === 'loaded'));
  expect(loaded).toBe(true);
  if (shots) element ? await page.locator('body').screenshot({ path: path.join(shots, `${name}.png`) }) : await page.screenshot({ path: path.join(shots, `${name}.png`), fullPage: true });
}
const text = (page: Page) => page.locator('body').innerText();

test.describe('end-user screens', () => {
  test('popup, signed out: one sign-in action', async () => {
    const page = await popup(answers({ 'account.status': signedOut }));
    await expect(page.locator('#sign-in')).toHaveText('Sign in to RZN');
    await expect(page.locator('#capture-switch')).toHaveCount(0);
    expect(await text(page)).not.toMatch(banned);
    await shot(page, 'popup-signed-out', true); await page.close();
  });

  test('popup, signed in: the site switch works from the keyboard and says where pages go', async () => {
    const page = await popup(answers());
    await expect(page.locator('.who')).toHaveText('sam@example.com');
    await expect(page.locator('.site-name')).toHaveText('nytimes.com');
    await expect(page.locator('.site .avatar img')).toBeVisible();
    const toggle = page.getByRole('switch');
    await expect(toggle).not.toBeChecked();
    expect(await text(page)).not.toMatch(banned);
    await shot(page, 'popup-signed-in-capture-off', true);

    await page.keyboard.press('Tab'); await page.keyboard.press('Tab');
    await expect(toggle).toBeFocused();
    const ring = await toggle.evaluate(el => getComputedStyle(el).outlineStyle);
    expect(ring).toBe('solid');
    await shot(page, 'popup-keyboard-focus', true);
    await page.keyboard.press('Space');
    await expect(toggle).toBeChecked();
    await expect(toggle).toBeFocused();
    await expect(page.locator('#capture-help')).toContainText('saved to reading');
    const calls = await page.evaluate(() => (window as any).__rznCalls.filter((c: any) => c.method === 'settings.set'));
    expect(calls.at(-1).params.patch.capture_sites).toEqual(['lambda.ai', 'nytimes.com']);
    await toggle.blur();
    await shot(page, 'popup-signed-in-capture-on', true); await page.close();
  });

  test('popup, helper not answering: short and friendly, with a retry', async () => {
    const page = await popup(answers({ unreachable: true }));
    await expect(page.locator('#retry')).toHaveText('Try again');
    expect(await text(page)).not.toMatch(banned);
    await shot(page, 'popup-unreachable', true); await page.close();
  });

  test('popup, helper too old for a method: an update notice, never raw RPC', async () => {
    const raw = 'Supervisor native_host.rpc failed: {"code":-32000,"message":"native-host RPC method not allowed: account.status"}';
    const page = await popup(answers({ errors: { 'account.status': raw } }));
    await expect(page.locator('h1')).toHaveText('RZN needs an update');
    const shown = await text(page);
    expect(shown).not.toMatch(/rpc|json|\{|-32000|account\.status|supervisor/i);
    await shot(page, 'popup-needs-update', true); await page.close();
  });

  test('popup, a page that is not a website', async () => {
    const page = await popup(answers({ tabUrl: 'chrome://newtab/' }));
    await expect(page.locator('.site')).toContainText("can't be saved");
    await shot(page, 'popup-not-a-website', true); await page.close();
  });

  test('Account, signed out', async () => {
    const page = await dashboard('account', answers({ 'account.status': signedOut }));
    await expect(page.locator('[data-sign-in]')).toHaveText('Sign in to RZN');
    await expect(page.locator('.sidebar nav a')).toHaveText(['Account', 'Settings']);
    await expect(page.locator('.sidebar .mark svg')).toBeVisible();
    await expect(page.locator('.sidebar')).toHaveCount(1);
    expect(await text(page)).not.toMatch(banned);
    await shot(page, 'account-signed-out'); await page.close();
  });

  test('Account, signed in: who, this browser, sign out', async () => {
    const page = await dashboard('account', answers());
    await expect(page.locator('.user')).toContainText('Signed in as');
    await expect(page.locator('.user-email')).toHaveText('sam@example.com');
    await expect(page.locator('.user .chip')).toHaveText('Connected');
    await expect(page.locator('main')).toContainText('This browser');
    await expect(page.locator('.device-row')).toContainText('Sam’s MacBook Air');
    await expect(page.locator('[data-sign-out]')).toHaveText('Sign out');
    expect(await text(page)).not.toMatch(banned);
    await shot(page, 'account-signed-in'); await page.close();
  });

  test('Account, signed out from the app', async () => {
    const page = await dashboard('account', answers({ 'account.status': { ...signedIn, fleet: { state: 'stopped_revoked' } } }));
    await expect(page.locator('main')).toContainText('signed out from the app');
    await shot(page, 'account-revoked'); await page.close();
  });

  test('Settings: saving pages first, everything else behind one collapsed Advanced', async () => {
    const page = await dashboard('settings', answers());
    const advanced = page.locator('details[data-advanced]');
    await expect(advanced).not.toHaveAttribute('open', '');
    await expect(page.locator('[name="server_url"]')).toBeHidden();
    await expect(page.locator('.site-label')).toHaveText(['lambda.ai']);
    await expect(page.locator('[data-site="lambda.ai"]')).toBeChecked();
    expect(await text(page)).not.toMatch(banned);
    await shot(page, 'settings-advanced-collapsed');
    await page.locator('details[data-advanced] > summary').focus(); await page.keyboard.press('Enter');
    await expect(page.locator('[name="server_url"]')).toBeVisible();
    await expect(advanced).toContainText(signedIn.device_id);
    await page.locator('details[data-advanced] > summary').blur();
    await page.setViewportSize({ width: 1280, height: 1800 });
    await shot(page, 'settings-advanced-open'); await page.close();
  });

  test('Settings: Add a site, switch one off, Save writes the list and says so', async () => {
    const page = await dashboard('settings', answers({ 'settings.get': { ...settings, capture_sites: ['lambda.ai', 'x.com'] } }));
    await page.getByLabel('Add a site').fill('https://Example.com/news');
    await page.keyboard.press('Enter');
    await expect(page.locator('.site-label')).toHaveText(['lambda.ai', 'x.com', 'example.com']);
    await page.locator('[data-site="x.com"]').uncheck();
    await page.getByRole('button', { name: 'Save changes' }).first().click();
    await expect(page.locator('[data-save-message]')).toHaveText('Saved');
    await shot(page, 'settings-saved');
    const calls = await page.evaluate(() => (window as any).__rznCalls.filter((c: any) => c.method === 'settings.set'));
    expect(calls.at(-1).params.patch.capture_sites).toEqual(['lambda.ai', 'example.com']);
    await page.close();
  });
});
