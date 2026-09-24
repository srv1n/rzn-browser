import { chromium, test, expect, type BrowserContext } from '@playwright/test';
import { execFile, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import type { AddressInfo } from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';

const execFileAsync = promisify(execFile);
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '../../..');
const extensionRoot = path.resolve(__dirname, '../..');
const smokeEnabled = process.env.RZN_E2E_NATIVE_HOST_SMOKE === '1';

// macOS's per-user os.tmpdir() (/var/folders/.../T) is long enough that
// `<root>/app-base/run/rzn-supervisor.sock` overflows sockaddr_un's 104-byte
// sun_path, so `supervisor serve` fails with "path must be shorter than
// SUN_LEN". /tmp (a short, stable path) stays well under the limit.
const tmpRoot = process.platform === 'darwin' ? '/tmp' : os.tmpdir();

// Branded Google Chrome (v137+) dropped support for --load-extension /
// --disable-extensions-except on its release channels, so the `chrome`
// target can't use a system Chrome install (this was the original bug: every
// test timed out in extensionOrigin() waiting for a "serviceworker" event
// that never fired). Playwright's bundled open-source Chromium build still
// honors those flags, but Chromium never implements
// chrome.runtime.connectNative at all (confirmed: `typeof
// chrome.runtime.connectNative === "undefined"` in a loaded worker) -- see
// background.ts's own "Some Chromium-based browsers disable native
// messaging" warning, which anticipates exactly this.
//
// "Chrome for Testing" (CfT) is Google's official build for automation, and
// it fixes the extension-loading half: --load-extension works, the manifest
// loads, and chrome.runtime.connectNative is a real function. But an actual
// connectNative() call still fails with "Specified native messaging host
// not found" -- verified with the manifest placed in every macOS
// NativeMessagingHosts directory a Chrome variant could plausibly read
// (Google/Chrome, Google/ChromeForTesting, "Google/Chrome for Testing"),
// under both a HOME-env override and the real account HOME, with and without
// Playwright's default `--disable-extensions` arg. CfT's own Crashpad/
// settings.dat under "Google/Chrome for Testing" prove that directory name
// is correct, so this isn't a manifest-path bug: CfT appears to disable
// native messaging outright, presumably because it's meant for CI sandboxes
// where letting web content spawn an arbitrary local process is a
// deliberate non-goal. We still wire CfT up below (via `@puppeteer/browsers`,
// since Playwright doesn't bundle it) because it's a strict improvement --
// the extension now loads and every doctor check up to the native
// messaging handshake passes -- but the `chrome` target's native-messaging
// assertions are expected to keep failing on this or any other CfT release
// until Google changes that policy.
//
// Branded Microsoft Edge has not dropped --load-extension and its
// connectNative works, so the `edge` target still runs Edge directly via the
// `msedge` channel.
const CHROME_FOR_TESTING_CACHE_DIR = path.join(os.homedir(), '.cache', 'rzn-e2e', 'chrome-for-testing');

function findChromeForTesting(): string | undefined {
  const fromEnv = process.env.RZN_CHROME_FOR_TESTING_BIN;
  if (fromEnv && fs.existsSync(fromEnv)) return fromEnv;
  if (!fs.existsSync(CHROME_FOR_TESTING_CACHE_DIR)) return undefined;
  const platformDirs = fs.readdirSync(path.join(CHROME_FOR_TESTING_CACHE_DIR, 'chrome')).sort().reverse();
  for (const dir of platformDirs) {
    const matches = [
      path.join(CHROME_FOR_TESTING_CACHE_DIR, 'chrome', dir, 'chrome-mac-arm64', 'Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing'),
      path.join(CHROME_FOR_TESTING_CACHE_DIR, 'chrome', dir, 'chrome-mac-x64', 'Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing'),
      path.join(CHROME_FOR_TESTING_CACHE_DIR, 'chrome', dir, 'chrome-linux64', 'chrome'),
      path.join(CHROME_FOR_TESTING_CACHE_DIR, 'chrome', dir, 'chrome-win64', 'chrome.exe'),
    ];
    const found = matches.find((candidate) => fs.existsSync(candidate));
    if (found) return found;
  }
  return undefined;
}

