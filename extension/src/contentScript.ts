// Content script for RZN Browser Automation
// Handles browser automation actions in the page context

import { nativeInput } from './content/native-input';
import * as cslog from './content-logger';
import { 
  captureCurrentDOM, 
  domHash, 
  diff, 
  ElementStub 
} from './content/dom-capture';
// Using new DOM capture and enhanced actions
// DOM injection removed - using new enhanced action system
// Enhanced action system with Element Resolution and Input Synthesis Ladder
import { enhancedActionExecutor, EnhancedAction } from './content/actions-enhanced';
import { TargetSpec, InputRung } from './types/targets';
import { RZN_BUILD_SIGNATURE, RZN_PAGE_TEST_BRIDGE_ENABLED } from './buildInfo';
import {
  actionResultFailureMessage,
  actionSuccess,
  isActionResultFailure,
  normalizeActionResult,
} from './actions/actionResult';

// DOM snapshot storage for delta tracking
let lastDOMSnapshot: ElementStub[] | null = null;
let lastDOMHash: string | null = null;
const cancelledContentRequestIds = new Map<string, { reason: string; atMs: number }>();
const cancelledContentLeaseIds = new Map<string, { reason: string; atMs: number }>();
const contentCancellationWaiters = new Map<string, Set<(reason: string) => void>>();

// Store the element resolution map for enhanced actions
let lastElementMap: Map<string, Element> = new Map();
let lastClickedSelector: string | undefined;
let lastEnhancedElements: ElementStub[] | null = null;

function parseRefIndex(input: string): number | null {
  const raw = String(input ?? '').trim();
  if (!raw) return null;

  let s = raw;
  if (s.startsWith('ref=')) s = s.slice(4);
  if (s.startsWith('@')) s = s.slice(1);

  const m = /^e(\d+)$/.exec(s);
  if (!m) return null;
  const n = Number(m[1]);
  if (!Number.isFinite(n) || n < 1) return null;
  return n - 1;
}

