/**
 * First-class click_element using CDP for "trusted" input.
 *
 * Some modern web apps ignore synthetic DOM events (isTrusted=false) for critical
 * UI actions. This helper uses chrome.debugger + Input.dispatchMouseEvent to
 * generate a real click at the element's center.
 *
 * This is intended as an explicit break-glass path (opt-in per step).
 */

import { cdpSessionManager } from '../runtime/cdp_session_manager';
import { getActiveBrowserTabId } from '../browserTabs';
import { actionSuccess } from './actionResult';

type Rect = { left: number; top: number; width: number; height: number };

export class ClickElementAction {
  async execute(
    sessionId: string,
    tabId: number,
    selector: string,
    opts?: { randomOffset?: boolean; forceSameTab?: boolean },
  ): Promise<{ x: number; y: number; rect: Rect }> {
    if (!selector || typeof selector !== "string") {
      throw new Error("click_element_cdp requires a CSS selector");
    }

    const handle = await cdpSessionManager.acquire(sessionId, tabId);

    const rect = await this.getElementRect(handle, selector, !!opts?.forceSameTab);
    const { x, y } = this.pickPoint(rect, !!opts?.randomOffset);

    await this.dispatchClick(handle, x, y);

    return { x, y, rect };
  }

  private async getElementRect(handle: { sendCommand<T = any>(method: string, params?: any): Promise<T> }, selector: string, forceSameTab: boolean): Promise<Rect> {
    const expression = `(() => {
      const sel = ${JSON.stringify(selector)};
      const forceSameTab = ${forceSameTab ? "true" : "false"};
      const qsDeep = (root, s) => {
        try {
          const el = root.querySelector(s);
          if (el) return el;
        } catch {}
        const nodes = root.querySelectorAll ? root.querySelectorAll('*') : [];
        for (const n of nodes) {
          const sr = n.shadowRoot;
          if (sr) {
            const found = qsDeep(sr, s);
            if (found) return found;
          }
        }
        return null;
      };
      const el = qsDeep(document, sel) || document.querySelector(sel);
      if (!el) return null;
      if (forceSameTab && el instanceof HTMLAnchorElement) {
        try {
          el.removeAttribute('target');
          el.target = '_self';
        } catch {}
      }
      try { el.scrollIntoView({ block: 'center', inline: 'center' }); } catch {}
      const r = el.getBoundingClientRect();
      if (!r) return null;
      return { left: r.left, top: r.top, width: r.width, height: r.height };
    })()`;

    const evalResp = await handle.sendCommand<any>("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: false,
    });

    const value = evalResp?.result?.value;
    if (!value || typeof value.left !== "number") {
      throw new Error(`CDP click: element not found for selector: ${selector}`);
    }

    return value as Rect;
  }

  private pickPoint(rect: Rect, randomOffset: boolean): { x: number; y: number } {
    const safeW = Math.max(1, rect.width);
    const safeH = Math.max(1, rect.height);

    const fx = randomOffset ? 0.35 + Math.random() * 0.3 : 0.5;
    const fy = randomOffset ? 0.35 + Math.random() * 0.3 : 0.5;

    // Background/service worker context has no `window`. Coordinates are in page CSS pixels.
    // Do not clamp to viewport; Input.dispatchMouseEvent can handle on-screen coordinates.
    const x = rect.left + safeW * fx;
    const y = rect.top + safeH * fy;

    return { x, y };
  }

  private async dispatchClick(handle: { sendCommand<T = any>(method: string, params?: any): Promise<T> }, x: number, y: number): Promise<void> {
    // Best-effort: move then press/release.
    await handle.sendCommand("Input.dispatchMouseEvent", {
      type: "mouseMoved",
      x,
      y,
      buttons: 0,
    });
    await handle.sendCommand("Input.dispatchMouseEvent", {
      type: "mousePressed",
      x,
      y,
      button: "left",
      clickCount: 1,
      buttons: 1,
    });
    await handle.sendCommand("Input.dispatchMouseEvent", {
      type: "mouseReleased",
      x,
      y,
      button: "left",
      clickCount: 1,
      buttons: 0,
    });

    await new Promise((resolve) => setTimeout(resolve, 50));
  }
}

export const clickElementAction = new ClickElementAction();

export async function handleClickElement(step: any): Promise<any> {
  const selector = step.selector;
  const randomOffset = step.random_offset === true;
  const forceSameTab = step.force_same_tab === true;
  const explicitTabId: number | undefined = step.tabId;

  if (!selector) {
    throw new Error("click_element_cdp requires selector");
  }

  let targetTabId: number | undefined = explicitTabId;
  if (!targetTabId) {
    targetTabId = await getActiveBrowserTabId("click_element_cdp");
  }

  const sessionId = String(step.session_id || step.sessionId || "default");
  const startedAt = Date.now();
  const clicked = await clickElementAction.execute(sessionId, targetTabId, String(selector), {
    randomOffset,
    forceSameTab,
  });

  const result = {
    selector: String(selector),
    clicked: true,
    force_same_tab: forceSameTab,
    point: { x: clicked.x, y: clicked.y },
    rect: clicked.rect,
    tabId: targetTabId,
  };

  return actionSuccess({
    action: "click_element_cdp",
    result,
    tabId: targetTabId,
    duration_ms: Date.now() - startedAt,
    legacy: result,
  });
}
