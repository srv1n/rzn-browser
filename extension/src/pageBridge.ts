// Page-World Bridge: exposes simple helpers on window for tests and tooling.
// Runs in MAIN world to bypass isolated world boundaries and CSP for inline code.
//
// In MV3, MAIN-world scripts don't have access to extension APIs (chrome.runtime, etc). We also
// avoid relying on `window.postMessage` across worlds for the test harness because it can be flaky.
// Instead, we use a tiny DOM-based bridge: pageBridge writes requests into a hidden DOM node, and
// the isolated-world content script consumes them and writes responses back.

import { RZN_BUILD_SIGNATURE, RZN_PAGE_TEST_BRIDGE_ENABLED } from './buildInfo';

declare global {
  interface Window {
    __rznExecuteStep?: (step: any) => Promise<any>;
    captureEnhancedDOMSnapshot?: (opts?: any) => Promise<any>;
    __rznEvalMainWorld?: (payload: any) => Promise<any>;
    __rznEvalIsolatedWorld?: (payload: any) => Promise<any>;
    __rznInspectElement?: (payload: any) => Promise<any>;
    __rznInspectClickSurface?: (payload: any) => Promise<any>;
    __rznCaptureUiBundle?: (payload?: any) => Promise<any>;
    __rznVerifyUiChange?: (payload?: any) => Promise<any>;
    __rznReadFieldValue?: (payload: any) => Promise<any>;
    __rznBuildInfo?: any;
  }
}

const BRIDGE_CONTAINER_ID = '__rzn_page_bridge';
const BRIDGE_TOKEN_ATTR = 'data-rzn-bridge-token';
const DOM_OWNER_ATTR = 'data-rzn-owner';

function ensureBridgeContainer(): HTMLElement {
  let el = document.getElementById(BRIDGE_CONTAINER_ID) as HTMLElement | null;
  if (!el) {
    el = document.createElement('div');
    el.id = BRIDGE_CONTAINER_ID;
    el.style.display = 'none';
    (document.documentElement || document.body || document.head).appendChild(el);
  }
  return el;
}

function contentOwnerId(container: HTMLElement): string | null {
  const value = container.getAttribute('data-rzn-content-instance');
  return value && value.trim().length > 0 ? value : null;
}

function contentBridgeToken(container: HTMLElement): string | null {
  const value = container.getAttribute(BRIDGE_TOKEN_ATTR);
  return value && value.trim().length > 0 ? value : null;
}

function hasContentBridgeMetadata(container: HTMLElement): boolean {
  return !!contentOwnerId(container) && !!contentBridgeToken(container);
}

function waitForContentBridgeMetadata(container: HTMLElement, timeoutMs: number): Promise<void> {
  if (hasContentBridgeMetadata(container)) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const finish = (error?: Error) => {
      observer.disconnect();
      clearTimeout(timer);
      if (error) reject(error);
      else resolve();
    };
    const observer = new MutationObserver(() => {
      if (hasContentBridgeMetadata(container)) finish();
    });
    observer.observe(container, {
      attributes: true,
      attributeFilter: ['data-rzn-content-instance', BRIDGE_TOKEN_ATTR],
    });
    const timer = setTimeout(
      () => finish(new Error('RZN content bridge is not ready')),
      Math.max(250, Math.min(timeoutMs, 3000))
    );
  });
}

