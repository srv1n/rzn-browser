// Keeps a copy of the responses a page's own app downloads while a workflow runs
// (backend docs/plans/browser-fleet/design.md, section 11 ruling 7). It reads bodies
// the browser already received through the CDP Network domain; it never issues a
// request. Nothing here is site-specific: a workflow names a URL pattern and maps the
// captured bodies in its own script.

export interface CaptureSpec {
  url_pattern?: unknown;
  resource_types?: unknown;
  methods?: unknown;
  max_responses?: unknown;
  max_body_bytes?: unknown;
  max_total_bytes?: unknown;
  stop?: unknown;
}

export interface CapturedResponse {
  seq: number;
  url: string;
  method: string;
  status: number;
  mime_type: string;
  resource_type: string;
  bytes: number;
  json?: unknown;
  text?: string;
}

export interface CaptureRead {
  url_pattern: string;
  responses: CapturedResponse[];
  duplicates: number;
  dropped: { too_large: number; over_budget: number; failed: number; body_unavailable: number };
  detached: string | null;
}

/** The parts of the debugger and frame router this module uses (injected in tests). */
export interface CaptureDeps {
  attach(tabId: number): Promise<void>;
  detach(tabId: number): Promise<void>;
  send(tabId: number, method: string, params?: Record<string, unknown>): Promise<any>;
  /** Keep a shared attachment alive while capturing; see FrameRouter.holdAttachment. */
  hold(tabId: number): void;
  /** Drop the hold; true when no other user still needs the attachment. */
  release(tabId: number): Promise<boolean>;
}

const DEFAULT_RESOURCE_TYPES = ['XHR', 'Fetch'];
const DEFAULT_MAX_RESPONSES = 200;
const DEFAULT_MAX_BODY_BYTES = 8 * 1024 * 1024;
const DEFAULT_MAX_TOTAL_BYTES = 24 * 1024 * 1024;

export function captureError(code: string, message: string): Error {
  const error = new Error(`${code}: ${message}`);
  (error as any).code = code;
  (error as any).retryable = false;
  return error;
}

interface Config {
  pattern: RegExp;
  urlPattern: string;
  resourceTypes: Set<string> | null;
  methods: Set<string> | null;
  maxResponses: number;
  maxBodyBytes: number;
  maxTotalBytes: number;
}

interface Request {
  url: string;
  method: string;
  resourceType: string;
  status?: number;
  mimeType?: string;
  seq?: number;
}

interface Capture {
  config: Config;
  tabId?: number;
  ownsAttachment: boolean;
  held: boolean;
  requests: Map<string, Request>;
  responses: CapturedResponse[];
  bodyReads: Set<Promise<void>>;
  bodyKeys: Set<string>;
  totalBytes: number;
  seq: number;
  duplicates: number;
  dropped: CaptureRead['dropped'];
  detached: string | null;
}

function positiveInt(value: unknown, fallback: number): number {
  const n = Number(value);
  return Number.isInteger(n) && n > 0 ? n : fallback;
}

function upperSet(value: unknown, fallback: string[] | null): Set<string> | null {
  if (value === undefined || value === null) return fallback ? new Set(fallback.map(v => v.toUpperCase())) : null;
  if (!Array.isArray(value)) throw captureError('CAPTURE_SPEC_INVALID', 'resource_types and methods must be arrays of strings');
  const items = value.map(v => String(v).trim().toUpperCase()).filter(Boolean);
  // ["*"] or [] means any.
  return items.length === 0 || items.includes('*') ? null : new Set(items);
}

export function parseCaptureSpec(spec: CaptureSpec): Config {
  const urlPattern = typeof spec.url_pattern === 'string' ? spec.url_pattern.trim() : '';
  if (!urlPattern) throw captureError('CAPTURE_SPEC_INVALID', 'capture_responses needs url_pattern (a regular expression)');
  let pattern: RegExp;
  try {
    pattern = new RegExp(urlPattern);
  } catch (error: any) {
    throw captureError('CAPTURE_SPEC_INVALID', `url_pattern is not a valid regular expression: ${error?.message || error}`);
  }
  return {
    pattern,
    urlPattern,
    resourceTypes: upperSet(spec.resource_types, DEFAULT_RESOURCE_TYPES),
    methods: upperSet(spec.methods, null),
    maxResponses: positiveInt(spec.max_responses, DEFAULT_MAX_RESPONSES),
    maxBodyBytes: positiveInt(spec.max_body_bytes, DEFAULT_MAX_BODY_BYTES),
    maxTotalBytes: positiveInt(spec.max_total_bytes, DEFAULT_MAX_TOTAL_BYTES),
  };
}

