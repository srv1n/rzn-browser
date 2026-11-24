import { afterEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';

// Exercise the actual background helpers without starting the service worker.
const source = readFileSync(new URL('./background.ts', import.meta.url), 'utf8');
function loadHelpers(names: string[], bindings: Record<string, unknown>) {
  const helpers = names.map(name => {
    const match = source.match(new RegExp(`^(?:async )?function ${name}(?=[<(])[\\s\\S]*?^}`, 'm'));
    if (!match) throw new Error(`Missing background helper: ${name}`);
    return stripTypeScriptTypes(match[0]);
  }).join('\n');
  return new Function(...Object.keys(bindings), `${helpers}\nreturn { ${names.join(', ')} };`)(...Object.values(bindings));
}

const cacheError = 'The page keeping the extension port is moved into back/forward cache, so the message channel is closed.';

afterEach(() => vi.useRealTimers());

describe('content-script navigation and cancellation', () => {
  function messaging() {
    const sendMessage = vi.fn();
    const ensureContentReady = vi.fn().mockResolvedValue(undefined);
    const helpers = loadHelpers([
      'sendMessageTopFrame', 'withMessageTimeout', 'sendMessageTopFrameWithRetry',
      'isContentScriptDisconnectError',
    ], {
      chrome: {
        tabs: { sendMessage, get: vi.fn().mockResolvedValue({ url: 'https://example.com/new' }) },
        webNavigation: { getAllFrames: vi.fn() },
      },
      withContentScriptCommand: (message: unknown) => message,
      ensureContentReady,
      console: { debug: vi.fn(), warn: vi.fn() },
    });
    return { ...helpers, sendMessage, ensureContentReady };
  }

  it('reports a cached-page disconnect as navigation without replaying a click', async () => {
    const h = messaging();
    h.sendMessage.mockRejectedValue(new Error(cacheError));
    await expect(h.sendMessageTopFrameWithRetry(1, { cmd: 'execute_step', payload: { step: { type: 'click_element' } } }))
      .rejects.toThrow('NAVIGATED_DURING_STEP:');
    expect(h.sendMessage).toHaveBeenCalledTimes(1);
    expect(h.ensureContentReady).not.toHaveBeenCalled();
    expect(h.isContentScriptDisconnectError(new Error(cacheError))).toBe(true);
    expect(h.isContentScriptDisconnectError(new Error('NAVIGATED_DURING_STEP: message channel closed'))).toBe(true);
    expect(h.isContentScriptDisconnectError(new Error('Unexpected DOM failure'))).toBe(false);
  });

  it('does not dispatch an already-cancelled request', async () => {
    const h = messaging();
    const controller = new AbortController();
    controller.abort();
    await expect(h.sendMessageTopFrameWithRetry(1, { cmd: 'get_dom_snapshot' }, { signal: controller.signal }))
      .rejects.toMatchObject({ name: 'AbortError' });
    expect(h.sendMessage).not.toHaveBeenCalled();
  });

  it('stops a pending retry after session cancellation', async () => {
    vi.useFakeTimers();
    const h = messaging();
    const controller = new AbortController();
    h.sendMessage.mockRejectedValue(new Error('Receiving end does not exist'));
    const request = h.sendMessageTopFrameWithRetry(1, { cmd: 'get_dom_snapshot' }, { signal: controller.signal });
    const rejected = expect(request).rejects.toMatchObject({ name: 'AbortError' });
    await vi.advanceTimersByTimeAsync(0);
    controller.abort();
    await vi.advanceTimersByTimeAsync(200);
    await rejected;
    expect(h.sendMessage).toHaveBeenCalledTimes(1);
    expect(h.ensureContentReady).not.toHaveBeenCalled();
  });

  it('still recovers when a receiver was missing before dispatch', async () => {
    vi.useFakeTimers();
    const h = messaging();
    h.sendMessage.mockRejectedValueOnce(new Error('Receiving end does not exist')).mockResolvedValue({ success: true });
    const request = h.sendMessageTopFrameWithRetry(1, { cmd: 'get_dom_snapshot' });
    await vi.advanceTimersByTimeAsync(200);
    await expect(request).resolves.toEqual({ success: true });
    expect(h.sendMessage).toHaveBeenCalledTimes(2);
    expect(h.ensureContentReady).toHaveBeenCalledOnce();
  });

  it.each([true, false])('DOM failure with cancelled=%s preserves real errors and ignores cancellation', async cancelled => {
    const lease = { cancelled: false };
    const error = vi.fn();
    const sendResponse = vi.fn();
    const { handleBrokerMessage } = loadHelpers(['handleBrokerMessage'], {
      console: { log: vi.fn(), warn: vi.fn(), error },
      brokerDispatchCommandName: (message: any) => message.cmd,
      assertBrokerLeaseCurrent: vi.fn(),
      loadWorkflowSessionsFromStorage: vi.fn(),
      normalizeSessionId: (session: string) => session,
      resolveSessionId: () => 'session-1',
      rememberFleetSession: vi.fn(),
      handleDeltaMessage: vi.fn(),
      getWorkflowTabId: () => undefined,
      resolveRequestedTabId: () => undefined,
      getBrokerCommandRegistration: () => ({ dispatch: 'get_dom_snapshot' }),
      sessionMayUseActiveTab: () => true,
      guardedBrokerSideEffect: async () => {
        lease.cancelled = cancelled;
        throw new DOMException(cancelled ? 'workflow session closed' : 'DOM failed', cancelled ? 'AbortError' : 'Error');
      },
      isContentScriptDisconnectError: () => false,
      retainedWorkflowTabLifecyclePayload: () => undefined,
      getWorkflowSessionState: () => ({}),
      sendResponseToBrokerRaw: sendResponse,
    });
    await handleBrokerMessage({ cmd: 'get_dom_snapshot', req_id: 'dom-1' }, 'session-1', lease);
    expect(error).toHaveBeenCalledTimes(cancelled ? 0 : 1);
    expect(sendResponse).toHaveBeenCalledTimes(cancelled ? 0 : 1);
    if (!cancelled) expect(sendResponse.mock.calls[0][0]).toMatchObject({ error_code: 'DOM_PROCESSING_ERROR' });
  });

  it('keeps watchdog failure and recovery, with readable command and operation details', async () => {
    vi.useFakeTimers();
    const lease = { startedAtMs: Date.now(), nativePortEpoch: 1, lastOperation: 'sendMessageTopFrameWithRetry DOM request' };
    const error = vi.fn();
    const recover = vi.fn();
    const respond = vi.fn();
    const { handleBrokerMessageWithWatchdog } = loadHelpers(['handleBrokerMessageWithWatchdog'], {
      brokerMessageWatchdogMs: () => 1000,
      normalizeSessionId: (session: string) => session,
      registerBrokerRequestLease: () => lease,
      nativePortEpoch: 1,
      assertBrokerLeaseCurrent: vi.fn(),
      clearBrokerRequestLease: vi.fn(),
      cancelBrokerRequestLease: vi.fn(),
      invalidateWorkflowSessionEpoch: vi.fn(),
      markBrokerRequestTimedOut: vi.fn(),
      sendResponseToBroker: respond,
      recoverAfterBrokerWatchdogTimeout: recover,
      lastBrokerWatchdogTimeoutMs: null,
      lastBrokerWatchdogRequestId: null,
      lastBrokerWatchdogSessionId: null,
      console: { error, warn: vi.fn() },
    });
    const request = handleBrokerMessageWithWatchdog(
      { cmd: 'get_dom_snapshot', req_id: 'stalled-1' }, 'session-1', 1,
      () => new Promise(() => {}),
    );
    await vi.advanceTimersByTimeAsync(1000);
    await request;
    expect(respond.mock.calls[0][0]).toMatchObject({ success: false, error_code: 'BROKER_WATCHDOG_TIMEOUT' });
    expect(recover).toHaveBeenCalledOnce();
    expect(error.mock.calls[0][0]).toContain('"req_id":"stalled-1"');
    expect(error.mock.calls[0][0]).toContain('"last_operation":"sendMessageTopFrameWithRetry DOM request"');
    expect(error.mock.calls[0][0]).toContain('"elapsed_ms":1000');
  });
});
