import assert from 'node:assert/strict';
import { afterEach, beforeEach, describe, test } from 'vitest';
import { CDPClient } from './cdpClient';
import { frameRouter } from './frameRouter';

type Command = {
  target: { tabId?: number; extensionId?: string; sessionId?: string };
  method: string;
  callback: (result?: any) => void;
};

describe('frame routing', () => {
  const tabId = 7;
  let commands: Command[];
  let listeners: Array<(source: any, method: string, params: any) => void>;
  let savedChrome: PropertyDescriptor | undefined;

  beforeEach(() => {
    commands = [];
    listeners = [];
    savedChrome = Object.getOwnPropertyDescriptor(globalThis, 'chrome');
    Object.assign(globalThis, {
      chrome: {
        runtime: { id: 'extension-under-test', lastError: undefined },
        tabs: { get: async () => ({ url: 'https://example.test/' }) },
        debugger: {
          attach: async () => {},
          detach: async () => {},
          onEvent: {
            addListener: (listener: (source: any, method: string, params: any) => void) => listeners.push(listener),
            removeListener: (listener: unknown) => { listeners = listeners.filter(item => item !== listener); },
          },
          sendCommand: (target: Command['target'], method: string, _params: any, callback: Command['callback']) => {
            commands.push({ target, method, callback });
            callback({});
          },
        },
      },
    });
  });

  afterEach(async () => {
    await frameRouter.detachFromTab(tabId);
    if (savedChrome) Object.defineProperty(globalThis, 'chrome', savedChrome);
    else Reflect.deleteProperty(globalThis, 'chrome');
  });

  test('uses only Chrome child sessions for frame commands and lease accounting', async () => {
    await frameRouter.attachToTab(tabId);
    const emit = (source: any, method: string, params: any) => listeners.forEach(listener => listener(source, method, params));
    emit({ tabId }, 'Page.frameNavigated', { frame: { id: 'main' } });
    emit({ tabId }, 'Target.attachedToTarget', {
      sessionId: 'chrome-child', targetInfo: { targetId: 'child-target', type: 'iframe' },
    });
    emit({ tabId, sessionId: 'chrome-child' }, 'Page.frameNavigated', { frame: { id: 'child' } });

    commands = [];
    const client = new CDPClient();
    await client.sendCommand({ tabId }, 'DOM.getDocument', {}, { frameId: 'main' });
    await client.sendCommand({ tabId }, 'DOM.getDocument', {}, { frameId: 'unknown' });
    await client.sendCommand({ tabId }, 'DOM.getDocument', {}, { frameId: 'child' });
    await client.sendCommand({ tabId }, 'DOM.getDocument', {}, { frameId: 'child', sessionId: 'explicit-child' });
    assert.deepEqual(commands.map(command => command.target), [
      { tabId }, { tabId }, { tabId, sessionId: 'chrome-child' }, { tabId, sessionId: 'explicit-child' },
    ]);

    commands = [];
    await client.enableDomains({ tabId }, ['DOM'], 'main');
    await client.enableDomains({ tabId }, ['DOM'], 'unknown');
    await client.enableDomains({ tabId }, ['DOM'], 'child');
    await client.disableDomains({ tabId }, ['DOM'], 'main');
    await client.disableDomains({ tabId }, ['DOM'], 'unknown');
    await client.disableDomains({ tabId }, ['DOM'], 'child');
    assert.deepEqual(commands.map(command => [command.method, command.target]), [
      ['DOM.enable', { tabId }], ['DOM.enable', { tabId, sessionId: 'chrome-child' }],
      ['DOM.disable', { tabId }], ['DOM.disable', { tabId, sessionId: 'chrome-child' }],
    ]);
  });
});