/** FNV-1a; with the URL and length it keys duplicate bodies without keeping a second copy. */
function bodyKey(url: string, text: string): string {
  let hash = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return `${url}\u0000${text.length}\u0000${(hash >>> 0).toString(16)}`;
}

function decodeBody(body: string, base64Encoded: boolean): string {
  if (!base64Encoded) return body;
  const binary = atob(body);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return new TextDecoder().decode(bytes);
}

function parseBody(text: string, mimeType: string): { json?: unknown; text?: string } {
  const trimmed = text.trimStart();
  if (/json/i.test(mimeType) || trimmed.startsWith('{') || trimmed.startsWith('[')) {
    try {
      return { json: JSON.parse(text) };
    } catch {
      // Not JSON after all; keep the text.
    }
  }
  return { text };
}

function isAnotherDebugger(error: unknown): boolean {
  return String((error as any)?.message || error).includes('Another debugger');
}

/**
 * One capture per workflow session. `start` before the session has a tab leaves the
 * capture pending; `attachPending` starts it on the tab the session then opens, before
 * that tab navigates, so the page's first responses are kept too.
 */
export class NetworkCapture {
  private captures = new Map<string, Capture>();

  constructor(private readonly deps: CaptureDeps) {}

  hasPending(sessionId: string): boolean {
    const capture = this.captures.get(sessionId);
    return !!capture && capture.tabId === undefined;
  }

  async start(sessionId: string, tabId: number | undefined, spec: CaptureSpec): Promise<Record<string, unknown>> {
    if (spec.stop === true) {
      const stopped = await this.stop(sessionId);
      return { stopped, session_id: sessionId };
    }
    const config = parseCaptureSpec(spec);
    await this.stop(sessionId);
    const capture: Capture = {
      config,
      ownsAttachment: false,
      held: false,
      requests: new Map(),
      responses: [],
      bodyReads: new Set(),
      bodyKeys: new Set(),
      totalBytes: 0,
      seq: 0,
      duplicates: 0,
      dropped: { too_large: 0, over_budget: 0, failed: 0, body_unavailable: 0 },
      detached: null,
    };
    this.captures.set(sessionId, capture);
    if (tabId !== undefined) {
      await this.attachCapture(sessionId, capture, tabId);
    }
    return {
      capturing: tabId !== undefined,
      pending: tabId === undefined,
      tab_id: tabId ?? null,
      url_pattern: config.urlPattern,
    };
  }

  async attachPending(sessionId: string, tabId: number): Promise<void> {
    const capture = this.captures.get(sessionId);
    if (!capture || capture.tabId !== undefined) return;
    await this.attachCapture(sessionId, capture, tabId);
  }

  private async attachCapture(sessionId: string, capture: Capture, tabId: number): Promise<void> {
    capture.tabId = tabId;
    this.deps.hold(tabId);
    capture.held = true;
    try {
      try {
        await this.deps.attach(tabId);
        capture.ownsAttachment = true;
      } catch (error) {
        // This extension already holds the tab (frame router, type_text); share it.
        if (!isAnotherDebugger(error)) throw error;
      }
      await this.deps.send(tabId, 'Network.enable', {
        maxTotalBufferSize: capture.config.maxTotalBytes * 2,
        maxResourceBufferSize: capture.config.maxBodyBytes,
      });
    } catch (error: any) {
      this.captures.delete(sessionId);
      await this.releaseAttachment(capture);
      throw captureError(
        'CAPTURE_ATTACH_FAILED',
        `cannot watch the page's responses on tab ${tabId} (${error?.message || error}); ` +
          'close DevTools or other debuggers on that tab and retry',
      );
    }
  }

  private async releaseAttachment(capture: Capture): Promise<void> {
    const tabId = capture.tabId;
    if (tabId === undefined || !capture.held) return;
    capture.held = false;
    const unused = await this.deps.release(tabId);
    if (unused && capture.ownsAttachment && capture.detached === null) {
      await this.deps.detach(tabId).catch(() => {});
    }
    capture.ownsAttachment = false;
  }

  /** Ends the capture and drops its bodies. True when one was running. */
  async stop(sessionId: string): Promise<boolean> {
    const capture = this.captures.get(sessionId);
    if (!capture) return false;
    this.captures.delete(sessionId);
    const tabId = capture.tabId;
    if (tabId !== undefined && capture.detached === null) {
      await this.deps.send(tabId, 'Network.disable', {}).catch(() => {});
    }
    await this.releaseAttachment(capture);
    return true;
  }