function contentRequestId(messageOrStep: any): string | undefined {
  const value = messageOrStep?.req_id ?? messageOrStep?.task_id ?? messageOrStep?.request_id;
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

function contentLeaseId(messageOrStep: any): string | undefined {
  const value =
    messageOrStep?.lease_id ??
    messageOrStep?.rzn?.lease_id ??
    messageOrStep?.payload?.lease_id ??
    messageOrStep?.__rzn_lease_id;
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

function pruneCancelledContentRequests(): void {
  const cutoff = Date.now() - 120_000;
  for (const [key, value] of cancelledContentRequestIds.entries()) {
    if (value.atMs < cutoff) cancelledContentRequestIds.delete(key);
  }
  for (const [key, value] of cancelledContentLeaseIds.entries()) {
    if (value.atMs < cutoff) cancelledContentLeaseIds.delete(key);
  }
}

function notifyContentCancellation(key: string, reason: string): void {
  const waiters = contentCancellationWaiters.get(key);
  if (!waiters) return;
  for (const waiter of Array.from(waiters)) {
    try {
      waiter(reason);
    } catch {}
  }
  contentCancellationWaiters.delete(key);
}

function markContentRequestCancelled(message: any): { requestId?: string; leaseId?: string; reason: string } {
  pruneCancelledContentRequests();
  const requestId = contentRequestId(message);
  const leaseId = contentLeaseId(message);
  const reason = String(message?.reason || 'cancelled by broker watchdog');
  const entry = { reason, atMs: Date.now() };
  if (requestId) {
    cancelledContentRequestIds.set(requestId, entry);
    notifyContentCancellation(`req:${requestId}`, reason);
  }
  if (leaseId) {
    cancelledContentLeaseIds.set(leaseId, entry);
    notifyContentCancellation(`lease:${leaseId}`, reason);
  }
  return { requestId, leaseId, reason };
}

function contentCancellationReason(messageOrStep: any): string | null {
  pruneCancelledContentRequests();
  const leaseId = contentLeaseId(messageOrStep);
  if (leaseId && cancelledContentLeaseIds.has(leaseId)) {
    return cancelledContentLeaseIds.get(leaseId)!.reason;
  }
  const requestId = contentRequestId(messageOrStep);
  if (requestId && cancelledContentRequestIds.has(requestId)) {
    return cancelledContentRequestIds.get(requestId)!.reason;
  }
  return null;
}

function throwIfContentRequestCancelled(messageOrStep: any, where: string): void {
  const reason = contentCancellationReason(messageOrStep);
  if (reason) {
    throw new Error(`${reason} (${where})`);
  }
}

function contentCancellationPromise(messageOrStep: any): Promise<never> {
  const keys: string[] = [];
  const requestId = contentRequestId(messageOrStep);
  const leaseId = contentLeaseId(messageOrStep);
  if (requestId) keys.push(`req:${requestId}`);
  if (leaseId) keys.push(`lease:${leaseId}`);
  if (!keys.length) {
    return new Promise(() => {});
  }
  return new Promise((_, reject) => {
    const waiters: Array<[string, (reason: string) => void]> = [];
    const cleanup = () => {
      for (const [key, waiter] of waiters) {
        contentCancellationWaiters.get(key)?.delete(waiter);
      }
    };
    for (const key of keys) {
      const waiter = (reason: string) => {
        cleanup();
        reject(new Error(reason));
      };
      waiters.push([key, waiter]);
      let set = contentCancellationWaiters.get(key);
      if (!set) {
        set = new Set();
        contentCancellationWaiters.set(key, set);
      }
      set.add(waiter);
    }
  });
}

async function withContentCancellation<T>(messageOrStep: any, where: string, work: Promise<T>): Promise<T> {
  throwIfContentRequestCancelled(messageOrStep, `before ${where}`);
  try {
    const result = await Promise.race([work, contentCancellationPromise(messageOrStep)]);
    throwIfContentRequestCancelled(messageOrStep, `after ${where}`);
    return result;
  } catch (error) {
    throwIfContentRequestCancelled(messageOrStep, `after ${where} error`);
    throw error;
  }
}

function selectorForRefIndex(refIndex: number): string | null {
  const stub = lastEnhancedElements?.[refIndex];
  if (!stub) return null;
  return stub.selector || null;
}

// Enhanced DOM snapshot using new DOM capture system
function captureEnhancedDOMSnapshot(options?: { maxElements?: number; highlightElements?: boolean }) {
  const maxElements = options?.maxElements || 200;
  
  // Use the new DOM capture system
  const domData = captureCurrentDOM(maxElements);
  lastEnhancedElements = domData.elements;
  
  // Store element references for resolution
  lastElementMap.clear();
  domData.elements.forEach((element, index) => {
    const encodedId = (element as any).id ? String((element as any).id) : `elem_${index}`;
    if (element.selector) {
      try {
        const domElement = document.querySelector(element.selector);
        if (domElement) {
          // Prefer stable ids; keep index-based ids for backward compatibility.
          lastElementMap.set(encodedId, domElement);
          lastElementMap.set(`elem_${index}`, domElement);
          lastElementMap.set(element.selector, domElement);
          // Agent-browser-style short refs for LLM-friendly selection:
          // idx=N maps to ref=@e{N+1}, so @e1 => elem_0, @e2 => elem_1, etc.
          const ref = `@e${index + 1}`;
          lastElementMap.set(ref, domElement);
          lastElementMap.set(ref.slice(1), domElement); // "eN"
          lastElementMap.set(`ref=${ref.slice(1)}`, domElement);
        }
      } catch (e) {
        // Invalid selector, skip
      }
    }
  });
  
  return {
    elements: domData.elements,
    hash: domData.hash,
    prompt: domData.prompt,
    metadata: domData.metadata,
    element_count: domData.elements.length,
    interactive_count: domData.elements.filter(el => el.tag !== 'div' && el.tag !== 'span').length
  };
}

// Inject a minimal test bridge into the page context so Playwright can call into the
// content script without chrome.* APIs. We use window.postMessage for IPC.
// Inline test bridge removed to avoid CSP violations. A separate pageBridge script is loaded via manifest in MAIN world.

// Enhanced DOM capture function using the new DOM capture module
function captureDOMSnapshot(options?: { maxElements?: number; forceFull?: boolean }) {
  const maxElements = options?.maxElements || 120;
  const forceFull = options?.forceFull || false;
  
  const domData = captureCurrentDOM(maxElements);
  
  // Check if we can send a delta instead of full snapshot
  let deltaData = null;
  if (!forceFull && lastDOMSnapshot && lastDOMHash !== domData.hash) {
    const delta = diff(lastDOMSnapshot, domData.elements);
    
    // Only use delta if changes are reasonable (not too many)
    const totalChanges = delta.added.length + delta.removed.length + delta.modified.length;
    if (totalChanges < domData.elements.length * 0.5) { // Less than 50% changed
      deltaData = delta;
    }
  }
  
  // Store current snapshot for future deltas
  lastDOMSnapshot = domData.elements;
  lastDOMHash = domData.hash;
  
  return {
    elements: domData.elements,
    hash: domData.hash,
    prompt: domData.prompt,
    metadata: domData.metadata,
    delta: deltaData,
    is_delta: !!deltaData
  };
}

// DOM pruning utility
function pruneDOM(options?: { maxSize?: number }): string {
  const snapshot = captureDOMSnapshot({ maxElements: 100 });
  
  // Convert new format back to HTML-like string for compatibility
  const maxSize = options?.maxSize || 1000000;
  let html = snapshot.prompt;
  
  if (html.length > maxSize) {
    html = html.substring(0, maxSize) + '<!-- truncated -->';
  }
  
  return html;
}

// Shadow-DOM aware querying (light DOM + open/closed shadow roots).
function getAnyShadowRoot(host: Element): ShadowRoot | null {
  const openRoot = (host as any).shadowRoot as ShadowRoot | null | undefined;
  if (openRoot) return openRoot;

  try {
    const getter = (window as any).__rznGetShadowRoot as ((el: Element) => ShadowRoot | undefined) | undefined;
    if (typeof getter === 'function') {
      const closedRoot = getter(host);
      if (closedRoot) return closedRoot;
    }
  } catch {
    // Ignore shadow getter failures.
  }

  return null;
}

function walkRootAndShadowRoots(root: ParentNode, visit: (scope: ParentNode) => void) {
  const seen = new Set<ParentNode>();
  const queue: ParentNode[] = [root];
  seen.add(root);

  while (queue.length > 0) {
    const scope = queue.shift()!;
    visit(scope);

    // NodeIterator does not cross into shadow roots; we add them manually.
    const it = document.createNodeIterator(scope as any, NodeFilter.SHOW_ELEMENT);
    let node: Node | null;
    while ((node = it.nextNode())) {
      const el = node as Element;
      const shadowRoot = getAnyShadowRoot(el);
      if (shadowRoot && !seen.has(shadowRoot)) {
        seen.add(shadowRoot);
        queue.push(shadowRoot);
      }
    }
  }
}

function querySelectorAllDeep(selector: string, root: ParentNode = document): Element[] {
  // Fast path: normal DOM query.
  const directQsa = (root as any).querySelectorAll as ((sel: string) => NodeListOf<Element>) | undefined;
  if (typeof directQsa === 'function') {
    const direct = Array.from(directQsa.call(root, selector) || []);
    if (direct.length > 0) return direct;
  }

  // Slow path: traverse into shadow roots only if nothing matched in light DOM.
  const out: Element[] = [];
  walkRootAndShadowRoots(root, scope => {
    // Skip repeating the root query (already attempted above).
    if (scope === root) return;
    const qsa = (scope as any).querySelectorAll as ((sel: string) => NodeListOf<Element>) | undefined;
    if (typeof qsa !== 'function') return;
    qsa.call(scope, selector).forEach(el => out.push(el));
  });
  return out;
}

function querySelectorDeep(selector: string, root: ParentNode = document): Element | null {
  // Fast path: normal DOM query.
  const directQs = (root as any).querySelector as ((sel: string) => Element | null) | undefined;
  if (typeof directQs === 'function') {
    const direct = directQs.call(root, selector);
    if (direct) return direct;
  }

  // Slow path: traverse into shadow roots only if not found in light DOM.
  let found: Element | null = null;
  walkRootAndShadowRoots(root, scope => {
    if (found) return;
    if (scope === root) return;
    const qs = (scope as any).querySelector as ((sel: string) => Element | null) | undefined;
    if (typeof qs !== 'function') return;
    const el = qs.call(scope, selector);
    if (el) found = el;
  });
  return found;
}

function querySelectorAllAcrossRoots(selector: string, root: ParentNode = document): Element[] {
  const out: Element[] = [];
  const seen = new Set<Element>();
  walkRootAndShadowRoots(root, scope => {
    const qsa = (scope as any).querySelectorAll as ((sel: string) => NodeListOf<Element>) | undefined;
    if (typeof qsa !== 'function') return;
    qsa.call(scope, selector).forEach(el => {
      if (seen.has(el)) return;
      seen.add(el);
      out.push(el);
    });
  });
  return out;
}

function findMatchingElement(
  selector: string,
  options?: {
    pierceShadow?: boolean;
    visibleOnly?: boolean;
    preferVisible?: boolean;
  }
): Element | null {
  const pierceShadow = options?.pierceShadow === true;
  const visibleOnly = options?.visibleOnly === true;
  const preferVisible = options?.preferVisible === true || visibleOnly;
  const matches = pierceShadow
    ? querySelectorAllAcrossRoots(selector)
    : Array.from(document.querySelectorAll(selector));

  if (matches.length === 0) {
    return null;
  }

  const visibleMatch = matches.find(match => isElementVisible(match)) || null;
  if (visibleOnly) {
    return visibleMatch;
  }
  if (preferVisible && visibleMatch) {
    return visibleMatch;
  }
  return matches[0] || null;
}

function isElementVisible(element: Element): boolean {
  if (!(element instanceof HTMLElement)) return false;
  const style = window.getComputedStyle(element);
  if (style.display === 'none') return false;
  if (style.visibility === 'hidden') return false;
  if (style.opacity === '0') return false;
  if (element.getAttribute('aria-hidden') === 'true') return false;
  const rect = element.getBoundingClientRect();
  if (!rect || rect.width <= 0 || rect.height <= 0) return false;
  // On-screen-ish check; tolerate partially offscreen.
  if (rect.bottom < 0 || rect.right < 0) return false;
  if (rect.top > window.innerHeight || rect.left > window.innerWidth) return false;
  return true;
}

// Enhanced element discovery using element resolver
async function findElementWithRetry(options: { selector?: string; index?: number; encoded_id?: string }): Promise<Element> {
  console.log('findElementWithRetry called with:', options);
  
  // Handle enhanced targeting
  if (options.encoded_id) {
    const element = lastElementMap.get(options.encoded_id);
    if (element) {
      console.log('Element found by encoded_id!', element);
      return element;
    }
    throw new Error(`Element with encoded_id ${options.encoded_id} not found in element map`);
  }
  
  // Handle index-based lookup (convert to encoded_id)
  if (options.index !== undefined) {
    console.log(`Looking up element by index: ${options.index}`);
    const encodedId = `elem_${options.index}`;
    const element = lastElementMap.get(encodedId);
    if (element) {
      console.log('Element found by index!', element);
      return element;
    }
    throw new Error(`Element with index ${options.index} not found in element map`);
  }
  
  // Handle selector-based lookup
  const selector = options.selector;
  if (!selector) {
    throw new Error('Missing selector, index, or encoded_id for element discovery');
  }

  // Direct cache lookup (supports CSS selectors, EncodedIds like "0:12", legacy "elem_N",
  // and ref aliases like "@e12"/"ref=e12" when present).
  const directCached = lastElementMap.get(selector);
  if (directCached && directCached.isConnected) {
    return directCached;
  }

  // Ref-based lookup (e.g. "@e12", "ref=e12", "e12")
  const refIndex = parseRefIndex(selector);
  if (refIndex !== null) {
    const key = `elem_${refIndex}`;
    const cached = lastElementMap.get(key);
    if (cached && cached.isConnected) {
      return cached;
    }

    const resolvedSelector = selectorForRefIndex(refIndex);
    if (resolvedSelector) {
      const el = findMatchingElement(resolvedSelector, {
        pierceShadow: (options as any).pierce_shadow === true,
        visibleOnly: (options as any).visible === true,
        preferVisible: (options as any).pierce_shadow === true,
      });
      if (el) {
        lastElementMap.set(key, el);
        lastElementMap.set(resolvedSelector, el);
        return el;
      }
    }

    throw new Error(
      `UNKNOWN_REF: ${selector} (take a fresh DOM snapshot and retry)`
    );
  }
  
  const maxRetries = 10;
  const retryDelay = 300;
  
  for (let i = 0; i < maxRetries; i++) {
    console.log(`Attempt ${i + 1} to find selector: ${selector}`);
    const element = findMatchingElement(selector, {
      pierceShadow: (options as any).pierce_shadow === true,
      visibleOnly: (options as any).visible === true,
      preferVisible: (options as any).pierce_shadow === true,
    });
    if (element) {
      console.log('Element found!', element);
      // Cache it for future use
      lastElementMap.set(selector, element);
      return element;
    }
    
    // Log what elements exist on the page
    if (i === 0) {
      console.log('Available input elements:', Array.from(document.querySelectorAll('input')).map(el => ({
        name: el.getAttribute('name'),
        type: el.getAttribute('type'),
        role: el.getAttribute('role'),
        id: el.getAttribute('id'),
        class: el.getAttribute('class')
      })));
    }
    
    if (i < maxRetries - 1) {
      await new Promise(resolve => setTimeout(resolve, retryDelay));
    }
  }
  
  throw new Error(`SELECTOR_NOT_FOUND: Unable to find element with selector: ${selector}`);
}

function bestEffortSelector(element: Element): string {
  const html = element as HTMLElement;
  if (html.id) return `#${CSS.escape(html.id)}`;
  for (const attr of ['data-testid', 'data-cy', 'data-test', 'name', 'role', 'aria-label']) {
    const value = html.getAttribute(attr);
    if (value) return `[${attr}=${JSON.stringify(value)}]`;
  }
  const tag = element.tagName.toLowerCase();
  const classList = Array.from(html.classList || []).slice(0, 3);
  if (classList.length > 0) return `${tag}.${classList.map(cls => CSS.escape(cls)).join('.')}`;
  const parent = element.parentElement;
  if (!parent) return tag;
  const siblings = Array.from(parent.children).filter(child => child.tagName === element.tagName);
  const index = Math.max(0, siblings.indexOf(element));
  return `${tag}:nth-of-type(${index + 1})`;
}

function extractElementValue(element: Element | null): string {
  if (!element) return '';
  if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement || element instanceof HTMLSelectElement) {
    return element.value || '';
  }
  const attr = element.getAttribute?.('value');
  if (attr != null) return attr;
  return (element.textContent || '').trim();
}

function summarizeListenerSurface(element: Element | null): Record<string, any> {
  if (!element) return { inline_handlers: [], property_handlers: [], limited: true };
  const inlineHandlers = Array.from(element.attributes || [])
    .filter(attr => attr.name.startsWith('on'))
    .map(attr => attr.name);

  const propertyHandlers: string[] = [];
  const anyEl = element as any;
  for (const key of [
    'onclick',
    'ondblclick',
    'onmousedown',
    'onmouseup',
    'onpointerdown',
    'onpointerup',
    'onkeydown',
    'onkeyup',
    'oninput',
    'onchange',
    'onsubmit',
    'onfocus',
    'onblur',
  ]) {
    if (typeof anyEl[key] === 'function') {
      propertyHandlers.push(key);
    }
  }

  return {
    inline_handlers: inlineHandlers,
    property_handlers: propertyHandlers,
    limited: true,
    note: 'DevTools-only event listener introspection is not available in the content script; this reports inline and property listeners only.',
  };
}

function buildShadowPath(element: Element | null): string[] {
  if (!element) return [];
  const path: string[] = [];
  let current: Node | null = element;
  while (current) {
    if (current instanceof Element) {
      path.push(bestEffortSelector(current));
      const root = current.getRootNode();
      if (root instanceof ShadowRoot && root.host) {
        current = root.host;
        continue;
      }
    }
    const parent = (current as any).parentElement || null;
    current = parent;
  }
  return path;
}

function isActionableElement(element: Element | null): boolean {
  if (!element || !(element instanceof HTMLElement)) return false;
  const tag = element.tagName.toLowerCase();
  if (['a', 'button', 'summary', 'label'].includes(tag)) return true;
  if (['input', 'textarea', 'select'].includes(tag)) return true;
  if (element.hasAttribute('onclick')) return true;
  if (element.getAttribute('role') && ['button', 'link', 'tab', 'menuitem', 'checkbox', 'switch'].includes((element.getAttribute('role') || '').toLowerCase())) return true;
  if (typeof (element as any).onclick === 'function') return true;
  if ((element as HTMLAnchorElement).href) return true;
  if (element.tabIndex >= 0) return true;
  return false;
}

function findActionableAncestor(element: Element | null): Element | null {
  let current: Element | null = element;
  while (current) {
    if (isActionableElement(current)) return current;
    current = current.parentElement;
  }
  return null;
}

function summarizeElementForDebug(element: Element | null): any {
  if (!element) return null;
  const html = element as HTMLElement;
  const rect = html.getBoundingClientRect?.();
  const attrs: Record<string, string> = {};
  for (const attr of Array.from(element.attributes || [])) {
    if (
      [
        'id',
        'class',
        'name',
        'type',
        'role',
        'href',
        'target',
        'placeholder',
        'aria-label',
        'aria-describedby',
        'aria-labelledby',
        'data-testid',
      ].includes(attr.name)
    ) {
      attrs[attr.name] = attr.value;
    }
  }
  return {
    selector: bestEffortSelector(element),
    tag: element.tagName.toLowerCase(),
    text: (element.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 200),
    visible: isElementVisible(element),
    value: extractElementValue(element),
    attributes: attrs,
    rect: rect
      ? {
          x: Math.round(rect.x),
          y: Math.round(rect.y),
          width: Math.round(rect.width),
          height: Math.round(rect.height),
        }
      : null,
    shadow_path: buildShadowPath(element),
    listeners: summarizeListenerSurface(element),
  };
}

function serializeForTransport(value: any, depth = 0, seen = new WeakSet<object>()): any {
  if (value == null || typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
    return value;
  }
  if (typeof value === 'bigint') return value.toString();
  if (typeof value === 'function') return `[Function ${value.name || 'anonymous'}]`;
  if (value instanceof Element) return summarizeElementForDebug(value);
  if (value instanceof Error) {
    return { name: value.name, message: value.message, stack: value.stack };
  }
  if (depth >= 4) {
    if (Array.isArray(value)) return `[Array(${value.length})]`;
    return '[Object]';
  }
  if (Array.isArray(value)) {
    return value.slice(0, 50).map(item => serializeForTransport(item, depth + 1, seen));
  }
  if (typeof value === 'object') {
    if (seen.has(value)) return '[Circular]';
    seen.add(value);
    const out: Record<string, any> = {};
    for (const [key, child] of Object.entries(value).slice(0, 50)) {
      out[key] = serializeForTransport(child, depth + 1, seen);
    }
    return out;
  }
  return String(value);
}

async function evaluateUserScript(
  scriptRaw: string,
  argsRaw: any,
  paramsRaw: any,
  returnValue = true
): Promise<any> {
  const script = String(scriptRaw || '').trim();
  const args = Array.isArray(argsRaw) ? argsRaw : [];
  const params =
    paramsRaw && typeof paramsRaw === 'object' && !Array.isArray(paramsRaw) ? paramsRaw : {};
  const body =
    script.includes('return ') ||
    /(^|[\s;])(?:const|let|var|if|for|while|throw|try|await)\b/.test(script) ||
    script.includes(';') ||
    script.includes('\n')
      ? script
      : `return (${script});`;
  const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor as FunctionConstructor;
  const run = new AsyncFunction(
    'args',
    'params',
    'windowRef',
    'documentRef',
    'consoleRef',
    `
    "use strict";
    const window = windowRef;
    const document = documentRef;
    const console = consoleRef;
    const __args = Array.isArray(args) ? args : [];
    const __rzn_params = params && typeof params === 'object' ? params : {};
    const arg0 = __args[0];
    const arg1 = __args[1];
    const arg2 = __args[2];
    const arg3 = __args[3];
    const arg4 = __args[4];
    const arg5 = __args[5];
    const arg6 = __args[6];
    const arg7 = __args[7];
    const arg8 = __args[8];
    const arg9 = __args[9];
    const previousParams = window.__rzn_params;
    window.__rzn_params = __rzn_params;
    try {
      return await (async () => {
        ${body}
      })();
    } finally {
      if (typeof previousParams === 'undefined') {
        try { delete window.__rzn_params; } catch {}
      } else {
        window.__rzn_params = previousParams;
      }
    }
  `
  );
  const value = await run(args, params, window, document, console);
  return returnValue === false ? null : serializeForTransport(value);
}

async function callPageBridge(type: string, payload: any, timeoutMs = 10000): Promise<any> {
  const containerId = '__rzn_page_bridge';
  let container = document.getElementById(containerId) as HTMLElement | null;
  if (!container) {
    container = document.createElement('div');
    container.id = containerId;
    container.style.display = 'none';
    (document.documentElement || document.body || document.head).appendChild(container);
  }

  const requestId = `${type}_${Math.random().toString(36).slice(2)}`;
  const node = document.createElement('div');
  node.setAttribute('data-rzn-req-id', requestId);
  node.setAttribute('data-rzn-type', type);
  node.setAttribute('data-rzn-target', 'page');
  node.setAttribute(CONTENT_SCRIPT_DOM_OWNER_ATTR, CONTENT_SCRIPT_INSTANCE_ID);
  node.textContent = JSON.stringify(payload ?? {});
  container.appendChild(node);

  return await new Promise((resolve, reject) => {
    let done = false;
    const finish = (err: Error | null, value?: any) => {
      if (done) return;
      done = true;
      observer.disconnect();
      clearTimeout(timer);
      try {
        node.remove();
      } catch {}
      if (err) reject(err);
      else resolve(value);
    };

    const check = () => {
      const err = node.getAttribute('data-rzn-err');
      if (err) {
        finish(new Error(err));
        return;
      }
      const resp = node.getAttribute('data-rzn-resp');
      if (!resp) return;
      try {
        finish(null, JSON.parse(resp));
      } catch (error: any) {
        finish(new Error(error?.message || String(error)));
      }
    };

    const observer = new MutationObserver(() => check());
    observer.observe(node, { attributes: true, attributeFilter: ['data-rzn-resp', 'data-rzn-err'] });
    const timer = window.setTimeout(() => finish(new Error(`Page bridge timeout for ${type}`)), timeoutMs);
    void contentCancellationPromise(payload).catch((error) => {
      finish(error instanceof Error ? error : new Error(String(error)));
    });
    check();
  });
}

function findVisibleOverlays(limit = 8): any[] {
  const selectors = [
    '[role="dialog"]',
    '[aria-modal="true"]',
    'dialog[open]',
    '[popover]',
    '[data-testid*="modal" i]',
    '[data-testid*="dialog" i]',
    '[class*="modal" i]',
    '[class*="dialog" i]',
    '[class*="drawer" i]',
    '[class*="popover" i]',
  ];
  const seen = new Set<Element>();
  const results: any[] = [];
  for (const selector of selectors) {
    for (const element of Array.from(document.querySelectorAll(selector))) {
      if (seen.has(element) || !isElementVisible(element)) continue;
      seen.add(element);
      results.push(summarizeElementForDebug(element));
      if (results.length >= limit) return results;
    }
  }
  return results;
}

function resolveSelectorWithRefs(selectorRaw: string | undefined): string | undefined {
  const selector = String(selectorRaw || '').trim();
  if (!selector) return undefined;
  const refIndex = parseRefIndex(selector);
  if (refIndex === null) return selector;
  const resolved = selectorForRefIndex(refIndex);
  if (!resolved) {
    throw new Error(`UNKNOWN_REF: ${selector} (take a fresh DOM snapshot and retry)`);
  }
  return resolved;
}

function textMatches(actual: string, expected: string, matchTypeRaw?: string): boolean {
  const matchType = String(matchTypeRaw || 'contains').toLowerCase();
  if (matchType === 'equals') return actual === expected;
  if (matchType === 'regex') {
    try {
      return new RegExp(expected).test(actual);
    } catch {
      return false;
    }
  }
  return actual.includes(expected);
}

function isTruthyWorkflowValue(value: unknown): boolean {
  const text = String(value ?? '').trim().toLowerCase();
  return text === 'true' || text === '1' || text === 'yes' || text === 'y' || text === 'on';
}

function shouldUseCdpEvalForStep(step: any): boolean {
  if (
    step?.use_cdp === true ||
    step?.use_cdp_eval === true ||
    step?.execution_backend === 'cdp' ||
    step?.backend === 'cdp'
  ) {
    return true;
  }
  const conditionalIndex = step?.use_cdp_eval_when_arg_truthy ?? step?.use_cdp_when_arg_truthy;
  if (conditionalIndex === undefined || conditionalIndex === null || conditionalIndex === '') {
    return false;
  }
  const index = Number(conditionalIndex);
  if (!Number.isInteger(index) || index < 0 || !Array.isArray(step?.args)) {
    return false;
  }
  return isTruthyWorkflowValue(step.args[index]);
}

function evaluateUiExpectation(expectation: any): { ok: boolean; details: any } {
  if (Array.isArray(expectation?.all)) {
    const entries = expectation.all.map((item: any) => evaluateUiExpectation(item));
    return { ok: entries.every(entry => entry.ok), details: { mode: 'all', entries } };
  }
  if (Array.isArray(expectation?.any)) {
    const entries = expectation.any.map((item: any) => evaluateUiExpectation(item));
    return { ok: entries.some(entry => entry.ok), details: { mode: 'any', entries } };
  }

  const selector = resolveSelectorWithRefs(expectation?.selector);
  const condition = String(expectation?.condition || '').trim().toLowerCase();
  const element = selector ? (document.querySelector(selector) || querySelectorDeep(selector)) : null;
  const elementCount = selector ? document.querySelectorAll(selector).length || (element ? 1 : 0) : 0;
  const value = extractElementValue(element);
  const text = (element?.textContent || '').replace(/\s+/g, ' ').trim();
  const activeElement = document.activeElement as Element | null;
  const overlays = findVisibleOverlays(4);
  let ok = true;
  const facts: Record<string, any> = {
    selector,
    condition,
    element_found: !!element,
    element_count: elementCount,
    value,
    text,
    current_url: window.location.href,
    active_element: summarizeElementForDebug(activeElement),
    overlay_count: overlays.length,
  };

  if (condition) {
    if (condition === 'exists') ok = ok && !!element;
    else if (condition === 'not_exists' || condition === 'missing') ok = ok && !element;
    else if (condition === 'visible') ok = ok && !!element && isElementVisible(element);
    else if (condition === 'hidden') ok = ok && (!element || !isElementVisible(element));
    else if (condition === 'focused') ok = ok && !!element && element === activeElement;
    else if (condition === 'value_nonempty') ok = ok && value.trim().length > 0;
  }

  if (expectation?.text != null) {
    ok = ok && textMatches(text, String(expectation.text), expectation?.match_type);
  }
  if (expectation?.value_equals != null) ok = ok && value === String(expectation.value_equals);
  if (expectation?.value_contains != null) ok = ok && value.includes(String(expectation.value_contains));
  if (expectation?.url_includes != null) ok = ok && window.location.href.includes(String(expectation.url_includes));
  if (expectation?.url_matches != null) {
    try {
      ok = ok && new RegExp(String(expectation.url_matches)).test(window.location.href);
    } catch {
      ok = false;
    }
  }
  if (expectation?.active_selector != null) {
    try {
      const activeSelector = resolveSelectorWithRefs(String(expectation.active_selector));
      ok =
        ok &&
        !!activeElement &&
        !!activeSelector &&
        ((activeElement as HTMLElement).matches?.(activeSelector) ||
          !!activeElement.closest?.(activeSelector));
    } catch {
      ok = false;
    }
  }
  if (expectation?.count_at_least != null) ok = ok && elementCount >= Number(expectation.count_at_least);
  if (expectation?.count_equals != null) ok = ok && elementCount === Number(expectation.count_equals);

  return { ok, details: facts };
}

async function waitForUiExpectation(expectation: any, timeoutMs: number): Promise<any> {
  const start = Date.now();
  let last: any = null;
  while (Date.now() - start <= timeoutMs) {
    last = evaluateUiExpectation(expectation);
    if (last.ok) {
      return { success: true, matched: true, checks: last.details };
    }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  return { success: false, matched: false, checks: last?.details || null };
}

async function captureUiBundleInternal(step: any): Promise<any> {
  const maxElements = Math.max(20, Number(step?.max_elements || 120));
  const selector = resolveSelectorWithRefs(step?.selector);
  const target = selector ? (document.querySelector(selector) || querySelectorDeep(selector)) : null;
  const activeElement = document.activeElement as Element | null;
  const bundle: any = {
    timestamp: Date.now(),
    url: window.location.href,
    title: document.title,
    ready_state: document.readyState,
    viewport: {
      width: window.innerWidth,
      height: window.innerHeight,
      scroll_x: window.scrollX,
      scroll_y: window.scrollY,
    },
    active_element: summarizeElementForDebug(activeElement),
    target_element: summarizeElementForDebug(target),
    visible_overlays: findVisibleOverlays(8),
  };

  if (step?.include_dom_snapshot !== false) {
    bundle.dom_snapshot = captureEnhancedDOMSnapshot({ maxElements, highlightElements: false });
  }

  return bundle;
}

// Validation with timeout
async function validateWithTimeout(rules: any[], timeoutMs: number): Promise<boolean> {
  const startTime = Date.now();
  
  while (Date.now() - startTime < timeoutMs) {
    let allValid = true;
    
    for (const rule of rules) {
      let isValid = false;
      
      switch (rule.rule) {
        case 'elementState':
          if (rule.selector) {
            const element = document.querySelector(rule.selector);
            if ((rule.state === 'visible' && element && element.offsetParent !== null) ||
                (rule.state === 'hidden' && (!element || element.offsetParent === null))) {
              isValid = true;
            }
          }
          break;
          
        case 'elementContent':
          if (rule.selector && rule.contains) {
            const element = document.querySelector(rule.selector);
            if (element && element.textContent?.includes(rule.contains)) {
              isValid = true;
            }
          }
          break;
          
        case 'elementCount':
          if (rule.selector && rule.min !== undefined) {
            const count = document.querySelectorAll(rule.selector).length;
            isValid = count >= rule.min;
            if (rule.max !== undefined) {
              isValid = isValid && count <= rule.max;
            }
          }
          break;
          
        case 'pageState':
          if (rule.state === 'urlMatches' && rule.pattern) {
            isValid = new RegExp(rule.pattern).test(window.location.href);
          }
          break;
      }
      
      if (!isValid) {
        allValid = false;
        break;
      }
    }
    
    if (allValid) {
      return true;
    }
    
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  
  return false;
}

// Enhanced action handlers using Element Resolution and Input Synthesis Ladder
const enhancedActionHandlers = {
  // Enhanced wait_for_element with better element resolution
  wait_for_element: async (step: any) => {
    const timeout = step.timeout_ms || step.timeoutMs || 10000;
    let selector = step.selector;

    if (selector) {
      const refIndex = parseRefIndex(selector);
      if (refIndex !== null) {
        const resolved = selectorForRefIndex(refIndex);
        if (!resolved) {
          throw new Error(`UNKNOWN_REF: ${selector} (take a fresh DOM snapshot and retry)`);
        }
        selector = resolved;
      }
    }
    
    const startTime = Date.now();
    while (Date.now() - startTime < timeout) {
      const element = selector
        ? findMatchingElement(selector, {
            pierceShadow: step.pierce_shadow === true,
            visibleOnly: step.visible === true,
            preferVisible: step.pierce_shadow === true,
          })
        : null;
      if (element && (!step.visible || isElementVisible(element))) {
        return { success: true, found: true };
      }
      await withContentCancellation(step, 'wait_for_element poll', new Promise(resolve => setTimeout(resolve, 100)));
    }
    
    if (step.optional) {
      return { success: true, found: false };
    }
    throw new Error(`Element not found: ${selector}`);
  },

  // Enhanced wait_for_timeout with precise timing
  wait_for_timeout: async (step: any) => {
    const timeout = step.timeout_ms || step.timeoutMs || 1000;
    await withContentCancellation(step, 'wait_for_timeout', new Promise(resolve => setTimeout(resolve, timeout)));
    return { success: true };
  },

  // Enhanced click with stable element resolution and input escalation
  click_element_enhanced: async (step: any) => {
    const targetSpec = normalizeStepToTargetSpec(step);
    const action: EnhancedAction = {
      type: 'click_element',
      target_spec: targetSpec,
      button: step.button || 'left',
      modifiers: step.modifiers || [],
      timeout: step.timeoutMs || 10000,
      retry_count: step.retry_count || 3,
      force: step.force || false
    };

    const result = await enhancedActionExecutor.execute(action);
    
    // Log performance info
    console.log(`[Enhanced Click] Used rung ${result.rung_used}, escalated: ${result.escalated}, time: ${result.execution_time_ms}ms`);
    
    if (!result.success) {
      throw new Error(result.error || 'Enhanced click failed');
    }

    // Best-effort: focus the element after a successful click when selector is available
    try {
      const sel = step.css || step.selector;
      if (sel) {
        const el = document.querySelector(sel) as HTMLElement | null;
        if (el && typeof el.focus === 'function') {
          el.focus();
        }
        lastClickedSelector = sel;
      }
    } catch {}

    return normalizeActionResult('click_element_enhanced', result.result, {
      duration_ms: result.execution_time_ms,
      rung_used: result.rung_used,
      escalated: result.escalated,
    });
  },

  // Enhanced fill with stable element resolution and input escalation
  fill_input_field_enhanced: async (step: any) => {
    const targetSpec = normalizeStepToTargetSpec(step);
    const action: EnhancedAction = {
      type: 'fill_input_field',
      target_spec: targetSpec,
      value: step.value || step.text || '',
      clear: step.clear_first !== false,
      timeout: step.timeoutMs || 10000,
      retry_count: step.retry_count || 3,
      force: step.force || false
    };

    const result = await enhancedActionExecutor.execute(action);
    
    // Log performance info
    console.log(`[Enhanced Fill] Used rung ${result.rung_used}, escalated: ${result.escalated}, time: ${result.execution_time_ms}ms`);
    
    if (!result.success) {
      throw new Error(result.error || 'Enhanced fill failed');
    }

    return result.result;
  },

  // Enhanced key press with input escalation
  press_special_key_enhanced: async (step: any) => {
    const key = step.key || 'Enter';
    console.log(`[ContentScript] Enhanced press_special_key: ${key}`);
    
    // Try DOM-first on the currently focused element
    try {
      const active = document.activeElement as HTMLElement | null;
      if (active) {
        active.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
        active.dispatchEvent(new KeyboardEvent('keyup', { key, bubbles: true, cancelable: true }));
      }
    } catch {}
    
    return actionSuccess({
      action: 'press_special_key',
      result: { pressed: true, key },
      legacy: { key },
    });
  },

  // New first-class press_key action using CDP
  press_key: async (step: any) => {
    const key = step.key || (step.args && step.args[0]) || 'Enter';
    console.log(`[ContentScript] Executing press_key: ${key}`);
    
    // DOM-first dispatch to focused element
    try {
      let active = document.activeElement as HTMLElement | null;
      // If nothing focusable is active, try to re-focus the last clicked selector
      if ((!active || !(active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement)) && lastClickedSelector) {
        const el = document.querySelector(lastClickedSelector) as HTMLElement | null;
        if (el && typeof el.focus === 'function') {
          el.focus();
          active = el;
        }
      }
      if (active) {
        active.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
        active.dispatchEvent(new KeyboardEvent('keyup', { key, bubbles: true, cancelable: true }));
      }
    } catch {}
    
    return actionSuccess({
      action: 'press_key',
      result: { pressed: true, key },
      legacy: { key },
    });
  },

  // Enhanced hover with element resolution
  hover_element_enhanced: async (step: any) => {
    const targetSpec = normalizeStepToTargetSpec(step);
    const action: EnhancedAction = {
      type: 'hover_element',
      target_spec: targetSpec,
      timeout: step.timeoutMs || 5000
    };

    const result = await enhancedActionExecutor.execute(action);
    
    if (!result.success) {
      throw new Error(result.error || 'Enhanced hover failed');
    }

    return normalizeActionResult('hover_element_enhanced', result.result, {
      duration_ms: result.execution_time_ms,
      rung_used: result.rung_used,
      escalated: result.escalated,
    });
  },

  // Enhanced scroll with element resolution
  scroll_element_into_view_enhanced: async (step: any) => {
    const targetSpec = normalizeStepToTargetSpec(step);
    const action: EnhancedAction = {
      type: 'scroll_element_into_view',
      target_spec: targetSpec,
      timeout: step.timeoutMs || 5000
    };

    const result = await enhancedActionExecutor.execute(action);
    
    if (!result.success) {
      throw new Error(result.error || 'Enhanced scroll failed');
    }

    return normalizeActionResult('scroll_element_into_view_enhanced', result.result, {
      duration_ms: result.execution_time_ms,
      rung_used: result.rung_used,
      escalated: result.escalated,
    });
  },

  // Enhanced text extraction
  get_element_text_enhanced: async (step: any) => {
    const targetSpec = normalizeStepToTargetSpec(step);
    const action: EnhancedAction = {
      type: 'get_element_text',
      target_spec: targetSpec,
      timeout: step.timeoutMs || 5000
    };

    const result = await enhancedActionExecutor.execute(action);
    
    if (!result.success) {
      throw new Error(result.error || 'Enhanced text extraction failed');
    }

    return normalizeActionResult('get_element_text_enhanced', result.result, {
      duration_ms: result.execution_time_ms,
      rung_used: result.rung_used,
      escalated: result.escalated,
    });
  },

  // Enhanced structured data extraction
  extract_structured_data_enhanced: async (step: any) => {
    const targetSpec = normalizeStepToTargetSpec(step);
    const action: EnhancedAction = {
      type: 'extract_structured_data',
      target_spec: targetSpec,
      fields: step.fields || [],
      // Pass through extraction_type for workflow compatibility (no site-specific fast paths)
      extraction_type: step.extraction_type,
      timeout: step.timeoutMs || 10000
    };

    const result = await enhancedActionExecutor.execute(action);
    
    if (!result.success) {
      throw new Error(result.error || 'Enhanced data extraction failed');
    }

    return actionSuccess({
      action: 'extract_structured_data_enhanced',
      result: result.result,
      legacy: {
        data: result.result,
        extraction_type: step.extraction_type,
        item_count: Array.isArray(result.result) ? result.result.length : 0,
      },
      duration_ms: result.execution_time_ms,
      rung_used: result.rung_used,
      escalated: result.escalated,
    });
  },

  // System information and diagnostics
  get_performance_stats: async (step: any) => {
    const inputStats = enhancedActionExecutor.getPerformanceStats();
    const cacheStats = enhancedActionExecutor.getCacheStats();
    
    return {
      input_ladder_stats: inputStats,
      element_cache_stats: cacheStats,
      current_url: window.location.href,
      timestamp: Date.now()
    };
  },

  // Cache management
  clear_enhanced_caches: async (step: any) => {
    enhancedActionExecutor.clearCaches();
    return true;
  }
};

// Helper function to convert legacy step format to TargetSpec
function normalizeStepToTargetSpec(step: any): TargetSpec {
  // If step already has target_spec, use it
  if (step.target_spec) {
    return step.target_spec;
  }

  // Convert legacy selectors
  if (step.encoded_id) {
    return { encoded_id: step.encoded_id };
  }
  
  if (step.css || step.selector) {
    const sel = step.css || step.selector;
    const refIndex = parseRefIndex(sel);
    if (refIndex !== null) {
      const resolved = selectorForRefIndex(refIndex);
      if (resolved) return { css: resolved };
    }
    return { css: sel };
  }
  
  if (step.xpath) {
    return { xpath: step.xpath };
  }
  
  if (step.role_name) {
    return { role_name: step.role_name };
  }
  
  if (step.text_near) {
    return { text_near: step.text_near };
  }

  // Fallback to selector field
  return { css: step.selector || 'body' };
}

const REDACTED_STEP_KEYS = ['value', 'text', 'password', 'passcode', 'otp', 'token', 'secret'];

function redactStepForLog(input: any): any {
  if (input === null || input === undefined) return input;
  if (Array.isArray(input)) return input.map(item => redactStepForLog(item));
  if (typeof input !== 'object') return input;

  const clone: Record<string, any> = {};
  for (const [key, value] of Object.entries(input)) {
    const lowered = key.toLowerCase();
    const shouldRedact = REDACTED_STEP_KEYS.some(marker => lowered.includes(marker));
    if (shouldRedact) {
      const raw = typeof value === 'string' ? value : String(value ?? '');
      clone[key] = `<redacted:${raw.length}>`;
      continue;
    }
    clone[key] = redactStepForLog(value);
  }
  return clone;
}

// Internal utility handlers used by some enhanced flows
const PAGE_BRIDGE_CONTAINER_ID = '__rzn_page_bridge';

function ensurePageBridgeContainer(): HTMLElement {
  let el = document.getElementById(PAGE_BRIDGE_CONTAINER_ID) as HTMLElement | null;
  if (!el) {
    el = document.createElement('div');
    el.id = PAGE_BRIDGE_CONTAINER_ID;
    el.style.display = 'none';
    (document.documentElement || document.body || document.head).appendChild(el);
  }
  installContentBridgeMetadata(el);
  return el;
}

async function sendPageBridgeRequest(type: string, payload: any, timeoutMs = 10000): Promise<any> {
  const container = ensurePageBridgeContainer();
  const requestId = `${type}_${Math.random().toString(36).slice(2)}`;
  const node = document.createElement('div');
  node.setAttribute('data-rzn-req-id', requestId);
  node.setAttribute('data-rzn-type', type);
  node.setAttribute('data-rzn-target', 'page');
  node.setAttribute(CONTENT_SCRIPT_DOM_OWNER_ATTR, CONTENT_SCRIPT_INSTANCE_ID);
  node.textContent = JSON.stringify(payload ?? {});
  container.appendChild(node);

  return await new Promise((resolve, reject) => {
    let done = false;
    const finish = (err: Error | null, value?: any) => {
      if (done) return;
      done = true;
      observer.disconnect();
      clearTimeout(timer);
      try {
        node.remove();
      } catch {}
      if (err) reject(err);
      else resolve(value);
    };

    const check = () => {
      const err = node.getAttribute('data-rzn-err');
      if (err) return finish(new Error(err));
      const resp = node.getAttribute('data-rzn-resp');
      if (!resp) return;
      try {
        finish(null, JSON.parse(resp));
      } catch (e: any) {
        finish(new Error(e?.message || String(e)));
      }
    };

    const observer = new MutationObserver(() => check());
    observer.observe(node, {
      attributes: true,
      attributeFilter: ['data-rzn-resp', 'data-rzn-err'],
    });

    const timer = setTimeout(() => finish(new Error('Page bridge timeout')), timeoutMs);
    void contentCancellationPromise(payload).catch((error) => {
      finish(error instanceof Error ? error : new Error(String(error)));
    });
    check();
  });
}

async function tryEvalViaPageBridge(step: any, executionBackend: string): Promise<any | null> {
  try {
    const timeoutMs = Number(step?.timeout_ms ?? step?.timeoutMs ?? 10000);
    const timeout = Number.isFinite(timeoutMs) ? Math.max(0, Math.round(timeoutMs)) : 10000;
    const resp = await sendPageBridgeRequest(
      'eval_main_world',
      {
        script: String(step?.script || ''),
        lease_id: contentLeaseId(step),
        req_id: contentRequestId(step),
        args: Array.isArray(step?.args) ? step.args : [],
        params:
          step?.params && typeof step.params === 'object' && !Array.isArray(step.params)
            ? step.params
            : {},
        return_value: step?.return_value !== false,
      },
      timeout
    );
    if (!resp?.success) return null;
    return {
      success: true,
      world: 'main',
      execution_backend: executionBackend,
      result: resp.result,
    };
  } catch {
    return null;
  }
}

function actionFailureResponseFields(result: any): { success: boolean; error_code?: string; error_msg?: string } {
  if (!isActionResultFailure(result)) {
    return { success: true };
  }

  return {
    success: false,
    error_code: typeof result.error_code === 'string' ? result.error_code : 'ACTION_FAILED',
    error_msg: actionResultFailureMessage(result),
  };
}

function isTypedActionResultEnvelope(value: any): boolean {
  return (
    value &&
    typeof value === 'object' &&
    typeof value.success === 'boolean' &&
    (value.status === 'ok' || value.status === 'error') &&
    typeof value.action === 'string' &&
    'result' in value &&
    Array.isArray(value.warnings) &&
    Array.isArray(value.artifacts) &&
    typeof value.timestamp === 'number'
  );
}

function bridgeResultPayload(result: any): any {
  return isTypedActionResultEnvelope(result) ? result.result : result;
}

async function tryNativeClickViaPageBridge(
  selector: string,
  timeoutMs: number,
  executionBackend: string
): Promise<any | null> {
  try {
    const timeout = Number.isFinite(timeoutMs) ? Math.max(0, Math.round(timeoutMs)) : 5000;
    const resp = await sendPageBridgeRequest(
      'native_click',
      { selector },
      timeout
    );
    if (!resp?.success) return null;
    return {
      success: true,
      world: 'main',
      execution_backend: executionBackend,
      result: resp.result,
    };
  } catch {
    return null;
  }
}

const actionHandlers = {
  // Minimal observe stub: returns candidate selectors for repeated items
  observe: async (step: any) => {
    const instruction: string = step.instruction || step.query || '';
    const maxItems: number = step.max_items || 10;
    const scopeSel: string | undefined = step.scope_selector;

    // Determine scope
    let scope: Element | Document = document;
    if (scopeSel) {
      const el = document.querySelector(scopeSel);
      if (el) scope = el;
    }

    // Patterns to detect list items
    const patterns = [
      '.card', '.result', '.item', 'article', 'li', '.product', '.entry'
    ];

    type Cand = { selector: string; kind: 'item'|'list'|'value'|'table'; score: number; reason: string; sample_text?: string };
    const candidates: Cand[] = [];

    const qsa = (sel: string) => (scope instanceof Element ? scope.querySelectorAll(sel) : document.querySelectorAll(sel));

    for (const sel of patterns) {
      try {
        const els = qsa(sel);
        if (els.length >= 2) {
          const first = els[0] as HTMLElement;
          // Try to compute a container selector
          let containerSel = '';
          let parent: Element | null = first.parentElement;
          while (parent && !containerSel) {
            if (parent.id) containerSel = `#${parent.id}`;
            else if (parent.getAttribute('role') === 'main') containerSel = 'main';
            else parent = parent.parentElement;
          }
          const itemSelector = containerSel ? `${containerSel} ${sel}` : sel;
          const sample = (first.textContent || '').trim().slice(0, 80);
          candidates.push({ selector: itemSelector, kind: 'item', score: Math.min(1, 0.6 + Math.min(els.length, 10)/20), reason: `pattern ${sel}`, sample_text: sample });
          if (candidates.length >= maxItems) break;
        }
      } catch {}
    }

    // Basic table detection
    try {
      const tables = qsa('table');
      tables.forEach(t => {
        const id = (t as HTMLElement).id;
        const sel = id ? `#${id}` : 'table';
        if (t.querySelectorAll('tbody tr').length >= 2) {
          candidates.push({ selector: `${sel} tbody tr`, kind: 'item', score: 0.7, reason: 'table rows', sample_text: (t.textContent||'').trim().slice(0,60) });
        }
      });
    } catch {}

    // Fallback: look for repeated anchors in a section with headings
    if (candidates.length === 0) {
      try {
        const headings = qsa('#search, main, section');
        headings.forEach(h => {
          const links = h.querySelectorAll('a[href]');
          if (links.length >= 3) {
            const id = (h as HTMLElement).id;
            const base = id ? `#${id}` : 'section';
            candidates.push({ selector: `${base} a[href]`, kind: 'item', score: 0.5, reason: 'many links', sample_text: (links[0].textContent||'').trim().slice(0,60) });
          }
        });
      } catch {}
    }

    // Sort by score desc
    candidates.sort((a,b) => b.score - a.score);
    return { candidates };
  },
  navigate_to_url: async (step: any) => {
    if (step.url) {
      window.location.href = step.url;
      return true;
    }
    throw new Error('Missing URL for navigation');
  },

  click_element: async (step: any) => {
    const startedAt = Date.now();
    const element = await findElementWithRetry(step);

    // Native interactive controls are generally more reliable with a single native click.
    // Save the noisier pointer/mouse sequence for custom elements that emulate buttons.
    const el = element as HTMLElement;
    try {
      if (typeof el.scrollIntoView === 'function') {
        el.scrollIntoView({ block: 'center', inline: 'center' });
      }
    } catch {}

    const isNativeClickable =
      el instanceof HTMLButtonElement ||
      el instanceof HTMLAnchorElement ||
      el instanceof HTMLInputElement ||
      el instanceof HTMLSelectElement ||
      el instanceof HTMLTextAreaElement ||
      el instanceof HTMLLabelElement ||
      el.tagName.toLowerCase() === 'summary';
    const inputType = el instanceof HTMLInputElement ? el.type.toLowerCase() : '';
    const prefersMainWorldClick =
      el instanceof HTMLButtonElement ||
      el instanceof HTMLAnchorElement ||
      el.tagName.toLowerCase() === 'summary' ||
      (el instanceof HTMLInputElement &&
        ['button', 'submit', 'reset', 'image'].includes(inputType));

    if (isNativeClickable && prefersMainWorldClick) {
      const selectorFromStep = (() => {
        try {
          return resolveSelectorWithRefs(step?.selector);
        } catch {
          return undefined;
        }
      })();
      const selectorForMainWorld = selectorFromStep || bestEffortSelector(el);

      if (selectorForMainWorld) {
        const mainWorldClick = await tryNativeClickViaPageBridge(
          selectorForMainWorld,
          Number(step?.timeout_ms ?? step?.timeoutMs ?? 5000),
          'page_bridge_native_click'
        );

        if (mainWorldClick?.result?.clicked) {
          if (step.successCriteria) {
            return await validateWithTimeout(step.successCriteria.validationRules || [], step.timeoutMs || 5000);
          }
          return actionSuccess({
            action: 'click_element',
            result: { clicked: true, selector: selectorForMainWorld, method: 'page_bridge_native_click' },
            duration_ms: Date.now() - startedAt,
            legacy: { selector: selectorForMainWorld, method: 'page_bridge_native_click' },
          });
        }
      }
    }

    if (isNativeClickable) {
      try {
        if (typeof (el as any).click === 'function') {
          (el as any).click();
        } else {
          el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, composed: true }));
        }
      } catch {
        el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, composed: true }));
      }

      if (step.successCriteria) {
        return await validateWithTimeout(step.successCriteria.validationRules || [], step.timeoutMs || 5000);
      }
      return actionSuccess({
        action: 'click_element',
        result: { clicked: true, selector: step.selector, method: 'native_click' },
        duration_ms: Date.now() - startedAt,
        legacy: { selector: step.selector, method: 'native_click' },
      });
    }

    const rect = (() => {
      try {
        return el.getBoundingClientRect();
      } catch {
        return null;
      }
    })();
    const clientX = rect ? rect.left + rect.width / 2 : 0;
    const clientY = rect ? rect.top + rect.height / 2 : 0;

    const baseEventInit: any = {
      bubbles: true,
      cancelable: true,
      composed: true,
      clientX,
      clientY,
      button: 0,
      buttons: 1
    };

    const dispatch = (type: string, ctor: any) => {
      try {
        el.dispatchEvent(new ctor(type, baseEventInit));
        return true;
      } catch {
        return false;
      }
    };

    const pointerOk = typeof (window as any).PointerEvent === 'function';
    if (pointerOk) {
      dispatch('pointerover', (window as any).PointerEvent);
      dispatch('pointerenter', (window as any).PointerEvent);
      dispatch('pointerdown', (window as any).PointerEvent);
      dispatch('pointerup', (window as any).PointerEvent);
    }

    dispatch('mouseover', MouseEvent);
    dispatch('mouseenter', MouseEvent);
    dispatch('mousedown', MouseEvent);
    dispatch('mouseup', MouseEvent);
    try {
      if (typeof (el as any).click === 'function') {
        (el as any).click();
      } else {
        dispatch('click', MouseEvent);
      }
    } catch {
      dispatch('click', MouseEvent);
    }

    if (step.successCriteria) {
      return await validateWithTimeout(step.successCriteria.validationRules || [], step.timeoutMs || 5000);
    }
    return actionSuccess({
      action: 'click_element',
      result: { clicked: true, selector: step.selector, method: pointerOk ? 'pointer_mouse_events' : 'mouse_events' },
      duration_ms: Date.now() - startedAt,
      legacy: { selector: step.selector, method: pointerOk ? 'pointer_mouse_events' : 'mouse_events' },
    });
  },

  hover_element: async (step: any) => {
    const element = await findElementWithRetry(step);
    const el = element as HTMLElement;

    try {
      if (typeof el.scrollIntoView === 'function') {
        el.scrollIntoView({ block: 'center', inline: 'center' });
      }
    } catch {}

    const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
    const rect = (() => {
      try {
        return el.getBoundingClientRect();
      } catch {
        return null;
      }
    })();

    if (!rect || rect.width <= 0 || rect.height <= 0) {
      throw new Error('Target element has no visible rect for hover');
    }

    const clampPoint = (x: number, y: number) => ({
      x: Math.max(1, Math.min(window.innerWidth - 1, x)),
      y: Math.max(1, Math.min(window.innerHeight - 1, y)),
    });

    const randomBetween = (min: number, max: number) => min + Math.random() * Math.max(0, max - min);

    const finalPoint = clampPoint(
      rect.left + rect.width / 2 + randomBetween(-Math.min(rect.width * 0.08, 8), Math.min(rect.width * 0.08, 8)),
      rect.top + rect.height / 2 + randomBetween(-Math.min(rect.height * 0.08, 8), Math.min(rect.height * 0.08, 8)),
    );

    const startPoint = clampPoint(
      Math.max(2, rect.left - Math.min(48, rect.width * 0.2)),
      Math.max(2, rect.top + Math.min(rect.height * 0.2, 20)),
    );

    const buildEventInit = (x: number, y: number) => ({
      bubbles: true,
      cancelable: true,
      composed: true,
      clientX: x,
      clientY: y,
      screenX: x + window.screenX,
      screenY: y + window.screenY,
      movementX: 0,
      movementY: 0,
      relatedTarget: null,
      view: window,
    });

    const dispatchToTarget = (target: EventTarget | null, type: string, eventInit: any, bubble = true) => {
      if (!target || !(target instanceof Element || target === document || target === window)) return;
      const ctor = type.startsWith('pointer') && typeof (window as any).PointerEvent === 'function'
        ? (window as any).PointerEvent
        : MouseEvent;
      try {
        target.dispatchEvent(new ctor(type, { ...eventInit, bubbles: bubble }));
      } catch {}
    };

    const dispatchEnterChain = (currentTargets: Element[], eventInit: any) => {
      for (const target of currentTargets) {
        dispatchToTarget(target, 'pointerover', eventInit);
        dispatchToTarget(target, 'mouseover', eventInit);
      }
      for (const target of currentTargets) {
        dispatchToTarget(target, 'pointerenter', eventInit, false);
        dispatchToTarget(target, 'mouseenter', eventInit, false);
      }
    };

    const dispatchMove = (currentTargets: Element[], eventInit: any) => {
      dispatchToTarget(document, 'pointermove', eventInit);
      dispatchToTarget(document, 'mousemove', eventInit);
      dispatchToTarget(window, 'pointermove', eventInit);
      dispatchToTarget(window, 'mousemove', eventInit);
      for (const target of currentTargets) {
        dispatchToTarget(target, 'pointermove', eventInit);
        dispatchToTarget(target, 'mousemove', eventInit);
      }
    };

    const sameChain = (a: Element[], b: Element[]) =>
      a.length === b.length && a.every((item, index) => item === b[index]);

    const steps = Math.max(8, Math.min(18, Math.round(Math.max(rect.width, rect.height) / 28)));
    let previousTargets: Element[] = [];

    for (let i = 0; i <= steps; i += 1) {
      const progress = i / steps;
      const eased = progress < 0.5
        ? 4 * progress * progress * progress
        : 1 - Math.pow(-2 * progress + 2, 3) / 2;
      const currentPoint = clampPoint(
        startPoint.x + (finalPoint.x - startPoint.x) * eased,
        startPoint.y + (finalPoint.y - startPoint.y) * eased,
      );
      const eventInit = buildEventInit(currentPoint.x, currentPoint.y);
      const currentTargets = document
        .elementsFromPoint(currentPoint.x, currentPoint.y)
        .filter((node): node is Element => node instanceof Element)
        .slice(0, 6);

      if (!sameChain(previousTargets, currentTargets)) {
        dispatchEnterChain(currentTargets, eventInit);
      }
      dispatchMove(currentTargets, eventInit);
      previousTargets = currentTargets;
      await sleep(18 + Math.floor(Math.random() * 26));
    }

    const settleEventInit = buildEventInit(finalPoint.x, finalPoint.y);
    const finalTargets = document
      .elementsFromPoint(finalPoint.x, finalPoint.y)
      .filter((node): node is Element => node instanceof Element)
      .slice(0, 6);
    dispatchEnterChain(finalTargets, settleEventInit);
    dispatchMove(finalTargets, settleEventInit);
    await sleep(160 + Math.floor(Math.random() * 140));

    return true;
  },

  scroll_element_into_view: async (step: any) => {
    const element = await findElementWithRetry(step);
    const el = element as HTMLElement;

    try {
      if (typeof el.scrollIntoView === 'function') {
        el.scrollIntoView({ block: 'center', inline: 'center' });
      }
    } catch {}

    return true;
  },

  dbl_click_element: async (step: any) => {
    const element = await findElementWithRetry(step);

    const el = element as HTMLElement;
    try {
      if (typeof el.scrollIntoView === 'function') {
        el.scrollIntoView({ block: 'center', inline: 'center' });
      }
    } catch {}

    const rect = (() => {
      try {
        return el.getBoundingClientRect();
      } catch {
        return null;
      }
    })();
    const clientX = rect ? rect.left + rect.width / 2 : 0;
    const clientY = rect ? rect.top + rect.height / 2 : 0;

    const baseEventInit: any = {
      bubbles: true,
      cancelable: true,
      composed: true,
      clientX,
      clientY,
      button: 0,
      buttons: 1,
      detail: 2
    };

    const dispatch = (type: string, ctor: any) => {
      try {
        el.dispatchEvent(new ctor(type, { ...baseEventInit, type, detail: 2 }));
        return true;
      } catch {
        return false;
      }
    };

    const pointerOk = typeof (window as any).PointerEvent === 'function';
    if (pointerOk) {
      dispatch('pointerover', (window as any).PointerEvent);
      dispatch('pointerenter', (window as any).PointerEvent);
      dispatch('pointerdown', (window as any).PointerEvent);
      dispatch('pointerup', (window as any).PointerEvent);
    }

    dispatch('mouseover', MouseEvent);
    dispatch('mouseenter', MouseEvent);
    dispatch('mousedown', MouseEvent);
    dispatch('mouseup', MouseEvent);
    dispatch('click', MouseEvent);
    dispatch('dblclick', MouseEvent);

    // Fallback: invoke native click twice last.
    try {
      if (typeof (el as any).click === 'function') {
        (el as any).click();
        (el as any).click();
      }
    } catch {}

    if (step.successCriteria) {
      return await validateWithTimeout(step.successCriteria.validationRules || [], step.timeoutMs || 5000);
    }
    return true;
  },

  fill_input_field: async (step: any) => {
    console.log('fill_input_field step:', JSON.stringify(redactStepForLog(step)));
    const element = await findElementWithRetry(step) as Element;
    const fillDebug: Record<string, any> = {
      mode: 'unknown',
      useNativeInput: step.use_native_input === true,
      preferTrustedTextInsert: step.prefer_trusted_text_insert === true,
      clearFirst: step.clear_first !== false,
      simulateTyping: step.simulate_typing === true,
      branch: null,
      nativeInputReady: false,
      triedNative: false,
      nativeInserted: false,
      triedTrustedTextInsert: false,
      trustedTextInsertResponse: null,
      trustedTextInsertError: null,
      domFallbackUsed: false
    };
    (window as any).__rznLastFillDebug = fillDebug;

    const value = String(step.value ?? step.text ?? '');
    const clearFirst = step.clear_first !== false;
    const shouldSimulateTyping = step.simulate_typing === true;
    const useNativeInput = step.use_native_input === true;
    const preferTrustedTextInsert = step.prefer_trusted_text_insert === true;
    const nativeInputReady = useNativeInput ? await nativeInput.ensureAvailable({ force: true }) : false;
    fillDebug.nativeInputReady = nativeInputReady;
    const delayMsRaw = step.delay_ms;
    const delayMs =
      typeof delayMsRaw === 'number' && Number.isFinite(delayMsRaw) && delayMsRaw >= 0
        ? Math.floor(delayMsRaw)
        : undefined;
    const typingSpeed = (step.typing_speed as 'slow' | 'medium' | 'fast' | undefined) || 'medium';

    const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
    const tryTrustedTextInsert = async (text: string): Promise<boolean> => {
      fillDebug.triedTrustedTextInsert = true;
      try {
        const response = await chrome.runtime.sendMessage({
          action: 'type_text_cdp',
          text
        });
        fillDebug.trustedTextInsertResponse = response ?? null;
        return response?.success === true;
      } catch (error) {
        console.warn('Trusted CDP text insert failed:', error);
        fillDebug.trustedTextInsertError = error instanceof Error ? error.message : String(error);
        return false;
      }
    };
    const normalizeEditableText = (text: string | null | undefined) =>
      String(text || '').replace(/\u00a0/g, ' ').replace(/\s+/g, ' ').trim();
    const readElementText = (target: Element): string => {
      if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
        return String(target.value || '');
      }
      if (target instanceof HTMLElement) {
        return normalizeEditableText(target.innerText || target.textContent || '');
      }
      return '';
    };
    const confirmsInsertedText = async (target: Element, expected: string, previousText: string): Promise<boolean> => {
      await sleep(120);
      const currentText = readElementText(target);
      if (!currentText) return false;
      if (currentText === expected) return true;
      if (currentText.endsWith(expected)) return true;
      if (previousText && currentText !== previousText && currentText.includes(expected.slice(0, Math.min(3, expected.length)))) {
        return true;
      }
      return false;
    };

    const dispatchInputEvent = (
      target: HTMLElement,
      text: string | null,
      inputType?: string
    ) => {
      try {
        target.dispatchEvent(new InputEvent('input', {
          data: text ?? undefined,
          inputType,
          bubbles: true,
          cancelable: true
        }));
      } catch {
        target.dispatchEvent(new Event('input', { bubbles: true, cancelable: true }));
      }
    };

    const dispatchBeforeInputEvent = (
      target: HTMLElement,
      text: string | null,
      inputType?: string
    ) => {
      try {
        target.dispatchEvent(new InputEvent('beforeinput', {
          data: text ?? undefined,
          inputType,
          bubbles: true,
          cancelable: true
        }));
      } catch {
        target.dispatchEvent(new Event('beforeinput', { bubbles: true, cancelable: true }));
      }
    };

    const setFormControlValue = (
      target: HTMLInputElement | HTMLTextAreaElement,
      nextValue: string,
      data: string | null,
      inputType?: string
    ) => {
      const proto =
        target instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      const valueSetter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
      if (valueSetter) valueSetter.call(target, nextValue);
      else target.value = nextValue;
      dispatchInputEvent(target, data, inputType);
    };

    const emitKeyEvent = (
      target: HTMLInputElement | HTMLTextAreaElement,
      type: 'keydown' | 'keypress' | 'keyup',
      key: string
    ) => {
      const code = key.length === 1 && /^[a-z]$/i.test(key) ? `Key${key.toUpperCase()}` : 'Unidentified';
      const charCode = key.length === 1 ? key.charCodeAt(0) : 0;
      target.dispatchEvent(new KeyboardEvent(type, {
        key,
        code,
        keyCode: charCode || undefined,
        which: charCode || undefined,
        charCode: type === 'keypress' ? charCode : undefined,
        bubbles: true,
        cancelable: true,
        view: window
      }));
    };

    const domTypeFormControl = async (
      target: HTMLInputElement | HTMLTextAreaElement,
      text: string
    ) => {
      target.focus();
      if (!clearFirst) {
        try {
          const end = target.value.length;
          target.setSelectionRange(end, end);
        } catch {}
      }

      let currentValue = clearFirst ? '' : target.value;
      if (clearFirst) {
        setFormControlValue(target, '', null, 'deleteContentBackward');
      }

      if (shouldSimulateTyping) {
        const perCharDelay = delayMs ?? 40;
        for (const character of text) {
          emitKeyEvent(target, 'keydown', character);
          emitKeyEvent(target, 'keypress', character);
          currentValue += character;
          setFormControlValue(target, currentValue, character, 'insertText');
          emitKeyEvent(target, 'keyup', character);
          if (perCharDelay > 0) await sleep(perCharDelay);
        }
      } else {
        const nextValue = clearFirst ? text : `${target.value}${text}`;
        setFormControlValue(
          target,
          nextValue,
          text,
          clearFirst ? 'insertReplacementText' : 'insertText'
        );
      }

      target.dispatchEvent(new Event('change', { bubbles: true, cancelable: true }));
    };

    // Check if element is input or textarea
    if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
      fillDebug.mode = 'form_control';
      fillDebug.branch = 'form_control';
      // Focus the element first
      element.focus();
      const previousText = readElementText(element);

      // If requested, prefer Layer 2 native input first. If that is unavailable or
      // does not actually update the control, fall back to trusted CDP typing and
      // only then to DOM mutation.
      if (useNativeInput || preferTrustedTextInsert) {
        if (clearFirst) {
          setFormControlValue(element, '', null, 'deleteContentBackward');
          await sleep(40);
        } else {
          try {
            const end = element.value.length;
            element.setSelectionRange(end, end);
          } catch {}
        }

        let inserted = false;
        if (preferTrustedTextInsert) {
          inserted = await tryTrustedTextInsert(value);
        }

        if (!inserted && nativeInputReady) {
          fillDebug.triedNative = true;
          console.log('Using Layer 2 native input for fill_input_field');
          inserted =
            delayMs !== undefined
              ? await nativeInput.nativeType(value, {
                  typing_delay_ms: delayMs,
                  natural_variance: true
                })
              : await nativeInput.nativeTypeNatural(value, typingSpeed);
          fillDebug.nativeInserted = inserted;
          if (inserted && !(await confirmsInsertedText(element, value, previousText))) {
            inserted = false;
          }
        }

        if (!inserted && !(await confirmsInsertedText(element, value, previousText))) {
          const trustedInserted = await tryTrustedTextInsert(value);
          inserted = trustedInserted || (await confirmsInsertedText(element, value, previousText));
        }

        if (!inserted) {
          fillDebug.domFallbackUsed = true;
          console.warn('Trusted typing did not update the form control, falling back to DOM method');
          await domTypeFormControl(element, value);
        }
      } else {
        fillDebug.domFallbackUsed = true;
        await domTypeFormControl(element, value);
      }

      // Don't auto-submit - let the LLM generate a separate press_key step
      // This gives more control over the workflow
    } else if (element instanceof HTMLElement && (element.isContentEditable || element.getAttribute('contenteditable') === 'true')) {
      fillDebug.mode = 'contenteditable';
      fillDebug.branch = 'contenteditable';
      // Contenteditable support for modern rich editors (e.g. comment boxes)
      element.focus();

      const getStructuredEditableLeaf = (): HTMLElement | null =>
        (element.querySelector('[data-contents="true"] span[data-offset-key]') as HTMLElement | null) ||
        (element.querySelector('[data-contents="true"] [data-offset-key] span') as HTMLElement | null) ||
        (element.querySelector('[data-contents="true"] span') as HTMLElement | null);

      const setCursorToEnd = () => {
        try {
          const selection = window.getSelection();
          if (!selection) return;
          const range = document.createRange();
          const textWalker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT, {
            acceptNode: (node) => {
              const parent = node.parentElement;
              if (!parent || !element.contains(parent)) return NodeFilter.FILTER_REJECT;
              return (node.textContent || '').length > 0 ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP;
            }
          });
          let lastTextNode: Text | null = null;
          while (textWalker.nextNode()) {
            lastTextNode = textWalker.currentNode as Text;
          }

          if (lastTextNode) {
            range.setStart(lastTextNode, lastTextNode.textContent?.length || 0);
            range.collapse(true);
          } else {
            const draftLeaf =
              getStructuredEditableLeaf() ||
              element.querySelector('[data-contents="true"]');
            if (draftLeaf) {
              range.selectNodeContents(draftLeaf);
              range.collapse(false);
            } else {
              range.selectNodeContents(element);
              range.collapse(false);
            }
          }
          selection.removeAllRanges();
          selection.addRange(range);
        } catch {}
      };

      const clearEditable = () => {
        if (hasStructuredEditableContent) {
          const draftLeaf = getStructuredEditableLeaf();
          if (draftLeaf) {
            draftLeaf.replaceChildren();
            const br = document.createElement('br');
            br.setAttribute('data-text', 'true');
            draftLeaf.appendChild(br);
            dispatchInputEvent(element, null, 'deleteContentBackward');
            return;
          }
        }

        try {
          const selection = window.getSelection();
          if (selection) {
            const range = document.createRange();
            range.selectNodeContents(element);
            selection.removeAllRanges();
            selection.addRange(range);
          }
          // Prefer execCommand so frameworks observe input changes
          document.execCommand('delete');
        } catch {
          element.textContent = '';
        }
      };

      const insertStructuredEditableText = (text: string) => {
        const draftLeaf = getStructuredEditableLeaf();
        if (!draftLeaf) {
          element.textContent = `${element.textContent || ''}${text}`;
          dispatchInputEvent(element, text, 'insertText');
          return;
        }

        const existingBreak = draftLeaf.querySelector('br[data-text="true"]');
        if (existingBreak) existingBreak.remove();

        let textNode = Array.from(draftLeaf.childNodes).find((node) => node.nodeType === Node.TEXT_NODE) as Text | undefined;
        if (!textNode) {
          textNode = document.createTextNode('');
          draftLeaf.appendChild(textNode);
        }

        textNode.textContent = `${textNode.textContent || ''}${text}`;
        dispatchInputEvent(element, text, 'insertText');
      };

      const insertEditableText = (text: string) => {
        if (hasStructuredEditableContent) {
          insertStructuredEditableText(text);
          return;
        }

        setCursorToEnd();
        const beforeText = readElementText(element);
        const inserted = (() => {
          try {
            return document.execCommand('insertText', false, text);
          } catch {
            return false;
          }
        })();
        const afterExecText = readElementText(element);
        const execChangedText = afterExecText !== beforeText;
        if (!inserted && !execChangedText) {
          const draftLeaf =
            element.querySelector('[data-contents="true"] span[data-offset-key]') ||
            element.querySelector('[data-contents="true"] [data-offset-key] span') ||
            element.querySelector('[data-contents="true"] span');
          if (draftLeaf instanceof HTMLElement) {
            const currentText = draftLeaf.textContent === '\n' ? '' : (draftLeaf.textContent || '');
            draftLeaf.textContent = `${currentText}${text}`;
          } else {
            element.textContent = `${element.textContent || ''}${text}`;
          }
          dispatchInputEvent(element, text, 'insertText');
        }
      };

      const hasStructuredEditableContent = !!element.querySelector('[data-contents="true"]');
      const emitEditableKeyEvent = (
        type: 'keydown' | 'keypress' | 'keyup',
        key: string
      ) => {
        const code = key.length === 1 && /^[a-z]$/i.test(key) ? `Key${key.toUpperCase()}` : 'Unidentified';
        const charCode = key.length === 1 ? key.charCodeAt(0) : 0;
        element.dispatchEvent(new KeyboardEvent(type, {
          key,
          code,
          keyCode: charCode || undefined,
          which: charCode || undefined,
          charCode: type === 'keypress' ? charCode : undefined,
          bubbles: true,
          cancelable: true,
          view: window
        }));
      };

      const typeEditableCharacters = async (text: string) => {
        const perCharDelay = delayMs ?? 40;
        for (const character of text) {
          emitEditableKeyEvent('keydown', character);
          emitEditableKeyEvent('keypress', character);
          dispatchBeforeInputEvent(element, character, 'insertText');
          insertEditableText(character);
          emitEditableKeyEvent('keyup', character);
          if (perCharDelay > 0) await sleep(perCharDelay);
        }
      };

      if (clearFirst) {
        clearEditable();
        await sleep(30);
      } else {
        setCursorToEnd();
      }
      const previousText = readElementText(element);

      if (useNativeInput || preferTrustedTextInsert) {
        let inserted = false;
        if (preferTrustedTextInsert) {
          inserted = await tryTrustedTextInsert(value);
        }

        if (!inserted && nativeInputReady) {
          fillDebug.triedNative = true;
          console.log('Using Layer 2 native input for contenteditable fill_input_field');
          inserted =
            delayMs !== undefined
              ? await nativeInput.nativeType(value, {
                  typing_delay_ms: delayMs,
                  natural_variance: true
                })
              : await nativeInput.nativeTypeNatural(value, typingSpeed);
          fillDebug.nativeInserted = inserted;
          if (inserted && !(await confirmsInsertedText(element, value, previousText))) {
            inserted = false;
          }
        }

        if (!inserted && !(await confirmsInsertedText(element, value, previousText))) {
          const trustedInserted = await tryTrustedTextInsert(value);
          inserted = trustedInserted || (await confirmsInsertedText(element, value, previousText));
        }

        if (!inserted) {
          fillDebug.domFallbackUsed = true;
          console.warn('Trusted typing failed for contenteditable, falling back to DOM method');
          setCursorToEnd();
          if (shouldSimulateTyping || hasStructuredEditableContent) {
            await typeEditableCharacters(value);
          } else {
            insertEditableText(value);
          }
        }
      } else {
        fillDebug.domFallbackUsed = true;
        setCursorToEnd();
        if (shouldSimulateTyping || hasStructuredEditableContent) {
          await typeEditableCharacters(value);
        } else {
          insertEditableText(value);
        }
      }
    } else {
      throw new Error(`Element is not an input, textarea, or contenteditable: ${element.tagName}`);
    }
    
    if (step.successCriteria) {
      return await validateWithTimeout(step.successCriteria.validationRules || [], step.timeoutMs || 5000);
    }
    return true;
  },

  fill_and_submit: async (step: any) => {
    const value = String(step.value ?? step.text ?? '');
    if (!value) throw new Error('Missing value for fill_and_submit');

    const timeoutMs = Math.max(500, Number(step.timeout_ms ?? step.timeoutMs ?? 10000));
    const waitTimeoutMs = Math.max(0, Number(step.wait_timeout_ms ?? step.waitTimeoutMs ?? 45000));
    let pageBridgeError: string | null = null;
    try {
      const bridgeResp = await sendPageBridgeRequest(
        'fill_and_submit',
        {
          selector: step.selector,
          value,
          submit_selector: step.submit_selector ?? step.submitSelector,
          submit_label_regex: step.submit_label_regex ?? step.submitLabelRegex,
          wait_for_increase_selector: step.wait_for_increase_selector ?? step.waitForIncreaseSelector,
          timeout_ms: timeoutMs,
          wait_timeout_ms: waitTimeoutMs
        },
        timeoutMs + waitTimeoutMs + 3000
      );
      if (bridgeResp?.success && bridgeResp.result?.submitted) {
        return {
          success: true,
          execution_backend: 'page_bridge_main_world_fill_and_submit',
          ...bridgeResp.result
        };
      }
      if (bridgeResp && bridgeResp.success === false) {
        pageBridgeError = bridgeResp.error_msg || bridgeResp.error || JSON.stringify(bridgeResp);
      }
    } catch (error: any) {
      pageBridgeError = error?.message || String(error);
    }

    const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
    const normalize = (text: string | null | undefined) =>
      String(text || '').replace(/\u00a0/g, ' ').replace(/\s+/g, ' ').trim();
    const readText = (target: Element | null): string => {
      if (!target) return '';
      if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
        return String(target.value || '');
      }
      if (target instanceof HTMLElement) {
        return normalize(target.innerText || target.textContent || '');
      }
      return normalize(target.textContent || '');
    };
    const dispatchEditableInput = (target: HTMLElement, data: string | null, inputType: string) => {
      try {
        target.dispatchEvent(new InputEvent('beforeinput', {
          bubbles: true,
          cancelable: true,
          composed: true,
          data: data ?? undefined,
          inputType
        }));
      } catch {
        target.dispatchEvent(new Event('beforeinput', { bubbles: true, cancelable: true, composed: true }));
      }
      try {
        target.dispatchEvent(new InputEvent('input', {
          bubbles: true,
          cancelable: true,
          composed: true,
          data: data ?? undefined,
          inputType
        }));
      } catch {
        target.dispatchEvent(new Event('input', { bubbles: true, cancelable: true, composed: true }));
      }
    };
    const setEditableTextFallback = (target: HTMLElement, text: string) => {
      target.focus();
      try {
        const selection = window.getSelection();
        const range = document.createRange();
        range.selectNodeContents(target);
        selection?.removeAllRanges();
        selection?.addRange(range);
        document.execCommand('delete');
        const inserted = document.execCommand('insertText', false, text);
        if (inserted && normalize(readText(target)).includes(normalize(text))) {
          dispatchEditableInput(target, text, 'insertText');
          return;
        }
      } catch {}

      const block = document.createElement('p');
      block.textContent = text;
      target.replaceChildren(block);
      dispatchEditableInput(target, text, 'insertReplacementText');
    };
    const setFormControlFallback = (target: HTMLInputElement | HTMLTextAreaElement, text: string) => {
      target.focus();
      const proto = target instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
      if (setter) setter.call(target, text);
      else target.value = text;
      try {
        target.dispatchEvent(new InputEvent('input', {
          bubbles: true,
          cancelable: true,
          composed: true,
          data: text,
          inputType: 'insertReplacementText'
        }));
      } catch {
        target.dispatchEvent(new Event('input', { bubbles: true, cancelable: true, composed: true }));
      }
      target.dispatchEvent(new Event('change', { bubbles: true, cancelable: true }));
    };
    const findCurrentTarget = () => {
      const selector = String(step.selector || '').trim();
      if (!selector) return null;
      return document.querySelector(selector) || querySelectorDeep(selector);
    };
    const labelFor = (target: Element) =>
      normalize([
        target.getAttribute('aria-label') || '',
        target.getAttribute('title') || '',
        target.getAttribute('name') || '',
        target.getAttribute('type') || '',
        target.getAttribute('data-testid') || '',
        target.textContent || ''
      ].join(' '));
    const isEnabledButton = (target: Element) => {
      if (!(target instanceof HTMLElement) || !isElementVisible(target)) return false;
      if (target.matches(':disabled') || target.getAttribute('aria-disabled') === 'true') return false;
      return true;
    };
    const findSubmitButton = (composer: Element | null): HTMLElement | null => {
      const explicitSelector = String(step.submit_selector || step.submitSelector || '').trim();
      if (explicitSelector) {
        const explicit =
          document.querySelector(explicitSelector) ||
          querySelectorDeep(explicitSelector) ||
          composer?.closest('form')?.querySelector(explicitSelector);
        if (explicit instanceof HTMLElement && isEnabledButton(explicit)) return explicit;
      }

      const labelRegexRaw = String(step.submit_label_regex || step.submitLabelRegex || 'send|submit').trim();
      const labelRegex = new RegExp(labelRegexRaw, 'i');
      const scopes = [
        composer?.closest('form'),
        composer?.parentElement,
        composer?.parentElement?.parentElement,
        document
      ].filter(Boolean) as ParentNode[];
      const seen = new Set<Element>();
      for (const scope of scopes) {
        const candidates = Array.from(scope.querySelectorAll("button, [role='button'], input[type='submit'], input[type='button']"));
        for (const candidate of candidates) {
          if (seen.has(candidate)) continue;
          seen.add(candidate);
          if (!isEnabledButton(candidate)) continue;
          if (labelRegex.test(labelFor(candidate))) return candidate as HTMLElement;
        }
      }
      return null;
    };
    const summarizeButton = (button: HTMLElement | null) => {
      if (!button) return null;
      const rect = button.getBoundingClientRect();
      return {
        tag: button.tagName.toLowerCase(),
        label: labelFor(button).slice(0, 120),
        rect: {
          x: Math.round(rect.x),
          y: Math.round(rect.y),
          width: Math.round(rect.width),
          height: Math.round(rect.height)
        }
      };
    };
    const visibleButtonSummaries = () =>
      Array.from(document.querySelectorAll("button, [role='button'], input[type='submit'], input[type='button']"))
        .filter(candidate => candidate instanceof HTMLElement && isElementVisible(candidate))
        .slice(-20)
        .map(candidate => ({
          label: labelFor(candidate).slice(0, 120),
          disabled: candidate.matches(':disabled'),
          aria_disabled: candidate.getAttribute('aria-disabled'),
          tag: candidate.tagName.toLowerCase()
        }));
    const waitForIncrease = async (selector: string, initialCount: number | null) => {
      if (!selector || initialCount === null) {
        return { increased: false, afterCount: initialCount };
      }
      const waitDeadline = Date.now() + waitTimeoutMs;
      let afterCount = initialCount;
      while (Date.now() < waitDeadline) {
        afterCount = document.querySelectorAll(selector).length;
        if (afterCount > initialCount) {
          return { increased: true, afterCount };
        }
        await sleep(500);
      }
      return { increased: false, afterCount };
    };
    const dispatchEnterSubmit = (target: Element | null) => {
      const eventTarget = target instanceof HTMLElement ? target : document.activeElement;
      if (!(eventTarget instanceof HTMLElement)) return false;
      eventTarget.focus();
      const init: KeyboardEventInit = {
        key: 'Enter',
        code: 'Enter',
        keyCode: 13,
        which: 13,
        bubbles: true,
        cancelable: true,
        composed: true,
        view: window
      };
      const downAccepted = eventTarget.dispatchEvent(new KeyboardEvent('keydown', init));
      eventTarget.dispatchEvent(new KeyboardEvent('keypress', init));
      eventTarget.dispatchEvent(new KeyboardEvent('keyup', init));
      const form = eventTarget.closest('form');
      if (form instanceof HTMLFormElement) {
        try {
          form.requestSubmit();
          return true;
        } catch {
          try {
            form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true, composed: true }));
            return true;
          } catch {}
        }
      }
      return downAccepted;
    };

    let target = await findElementWithRetry(step) as Element;
    const beforeCountSelector = String(step.wait_for_increase_selector || step.waitForIncreaseSelector || '').trim();
    const beforeCount = beforeCountSelector ? document.querySelectorAll(beforeCountSelector).length : null;

    await actionHandlers.fill_input_field({
      ...step,
      type: 'fill_input_field',
      value,
      clear_first: step.clear_first !== false,
      simulate_typing: step.simulate_typing === true
    });
    await sleep(180);

    target = findCurrentTarget() || target;
    let textConfirmed = normalize(readText(target)).includes(normalize(value));
    if (!textConfirmed) {
      if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
        setFormControlFallback(target, value);
      } else if (target instanceof HTMLElement && (target.isContentEditable || target.getAttribute('contenteditable'))) {
        setEditableTextFallback(target, value);
      }
      await sleep(180);
      target = findCurrentTarget() || target;
      textConfirmed = normalize(readText(target)).includes(normalize(value));
    }

    const submitDeadline = Date.now() + timeoutMs;
    let submitButton: HTMLElement | null = null;
    while (Date.now() < submitDeadline) {
      target = findCurrentTarget() || target;
      submitButton = findSubmitButton(target);
      if (submitButton) break;
      await sleep(150);
    }
    if (!submitButton) {
      const keyboardSubmitted = dispatchEnterSubmit(target);
      const keyboardWait = await waitForIncrease(beforeCountSelector, beforeCount);
      if (keyboardWait.increased) {
        return {
          success: true,
          filled: textConfirmed,
          submitted: true,
          submit_method: 'keyboard_enter',
          keyboard_submitted: keyboardSubmitted,
          submit_button: null,
          wait_for_increase_selector: beforeCountSelector || null,
          count_before: beforeCount,
          count_after: keyboardWait.afterCount,
          increased: true
        };
      }
      throw new Error(
        `No enabled submit button found for fill_and_submit: ${JSON.stringify({
          page_bridge_error: pageBridgeError,
          text_confirmed: textConfirmed,
          target_text: normalize(readText(target)).slice(0, 120),
          visible_buttons: visibleButtonSummaries()
        })}`
      );
    }

    try {
      submitButton.scrollIntoView({ block: 'center', inline: 'center' });
    } catch {}
    submitButton.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true, composed: true, view: window }));
    submitButton.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, cancelable: true, composed: true, view: window }));
    submitButton.click();

    const { increased, afterCount } = await waitForIncrease(beforeCountSelector, beforeCount);

    return {
      success: true,
      filled: textConfirmed,
      submitted: true,
      submit_method: 'button_click',
      submit_button: summarizeButton(submitButton),
      wait_for_increase_selector: beforeCountSelector || null,
      count_before: beforeCount,
      count_after: afterCount,
      increased
    };
  },

  type_text: async (step: any) => {
    const startedAt = Date.now();
    const text = String(step.text ?? step.value ?? '');
    if (!text) {
      return actionSuccess({
        action: 'type_text',
        result: { inserted: false, textLength: 0, reason: 'empty_text' },
        duration_ms: Date.now() - startedAt,
        legacy: { textLength: 0 },
      });
    }

    const target = step.selector
      ? await findElementWithRetry(step)
      : (document.activeElement as Element | null);
    if (!target) {
      throw new Error('No target element found for type_text');
    }

    const useNativeInput = step.use_native_input === true;
    const nativeInputReady = useNativeInput ? await nativeInput.ensureAvailable({ force: true }) : false;
    const delayMsRaw = step.delay_ms;
    const delayMs =
      typeof delayMsRaw === 'number' && Number.isFinite(delayMsRaw) && delayMsRaw >= 0
        ? Math.floor(delayMsRaw)
        : undefined;
    const typingSpeed = (step.typing_speed as 'slow' | 'medium' | 'fast' | undefined) || 'medium';
    const shouldSimulateTyping = step.simulate_typing === true;
    const allowCdpTyping = step.use_cdp === true || step.prefer_trusted_text_insert === true;

    const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
    const dispatchInputEvent = (
      targetEl: HTMLElement,
      data: string | null,
      inputType?: string
    ) => {
      try {
        targetEl.dispatchEvent(new InputEvent('input', {
          data: data ?? undefined,
          inputType,
          bubbles: true,
          cancelable: true
        }));
      } catch {
        targetEl.dispatchEvent(new Event('input', { bubbles: true, cancelable: true }));
      }
    };

    const placeCursorAtEnd = (element: Element) => {
      if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
        element.focus();
        try {
          const end = element.value.length;
          element.setSelectionRange(end, end);
        } catch {}
        return;
      }

      if (element instanceof HTMLElement && (element.isContentEditable || element.getAttribute('contenteditable') === 'true')) {
        element.focus();
        try {
          const selection = window.getSelection();
          if (!selection) return;
          const range = document.createRange();
          range.selectNodeContents(element);
          range.collapse(false);
          selection.removeAllRanges();
          selection.addRange(range);
        } catch {}
      }
    };

    const domTypeCharacters = async (element: Element, value: string) => {
      placeCursorAtEnd(element);

      if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
        const proto =
          element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
        const valueSetter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
        let current = String(element.value || '');
        if (!shouldSimulateTyping) {
          const nextValue = `${current}${value}`;
          if (valueSetter) valueSetter.call(element, nextValue);
          else element.value = nextValue;
          dispatchInputEvent(element, value, 'insertText');
          element.dispatchEvent(new Event('change', { bubbles: true, cancelable: true }));
          return;
        }
        const perCharDelay = delayMs ?? 35;
        for (const character of value) {
          current += character;
          if (valueSetter) valueSetter.call(element, current);
          else element.value = current;
          dispatchInputEvent(element, character, 'insertText');
          if (perCharDelay > 0) await sleep(perCharDelay);
        }
        return;
      }

      if (element instanceof HTMLElement && (element.isContentEditable || element.getAttribute('contenteditable') === 'true')) {
        if (!shouldSimulateTyping) {
          placeCursorAtEnd(element);
          const beforeText = element.innerText || element.textContent || '';
          let inserted = false;
          try {
            inserted = document.execCommand('insertText', false, value);
          } catch {
            inserted = false;
          }
          const afterText = element.innerText || element.textContent || '';
          if (!inserted && afterText === beforeText) {
            element.textContent = `${element.textContent || ''}${value}`;
            dispatchInputEvent(element, value, 'insertText');
          }
          return;
        }
        const perCharDelay = delayMs ?? 35;
        for (const character of value) {
          placeCursorAtEnd(element);
          const beforeText = element.innerText || element.textContent || '';
          let inserted = false;
          try {
            inserted = document.execCommand('insertText', false, character);
          } catch {
            inserted = false;
          }
          const afterText = element.innerText || element.textContent || '';
          if (!inserted && afterText === beforeText) {
            element.textContent = `${element.textContent || ''}${character}`;
            dispatchInputEvent(element, character, 'insertText');
          }
          if (perCharDelay > 0) await sleep(perCharDelay);
        }
        return;
      }

      throw new Error(`Unsupported target for type_text: ${(element as Element).tagName}`);
    };

    placeCursorAtEnd(target);

    let inserted = false;
    if (nativeInputReady) {
      inserted =
        delayMs !== undefined
          ? await nativeInput.nativeType(text, {
              typing_delay_ms: delayMs,
              natural_variance: true
            })
          : await nativeInput.nativeTypeNatural(text, typingSpeed);
    }

    if (!inserted) {
      await domTypeCharacters(target, text);
      inserted = true;
    }

    if (allowCdpTyping) {
      try {
        const response = await chrome.runtime.sendMessage({
          action: 'type_text_cdp',
          text
        });
        inserted = response?.success === true || inserted;
      } catch (error) {
        console.warn('type_text_cdp failed:', error);
      }
    }

    if (step.successCriteria) {
      return await validateWithTimeout(step.successCriteria.validationRules || [], step.timeoutMs || 5000);
    }
    return actionSuccess({
      action: 'type_text',
      result: {
        inserted,
        textLength: text.length,
        method: nativeInputReady ? 'native_or_dom' : 'dom',
        used_cdp: allowCdpTyping,
      },
      duration_ms: Date.now() - startedAt,
      legacy: { textLength: text.length },
    });
  },

  // Generic, robust text submission for search boxes and forms
  // Attempts Enter → form.requestSubmit/submit → click submit-like buttons
  submit_text_query: async (step: any) => {
    cslog.logInfo('submit_text_query: start', { step: redactStepForLog(step) });
    const sel: string | undefined = step.selector;
    const value: string | undefined = step.value;
    const timeoutMs: number = step.timeout_ms || step.timeoutMs || 10000;
    const pressEnterFirst: boolean = step.press_enter_first !== false; // default true
    const tryFormSubmit: boolean = step.try_form_submit !== false; // default true
    const buttonSelectors: string = step.button_selectors || "input[type='submit'], button[type='submit'], input[name='btnK'], input[value='Google Search'], button[aria-label*='search' i]";

    // Resolve target input
    let inputEl: HTMLElement | null = null;
    if (sel) {
      inputEl = (await findElementWithRetry({ selector: sel })) as HTMLElement;
    } else {
      inputEl = (document.activeElement as HTMLElement) || null;
      if (!(inputEl instanceof HTMLInputElement || inputEl instanceof HTMLTextAreaElement)) {
        // Try common search inputs
        inputEl = document.querySelector("input[name='q'], textarea[name='q'], input[type='search']") as HTMLElement | null;
      }
    }
    if (!inputEl) {
      cslog.logWarn('submit_text_query: no input element resolved', { step: redactStepForLog(step) });
      return { success: false, reason: 'no_input' };
    }

    // Fill value if provided
    if (typeof value === 'string') {
      (inputEl as any).focus?.();
      if (inputEl instanceof HTMLInputElement || inputEl instanceof HTMLTextAreaElement) {
        inputEl.value = '';
        inputEl.value = value;
        inputEl.dispatchEvent(new Event('input', { bubbles: true, cancelable: true }));
        inputEl.dispatchEvent(new Event('change', { bubbles: true, cancelable: true }));
      }
      cslog.logDebug('submit_text_query: filled value', { length: (value||'').length });
    }

    // Helper: check if suggestions overlay is open (combobox/listbox pattern)
    const hasSuggestionsOpen = (): boolean => {
      try {
        const active = document.activeElement as HTMLElement | null;
        if (!active) return false;
        const expanded = active.getAttribute('aria-expanded');
        if (expanded === 'true') return true;
        // Visible listbox nearby
        const listbox = document.querySelector('[role="listbox"], [role="listbox"] [role="option"]');
        if (listbox && (listbox as HTMLElement).offsetParent !== null) return true;
      } catch {}
      return false;
    };

    const beforeUrl = window.location.href;
    const beforeHash = domHash();
    cslog.logDebug('submit_text_query: baseline', { beforeUrl, beforeHash });

    const waitForChange = async (deadline: number): Promise<boolean> => {
      while (Date.now() < deadline) {
        const nowHash = domHash();
        if (nowHash !== beforeHash) return true;
        if (window.location.href !== beforeUrl) return true;
        await new Promise(r => setTimeout(r, 150));
      }
      return false;
    };

    // Attempt 1: Press Enter on the input
    if (pressEnterFirst) {
      (inputEl as any).focus?.();
      const kd = new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true });
      const defaultPrevented = !inputEl.dispatchEvent(kd);
      if (!defaultPrevented) {
        const kp = new KeyboardEvent('keypress', { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, charCode: 13, bubbles: true, cancelable: true });
        inputEl.dispatchEvent(kp);
      }
      const ku = new KeyboardEvent('keyup', { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true });
      inputEl.dispatchEvent(ku);

      const changed = await waitForChange(Date.now() + Math.min(1500, timeoutMs));
      cslog.logDebug('submit_text_query: enter attempt done', { changed });
      if (changed) return { success: true, method: 'enter' };
    }

    // Attempt 2: If suggestions overlay open, prefer semantic submit (form.submit or button) instead of re-pressing Enter
    if (hasSuggestionsOpen()) {
      const form = inputEl.closest('form') as HTMLFormElement | null;
      if (form) {
        if (typeof form.requestSubmit === 'function') form.requestSubmit();
        else if (typeof form.submit === 'function') form.submit();
        const changed2a = await waitForChange(Date.now() + Math.min(1500, timeoutMs));
        cslog.logDebug('submit_text_query: listbox open → form submit', { changed: changed2a });
        if (changed2a) return { success: true, method: 'listbox->form.submit' };
      }
      // Try a submit-like button within the same form first
      const localBtn = form?.querySelector("input[type='submit'], button[type='submit'], input[name='btnK'], button[aria-label*='search' i]") as HTMLElement | null;
      if (localBtn && localBtn.offsetParent !== null) {
        localBtn.click();
        const changed2b = await waitForChange(Date.now() + Math.min(1500, timeoutMs));
        cslog.logDebug('submit_text_query: listbox open → local button click', { changed: changed2b });
        if (changed2b) return { success: true, method: 'listbox->local.button' };
      }
      // As last resort, Escape then Enter once
      const esc = new KeyboardEvent('keydown', { key: 'Escape', code: 'Escape', keyCode: 27, which: 27, bubbles: true, cancelable: true });
      inputEl.dispatchEvent(esc);
      await new Promise(r => setTimeout(r, 50));
      const kd2 = new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true });
      inputEl.dispatchEvent(kd2);
      const ku2 = new KeyboardEvent('keyup', { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true });
      inputEl.dispatchEvent(ku2);
      const changed2 = await waitForChange(Date.now() + Math.min(1200, timeoutMs));
      cslog.logDebug('submit_text_query: escape+enter done', { changed: changed2 });
      if (changed2) return { success: true, method: 'escape+enter' };
    }

    // Attempt 3: Submit enclosing form
    if (tryFormSubmit) {
      const form = inputEl.closest('form') as HTMLFormElement | null;
      if (form) {
        if (typeof form.requestSubmit === 'function') {
          form.requestSubmit();
        } else if (typeof form.submit === 'function') {
          form.submit();
        }
        const changed3 = await waitForChange(Date.now() + Math.min(1500, timeoutMs));
        cslog.logDebug('submit_text_query: form submit done', { changed: changed3 });
        if (changed3) return { success: true, method: 'form.submit' };
      }
    }

    // Attempt 4: Click submit-like buttons (prefer within same form)
    const form = inputEl.closest('form') as HTMLFormElement | null;
    const formButtons = form ? Array.from(form.querySelectorAll(buttonSelectors)) as HTMLElement[] : [];
    const globalButtons = Array.from(document.querySelectorAll(buttonSelectors)) as HTMLElement[];
    const buttons = [...formButtons, ...globalButtons];
    for (const btn of buttons) {
      if (btn.offsetParent === null) continue;
      btn.click();
      const changed4 = await waitForChange(Date.now() + Math.min(1500, timeoutMs));
      cslog.logDebug('submit_text_query: button click attempt', { selector: buttonSelectors, changed: changed4 });
      if (changed4) return { success: true, method: 'button.click' };
    }

    // Attempt 5: CDP Enter (trusted) if allowed
    const allowCdp: boolean = step.allow_cdp !== false; // default true
    if (allowCdp) {
      try {
        // Ensure input has focus in DOM world
        (inputEl as any).focus?.();
        const resp = await chrome.runtime.sendMessage({ action: 'press_key_cdp', key: 'Enter' });
        cslog.logDebug('submit_text_query: cdp enter attempt', { ok: !!(resp && resp.success) });
        if (resp && resp.success) {
          const changed5 = await waitForChange(Date.now() + Math.min(1500, timeoutMs));
          if (changed5) return { success: true, method: 'cdp.enter' };
        }
      } catch (e: any) {
        cslog.logWarn('submit_text_query: cdp enter failed', { error: e?.message || String(e) });
      }
    }

    cslog.logWarn('submit_text_query: all attempts failed');
    return { success: false, method: 'none' };
  },

  // Click a filter/tab/chip by visible text or aria-label (generic, domain-agnostic)
  apply_filter_by_text: async (step: any) => {
    const text: string = (step.text || '').toString().trim();
    const scopeSel: string | undefined = step.scope_selector;
    const timeoutMs: number = step.timeout_ms || 5000;
    if (!text) throw new Error('apply_filter_by_text requires step.text');
    const scope: Document | Element = scopeSel ? (document.querySelector(scopeSel) || document) : document;

    const visible = (el: Element) => {
      const s = getComputedStyle(el as HTMLElement);
      const r = (el as HTMLElement).getBoundingClientRect();
      return s && s.visibility !== 'hidden' && s.display !== 'none' && r.width > 1 && r.height > 1;
    };

    const candidates: Element[] = [];
    const selList = [
      '[role="button"]','button','a[role="tab"]','a[aria-label]',
      'input[type="radio"]+label','input[type="checkbox"]+label',
      'a','[role="link"]','[role="option"]'
    ];
    const qsa = (sel: string) => Array.from((scope instanceof Element ? scope : document).querySelectorAll(sel));
    for (const sel of selList) { candidates.push(...qsa(sel)); }
    const lower = text.toLowerCase();
    const match = candidates.find(el => {
      if (!visible(el)) return false;
      const label = (el.getAttribute('aria-label') || el.textContent || '').replace(/\s+/g,' ').trim().toLowerCase();
      return label.includes(lower);
    }) as HTMLElement | undefined;
    if (!match) throw new Error(`No filter element found matching text: ${text}`);
    match.click();
    await new Promise(r => setTimeout(r, Math.min(600, timeoutMs)));
    return { success: true };
  },

  // Attempt to set a date range using inputs or simple pickers (best-effort, generic)
  date_set_range: async (step: any) => {
    const from: string = step.from;
    const to: string | undefined = step.to;
    const scopeSel: string | undefined = step.scope_selector;
    const timeoutMs: number = step.timeout_ms || 5000;
    if (!from) throw new Error('date_set_range requires from');
    const scope: Document | Element = scopeSel ? (document.querySelector(scopeSel) || document) : document;

    const trySet = (el: HTMLInputElement, val: string) => {
      el.focus();
      el.value = val;
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
    };

    const dateInputs = Array.from((scope instanceof Element ? scope : document).querySelectorAll('input[type="date"], input[name*="date" i], input[placeholder*="date" i]')) as HTMLInputElement[];
    if (dateInputs.length) {
      if (dateInputs[0]) trySet(dateInputs[0], from);
      if (to && dateInputs[1]) trySet(dateInputs[1], to);
      await new Promise(r => setTimeout(r, Math.min(400, timeoutMs)));
      return { success: true, method: 'inputs' };
    }

    const opener = (scope instanceof Element ? scope : document).querySelector('[aria-label*="date" i], [aria-haspopup*="calendar" i], [class*="date" i]') as HTMLElement | null;
    if (opener) { opener.click(); await new Promise(r => setTimeout(r, 300)); return { success: true, method: 'opener' }; }
    return { success: false };
  },

  press_special_key: async (step: any) => {
    const keyMap: { [key: string]: string } = {
      'Enter': 'Enter',
      'Tab': 'Tab',
      'Escape': 'Escape',
      'ArrowUp': 'ArrowUp',
      'ArrowDown': 'ArrowDown',
      'ArrowLeft': 'ArrowLeft',
      'ArrowRight': 'ArrowRight'
    };
    
    const key = step.key || 'Enter';
    const keyCode = keyMap[key] || key;
    
    if (step.selector) {
      const element = await findElementWithRetry(step);
      element.focus();
    }
    
    const useNativeInput = step.use_native_input === true;
    const nativeInputReady = useNativeInput ? await nativeInput.ensureAvailable({ force: true }) : false;
    
    // If requested, use native input for key press (Layer 2)
    if (useNativeInput && nativeInputReady) {
      const nativeKey = key === 'Enter' ? 'Return' : keyCode;
      console.log('Using Layer 2 native input for key press', nativeKey);
      const success = await nativeInput.nativeKey(nativeKey);
      if (success) {
        // Native input succeeded
        if (step.successCriteria) {
          return await validateWithTimeout(step.successCriteria.validationRules || [], step.timeoutMs || 5000);
        }
        return true;
      } else {
        console.warn('Native Enter key failed, falling back to DOM method');
      }
    }
    
    // Regular DOM-based key press
    const keydownEvent = new KeyboardEvent('keydown', {
      key: keyCode,
      code: keyCode,
      keyCode: key === 'Enter' ? 13 : undefined,
      which: key === 'Enter' ? 13 : undefined,
      bubbles: true,
      cancelable: true,
      view: window
    });
    
    const target = step.selector ? document.querySelector(step.selector) : (document.activeElement || document.body);
    const defaultPrevented = !target.dispatchEvent(keydownEvent);
    
    // Also dispatch keypress for Enter (some forms listen to this)
    if (key === 'Enter' && !defaultPrevented) {
      const keypressEvent = new KeyboardEvent('keypress', {
        key: 'Enter',
        code: 'Enter',
        keyCode: 13,
        which: 13,
        charCode: 13,
        bubbles: true,
        cancelable: true,
        view: window
      });
      target.dispatchEvent(keypressEvent);
      
    }
    
    const keyupEvent = new KeyboardEvent('keyup', {
      key: keyCode,
      code: keyCode,
      keyCode: key === 'Enter' ? 13 : undefined,
      which: key === 'Enter' ? 13 : undefined,
      bubbles: true,
      cancelable: true,
      view: window
    });
    target.dispatchEvent(keyupEvent);
    
    if (step.successCriteria) {
      return await validateWithTimeout(step.successCriteria.validationRules || [], step.timeoutMs || 5000);
    }
    return true;
  },

  wait_for_element: async (step: any) => {
    let selector = step.selector;
    if (!selector) {
      throw new Error('Missing selector for wait_for_element');
    }

    const refIndex = parseRefIndex(selector);
    if (refIndex !== null) {
      const resolved = selectorForRefIndex(refIndex);
      if (!resolved) {
        throw new Error(`UNKNOWN_REF: ${selector} (take a fresh DOM snapshot and retry)`);
      }
      selector = resolved;
    }
    
    const timeoutMs = step.timeout_ms || step.timeoutMs || 30000;
    const startTime = Date.now();
    
    while (Date.now() - startTime < timeoutMs) {
      const element = findMatchingElement(selector, {
        pierceShadow: step.pierce_shadow === true,
        visibleOnly: step.visible === true,
        preferVisible: step.pierce_shadow === true,
      });
      if (element) {
        return true;
      }
      await withContentCancellation(step, 'wait_for_element poll', new Promise(resolve => setTimeout(resolve, 100)));
    }
    
    throw new Error(`Timeout waiting for element: ${selector}`);
  },

  wait_for_timeout: async (step: any) => {
    const timeoutMs = step.timeout_ms || step.timeoutMs || 1000;
    await withContentCancellation(step, 'wait_for_timeout', new Promise(resolve => setTimeout(resolve, timeoutMs)));
    return true;
  },

  take_screenshot: async (step: any) => {
    const formatRaw = typeof step.format === 'string' ? step.format : undefined;
    const qualityRaw = step.quality;
    const annotate = step.annotate === true;
    const annotateMaxLabels = step.annotate_max_labels;
    const annotateMaxElements = step.annotate_max_elements;

    const response: any = await new Promise((resolve, reject) => {
      try {
        chrome.runtime.sendMessage(
          {
            cmd: 'take_screenshot',
            format: formatRaw,
            quality: qualityRaw,
            annotate,
            annotate_max_labels: annotateMaxLabels,
            annotate_max_elements: annotateMaxElements,
          },
          (resp) => {
            const err = chrome.runtime.lastError;
            if (err) {
              reject(new Error(err.message));
              return;
            }
            resolve(resp);
          }
        );
      } catch (e) {
        reject(e);
      }
    });

    if (!response || response.success !== true || typeof response.dataUrl !== 'string') {
      const msg = response?.error ? String(response.error) : 'Screenshot capture failed';
      throw new Error(msg);
    }

    return {
      type: 'screenshot',
      format: formatRaw || 'png',
      full_page: !!step.full_page,
      data_url: response.dataUrl,
      annotated: response.annotated === true,
      annotations: response.annotations,
      annotate_error: response.annotate_error,
    };
  },

  // Window-level scroll (domain-agnostic)
  scroll_window_to: async (step: any) => {
    const currentX = window.scrollX || window.pageXOffset || 0;
    const currentY = window.scrollY || window.pageYOffset || 0;

    const dir = (step.direction || '').toString().toLowerCase();
    const dx = typeof step.x === 'number' ? step.x : currentX;
    let targetY: number = currentY;

    if (typeof step.y === 'number') {
      targetY = step.y;
    } else if (dir === 'down') {
      const amt = typeof step.amount === 'number' ? step.amount : (typeof step.delta === 'number' ? step.delta : 600);
      targetY = currentY + amt;
    } else if (dir === 'up') {
      const amt = typeof step.amount === 'number' ? step.amount : (typeof step.delta === 'number' ? step.delta : 600);
      targetY = Math.max(0, currentY - amt);
    }

    try {
      window.scrollTo({ left: dx, top: targetY, behavior: 'smooth' });
    } catch {
      // Fallback without smooth behavior
      window.scrollTo(dx, targetY);
    }

    const wait = typeof step.wait_after_ms === 'number' ? step.wait_after_ms : 350;
    await new Promise(resolve => setTimeout(resolve, wait));
    return true;
  },

  get_element_text: async (step: any) => {
    const element = await findElementWithRetry(step);
    return {
      text: element.textContent?.trim() || ''
    };
  },

  get_element_value: async (step: any) => {
    const element = await findElementWithRetry(step);
    const value = (() => {
      if (element instanceof HTMLInputElement) return element.value;
      if (element instanceof HTMLTextAreaElement) return element.value;
      if (element instanceof HTMLSelectElement) return element.value;
      const attr = element.getAttribute?.('value');
      if (attr != null) return attr;
      return element.textContent?.trim() || '';
    })();

    return { value };
  },

  read_field_value: async (step: any) => {
    const element = await findElementWithRetry(step);
    return {
      selector: resolveSelectorWithRefs(step.selector),
      value: extractElementValue(element),
      element: summarizeElementForDebug(element),
    };
  },

  get_element_count: async (step: any) => {
    const selector = String(step.selector || '').trim();
    if (!selector) throw new Error('Missing selector for get_element_count');

    // If selector is a ref, count is either 0 or 1.
    const refIndex = parseRefIndex(selector);
    if (refIndex !== null) {
      try {
        await findElementWithRetry({ selector });
        return { count: 1 };
      } catch {
        return { count: 0 };
      }
    }

    let count = 0;
    try {
      count = document.querySelectorAll(selector).length;
    } catch (e) {
      throw new Error(`Invalid selector for get_element_count: ${selector}`);
    }
    return { count };
  },

  get_element_attribute: async (step: any) => {
    const attribute = String(step.attribute || '').trim();
    if (!attribute) throw new Error('Missing attribute for get_element_attribute');

    const element = await findElementWithRetry(step);
    const value = element.getAttribute(attribute);
    return { attribute, value };
  },

  select_option_in_dropdown: async (step: any) => {
    const valueRaw = step.value;
    const value = String(valueRaw ?? '').trim();
    if (!value) throw new Error('Missing value for select_option_in_dropdown');

    const element = await findElementWithRetry(step);
    if (!(element instanceof HTMLSelectElement)) {
      throw new Error(`select_option_in_dropdown requires a <select>, got: ${element.tagName}`);
    }

    const options = Array.from(element.options || []);
    const byValue = options.find(o => o.value === value);
    const byText = options.find(o => (o.textContent || '').trim() === value);
    const asIndex = (() => {
      if (!/^\d+$/.test(value)) return null;
      const n = parseInt(value, 10);
      if (!Number.isFinite(n)) return null;
      if (n < 0 || n >= options.length) return null;
      return n;
    })();

    let nextValue: string | null = null;
    if (byValue) nextValue = byValue.value;
    else if (byText) nextValue = byText.value;
    else if (asIndex !== null) nextValue = options[asIndex]?.value ?? null;

    if (nextValue === null) {
      const preview = options
        .slice(0, 20)
        .map(o => `${o.value}:${(o.textContent || '').trim().slice(0, 30)}`)
        .join(', ');
      throw new Error(`Option not found for value="${value}". Available: ${preview}`);
    }

    element.focus();
    element.value = nextValue;
    element.dispatchEvent(new Event('input', { bubbles: true, cancelable: true }));
    element.dispatchEvent(new Event('change', { bubbles: true, cancelable: true }));
    return { success: true, value: nextValue };
  },

  drag_and_drop: async (step: any) => {
    const sourceSel = String(step.source_selector || '').trim();
    const targetSel = String(step.target_selector || '').trim();
    if (!sourceSel) throw new Error('Missing source_selector for drag_and_drop');
    if (!targetSel) throw new Error('Missing target_selector for drag_and_drop');

    const source = await findElementWithRetry({ selector: sourceSel });
    const target = await findElementWithRetry({ selector: targetSel });

    const src = source as HTMLElement;
    const dst = target as HTMLElement;

    try {
      src.scrollIntoView?.({ block: 'center', inline: 'center' } as any);
      dst.scrollIntoView?.({ block: 'center', inline: 'center' } as any);
    } catch {}

    const srcRect = src.getBoundingClientRect();
    const dstRect = dst.getBoundingClientRect();

    const sx = srcRect.left + srcRect.width / 2;
    const sy = srcRect.top + srcRect.height / 2;
    const tx = dstRect.left + dstRect.width / 2;
    const ty = dstRect.top + dstRect.height / 2;

    const dataTransfer = new DataTransfer();

    const dispatchMouse = (el: Element, type: string, clientX: number, clientY: number) => {
      try {
        return el.dispatchEvent(
          new MouseEvent(type, {
            bubbles: true,
            cancelable: true,
            composed: true,
            clientX,
            clientY,
            button: 0,
            buttons: type === 'mouseup' ? 0 : 1,
          })
        );
      } catch {
        return el.dispatchEvent(new Event(type, { bubbles: true, cancelable: true, composed: true }));
      }
    };

    const dispatchDrag = (el: Element, type: string, clientX: number, clientY: number) => {
      try {
        const ev = new DragEvent(type, {
          bubbles: true,
          cancelable: true,
          composed: true,
          clientX,
          clientY,
          dataTransfer,
        } as any);
        return el.dispatchEvent(ev);
      } catch {
        // Fallback without DragEvent (still triggers some handlers).
        const ev = new Event(type, { bubbles: true, cancelable: true, composed: true });
        return el.dispatchEvent(ev);
      }
    };

    dispatchMouse(src, 'mousedown', sx, sy);
    dispatchDrag(src, 'dragstart', sx, sy);
    dispatchDrag(dst, 'dragenter', tx, ty);
    dispatchDrag(dst, 'dragover', tx, ty);
    dispatchDrag(dst, 'drop', tx, ty);
    dispatchDrag(src, 'dragend', tx, ty);
    dispatchMouse(dst, 'mouseup', tx, ty);

    return { success: true };
  },

  assert_selector_state: async (step: any) => {
    const selectorRaw = String(step.selector || '').trim();
    const conditionRaw = String(step.condition || '').trim();
    if (!selectorRaw) throw new Error('Missing selector for assert_selector_state');
    if (!conditionRaw) throw new Error('Missing condition for assert_selector_state');

    const timeoutMs = Number(step.timeout_ms ?? step.timeoutMs ?? 0);
    const timeout = Number.isFinite(timeoutMs) ? Math.max(0, Math.round(timeoutMs)) : 0;
    const start = Date.now();

    const resolveSelector = (sel: string): string => {
      const refIndex = parseRefIndex(sel);
      if (refIndex !== null) {
        const resolved = selectorForRefIndex(refIndex);
        if (!resolved) throw new Error(`UNKNOWN_REF: ${sel} (take a fresh DOM snapshot and retry)`);
        return resolved;
      }
      return sel;
    };

    const isDisabled = (el: Element): boolean => {
      const anyEl = el as any;
      if (typeof anyEl.disabled === 'boolean') return !!anyEl.disabled;
      const aria = el.getAttribute('aria-disabled');
      if (aria && aria.toLowerCase() === 'true') return true;
      return el.hasAttribute('disabled');
    };

    const isChecked = (el: Element): boolean => {
      if (el instanceof HTMLInputElement && (el.type === 'checkbox' || el.type === 'radio')) {
        return !!el.checked;
      }
      const aria = el.getAttribute('aria-checked');
      return aria ? aria.toLowerCase() === 'true' : false;
    };

    const want = conditionRaw.toLowerCase();

    while (true) {
      const selector = resolveSelector(selectorRaw);
      const element = (document.querySelector(selector) || querySelectorDeep(selector)) as Element | null;

      const ok = (() => {
        if (want === 'exists') return !!element;
        if (want === 'not_exists' || want === 'missing') return !element;
        if (want === 'visible') return !!element && isElementVisible(element);
        if (want === 'hidden') return !element || !isElementVisible(element);
        if (want === 'enabled') return !!element && !isDisabled(element);
        if (want === 'disabled') return !!element && isDisabled(element);
        if (want === 'checked') return !!element && isChecked(element);
        if (want === 'unchecked') return !!element && !isChecked(element);
        return false;
      })();

      if (ok) {
        return { success: true, selector: selectorRaw, condition: conditionRaw };
      }

      if (!timeout || Date.now() - start >= timeout) {
        throw new Error(`ASSERT_SELECTOR_STATE_FAILED: selector="${selectorRaw}" condition="${conditionRaw}"`);
      }
      await new Promise(r => setTimeout(r, 100));
    }
  },

  assert_text_in_element: async (step: any) => {
    const selectorRaw = String(step.selector || '').trim();
    const expectedRaw = String(step.text || '').trim();
    if (!selectorRaw) throw new Error('Missing selector for assert_text_in_element');
    if (!expectedRaw) throw new Error('Missing text for assert_text_in_element');

    const matchType = String(step.match_type || 'contains').trim().toLowerCase();
    const timeoutMs = Number(step.timeout_ms ?? step.timeoutMs ?? 0);
    const timeout = Number.isFinite(timeoutMs) ? Math.max(0, Math.round(timeoutMs)) : 0;
    const start = Date.now();

    const resolveSelector = (sel: string): string => {
      const refIndex = parseRefIndex(sel);
      if (refIndex !== null) {
        const resolved = selectorForRefIndex(refIndex);
        if (!resolved) throw new Error(`UNKNOWN_REF: ${sel} (take a fresh DOM snapshot and retry)`);
        return resolved;
      }
      return sel;
    };

    while (true) {
      const selector = resolveSelector(selectorRaw);
      const el = (document.querySelector(selector) || querySelectorDeep(selector)) as Element | null;

      const actual = (() => {
        if (!el) return '';
        if (el instanceof HTMLInputElement) return el.value ?? '';
        if (el instanceof HTMLTextAreaElement) return el.value ?? '';
        return (el.textContent || '').trim();
      })();

      const ok = (() => {
        if (matchType === 'exact') return actual === expectedRaw;
        if (matchType === 'regex') {
          try {
            const re = new RegExp(expectedRaw);
            return re.test(actual);
          } catch {
            return false;
          }
        }
        // contains (default)
        return actual.includes(expectedRaw);
      })();

      if (ok) {
        return { success: true, selector: selectorRaw, match_type: matchType };
      }

      if (!timeout || Date.now() - start >= timeout) {
        throw new Error(
          `ASSERT_TEXT_FAILED: selector="${selectorRaw}" match_type="${matchType}" expected="${expectedRaw}" actual="${actual.slice(0, 80)}"`
        );
      }
      await new Promise(r => setTimeout(r, 100));
    }
  },

  assert_url_matches: async (step: any) => {
    const pattern = String(step.url_pattern || '').trim();
    if (!pattern) throw new Error('Missing url_pattern for assert_url_matches');

    const matchType = String(step.match_type || 'contains').trim().toLowerCase();
    const url = window.location.href;

    const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const globToRe = (p: string) => {
      const escaped = escapeRe(p).replace(/\\\*\\\*/g, '.*').replace(/\\\*/g, '.*');
      return new RegExp(`^${escaped}$`);
    };

    const ok = (() => {
      if (matchType === 'exact') return url === pattern;
      if (matchType === 'regex') {
        try {
          return new RegExp(pattern).test(url);
        } catch {
          return false;
        }
      }
      if (matchType === 'glob') {
        try {
          return globToRe(pattern).test(url);
        } catch {
          return false;
        }
      }
      // contains (default)
      return url.includes(pattern);
    })();

    if (!ok) {
      throw new Error(`ASSERT_URL_FAILED: match_type="${matchType}" pattern="${pattern}" url="${url}"`);
    }

    return { success: true, url, match_type: matchType, url_pattern: pattern };
  },

  inspect_element: async (step: any) => {
    const selector = resolveSelectorWithRefs(step.selector);
    if (!selector) throw new Error('Missing selector for inspect_element');
    const element = await findElementWithRetry({ selector });
    const includeAncestors = step.include_ancestors !== false;
    const includeShadowPath = step.include_shadow_path !== false;
    const ancestors: any[] = [];
    if (includeAncestors) {
      let current: Element | null = element.parentElement;
      for (let i = 0; current && i < 6; i += 1) {
        ancestors.push(summarizeElementForDebug(current));
        current = current.parentElement;
      }
    }
    return {
      selector,
      element: summarizeElementForDebug(element),
      actionable_ancestor: summarizeElementForDebug(findActionableAncestor(element)),
      ancestors,
      shadow_path: includeShadowPath ? buildShadowPath(element) : [],
    };
  },

  inspect_click_surface: async (step: any) => {
    const selector = resolveSelectorWithRefs(step.selector);
    if (!selector) throw new Error('Missing selector for inspect_click_surface');
    const element = await findElementWithRetry({ selector });
    const actionableAncestor = findActionableAncestor(element);
    const anchor = actionableAncestor instanceof HTMLAnchorElement
      ? actionableAncestor
      : element instanceof HTMLAnchorElement
        ? element
        : (actionableAncestor?.closest?.('a[href]') || element.closest?.('a[href]')) as HTMLAnchorElement | null;
    return {
      selector,
      target: summarizeElementForDebug(element),
      actionable_ancestor: summarizeElementForDebug(actionableAncestor),
      click_surface: {
        href: anchor?.href || anchor?.getAttribute('href') || null,
        target: anchor?.target || anchor?.getAttribute('target') || null,
        role: actionableAncestor?.getAttribute?.('role') || element.getAttribute('role') || null,
        shadow_path: buildShadowPath(element),
        listeners: summarizeListenerSurface(actionableAncestor || element),
      },
    };
  },

  capture_ui_bundle: async (step: any) => {
    const bundle = await captureUiBundleInternal(step);
    if (step?.include_screenshot === true) {
      try {
        const shot = await actionHandlers.take_screenshot({
          type: 'take_screenshot',
          annotate: step.annotate === true,
          annotate_max_labels: step.annotate_max_labels,
          annotate_max_elements: step.annotate_max_elements,
          format: step.format,
          quality: step.quality,
        });
        bundle.screenshot = shot;
      } catch (error: any) {
        bundle.screenshot_error = error?.message || String(error);
      }
    }
    return bundle;
  },

  verify_ui_change: async (step: any) => {
    const timeoutMs = Number(step.timeout_ms ?? step.timeoutMs ?? 3000);
    const timeout = Number.isFinite(timeoutMs) ? Math.max(0, Math.round(timeoutMs)) : 3000;
    const expectation = { ...step };
    delete expectation.type;
    const result = await waitForUiExpectation(expectation, timeout);
    if (!result.success) {
      throw new Error(`UI verification failed: ${JSON.stringify(result.checks)}`);
    }
    return result;
  },

  eval_isolated_world: async (step: any) => {
    if (!step?.script) throw new Error('Missing script for eval_isolated_world');
    const response: any = await new Promise((resolve, reject) => {
      chrome.runtime.sendMessage(
        {
          cmd: 'eval_with_scripting',
          world: 'isolated',
          script: String(step.script),
          args: Array.isArray(step.args) ? step.args : [],
          params:
            step?.params && typeof step.params === 'object' && !Array.isArray(step.params)
              ? step.params
              : {},
          return_value: step.return_value !== false,
          timeout_ms: step.timeout_ms ?? step.timeoutMs,
        },
        (resp) => {
          const err = chrome.runtime.lastError;
          if (err) reject(new Error(err.message));
          else resolve(resp);
        }
      );
    });
    if (!response?.success) {
      throw new Error(response?.error || 'Scripting eval failed');
    }
    return {
      success: true,
      world: 'isolated',
      execution_backend: response.execution_backend || 'chrome_scripting_isolated_world',
      result: response.result,
    };
  },

  eval_main_world: async (step: any) => {
    if (!step?.script) throw new Error('Missing script for eval_main_world');
    const pageBridgeResult = await tryEvalViaPageBridge(step, 'page_bridge_main_world');
    if (pageBridgeResult) {
      return pageBridgeResult;
    }
    if (!shouldUseCdpEvalForStep(step)) {
      const response: any = await new Promise((resolve, reject) => {
        chrome.runtime.sendMessage(
          {
            cmd: 'eval_with_scripting',
            world: 'main',
            script: String(step.script),
            args: Array.isArray(step.args) ? step.args : [],
            params:
              step?.params && typeof step.params === 'object' && !Array.isArray(step.params)
                ? step.params
                : {},
            return_value: step.return_value !== false,
            timeout_ms: step.timeout_ms ?? step.timeoutMs,
          },
          (resp) => {
            const err = chrome.runtime.lastError;
            if (err) reject(new Error(err.message));
            else resolve(resp);
          }
        );
      });
      if (!response?.success) {
        throw new Error(response?.error || 'Scripting eval failed');
      }
      return {
        success: true,
        world: 'main',
        execution_backend: response.execution_backend || 'chrome_scripting_main_world',
        result: response.result,
      };
    }
    const response: any = await new Promise((resolve, reject) => {
      chrome.runtime.sendMessage(
        {
          cmd: 'eval_with_cdp',
          world: 'main',
          script: String(step.script),
          args: Array.isArray(step.args) ? step.args : [],
          params:
            step?.params && typeof step.params === 'object' && !Array.isArray(step.params)
              ? step.params
              : {},
          return_value: step.return_value !== false,
          timeout_ms: step.timeout_ms ?? step.timeoutMs,
        },
        (resp) => {
          const err = chrome.runtime.lastError;
          if (err) reject(new Error(err.message));
          else resolve(resp);
        }
      );
    });
    if (!response?.success) {
      throw new Error(response?.error || 'CDP eval failed');
    }
    return {
      success: true,
      world: 'main',
      execution_backend: response.execution_backend || 'cdp_runtime_evaluate',
      result: response.result,
    };
  },

  semantic_action: async (step: any) => {
    const action = String(step.action || '').trim().toLowerCase();
    if (!action) throw new Error('Missing action for semantic_action');

    let innerStep: any = step.step && typeof step.step === 'object' ? { ...step.step } : null;
    if (!innerStep) {
      if (action === 'click') {
        innerStep = { type: 'click_element', selector: step.selector };
      } else if (action === 'type') {
        innerStep = {
          type: 'fill_input_field',
          selector: step.selector,
          value: String(step.value ?? ''),
          clear_first: step.clear_first !== false,
        };
      } else if (action === 'press_key') {
        innerStep = { type: 'press_special_key', key: String(step.key || 'Enter'), selector: step.selector };
      } else if (action === 'hover') {
        innerStep = { type: 'hover_element', selector: step.selector };
      } else {
        throw new Error(`Unsupported semantic_action action: ${action}`);
      }
    }

    if (!innerStep.type) {
      throw new Error('semantic_action step is missing type');
    }

    const handler = (enhancedActionHandlers as any)[innerStep.type] || (actionHandlers as any)[innerStep.type];
    if (typeof handler !== 'function') {
      throw new Error(`semantic_action could not resolve handler for ${innerStep.type}`);
    }

    const actionResult = await handler(innerStep);
    const postconditionRequired = step.postcondition_required !== false;
    if (!step.postcondition) {
      if (postconditionRequired) {
        throw new Error('semantic_action requires postcondition unless postcondition_required is false');
      }
      return { success: true, action_result: actionResult, postcondition_verified: false };
    }

    const timeoutMs = Number(step.timeout_ms ?? step.timeoutMs ?? step.postcondition?.timeout_ms ?? 3000);
    const verification = await waitForUiExpectation(step.postcondition, Math.max(0, timeoutMs));
    if (!verification.success) {
      throw new Error(`semantic_action postcondition failed: ${JSON.stringify(verification.checks)}`);
    }

    return {
      success: true,
      action_result: actionResult,
      postcondition_verified: true,
      postcondition_result: verification,
    };
  },

  extract_structured_data: async (step: any) => {
    const startedAt = Date.now();
    const { item_selector, fields, extraction_type } = step;

    if (!item_selector || !fields) {
      throw new Error('Missing item_selector or fields for extraction');
    }
    const limitRaw = (step as any).limit;
    const limit = (() => {
      const n =
        typeof limitRaw === 'number'
          ? limitRaw
          : typeof limitRaw === 'string'
            ? parseInt(limitRaw, 10)
            : NaN;
      if (!Number.isFinite(n) || n <= 0) return null;
      return Math.max(1, Math.min(50, Math.floor(n)));
    })();
    const safeText = (root: Element): string => {
      // Avoid leaking script/style/noscript payloads into extracted text (generic hygiene).
      const clone = root.cloneNode(true) as Element;
      clone.querySelectorAll('script,style,noscript').forEach(n => n.remove());
      return (clone.textContent || '').trim();
    };

    const applyPostProcessing = (value: string, ops: any): string => {
      if (!Array.isArray(ops) || ops.length === 0) return value;
      let out = value;
      for (const opRaw of ops) {
        const op = String(opRaw || '').trim();
        if (!op) continue;

        if (op === 'trim') {
          out = out.trim();
          continue;
        }
        if (op === 'collapse_whitespace') {
          out = out.replace(/\s+/g, ' ').trim();
          continue;
        }

        if (op.startsWith('regex_group:')) {
          const rest = op.slice('regex_group:'.length);
          const lastColon = rest.lastIndexOf(':');
          const pattern = lastColon > 0 ? rest.slice(0, lastColon) : rest;
          const group = lastColon > 0 ? parseInt(rest.slice(lastColon + 1), 10) : 1;
          try {
            const re = new RegExp(pattern, 'i');
            const m = out.match(re);
            if (m) out = (m[group] ?? m[0] ?? out) as string;
          } catch {
            // ignore invalid regex
          }
          continue;
        }

        if (op.startsWith('regex:')) {
          const pattern = op.slice('regex:'.length);
          try {
            const re = new RegExp(pattern, 'i');
            const m = out.match(re);
            if (m) out = (m[1] ?? m[0] ?? out) as string;
          } catch {
            // ignore invalid regex
          }
          continue;
        }
      }
      return out;
    };
    
    const scopeSelectorRaw = (step as any).scope_selector ?? (step as any).scopeSelector;
    const scopeSelector = typeof scopeSelectorRaw === 'string' ? scopeSelectorRaw.trim() : '';
    const scope = scopeSelector ? (document.querySelector(scopeSelector) || document) : document;

    const includeShadowRaw = (step as any).include_shadow ?? (step as any).includeShadow;
    const includeShadow =
      includeShadowRaw === true ||
      includeShadowRaw === 1 ||
      (typeof includeShadowRaw === 'string' && includeShadowRaw.toLowerCase() === 'true');

    const items = (() => {
      if (!includeShadow) {
        const qsa = (scope as any).querySelectorAll as ((sel: string) => NodeListOf<Element>) | undefined;
        return Array.from((typeof qsa === 'function' ? qsa.call(scope, item_selector) : []) || []);
      }

      const acc: Element[] = [];
      const seen = new Set<Element>();
      const pushMatches = (root: ParentNode) => {
        const qsa = (root as any).querySelectorAll as ((sel: string) => NodeListOf<Element>) | undefined;
        if (typeof qsa !== 'function') return;
        qsa.call(root, item_selector).forEach(el => {
          if (seen.has(el)) return;
          seen.add(el);
          acc.push(el);
        });
      };

      // Always include light DOM matches from the selected scope.
      pushMatches(scope as ParentNode);

      // Prefer the main-world shadow-root registry if available (fast path).
      const registeredRoots = (window as any).__rznShadowRoots;
      if (Array.isArray(registeredRoots) && registeredRoots.length > 0) {
        for (const sr of registeredRoots) {
          if (sr && typeof (sr as any).querySelectorAll === 'function') {
            pushMatches(sr as ParentNode);
          }
        }
      } else {
        // Fallback: discover shadow roots by scanning the DOM.
        walkRootAndShadowRoots(scope as ParentNode, root => {
          if (root === scope) return; // already pushed
          pushMatches(root);
        });
      }
      return acc;
    })();
    const results: any[] = [];
    
    // Log to console for debugging
    console.log(`[RZN] Extract structured data:`);
    console.log(`[RZN] - Selector: ${item_selector}`);
    if (scopeSelector) console.log(`[RZN] - Scope: ${scopeSelector}`);
    if (includeShadow) console.log(`[RZN] - Shadow DOM: enabled`);
    console.log(`[RZN] - Found ${items.length} items`);
    if (limit !== null) console.log(`[RZN] - Limit: ${limit}`);
    console.log(`[RZN] - Fields:`, fields);
    
    // Also log first few items for debugging
    if (items.length > 0) {
      console.log(`[RZN] First item HTML:`, items[0].outerHTML.substring(0, 200) + '...');
    }
    
    for (const item of items) {
      const result: any = {};
      for (const field of fields) {
        // If selector is "*", use the item element itself
        const element = field.selector === '*' ? item : item.querySelector(field.selector);
        if (element) {
          if (field.attribute) {
            const raw = element.getAttribute(field.attribute) || '';
            result[field.name] = applyPostProcessing(raw, field.post_processing);
          } else {
            const raw = safeText(element);
            result[field.name] = applyPostProcessing(raw, field.post_processing);
          }
        }
      }
      if (Object.keys(result).length > 0) {
        console.log(`[RZN] Extracted item:`, result);
        results.push(result);
        if (limit !== null && results.length >= limit) break;
      }
    }
    
    console.log(`[RZN] Total extracted results: ${results.length}`);
    return actionSuccess({
      action: 'extract_structured_data',
      result: results,
      duration_ms: Date.now() - startedAt,
      legacy: {
        data: results,
        item_count: results.length,
        extraction_type,
      },
    });
  },

  // Test/bridge helper: run a validated extraction plan without arbitrary JS execution.
  // This mirrors the runtime command handler (message.cmd === 'execute_extraction_plan').
  execute_extraction_plan: async (step: any) => {
    const { ExtractionPlanV1Schema } = await import('./types/extractionPlan');
    const plan = ExtractionPlanV1Schema.parse(step.plan ?? step);

    const resolveScope = (): ParentNode => {
      if (!plan.scope) return document;
      if (plan.scope.css) {
        const el = querySelectorDeep(plan.scope.css);
        if (!el) throw new Error(`Scope not found for css: ${plan.scope.css}`);
        return el;
      }
      if (plan.scope.xpath) {
        const xr = document.evaluate(
          plan.scope.xpath,
          document,
          null,
          XPathResult.FIRST_ORDERED_NODE_TYPE,
          null
        );
        const node = xr.singleNodeValue as Element | null;
        if (!node) throw new Error(`Scope not found for xpath: ${plan.scope.xpath}`);
        return node;
      }
      return document;
    };

    const scopeNode = resolveScope();
    const limit = plan.limit ?? 50;

    const safeText = (root: Element): string => {
      const clone = root.cloneNode(true) as Element;
      clone.querySelectorAll('script,style,noscript').forEach(n => n.remove());
      return (clone.textContent || '').trim();
    };
    const extractValue = (el: Element, attribute?: string): string | null => {
      if (!attribute) return safeText(el);
      return el.getAttribute(attribute);
    };

    if (plan.mode === 'single') {
      const out: Record<string, any> = {};
      for (const field of plan.fields) {
        const el = querySelectorDeep(field.selector, scopeNode);
        out[field.name] = el ? extractValue(el, field.attribute) : null;
      }
      return out;
    }

    const itemSel = plan.item_selector!;
    const nodes = querySelectorAllDeep(itemSel, scopeNode);
    const items: any[] = [];
    for (const item of nodes.slice(0, limit)) {
      const row: Record<string, any> = {};
      for (const field of plan.fields) {
        const selector = (field.selector || '').trim();
        const el = selector === ':scope' ? item : querySelectorDeep(selector, item);
        row[field.name] = el ? extractValue(el, field.attribute) : null;
      }
      if (Object.values(row).some(v => v !== null && v !== '')) {
        items.push(row);
      }
    }
    return items;
  },

  // Popup handling
  detect_popups: async (step: any) => {
    const popupSelectors = [
      // Cookie consent
      '[class*="cookie-consent"]', '[id*="cookie-consent"]',
      '[class*="cookie-banner"]', '[id*="cookie-banner"]',
      '[class*="gdpr"]', '[id*="gdpr"]',
      // Modal dialogs
      '[class*="modal"][style*="display: block"]',
      '[class*="popup"][style*="display: block"]',
      '[class*="overlay"][style*="display: block"]',
      // Newsletter popups
      '[class*="newsletter-popup"]', '[class*="subscribe-popup"]',
      // Custom selectors from step
      ...(step.custom_selectors || [])
    ];
    
    const detected = [];
    for (const selector of popupSelectors) {
      const elements = document.querySelectorAll(selector);
      elements.forEach(el => {
        if (el && el.offsetParent !== null) { // Check if visible
          detected.push({
            selector,
            text: el.textContent?.substring(0, 100) || '',
            type: selector.includes('cookie') ? 'cookie' : 
                  selector.includes('newsletter') ? 'newsletter' : 'generic'
          });
        }
      });
    }
    
    return { popups_detected: detected.length > 0, popups: detected };
  },

  dismiss_popups: async (step: any) => {
    const maxDismissals = Math.max(0, Math.min(20, Number(step.max_dismissals ?? 8) || 8));
    const maxRuntimeMs = Math.max(250, Math.min(10_000, Number(step.timeout_ms ?? step.timeoutMs ?? 2_500) || 2_500));
    const deadline = Date.now() + maxRuntimeMs;
    const dismissSelectors = [
      // Common dismiss buttons
      'button[class*="close"]', 'button[class*="dismiss"]',
      'button[class*="accept"]', 'button[class*="agree"]',
      'button[class*="ok"]', 'button[class*="got it"]',
      '[aria-label*="close"]', '[aria-label*="dismiss"]',
      // Other close elements
      'a[class*="close"]', 'span[class*="close"]',
      // Custom selectors
      ...(step.dismiss_selectors || [])
    ];
    
    let dismissed = 0;
    let capped = false;

    const shouldStop = () => {
      const stop = dismissed >= maxDismissals || Date.now() >= deadline;
      if (stop) capped = true;
      return stop;
    };

    const tryClickDismiss = async (el: Element) => {
      if (shouldStop()) return false;
      if (el && (el as HTMLElement).offsetParent !== null) {
        (el as HTMLElement).click();
        dismissed++;
        await new Promise(resolve => setTimeout(resolve, 100));
      }
      return !shouldStop();
    };
    
    // Try selectors first
    for (const selector of dismissSelectors) {
      if (shouldStop()) break;
      try {
        const elements = document.querySelectorAll(selector);
        for (const el of elements) {
          const shouldContinue = await tryClickDismiss(el);
          if (!shouldContinue) break;
        }
      } catch (e) {
        // Continue with next selector
      }
    }
    
    // Also check for buttons with X or × text
    const allButtons = document.querySelectorAll('button');
    for (const button of allButtons) {
      if (shouldStop()) break;
      const text = button.textContent?.trim();
      if (text === 'X' || text === '×' || text === 'x') {
        await tryClickDismiss(button);
      }
    }

    // Text-based dismissal scoped to modal/dialog containers. Restricted to
    // unambiguously-dismissive labels — never "Continue / Accept / Allow / OK"
    // which can launch OAuth ("Continue with Google") or grant permissions
    // inside auth/share/consent dialogs.
    const dismissTextPatterns = /^(got\s*it|dismiss|close|maybe\s+later|no\s+thanks|no,?\s*thanks|not\s+now|skip|i\s+understand|cancel)\s*\.?\s*!?$/i;
    const modalContainers = document.querySelectorAll('[role="dialog"], [role="alertdialog"], [aria-modal="true"]');
    for (const modal of modalContainers) {
      if (shouldStop()) break;
      const buttons = modal.querySelectorAll('button, [role="button"]');
      for (const button of buttons) {
        if (shouldStop()) break;
        const text = button.textContent?.trim() || '';
        if (text && dismissTextPatterns.test(text)) {
          await tryClickDismiss(button);
        }
      }
    }

    return { dismissed_count: dismissed, capped, max_dismissals: maxDismissals, max_runtime_ms: maxRuntimeMs };
  },

  wait_for_no_popups: async (step: any) => {
    const timeoutMs = step.timeout_ms || 5000;
    const checkInterval = 500;
    const startTime = Date.now();
    
    while (Date.now() - startTime < timeoutMs) {
      const popupCheck = await actionHandlers.detect_popups(step);
      if (!popupCheck.popups_detected) {
        return true;
      }
      await new Promise(resolve => setTimeout(resolve, checkInterval));
    }
    
    throw new Error('Timeout waiting for popups to clear');
  },

  // Authentication handling
  wait_for_auth: async (step: any) => {
    const timeoutMs = step.timeout_ms || 300000; // 5 minutes default
    const checkInterval = 1000;
    const startTime = Date.now();
    
    // Check for common auth success indicators
    const authIndicators = [
      ...(step.success_selectors || []),
      '[class*="dashboard"]', '[class*="account"]',
      '[class*="profile"]', '[class*="welcome"]',
      '[class*="logout"]', '[class*="sign-out"]'
    ];
    
    while (Date.now() - startTime < timeoutMs) {
      // Check URL change
      if (step.success_url_pattern && new RegExp(step.success_url_pattern).test(window.location.href)) {
        return { authenticated: true, method: 'url_change' };
      }
      
      // Check for auth indicators
      for (const selector of authIndicators) {
        if (document.querySelector(selector)) {
          return { authenticated: true, method: 'element_detected', selector };
        }
      }
      
      await new Promise(resolve => setTimeout(resolve, checkInterval));
    }
    
    throw new Error('Authentication timeout');
  },

  wait_for_totp: async (step: any) => {
    const timeoutMs = step.timeout_ms || 120000; // 2 minutes default
    const checkInterval = 500;
    const startTime = Date.now();
    
    // Common TOTP input selectors
    const totpSelectors = [
      'input[type="text"][name*="code"]',
      'input[type="text"][name*="totp"]',
      'input[type="text"][name*="2fa"]',
      'input[type="text"][placeholder*="code"]',
      'input[type="number"][maxlength="6"]',
      ...(step.totp_selectors || [])
    ];
    
    while (Date.now() - startTime < timeoutMs) {
      for (const selector of totpSelectors) {
        const input = document.querySelector(selector) as HTMLInputElement;
        if (input && input.value && input.value.length >= 6) {
          // Wait a bit for submission
          await new Promise(resolve => setTimeout(resolve, 2000));
          return { totp_entered: true };
        }
      }
      
      await new Promise(resolve => setTimeout(resolve, checkInterval));
    }
    
    throw new Error('TOTP entry timeout');
  },

  wait_for_verification: async (step: any) => {
    const timeoutMs = step.timeout_ms || 300000; // 5 minutes default
    const checkInterval = 1000;
    const startTime = Date.now();
    
    while (Date.now() - startTime < timeoutMs) {
      // Check for verification success
      if (step.success_url_pattern && new RegExp(step.success_url_pattern).test(window.location.href)) {
        return { verified: true };
      }
      
      // Check for success message
      const successSelectors = step.success_selectors || [
        '[class*="success"]', '[class*="verified"]',
        '[class*="complete"]', '[class*="done"]'
      ];
      
      for (const selector of successSelectors) {
        const el = document.querySelector(selector);
        if (el && el.textContent && 
            (el.textContent.toLowerCase().includes('verified') ||
             el.textContent.toLowerCase().includes('success') ||
             el.textContent.toLowerCase().includes('complete'))) {
          return { verified: true, message: el.textContent };
        }
      }
      
      await new Promise(resolve => setTimeout(resolve, checkInterval));
    }
    
    throw new Error('Verification timeout');
  },

  // CAPTCHA handling
  handle_captcha: async (step: any) => {
    // Detect CAPTCHA type
    const captchaTypes = {
      recaptcha_v2: 'iframe[src*="recaptcha"]',
      recaptcha_v3: '[class*="g-recaptcha"]',
      hcaptcha: 'iframe[src*="hcaptcha.com"]',
      funcaptcha: '#funcaptcha',
      geetest: '[class*="geetest"]'
    };
    
    let detectedType = null;
    for (const [type, selector] of Object.entries(captchaTypes)) {
      if (document.querySelector(selector)) {
        detectedType = type;
        break;
      }
    }
    
    if (!detectedType) {
      return { captcha_detected: false };
    }
    
    // If solver is configured, we would call it here
    // For now, just wait for manual solving
    return {
      captcha_detected: true,
      captcha_type: detectedType,
      requires_manual_solve: true
    };
  },

  configure_captcha_solver: async (step: any) => {
    // Store solver configuration for future use
    // This would integrate with 2captcha, anti-captcha, etc.
    return {
      configured: true,
      solver: step.solver || 'manual'
    };
  },

  // User intervention
  request_user_intervention: async (step: any) => {
    const message = step.message || step.reason || 'Manual intervention required';
    const instructions = step.instructions || '';
    const timeoutMs = step.timeout_ms || 300000; // 5 minutes default
    const rawApprovalMode = String(step.approval_mode || step.approval_policy || 'ask_user')
      .trim()
      .toLowerCase()
      .replace(/[\s-]+/g, '_');
    const approvalMode =
      rawApprovalMode === 'ask' || rawApprovalMode === 'prompt'
        ? 'ask_user'
        : rawApprovalMode === 'notification' || rawApprovalMode === 'system_notify'
          ? 'notify'
          : rawApprovalMode === 'auto' || rawApprovalMode === 'continue' || rawApprovalMode === 'yolo'
            ? 'auto_continue'
            : rawApprovalMode === 'none' || rawApprovalMode === 'stop' || rawApprovalMode === 'do_nothing'
              ? 'noop'
              : rawApprovalMode;
    const continueOnTimeout =
      typeof step.continue_on_timeout === 'boolean' ? step.continue_on_timeout : true;
    const notificationTitle = step.notification_title || 'RZN Automation';
    const notificationMessage =
      step.notification_message ||
      [message, instructions].filter(Boolean).join('\n');
    
    console.log(`USER INTERVENTION REQUIRED: ${message}`);

    if (approvalMode === 'auto_continue') {
      return {
        intervention_completed: true,
        continued_by: 'policy',
        approval_mode: 'auto_continue'
      };
    }

    if (approvalMode === 'notify') {
      let notificationSent = false;
      let notificationError: string | undefined;

      try {
        const response = await chrome.runtime.sendMessage({
          cmd: 'rzn_system_notification',
          title: notificationTitle,
          message: notificationMessage || message
        });
        notificationSent = !!(response?.success || response?.ok);
        if (!notificationSent) {
          notificationError = response?.error || 'Notification request failed';
        }
      } catch (error: any) {
        notificationError = error?.message || String(error);
      }

      return {
        intervention_completed: false,
        approval_mode: 'notify',
        notification_sent: notificationSent,
        notification_error: notificationError,
        stop_workflow: true,
        stop_reason: 'notification_only'
      };
    }

    if (approvalMode === 'noop') {
      return {
        intervention_completed: false,
        approval_mode: 'noop',
        stop_workflow: true,
        stop_reason: 'noop'
      };
    }
    
    const banner = document.createElement('div');
    banner.style.cssText = `
      position: fixed;
      top: 0;
      left: 0;
      right: 0;
      background: #ff9800;
      color: white;
      padding: 10px;
      z-index: 999999;
      font-family: Arial, sans-serif;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      box-shadow: 0 2px 12px rgba(0, 0, 0, 0.25);
    `;

    const textWrap = document.createElement('div');
    textWrap.style.cssText = 'display: flex; flex-direction: column; gap: 4px; min-width: 0;';

    const title = document.createElement('div');
    title.style.cssText = 'font-weight: 600;';
    title.textContent = `RZN Automation: ${message}`;
    textWrap.appendChild(title);

    if (instructions) {
      const detail = document.createElement('div');
      detail.style.cssText = 'font-size: 13px; opacity: 0.95;';
      detail.textContent = instructions;
      textWrap.appendChild(detail);
    }

    const actionWrap = document.createElement('div');
    actionWrap.style.cssText = 'display: flex; align-items: center; gap: 8px; flex-shrink: 0;';

    const timer = document.createElement('div');
    timer.style.cssText = 'font-size: 12px; opacity: 0.95;';

    const continueButton = document.createElement('button');
    continueButton.type = 'button';
    continueButton.textContent = 'Continue';
    continueButton.style.cssText = `
      background: white;
      color: #ff9800;
      border: 0;
      border-radius: 6px;
      padding: 8px 12px;
      font-weight: 600;
      cursor: pointer;
    `;

    actionWrap.appendChild(timer);
    actionWrap.appendChild(continueButton);
    banner.appendChild(textWrap);
    banner.appendChild(actionWrap);
    (document.body || document.documentElement).appendChild(banner);

    let timeoutId: ReturnType<typeof setTimeout> | null = null;
    let intervalId: ReturnType<typeof setInterval> | null = null;
    let settled = false;

    const cleanup = () => {
      if (timeoutId) clearTimeout(timeoutId);
      if (intervalId) clearInterval(intervalId);
      continueButton.removeEventListener('click', onContinue);
      banner.remove();
    };

    const deadline = Date.now() + timeoutMs;
    const updateTimer = () => {
      const remainingMs = Math.max(0, deadline - Date.now());
      const seconds = Math.ceil(remainingMs / 1000);
      timer.textContent = continueOnTimeout
        ? `Auto-continue in ${seconds}s`
        : `Stops in ${seconds}s`;
    };

    const finish = (result: any) => {
      if (settled) return;
      settled = true;
      cleanup();
      resolvePromise(result);
    };

    const onContinue = () =>
      finish({
        intervention_completed: true,
        continued_by: 'user',
        approval_mode: 'ask_user'
      });
    continueButton.addEventListener('click', onContinue);

    updateTimer();
    intervalId = setInterval(updateTimer, 1000);

    let resolvePromise: (value: any) => void = () => {};
    const result = await new Promise(resolve => {
      resolvePromise = resolve;
      timeoutId = setTimeout(
        () =>
          finish(
            continueOnTimeout
              ? {
                  intervention_completed: true,
                  continued_by: 'timeout',
                  approval_mode: 'ask_user'
                }
              : {
                  intervention_completed: false,
                  continued_by: 'timeout',
                  approval_mode: 'ask_user',
                  stop_workflow: true,
                  stop_reason: 'timeout_no_continue'
                }
          ),
        timeoutMs
      );
    });

    return result;
  },

  // Scrolling
  infinite_scroll: async (step: any) => {
    const maxScrolls = step.max_scrolls || 10;
    const scrollDelay = step.scroll_delay || 1000;
    const targetSelector = step.target_selector;
    let previousHeight = document.body.scrollHeight;
    let scrollCount = 0;
    let targetCount = 0;
    
    while (scrollCount < maxScrolls) {
      // Scroll to bottom
      window.scrollTo(0, document.body.scrollHeight);
      await new Promise(resolve => setTimeout(resolve, scrollDelay));
      
      const currentHeight = document.body.scrollHeight;
      
      // Check if we have enough target elements
      if (targetSelector) {
        targetCount = document.querySelectorAll(targetSelector).length;
        if (step.target_count && targetCount >= step.target_count) {
          break;
        }
      }
      
      // Check if no new content loaded
      if (currentHeight === previousHeight) {
        // Try scrolling up a bit and down again
        window.scrollTo(0, currentHeight - 500);
        await new Promise(resolve => setTimeout(resolve, 500));
        window.scrollTo(0, currentHeight);
        await new Promise(resolve => setTimeout(resolve, scrollDelay));
        
        // Check again
        if (document.body.scrollHeight === currentHeight) {
          break; // No more content
        }
      }
      
      previousHeight = currentHeight;
      scrollCount++;
    }
    
    return {
      scrolls_performed: scrollCount,
      final_height: document.body.scrollHeight,
      items_found: targetCount
    };
  },

  // Asset extraction
  extract_page_assets: async (step: any) => {
    const assetTypes = step.asset_types || ['images', 'videos'];
    const assets: any = {};
    
    if (assetTypes.includes('images')) {
      const images = Array.from(document.querySelectorAll('img')).map(img => ({
        src: img.src,
        alt: img.alt,
        width: img.naturalWidth,
        height: img.naturalHeight,
        loaded: img.complete
      })).filter(img => img.src && img.loaded);
      
      assets.images = images;
    }
    
    if (assetTypes.includes('videos')) {
      const videos = Array.from(document.querySelectorAll('video')).map(video => ({
        src: video.src || video.querySelector('source')?.src || '',
        poster: video.poster,
        duration: video.duration,
        width: video.videoWidth,
        height: video.videoHeight,
        currentTime: video.currentTime
      })).filter(video => video.src);
      
      assets.videos = videos;
    }
    
    if (assetTypes.includes('links')) {
      const links = Array.from(document.querySelectorAll('a[href]')).map(link => ({
        href: link.href,
        text: link.textContent?.trim() || '',
        target: link.target
      })).filter(link => link.href && !link.href.startsWith('javascript:'));
      
      assets.links = links;
    }
    
    if (assetTypes.includes('scripts')) {
      const scripts = Array.from(document.querySelectorAll('script[src]')).map(script => ({
        src: script.src,
        async: script.async,
        defer: script.defer,
        type: script.type
      }));
      
      assets.scripts = scripts;
    }
    
    return assets;
  },

  submit_input: async (step: any) => {
    console.log('submit_input step:', JSON.stringify(redactStepForLog(step)));
    const element = await findElementWithRetry(step) as HTMLInputElement | HTMLTextAreaElement;
    
    if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
      const useNativeInput = step.use_native_input === true;
      const beforeUrl = window.location.href;

      await actionHandlers.fill_input_field({
        ...step,
        value: String(step.text ?? ''),
      });
      await new Promise(resolve => setTimeout(resolve, 120));

      const dispatchDomEnter = () => {
        const keydown = new KeyboardEvent('keydown', {
          key: 'Enter',
          code: 'Enter',
          keyCode: 13,
          which: 13,
          bubbles: true,
          cancelable: true,
          view: window
        });
        const accepted = element.dispatchEvent(keydown);
        if (accepted) {
          const keypress = new KeyboardEvent('keypress', {
            key: 'Enter',
            code: 'Enter',
            keyCode: 13,
            which: 13,
            charCode: 13,
            bubbles: true,
            cancelable: true,
            view: window
          });
          element.dispatchEvent(keypress);
        }
        const keyup = new KeyboardEvent('keyup', {
          key: 'Enter',
          code: 'Enter',
          keyCode: 13,
          which: 13,
          bubbles: true,
          cancelable: true,
          view: window
        });
        element.dispatchEvent(keyup);
      };

      const nativeInputReady = useNativeInput ? await nativeInput.ensureAvailable({ force: true }) : false;
      if (useNativeInput && nativeInputReady) {
        console.log('Using native Enter key for submit_input');
        const success = await nativeInput.nativeKey('Return');
        if (!success) {
          console.warn('Native Enter key failed, falling back to DOM method');
          dispatchDomEnter();
        }
      } else {
        dispatchDomEnter();
      }

      // Fallback submit if Enter did not trigger navigation/form handlers.
      const form = element.form || (element.closest('form') as HTMLFormElement | null);
      const allowFallbackSubmit = step.submit_fallback !== false;
      if (allowFallbackSubmit && form && window.location.href === beforeUrl) {
        try {
          if (typeof form.requestSubmit === 'function') form.requestSubmit();
          else if (typeof form.submit === 'function') form.submit();
        } catch {}
      }
      
      return true;
    } else {
      throw new Error(`Element is not an input or textarea: ${element.tagName}`);
    }
  },

  download_images: async (step: any) => {
    const selector = step.selector || 'img';
    const images = document.querySelectorAll(selector);
    const downloadFolder = step.download_folder || 'downloaded_images';
    const limit = step.limit || Infinity;
    
    const imageUrls: string[] = [];
    const results: any[] = [];

    const imageUrl = (img: HTMLImageElement): string => {
      const srcset = String(img.getAttribute('srcset') || '')
        .split(',')
        .map((part) => part.trim())
        .filter(Boolean);
      const srcsetUrl = srcset.length ? srcset[srcset.length - 1].split(/\s+/)[0] : '';
      const raw =
        img.currentSrc ||
        img.src ||
        img.getAttribute('src') ||
        img.getAttribute('data-src') ||
        img.getAttribute('data-lazy-src') ||
        srcsetUrl ||
        '';
      try {
        return raw ? new URL(raw, window.location.href).toString() : '';
      } catch {
        return raw;
      }
    };

    const inferExtension = (url: string): string => {
      try {
        const parsed = new URL(url, window.location.href);
        const last = parsed.pathname.split('/').pop() || '';
        const ext = (last.match(/\.([a-z0-9]{1,8})$/i)?.[1] || '').toLowerCase();
        if (ext && !['php', 'asp', 'aspx', 'cgi'].includes(ext)) return ext;
      } catch {}
      if (/^data:image\/png/i.test(url)) return 'png';
      if (/^data:image\/webp/i.test(url)) return 'webp';
      if (/^data:image\/gif/i.test(url)) return 'gif';
      return 'jpg';
    };
    
    // Collect all image URLs
    const seen = new Set<string>();
    for (const img of Array.from(images)) {
      if (img instanceof HTMLImageElement) {
        const src = imageUrl(img);
        if (src && /^(https?:|data:|blob:)/i.test(src)) {
          // Skip tracking pixels and duplicate URLs.
          if (!src.includes('1x1') && !seen.has(src)) {
            seen.add(src);
            imageUrls.push(src);
          }
        }
      }
    }
    
    console.log(`Found ${imageUrls.length} images to download (limit: ${limit})`);
    
    // Download each image using browser's download API (up to limit)
    const imagesToDownload = Math.min(imageUrls.length, limit);
    for (let i = 0; i < imagesToDownload; i++) {
      const url = imageUrls[i];
      try {
        // Create a unique filename
        const ext = inferExtension(url);
        const filename = `${downloadFolder}/image_${i + 1}.${ext}`;
        
        // Send download request to background script
        const response = await chrome.runtime.sendMessage({
          cmd: 'download_image',
          url: url,
          filename: filename
        });
        
        results.push({
          url: url,
          filename: filename,
          success: response.success,
          download_id: response.downloadId
        });
      } catch (error) {
        console.error(`Failed to download image ${url}:`, error);
        results.push({
          url: url,
          error: error.message
        });
      }
    }
    
    return {
      total_images: imageUrls.length,
      downloaded: results.filter(r => r.success).length,
      failed: results.filter(r => !r.success).length,
      results: results
    };
  },


  // Human behavior simulation
  simulate_human_behavior: async (step: any) => {
    const behaviors = step.behaviors || ['mouse_movement', 'random_delays'];
    
    if (behaviors.includes('mouse_movement')) {
      // Simulate mouse movement by dispatching mousemove events
      const moveCount = 5;
      for (let i = 0; i < moveCount; i++) {
        const x = Math.random() * window.innerWidth;
        const y = Math.random() * window.innerHeight;
        
        const event = new MouseEvent('mousemove', {
          clientX: x,
          clientY: y,
          bubbles: true
        });
        document.dispatchEvent(event);
        
        await new Promise(resolve => setTimeout(resolve, 100 + Math.random() * 200));
      }
    }
    
    if (behaviors.includes('random_delays')) {
      const delay = 500 + Math.random() * 1500;
      await new Promise(resolve => setTimeout(resolve, delay));
    }
    
    if (behaviors.includes('scroll_pause')) {
      // Scroll a bit and pause like a human reading
      const scrolls = 3;
      for (let i = 0; i < scrolls; i++) {
        const scrollAmount = 100 + Math.random() * 300;
        window.scrollBy(0, scrollAmount);
        await new Promise(resolve => setTimeout(resolve, 1000 + Math.random() * 2000));
      }
    }
    
    return { behaviors_simulated: behaviors };
  },

  same_origin_request: async (step: any) => {
    const rawPath = step.path || step.url || step.endpoint;
    if (!rawPath || typeof rawPath !== 'string') {
      throw new Error('Missing path for same_origin_request');
    }

    const path = rawPath.trim();
    if (!path.startsWith('/')) {
      throw new Error('same_origin_request only supports same-origin paths starting with "/"');
    }

    const method = String(step.method || 'GET').trim().toUpperCase();
    const responseFormat = String(step.response_format || step.responseFormat || 'json').toLowerCase();
    const timeoutMs = step.timeout_ms || step.timeoutMs || 20000;
    const maxBytesRaw = step.max_bytes ?? step.maxBytes ?? 500000;
    const maxBytes = Number(maxBytesRaw);

    const url = new URL(path, window.location.origin);
    const query = step.query || step.query_params || step.queryParams;
    if (query && typeof query === 'object') {
      for (const [k, v] of Object.entries(query)) {
        if (v === undefined || v === null) continue;
        url.searchParams.set(String(k), String(v));
      }
    }

    const headers: Record<string, string> = {};
    const hdrs = step.headers;
    if (hdrs && typeof hdrs === 'object') {
      for (const [k, v] of Object.entries(hdrs)) {
        if (v === undefined || v === null) continue;
        headers[String(k)] = String(v);
      }
    }

    if (!headers['Accept']) {
      headers['Accept'] =
        responseFormat === 'text'
          ? 'text/plain,*/*'
          : 'application/json, text/plain, */*';
    }

    let body: any = undefined;
    if (step.body !== undefined && step.body !== null && method !== 'GET' && method !== 'HEAD') {
      if (typeof step.body === 'string') {
        body = step.body;
      } else {
        body = JSON.stringify(step.body);
        if (!headers['Content-Type']) headers['Content-Type'] = 'application/json';
      }
    }

    const controller = typeof AbortController !== 'undefined' ? new AbortController() : null;
    const timer = controller ? setTimeout(() => controller.abort(), timeoutMs) : null;
    let cancellationArmed = true;
    if (controller) {
      void contentCancellationPromise(step).catch(() => {
        if (cancellationArmed) {
          controller.abort();
        }
      });
    }

    try {
      throwIfContentRequestCancelled(step, 'before fetch_url');
      const resp = await fetch(url.toString(), {
        method,
        headers,
        body,
        credentials: 'same-origin',
        cache: 'no-store',
        signal: controller ? controller.signal : undefined
      });
      throwIfContentRequestCancelled(step, 'after fetch_url response');

      let text = await resp.text();
      if (Number.isFinite(maxBytes) && maxBytes > 0 && text.length > maxBytes) {
        text = text.slice(0, maxBytes);
      }

      if (!resp.ok) {
        throw new Error(`HTTP ${resp.status} ${resp.statusText}: ${text.slice(0, 300)}`);
      }

      let data: any = text;
      if (responseFormat === 'json') {
        try {
          data = text ? JSON.parse(text) : null;
        } catch (e: any) {
          throw new Error(`Failed to parse JSON: ${e?.message || String(e)}`);
        }
      }

      return {
        ok: true,
        status: resp.status,
        status_text: resp.statusText,
        method,
        url: url.toString(),
        response_format: responseFormat,
        data
      };
    } finally {
      cancellationArmed = false;
      if (timer) clearTimeout(timer as any);
    }
  },

  set_cookie: async (step: any) => {
    const cookie = step.cookie;
    if (!cookie || typeof cookie !== 'object') throw new Error('Missing cookie for set_cookie');

    const name = String(cookie.name || '').trim();
    const value = String(cookie.value ?? '').trim();
    if (!name) throw new Error('cookie.name is required for set_cookie');

    const parts: string[] = [];
    parts.push(`${encodeURIComponent(name)}=${encodeURIComponent(value)}`);

    const domain = cookie.domain ? String(cookie.domain).trim() : '';
    if (domain) parts.push(`Domain=${domain}`);

    const path = cookie.path ? String(cookie.path).trim() : '/';
    if (path) parts.push(`Path=${path}`);

    if (cookie.secure === true) parts.push('Secure');

    const expires = cookie.expiration_date;
    if (typeof expires === 'number' && Number.isFinite(expires) && expires > 0) {
      const d = new Date(expires * 1000);
      parts.push(`Expires=${d.toUTCString()}`);
    }

    // HttpOnly cannot be set via document.cookie; ignore.
    document.cookie = parts.join('; ');

    return {
      success: true,
      name,
      domain: domain || window.location.hostname,
      path,
      http_only_ignored: cookie.http_only === true,
    };
  },

  get_cookies: async (step: any) => {
    const domain = step.domain ? String(step.domain).trim() : '';
    const host = window.location.hostname;
    if (domain && !(host === domain || host.endsWith(`.${domain}`) || domain.endsWith(`.${host}`))) {
      throw new Error(`get_cookies only supports current origin. current=${host} requested=${domain}`);
    }

    const raw = String(document.cookie || '');
    const cookies = raw
      ? raw.split(';').map(s => s.trim()).filter(Boolean).map(pair => {
          const eq = pair.indexOf('=');
          const k = eq >= 0 ? pair.slice(0, eq) : pair;
          const v = eq >= 0 ? pair.slice(eq + 1) : '';
          return { name: decodeURIComponent(k), value: decodeURIComponent(v) };
        })
      : [];

    return { cookies, domain: domain || host, count: cookies.length };
  },

  clear_cookies: async (step: any) => {
    const domain = step.domain ? String(step.domain).trim() : '';
    const host = window.location.hostname;
    if (domain && !(host === domain || host.endsWith(`.${domain}`) || domain.endsWith(`.${host}`))) {
      throw new Error(`clear_cookies only supports current origin. current=${host} requested=${domain}`);
    }

    const raw = String(document.cookie || '');
    const names = raw
      ? raw.split(';').map(s => s.trim()).filter(Boolean).map(pair => {
          const eq = pair.indexOf('=');
          const k = eq >= 0 ? pair.slice(0, eq) : pair;
          return decodeURIComponent(k);
        })
      : [];

    const expired = new Date(0).toUTCString();
    let cleared = 0;
    for (const name of names) {
      const parts: string[] = [];
      parts.push(`${encodeURIComponent(name)}=`);
      parts.push(`Expires=${expired}`);
      parts.push('Path=/');
      if (domain) parts.push(`Domain=${domain}`);
      document.cookie = parts.join('; ');
      cleared += 1;
    }

    return { success: true, cleared, domain: domain || host };
  },

  set_local_storage_item: async (step: any) => {
    const key = String(step.storage_key || '').trim();
    const value = String(step.storage_value ?? '');
    if (!key) throw new Error('Missing storage_key for set_local_storage_item');
    localStorage.setItem(key, value);
    return { success: true, storage_key: key };
  },

  get_local_storage_item: async (step: any) => {
    const key = String(step.storage_key || '').trim();
    if (!key) throw new Error('Missing storage_key for get_local_storage_item');
    const value = localStorage.getItem(key);
    return { storage_key: key, storage_value: value };
  },

  clear_local_storage: async (_step: any) => {
    localStorage.clear();
    return { success: true };
  },

  execute_javascript: async (step: any) => {
    if (!step?.script) {
      throw new Error('Missing script for execute_javascript');
    }
    const pageBridgeResult = await tryEvalViaPageBridge(step, 'page_bridge_main_world_compat');
    if (pageBridgeResult) {
      return pageBridgeResult;
    }
    const world = String(step.world || 'main').toLowerCase();
    if (world === 'main') {
      return await actionHandlers.eval_main_world(step);
    }
    return await actionHandlers.eval_isolated_world(step);
  }
};