async function sendDomRequest(
  type: string,
  payload: any,
  timeoutMs = 10000,
  target: 'content' | 'page' = 'content'
): Promise<any> {
  const container = ensureBridgeContainer();
  if (target === 'content') {
    await waitForContentBridgeMetadata(container, timeoutMs);
  }
  const requestId = `${type}_${Math.random().toString(36).slice(2)}`;

  const node = document.createElement('div');
  node.setAttribute('data-rzn-req-id', requestId);
  node.setAttribute('data-rzn-type', type);
  node.setAttribute('data-rzn-target', target);
  const owner = contentOwnerId(container);
  if (owner) {
    node.setAttribute(DOM_OWNER_ATTR, owner);
  }
  if (target === 'content') {
    const token = contentBridgeToken(container);
    if (token) {
      node.setAttribute(BRIDGE_TOKEN_ATTR, token);
    }
  }
  node.textContent = JSON.stringify(payload ?? {});
  container.appendChild(node);

  return new Promise((resolve, reject) => {
    const finish = (err: Error | null, value?: any) => {
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
    observer.observe(node, { attributes: true, attributeFilter: ['data-rzn-resp', 'data-rzn-err'] });

    const timer = setTimeout(() => finish(new Error('RZN pageBridge timeout')), timeoutMs);

    // In case the response is set before the observer runs.
    check();
  });
}

function summarizeElementForBridge(value: any): any {
  if (!(value instanceof Element)) return null;
  const text = (value.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 160);
  const attrs: Record<string, string> = {};
  for (const attr of Array.from(value.attributes || [])) {
    if (
      [
        'id',
        'class',
        'name',
        'type',
        'role',
        'href',
        'target',
        'aria-label',
        'data-testid',
      ].includes(attr.name)
    ) {
      attrs[attr.name] = attr.value;
    }
  }
  const rect = value instanceof HTMLElement ? value.getBoundingClientRect() : null;
  return {
    tag: value.tagName.toLowerCase(),
    text,
    attributes: attrs,
    visible:
      value instanceof HTMLElement
        ? rect !== null &&
          rect.width > 0 &&
          rect.height > 0 &&
          getComputedStyle(value).display !== 'none' &&
          getComputedStyle(value).visibility !== 'hidden'
        : false,
    rect: rect
      ? {
          x: Math.round(rect.x),
          y: Math.round(rect.y),
          width: Math.round(rect.width),
          height: Math.round(rect.height),
        }
      : null,
  };
}

function serializeForBridge(value: any, depth = 0, seen = new WeakSet<object>()): any {
  if (value == null || typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
    return value;
  }
  if (typeof value === 'bigint') return value.toString();
  if (typeof value === 'function') return `[Function ${value.name || 'anonymous'}]`;
  if (value instanceof Element) return summarizeElementForBridge(value);
  if (value instanceof Error) {
    return { name: value.name, message: value.message, stack: value.stack };
  }
  if (depth >= 4) {
    if (Array.isArray(value)) return `[Array(${value.length})]`;
    return '[Object]';
  }
  if (Array.isArray(value)) {
    return value.slice(0, 50).map(item => serializeForBridge(item, depth + 1, seen));
  }
  if (typeof value === 'object') {
    if (seen.has(value)) return '[Circular]';
    seen.add(value);
    const out: Record<string, any> = {};
    for (const [key, child] of Object.entries(value).slice(0, 50)) {
      out[key] = serializeForBridge(child, depth + 1, seen);
    }
    return out;
  }
  return String(value);
}

function isCspEvalError(err: any): boolean {
  const msg = (err && (err.message || String(err))) || '';
  return /unsafe-eval|EvalError|Content Security Policy/i.test(msg);
}

// Chrome strips the `nonce` HTML attribute from the DOM after parsing for
// security, but exposes the value via the `.nonce` IDL property to scripts in
// the same world. So `querySelector('script[nonce]')` returns nothing, but
// iterating all scripts and reading `.nonce` finds the page's allowed nonce.
function findPageScriptNonce(): string | null {
  const scripts = document.querySelectorAll('script');
  for (const s of Array.from(scripts) as HTMLScriptElement[]) {
    if (s.nonce) return s.nonce;
  }
  return null;
}

async function runViaScriptTag(
  functionBody: string,
  args: any[],
  params: Record<string, any>,
  returnValue: boolean,
  nonce: string,
  timeoutMs = 30_000
): Promise<any> {
  const channelId = '__rzn_eval_' + Math.random().toString(36).slice(2);
  // Each window slot must be reachable by the injected script when it runs.
  // Prepend an early sentinel so we know the script actually executed (i.e.
  // wasn't silently blocked by CSP) — if `__started` never flips we can fail
  // fast instead of waiting for the full timeout.
  const wrapped = `(function(){
    "use strict";
    try { window['${channelId}_started'] = true; } catch (e) {}
    return (async function(args, params){
      const __args = Array.isArray(args) ? args : [];
      const __rzn_params = params && typeof params === 'object' ? params : {};
      const arg0 = __args[0]; const arg1 = __args[1]; const arg2 = __args[2];
      const arg3 = __args[3]; const arg4 = __args[4]; const arg5 = __args[5];
      const arg6 = __args[6]; const arg7 = __args[7]; const arg8 = __args[8];
      const arg9 = __args[9];
      const previousParams = window.__rzn_params;
      window.__rzn_params = __rzn_params;
      try {
        return await (async () => {
          ${functionBody}
        })();
      } finally {
        if (typeof previousParams === 'undefined') {
          try { delete window.__rzn_params; } catch {}
        } else {
          window.__rzn_params = previousParams;
        }
      }
    })(window['${channelId}_args'], window['${channelId}_params']).then(
      function(v){
        var fn = window['${channelId}_resolve'];
        if (fn) fn(v);
      },
      function(e){
        var fn = window['${channelId}_reject'];
        var err = e instanceof Error ? { name: e.name, message: e.message, stack: e.stack } : e;
        if (fn) fn(err);
      }
    );
  })();`;

  return await new Promise<any>((resolve, reject) => {
    let timer: any;
    let earlyCheck: any;
    const cleanup = () => {
      clearTimeout(timer);
      clearTimeout(earlyCheck);
      try { delete (window as any)[channelId + '_args']; } catch {}
      try { delete (window as any)[channelId + '_params']; } catch {}
      try { delete (window as any)[channelId + '_resolve']; } catch {}
      try { delete (window as any)[channelId + '_reject']; } catch {}
      try { delete (window as any)[channelId + '_started']; } catch {}
    };

    (window as any)[channelId + '_args'] = args;
    (window as any)[channelId + '_params'] = params;
    (window as any)[channelId + '_resolve'] = (v: any) => {
      cleanup();
      resolve(returnValue === false ? null : serializeForBridge(v));
    };
    (window as any)[channelId + '_reject'] = (e: any) => {
      cleanup();
      const message = e && typeof e === 'object' && e.message ? e.message : String(e);
      const err = new Error(message);
      if (e && typeof e === 'object' && e.name) err.name = e.name;
      reject(err);
    };

    const s = document.createElement('script');
    // Setting via the IDL property is what Chrome's CSP checks; setAttribute
    // sets the HTML attribute which is cleared post-parse and ignored.
    (s as HTMLScriptElement).nonce = nonce;
    s.textContent = wrapped;

    timer = setTimeout(() => {
      cleanup();
      try { s.remove(); } catch {}
      reject(new Error('Script-tag eval timeout'));
    }, timeoutMs);

    // If the script never starts within 200ms it was almost certainly blocked
    // by CSP — fail fast instead of waiting the full timeoutMs.
    earlyCheck = setTimeout(() => {
      if (!(window as any)[channelId + '_started']) {
        cleanup();
        try { s.remove(); } catch {}
        reject(new Error('Script-tag injection blocked (likely CSP nonce mismatch)'));
      }
    }, 200);

    const root = document.head || document.documentElement || document.body;
    if (!root) {
      cleanup();
      reject(new Error('No DOM root for script-tag injection'));
      return;
    }
    root.appendChild(s);
    // The inline script executes synchronously on append (modulo CSP), so it's
    // safe to remove the element from the DOM right after — the kicked-off
    // async work continues without it.
    try { s.remove(); } catch {}
  });
}

async function runMainWorldScript(
  script: string,
  args: any[] = [],
  params: Record<string, any> = {},
  returnValue = true
): Promise<any> {
  const source = String(script || '');
  const trimmed = source.trim();
  const functionBody =
    trimmed.includes('return ') ||
    /(^|[\s;])(?:const|let|var|if|for|while|throw|try|await)\b/.test(trimmed) ||
    trimmed.includes(';') ||
    trimmed.includes('\n')
      ? source
      : `return (${source});`;
  // Fast path: AsyncFunction. Subject to page CSP `unsafe-eval`.
  try {
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
          ${functionBody}
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
    return returnValue === false ? null : serializeForBridge(value);
  } catch (err) {
    if (!isCspEvalError(err)) throw err;
    // Page CSP blocks `unsafe-eval` (e.g. chatgpt.com). Fall back to a
    // <script nonce=...> injection — script tags whose nonce matches the page
    // policy execute even when `unsafe-eval` is forbidden.
  }

  const nonce = findPageScriptNonce();
  if (!nonce) {
    throw new Error('CSP blocks unsafe-eval and no script[nonce] available on the page');
  }
  return await runViaScriptTag(functionBody, args, params, returnValue, nonce);
}

async function fillAndSubmitInMainWorld(payload: any): Promise<any> {
  const selector = String(payload?.selector || '').trim();
  const value = String(payload?.value ?? payload?.text ?? '');
  if (!selector) throw new Error('Missing selector for fill_and_submit');
  if (!value) throw new Error('Missing value for fill_and_submit');

  const timeoutMs = Math.max(500, Number(payload?.timeout_ms ?? payload?.timeoutMs ?? 10000));
  const waitTimeoutMs = Math.max(0, Number(payload?.wait_timeout_ms ?? payload?.waitTimeoutMs ?? 15000));
  const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
  const normalize = (text: string | null | undefined) =>
    String(text || '').replace(/\u00a0/g, ' ').replace(/\s+/g, ' ').trim();
  const isVisible = (element: Element | null) => {
    if (!(element instanceof Element)) return false;
    const style = getComputedStyle(element);
    if (style.display === 'none' || style.visibility === 'hidden') return false;
    const rect = element.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  };
  const readText = (target: Element | null): string => {
    if (!target) return '';
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) return String(target.value || '');
    if (target instanceof HTMLElement) return normalize(target.innerText || target.textContent || '');
    return normalize(target.textContent || '');
  };
  const dispatchInput = (target: HTMLElement, data: string | null, inputType: string) => {
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
  const setText = (target: Element, text: string) => {
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
      target.focus();
      const proto = target instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
      if (setter) setter.call(target, text);
      else target.value = text;
      target.dispatchEvent(new InputEvent('input', {
        bubbles: true,
        cancelable: true,
        composed: true,
        data: text,
        inputType: 'insertReplacementText'
      }));
      target.dispatchEvent(new Event('change', { bubbles: true, cancelable: true }));
      return;
    }

    if (target instanceof HTMLElement && (target.isContentEditable || target.getAttribute('contenteditable'))) {
      target.focus();
      try {
        const selection = getSelection();
        const range = document.createRange();
        range.selectNodeContents(target);
        selection?.removeAllRanges();
        selection?.addRange(range);
        document.execCommand('delete');
        const inserted = document.execCommand('insertText', false, text);
        if (inserted && normalize(readText(target)).includes(normalize(text))) {
          dispatchInput(target, text, 'insertText');
          return;
        }
      } catch {}

      const paragraph = document.createElement('p');
      paragraph.textContent = text;
      target.replaceChildren(paragraph);
      dispatchInput(target, text, 'insertReplacementText');
    }
  };
  const findTarget = () => Array.from(document.querySelectorAll(selector)).find(isVisible) || null;
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
    if (!(target instanceof HTMLElement) || !isVisible(target)) return false;
    if (target.matches(':disabled') || target.getAttribute('aria-disabled') === 'true') return false;
    return true;
  };
  const findSubmitButton = (target: Element | null): HTMLElement | null => {
    const explicitSelector = String(payload?.submit_selector || payload?.submitSelector || '').trim();
    if (explicitSelector) {
      const explicit =
        document.querySelector(explicitSelector) ||
        target?.closest('form')?.querySelector(explicitSelector);
      if (explicit instanceof HTMLElement && isEnabledButton(explicit)) return explicit;
    }
    const labelRegex = new RegExp(String(payload?.submit_label_regex || payload?.submitLabelRegex || 'send|submit'), 'i');
    const scopes = [target?.closest('form'), target?.parentElement, target?.parentElement?.parentElement, document].filter(Boolean) as ParentNode[];
    const seen = new Set<Element>();
    for (const scope of scopes) {
      for (const candidate of Array.from(scope.querySelectorAll("button, [role='button'], input[type='submit'], input[type='button']"))) {
        if (seen.has(candidate)) continue;
        seen.add(candidate);
        if (isEnabledButton(candidate) && labelRegex.test(labelFor(candidate))) return candidate as HTMLElement;
      }
    }
    return null;
  };
  const waitForIncrease = async (selectorRaw: string, before: number | null) => {
    if (!selectorRaw || before === null) return { increased: false, count_after: before };
    const deadline = Date.now() + waitTimeoutMs;
    let countAfter = before;
    while (Date.now() < deadline) {
      countAfter = document.querySelectorAll(selectorRaw).length;
      if (countAfter > before) return { increased: true, count_after: countAfter };
      await sleep(400);
    }
    return { increased: false, count_after: countAfter };
  };
  const pressEnter = (target: Element | null) => {
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
    eventTarget.dispatchEvent(new KeyboardEvent('keydown', init));
    eventTarget.dispatchEvent(new KeyboardEvent('keypress', init));
    eventTarget.dispatchEvent(new KeyboardEvent('keyup', init));
    return true;
  };

  let target = findTarget();
  if (!target) throw new Error('Target not found for fill_and_submit');
  const increaseSelector = String(payload?.wait_for_increase_selector || payload?.waitForIncreaseSelector || '').trim();
  const countBefore = increaseSelector ? document.querySelectorAll(increaseSelector).length : null;

  setText(target, value);
  await sleep(250);
  target = findTarget() || target;
  const textConfirmed = normalize(readText(target)).includes(normalize(value));

  const buttonDeadline = Date.now() + timeoutMs;
  let submitButton: HTMLElement | null = null;
  while (Date.now() < buttonDeadline) {
    submitButton = findSubmitButton(target);
    if (submitButton) break;
    await sleep(150);
  }

  if (submitButton) {
    submitButton.scrollIntoView?.({ block: 'center', inline: 'center' });
    submitButton.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true, composed: true, view: window }));
    submitButton.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, cancelable: true, composed: true, view: window }));
    submitButton.click();
    const waited = await waitForIncrease(increaseSelector, countBefore);
    return { submitted: true, submit_method: 'button_click', filled: textConfirmed, count_before: countBefore, ...waited };
  }

  const keyboardSubmitted = pressEnter(target);
  const waited = await waitForIncrease(increaseSelector, countBefore);
  if (waited.increased) {
    return { submitted: true, submit_method: 'keyboard_enter', keyboard_submitted: keyboardSubmitted, filled: textConfirmed, count_before: countBefore, ...waited };
  }

  const visibleButtons = Array.from(document.querySelectorAll("button, [role='button'], input[type='submit'], input[type='button']"))
    .filter(candidate => candidate instanceof HTMLElement && isVisible(candidate))
    .slice(-20)
    .map(candidate => ({
      label: labelFor(candidate).slice(0, 120),
      disabled: candidate.matches(':disabled'),
      aria_disabled: candidate.getAttribute('aria-disabled'),
      tag: candidate.tagName.toLowerCase()
    }));
  throw new Error(`No enabled submit button found for fill_and_submit: ${JSON.stringify({
    text_confirmed: textConfirmed,
    target_text: normalize(readText(target)).slice(0, 120),
    visible_buttons: visibleButtons
  })}`);
}

