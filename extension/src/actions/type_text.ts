import { getActiveBrowserTabId } from '../browserTabs';
import { actionSuccess } from './actionResult';

/**
 * First-class type_text action using CDP for trusted per-key typing.
 * This is useful for structured editors that reject plain DOM mutation or bulk insertText.
 */

interface CharKeyMapping {
  key: string;
  code: string;
  keyCode: number;
  text: string;
}

export class TypeTextAction {
  private debugger: chrome.debugger.Debuggee | null = null;
  private ownsDebuggerAttachment = false;

  async execute(
    tabId: number,
    text: string,
    options: { manageDebuggerLifecycle?: boolean } = {}
  ): Promise<void> {
    console.log(`[TypeTextAction] Typing text via CDP (length=${text.length})`);
    const manageDebuggerLifecycle = options.manageDebuggerLifecycle !== false;

    try {
      if (manageDebuggerLifecycle && (!this.debugger || this.debugger.tabId !== tabId)) {
        await this.attachDebugger(tabId);
      }

      for (const ch of text) {
        const mapping = this.mapChar(ch);
        if (mapping.text) {
          await chrome.debugger.sendCommand({ tabId }, 'Input.insertText', {
            text: mapping.text,
          });
          await new Promise(resolve => setTimeout(resolve, 20));
          continue;
        }

        await chrome.debugger.sendCommand({ tabId }, 'Input.dispatchKeyEvent', {
          // Non-printable keys still need the keyboard event path.
          type: 'keyDown',
          key: mapping.key,
          code: mapping.code,
          windowsVirtualKeyCode: mapping.keyCode,
          nativeVirtualKeyCode: mapping.keyCode,
          text: '',
          unmodifiedText: '',
          autoRepeat: false,
          isKeypad: false,
          isSystemKey: false
        });
        await chrome.debugger.sendCommand({ tabId }, 'Input.dispatchKeyEvent', {
          type: 'keyUp',
          key: mapping.key,
          code: mapping.code,
          windowsVirtualKeyCode: mapping.keyCode,
          nativeVirtualKeyCode: mapping.keyCode,
          text: '',
          unmodifiedText: '',
          autoRepeat: false,
          isKeypad: false,
          isSystemKey: false
        });
        await new Promise(resolve => setTimeout(resolve, 20));
      }
    } catch (error) {
      console.error('[TypeTextAction] Failed to type text:', error);
      throw error;
    } finally {
      if (manageDebuggerLifecycle) {
        await this.cleanup().catch(() => {});
      }
    }
  }

  private mapChar(ch: string): CharKeyMapping {
    if (/^[a-zA-Z]$/.test(ch)) {
      const upper = ch.toUpperCase();
      return { key: ch, code: `Key${upper}`, keyCode: upper.charCodeAt(0), text: ch };
    }
    if (/^[0-9]$/.test(ch)) {
      return { key: ch, code: `Digit${ch}`, keyCode: ch.charCodeAt(0), text: ch };
    }

    const special: Record<string, CharKeyMapping> = {
      ' ': { key: ' ', code: 'Space', keyCode: 32, text: ' ' },
      '.': { key: '.', code: 'Period', keyCode: 190, text: '.' },
      ',': { key: ',', code: 'Comma', keyCode: 188, text: ',' },
      '-': { key: '-', code: 'Minus', keyCode: 189, text: '-' },
      '_': { key: '_', code: 'Minus', keyCode: 189, text: '_' },
      '/': { key: '/', code: 'Slash', keyCode: 191, text: '/' },
      ':': { key: ':', code: 'Semicolon', keyCode: 186, text: ':' },
      ';': { key: ';', code: 'Semicolon', keyCode: 186, text: ';' },
      '\'': { key: '\'', code: 'Quote', keyCode: 222, text: '\'' },
      '"': { key: '"', code: 'Quote', keyCode: 222, text: '"' },
      '!': { key: '!', code: 'Digit1', keyCode: 49, text: '!' },
      '?': { key: '?', code: 'Slash', keyCode: 191, text: '?' },
      '(': { key: '(', code: 'Digit9', keyCode: 57, text: '(' },
      ')': { key: ')', code: 'Digit0', keyCode: 48, text: ')' }
    };

    return special[ch] || { key: ch, code: 'Unidentified', keyCode: ch.charCodeAt(0) || 0, text: ch };
  }

  private async attachDebugger(tabId: number): Promise<void> {
    this.debugger = { tabId };
    this.ownsDebuggerAttachment = false;

    try {
      await chrome.debugger.attach(this.debugger, '1.3');
      this.ownsDebuggerAttachment = true;
      console.log(`[TypeTextAction] Debugger attached to tab ${tabId}`);
    } catch (error: any) {
      if (error?.message?.includes('Another debugger')) {
        console.log(`[TypeTextAction] Debugger already attached to tab ${tabId}`);
      } else {
        throw error;
      }
    }
  }

  async cleanup(): Promise<void> {
    if (this.debugger) {
      if (this.ownsDebuggerAttachment) {
        try {
          await chrome.debugger.detach(this.debugger);
          console.log('[TypeTextAction] Debugger detached');
        } catch (error) {
          console.warn('[TypeTextAction] Failed to detach debugger:', error);
        }
      }
      this.debugger = null;
      this.ownsDebuggerAttachment = false;
    }
  }
}

export const typeTextAction = new TypeTextAction();

export async function handleTypeText(step: any): Promise<any> {
  const text = String(step.text ?? step.value ?? '');
  const explicitTabId: number | undefined = step.tabId;
  const manageDebuggerLifecycle = step.manageDebuggerLifecycle !== false;

  let targetTabId: number | undefined = explicitTabId;
  if (!targetTabId) {
    targetTabId = await getActiveBrowserTabId('type_text');
  }

  const startedAt = Date.now();
  await typeTextAction.execute(targetTabId, text, { manageDebuggerLifecycle });

  const result = {
    inserted: true,
    textLength: text.length,
    tabId: targetTabId,
  };

  return actionSuccess({
    action: 'type_text',
    result,
    tabId: targetTabId,
    duration_ms: Date.now() - startedAt,
    legacy: result,
  });
}