// Expose enhanced DOM capture functions to the page context
// This allows the autonomous planner to call them via ExecuteJavascript
(window as any).captureEnhancedDOMSnapshot = captureEnhancedDOMSnapshot;
(window as any).captureCurrentDOM = captureCurrentDOM;

// Also expose to isolated world for direct access
if (typeof exportFunction !== 'undefined') {
  // Firefox - export to page context
  exportFunction(captureEnhancedDOMSnapshot, window, { defineAs: 'captureEnhancedDOMSnapshot' });
  exportFunction(captureCurrentDOM, window, { defineAs: 'captureCurrentDOM' });
}

const CONTENT_SCRIPT_PROTOCOL_VERSION = 'rzn-cs-2026-03-17-3';
const CONTENT_SCRIPT_HANDSHAKE_CMD = 'rzn_handshake_v1';
const CONTENT_SCRIPT_EXECUTE_STEP_CMD = 'rzn_execute_step_v1';
const CONTENT_SCRIPT_DOM_SNAPSHOT_CMD = 'rzn_get_dom_snapshot_v1';
const CONTENT_SCRIPT_ACTIVE_INSTANCE_ATTR = 'data-rzn-active-content-script-instance';
const CONTENT_SCRIPT_ACTIVE_PROTOCOL_ATTR = 'data-rzn-active-content-script-protocol';
const CONTENT_SCRIPT_EXECUTION_CACHE_CONTAINER_ID = '__rzn_content_script_execution_cache';
const CONTENT_SCRIPT_EXECUTION_CACHE_TTL_MS = 30_000;
const CONTENT_SCRIPT_INSTANCE_ID = `${CONTENT_SCRIPT_PROTOCOL_VERSION}:${Date.now().toString(36)}:${Math.random().toString(36).slice(2, 10)}`;
const CONTENT_SCRIPT_BRIDGE_TOKEN_ATTR = 'data-rzn-bridge-token';
const CONTENT_SCRIPT_DOM_OWNER_ATTR = 'data-rzn-owner';
const CONTENT_SCRIPT_PAGE_BRIDGE_TOKEN = (() => {
  try {
    const bytes = new Uint8Array(24);
    crypto.getRandomValues(bytes);
    return Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
  } catch {
    return `${Date.now().toString(36)}:${Math.random().toString(36).slice(2)}:${Math.random().toString(36).slice(2)}`;
  }
})();

