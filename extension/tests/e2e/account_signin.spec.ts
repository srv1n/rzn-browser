// Sign in with an RZN account (BFL-T-0012), live, in a headless Chromium with the extension loaded
// unpacked, the native host and a supervisor: the Account page signs in through the server's own
// sign-in page with no code or server URL typed, the device is enrolled as the user, a captured page
// is pushed to the user's index with no capture.json, and a device revoked in the app stops at its
// next check-in and says so.
//
//   RZN_E2E_SIGNIN=1 RZN_E2E_SERVER=https://build1-api.rzn.ai RZN_E2E_EMAIL=smoke@example.com \
//   RZN_E2E_CTL=/path/ctl RZN_E2E_PAGE_URL=https://voicebraindump.com/offline \
//   RZN_E2E_CAPTURE_INDEX=x-push-1 RZN_E2E_CAPTURE_SOURCE=capture \
//     bun x playwright test tests/e2e/account_signin.spec.ts
//
// The sign-in code arrives by email, so a driver outside the browser exchanges marker files in
// RZN_E2E_CTL: this test writes `otp-requested`, the driver writes `otp` (the emailed code); this
// test writes `signed-in.json` and `captured.json`, the driver revokes the device in the app and
// writes `revoked`; this test writes `stopped.json`. No token is written there. Never run against a
// browser profile of a person.
import { chromium, test, expect, type BrowserContext } from '@playwright/test';
import { execFile, spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';

const execFileAsync = promisify(execFile);
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, '../../..');
const extensionDir = path.resolve(__dirname, '../../dist/chromium');
const enabled = process.env.RZN_E2E_SIGNIN === '1';
const server = process.env.RZN_E2E_SERVER || '';
const email = process.env.RZN_E2E_EMAIL || '';
const ctl = process.env.RZN_E2E_CTL || '';
const pageUrl = process.env.RZN_E2E_PAGE_URL || '';
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function binary(name: 'rzn-browser' | 'rzn-native-host'): string {
  const env = name === 'rzn-browser' ? process.env.RZN_BROWSER_BIN : process.env.RZN_NATIVE_HOST_BIN;
  return env || path.join(repoRoot, 'target/debug', name);
}

function envFor(home: string, appBase: string): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {
    ...process.env, HOME: home, XDG_CONFIG_HOME: path.join(home, '.config'), XDG_DATA_HOME: path.join(home, '.local/share'),
    RZN_APP_BASE_DIR: appBase, RZN_SUPERVISOR_APP_BASE: appBase, RZN_WORKFLOWS_DIR: path.join(repoRoot, 'workflows'),
    RZN_FLEET_CONFIG_PATH: path.join(appBase, 'fleet_config.json'),
  };
  for (const key of ['RZN_LOCAL_RUNTIME_SOCKET_PATH', 'RZN_SUPERVISOR_SOCKET_PATH', 'RZN_LOCAL_RUNTIME_TOKEN_PATH', 'RZN_SUPERVISOR_TOKEN_PATH']) delete env[key];
  return env;
}

async function cli(env: NodeJS.ProcessEnv, args: string[]): Promise<any> {
  const { stdout } = await execFileAsync(binary('rzn-browser'), args, { cwd: repoRoot, env, timeout: 60_000 });
  return JSON.parse(stdout);
}

const call = (env: NodeJS.ProcessEnv, appBase: string, method: string, params: object = {}) =>
  cli(env, ['supervisor', 'call', '--app-base', appBase, '--json', method, '--params', JSON.stringify(params)]);

async function launch(profile: string, env: NodeJS.ProcessEnv): Promise<BrowserContext> {
  return await chromium.launchPersistentContext(profile, {
    headless: process.env.RZN_PW_HEADFUL !== '1', channel: 'chromium', env,
    ignoreDefaultArgs: ['--disable-extensions'],
    args: [`--disable-extensions-except=${extensionDir}`, `--load-extension=${extensionDir}`],
  });
}