function ensureChromeForTesting(): string | undefined {
  const existing = findChromeForTesting();
  if (existing) return existing;
  try {
    fs.mkdirSync(CHROME_FOR_TESTING_CACHE_DIR, { recursive: true });
    execFileSync('npx', ['--yes', '@puppeteer/browsers', 'install', 'chrome@stable', '--path', CHROME_FOR_TESTING_CACHE_DIR], {
      stdio: 'ignore',
      timeout: 120_000,
    });
  } catch {
    return undefined;
  }
  return findChromeForTesting();
}

type SmokeTarget = {
  browser: 'chrome' | 'edge' | 'chromium';
  extensionDir: string;
  // `channel`/`executablePath` decide which actual binary Playwright drives.
  // `manifestBrowsers` lists every native-messaging directory that binary
  // reads from, so `installNativeHost` can populate all of them.
  channel?: 'msedge';
  executablePath?: string;
  manifestBrowsers: string[];
};

const targets: SmokeTarget[] = [
  { browser: 'chrome', extensionDir: path.join(extensionRoot, 'dist/chrome'), executablePath: ensureChromeForTesting(), manifestBrowsers: ['chrome-for-testing'] },
  { browser: 'edge', extensionDir: path.join(extensionRoot, 'dist/edge'), channel: 'msedge', manifestBrowsers: ['edge'] },
  { browser: 'chromium', extensionDir: path.join(extensionRoot, 'dist/chromium'), manifestBrowsers: ['chromium'] },
];

function binaryPath(name: 'rzn-browser' | 'rzn-native-host'): string {
  const envKey = name === 'rzn-browser' ? 'RZN_BROWSER_BIN' : 'RZN_NATIVE_HOST_BIN';
  const fromEnv = process.env[envKey];
  if (fromEnv) return fromEnv;
  const exe = process.platform === 'win32' ? `${name}.exe` : name;
  return path.join(repoRoot, 'target/debug', exe);
}

function envFor(home: string, appBase: string): NodeJS.ProcessEnv {
  return {
    ...process.env,
    HOME: home,
    USERPROFILE: home,
    LOCALAPPDATA: path.join(home, 'AppData/Local'),
    APPDATA: path.join(home, 'AppData/Roaming'),
    RZN_APP_BASE_DIR: appBase,
    RZN_SUPERVISOR_APP_BASE: appBase,
  };
}

async function runCli(args: string[], env: NodeJS.ProcessEnv) {
  const { stdout, stderr } = await execFileAsync(binaryPath('rzn-browser'), args, {
    cwd: repoRoot,
    env,
    timeout: 20_000,
  });
  return { stdout, stderr };
}

async function launchWithExtension(target: SmokeTarget, userDataDir: string, env: NodeJS.ProcessEnv) {
  if (!fs.existsSync(path.join(target.extensionDir, 'manifest.json'))) {
    test.skip(true, `missing ${target.extensionDir}; run bun run build first`);
  }
  if (target.browser === 'chrome' && !target.executablePath) {
    test.skip(true, 'Chrome for Testing is unavailable (could not download it via @puppeteer/browsers)');
  }

  try {
    return await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: target.channel,
      executablePath: target.executablePath,
      env,
      // Playwright's own default args include `--disable-extensions`. It
      // doesn't block --load-extension from loading the unpacked bundle, but
      // drop it anyway so the loaded extension runs exactly as it would in a
      // normal (non-automated) browser launch.
      ignoreDefaultArgs: ['--disable-extensions'],
      args: [
        `--disable-extensions-except=${target.extensionDir}`,
        `--load-extension=${target.extensionDir}`,
      ],
    });
  } catch (error) {
    test.skip(true, `${target.browser} browser is unavailable: ${String(error)}`);
    throw error;
  }
}

async function extensionOrigin(context: BrowserContext): Promise<string> {
  let worker = context.serviceWorkers()[0];
  if (!worker) {
    worker = await context.waitForEvent('serviceworker', { timeout: 10_000 });
  }
  const url = new URL(worker.url());
  return `chrome-extension://${url.host}/`;
}