const PAGE_CHANNEL_ALLOWED_STEP_TYPES = new Set([
  'assert_selector_state',
  'assert_text_in_element',
  'assert_url_matches',
  'capture_ui_bundle',
  'click_element',
  'dbl_click_element',
  'detect_popups',
  'dismiss_popups',
  'drag_and_drop',
  'extract_structured_data',
  'fill_and_submit',
  'fill_input_field',
  'get_element_attribute',
  'get_element_count',
  'get_element_text',
  'get_element_value',
  'hover_element',
  'inspect_click_surface',
  'inspect_element',
  'observe',
  'press_key',
  'press_special_key',
  'read_field_value',
  'scroll_element_into_view',
  'scroll_window_to',
  'select_option_in_dropdown',
  'semantic_action',
  'submit_input',
  'submit_text_query',
  'type_text',
  'verify_ui_change',
  'wait_for_element',
  'wait_for_no_popups',
  'wait_for_timeout',
]);

const PAGE_CHANNEL_DEV_ONLY_STEP_TYPES = new Set([
  'click_element_enhanced',
  'eval_isolated_world',
  'eval_main_world',
  'execute_extraction_plan',
  'execute_javascript',
  'extract_structured_data_enhanced',
  'fill_input_field_enhanced',
  'infinite_scroll',
  'scroll_element_into_view_enhanced',
  'take_screenshot',
]);