function handlePageRequestNode(node: HTMLElement) {
  const target = node.getAttribute('data-rzn-target');
  if (target !== 'page' || node.hasAttribute('data-rzn-resp') || node.hasAttribute('data-rzn-err')) {
    return;
  }
  const owner = contentOwnerId(ensureBridgeContainer());
  if (!owner || node.getAttribute(DOM_OWNER_ATTR) !== owner) {
    return;
  }

  const type = node.getAttribute('data-rzn-type') || '';
  const payload = (() => {
    try {
      return JSON.parse(node.textContent || '{}');
    } catch {
      return {};
    }
  })();

  (async () => {
    if (type === 'eval_main_world') {
      const result = await runMainWorldScript(
        String(payload?.script || ''),
        Array.isArray(payload?.args) ? payload.args : [],
        payload?.params && typeof payload.params === 'object' && !Array.isArray(payload.params)
          ? payload.params
          : {},
        payload?.return_value !== false
      );
      node.setAttribute('data-rzn-resp', JSON.stringify({ success: true, result }));
      return;
    }

    if (type === 'native_click') {
      const selector = String(payload?.selector || '').trim();
      if (!selector) {
        node.setAttribute('data-rzn-resp', JSON.stringify({
          success: true,
          result: { clicked: false, reason: 'missing_selector' },
        }));
        return;
      }

      const target = document.querySelector(selector);
      if (!(target instanceof HTMLElement)) {
        node.setAttribute('data-rzn-resp', JSON.stringify({
          success: true,
          result: { clicked: false, reason: 'not_found', selector },
        }));
        return;
      }

      try {
        target.scrollIntoView?.({ block: 'center', inline: 'center' });
      } catch {}

      if (typeof target.click === 'function') {
        target.click();
      } else {
        target.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, composed: true }));
      }

      node.setAttribute('data-rzn-resp', JSON.stringify({
        success: true,
        result: { clicked: true, selector, tag: target.tagName.toLowerCase() },
      }));
      return;
    }

    if (type === 'fill_and_submit') {
      const result = await fillAndSubmitInMainWorld(payload);
      node.setAttribute('data-rzn-resp', JSON.stringify({ success: true, result }));
      return;
    }

    node.setAttribute('data-rzn-err', `Unknown page bridge request type: ${type}`);
  })().catch((error: any) => {
    node.setAttribute('data-rzn-err', error?.message || String(error));
  });
}

