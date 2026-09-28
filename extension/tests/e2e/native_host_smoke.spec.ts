import { chromium, test, expect, type BrowserContext } from '@playwright/test';
import { execFile } from 'node:child_process';
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
if (process.env.RZN_E2E_CHAOS_BROWSER && !smokeEnabled) {
  throw new Error('RZN_E2E_CHAOS_BROWSER requires RZN_E2E_NATIVE_HOST_SMOKE=1');
}

// macOS's per-user os.tmpdir() (/var/folders/.../T) is long enough that
// `<root>/app-base/run/rzn-supervisor.sock` overflows sockaddr_un's 104-byte
// sun_path, so `supervisor serve` fails with "path must be shorter than
// SUN_LEN". /tmp (a short, stable path) stays well under the limit.
const tmpRoot = process.platform === 'darwin' ? '/tmp' : os.tmpdir();

// Chrome for Testing is optional for local multi-browser smoke runs. Supply
// RZN_CHROME_FOR_TESTING_BIN or install it in the local cache before testing.
// The required CI chaos lane uses Playwright's full Chromium channel.
const CHROME_FOR_TESTING_CACHE_DIR = path.join(os.homedir(), '.cache', 'rzn-e2e', 'chrome-for-testing');

function findChromeForTesting(): string | undefined {
  const fromEnv = process.env.RZN_CHROME_FOR_TESTING_BIN;
  if (fromEnv && fs.existsSync(fromEnv)) return fromEnv;
  const chromeCache = path.join(CHROME_FOR_TESTING_CACHE_DIR, 'chrome');
  if (!fs.existsSync(chromeCache)) return undefined;
  const platformDirs = fs.readdirSync(chromeCache).sort().reverse();
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

type SmokeTarget = {
  browser: 'chrome' | 'edge' | 'chromium';
  extensionDir: string;
  // `channel`/`executablePath` decide which actual binary Playwright drives.
  // `manifestBrowsers` lists every native-messaging directory that binary
  // reads from, so `installNativeHost` can populate all of them.
  channel?: 'msedge' | 'chromium';
  executablePath?: string;
  manifestBrowsers: string[];
};

const targets: SmokeTarget[] = [
  { browser: 'chrome', extensionDir: path.join(extensionRoot, 'dist/chrome'), executablePath: findChromeForTesting(), manifestBrowsers: ['chrome-for-testing'] },
  { browser: 'edge', extensionDir: path.join(extensionRoot, 'dist/edge'), channel: 'msedge', manifestBrowsers: ['edge'] },
  { browser: 'chromium', extensionDir: path.join(extensionRoot, 'dist/chromium'), channel: 'chromium', manifestBrowsers: ['chromium'] },
];

function binaryPath(name: 'rzn-browser' | 'rzn-native-host'): string {
  const envKey = name === 'rzn-browser' ? 'RZN_BROWSER_BIN' : 'RZN_NATIVE_HOST_BIN';
  const fromEnv = process.env[envKey];
  if (fromEnv) return fromEnv;
  const exe = process.platform === 'win32' ? `${name}.exe` : name;
  return path.join(repoRoot, 'target/debug', exe);
}

function requireBinaries() {
  for (const name of ['rzn-browser', 'rzn-native-host'] as const) {
    const binary = binaryPath(name);
    try {
      fs.accessSync(binary, fs.constants.X_OK);
    } catch {
      throw new Error(`missing executable ${binary}; run make build-rust`);
    }
  }
}

function envFor(home: string, appBase: string): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    HOME: home,
    USERPROFILE: home,
    XDG_CONFIG_HOME: path.join(home, '.config'),
    XDG_CACHE_HOME: path.join(home, '.cache'),
    XDG_DATA_HOME: path.join(home, '.local/share'),
    LOCALAPPDATA: path.join(home, 'AppData/Local'),
    APPDATA: path.join(home, 'AppData/Roaming'),
    RZN_APP_BASE_DIR: appBase,
    RZN_SUPERVISOR_APP_BASE: appBase,
  };
  for (const key of [
    'RZN_LOCAL_RUNTIME_SOCKET_PATH', 'RZN_SUPERVISOR_SOCKET_PATH',
    'RZN_LOCAL_RUNTIME_TOKEN_PATH', 'RZN_SUPERVISOR_TOKEN_PATH',
  ]) delete env[key];
  return env;
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
    throw new Error(`missing ${target.extensionDir}/manifest.json; build the ${target.browser} extension`);
  }
  if (target.browser === 'chrome' && !target.executablePath) {
    throw new Error('Chrome for Testing is unavailable; set RZN_CHROME_FOR_TESTING_BIN');
  }

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
}

