import { chromium, test, expect, type BrowserContext } from '@playwright/test';
import { execFile } from 'node:child_process';
import fs from 'node:fs';
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

type SmokeTarget = {
  browser: 'chrome' | 'edge' | 'chromium';
  extensionDir: string;
  channel?: 'chrome' | 'msedge';
};

const targets: SmokeTarget[] = [
  { browser: 'chrome', extensionDir: path.join(extensionRoot, 'dist/chrome'), channel: 'chrome' },
  { browser: 'edge', extensionDir: path.join(extensionRoot, 'dist/edge'), channel: 'msedge' },
  { browser: 'chromium', extensionDir: path.join(extensionRoot, 'dist/chromium') },
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

  try {
    return await chromium.launchPersistentContext(userDataDir, {
      headless: process.env.RZN_PW_HEADFUL !== '1',
      channel: target.channel,
      env,
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
  await runCli(
    [
      'native-host',
      'install',
      '--browser',
      target.browser,
      '--extension-origin',
      origin,
      '--native-host-path',
      binaryPath('rzn-native-host'),
      '--json',
    ],
    env
  );
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

test.describe('native-host browser smoke', () => {
  test.skip(!smokeEnabled, 'set RZN_E2E_NATIVE_HOST_SMOKE=1 to run local native-host smoke tests');

  for (const target of targets) {
    test(`${target.browser} extension connects to native host and appears in browser targets`, async () => {
      test.skip(!fs.existsSync(binaryPath('rzn-browser')), `missing ${binaryPath('rzn-browser')}`);
      test.skip(!fs.existsSync(binaryPath('rzn-native-host')), `missing ${binaryPath('rzn-native-host')}`);

      const root = await fs.promises.mkdtemp(path.join(os.tmpdir(), `rzn-${target.browser}-smoke-`));
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

  test('Chrome and Edge can connect simultaneously and route separately', async () => {
    const chrome = targets[0];
    const edge = targets[1];
    test.skip(!fs.existsSync(binaryPath('rzn-browser')), `missing ${binaryPath('rzn-browser')}`);
    test.skip(!fs.existsSync(binaryPath('rzn-native-host')), `missing ${binaryPath('rzn-native-host')}`);

    const root = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'rzn-chrome-edge-smoke-'));
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
