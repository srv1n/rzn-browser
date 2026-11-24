import { describe, expect, it } from 'vitest';
import { NetworkCapture, type CaptureDeps } from './networkCapture';

type Sent = { tabId: number; method: string; params?: Record<string, unknown> };

function harness(options: { attachError?: string; bodies?: Record<string, { body: string; base64Encoded?: boolean } | Error> } = {}) {
  const sent: Sent[] = [];
  const log: string[] = [];
  let holds = 0;
  let frameRouterAttached = false;
  const deps: CaptureDeps = {
    attach: async (tabId) => {
      log.push(`attach:${tabId}`);
      if (options.attachError) throw new Error(options.attachError);
    },
    detach: async (tabId) => { log.push(`detach:${tabId}`); },
    send: async (tabId, method, params) => {
      sent.push({ tabId, method, params });
      if (method === 'Network.getResponseBody') {
        const body = options.bodies?.[String(params?.requestId)];
        if (body instanceof Error) throw body;
        return body ?? { body: '' };
      }
      return {};
    },
    hold: () => { holds++; },
    release: async () => { holds--; return holds === 0 && !frameRouterAttached; },
  };
  const capture = new NetworkCapture(deps);
  const page = (tabId: number, requestId: string, url: string, opts: { type?: string; method?: string; status?: number; mime?: string } = {}) => {
    capture.onEvent({ tabId }, 'Network.requestWillBeSent', { requestId, request: { url, method: opts.method ?? 'GET' }, type: opts.type ?? 'Fetch' });
    capture.onEvent({ tabId }, 'Network.responseReceived', { requestId, response: { status: opts.status ?? 200, mimeType: opts.mime ?? 'application/json', url } });
  };
  const finish = (tabId: number, requestId: string) => capture.onEvent({ tabId }, 'Network.loadingFinished', { requestId });
  return { capture, sent, log, page, finish, holds: () => holds, setFrameRouterAttached: (v: boolean) => { frameRouterAttached = v; } };
}

const TIMELINE = 'https://x.com/i/api/graphql/q1/UserTweets?variables=%7B%7D';