function installContentBridgeMetadata(el: HTMLElement): void {
  el.setAttribute('data-rzn-content-build', RZN_BUILD_SIGNATURE);
  el.setAttribute('data-rzn-content-protocol', CONTENT_SCRIPT_PROTOCOL_VERSION);
  el.setAttribute('data-rzn-content-instance', CONTENT_SCRIPT_INSTANCE_ID);
  if (RZN_PAGE_TEST_BRIDGE_ENABLED) {
    el.setAttribute(CONTENT_SCRIPT_BRIDGE_TOKEN_ATTR, CONTENT_SCRIPT_PAGE_BRIDGE_TOKEN);
  } else {
    el.removeAttribute(CONTENT_SCRIPT_BRIDGE_TOKEN_ATTR);
  }
}

function isValidBridgeToken(value: unknown): boolean {
  return typeof value === 'string' && value.length > 0 && value === CONTENT_SCRIPT_PAGE_BRIDGE_TOKEN;
}

function assertPageChannelStepAllowed(step: any): void {
  const stepType = typeof step?.type === 'string' ? step.type : '';
  const isAllowed =
    PAGE_CHANNEL_ALLOWED_STEP_TYPES.has(stepType) ||
    (RZN_PAGE_TEST_BRIDGE_ENABLED && PAGE_CHANNEL_DEV_ONLY_STEP_TYPES.has(stepType));
  if (!stepType || !isAllowed) {
    throw new Error(`Step type is not allowed from the page bridge: ${stepType || '<missing>'}`);
  }
}