async function extensionId(context: BrowserContext): Promise<string> {
  const worker = context.serviceWorkers()[0] ?? (await context.waitForEvent('serviceworker', { timeout: 10_000 }));
  return new URL(worker.url()).host;
}

function activity(appBase: string): any[] {
  try {
    return fs.readFileSync(path.join(appBase, 'capture', 'activity.jsonl'), 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
  } catch {
    return [];
  }
}

async function waitFor<T>(what: string, probe: () => T | undefined | Promise<T | undefined>, ms = 30_000, every = 250): Promise<T> {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    const value = await probe();
    if (value !== undefined) return value;
    await sleep(every);
  }
  throw new Error(`timed out waiting for ${what}`);
}

const mark = (name: string, value: unknown) => fs.writeFileSync(path.join(ctl, name), JSON.stringify(value, null, 2));
const marker = (name: string) => () => (fs.existsSync(path.join(ctl, name)) ? fs.readFileSync(path.join(ctl, name), 'utf8').trim() : undefined);

test.describe('sign in with an RZN account', () => {
  test.skip(!enabled, 'set RZN_E2E_SIGNIN=1 and the RZN_E2E_* variables (needs target/debug binaries and dist/chromium)');

  test('signs in with no code typed, captures to the own index, stops when revoked in the app', async () => {
    test.setTimeout(900_000);
    fs.mkdirSync(ctl, { recursive: true });
    const root = await fs.promises.mkdtemp('/tmp/rzn-signin-');
    const home = path.join(root, 'home');
    const appBase = path.join(root, 'app-base');
    await fs.promises.mkdir(appBase, { recursive: true });
    const env = envFor(home, appBase);

    const first = await launch(path.join(root, 'profile-first'), env);
    const id = await extensionId(first);
    await first.close();
    expect(id).toBe('bogjdnehdficgkhklinmnbgiiofbamji');
    const installed = await cli(env, ['native-host', 'install', '--browser', 'chromium', '--extension-origin', `chrome-extension://${id}/`,
      '--native-host-path', binary('rzn-native-host'), '--json']);
    const manifest = installed.reports?.[0]?.manifest_path;
    const profile = path.join(root, 'profile-live');
    await fs.promises.mkdir(path.join(profile, 'NativeMessagingHosts'), { recursive: true });
    await fs.promises.copyFile(manifest, path.join(profile, 'NativeMessagingHosts', path.basename(manifest)));
    const log = fs.openSync(path.join(root, 'supervisor.log'), 'a');
    const supervisor = spawn(binary('rzn-browser'), ['supervisor', 'serve', '--app-base', appBase], {
      cwd: repoRoot, env: { ...env, RUST_LOG: env.RUST_LOG || 'info' }, stdio: ['ignore', log, log],
    });
    let context: BrowserContext | null = null;
    try {
      await sleep(750);
      context = await launch(profile, env);
      await waitFor('the extension at the supervisor', async () => {
        const targets = await call(env, appBase, 'browser.targets').catch(() => null);
        return JSON.stringify(targets?.targets ?? []).match(/"browser_instance_id":"[0-9a-f-]{36}"/) ? true : undefined;
      }, 90_000, 500);

      // Settings: the server override, and where captured pages go.
      const site = new URL(pageUrl).hostname.replace(/^www\./, '');
      const dashboard = await context.newPage();
      await dashboard.goto(`chrome-extension://${id}/dashboard.html#account`);
      const saved = await dashboard.evaluate(async (patch) => await chrome.runtime.sendMessage({
        cmd: 'supervisor_rpc', payload: { method: 'settings.set', params: { patch } },
      }), { server_url: server, capture_sites: [site], capture_index: process.env.RZN_E2E_CAPTURE_INDEX, capture_source: process.env.RZN_E2E_CAPTURE_SOURCE });
      expect(saved?.result?.server_url).toBe(server);
      await dashboard.reload();
      await expect(dashboard.locator('[data-sign-in]')).toHaveText('Sign in to RZN', { timeout: 30_000 });
      await expect(dashboard.locator('main')).not.toContainText('Enrollment code');

      // Sign in: the server's own page opens in a tab; email, then the emailed code.
      const opened = context.waitForEvent('page');
      await dashboard.locator('[data-sign-in]').click();
      const signin = await opened;
      await signin.waitForLoadState('domcontentloaded');
      expect(new URL(signin.url()).origin).toBe(new URL(server).origin);
      expect(signin.url()).toContain('client_id=rzn-browser');
      await signin.locator('input[name=email]').fill(email);
      await signin.locator('button').click();
      await signin.locator('input[name=otp]').waitFor({ timeout: 30_000 });
      mark('otp-requested', { at_ms: Date.now() });
      const otp = await waitFor('the emailed code from the driver', marker('otp'), 180_000, 500);
      await signin.locator('input[name=otp]').fill(otp);
      await signin.locator('button').click();
      await waitFor('the sign-in tab to close', () => (signin.isClosed() ? true : undefined), 60_000);

      const account = await waitFor('signed in', async () => {
        const status = await call(env, appBase, 'account.status').catch(() => null);
        return status?.signed_in ? status : undefined;
      }, 60_000, 500);
      expect(account.user_email).toBe(email);
      expect(fs.existsSync(path.join(appBase, 'capture.json'))).toBe(false);
      const polling = await waitFor('the first check-in', async () => {
        const status = await call(env, appBase, 'account.status');
        return status.fleet?.state === 'polling' && status.fleet?.last_poll_ms ? status : undefined;
      }, 120_000, 1_000);
      await dashboard.reload();
      await expect(dashboard.locator('main')).toContainText(email, { timeout: 30_000 });
      await expect(dashboard.locator('main')).toContainText('This browser');
      await expect(dashboard.locator('[data-sign-out]')).toHaveText('Sign out');
      await dashboard.screenshot({ path: path.join(ctl, 'account.png'), fullPage: true });
      // The device id is ours, not the user's: Settings shows it under Advanced.
      await dashboard.goto(`chrome-extension://${id}/dashboard.html#settings`);
      await expect(dashboard.locator('details[data-advanced]')).toContainText(account.device_id, { timeout: 30_000 });
      await dashboard.goto(`chrome-extension://${id}/dashboard.html#account`);
      mark('signed-in.json', { device_id: account.device_id, user_id: account.user_id, user_email: account.user_email, server_url: account.server_url, fleet_state: polling.fleet.state, last_poll_ms: polling.fleet.last_poll_ms });

      // Capture: a page of the switched-on site goes to the user's own index with the user's token.
      const page = await context.newPage();
      await page.goto(pageUrl, { waitUntil: 'load' });
      const read = await waitFor('the page read', () => activity(appBase).find((e) => e.event === 'page_read'), 90_000);
      expect(read.error).toBeUndefined();
      expect((await call(env, appBase, 'capture.flush')).items).toBeGreaterThan(0);
      const sent = await waitFor('the batch', () => activity(appBase).find((e) => e.event === 'batch_sent' || e.event === 'batch_refused' || e.event === 'batch_held'), 90_000);
      mark('captured.json', { read, sent });
      expect(sent.event).toBe('batch_sent');
      expect(sent.job_id).toBeTruthy();

      // Revoked in the app: the next check-in stops it, and the Account page says so.
      await waitFor('the driver to revoke the device', marker('revoked'), 600_000, 1_000);
      const stopped = await waitFor('the loop to stop', async () => {
        const status = await call(env, appBase, 'account.status');
        return status.fleet?.state === 'stopped_revoked' ? status : undefined;
      }, 180_000, 1_000);
      await dashboard.reload();
      await expect(dashboard.locator('main')).toContainText('signed out from the app', { timeout: 30_000 });
      await dashboard.screenshot({ path: path.join(ctl, 'revoked.png'), fullPage: true });
      mark('stopped.json', { fleet: stopped.fleet, at_ms: Date.now() });
    } finally {
      await context?.close().catch(() => undefined);
      supervisor.kill('SIGTERM');
      console.log('logs in', root);
    }
  });
});
