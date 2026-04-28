/**
 * upload_file using CDP (break-glass).
 *
 * Browsers intentionally prevent setting <input type="file"> programmatically from page JS.
 * CDP's DOM.setFileInputFiles is the reliable, generic way to drive uploads when explicitly requested.
 */

import { cdpSessionManager } from '../runtime/cdp_session_manager';

export class UploadFileAction {
  async execute(
    sessionId: string,
    tabId: number,
    selector: string,
    filePaths: string[],
  ): Promise<{ selector: string; file_count: number; files: string[]; tabId: number; skipped?: boolean }> {
    if (!selector || typeof selector !== "string") {
      throw new Error("upload_file requires a CSS selector");
    }
    if (!Array.isArray(filePaths) || filePaths.length === 0) {
      // Empty/unsubstituted paths → no-op so callers can have conditional uploads.
      return { selector, file_count: 0, files: [], tabId, skipped: true };
    }

    const handle = await cdpSessionManager.acquire(sessionId, tabId);
    const objectId = await this.getFileInputObjectId(handle, selector);

    await handle.sendCommand("DOM.enable", {});
    await handle.sendCommand("DOM.setFileInputFiles", {
      objectId,
      files: filePaths,
    });

    // Best-effort: dispatch input/change so frameworks pick it up.
    try {
      await handle.sendCommand("Runtime.callFunctionOn", {
        objectId,
        functionDeclaration: `function() {
            try {
              this.dispatchEvent(new Event('input', { bubbles: true, cancelable: true }));
              this.dispatchEvent(new Event('change', { bubbles: true, cancelable: true }));
            } catch {}
            return true;
          }`,
        returnByValue: true,
      });
    } catch {}

    const files = filePaths.map((p) => {
      const s = String(p);
      const parts = s.split(/[/\\\\]/);
      return parts[parts.length - 1] || s;
    });

    return { selector, file_count: filePaths.length, files, tabId };
  }

  private async getFileInputObjectId(handle: { sendCommand<T = any>(method: string, params?: any): Promise<T> }, selector: string): Promise<string> {
    const expression = `(() => {
      const sel = ${JSON.stringify(selector)};
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
      if (!el) throw new Error('UPLOAD_FILE_SELECTOR_NOT_FOUND: ' + sel);
      if (!(el instanceof HTMLInputElement) || el.type !== 'file') {
        throw new Error('UPLOAD_FILE_NOT_FILE_INPUT: ' + (el && el.tagName));
      }
      try { el.scrollIntoView({ block: 'center', inline: 'center' }); } catch {}
      return el;
    })()`;

    const evalResp: any = await handle.sendCommand("Runtime.evaluate", {
      expression,
      returnByValue: false,
      awaitPromise: false,
    });

    const objectId = evalResp?.result?.objectId;
    if (!objectId) {
      const details = evalResp?.exceptionDetails;
      const msg = details?.exception?.description || details?.text || "Failed to resolve file input";
      throw new Error(String(msg));
    }

    return String(objectId);
  }
}

export const uploadFileAction = new UploadFileAction();

export async function handleUploadFile(step: any): Promise<any> {
  const selector = step.selector;
  const filePath = step.file_path;
  const filePathsRaw = (step.file_paths as any) ?? filePath;

  const isPlaceholder = (value: string) => /^\{[a-zA-Z0-9_]+\}$/.test(value);
  const filePaths = (() => {
    if (Array.isArray(filePathsRaw)) {
      return filePathsRaw
        .map((p) => String(p).trim())
        .filter((p) => p && !isPlaceholder(p));
    }
    const s = String(filePathsRaw || "").trim();
    if (!s || isPlaceholder(s)) return [];
    if (s.startsWith("[")) {
      // JSON-encoded list: [\"/a\", \"/b\"]
      try {
        const parsed = JSON.parse(s);
        if (Array.isArray(parsed)) {
          return parsed
            .map((p) => String(p).trim())
            .filter((p) => p && !isPlaceholder(p));
        }
      } catch {}
    }
    if (s.includes(",")) {
      return s
        .split(",")
        .map((p) => p.trim())
        .filter((p) => p && !isPlaceholder(p));
    }
    return [s];
  })();

  if (!selector) {
    throw new Error("upload_file requires selector");
  }

  const explicitTabId: number | undefined = step.tabId;
  let targetTabId: number | undefined = explicitTabId;
  if (!targetTabId) {
    const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
    if (!tabs[0]?.id) {
      throw new Error("No active tab found");
    }
    targetTabId = tabs[0].id;
  }

  const sessionId = String(step.session_id || step.sessionId || "default");
  const result = await uploadFileAction.execute(sessionId, targetTabId, String(selector), filePaths);
  return {
    success: true,
    action: "upload_file_cdp",
    selector: String(selector),
    file_count: result.file_count,
    files: result.files,
    tabId: targetTabId,
    timestamp: Date.now(),
  };
}