function windowMessageTargetOrigin(): string | null {
  const origin = window.location.origin;
  if (!origin || origin === 'null') return null;
  return origin;
}

function postPageBridgeMessage(message: any): void {
  const targetOrigin = windowMessageTargetOrigin();
  if (!targetOrigin) return;
  window.postMessage(message, targetOrigin);
}

function claimActiveContentScriptInstance(): void {
  const root = document.documentElement;
  if (!root) return;
  root.setAttribute(CONTENT_SCRIPT_ACTIVE_INSTANCE_ATTR, CONTENT_SCRIPT_INSTANCE_ID);
  root.setAttribute(CONTENT_SCRIPT_ACTIVE_PROTOCOL_ATTR, CONTENT_SCRIPT_PROTOCOL_VERSION);
}

function isActiveContentScriptInstance(): boolean {
  const root = document.documentElement;
  if (!root) return true;
  return root.getAttribute(CONTENT_SCRIPT_ACTIVE_INSTANCE_ATTR) === CONTENT_SCRIPT_INSTANCE_ID;
}

function ensureExecutionCacheContainer(): HTMLElement {
  let el = document.getElementById(CONTENT_SCRIPT_EXECUTION_CACHE_CONTAINER_ID) as HTMLElement | null;
  if (!el) {
    el = document.createElement('div');
    el.id = CONTENT_SCRIPT_EXECUTION_CACHE_CONTAINER_ID;
    el.style.display = 'none';
    (document.documentElement || document.body || document.head).appendChild(el);
  }
  return el;
}

function pruneExecutionCache(container: HTMLElement): void {
  const now = Date.now();
  for (const child of Array.from(container.children)) {
    if (!(child instanceof HTMLElement)) continue;
    const ts = Number(child.getAttribute('data-rzn-ts') || '0');
    if (ts > 0 && now - ts > CONTENT_SCRIPT_EXECUTION_CACHE_TTL_MS) {
      try {
        child.remove();
      } catch {}
    }
  }
}

function getExecutionCacheNode(container: HTMLElement, requestId: string): HTMLElement | null {
  for (const child of Array.from(container.children)) {
    if (!(child instanceof HTMLElement)) continue;
    if (child.getAttribute(CONTENT_SCRIPT_DOM_OWNER_ATTR) !== CONTENT_SCRIPT_INSTANCE_ID) continue;
    if (child.getAttribute('data-rzn-req-id') === requestId) {
      return child;
    }
  }
  return null;
}