async function installNativeHost(target: SmokeTarget, origin: string, env: NodeJS.ProcessEnv) {
  for (const browser of target.manifestBrowsers) {
    await runCli(
      [
        'native-host',
        'install',
        '--browser',
        browser,
        '--extension-origin',
        origin,
        '--native-host-path',
        binaryPath('rzn-native-host'),
        '--json',
      ],
      env
    );
  }
}

async function doctorOutput(target: SmokeTarget, origin: string, appBase: string, env: NodeJS.ProcessEnv) {
  try {
    const result = await runCli(
      [
        'native-host',
        'doctor',
        '--browser',
        target.browser,
        '--extension-origin',
        origin,
        '--app-base',
        appBase,
        '--json',
      ],
      env
    );
    return result.stdout || result.stderr;
  } catch (error: any) {
    return `${error?.stdout ?? ''}\n${error?.stderr ?? ''}`.trim();
  }
}

async function waitForTargets(appBase: string, env: NodeJS.ProcessEnv, expected: string[]) {
  const deadline = Date.now() + 20_000;
  let last = '';
  while (Date.now() < deadline) {
    try {
      const { stdout } = await runCli(['browser', 'targets', '--app-base', appBase, '--json'], env);
      last = stdout;
      const parsed = JSON.parse(stdout);
      const seen = new Set(
        (parsed.targets ?? parsed.bridges ?? [])
          .map((target: any) => target.extension_target ?? target.browser)
          .filter(Boolean)
      );
      if (expected.every((target) => seen.has(target))) {
        return parsed;
      }
    } catch (error: any) {
      last = `${error?.stdout ?? ''}\n${error?.stderr ?? ''}`.trim();
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error(`Timed out waiting for targets ${expected.join(', ')}. Last output:\n${last}`);
}

async function withSupervisor<T>(appBase: string, env: NodeJS.ProcessEnv, run: () => Promise<T>) {
  const child = execFile(binaryPath('rzn-browser'), ['supervisor', 'serve', '--app-base', appBase], {
    cwd: repoRoot,
    env,
  });
  try {
    await new Promise((resolve) => setTimeout(resolve, 750));
    return await run();
  } finally {
    child.kill();
    await new Promise((resolve) => child.once('exit', resolve));
  }
}

// ---------------------------------------------------------------------------
// Chaos helpers (STAB-21): inject a failure mid-run, expect a named error in
// bounded time, then expect the next run to succeed with no manual step.
// ---------------------------------------------------------------------------

const CHAOS_TEST_TIMEOUT_MS = 150_000;
const CHAOS_FAIL_BOUND_MS = 15_000;
const CHAOS_RECOVERY_BOUND_MS = 30_000;
const LONG_STEP = { type: 'wait_for_timeout', timeout_ms: 25_000 };
const NAMED_TRANSPORT_ERROR = /\b(NATIVE_HOST_DISCONNECTED|EXTENSION_BRIDGE_TIMEOUT|SUPERVISOR_UNREACHABLE)\b/;
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

type CallResult = { json: any; raw: string; ms: number };

// `supervisor call` against the chaos browser's bridge. Never throws: a non-zero exit or
// a process timeout is returned as raw output so the caller can assert on it.
async function supervisorCall(
  appBase: string,
  env: NodeJS.ProcessEnv,
  method: string,
  params: Record<string, unknown>,
  timeoutMs = 45_000
): Promise<CallResult> {
  const started = Date.now();
  const args = ['supervisor', 'call', '--app-base', appBase, '--json', '--browser', chaosBrowser, method, '--params', JSON.stringify(params)];
  try {
    const { stdout } = await execFileAsync(binaryPath('rzn-browser'), args, { cwd: repoRoot, env, timeout: timeoutMs });
    return { json: JSON.parse(stdout), raw: stdout, ms: Date.now() - started };
  } catch (error: any) {
    let json: any = null;
    try {
      json = JSON.parse(error?.stdout ?? '');
    } catch {}
    const raw = `${error?.stdout ?? ''}\n${error?.stderr ?? ''}\n${error?.killed ? 'CLI_KILLED_BY_TEST_TIMEOUT' : ''}`;
    return { json, raw, ms: Date.now() - started };
  }
}

// Mirrors workflow_runner::response_success.
function stepSucceeded(res: any): boolean {
  if (!res) return false;
  const status = res.run_result?.status;
  if (typeof status === 'string') return status === 'succeeded';
  const nested = [res.result?.success, res.result?.ok, res.result?.result?.success, res.result?.result?.ok].find(
    (value) => typeof value === 'boolean'
  );
  if (nested !== undefined) return nested;
  if (res.error_code || res.error) return false;
  return res.success ?? res.ok ?? true;
}

async function chaosNativeHostPid(appBase: string, env: NodeJS.ProcessEnv): Promise<number | undefined> {
  const { stdout } = await runCli(['browser', 'targets', '--app-base', appBase, '--json'], env);
  const parsed = JSON.parse(stdout);
  const target = (parsed.targets ?? []).find((t: any) => (t.extension_target ?? t.browser) === chaosBrowser);
  const pid = target?.metadata?.native_host_pid;
  return typeof pid === 'number' ? pid : undefined;
}

async function waitForNativeHostPid(appBase: string, env: NodeJS.ProcessEnv, notPid?: number): Promise<number> {
  const deadline = Date.now() + CHAOS_RECOVERY_BOUND_MS;
  let last: unknown;
  while (Date.now() < deadline) {
    try {
      const pid = await chaosNativeHostPid(appBase, env);
      if (pid !== undefined && pid !== notPid) return pid;
      last = pid;
    } catch (error) {
      last = error;
    }
    await sleep(500);
  }
  throw new Error(`Timed out waiting for a ${chaosBrowser} native host pid other than ${notPid}. Last: ${String(last)}`);
}

// One "run": open a session on `url`, execute one step, close the session.
async function runOnce(appBase: string, env: NodeJS.ProcessEnv, url: string, step: Record<string, unknown>) {
  const open = await supervisorCall(appBase, env, 'browser.session_open', { url });
  expect(stepSucceeded(open.json), `session_open failed: ${open.raw}`).toBe(true);
  const sessionId = open.json.session_id as string;
  const res = await supervisorCall(appBase, env, 'browser.execute_step', { session_id: sessionId, step });
  await supervisorCall(appBase, env, 'browser.session_close', { session_id: sessionId }, 15_000);
  return res;
}

type ChaosServer = { base: string; hits: Map<string, number>; close: () => Promise<void> };

async function startChaosServer(): Promise<ChaosServer> {
  const pages: Record<string, string> = {
    '/idle': '<!doctype html><title>idle</title><p>idle</p>',
    '/nav':
      '<!doctype html><title>nav</title><button id="go">go</button>' +
      "<script>document.getElementById('go').addEventListener('click', () => { location.href = '/landed'; });</script>",
    '/landed':
      '<!doctype html><title>landed</title><button id="after">after</button>' +
      "<script>document.getElementById('after').addEventListener('click', () => { fetch('/after-hit'); });</script>",
    '/after-hit': 'ok',
  };
  const hits = new Map<string, number>();
  const server = http.createServer((req, res) => {
    const pathname = new URL(req.url ?? '/', 'http://x').pathname;
    const body = pages[pathname];
    if (body === undefined) {
      res.writeHead(404).end();
      return;
    }
    hits.set(pathname, (hits.get(pathname) ?? 0) + 1);
    res.writeHead(200, { 'content-type': 'text/html', 'cache-control': 'no-store' }).end(body);
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address() as AddressInfo;
  return {
    base: `http://127.0.0.1:${port}`,
    hits,
    close: () => new Promise<void>((resolve) => server.close(() => resolve())),
  };
}

// Chaos lanes default to Edge: branded Chrome >=137 ignores --load-extension and
// Chrome for Testing cannot find a native messaging host, so Edge is the browser
// that can drive the full chain locally. Override with RZN_E2E_CHAOS_BROWSER.
const chaosBrowser = process.env.RZN_E2E_CHAOS_BROWSER ?? 'edge';

// Same boot sequence as the kill-and-replace case: discover the origin, install
// the manifest, start the supervisor, launch the browser, wait for the bridge.
async function bootChaosLane(prefix: string) {
  const target = targets.find((candidate) => candidate.browser === chaosBrowser);
  if (!target) throw new Error(`unknown RZN_E2E_CHAOS_BROWSER=${chaosBrowser}`);
  test.skip(!fs.existsSync(binaryPath('rzn-browser')), `missing ${binaryPath('rzn-browser')}`);
  test.skip(!fs.existsSync(binaryPath('rzn-native-host')), `missing ${binaryPath('rzn-native-host')}`);

  const root = await fs.promises.mkdtemp(path.join(tmpRoot, `rzn-${prefix}-`));
  const home = path.join(root, 'home');
  const appBase = path.join(root, 'app-base');
  const env = envFor(home, appBase);
  const firstContext = await launchWithExtension(target, path.join(root, 'profile-first'), env);
  const origin = await extensionOrigin(firstContext);
  await firstContext.close();
  await installNativeHost(target, origin, env);

  const supervisor = execFile(binaryPath('rzn-browser'), ['supervisor', 'serve', '--app-base', appBase], { cwd: repoRoot, env });
  const context = await launchWithExtension(target, path.join(root, 'profile-live'), env);
  const server = await startChaosServer();
  await waitForTargets(appBase, env, [target.browser]);
  return {
    appBase,
    env,
    context,
    server,
    dumpDoctor: async () => console.log(await doctorOutput(target, origin, appBase, env)),
    cleanup: async () => {
      await context.close().catch(() => {});
      await server.close();
      supervisor.kill();
      if (supervisor.exitCode === null) await new Promise((resolve) => supervisor.once('exit', resolve));
      // Browser profiles are ~300MB each; keep them only when debugging.
      if (process.env.RZN_E2E_KEEP_TMP !== '1') await fs.promises.rm(root, { recursive: true, force: true });
    },
  };
}

// Start LONG_STEP, inject the failure once it is in flight, and require a named
// failure within CHAOS_FAIL_BOUND_MS of the injection.
async function expectNamedFailureAfter(
  appBase: string,
  env: NodeJS.ProcessEnv,
  baseUrl: string,
  inject: () => Promise<void>
) {
  const open = await supervisorCall(appBase, env, 'browser.session_open', { url: `${baseUrl}/idle` });
  expect(stepSucceeded(open.json), `session_open failed: ${open.raw}`).toBe(true);
  const inFlight = supervisorCall(appBase, env, 'browser.execute_step', { session_id: open.json.session_id, step: LONG_STEP });
  await sleep(2_000);
  const injectedAt = Date.now();
  await inject();
  const res = await inFlight;
  const elapsed = Date.now() - injectedAt;
  expect(stepSucceeded(res.json), `step should fail after injection: ${res.raw}`).toBe(false);
  expect(elapsed, `failure took ${elapsed}ms: ${res.raw}`).toBeLessThan(CHAOS_FAIL_BOUND_MS);
  expect(res.raw, 'failure must carry a named error code').toMatch(NAMED_TRANSPORT_ERROR);
}

test.describe('native-host browser smoke', () => {
  test.skip(!smokeEnabled, 'set RZN_E2E_NATIVE_HOST_SMOKE=1 to run local native-host smoke tests');

  for (const target of targets) {
    test(`${target.browser} extension connects to native host and appears in browser targets`, async () => {
      test.skip(!fs.existsSync(binaryPath('rzn-browser')), `missing ${binaryPath('rzn-browser')}`);
      test.skip(!fs.existsSync(binaryPath('rzn-native-host')), `missing ${binaryPath('rzn-native-host')}`);

      const root = await fs.promises.mkdtemp(path.join(tmpRoot, `rzn-${target.browser}-smoke-`));
      const home = path.join(root, 'home');
      const appBase = path.join(root, 'app-base');
      const env = envFor(home, appBase);

      const firstContext = await launchWithExtension(target, path.join(root, 'profile-first'), env);
      const origin = await extensionOrigin(firstContext);
      await firstContext.close();
      await installNativeHost(target, origin, env);

      await withSupervisor(appBase, env, async () => {
        const context = await launchWithExtension(target, path.join(root, 'profile-live'), env);
        try {
          const targetsResult = await waitForTargets(appBase, env, [target.browser]);
          expect(targetsResult.target_count).toBeGreaterThanOrEqual(1);
        } catch (error) {
          console.log(await doctorOutput(target, origin, appBase, env));
          throw error;
        } finally {
          await context.close();
        }
      });
    });
  }

  test('Chrome reconnects after the supervisor is replaced', async () => {
    const target = targets[0];
    test.skip(!fs.existsSync(binaryPath('rzn-browser')), `missing ${binaryPath('rzn-browser')}`);
    test.skip(!fs.existsSync(binaryPath('rzn-native-host')), `missing ${binaryPath('rzn-native-host')}`);

    const root = await fs.promises.mkdtemp(path.join(tmpRoot, 'rzn-supervisor-chaos-'));
    const home = path.join(root, 'home');
    const appBase = path.join(root, 'app-base');
    const env = envFor(home, appBase);
    const firstContext = await launchWithExtension(target, path.join(root, 'profile-first'), env);
    const origin = await extensionOrigin(firstContext);
    await firstContext.close();
    await installNativeHost(target, origin, env);

    let supervisor = execFile(binaryPath('rzn-browser'), ['supervisor', 'serve', '--app-base', appBase], { cwd: repoRoot, env });
    const context = await launchWithExtension(target, path.join(root, 'profile-live'), env);
    try {
      await waitForTargets(appBase, env, ['chrome']);
      supervisor.kill('SIGKILL');
      await new Promise((resolve) => supervisor.once('exit', resolve));
      supervisor = execFile(binaryPath('rzn-browser'), ['supervisor', 'serve', '--app-base', appBase], { cwd: repoRoot, env });
      await waitForTargets(appBase, env, ['chrome']);
    } finally {
      await context.close();
      supervisor.kill();
      if (supervisor.exitCode === null) await new Promise((resolve) => supervisor.once('exit', resolve));
    }
  });

  test('chaos: native host killed mid-run fails named and the next run succeeds', async () => {
    test.setTimeout(CHAOS_TEST_TIMEOUT_MS);
    const lane = await bootChaosLane('native-host-kill-chaos');
    try {
      const pid = await waitForNativeHostPid(lane.appBase, lane.env);
      await expectNamedFailureAfter(lane.appBase, lane.env, lane.server.base, async () => {
        process.kill(pid, 'SIGKILL');
      });
      await waitForNativeHostPid(lane.appBase, lane.env, pid);
      const next = await runOnce(lane.appBase, lane.env, `${lane.server.base}/idle`, { type: 'get_current_url' });
      expect(stepSucceeded(next.json), `next run failed: ${next.raw}`).toBe(true);
    } catch (error) {
      await lane.dumpDoctor();
      throw error;
    } finally {
      await lane.cleanup();
    }
  });

  test('chaos: extension service worker reload mid-run fails named and the next run succeeds', async () => {
    test.setTimeout(CHAOS_TEST_TIMEOUT_MS);
    const lane = await bootChaosLane('service-worker-chaos');
    try {
      const pid = await waitForNativeHostPid(lane.appBase, lane.env);
      await expectNamedFailureAfter(lane.appBase, lane.env, lane.server.base, async () => {
        const worker = lane.context.serviceWorkers()[0] ?? (await lane.context.waitForEvent('serviceworker', { timeout: 10_000 }));
        // The worker dies while evaluating, so the evaluate promise may reject.
        await worker.evaluate(() => (globalThis as any).chrome.runtime.reload()).catch(() => {});
      });
      // A reload tears down the native port; Chrome must spawn a fresh host.
      await waitForNativeHostPid(lane.appBase, lane.env, pid);
      const next = await runOnce(lane.appBase, lane.env, `${lane.server.base}/idle`, { type: 'get_current_url' });
      expect(stepSucceeded(next.json), `next run failed: ${next.raw}`).toBe(true);
    } catch (error) {
      await lane.dumpDoctor();
      throw error;
    } finally {
      await lane.cleanup();
    }
  });

  test('chaos: click that navigates runs exactly once and the next step on the tab works', async () => {
    test.setTimeout(CHAOS_TEST_TIMEOUT_MS);
    const lane = await bootChaosLane('navigate-mid-step-chaos');
    try {
      const { appBase, env, server } = lane;
      const open = await supervisorCall(appBase, env, 'browser.session_open', { url: `${server.base}/nav` });
      expect(stepSucceeded(open.json), `session_open failed: ${open.raw}`).toBe(true);
      const sessionId = open.json.session_id as string;

      const click = await supervisorCall(appBase, env, 'browser.execute_step', {
        session_id: sessionId,
        step: { type: 'click_element', selector: '#go' },
      });
      expect(click.ms, `navigating click took ${click.ms}ms: ${click.raw}`).toBeLessThan(CHAOS_FAIL_BOUND_MS);
      if (!stepSucceeded(click.json)) {
        expect(click.raw, 'a navigating click may only fail as NAVIGATED_DURING_STEP').toMatch(/\bNAVIGATED_DURING_STEP\b/);
      }

      // Exactly once: the click handler is the only thing that loads /landed.
      await expect.poll(() => server.hits.get('/landed') ?? 0, { timeout: 10_000 }).toBe(1);

      const after = await supervisorCall(appBase, env, 'browser.execute_step', {
        session_id: sessionId,
        step: { type: 'click_element', selector: '#after' },
      });
      expect(stepSucceeded(after.json), `next step on the navigated tab failed: ${after.raw}`).toBe(true);
      await expect.poll(() => server.hits.get('/after-hit') ?? 0, { timeout: 10_000 }).toBe(1);
      await sleep(1_000);
      expect(server.hits.get('/landed'), 'the navigating click must not be re-sent').toBe(1);
      await supervisorCall(appBase, env, 'browser.session_close', { session_id: sessionId }, 15_000);
    } catch (error) {
      await lane.dumpDoctor();
      throw error;
    } finally {
      await lane.cleanup();
    }
  });

  test('Chrome and Edge can connect simultaneously and route separately', async () => {
    const chrome = targets[0];
    const edge = targets[1];
    test.skip(!fs.existsSync(binaryPath('rzn-browser')), `missing ${binaryPath('rzn-browser')}`);
    test.skip(!fs.existsSync(binaryPath('rzn-native-host')), `missing ${binaryPath('rzn-native-host')}`);

    const root = await fs.promises.mkdtemp(path.join(tmpRoot, 'rzn-chrome-edge-smoke-'));
    const home = path.join(root, 'home');
    const appBase = path.join(root, 'app-base');
    const env = envFor(home, appBase);

    const chromeFirst = await launchWithExtension(chrome, path.join(root, 'chrome-first'), env);
    const chromeOrigin = await extensionOrigin(chromeFirst);
    await chromeFirst.close();
    const edgeFirst = await launchWithExtension(edge, path.join(root, 'edge-first'), env);
    const edgeOrigin = await extensionOrigin(edgeFirst);
    await edgeFirst.close();

    await installNativeHost(chrome, chromeOrigin, env);
    await installNativeHost(edge, edgeOrigin, env);

    await withSupervisor(appBase, env, async () => {
      const chromeContext = await launchWithExtension(chrome, path.join(root, 'chrome-live'), env);
      const edgeContext = await launchWithExtension(edge, path.join(root, 'edge-live'), env);
      try {
        const targetsResult = await waitForTargets(appBase, env, ['chrome', 'edge']);
        expect(targetsResult.target_count).toBeGreaterThanOrEqual(2);

        const chromeSession = JSON.parse(
          (await runCli(
            ['supervisor', 'call', '--app-base', appBase, '--json', '--browser', 'chrome', 'browser.session_open'],
            env
          )).stdout
        );
        const edgeSession = JSON.parse(
          (await runCli(
            ['supervisor', 'call', '--app-base', appBase, '--json', '--browser', 'edge', 'browser.session_open'],
            env
          )).stdout
        );

        expect(chromeSession.resolved_browser_target.browser).toBe('chrome');
        expect(edgeSession.resolved_browser_target.browser).toBe('edge');
      } catch (error) {
        console.log(await doctorOutput(chrome, chromeOrigin, appBase, env));
        console.log(await doctorOutput(edge, edgeOrigin, appBase, env));
        throw error;
      } finally {
        await edgeContext.close();
        await chromeContext.close();
      }
    });
  });
});
