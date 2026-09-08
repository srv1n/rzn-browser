import assert from 'node:assert/strict';
import { afterEach, beforeEach, describe, test, vi } from 'vitest';
import { CDPClient } from './cdpClient';
import { frameRouter } from './frameRouter';

vi.mock('./frameRouter', () => ({
  frameRouter: {
    routeForFrame: (frameId: string) => ({ sessionId: `routed:${frameId}` }),
    markTabDetached: () => {},
  },
}));

type Command = {
  target: { tabId?: number; extensionId?: string; sessionId?: string };
  method: string;
  params: any;
  callback: (result?: any) => void;
};

// A controlled callback/clock harness: no real debugger or timing-dependent sleeps.
describe('CDP client transport and domain lifecycle', () => {
  let client: CDPClient;
  let commands: Command[];
  let timers: Map<number, () => void>;
  let detached: number[];
  let lastError: { message: string } | undefined;
  let lastErrorReads: number;
  let onSend: (command: Command) => void;
  let savedGlobals: Map<string, PropertyDescriptor | undefined>;

  function reply(command: Command, result: any = {}, error?: string): void {
    lastError = error ? { message: error } : undefined;
    try { command.callback(result); } finally { lastError = undefined; }
  }

  async function flush(): Promise<void> {
    for (let i = 0; i < 12; i++) await Promise.resolve();
  }

  beforeEach(() => {
    commands = [];
    timers = new Map();
    detached = [];
    lastError = undefined;
    lastErrorReads = 0;
    let timerId = 0;
    savedGlobals = new Map(['chrome', 'setTimeout', 'clearTimeout'].map(key =>
      [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
    Object.assign(globalThis, {
      setTimeout: (callback: () => void) => {
        timers.set(++timerId, callback);
        return timerId;
      },
      clearTimeout: (id: number) => timers.delete(id),
      chrome: {
        runtime: {
          id: 'extension-under-test',
          get lastError() { lastErrorReads++; return lastError; },
        },
        debugger: {
          sendCommand: (target: Command['target'], method: string, params: any,
                        callback: Command['callback']) => {
            const command = { target, method, params, callback };
            commands.push(command);
            onSend(command);
          },
        },
      },
    });
    frameRouter.markTabDetached = (tabId: number) => { detached.push(tabId); };
    onSend = command => reply(command);
    client = new CDPClient();
  });

  afterEach(() => {
    for (const [key, descriptor] of savedGlobals) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else Reflect.deleteProperty(globalThis, key);
    }
    assert.equal(timers.size, 0, 'all command timers must be released');
  });

  test('routes tab zero and preserves the original CDP params', async () => {
    const params = Object.freeze({ expression: '1 + 1' });
    await client.sendCommand({ tabId: 0 }, 'Runtime.evaluate', params);
    assert.deepEqual(commands[0].target, { tabId: 0 });
    assert.equal(commands[0].params, params);
  });

  test('places a child session on the debugger target, not protocol params', async () => {
    await client.sendCommand({ tabId: 7, sessionId: 'child' }, 'DOM.getDocument');
    assert.deepEqual(commands[0].target, { tabId: 7, sessionId: 'child' });
    assert.deepEqual(commands[0].params, {});
  });

  test('explicit session overrides the target and frame route', async () => {
    await client.sendCommand({ tabId: 7, sessionId: 'target' }, 'DOM.getDocument', {},
      { frameId: 'frame', sessionId: 'explicit' });
    assert.equal(commands[0].target.sessionId, 'explicit');
  });

  test('resolves frame sessions when no session is explicit', async () => {
    await client.sendCommand({ tabId: 7 }, 'DOM.getDocument', {}, { frameId: 'frame' });
    assert.equal(commands[0].target.sessionId, 'routed:frame');
  });

  test('preserves the extension target fallback', async () => {
    await client.sendCommand({}, 'Runtime.evaluate');
    assert.deepEqual(commands[0].target, { extensionId: 'extension-under-test' });
  });

  test('returns successful results and clears the watchdog', async () => {
    const result = { value: 42 };
    onSend = command => reply(command, result);
    assert.equal(await client.sendCommand({ tabId: 1 }, 'Runtime.evaluate'), result);
    assert.equal(timers.size, 0);
  });

  test('clears the watchdog on a synchronous Chrome API throw', async () => {
    const error = new Error('invalid debugger arguments');
    onSend = () => { throw error; };
    await assert.rejects(client.sendCommand({ tabId: 1 }, 'DOM.enable'), e => e === error);
    assert.equal(timers.size, 0);
  });

  for (const [message, code, detaches] of [
    ['invalid parameters', 'CDP_COMMAND_FAILED', false],
    ['Debugger is not attached', 'CDP_TARGET_DETACHED', true],
  ] as const) {
    test(`preserves error classification: ${code}`, async () => {
      onSend = command => reply(command, undefined, message);
      await assert.rejects(client.sendCommand({ tabId: 1 }, 'DOM.enable'),
        (error: any) => error.code === code && error.message.includes(message));
      assert.deepEqual(detached, detaches ? [1] : []);
    });
  }

  test('ignores late callbacks but consumes lastError after a timeout', async () => {
    onSend = () => {};
    const pending = client.sendCommand({ tabId: 1 }, 'DOM.enable', {}, { timeout: 1 });
    const rejected = assert.rejects(pending, /CDP command timeout: DOM.enable/);
    const [id, fire] = [...timers][0];
    timers.delete(id);
    fire();
    await rejected;
    reply(commands[0], undefined, 'Debugger is not attached');
    reply(commands[0], { late: true });
    assert.equal(lastErrorReads, 2);
    assert.deepEqual(detached, []);
  });

  test('evaluate routes frameId without leaking it into Runtime.evaluate params', async () => {
    await client.evaluate({ tabId: 1 }, 'answer', { frameId: 'frame', timeout: 500, returnByValue: false });
    assert.deepEqual(commands[0].params,
      { expression: 'answer', returnByValue: false, awaitPromise: true, timeout: 500 });
    assert.equal(commands[0].target.sessionId, 'routed:frame');
  });

  test('shares one domain enable and disables only after the final release', async () => {
    await Promise.all([client.enableDomains({ tabId: 1 }, ['DOM']), client.enableDomains({ tabId: 1 }, ['DOM'])]);
    await client.disableDomains({ tabId: 1 }, ['DOM']);
    assert.deepEqual(commands.map(c => c.method), ['DOM.enable']);
    await client.disableDomains({ tabId: 1 }, ['DOM']);
    await client.disableDomains({ tabId: 1 }, ['DOM', 'Console', 'Unheld']);
    assert.deepEqual(commands.map(c => c.method), ['DOM.enable', 'DOM.disable']);
    await flush();
    assert.equal((client as any).domainRefs.size, 0);
    assert.equal((client as any).domainOperations.size, 0);
  });

  test('a concurrent borrower waits for Chrome to finish enabling the domain', async () => {
    onSend = () => {};
    let secondDone = false;
    const first = client.enableDomains({ tabId: 1 }, ['DOM']);
    const second = client.enableDomains({ tabId: 1 }, ['DOM']).then(() => { secondDone = true; });
    await flush();
    assert.equal(commands.length, 1);
    assert.equal(secondDone, false);
    reply(commands[0]);
    await Promise.all([first, second]);
    assert.equal(secondDone, true);
  });

  test('a failed enable does not poison the next queued acquisition', async () => {
    onSend = command => reply(command, {}, commands.length === 1 ? 'enable failed' : undefined);
    const failed = assert.rejects(client.enableDomains({ tabId: 1 }, ['DOM']), /enable failed/);
    const retry = client.enableDomains({ tabId: 1 }, ['DOM']);
    await Promise.all([failed, retry]);
    await client.disableDomains({ tabId: 1 }, ['DOM']);
    assert.deepEqual(commands.map(c => c.method), ['DOM.enable', 'DOM.enable', 'DOM.disable']);
  });

  test('rolls back newly acquired domains after a partial batch failure', async () => {
    onSend = command => reply(command, {}, command.method === 'Page.enable' ? 'page failed' : undefined);
    await assert.rejects(client.enableDomains({ tabId: 1 }, ['DOM', 'Page']), /page failed/);
    assert.deepEqual(commands.map(c => c.method), ['DOM.enable', 'Page.enable', 'DOM.disable']);
    assert.equal((client as any).domainRefs.size, 0);
    await client.enableDomains({ tabId: 1 }, ['DOM']);
    assert.equal(commands.at(-1)?.method, 'DOM.enable');
  });

  test('rollback preserves references owned by an earlier caller', async () => {
    await client.enableDomains({ tabId: 1 }, ['DOM']);
    onSend = command => reply(command, {}, command.method === 'Page.enable' ? 'page failed' : undefined);
    await assert.rejects(client.enableDomains({ tabId: 1 }, ['DOM', 'Page']), /page failed/);
    assert.deepEqual(commands.map(c => c.method), ['DOM.enable', 'Page.enable']);
    await client.disableDomains({ tabId: 1 }, ['DOM']);
    assert.equal(commands.at(-1)?.method, 'DOM.disable');
  });

  test('a failed rollback retains the original error and permits later acquisition', async () => {
    onSend = command => reply(command, {}, command.method === 'Page.enable' ? 'original failure' :
      command.method === 'DOM.disable' ? 'cleanup failure' : undefined);
    await assert.rejects(client.enableDomains({ tabId: 1 }, ['DOM', 'Page']), /original failure/);
    assert.equal((client as any).domainRefs.size, 0);
    onSend = command => reply(command);
    await client.enableDomains({ tabId: 1 }, ['DOM']);
    assert.equal(commands.at(-1)?.method, 'DOM.enable');
  });

  test('a release cannot overtake a pending enable', async () => {
    onSend = () => {};
    const enabling = client.enableDomains({ tabId: 1 }, ['DOM']);
    const disabling = client.disableDomains({ tabId: 1 }, ['DOM']);
    await flush();
    assert.deepEqual(commands.map(c => c.method), ['DOM.enable']);
    onSend = command => reply(command);
    reply(commands[0]);
    await Promise.all([enabling, disabling]);
    assert.deepEqual(commands.map(c => c.method), ['DOM.enable', 'DOM.disable']);
  });

  test('keeps child-frame domain accounting separate on the same tab', async () => {
    await client.enableDomains({ tabId: 1 }, ['DOM'], 'a');
    await client.enableDomains({ tabId: 1 }, ['DOM'], 'b');
    await client.disableDomains({ tabId: 1 }, ['DOM'], 'a');
    await client.disableDomains({ tabId: 1 }, ['DOM'], 'b');
    assert.deepEqual(commands.map(c => [c.method, c.target.sessionId]), [
      ['DOM.enable', 'routed:a'], ['DOM.enable', 'routed:b'],
      ['DOM.disable', 'routed:a'], ['DOM.disable', 'routed:b'],
    ]);
  });

  test('keys a session by its parent tab as well as its session ID', async () => {
    await client.enableDomains({ tabId: 1, sessionId: 'child' }, ['DOM']);
    await client.enableDomains({ tabId: 2, sessionId: 'child' }, ['DOM']);
    assert.equal(commands.length, 2);
  });

  test('does not block another tab or ordinary commands behind a pending lease', async () => {
    onSend = command => { if (command.method !== 'DOM.enable' || command.target.tabId !== 1) reply(command); };
    const pending = client.enableDomains({ tabId: 1 }, ['DOM']);
    await flush();
    await client.enableDomains({ tabId: 2 }, ['DOM']);
    await client.sendCommand({ tabId: 1 }, 'Runtime.evaluate');
    assert.equal(commands.length, 3);
    reply(commands[0]);
    await pending;
  });

  test('ignores Console and unheld releases without protocol traffic', async () => {
    await client.enableDomains({ tabId: 1 }, ['Console']);
    await client.disableDomains({ tabId: 1 }, ['Console', 'DOM']);
    assert.equal(commands.length, 0);
  });

  test('empty and Console-only acquisitions do not resolve unused frame routes', async () => {
    const originalRoute = frameRouter.routeForFrame;
    frameRouter.routeForFrame = () => { throw new Error('unused frame'); };
    try {
      await client.enableDomains({ tabId: 1 }, [], 'missing');
      await client.enableDomains({ tabId: 1 }, ['Console'], 'missing');
      assert.equal(commands.length, 0);
    } finally {
      frameRouter.routeForFrame = originalRoute;
    }
  });

  test('a failed disable removes bookkeeping and does not block a fresh enable', async () => {
    await client.enableDomains({ tabId: 1 }, ['DOM']);
    onSend = command => reply(command, {}, 'disable failed');
    await client.disableDomains({ tabId: 1 }, ['DOM']);
    assert.equal((client as any).domainRefs.size, 0);
    onSend = command => reply(command);
    await client.enableDomains({ tabId: 1 }, ['DOM']);
    assert.deepEqual(commands.map(c => c.method), ['DOM.enable', 'DOM.disable', 'DOM.enable']);
  });

  test('preserves duplicate-domain reference counts', async () => {
    await client.enableDomains({ tabId: 1 }, ['DOM', 'DOM']);
    await client.disableDomains({ tabId: 1 }, ['DOM']);
    assert.equal(commands.length, 1);
    await client.disableDomains({ tabId: 1 }, ['DOM']);
    assert.deepEqual(commands.map(c => c.method), ['DOM.enable', 'DOM.disable']);
  });

  test('cleans up session bookkeeping after repeated acquire/release cycles', async () => {
    for (let tabId = 0; tabId < 50; tabId++) {
      await client.enableDomains({ tabId }, ['DOM']);
      await client.disableDomains({ tabId }, ['DOM']);
    }
    await flush();
    assert.equal((client as any).domainRefs.size, 0);
    assert.equal((client as any).domainOperations.size, 0);
  });
});