function parseExecutionCacheResponse(node: HTMLElement): any | null {
  const raw = node.getAttribute('data-rzn-resp');
  if (!raw) return null;
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

function stepFingerprint(step: any): string | undefined {
  if (!step || typeof step !== 'object') return undefined;
  try {
    const raw = JSON.stringify(step);
    let hash = 2166136261;
    for (let i = 0; i < raw.length; i++) {
      hash ^= raw.charCodeAt(i);
      hash = Math.imul(hash, 16777619);
    }
    return `step:${(hash >>> 0).toString(16)}`;
  } catch {
    return undefined;
  }
}

function getRecentExecutionNodeByStepKey(
  container: HTMLElement,
  stepKey: string | undefined,
): HTMLElement | null {
  if (!stepKey) return null;
  const now = Date.now();
  for (const child of Array.from(container.children)) {
    if (!(child instanceof HTMLElement)) continue;
    if (child.getAttribute(CONTENT_SCRIPT_DOM_OWNER_ATTR) !== CONTENT_SCRIPT_INSTANCE_ID) continue;
    if (child.getAttribute('data-rzn-step-key') !== stepKey) continue;
    if (child.hasAttribute('data-rzn-resp') || child.hasAttribute('data-rzn-err')) continue;
    const ts = Number(child.getAttribute('data-rzn-ts') || '0');
    if (ts > 0 && now - ts <= CONTENT_SCRIPT_EXECUTION_CACHE_TTL_MS) {
      return child;
    }
  }
  return null;
}

async function executeWithSharedRequestDedup<T>(
  requestId: string | undefined,
  stepKey: string | undefined,
  compute: () => Promise<T>,
): Promise<T> {
  const container = ensureExecutionCacheContainer();
  pruneExecutionCache(container);

  let node =
    (requestId ? getExecutionCacheNode(container, requestId) : null) ||
    getRecentExecutionNodeByStepKey(container, stepKey);
  if (!node) {
    node = document.createElement('div');
    if (requestId) {
      node.setAttribute('data-rzn-req-id', requestId);
    }
    if (stepKey) {
      node.setAttribute('data-rzn-step-key', stepKey);
    }
    node.setAttribute(CONTENT_SCRIPT_DOM_OWNER_ATTR, CONTENT_SCRIPT_INSTANCE_ID);
    node.setAttribute('data-rzn-ts', String(Date.now()));
    container.appendChild(node);
  }

  const owner = node.getAttribute(CONTENT_SCRIPT_DOM_OWNER_ATTR);
  if (owner && owner !== CONTENT_SCRIPT_INSTANCE_ID) {
    return await new Promise<T>((resolve, reject) => {
      const finish = (value: T | null, error?: string) => {
        observer.disconnect();
        clearTimeout(timer);
        if (error) reject(new Error(error));
        else if (value !== null) resolve(value);
        else reject(new Error(`Timed out waiting for duplicate execute_step response for ${requestId}`));
      };

      const read = () => {
        const response = parseExecutionCacheResponse(node!);
        if (response !== null) {
          finish(response as T);
          return;
        }
        const err = node!.getAttribute('data-rzn-err');
        if (err) {
          finish(null, err);
        }
      };

      const observer = new MutationObserver(() => read());
      observer.observe(node!, {
        attributes: true,
        attributeFilter: ['data-rzn-resp', 'data-rzn-err', 'data-rzn-ts'],
      });
      const timer = setTimeout(() => finish(null), 10_000);
      read();
    });
  }

  const cached = parseExecutionCacheResponse(node);
  if (cached !== null) {
    return cached as T;
  }

  try {
    const result = await compute();
    node.setAttribute('data-rzn-resp', JSON.stringify(result));
    node.setAttribute('data-rzn-ts', String(Date.now()));
    return result;
  } catch (error: any) {
    const message = error?.message || String(error);
    node.setAttribute('data-rzn-err', message);
    node.setAttribute('data-rzn-ts', String(Date.now()));
    throw error;
  }
}

claimActiveContentScriptInstance();

// Message listener
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (!isActiveContentScriptInstance()) {
    return false;
  }

  if (message.cmd === 'RZN_CANCEL_REQUEST') {
    const cancelled = markContentRequestCancelled(message);
    sendResponse({
      req_id: cancelled.requestId,
      lease_id: cancelled.leaseId,
      success: true,
      cancelled: true,
      reason: cancelled.reason,
    });
    return false;
  }

  // Handle ping for connection check
  if (message.cmd === 'ping') {
    sendResponse({ success: true, pong: true, protocol_version: CONTENT_SCRIPT_PROTOCOL_VERSION });
    return false;
  }

  if (message.cmd === CONTENT_SCRIPT_HANDSHAKE_CMD) {
    sendResponse({
      success: true,
      pong: true,
      protocol_version: CONTENT_SCRIPT_PROTOCOL_VERSION,
      instance_id: CONTENT_SCRIPT_INSTANCE_ID,
      capabilities: {
        execute_step_cmd: CONTENT_SCRIPT_EXECUTE_STEP_CMD,
        dom_snapshot_cmd: CONTENT_SCRIPT_DOM_SNAPSHOT_CMD,
      }
    });
    return false;
  }

  // Reference surface: enumerate candidates + robust selectors in top frame
  if (message.cmd === 'process_dom') {
    try {
      const opts = message.payload?.options || {};
      const scope: Document | Element = opts.scopeSelector ? (document.querySelector(opts.scopeSelector) || document) : document;
      const LIMIT: number = Math.max(1, Math.min(1500, opts.limit || 800));

      // Basic visibility
      const isVisible = (el: Element): boolean => {
        const cs = getComputedStyle(el as HTMLElement);
        if (!cs || cs.display === 'none' || cs.visibility === 'hidden' || cs.opacity === '0') return false;
        const rect = (el as HTMLElement).getBoundingClientRect();
        return rect.width > 0 && rect.height > 0;
      };
      const shortText = (el: Element): string => (el.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 160);
      const roleFor = (el: Element): string => {
        const ariaRole = el.getAttribute('role');
        if (ariaRole) return ariaRole;
        const tag = el.tagName.toLowerCase();
        if (tag === 'a') return 'link';
        if (['button','input','select','textarea'].includes(tag)) return tag;
        if (/^h[1-6]$/.test(tag)) return 'heading';
        if (tag === 'img') return 'img';
        if (tag === 'li') return 'listitem';
        return tag;
      };
      const collectAttrs = (el: Element) => {
        const attrs: Record<string, string> = {};
        for (const a of Array.from(el.attributes)) {
          if (['id','name','href','src','alt','title','role','aria-label','aria-labelledby','aria-describedby'].includes(a.name)) {
            attrs[a.name] = (a.value || '').toString().slice(0, 500);
          }
        }
        return attrs;
      };
      const xpathsFor = (el: Element): string[] => {
        const out: string[] = [];
        const id = (el as HTMLElement).id;
        if (id) {
          try { if (document.querySelectorAll(`#${CSS.escape(id)}`).length === 1) { out.push(`//*[@id=${JSON.stringify(id)}]`); } } catch {}
        }
        const absoluteXPath = (node: Element | null): string => {
          if (!node) return '';
          const parts: string[] = [];
          for (let e: Element | null = node; e && e.nodeType === 1 && e !== document.documentElement; e = e.parentElement) {
            const tag = e.tagName.toLowerCase();
            const siblings = e.parentElement ? Array.from(e.parentElement.children).filter(c => (c as Element).tagName === e.tagName) : [];
            const index = siblings.length > 1 ? `[${siblings.indexOf(e)+1}]` : '';
            parts.unshift(`${tag}${index}`);
          }
          return `/html/${parts.join('/')}`;
        };
        out.push(absoluteXPath(el));
        const text = shortText(el);
        if (text) out.push(`//*[normalize-space(.)=${JSON.stringify(text)}]`);
        return Array.from(new Set(out)).filter(Boolean);
      };

      const elements: Array<{ id: string; role: string; tag: string; text: string; attrs: Record<string,string>; bbox: {x:number;y:number;width:number;height:number} }> = [];
      const idToXPaths: Record<string,string[]> = {};
      const idToUrl: Record<string,string> = {};
      const elementToId = new Map<Element, string>();
      let localId = 0;
      const pushEl = (el: Element) => {
        if (elementToId.has(el)) return elementToId.get(el)!;
        localId++;
        const id = `0:${localId}`;
        elementToId.set(el, id);
        const tag = el.tagName.toLowerCase();
        const role = roleFor(el);
        const rect = (el as HTMLElement).getBoundingClientRect();
        const bbox = { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
        const attrs = collectAttrs(el);
        idToXPaths[id] = xpathsFor(el);
        if (tag === 'a' && (el as HTMLAnchorElement).href) idToUrl[id] = (el as HTMLAnchorElement).href;
        if (tag === 'img' && (el as HTMLImageElement).src) idToUrl[id] = (el as HTMLImageElement).src;
        elements.push({ id, role, tag, text: shortText(el), attrs, bbox });
        return id;
      };

      // Walk DOM and collect candidates
      const walker = document.createTreeWalker(scope, NodeFilter.SHOW_ELEMENT, {
        acceptNode(node) {
          const el = node as Element;
          const tag = el.tagName.toLowerCase();
          const keepByTag = ['a','button','input','select','textarea','img','li','article'].includes(tag);
          const keepByRole = !!el.getAttribute('role');
          const keepHeading = /^h[1-6]$/.test(tag);
          const keepContent = (shortText(el).length >= 3);
          if ((keepByTag || keepByRole || keepHeading || keepContent) && isVisible(el)) {
            return NodeFilter.FILTER_ACCEPT;
          }
          return NodeFilter.FILTER_SKIP;
        }
      } as any, false);

      let node: Node | null;
      while ((node = walker.nextNode()) && elements.length < LIMIT) {
        pushEl(node as Element);
      }

      // Optional: auto list detection integrated
      let autoList: any = null;
      if (opts.detectAutoList) {
        const $$ = (root: ParentNode, sel: string) => Array.from(root.querySelectorAll(sel));
        const classTokens = (el: Element) => Array.from((el as HTMLElement).classList || [])
          .filter(c => !/\d{3,}/.test(c)).filter(c => c.length >= 3).slice(0, 6).sort();
        const signature = (el: Element) => {
          const tag = el.tagName.toLowerCase();
          const cls = classTokens(el).join('.');
          const hasA = !!el.querySelector('a[href]');
          const hasImg = !!el.querySelector('img');
          const hasHead = !!el.querySelector('h1,h2,h3,h4,h5,h6,[role="heading"]');
          return `${tag}|${cls}|a:${hasA?'1':'0'}|img:${hasImg?'1':'0'}|h:${hasHead?'1':'0'}`;
        };
        const anchorDensity = (el: Element) => {
          const all = (el.textContent || '').replace(/\s+/g, ' ').trim();
          if (!all) return 0;
          const aText = $$(el, 'a').map(a => (a.textContent || '').replace(/\s+/g, ' ').trim()).join(' ');
          return aText.length / Math.max(1, all.length);
        };
        const hasPrice = (el: Element) => /\b(?:[$€£¥]|USD|EUR|GBP|JPY)\s?\d[\d,\.\s]*/i.test(el.textContent || '');
        const hasRating = (el: Element) => /★|☆|(\b\d\.\d\b\s*\/\s*5)|\b(stars?)\b/i.test(el.textContent || '') || !!el.querySelector('[aria-label*="star"]');
        const stddev = (nums: number[]) => { if (nums.length < 2) return 0; const m = nums.reduce((a,b)=>a+b,0)/nums.length; const v = nums.reduce((a,b)=>a+(b-m)*(b-m),0)/nums.length; return Math.sqrt(v); };
        const findCommonAncestor = (elements: Element[]): Element | null => {
          if (!elements.length) return null;
          let ancestor: Element | null = elements[0];
          while (ancestor) {
            if (elements.every(el => ancestor === el || ancestor.contains(el))) break;
            ancestor = ancestor.parentElement;
          }
          if (!ancestor || ancestor === document.body || ancestor === document.documentElement) return null;
          return ancestor;
        };
        const fallbackGlobalLists = (exclude: Set<Element>): Scored[] => {
          const rawItems = $$(document, 'article, section, li, div')
            .filter((el: Element) => isVisible(el))
            .filter((el: Element) => {
              const link = el.querySelector('a[href]');
              const linkText = link ? (link.textContent || '').replace(/\s+/g, ' ').trim() : '';
              const text = shortText(el).length;
              return (link && linkText.length >= 12) || text >= 60;
            })
            .slice(0, 800);

          const groups = new Map<string, Element[]>();
          for (const item of rawItems) {
            const sig = signature(item);
            if (!groups.has(sig)) groups.set(sig, []);
            groups.get(sig)!.push(item);
          }

          const results: Scored[] = [];
          for (const group of groups.values()) {
            if (group.length < 3) continue;
            const ancestor = findCommonAncestor(group);
            if (!ancestor || exclude.has(ancestor)) continue;
            const contained = group.filter(el => ancestor.contains(el));
            const uniqueItems = Array.from(new Set(contained));
            if (uniqueItems.length < 3) continue;

            const widths = uniqueItems.map(el => (el as HTMLElement).getBoundingClientRect().width);
            const heights = uniqueItems.map(el => (el as HTMLElement).getBoundingClientRect().height);
            const sdW = stddev(widths);
            const sdH = stddev(heights);
            const medianText = uniqueItems.map(el => shortText(el).length).sort((a,b)=>a-b)[Math.floor(uniqueItems.length/2)];
            const linkD = anchorDensity(ancestor);
            const priceHits = uniqueItems.reduce((n,el)=>n+(hasPrice(el)?1:0),0);
            const ratingHits = uniqueItems.reduce((n,el)=>n+(hasRating(el)?1:0),0);
            const ariaList = ancestor.getAttribute('role') === 'list' ? 1 : 0;
            const navPenalty = (linkD > 0.6 && medianText < 30) ? 1 : 0;
            const visibleChildren = Array.from(ancestor.children).filter((el: Element) => isVisible(el));
            const features = {
              repeat: uniqueItems.length / Math.max(3, visibleChildren.length || uniqueItems.length),
              visual: 1 / (1 + sdW / Math.max(1, widths[0] || 1) + sdH / Math.max(1, heights[0] || 1)),
              textDensity: Math.min(1, medianText / 120),
              linkBalance: 1 - Math.min(1, Math.abs(linkD - 0.35)),
              price: Math.min(1, priceHits / uniqueItems.length),
              rating: Math.min(1, ratingHits / uniqueItems.length),
              aria: ariaList,
              jsonld: 0,
              navPenalty
            };
            const score = 3.0*features.repeat + 1.3*features.visual + 1.2*features.textDensity + 1.0*features.linkBalance + 0.8*features.aria + 0.4*features.price + 0.3*features.rating - 1.5*features.navPenalty;
            const containerSelector = cssFor(ancestor);
            const itemSelector = robustItemSelector(ancestor, uniqueItems);
            results.push({ container: ancestor, items: uniqueItems, containerSelector, itemSelector, score, features });
          }
          results.sort((a,b)=>b.score - a.score);
          return results.slice(0, 5);
        };

        const seeds = [
          'main', '#search', '[role="main"]', '[role="feed"]',
          '[role="list"]', 'ul', 'ol', 'table', 'tbody',
          'section', 'article', 'div[class*="list"]', 'div[class*="result"]',
          'div[class*="items"]', 'div[class*="grid"]',
          'div[id*="table"]', 'div[id*="list"]', 'div[id*="feed"]',
          'div[data-testid*="list"]'
        ].join(',');
        const maxContainers = opts.maxContainers ?? 80;
        const candidates = $$(document, seeds).filter((c: Element) => isVisible(c)).slice(0, maxContainers);

        type Scored = { container: Element; items: Element[]; containerSelector: string; itemSelector: string; score: number; features: Record<string, number> };
        const seenContainers = new Set<Element>();
        const scored: Scored[] = [];
        const cssFor = (el: Element): string => {
          const id = (el as HTMLElement).id;
          if (id && document.querySelectorAll(`#${CSS.escape(id)}`).length === 1) return `#${CSS.escape(id)}`;
          const toks = classTokens(el);
          if (toks.length) {
            const sel = `${el.tagName.toLowerCase()}.${toks.map(CSS.escape).join('.')}`;
            if (document.querySelectorAll(sel).length <= 10) return sel;
          }
          const role = el.getAttribute('role');
          if (role) {
            const sel = `${el.tagName.toLowerCase()}[role="${role}"]`;
            if (document.querySelectorAll(sel).length <= 10) return sel;
          }
          const p = el.parentElement;
          if (p && (p as HTMLElement).id) return `#${CSS.escape((p as HTMLElement).id)} > ${el.tagName.toLowerCase()}`;
          return el.tagName.toLowerCase();
        };
        const robustItemSelector = (container: Element, items: Element[]) => {
          const inter = items.reduce<string[]>((acc, el, i) => { const cls = classTokens(el); return i === 0 ? cls : acc.filter(c => cls.includes(c)); }, []);
          const tag = items[0].tagName.toLowerCase();
          if (inter.length) { const sel = `${tag}.${inter.map(CSS.escape).join('.')}`; if ($$(container, sel).length >= items.length * 0.8) return sel; }
          const roleSel = `${tag}[role="listitem"],${tag}[role="article"]`;
          if ($$(container, roleSel).length >= items.length * 0.6) return roleSel;
          return `${tag}`;
        };

        for (const container of candidates) {
          const kids = Array.from(container.children).filter((el: Element) => isVisible(el)).filter((ch: Element) => shortText(ch).length + $$(ch, 'img,a').length > 0);
          if (kids.length < 3) continue;
          const groups = new Map<string, Element[]>();
          for (const k of kids) { const sig = signature(k); (groups.get(sig) || groups.set(sig, []).get(sig)!).push(k); }
          const items = Array.from(groups.values()).sort((a,b)=>b.length-a.length)[0] || [];
          if (items.length < 3) continue;

          const widths = items.map(el => (el as HTMLElement).getBoundingClientRect().width);
          const heights = items.map(el => (el as HTMLElement).getBoundingClientRect().height);
          const sdW = stddev(widths), sdH = stddev(heights);
          const medianText = items.map(el => shortText(el).length).sort((a,b)=>a-b)[Math.floor(items.length/2)];
          const linkD = anchorDensity(container);
          const priceHits = items.reduce((n,el)=>n+(hasPrice(el)?1:0),0);
          const ratingHits = items.reduce((n,el)=>n+(hasRating(el)?1:0),0);
          const ariaList = container.getAttribute('role') === 'list' ? 1 : 0;
          const navPenalty = (linkD > 0.6 && medianText < 30) ? 1 : 0;
          const features = {
            repeat: items.length / Math.max(3, kids.length),
            visual: 1 / (1 + sdW / Math.max(1, widths[0]) + sdH / Math.max(1, heights[0])),
            textDensity: Math.min(1, medianText / 120),
            linkBalance: 1 - Math.min(1, Math.abs(linkD - 0.35)),
            price: Math.min(1, priceHits / items.length),
            rating: Math.min(1, ratingHits / items.length),
            aria: ariaList,
            jsonld: 0, // omitted deep JSON-LD scan here for perf; detector still robust
            navPenalty
          };
          const score = 3.0*features.repeat + 1.3*features.visual + 1.2*features.textDensity + 1.0*features.linkBalance + 0.8*features.aria + 0.4*features.price + 0.3*features.rating - 1.5*features.navPenalty;
          const containerSelector = cssFor(container);
          const itemSelector = robustItemSelector(container, items);
          scored.push({ container, items, containerSelector, itemSelector, score, features });
          seenContainers.add(container);
        }
        if (!scored.length) {
          const fallback = fallbackGlobalLists(seenContainers);
          fallback.forEach(entry => scored.push(entry));
        }
        if (scored.length) {
          scored.sort((a,b)=>b.score-a.score);
          const best = scored[0];
          // ensure item elements are in inventory and map to IDs
          const itemIds = best.items.map(el => pushEl(el));
          autoList = {
            containerSelector: best.containerSelector,
            itemSelector: best.itemSelector,
            itemIds,
            score: best.score,
            features: best.features,
            alternatives: scored.slice(1,3).map(s=>({ containerSelector: s.containerSelector, itemSelector: s.itemSelector, size: s.items.length, score: s.score }))
          };
        }
      }

      sendResponse({ success: true, frameOrdinal: 0, elements, idToXPaths, idToUrl, autoList, count: elements.length });
    } catch (e: any) {
      sendResponse({ success: false, error: e?.message || String(e) });
    }
    return false;
  }

  // Deterministic extraction via a validated plan (no arbitrary JS execution).
  // This is intended for desktop/agent tool calls that need a safe, auditable surface.
  if (message.cmd === 'execute_extraction_plan') {
    (async () => {
      try {
        const { ExtractionPlanV1Schema } = await import('./types/extractionPlan');
        const plan = ExtractionPlanV1Schema.parse(message.payload?.plan);

        const resolveScope = (): ParentNode => {
          if (!plan.scope) return document;
          if (plan.scope.css) {
            const el = document.querySelector(plan.scope.css);
            if (!el) throw new Error(`Scope not found for css: ${plan.scope.css}`);
            return el;
          }
          if (plan.scope.xpath) {
            const xr = document.evaluate(
              plan.scope.xpath,
              document,
              null,
              XPathResult.FIRST_ORDERED_NODE_TYPE,
              null
            );
            const node = xr.singleNodeValue as Element | null;
            if (!node) throw new Error(`Scope not found for xpath: ${plan.scope.xpath}`);
            return node;
          }
          return document;
        };

        const scopeNode = resolveScope();
        const limit = plan.limit ?? 50;

        const safeText = (root: Element): string => {
          const clone = root.cloneNode(true) as Element;
          clone.querySelectorAll('script,style,noscript').forEach(n => n.remove());
          return (clone.textContent || '').trim();
        };
        const extractValue = (el: Element, attribute?: string): string | null => {
          if (!attribute) return safeText(el);
          return el.getAttribute(attribute);
        };

        let result: any;
        if (plan.mode === 'single') {
          const out: Record<string, any> = {};
          for (const field of plan.fields) {
            const el = (scopeNode as any).querySelector?.(field.selector) as Element | null;
            if (!el) {
              out[field.name] = field.optional ? null : null;
              continue;
            }
            out[field.name] = extractValue(el, field.attribute);
          }
          result = out;
        } else {
          const itemSel = plan.item_selector!;
          const nodes = Array.from((scopeNode as any).querySelectorAll?.(itemSel) || []) as Element[];
          const items: any[] = [];
          for (const item of nodes.slice(0, limit)) {
            const row: Record<string, any> = {};
            for (const field of plan.fields) {
              const selector = (field.selector || '').trim();
              const el = selector === ':scope' ? item : item.querySelector(selector);
              if (!el) {
                row[field.name] = field.optional ? null : null;
                continue;
              }
              row[field.name] = extractValue(el, field.attribute);
            }
            // Keep only rows with at least one non-null field
            if (Object.values(row).some(v => v !== null && v !== '')) {
              items.push(row);
            }
          }
          result = items;
        }

        sendResponse({
          req_id: message.req_id,
          success: true,
          result,
          plan_version: plan.version,
          rung_used: 1,
          dom_hash: domHash(),
          current_url: window.location.href
        });
      } catch (e: any) {
        sendResponse({
          req_id: message.req_id,
          success: false,
          error_code: 'EXTRACTION_PLAN_ERROR',
          error_msg: e?.message || String(e),
          plan_version: 1,
          rung_used: 1,
          dom_hash: domHash(),
          current_url: window.location.href
        });
      }
    })();

    return true;
  }
  
  if (message.cmd === 'execute_step' || message.cmd === CONTENT_SCRIPT_EXECUTE_STEP_CMD) {
    (async () => {
      // Declared outside the try so the catch handler below can reuse the same
      // dedup cache key. Previously these lived inside the try, so any handler
      // throw (e.g. a wait_for_element timeout) made the catch reference an
      // out-of-scope `executionCacheRequestId`, masking the real error with a
      // `ReferenceError: executionCacheRequestId is not defined`.
      const contentLease = contentLeaseId(message);
      const executionCacheRequestId =
        message.req_id && contentLease ? `${message.req_id}:${contentLease}` : message.req_id;
      try {
        throwIfContentRequestCancelled(message, 'execute_step message start');
        const response = await executeWithSharedRequestDedup(
          executionCacheRequestId,
          stepFingerprint(message.payload?.step),
          async () => {
          // Prefer standard handlers by default; only use enhanced when explicitly requested
          const stepObj = {
            ...(message.payload?.step || {}),
            req_id: message.req_id,
            lease_id: contentLeaseId(message),
            __rzn_lease_id: contentLeaseId(message),
          };
          const stepType = stepObj.type;
          const enhancedType = stepType + '_enhanced';
          const forceLegacy = !!stepObj.force_legacy;
          const wantEnhanced = !!(stepObj.use_enhanced === true || stepType.endsWith('_enhanced') || stepObj.target_spec);

          let handler;

          if (!forceLegacy && wantEnhanced) {
            if (enhancedActionHandlers[enhancedType as keyof typeof enhancedActionHandlers]) {
              handler = enhancedActionHandlers[enhancedType as keyof typeof enhancedActionHandlers];
              console.log(`[RZN] Using enhanced handler for ${stepType}`);
            } else if (enhancedActionHandlers[stepType as keyof typeof enhancedActionHandlers]) {
              handler = enhancedActionHandlers[stepType as keyof typeof enhancedActionHandlers];
              console.log(`[RZN] Using enhanced handler for ${stepType}`);
            }
          }

          if (!handler && (actionHandlers as any)[stepType]) {
            handler = (actionHandlers as any)[stepType];
            console.log(`[RZN] Using standard handler for ${stepType}${forceLegacy ? ' (force_legacy)' : ''}`);
          }

          if (!handler) {
            throw new Error(`Unknown action type: ${stepType}`);
          }

          throwIfContentRequestCancelled(stepObj, `before ${stepType}`);
          const result = await handler(stepObj);
          throwIfContentRequestCancelled(stepObj, `after ${stepType}`);
          const actionFailure = actionFailureResponseFields(result);
          const domSnapshot = captureEnhancedDOMSnapshot({
            maxElements: 120,
            highlightElements: false,
          });

          return {
            req_id: message.req_id,
            lease_id: contentLeaseId(message),
            ...actionFailure,
            result: result,
            validation_passed: typeof result === 'boolean' ? result : actionFailure.success,
            current_url: window.location.href,
            dom_snapshot: domSnapshot,
            dom_hash: domSnapshot.hash,
          };
        });

        sendResponse(response);
      } catch (error) {
        const err = error as Error;
        try {
          const response = await executeWithSharedRequestDedup(
            executionCacheRequestId,
            stepFingerprint(message.payload?.step),
            async () => {
            const errorCode = err.message.includes('SELECTOR_NOT_FOUND') ? 'SELECTOR_NOT_FOUND' :
                             err.message.includes('Timeout') ? 'TIMEOUT' : 'EXECUTION_ERROR';

            const domSnapshot = captureEnhancedDOMSnapshot({
              maxElements: 120,
              highlightElements: false
            });

            return {
              req_id: message.req_id,
              lease_id: contentLeaseId(message),
              success: false,
              error_code: errorCode,
              error_msg: err.message,
              dom_snapshot: domSnapshot,
              dom_hash: domSnapshot.hash
            };
          });
          sendResponse(response);
        } catch (dedupError) {
          const fallbackError = dedupError as Error;
          sendResponse({
            req_id: message.req_id,
            lease_id: contentLeaseId(message),
            success: false,
            error_code: 'EXECUTION_ERROR',
            error_msg: fallbackError.message
          });
        }
      }
    })();
    return true; // Keep message channel open for async response
  }
  
  // Auto list detection (production-oriented). Returns container/item selectors and item xpaths
  if (message.cmd === 'detect_auto_list') {
    try {
      const purposeRaw = message.payload?.options?.purpose ?? 'general';
      const purpose = String(purposeRaw).toLowerCase();
      const maxCandidatesRaw =
        message.payload?.options?.maxCandidates ?? message.payload?.options?.topK ?? 3;
      const maxCandidates = Math.max(1, Math.min(10, Number(maxCandidatesRaw) || 3));

      // Inline detector (lightweight) — mirrors the public reference list detection heuristics
      const $$ = (root: ParentNode, sel: string) => Array.from(root.querySelectorAll(sel));
      const visible = (el: Element) => {
        const cs = getComputedStyle(el as HTMLElement);
        if (!cs || cs.visibility === 'hidden' || cs.display === 'none' || parseFloat(cs.opacity || '1') === 0) return false;
        const r = (el as HTMLElement).getBoundingClientRect();
        return r.width > 1 && r.height > 1;
      };
      const textLen = (el: Element) => ((el.textContent || '').replace(/\s+/g, ' ').trim()).length;
      const hasPrice = (el: Element) => /\b(?:[$€£¥]|USD|EUR|GBP|JPY)\s?\d[\d,\.\s]*/i.test(el.textContent || '');
      const hasRating = (el: Element) => /★|☆|(\b\d\.\d\b\s*\/\s*5)|\b(stars?)\b/i.test(el.textContent || '') || !!el.querySelector('[aria-label*="star"]');
      const anchorDensity = (el: Element) => {
        const text = (el.textContent || '').replace(/\s+/g, ' ').trim();
        if (!text) return 0;
        const aText = $$(el, 'a').map(a => (a.textContent || '').replace(/\s+/g, ' ').trim()).join(' ');
        return aText.length / Math.max(1, text.length);
      };
      const classTokens = (el: Element) => Array.from((el as HTMLElement).classList || [])
          .filter(c => !/\d{3,}/.test(c)).filter(c => c.length >= 3).slice(0, 6).sort();
      const signature = (el: Element) => {
        const tag = el.tagName.toLowerCase();
        const hasImg = !!el.querySelector('img');
        const hasA = !!el.querySelector('a[href]');
        const cls = classTokens(el).join('.');
        const headings = el.querySelector('h1,h2,h3,h4,h5,h6,[role="heading"]') ? 'h' : '';
        return `${tag}|${cls}|a:${hasA?'1':'0'}|img:${hasImg?'1':'0'}|h:${headings}`;
      };
      const cssFor = (el: Element): string => {
        const id = (el as HTMLElement).id;
        if (id && document.querySelectorAll(`#${CSS.escape(id)}`).length === 1) return `#${CSS.escape(id)}`;
        const tokens = classTokens(el);
        if (tokens.length) {
          const sel = `${el.tagName.toLowerCase()}.${tokens.map(CSS.escape).join('.')}`;
          if (document.querySelectorAll(sel).length <= 10) return sel;
        }
        const role = el.getAttribute('role');
        if (role) {
          const sel = `${el.tagName.toLowerCase()}[role="${role}"]`;
          if (document.querySelectorAll(sel).length <= 10) return sel;
        }
        const it = el.getAttribute('itemtype');
        if (it) {
          const sel = `${el.tagName.toLowerCase()}[itemtype]`;
          if (document.querySelectorAll(sel).length <= 20) return sel;
        }
        const p = el.parentElement;
        if (p) {
          const pId = (p as HTMLElement).id;
          if (pId) return `#${CSS.escape(pId)} > ${el.tagName.toLowerCase()}`;
        }
        return el.tagName.toLowerCase();
      };
      const robustItemSelector = (container: Element, items: Element[]) => {
        const inter = items.reduce<string[]>((acc, el, i) => {
          const cls = classTokens(el);
          return i === 0 ? cls : acc.filter(c => cls.includes(c));
        }, []);
        const tag = items[0].tagName.toLowerCase();
        if (inter.length) {
          const sel = `${tag}.${inter.map(CSS.escape).join('.')}`;
          if ($$(container, sel).length >= items.length * 0.8) return sel;
        }
        const roleSel = `${tag}[role="listitem"],${tag}[role="article"]`;
        if ($$(container, roleSel).length >= items.length * 0.6) return roleSel;
        return `${tag}`;
      };
      const stddev = (nums: number[]) => {
        if (nums.length < 2) return 0;
        const m = nums.reduce((a,b)=>a+b,0)/nums.length;
        const v = nums.reduce((a,b)=>a+(b-m)*(b-m),0)/nums.length;
        return Math.sqrt(v);
      };
      const findCommonAncestor = (elements: Element[]): Element | null => {
        if (!elements.length) return null;
        let ancestor: Element | null = elements[0];
        while (ancestor) {
          if (elements.every(el => ancestor === el || ancestor.contains(el))) break;
          ancestor = ancestor.parentElement;
        }
        if (!ancestor || ancestor === document.body || ancestor === document.documentElement) return null;
        return ancestor;
      };
      const fallbackGlobalLists = (exclude: Set<Element>): Scored[] => {
        const candidateSelector = 'article, section, li, div';
        const rawItems = $$(document, candidateSelector)
          .filter(visible)
          .filter(el => {
            const link = el.querySelector('a[href]');
            const linkText = link ? (link.textContent || '').replace(/\s+/g, ' ').trim() : '';
            const text = textLen(el);
            return (link && linkText.length >= 12) || text >= 60;
          })
          .slice(0, 800);

        const groups = new Map<string, Element[]>();
        for (const item of rawItems) {
          const sig = signature(item);
          if (!groups.has(sig)) groups.set(sig, []);
          groups.get(sig)!.push(item);
        }

        const results: Scored[] = [];
        for (const group of groups.values()) {
          if (group.length < 3) continue;
          const ancestor = findCommonAncestor(group);
          if (!ancestor || exclude.has(ancestor)) continue;
          const contained = group.filter(el => ancestor.contains(el));
          const uniqueItems = Array.from(new Set(contained));
          if (uniqueItems.length < 3) continue;

          const containerSelector = cssFor(ancestor);
          const itemSelector = robustItemSelector(ancestor, uniqueItems);
          const widths = uniqueItems.map(el => (el as HTMLElement).getBoundingClientRect().width);
          const heights = uniqueItems.map(el => (el as HTMLElement).getBoundingClientRect().height);
          const sdW = stddev(widths);
          const sdH = stddev(heights);
          const medianText = uniqueItems.map(textLen).sort((a,b)=>a-b)[Math.floor(uniqueItems.length/2)];
          const linkD = anchorDensity(ancestor);
          const priceHits = uniqueItems.reduce((n,el)=>n + (hasPrice(el)?1:0), 0);
          const ratingHits = uniqueItems.reduce((n,el)=>n + (hasRating(el)?1:0), 0);
          const ariaList = ancestor.getAttribute('role') === 'list' ? 1 : 0;
          const navPenalty = (linkD > 0.6 && medianText < 30) ? 1 : 0;
          const carouselPenalty = (() => {
            try {
              const el = ancestor as HTMLElement;
              const cs = getComputedStyle(el);
              const overflowX = (cs.overflowX || '').toLowerCase();
              const scrollableX = overflowX && overflowX !== 'visible' && overflowX !== 'clip';
              const hasAriaCarousel =
                (ancestor.getAttribute('aria-roledescription') || '').toLowerCase() === 'carousel' ||
                !!ancestor.closest('[aria-roledescription="carousel"]');
              const hasScrollableX = scrollableX && (el.scrollWidth > el.clientWidth + 40);
              return (hasAriaCarousel || hasScrollableX) ? 1 : 0;
            } catch {
              return 0;
            }
          })();
          const childCount = Array.from(ancestor.children).filter(visible).length || uniqueItems.length;
          const baseScore = 3.0 * (uniqueItems.length/Math.max(3, childCount)) +
                        1.3 * (1 / (1 + sdW / Math.max(1, widths[0] || 1) + sdH / Math.max(1, heights[0] || 1))) +
                        1.2 * Math.min(1, medianText / 120) +
                        1.0 * (1 - Math.min(1, Math.abs(linkD - 0.35))) +
                        0.4 * Math.min(1, priceHits / uniqueItems.length) +
                        0.3 * Math.min(1, ratingHits / uniqueItems.length) +
                        0.8 * ariaList -
                        1.5 * navPenalty -
                        2.2 * carouselPenalty;
          const metrics = {
            item_count: uniqueItems.length,
            median_text_len: medianText,
            anchor_density: linkD,
            price_hits: priceHits,
            rating_hits: ratingHits,
            aria_list: ariaList,
            nav_penalty: navPenalty,
            carousel_penalty: carouselPenalty,
          };
          const score = applyPurposeBonus(baseScore, metrics);
          results.push({ container: ancestor, items: uniqueItems, containerSelector, itemSelector, score, metrics });
        }
        results.sort((a,b)=>b.score - a.score);
        return results.slice(0, 5);
      };
      function applyPurposeBonus(baseScore: number, metrics: any): number {
        const itemCount = Math.max(1, Number(metrics.item_count) || 1);
        const medianText = Number(metrics.median_text_len) || 0;
        const linkD = Number(metrics.anchor_density) || 0;
        const priceRatio = Math.min(1, (Number(metrics.price_hits) || 0) / itemCount);
        const ratingRatio = Math.min(1, (Number(metrics.rating_hits) || 0) / itemCount);
        const carouselPenalty = Number(metrics.carousel_penalty) || 0;

        if (purpose === 'links' || purpose === 'results') {
          // Prefer link-heavy, commerce-like result lists.
          const bonus =
            0.6 * Math.min(1, linkD / 0.7) +
            0.35 * priceRatio +
            0.2 * ratingRatio -
            0.7 * Math.min(1, medianText / 180);
          return baseScore + bonus;
        }

        if (purpose === 'reviews' || purpose === 'comments') {
          // Prefer dense text lists with rating signals and low price/link density.
          const bonus =
            0.95 * Math.min(1, medianText / 170) +
            0.75 * ratingRatio -
            0.75 * priceRatio -
            0.9 * linkD -
            1.8 * carouselPenalty;
          return baseScore + bonus;
        }

        if (purpose === 'text') {
          // Generic “comment-like” content lists.
          const bonus = 1.0 * Math.min(1, medianText / 220) - 0.9 * linkD;
          return baseScore + bonus;
        }

        return baseScore;
      }
      // XPaths helper (mirrors process_dom)
      const shortText = (el: Element) => (el.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 160);
      const xpathsFor = (el: Element): string[] => {
        const out: string[] = [];
        const id = (el as HTMLElement).id;
        if (id) {
          try { if (document.querySelectorAll(`#${CSS.escape(id)}`).length === 1) out.push(`//*[@id=${JSON.stringify(id)}]`); } catch {}
        }
        const absoluteXPath = (node: Element | null): string => {
          if (!node) return '';
          const parts: string[] = [];
          for (let e: Element | null = node; e && e.nodeType === 1 && e !== document.documentElement; e = e.parentElement) {
            const tag = e.tagName.toLowerCase();
            const siblings = e.parentElement ? Array.from(e.parentElement.children).filter(c => (c as Element).tagName === e.tagName) : [];
            const index = siblings.length > 1 ? `[${siblings.indexOf(e)+1}]` : '';
            parts.unshift(`${tag}${index}`);
          }
          return `/html/${parts.join('/')}`;
        };
        out.push(absoluteXPath(el));
        const text = shortText(el);
        if (text) out.push(`//*[normalize-space(.)=${JSON.stringify(text)}]`);
        return Array.from(new Set(out)).filter(Boolean);
      };

      // 1) Seed containers
      const seeds = [
        'main', '#search', '[role="main"]', '[role="feed"]',
        '[role="list"]', 'ul', 'ol', 'table', 'tbody',
        'section', 'article', 'div[class*="list"]', 'div[class*="result"]',
        'div[class*="items"]', 'div[class*="grid"]',
        'div[id*="table"]', 'div[id*="list"]', 'div[id*="feed"]',
        'div[data-testid*="list"]'
      ].join(',');
      const maxContainers = message.payload?.options?.maxContainers ?? 60;
      const candidates = $$(document, seeds).filter(visible).slice(0, maxContainers);
      type Scored = {
        container: Element;
        items: Element[];
        containerSelector: string;
        itemSelector: string;
        score: number;
        metrics: {
          item_count: number;
          median_text_len: number;
          anchor_density: number;
          price_hits: number;
          rating_hits: number;
          aria_list: number;
          nav_penalty: number;
        };
      };
      const seenContainers = new Set<Element>();
      const scored: Scored[] = [];

      for (const container of candidates) {
        const kids = Array.from(container.children).filter(visible).filter(ch => textLen(ch) + $$(ch, 'img, a').length > 0);
        if (kids.length < 3) continue;
        const groups = new Map<string, Element[]>();
        for (const k of kids) {
          const sig = signature(k);
          (groups.get(sig) || groups.set(sig, []).get(sig)!).push(k);
        }
        const topGroups = Array.from(groups.values()).sort((a,b)=>b.length-a.length);
        const items = topGroups[0] || [];
        if (items.length < 3) continue;

        const widths = items.map(el => (el as HTMLElement).getBoundingClientRect().width);
        const heights = items.map(el => (el as HTMLElement).getBoundingClientRect().height);
        const sdW = stddev(widths), sdH = stddev(heights);
        const medianText = items.map(textLen).sort((a,b)=>a-b)[Math.floor(items.length/2)];
        const linkD = anchorDensity(container);
        const priceHits = items.reduce((n,el)=>n + (hasPrice(el)?1:0), 0);
        const ratingHits = items.reduce((n,el)=>n + (hasRating(el)?1:0), 0);
        const ariaList = container.getAttribute('role') === 'list' ? 1 : 0;
        const navPenalty = (linkD > 0.6 && medianText < 30) ? 1 : 0;

        const baseScore = 3.0 * (items.length/Math.max(3, kids.length)) +
                      1.3 * (1 / (1 + sdW / Math.max(1, widths[0]) + sdH / Math.max(1, heights[0]))) +
                      1.2 * Math.min(1, medianText / 120) +
                      1.0 * (1 - Math.min(1, Math.abs(linkD - 0.35))) +
                      0.4 * Math.min(1, priceHits / items.length) +
                      0.3 * Math.min(1, ratingHits / items.length) +
                      0.8 * ariaList -
                      1.5 * navPenalty -
                      2.2 * (() => {
                        try {
                          const el = container as HTMLElement;
                          const cs = getComputedStyle(el);
                          const overflowX = (cs.overflowX || '').toLowerCase();
                          const scrollableX = overflowX && overflowX !== 'visible' && overflowX !== 'clip';
                          const hasAriaCarousel =
                            (container.getAttribute('aria-roledescription') || '').toLowerCase() === 'carousel' ||
                            !!container.closest('[aria-roledescription="carousel"]');
                          const hasScrollableX = scrollableX && (el.scrollWidth > el.clientWidth + 40);
                          return (hasAriaCarousel || hasScrollableX) ? 1 : 0;
                        } catch {
                          return 0;
                        }
                      })();
        const containerSelector = cssFor(container);
        const itemSelector = robustItemSelector(container, items);
        const carouselPenalty = (() => {
          try {
            const el = container as HTMLElement;
            const cs = getComputedStyle(el);
            const overflowX = (cs.overflowX || '').toLowerCase();
            const scrollableX = overflowX && overflowX !== 'visible' && overflowX !== 'clip';
            const hasAriaCarousel =
              (container.getAttribute('aria-roledescription') || '').toLowerCase() === 'carousel' ||
              !!container.closest('[aria-roledescription="carousel"]');
            const hasScrollableX = scrollableX && (el.scrollWidth > el.clientWidth + 40);
            return (hasAriaCarousel || hasScrollableX) ? 1 : 0;
          } catch {
            return 0;
          }
        })();
        const metrics = {
          item_count: items.length,
          median_text_len: medianText,
          anchor_density: linkD,
          price_hits: priceHits,
          rating_hits: ratingHits,
          aria_list: ariaList,
          nav_penalty: navPenalty,
          carousel_penalty: carouselPenalty,
        };
        const score = applyPurposeBonus(baseScore, metrics);
        scored.push({ container, items, containerSelector, itemSelector, score, metrics });
        seenContainers.add(container);
      }

      if (!scored.length) {
        const fallback = fallbackGlobalLists(seenContainers);
        fallback.forEach(entry => scored.push(entry));
      }
      if (!scored.length) {
        sendResponse({ req_id: message.req_id, success: true, result: null });
        return false;
      }
      scored.sort((a,b)=>b.score - a.score);
      const best = scored[0];
      const itemsOut = best.items.slice(0, 200).map(el => ({ xpaths: xpathsFor(el), href: (el.querySelector('a[href]') as HTMLAnchorElement)?.href, text: shortText(el) }));
      const candidatesOut = scored.slice(0, maxCandidates).map(s => ({
        containerSelector: s.containerSelector,
        itemSelector: s.itemSelector,
        score: s.score,
        metrics: s.metrics,
        items: s.items.slice(0, 40).map(el => ({
          xpaths: xpathsFor(el),
          href: (el.querySelector('a[href]') as HTMLAnchorElement)?.href,
          text: shortText(el),
        })),
      }));
      sendResponse({
        req_id: message.req_id,
        success: true,
        result: {
          containerSelector: best.containerSelector,
          itemSelector: best.itemSelector,
          items: itemsOut,
          candidates: candidatesOut,
          purpose,
        }
      });
      return false;
    } catch (e: any) {
      sendResponse({ req_id: message.req_id, success: false, error: e?.message || String(e) });
      return false;
    }
  }
  
if (message.cmd === 'get_pruned_dom') {
    const options = message.payload?.options || {};
    
    // Use enhanced DOM capture approach
    const domSnapshot = captureEnhancedDOMSnapshot({
      maxElements: options.maxElements || 200,
      highlightElements: false
    });
    
    // For backwards compatibility, use the formatted prompt as HTML
    const html = domSnapshot.prompt || pruneDOM(options);
    
    sendResponse({
      req_id: message.req_id,
      success: true,
      html: html, // HTML with embedded prompt
      dom_snapshot: domSnapshot, // New enhanced format
      dom_hash: domSnapshot.hash
    });
    return false; // Synchronous response
  }
  
if (message.cmd === 'get_dom_snapshot' || message.cmd === CONTENT_SCRIPT_DOM_SNAPSHOT_CMD) {
    const options = message.payload?.options || {};
    
    // Use enhanced DOM capture approach
    const domSnapshot = captureEnhancedDOMSnapshot({
      maxElements: options.maxElements || 200,
      highlightElements: options.highlightElements ?? true
    });
    
    sendResponse({
      req_id: message.req_id,
      success: true,
      dom_snapshot: domSnapshot,
      dom_hash: domSnapshot.hash
    });
  return false; // Synchronous response
}
  
if (message.cmd === 'get_dom_hash') {
    const hash = domHash();
    sendResponse({
      req_id: message.req_id,
      success: true,
      hash: hash
    });
  return false; // Synchronous response
}
});