async function extensionOrigin(context: BrowserContext): Promise<string> {
  let worker = context.serviceWorkers()[0];
  if (!worker) {
    worker = await context.waitForEvent('serviceworker', { timeout: 10_000 });
  }
  const url = new URL(worker.url());
  return `chrome-extension://${url.host}/`;
}

async function installNativeHost(target: SmokeTarget, origin: string, env: NodeJS.ProcessEnv, nativeHostPath = binaryPath('rzn-native-host'), profileDir?: string) {
  const manifests: string[] = [];
  for (const browser of target.manifestBrowsers) {
    const { stdout } = await runCli(
      [
        'native-host',
        'install',
        '--browser',
        browser,
        '--extension-origin',
        origin,
        '--native-host-path',
        nativeHostPath,
        '--json',
      ],
      env
    );
    const manifestPath = JSON.parse(stdout).reports?.[0]?.manifest_path;
    if (typeof manifestPath !== 'string' || !fs.existsSync(manifestPath)) {
      throw new Error(`native-host install did not create a manifest for ${browser}: ${stdout}`);
    }
    manifests.push(manifestPath);
    if (profileDir) {
      const profileManifestDir = path.join(profileDir, 'NativeMessagingHosts');
      await fs.promises.mkdir(profileManifestDir, { recursive: true });
      const profileManifest = path.join(profileManifestDir, path.basename(manifestPath));
      await fs.promises.copyFile(manifestPath, profileManifest);
      manifests.push(profileManifest);
    }
  }
  return manifests;
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
      const { stdout } = await runCli(['supervisor', 'call', '--app-base', appBase, '--json', 'browser.targets', '--params', '{}'], env);
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

async function chaosTargetMetadata(appBase: string, env: NodeJS.ProcessEnv): Promise<any> {
  const { stdout } = await runCli(['browser', 'targets', '--app-base', appBase, '--json'], env);
  const parsed = JSON.parse(stdout);
  const target = (parsed.targets ?? []).find((t: any) => (t.extension_target ?? t.browser) === chaosBrowser);
  return target?.metadata;
}

async function chaosNativeHostPid(appBase: string, env: NodeJS.ProcessEnv): Promise<number | undefined> {
  const pid = (await chaosTargetMetadata(appBase, env))?.native_host_pid;
  return typeof pid === 'number' ? pid : undefined;
}

async function waitForNativeHostPid(appBase: string, env: NodeJS.ProcessEnv, notPid?: number, timeoutMs = CHAOS_RECOVERY_BOUND_MS): Promise<number> {
  const deadline = Date.now() + timeoutMs;
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

async function waitForAutoRespawn(appBase: string, env: NodeJS.ProcessEnv, oldPid: number): Promise<number> {
  const deadline = Date.now() + CHAOS_RECOVERY_BOUND_MS;
  let last = '';
  while (Date.now() < deadline) {
    const status = await supervisorCall(appBase, env, 'runtime.status', {}, 5_000);
    const pid = status.json?.pid;
    if (typeof pid === 'number' && pid !== oldPid) {
      const targetsResult = await supervisorCall(appBase, env, 'browser.targets', {}, 5_000);
      const seen = (targetsResult.json?.targets ?? []).some(
        (target: any) => (target.extension_target ?? target.browser) === chaosBrowser
      );
      if (seen) return pid;
      last = targetsResult.raw;
    } else {
      last = status.raw;
    }
    await sleep(500);
  }
  throw new Error(`Timed out waiting for native-host auto-respawn after supervisor ${oldPid}. Last: ${last}`);
}

// One "run": open a session on `url`, execute one step, close the session.
async function runOnce(appBase: string, env: NodeJS.ProcessEnv, url: string, step: Record<string, unknown>, trace = false) {
  const open = await supervisorCall(appBase, env, 'browser.session_open', { url });
  if (trace) console.log('Chaos auto-respawn: next session open returned');
  expect(stepSucceeded(open.json), `session_open failed: ${open.raw}`).toBe(true);
  const sessionId = open.json.session_id as string;
  const res = await supervisorCall(appBase, env, 'browser.execute_step', { session_id: sessionId, step });
  if (trace) console.log('Chaos auto-respawn: next step returned');
  await supervisorCall(appBase, env, 'browser.session_close', { session_id: sessionId }, 15_000);
  if (trace) console.log('Chaos auto-respawn: next session close returned');
  return res;
}

type ChaosServer = { base: string; hits: Map<string, number>; close: () => Promise<void> };

async function startChaosServer(): Promise<ChaosServer> {
  const pages: Record<string, string> = {
    '/idle': '<!doctype html><title>idle</title><p>idle</p>',
    '/typing': '<!doctype html><title>typing</title><textarea id="slow"></textarea>',
    '/submit': '<!doctype html><title>submit</title><textarea id="message"></textarea>',
    '/submit-hit': 'ok',
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
    close: () => new Promise<void>((resolve) => {
      server.close(() => resolve());
      server.closeAllConnections();
    }),
  };
}

// Use the full Chromium build for extension and native messaging coverage.
// Override for local browser runs.
const chaosBrowser = process.env.RZN_E2E_CHAOS_BROWSER ?? 'chromium';

// Same boot sequence as the kill-and-replace case: discover the origin, install
// the manifest, start the supervisor, launch the browser, wait for the bridge.
async function bootChaosLane(prefix: string, autoRespawn = false) {
  const target = targets.find((candidate) => candidate.browser === chaosBrowser);
  if (!target) throw new Error(`unknown RZN_E2E_CHAOS_BROWSER=${chaosBrowser}`);
  requireBinaries();

  const root = await fs.promises.realpath(await fs.promises.mkdtemp(path.join(tmpRoot, `rzn-${prefix}-`)));
  const home = path.join(root, 'home');
  const appBase = path.join(root, 'app-base');
  const liveProfile = path.join(root, 'profile-live');
  const fixtureHostPath = path.join(appBase, 'bin', path.basename(binaryPath('rzn-native-host')));
  const fixtureSupervisorPath = path.join(appBase, 'bin', path.basename(binaryPath('rzn-browser')));
  const env = envFor(home, appBase);
  let firstContext: BrowserContext | undefined;
  let context: BrowserContext | undefined;
  let server: ChaosServer | undefined;
  let supervisor: ReturnType<typeof execFile> | undefined;
  let browserPid: number | undefined;
  let manifests: string[] = [];
  let liveWorkerState: unknown = 'live worker readiness not reached';
  const supervisorLogs: string[] = [];
  const browserConsole: string[] = [];
  const cleanupErrors: Error[] = [];
  const commandForPid = async (pid: number) => execFileAsync('ps', ['-ww', '-p', String(pid), '-o', 'command='], { timeout: 2_000 })
    .then((result) => result.stdout.trim(), () => '');
  const ownedPids = async (executable: string) => {
    const { stdout } = await execFileAsync('ps', ['-ww', '-axo', 'pid=,command='], { timeout: 2_000, maxBuffer: 4_000_000 });
    return stdout.split('\n').flatMap((line) => {
      const match = line.match(/^\s*(\d+)\s+(.+)$/);
      return match && (match[2] === executable || match[2].startsWith(`${executable} `)) ? [Number(match[1])] : [];
    });
  };
  const stopOwned = async (executable: string) => {
    for (const signal of ['SIGTERM', 'SIGKILL'] as const) {
      const pids = await ownedPids(executable);
      if (pids.length === 0) return;
      for (const pid of pids) {
        try { process.kill(pid, signal); }
        catch (cause: any) { if (cause?.code !== 'ESRCH') throw cause; }
      }
      for (let attempt = 0; attempt < 20; attempt += 1) {
        if ((await ownedPids(executable)).length === 0) return;
        await sleep(100);
      }
    }
    throw new Error(`fixture process still alive: ${executable} PIDs ${(await ownedPids(executable)).join(',')}`);
  };
  const cleanupStep = async (name: string, run: () => Promise<unknown>, timeoutMs = 10_000): Promise<boolean> => {
    if (autoRespawn) console.log(`Chaos cleanup ${prefix}: ${name} start`);
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([
        run(),
        new Promise<never>((_, reject) => {
          timer = setTimeout(() => reject(new Error(`${name} exceeded ${timeoutMs}ms`)), timeoutMs);
        }),
      ]);
      if (autoRespawn) console.log(`Chaos cleanup ${prefix}: ${name} done`);
      return true;
    } catch (cause) {
      console.error(`Chaos cleanup ${prefix}: ${name} failed: ${String(cause)}`);
      cleanupErrors.push(new Error(`${name}: ${String(cause)}`));
      return false;
    } finally {
      if (timer) clearTimeout(timer);
    }
  };
  const dumpDiagnostics = async () => {
    try {
      const diagnostic: Record<string, unknown> = { root, appBase, manifests, supervisor_stderr: supervisorLogs.join('').slice(-4_000) };
      diagnostic.worker = liveWorkerState;
      diagnostic.worker_urls = context?.serviceWorkers().map((worker) => worker.url()) ?? [];
      diagnostic.browser_console = browserConsole.slice(-12);
      const hostPids = await ownedPids(fixtureHostPath);
      diagnostic.native_host_pids = hostPids;
      if (hostPids.length && !fs.existsSync(path.join(appBase, 'logs/native-host.log'))) {
        diagnostic.native_host_processes = await execFileAsync('ps', ['-p', hostPids.join(','), '-o', 'pid=,stat=,etime=,comm='], { timeout: 2_000 })
          .then((result) => result.stdout, (cause) => String(cause));
        if (process.platform === 'darwin') {
          const samplePath = path.join(root, 'native-host-startup.sample.txt');
          await execFileAsync('sample', [String(hostPids[0]), '1', '1', '-file', samplePath], { timeout: 5_000 })
            .catch((cause) => { diagnostic.native_host_sample_error = String(cause); });
          if (fs.existsSync(samplePath)) diagnostic.native_host_sample = fs.readFileSync(samplePath, 'utf8').slice(0, 14_000);
        }
      }
      diagnostic.manifest_contents = manifests.map((manifest) => {
        try { return { path: manifest, value: JSON.parse(fs.readFileSync(manifest, 'utf8')) }; }
        catch (cause) { return { path: manifest, error: String(cause) }; }
      });
      diagnostic.supervisor_status = await supervisorCall(appBase, env, 'runtime.status', {}, 5_000)
        .then((result) => result.raw, (cause) => String(cause));
      for (const [name, file] of [
        ['native_host_log', path.join(appBase, 'logs/native-host.log')],
        ['supervisor_log', path.join(appBase, 'run/supervisor.log')],
      ]) {
        if (fs.existsSync(file)) diagnostic[name] = fs.readFileSync(file, 'utf8').split('\n').slice(-25).join('\n');
      }
      console.error(`Chaos diagnostics: ${JSON.stringify(diagnostic)}`);
    } catch (diagnosticError) {
      console.error(`Chaos diagnostics failed: ${String(diagnosticError)}`);
    }
  };
  const cleanup = async () => {
    if (firstContext) await cleanupStep('first context', () => firstContext!.close());
    const capturedHosts = await ownedPids(fixtureHostPath).catch((cause) => {
      cleanupErrors.push(new Error(`native-host PID capture: ${String(cause)}`));
      return [];
    });
    if (autoRespawn) console.log(`Chaos cleanup ${prefix}: owned native-host PIDs ${capturedHosts.join(',') || 'none'}`);
    // Finish this fixture's process-tree cleanup before awaiting Playwright's
    // close promise, which can remain pending after the browser PID exits.
    let browserCloseError: unknown;
    const browserClose = context?.close().catch((cause) => { browserCloseError = cause; });
    if (autoRespawn && context) console.log(`Chaos cleanup ${prefix}: browser context close requested`);
    if (browserClose && browserPid) await cleanupStep('browser process exit', async () => {
      for (let attempt = 0; attempt < 50; attempt += 1) {
        const command = await commandForPid(browserPid!);
        if (!command.includes(`--user-data-dir=${liveProfile}`)) return;
        await sleep(100);
      }
      const command = await commandForPid(browserPid!);
      if (!command.includes(`--user-data-dir=${liveProfile}`)) return;
      console.error(`Chaos cleanup ${prefix}: force killing owned browser PID ${browserPid}`);
      try { process.kill(browserPid!, 'SIGKILL'); }
      catch (cause: any) { if (cause?.code !== 'ESRCH') throw cause; }
      for (let attempt = 0; attempt < 50; attempt += 1) {
        if (!(await commandForPid(browserPid!)).includes(`--user-data-dir=${liveProfile}`)) return;
        await sleep(100);
      }
      throw new Error(`owned browser PID ${browserPid} survived SIGKILL`);
    }, 15_000);
    await cleanupStep('native-host exit', () => stopOwned(fixtureHostPath), 10_000);
    // After the browser and host are gone, shutdown cannot auto-respawn.
    if (supervisor) await supervisorCall(appBase, env, 'runtime.shutdown', {}, 5_000);
    await cleanupStep('fixture supervisor exit', () => stopOwned(fixtureSupervisorPath), 10_000);
    if (browserClose) await cleanupStep('browser context', async () => {
      await browserClose;
      if (browserCloseError) throw browserCloseError;
    });
    if (server) await cleanupStep('HTTP server', () => server!.close());
    supervisor?.kill();
    if (supervisor && supervisor.exitCode === null && supervisor.signalCode === null) {
      await cleanupStep('original supervisor exit', () => new Promise((resolve) => supervisor!.once('exit', resolve)));
    }
    // Browser profiles are ~300MB each; keep them only when debugging.
    if (process.env.RZN_E2E_KEEP_TMP !== '1' && cleanupErrors.length === 0) {
      await cleanupStep('temporary files', () => fs.promises.rm(root, { recursive: true, force: true }), 30_000);
    } else if (autoRespawn || cleanupErrors.length > 0) {
      console.log(`Chaos cleanup ${prefix}: retained ${root}`);
    }
    if (cleanupErrors.length) throw new AggregateError(cleanupErrors, `Chaos cleanup ${prefix} failed`);
  };

  try {
    let nativeHostPath = binaryPath('rzn-native-host');
    await fs.promises.mkdir(path.join(appBase, 'bin'), { recursive: true });
    for (const name of ['rzn-browser', 'rzn-native-host'] as const) {
      const source = binaryPath(name);
      const destination = path.join(appBase, 'bin', path.basename(source));
      await fs.promises.copyFile(source, destination);
      await fs.promises.chmod(destination, (await fs.promises.stat(source)).mode);
      if (name === 'rzn-native-host') nativeHostPath = destination;
    }
    // Validate cold, copied executables before measuring browser transport.
    // macOS can stall a fresh Mach-O path before Rust main (and logging).
    await execFileAsync(fixtureSupervisorPath, ['--help'], { env, timeout: 20_000 });
    await execFileAsync(fixtureHostPath, ['--self-test'], { env, timeout: 20_000 });
    const preflightLog = path.join(appBase, 'logs/native-host.log');
    if (fs.existsSync(preflightLog)) {
      await fs.promises.rename(preflightLog, path.join(appBase, 'logs/native-host-preflight.log'));
    }
    firstContext = await launchWithExtension(target, path.join(root, 'profile-first'), env);
    const origin = await extensionOrigin(firstContext);
    await firstContext.close();
    firstContext = undefined;
    manifests = await installNativeHost(target, origin, env, nativeHostPath, liveProfile);

    supervisor = execFile(fixtureSupervisorPath, ['supervisor', 'serve', '--app-base', appBase], { cwd: repoRoot, env });
    supervisor.stderr?.on('data', (chunk) => supervisorLogs.push(String(chunk)));
    let supervisorReady = false;
    const supervisorReadyBy = Date.now() + 10_000;
    while (Date.now() < supervisorReadyBy) {
      const status = await supervisorCall(appBase, env, 'runtime.status', {}, 1_000);
      if (status.json?.pid === supervisor.pid && status.json?.exe === fixtureSupervisorPath) {
        supervisorReady = true;
        break;
      }
      await sleep(100);
    }
    if (!supervisorReady) throw new Error(`copied fixture supervisor did not become ready: ${fixtureSupervisorPath}`);
    context = await launchWithExtension(target, liveProfile, env);
    const browser = context.browser();
    if (!browser) throw new Error('persistent context did not expose its browser for owned-PID capture');
    const browserSession = await browser.newBrowserCDPSession();
    try {
      const info = await browserSession.send('SystemInfo.getProcessInfo');
      browserPid = info.processInfo?.find((entry: any) => entry.type === 'browser')?.id;
    } finally {
      await browserSession.detach();
    }
    if (typeof browserPid !== 'number') throw new Error('browser CDP did not report the owned browser PID');
    if (autoRespawn) console.log(`Chaos ${prefix}: owned browser PID ${browserPid}`);
    context.on('console', (message) => {
      browserConsole.push(`${message.type()}: ${message.text()}`);
      if (browserConsole.length > 50) browserConsole.shift();
    });
    context.on('pageerror', (error) => {
      browserConsole.push(`pageerror: ${String(error)}`);
      if (browserConsole.length > 50) browserConsole.shift();
    });
    const expectedId = new URL(origin).host;
    const workerReadyBy = Date.now() + 10_000;
    while (Date.now() < workerReadyBy) {
      const liveWorker = context.serviceWorkers()[0];
      if (liveWorker) {
        try {
          liveWorkerState = await liveWorker.evaluate(() => {
            const runtime = (globalThis as any).chrome?.runtime;
            return {
              extension_id: runtime?.id,
              connect_native_type: typeof runtime?.connectNative,
              native_messaging_permission: runtime?.getManifest?.().permissions?.includes('nativeMessaging'),
              user_agent: navigator.userAgent,
            };
          });
        } catch (cause) {
          liveWorkerState = { error: String(cause) };
        }
        if ((liveWorkerState as any).extension_id === expectedId &&
            (liveWorkerState as any).connect_native_type === 'function' &&
            (liveWorkerState as any).native_messaging_permission === true) break;
      } else {
        liveWorkerState = 'no live service worker';
      }
      await sleep(200);
    }
    if ((liveWorkerState as any).extension_id !== expectedId ||
        (liveWorkerState as any).connect_native_type !== 'function' ||
        (liveWorkerState as any).native_messaging_permission !== true) {
      throw new Error(`live extension worker runtime not ready: ${JSON.stringify(liveWorkerState)}`);
    }
    server = await startChaosServer();
    await waitForTargets(appBase, env, [target.browser]);
    return {
      appBase,
      env,
      context,
      server,
      dumpDoctor: async () => {
        try { console.log(await doctorOutput(target, origin, appBase, env)); }
        catch (cause) { console.error(`Chaos doctor failed: ${String(cause)}`); }
        finally { await dumpDiagnostics(); }
      },
      cleanup,
    };
  } catch (error) {
    await dumpDiagnostics();
    await cleanup().catch(() => {});
    throw error;
  }
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
      requireBinaries();

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
    requireBinaries();

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
      if (supervisor.exitCode === null && supervisor.signalCode === null) {
        await new Promise((resolve) => supervisor.once('exit', resolve));
      }
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

  test('chaos: native host respawns a killed supervisor and the next action succeeds', async () => {
    test.setTimeout(CHAOS_TEST_TIMEOUT_MS);
    const lane = await bootChaosLane('supervisor-kill-chaos', true);
    console.log('Chaos auto-respawn: lane ready');
    try {
      const { appBase, env, server } = lane;
      const status = await supervisorCall(appBase, env, 'runtime.status', {});
      const oldPid = status.json?.pid;
      expect(typeof oldPid, `missing supervisor pid: ${status.raw}`).toBe('number');

      const open = await supervisorCall(appBase, env, 'browser.session_open', { url: `${server.base}/idle` });
      expect(stepSucceeded(open.json), `session_open failed: ${open.raw}`).toBe(true);
      const inFlight = supervisorCall(appBase, env, 'browser.execute_step', {
        session_id: open.json.session_id, step: LONG_STEP,
      });
      await sleep(2_000);
      const killedAt = Date.now();
      process.kill(oldPid, 'SIGKILL');
      console.log(`Chaos auto-respawn: killed supervisor ${oldPid}`);
      const failed = await inFlight;
      console.log(`Chaos auto-respawn: in-flight call returned in ${Date.now() - killedAt}ms`);
      expect(stepSucceeded(failed.json), `step should fail when supervisor dies: ${failed.raw}`).toBe(false);
      expect(Date.now() - killedAt, `failure took too long: ${failed.raw}`).toBeLessThan(CHAOS_FAIL_BOUND_MS);
      expect(failed.raw).not.toContain('CLI_KILLED_BY_TEST_TIMEOUT');

      const newPid = await waitForAutoRespawn(appBase, env, oldPid);
      console.log(`Chaos auto-respawn: replacement ${newPid} ready`);
      expect(newPid).not.toBe(oldPid);
      const next = await runOnce(appBase, env, `${server.base}/idle`, { type: 'get_current_url' }, true);
      console.log('Chaos auto-respawn: next action complete');
      expect(stepSucceeded(next.json), `next run after auto-respawn failed: ${next.raw}`).toBe(true);
    } catch (error) {
      await lane.dumpDoctor();
      throw error;
    } finally {
      await lane.cleanup();
    }
  });

  test('chaos: extension service worker stopped mid-run fails named and the next run succeeds', async () => {
    test.setTimeout(CHAOS_TEST_TIMEOUT_MS);
    const lane = await bootChaosLane('service-worker-chaos');
    try {
      const pid = await waitForNativeHostPid(lane.appBase, lane.env);
      const oldWorkerBootId = (await chaosTargetMetadata(lane.appBase, lane.env))?.extension_worker_boot_id;
      expect(typeof oldWorkerBootId, 'initial extension worker boot id must be reported').toBe('string');
      const worker = lane.context.serviceWorkers()[0] ?? await lane.context.waitForEvent('serviceworker', { timeout: 10_000 });
      const page = lane.context.pages()[0] ?? await lane.context.newPage();
      const cdp = await lane.context.newCDPSession(page);
      const versions = new Map<string, { versionId: string; scriptURL: string; runningStatus: string }>();
      cdp.on('ServiceWorker.workerVersionUpdated', (event: any) => {
        for (const version of event.versions ?? []) versions.set(version.versionId, version);
      });
      await cdp.send('ServiceWorker.enable');
      const runningVersion = () => [...versions.values()].find(
        (version) => version.scriptURL === worker.url() && version.runningStatus === 'running'
      );
      await expect.poll(() => runningVersion()?.versionId, { timeout: 10_000 }).toBeTruthy();
      const versionId = runningVersion()!.versionId;
      await expectNamedFailureAfter(lane.appBase, lane.env, lane.server.base, async () => {
        await cdp.send('ServiceWorker.stopWorker', { versionId });
      });
      // The native host and the extension's per-boot worker identity must both
      // change through normal browser lifecycle, without a manual reconnect.
      await waitForNativeHostPid(lane.appBase, lane.env, pid, 45_000);
      await expect.poll(async () => {
        const metadata = await chaosTargetMetadata(lane.appBase, lane.env);
        const bootId = metadata?.extension_worker_boot_id;
        return typeof bootId === 'string' && bootId.length > 0 && bootId !== oldWorkerBootId && metadata.native_host_pid !== pid;
      }, { timeout: 45_000 }).toBe(true);
      const next = await runOnce(lane.appBase, lane.env, `${lane.server.base}/idle`, { type: 'get_current_url' });
      expect(stepSucceeded(next.json), `next run failed: ${next.raw}`).toBe(true);
    } catch (error) {
      await lane.dumpDoctor();
      throw error;
    } finally {
      await lane.cleanup();
    }
  });

  test('chaos: cancelling a run stops typing in its retained page', async () => {
    test.setTimeout(CHAOS_TEST_TIMEOUT_MS);
    const lane = await bootChaosLane('cancel-typing-chaos');
    const runId = `cancel-typing-${Date.now()}`;
    let sessionId: string | undefined;
    try {
      const { appBase, env, server, context } = lane;
      const claim = await supervisorCall(appBase, env, 'runs.claim', {
        run_id: runId, workflow_id: 'e2e/cancel-typing', origin: 'local_cli', step_total: 1,
      });
      expect(claim.json?.ok, `runs.claim failed: ${claim.raw}`).toBe(true);

      const open = await supervisorCall(appBase, env, 'browser.session_open', {
        url: `${server.base}/typing`, run_id: runId, origin: 'local_cli',
      });
      expect(stepSucceeded(open.json), `session_open failed: ${open.raw}`).toBe(true);
      sessionId = open.json.session_id as string;
      const page = context.pages().find((candidate) => candidate.url().startsWith(`${server.base}/typing`));
      expect(page, 'typing page must remain open through cancellation').toBeDefined();
      const input = page!.locator('#slow');
      const text = 'x'.repeat(160);
      const typing = supervisorCall(appBase, env, 'browser.execute_step', {
        session_id: sessionId,
        step: { type: 'type_text', selector: '#slow', text, simulate_typing: true, delay_ms: 75, use_native_input: false, use_cdp: false },
      });

      await expect.poll(async () => (await input.inputValue()).length, { timeout: 10_000 })
        .toBeGreaterThanOrEqual(2);
      expect((await input.inputValue()).length, 'typing should still be in progress').toBeLessThan(text.length);
      const cancel = await supervisorCall(appBase, env, 'runs.cancel', { run_id: runId });
      expect(cancel.json?.ok, `runs.cancel failed: ${cancel.raw}`).toBe(true);
      const stopped = await typing;
      expect(stepSucceeded(stopped.json), `cancelled step should fail: ${stopped.raw}`).toBe(false);
      const settledLength = (await input.inputValue()).length;
      expect(settledLength, 'cancelled typing must stop before the full value').toBeLessThan(text.length);
      await sleep(350);
      expect((await input.inputValue()).length, 'page must not mutate after cancel is acknowledged').toBe(settledLength);
    } catch (error) {
      await lane.dumpDoctor();
      throw error;
    } finally {
      try {
        if (sessionId) await supervisorCall(lane.appBase, lane.env, 'browser.session_close', { session_id: sessionId }, 15_000);
      } finally {
        try { await supervisorCall(lane.appBase, lane.env, 'runs.release', { run_id: runId }); }
        finally { await lane.cleanup(); }
      }
    }
  });

  test('chaos: cancelled fill cannot submit when a button appears later', async () => {
    test.setTimeout(CHAOS_TEST_TIMEOUT_MS);
    const lane = await bootChaosLane('cancel-submit-chaos');
    const runId = `cancel-submit-${Date.now()}`;
    let sessionId: string | undefined;
    try {
      const { appBase, env, server, context } = lane;
      const claim = await supervisorCall(appBase, env, 'runs.claim', {
        run_id: runId, workflow_id: 'e2e/cancel-submit', origin: 'local_cli', step_total: 1,
      });
      expect(claim.json?.ok, `runs.claim failed: ${claim.raw}`).toBe(true);
      const open = await supervisorCall(appBase, env, 'browser.session_open', {
        url: `${server.base}/submit`, run_id: runId, origin: 'local_cli',
      });
      expect(stepSucceeded(open.json), `session_open failed: ${open.raw}`).toBe(true);
      sessionId = open.json.session_id as string;
      const page = context.pages().find((candidate) => candidate.url().startsWith(`${server.base}/submit`));
      expect(page, 'submit page must remain open through cancellation').toBeDefined();
      const submit = supervisorCall(appBase, env, 'browser.execute_step', {
        session_id: sessionId,
        step: { type: 'fill_and_submit', selector: '#message', value: 'local only', submit_selector: '#send', timeout_ms: 5_000, wait_timeout_ms: 0 },
      });
      await expect.poll(() => page!.locator('#message').inputValue(), { timeout: 8_000 }).toBe('local only');
      const cancel = await supervisorCall(appBase, env, 'runs.cancel', { run_id: runId });
      expect(cancel.json?.ok, `runs.cancel failed: ${cancel.raw}`).toBe(true);
      const stopped = await submit;
      expect(stepSucceeded(stopped.json), `cancelled submit should fail: ${stopped.raw}`).toBe(false);
      expect(page!.isClosed(), 'cancelled page should remain open for observation').toBe(false);
      await page!.evaluate(() => {
        const button = document.createElement('button');
        button.id = 'send';
        button.textContent = 'Send';
        button.addEventListener('click', () => { void fetch('/submit-hit'); });
        document.body.append(button);
      });
      await sleep(800);
      expect(server.hits.get('/submit-hit') ?? 0, 'cancelled action must not click the later button').toBe(0);
    } catch (error) {
      await lane.dumpDoctor();
      throw error;
    } finally {
      try {
        if (sessionId) await supervisorCall(lane.appBase, lane.env, 'browser.session_close', { session_id: sessionId }, 15_000);
      } finally {
        try { await supervisorCall(lane.appBase, lane.env, 'runs.release', { run_id: runId }); }
        finally { await lane.cleanup(); }
      }
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
    requireBinaries();

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