  /** The kept bodies, in the order the responses arrived. */
  async read(sessionId: string): Promise<CaptureRead> {
    const capture = this.captures.get(sessionId);
    if (!capture) {
      throw captureError(
        'CAPTURE_NOT_STARTED',
        'no response capture is running for this session; add a capture_responses step before the page opens',
      );
    }
    while (capture.bodyReads.size) {
      await Promise.all([...capture.bodyReads]);
    }
    return {
      url_pattern: capture.config.urlPattern,
      responses: capture.responses.slice().sort((a, b) => a.seq - b.seq),
      duplicates: capture.duplicates,
      dropped: { ...capture.dropped },
      detached: capture.detached,
    };
  }

  private captureForTab(tabId: number): Capture | undefined {
    for (const capture of this.captures.values()) {
      if (capture.tabId === tabId && capture.detached === null) return capture;
    }
    return undefined;
  }

  /** chrome.debugger.onDetach: the user dismissed the banner, the tab closed, or DevTools took over. */
  onDetach(tabId: number | undefined, reason: string): void {
    if (tabId === undefined) return;
    const capture = this.captureForTab(tabId);
    if (!capture) return;
    capture.detached = reason || 'detached';
    capture.ownsAttachment = false;
    void this.releaseAttachment(capture);
  }

  /** chrome.debugger.onEvent for root targets (frame sessions are ignored). */
  onEvent(source: { tabId?: number; sessionId?: string }, method: string, params: any): void {
    if (source?.tabId === undefined || source.sessionId) return;
    const capture = this.captureForTab(source.tabId);
    if (!capture || !params?.requestId) return;
    const { config, requests } = capture;
    switch (method) {
      case 'Network.requestWillBeSent': {
        const url = String(params.request?.url || '');
        const method = String(params.request?.method || 'GET').toUpperCase();
        const resourceType = String(params.type || '');
        const wanted =
          config.pattern.test(url) &&
          (!config.resourceTypes || config.resourceTypes.has(resourceType.toUpperCase())) &&
          (!config.methods || config.methods.has(method));
        if (wanted) requests.set(params.requestId, { url, method, resourceType });
        else requests.delete(params.requestId);
        break;
      }
      case 'Network.responseReceived': {
        const request = requests.get(params.requestId);
        if (!request) return;
        request.status = Number(params.response?.status ?? 0);
        request.mimeType = String(params.response?.mimeType || '');
        request.seq = ++capture.seq;
        break;
      }
      case 'Network.loadingFailed':
        if (requests.delete(params.requestId)) capture.dropped.failed++;
        break;
      case 'Network.loadingFinished': {
        const request = requests.get(params.requestId);
        requests.delete(params.requestId);
        if (!request) return;
        const read = this.keepBody(capture, source.tabId, params.requestId, request);
        capture.bodyReads.add(read);
        void read.finally(() => capture.bodyReads.delete(read));
        break;
      }
    }
  }

  private async keepBody(capture: Capture, tabId: number, requestId: string, request: Request): Promise<void> {
    const { config } = capture;
    if (capture.responses.length >= config.maxResponses) {
      capture.dropped.over_budget++;
      return;
    }
    let text: string;
    try {
      const body = await this.deps.send(tabId, 'Network.getResponseBody', { requestId });
      text = decodeBody(String(body?.body ?? ''), body?.base64Encoded === true);
    } catch {
      // Evicted from the browser's buffer, or the page navigated away first.
      capture.dropped.body_unavailable++;
      return;
    }
    if (text.length > config.maxBodyBytes) {
      capture.dropped.too_large++;
      return;
    }
    const key = bodyKey(request.url, text);
    if (capture.bodyKeys.has(key)) {
      capture.duplicates++;
      return;
    }
    if (capture.totalBytes + text.length > config.maxTotalBytes || capture.responses.length >= config.maxResponses) {
      capture.dropped.over_budget++;
      return;
    }
    capture.bodyKeys.add(key);
    capture.totalBytes += text.length;
    capture.responses.push({
      seq: request.seq ?? ++capture.seq,
      url: request.url,
      method: request.method,
      status: request.status ?? 0,
      mime_type: request.mimeType ?? '',
      resource_type: request.resourceType,
      bytes: text.length,
      ...parseBody(text, request.mimeType ?? ''),
    });
  }
}