describe('network capture', () => {
  it('keeps matching bodies the page received, in arrival order, once each, and sends no request', async () => {
    const h = harness({ bodies: {
      a: { body: '{"page":1}' },
      b: { body: btoa('{"page":2}'), base64Encoded: true },
      c: { body: '{"page":1}' },
      d: { body: 'not json' },
    } });
    const started = await h.capture.start('s1', 7, { url_pattern: '/graphql/[^/?]+/UserTweets\\?' });
    expect(started).toMatchObject({ capturing: true, tab_id: 7 });

    h.page(7, 'a', TIMELINE);
    h.page(7, 'b', TIMELINE + '&cursor=2');
    h.page(7, 'x', 'https://x.com/i/api/graphql/q2/UserByScreenName?v=1');
    h.page(7, 'img', TIMELINE, { type: 'Image' });
    h.page(7, 'c', TIMELINE);
    h.page(7, 'd', TIMELINE + '&cursor=3', { mime: 'text/plain' });
    h.page(8, 'other-tab', TIMELINE);
    // b finishes before a; arrival order is kept regardless.
    for (const id of ['b', 'a', 'x', 'img', 'c', 'd', 'other-tab']) h.finish(7, id);

    const read = await h.capture.read('s1');
    expect(read.responses.map(r => r.json ?? r.text)).toEqual([{ page: 1 }, { page: 2 }, 'not json']);
    expect(read.responses.map(r => r.seq)).toEqual([1, 2, 4]);
    expect(read.duplicates).toBe(1);
    expect(read.detached).toBeNull();
    // Only debugger-local commands: enable, then body reads of what the page already loaded.
    expect(new Set(h.sent.map(s => s.method))).toEqual(new Set(['Network.enable', 'Network.getResponseBody']));
    expect(h.sent.filter(s => s.method === 'Network.getResponseBody').map(s => s.params?.requestId)).toEqual(['b', 'a', 'c', 'd']);
  });

  it('waits for a tab when started first, and starts on the tab the session opens', async () => {
    const h = harness({ bodies: { a: { body: '[1]' } } });
    expect(await h.capture.start('s1', undefined, { url_pattern: 'UserTweets' })).toMatchObject({ pending: true });
    expect(h.capture.hasPending('s1')).toBe(true);
    expect(h.log).toEqual([]);
    await h.capture.attachPending('s1', 9);
    expect(h.capture.hasPending('s1')).toBe(false);
    h.page(9, 'a', TIMELINE);
    h.finish(9, 'a');
    expect((await h.capture.read('s1')).responses[0].json).toEqual([1]);
  });

  it('ends with the session: disables Network, drops its hold and detaches what it attached', async () => {
    const h = harness();
    await h.capture.start('s1', 7, { url_pattern: 'x' });
    expect(h.holds()).toBe(1);
    expect(await h.capture.stop('s1')).toBe(true);
    expect(h.sent[h.sent.length - 1]?.method).toBe('Network.disable');
    expect(h.holds()).toBe(0);
    expect(h.log).toEqual(['attach:7', 'detach:7']);
    await expect(h.capture.read('s1')).rejects.toMatchObject({ code: 'CAPTURE_NOT_STARTED' });
  });

  it('shares an attachment this extension already holds and leaves it attached', async () => {
    const h = harness({ attachError: 'Another debugger is already attached to the tab with id: 7.' });
    h.setFrameRouterAttached(true);
    await h.capture.start('s1', 7, { url_pattern: 'x' });
    await h.capture.stop('s1');
    expect(h.log).toEqual(['attach:7']);
  });

  it('fails with CAPTURE_ATTACH_FAILED when the tab cannot be watched', async () => {
    const h = harness({ attachError: 'Cannot access a chrome:// URL' });
    await expect(h.capture.start('s1', 7, { url_pattern: 'x' })).rejects.toMatchObject({ code: 'CAPTURE_ATTACH_FAILED' });
    expect(h.holds()).toBe(0);
    await expect(h.capture.read('s1')).rejects.toMatchObject({ code: 'CAPTURE_NOT_STARTED' });
  });

  it('rejects a missing or invalid pattern', async () => {
    const h = harness();
    await expect(h.capture.start('s1', 7, {})).rejects.toMatchObject({ code: 'CAPTURE_SPEC_INVALID' });
    await expect(h.capture.start('s1', 7, { url_pattern: '(' })).rejects.toMatchObject({ code: 'CAPTURE_SPEC_INVALID' });
  });

  it('reports a detach and the bodies it could not keep', async () => {
    const h = harness({ bodies: { big: { body: 'x'.repeat(20) }, gone: new Error('No resource with given identifier found') } });
    await h.capture.start('s1', 7, { url_pattern: 'x', max_body_bytes: 10 });
    h.page(7, 'big', 'https://x.test/a');
    h.page(7, 'gone', 'https://x.test/b');
    h.page(7, 'fail', 'https://x.test/c');
    h.finish(7, 'big');
    h.finish(7, 'gone');
    h.capture.onEvent({ tabId: 7 }, 'Network.loadingFailed', { requestId: 'fail' });
    h.capture.onDetach(7, 'canceled_by_user');
    const read = await h.capture.read('s1');
    expect(read.dropped).toEqual({ too_large: 1, over_budget: 0, failed: 1, body_unavailable: 1 });
    expect(read.detached).toBe('canceled_by_user');
    expect(h.holds()).toBe(0);
    await h.capture.stop('s1');
    // Chrome already detached; nothing more is sent.
    expect(h.sent.some(s => s.method === 'Network.disable')).toBe(false);
    expect(h.log).toEqual(['attach:7']);
  });
});