// Listen for test bridge messages from the page and execute actions
window.addEventListener('message', (event: MessageEvent) => {
  if (!isActiveContentScriptInstance()) return;
  if (event.origin !== window.location.origin) return;

  // In MV3, comparing `event.source` across MAIN/isolated worlds is unreliable (it may be `null`
  // or a different WindowProxy wrapper). Instead, filter on message shape + type.
  const data: any = (event as any).data;
  if (!data || typeof data !== 'object') return;
  if (
    data.type !== 'RZN_TEST_PING' &&
    data.type !== 'RZN_TEST_DOM_SNAPSHOT' &&
    data.type !== 'RZN_TEST_EXECUTE'
  ) {
    return;
  }
  if (!isValidBridgeToken(data.token)) return;

  if (data.type === 'RZN_TEST_PING') {
    postPageBridgeMessage({ type: 'RZN_TEST_PONG', requestId: data.requestId, payload: true });
    return;
  }

  if (data.type === 'RZN_TEST_DOM_SNAPSHOT') {
    try {
      const domSnapshot = captureEnhancedDOMSnapshot({
        maxElements: data.options?.maxElements || 200,
        highlightElements: !!data.options?.highlightElements,
      });
      postPageBridgeMessage({
        type: 'RZN_TEST_DOM_SNAPSHOT_RESULT',
        requestId: data.requestId,
        snapshot: domSnapshot,
      });
    } catch (e: any) {
      postPageBridgeMessage({
        type: 'RZN_TEST_DOM_SNAPSHOT_RESULT',
        requestId: data.requestId,
        error: e?.message || String(e),
      });
    }
  }

  if (data.type === 'RZN_TEST_EXECUTE') {
    (async () => {
      try {
        const response = await executeWithSharedRequestDedup(
          data.requestId,
          stepFingerprint(data.step),
          async () => {
          const step = data.step || {};
          assertPageChannelStepAllowed(step);
          const stepType: string = step.type;
          const enhancedType = `${stepType}_enhanced`;
          const forceLegacy = !!(step && (step as any).force_legacy);

          let handler: any = undefined;
          if (!forceLegacy && (enhancedActionHandlers as any)[enhancedType]) {
            handler = (enhancedActionHandlers as any)[enhancedType];
          } else if (!forceLegacy && (enhancedActionHandlers as any)[stepType]) {
            handler = (enhancedActionHandlers as any)[stepType];
          } else if ((actionHandlers as any)[stepType]) {
            handler = (actionHandlers as any)[stepType];
          }

          if (!handler) {
            throw new Error(`Unknown action type: ${stepType}`);
          }

          const result = await handler(step);
          const actionFailure = actionFailureResponseFields(result);
          const domSnapshot = captureEnhancedDOMSnapshot({ maxElements: 120, highlightElements: false });

          return {
            ...actionFailure,
            result,
            current_url: window.location.href,
            dom_snapshot: domSnapshot,
            dom_hash: domSnapshot.hash,
          };
        });

        postPageBridgeMessage({
          type: 'RZN_TEST_RESULT',
          requestId: data.requestId,
          response,
        });
      } catch (err: any) {
        const domSnapshot = captureEnhancedDOMSnapshot({ maxElements: 120, highlightElements: false });
        postPageBridgeMessage({
          type: 'RZN_TEST_RESULT',
          requestId: data.requestId,
          response: {
            success: false,
            error_msg: err?.message || String(err),
            dom_snapshot: domSnapshot,
            dom_hash: domSnapshot.hash,
          }
        });
      }
    })();
  }
});

// DOM-based test bridge (used by pageBridge.js in MAIN world).
// The pageBridge writes request nodes into a hidden container, and we write responses back as
// attributes on the same node. This avoids relying on cross-world postMessage delivery.
const RZN_DOM_BRIDGE_CONTAINER_ID = '__rzn_page_bridge';

async function handleDomBridgeRequest(node: HTMLElement) {
  if (!isActiveContentScriptInstance()) return;
  if (node.hasAttribute('data-rzn-resp') || node.hasAttribute('data-rzn-err')) return;
  // Requests targeted at the page (data-rzn-target='page') belong to pageBridge.js
  // running in MAIN world. Don't touch them or we race with pageBridge and
  // erroneously set data-rzn-err: "Unknown bridge request type".
  const target = node.getAttribute('data-rzn-target');
  if (target && target !== 'content') return;
  if (node.getAttribute(CONTENT_SCRIPT_DOM_OWNER_ATTR) !== CONTENT_SCRIPT_INSTANCE_ID) {
    node.setAttribute('data-rzn-err', 'Unauthorized bridge request owner');
    return;
  }
  if (!isValidBridgeToken(node.getAttribute(CONTENT_SCRIPT_BRIDGE_TOKEN_ATTR))) {
    node.setAttribute('data-rzn-err', 'Unauthorized bridge request token');
    return;
  }
  const type = node.getAttribute('data-rzn-type') || '';

  let payload: any = {};
  try {
    payload = JSON.parse(node.textContent || '{}');
  } catch {
    payload = {};
  }

  try {
    if (type === 'dom_snapshot') {
      const domSnapshot = captureEnhancedDOMSnapshot({
        maxElements: payload?.options?.maxElements || 200,
        highlightElements: !!payload?.options?.highlightElements,
      });
      node.setAttribute('data-rzn-resp', JSON.stringify(domSnapshot));
      return;
    }

    if (type === 'execute') {
      assertPageChannelStepAllowed(payload?.step);
      const response = await executeWithSharedRequestDedup(
        node.getAttribute('data-rzn-req-id') || undefined,
        stepFingerprint(payload?.step),
        async () => {
          const step = payload?.step || {};
          const stepType: string = step.type;
          const enhancedType = `${stepType}_enhanced`;
          const forceLegacy = !!(step && (step as any).force_legacy);

          let handler: any = undefined;
          if (!forceLegacy && (enhancedActionHandlers as any)[enhancedType]) {
            handler = (enhancedActionHandlers as any)[enhancedType];
          } else if (!forceLegacy && (enhancedActionHandlers as any)[stepType]) {
            handler = (enhancedActionHandlers as any)[stepType];
          } else if ((actionHandlers as any)[stepType]) {
            handler = (actionHandlers as any)[stepType];
          }

          if (!handler) {
            throw new Error(`Unknown action type: ${stepType}`);
          }

          const result = await handler(step);
          const domSnapshot = captureEnhancedDOMSnapshot({ maxElements: 120, highlightElements: false });

          return {
            success: true,
            result: bridgeResultPayload(result),
            current_url: window.location.href,
            dom_snapshot: domSnapshot,
            dom_hash: domSnapshot.hash,
          };
        }
      );

      node.setAttribute('data-rzn-resp', JSON.stringify(response));
      return;
    }

    node.setAttribute('data-rzn-err', `Unknown bridge request type: ${type}`);
  } catch (e: any) {
    try {
      const domSnapshot = captureEnhancedDOMSnapshot({ maxElements: 120, highlightElements: false });
      node.setAttribute(
        'data-rzn-resp',
        JSON.stringify({
          success: false,
          error_msg: e?.message || String(e),
          dom_snapshot: domSnapshot,
          dom_hash: domSnapshot.hash,
        })
      );
    } catch {
      node.setAttribute('data-rzn-err', e?.message || String(e));
    }
  }
}

function attachDomBridgeObserver(container: HTMLElement) {
  const obs = new MutationObserver((mutations) => {
    for (const m of mutations) {
      for (const n of Array.from(m.addedNodes)) {
        if (n instanceof HTMLElement && n.hasAttribute('data-rzn-req-id')) {
          handleDomBridgeRequest(n);
        }
      }
    }
  });
  obs.observe(container, { childList: true });

  // Process anything already queued.
  for (const child of Array.from(container.children)) {
    if (child instanceof HTMLElement && child.hasAttribute('data-rzn-req-id')) {
      handleDomBridgeRequest(child);
    }
  }
}

(() => {
  const existing = document.getElementById(RZN_DOM_BRIDGE_CONTAINER_ID);
  if (existing) {
    installContentBridgeMetadata(existing as HTMLElement);
    attachDomBridgeObserver(existing as HTMLElement);
    return;
  }

  const root = document.documentElement;
  if (!root) return;

  const obs = new MutationObserver(() => {
    const el = document.getElementById(RZN_DOM_BRIDGE_CONTAINER_ID);
    if (el) {
      obs.disconnect();
      installContentBridgeMetadata(el as HTMLElement);
      attachDomBridgeObserver(el as HTMLElement);
    }
  });
  obs.observe(root, { childList: true, subtree: true });
})();

let lastNativeWakeMs = 0;
const NATIVE_WAKE_THROTTLE_MS = 10_000;
const NATIVE_KEEPALIVE_PORT_NAME = 'rzn_content_keepalive';
const NATIVE_KEEPALIVE_INTERVAL_MS = 15_000;
const NATIVE_KEEPALIVE_RECONNECT_MS = 1_500;
let nativeKeepalivePort: chrome.runtime.Port | null = null;
let nativeKeepaliveTimer: ReturnType<typeof setInterval> | null = null;
let nativeKeepaliveReconnectTimer: ReturnType<typeof setTimeout> | null = null;
let nativeKeepalivePausedForPageLifecycle = false;

function isTopLevelFrame(): boolean {
  try {
    return window.top === window;
  } catch {
    return false;
  }
}

function nativeWakePayload(reason: string): Record<string, unknown> {
  return {
    type: 'RZN_WAKE_NATIVE',
    reason,
    build: RZN_BUILD_SIGNATURE,
    url: window.location.origin,
  };
}

function wakeNativeHost(reason: string): void {
  if (!isTopLevelFrame()) return;
  const now = Date.now();
  if (now - lastNativeWakeMs < NATIVE_WAKE_THROTTLE_MS) return;
  lastNativeWakeMs = now;
  try {
    chrome.runtime.sendMessage(nativeWakePayload(reason)).catch(() => {});
  } catch {}
}

function clearNativeKeepaliveTimer(): void {
  if (nativeKeepaliveTimer) {
    clearInterval(nativeKeepaliveTimer);
    nativeKeepaliveTimer = null;
  }
}

function clearNativeKeepaliveReconnectTimer(): void {
  if (nativeKeepaliveReconnectTimer) {
    clearTimeout(nativeKeepaliveReconnectTimer);
    nativeKeepaliveReconnectTimer = null;
  }
}

function runtimeLastErrorMessage(): string | undefined {
  try {
    return chrome.runtime?.lastError?.message;
  } catch {
    return undefined;
  }
}

function scheduleNativeKeepaliveReconnect(): void {
  if (!isTopLevelFrame()) return;
  if (nativeKeepalivePausedForPageLifecycle) return;
  if (nativeKeepaliveReconnectTimer) return;
  nativeKeepaliveReconnectTimer = setTimeout(() => {
    nativeKeepaliveReconnectTimer = null;
    connectNativeKeepalivePort('port_reconnect');
  }, NATIVE_KEEPALIVE_RECONNECT_MS);
}

function disconnectNativeKeepalivePort(_reason: string): void {
  const port = nativeKeepalivePort;
  nativeKeepalivePort = null;
  clearNativeKeepaliveTimer();
  clearNativeKeepaliveReconnectTimer();
  if (!port) return;
  try {
    port.disconnect();
  } catch {}
}

function sendNativeKeepalive(reason: string): void {
  const port = nativeKeepalivePort;
  if (!port) return;
  try {
    port.postMessage({
      type: 'RZN_CONTENT_KEEPALIVE',
      reason,
      build: RZN_BUILD_SIGNATURE,
      url: window.location.origin,
      visibilityState: document.visibilityState,
      ts: Date.now(),
    });
  } catch {
    nativeKeepalivePort = null;
    clearNativeKeepaliveTimer();
    scheduleNativeKeepaliveReconnect();
  }
}

function connectNativeKeepalivePort(reason: string): void {
  if (!isTopLevelFrame()) return;
  if (nativeKeepalivePausedForPageLifecycle) return;
  if (nativeKeepalivePort) {
    sendNativeKeepalive(reason);
    return;
  }

  try {
    const port = chrome.runtime.connect({ name: NATIVE_KEEPALIVE_PORT_NAME });
    nativeKeepalivePort = port;

    port.onDisconnect.addListener(() => {
      const err = runtimeLastErrorMessage();
      nativeKeepalivePort = null;
      clearNativeKeepaliveTimer();
      if (err?.toLowerCase().includes('back/forward cache')) {
        nativeKeepalivePausedForPageLifecycle = true;
        return;
      }
      scheduleNativeKeepaliveReconnect();
    });

    port.onMessage.addListener((message) => {
      if (message?.type === 'RZN_CONTENT_KEEPALIVE_ACK') {
        // Ack is intentionally diagnostic-only; the background owns native state.
      }
    });

    sendNativeKeepalive(reason);
    clearNativeKeepaliveTimer();
    nativeKeepaliveTimer = setInterval(
      () => sendNativeKeepalive('port_heartbeat'),
      NATIVE_KEEPALIVE_INTERVAL_MS
    );
  } catch {
    nativeKeepalivePort = null;
    clearNativeKeepaliveTimer();
    scheduleNativeKeepaliveReconnect();
  }
}

wakeNativeHost('content_script_loaded');
connectNativeKeepalivePort('content_script_loaded');
window.addEventListener('focus', () => wakeNativeHost('window_focus'), { passive: true });
window.addEventListener(
  'focus',
  () => connectNativeKeepalivePort('window_focus'),
  { passive: true }
);
document.addEventListener(
  'visibilitychange',
  () => {
    if (document.visibilityState === 'visible') {
      wakeNativeHost('visibility_visible');
      connectNativeKeepalivePort('visibility_visible');
    }
  },
  { passive: true }
);
window.addEventListener(
  'pagehide',
  () => {
    nativeKeepalivePausedForPageLifecycle = true;
    disconnectNativeKeepalivePort('pagehide');
  },
  { passive: true }
);
window.addEventListener(
  'pageshow',
  () => {
    nativeKeepalivePausedForPageLifecycle = false;
    wakeNativeHost('pageshow');
    connectNativeKeepalivePort('pageshow');
  },
  { passive: true }
);

// DOM function injection removed - enhanced actions handle element resolution directly

console.log('RZN Content Script loaded with DOM function injection');