function attachPageBridgeObserver() {
  const container = ensureBridgeContainer();
  const observer = new MutationObserver((mutations) => {
    for (const mutation of mutations) {
      for (const added of Array.from(mutation.addedNodes)) {
        if (added instanceof HTMLElement && added.hasAttribute('data-rzn-req-id')) {
          handlePageRequestNode(added);
        }
      }
    }
  });
  observer.observe(container, { childList: true });

  for (const child of Array.from(container.children)) {
    if (child instanceof HTMLElement && child.hasAttribute('data-rzn-req-id')) {
      handlePageRequestNode(child);
    }
  }
}

try {
  attachPageBridgeObserver();

  window.__rznBuildInfo = {
    ...(window.__rznBuildInfo || {}),
    pageBridge: {
      signature: RZN_BUILD_SIGNATURE,
      testBridgeEnabled: RZN_PAGE_TEST_BRIDGE_ENABLED,
    },
  };

  if (RZN_PAGE_TEST_BRIDGE_ENABLED) {
  if (typeof window.__rznExecuteStep !== 'function') {
    Object.defineProperty(window, '__rznExecuteStep', {
      value: (step: any) => {
        const timeoutMs = Math.max(1000, Number(step?.timeout_ms ?? step?.timeoutMs ?? 10000));
        return sendDomRequest('execute', { step }, timeoutMs);
      },
      configurable: false,
      writable: false,
    });
  }

  if (typeof window.captureEnhancedDOMSnapshot !== 'function') {
    Object.defineProperty(window, 'captureEnhancedDOMSnapshot', {
      value: (options?: any) => {
        return sendDomRequest('dom_snapshot', { options }, 10000);
      },
      configurable: false,
      writable: false,
    });
  }

  if (typeof window.__rznEvalMainWorld !== 'function') {
    Object.defineProperty(window, '__rznEvalMainWorld', {
      value: (payload: any) =>
        sendDomRequest(
          'execute',
          { step: { type: 'eval_main_world', ...(payload || {}) } },
          Math.max(1000, Number(payload?.timeout_ms || 30000))
        ),
      configurable: false,
      writable: false,
    });
  }

  if (typeof window.__rznEvalIsolatedWorld !== 'function') {
    Object.defineProperty(window, '__rznEvalIsolatedWorld', {
      value: (payload: any) =>
        sendDomRequest(
          'execute',
          { step: { type: 'eval_isolated_world', ...(payload || {}) } },
          Math.max(1000, Number(payload?.timeout_ms || 30000))
        ),
      configurable: false,
      writable: false,
    });
  }

  if (typeof window.__rznInspectElement !== 'function') {
    Object.defineProperty(window, '__rznInspectElement', {
      value: (payload: any) =>
        sendDomRequest(
          'execute',
          { step: { type: 'inspect_element', ...(payload || {}) } },
          Math.max(1000, Number(payload?.timeout_ms || 15000))
        ),
      configurable: false,
      writable: false,
    });
  }

  if (typeof window.__rznInspectClickSurface !== 'function') {
    Object.defineProperty(window, '__rznInspectClickSurface', {
      value: (payload: any) =>
        sendDomRequest(
          'execute',
          { step: { type: 'inspect_click_surface', ...(payload || {}) } },
          Math.max(1000, Number(payload?.timeout_ms || 15000))
        ),
      configurable: false,
      writable: false,
    });
  }

  if (typeof window.__rznCaptureUiBundle !== 'function') {
    Object.defineProperty(window, '__rznCaptureUiBundle', {
      value: (payload: any) =>
        sendDomRequest(
          'execute',
          { step: { type: 'capture_ui_bundle', ...(payload || {}) } },
          Math.max(1000, Number(payload?.timeout_ms || 15000))
        ),
      configurable: false,
      writable: false,
    });
  }

  if (typeof window.__rznVerifyUiChange !== 'function') {
    Object.defineProperty(window, '__rznVerifyUiChange', {
      value: (payload: any) =>
        sendDomRequest(
          'execute',
          { step: { type: 'verify_ui_change', ...(payload || {}) } },
          Math.max(1000, Number(payload?.timeout_ms || 15000))
        ),
      configurable: false,
      writable: false,
    });
  }

  if (typeof window.__rznReadFieldValue !== 'function') {
    Object.defineProperty(window, '__rznReadFieldValue', {
      value: (payload: any) =>
        sendDomRequest(
          'execute',
          { step: { type: 'read_field_value', ...(payload || {}) } },
          Math.max(1000, Number(payload?.timeout_ms || 15000))
        ),
      configurable: false,
      writable: false,
    });
  }
  }
} catch (e) {
  // Silently ignore; page may have aggressive CSP, but MAIN world helps
}

export {};
