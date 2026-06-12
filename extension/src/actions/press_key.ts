import { getActiveBrowserTabId } from '../browserTabs';
import { actionSuccess } from './actionResult';

/**
 * First-class press_key action using CDP for reliable key input
 * Bypasses DOM event simulation for better reliability
 */

interface KeyMapping {
  key: string;
  code: string;
  keyCode: number;
  text?: string;
}

const KEY_MAPPINGS: Record<string, KeyMapping> = {
  'Enter': { key: 'Enter', code: 'Enter', keyCode: 13, text: '\r' },
  'Tab': { key: 'Tab', code: 'Tab', keyCode: 9, text: '\t' },
  'Escape': { key: 'Escape', code: 'Escape', keyCode: 27 },
  'ArrowUp': { key: 'ArrowUp', code: 'ArrowUp', keyCode: 38 },
  'ArrowDown': { key: 'ArrowDown', code: 'ArrowDown', keyCode: 40 },
  'ArrowLeft': { key: 'ArrowLeft', code: 'ArrowLeft', keyCode: 37 },
  'ArrowRight': { key: 'ArrowRight', code: 'ArrowRight', keyCode: 39 },
  'Backspace': { key: 'Backspace', code: 'Backspace', keyCode: 8 },
  'Delete': { key: 'Delete', code: 'Delete', keyCode: 46 },
  'Home': { key: 'Home', code: 'Home', keyCode: 36 },
  'End': { key: 'End', code: 'End', keyCode: 35 },
  'PageUp': { key: 'PageUp', code: 'PageUp', keyCode: 33 },
  'PageDown': { key: 'PageDown', code: 'PageDown', keyCode: 34 },
};

export class PressKeyAction {
  private debugger: chrome.debugger.Debuggee | null = null;
  private ownsDebuggerAttachment = false;

  async execute(
    tabId: number,
    key: string,
    options: { manageDebuggerLifecycle?: boolean } = {}
  ): Promise<void> {
    const mapping = KEY_MAPPINGS[key];
    if (!mapping) {
      throw new Error(`Unsupported key: ${key}. Supported keys: ${Object.keys(KEY_MAPPINGS).join(', ')}`);
    }

    console.log(`[PressKeyAction] Pressing key: ${key} via CDP`);
    const manageDebuggerLifecycle = options.manageDebuggerLifecycle !== false;

    try {
      // Attach debugger if not already attached
      if (manageDebuggerLifecycle && (!this.debugger || this.debugger.tabId !== tabId)) {
        await this.attachDebugger(tabId);
      }

      // Send key events via CDP
      await this.sendKeyPress(tabId, mapping);

      console.log(`[PressKeyAction] Successfully pressed ${key}`);
    } catch (error) {
      console.error(`[PressKeyAction] Failed to press ${key}:`, error);
      throw error;
    } finally {
      if (manageDebuggerLifecycle) {
        // Always detach quickly to minimize detection and avoid hanging contexts
        await this.cleanup().catch(() => {});
      }
    }
  }

  private async attachDebugger(tabId: number): Promise<void> {
    this.debugger = { tabId };
    this.ownsDebuggerAttachment = false;
    
    try {
      await chrome.debugger.attach(this.debugger, '1.3');
      this.ownsDebuggerAttachment = true;
      console.log(`[PressKeyAction] Debugger attached to tab ${tabId}`);
    } catch (error: any) {
      if (error?.message?.includes('Another debugger')) {
        console.log(`[PressKeyAction] Debugger already attached to tab ${tabId}`);
      } else {
        throw error;
      }
    }
  }

  private async sendKeyPress(tabId: number, mapping: KeyMapping): Promise<void> {
    const debuggee = { tabId };

    // Send keydown event
    await chrome.debugger.sendCommand(debuggee, 'Input.dispatchKeyEvent', {
      type: 'keyDown',
      key: mapping.key,
      code: mapping.code,
      windowsVirtualKeyCode: mapping.keyCode,
      nativeVirtualKeyCode: mapping.keyCode,
      text: '',
      unmodifiedText: '',
      autoRepeat: false,
      isKeypad: false,
      isSystemKey: false,
    });

    // For keys with text representation, send char event
    if (mapping.text) {
      await chrome.debugger.sendCommand(debuggee, 'Input.dispatchKeyEvent', {
        type: 'char',
        key: mapping.key,
        code: mapping.code,
        windowsVirtualKeyCode: mapping.keyCode,
        nativeVirtualKeyCode: mapping.keyCode,
        text: mapping.text,
        unmodifiedText: mapping.text,
        autoRepeat: false,
        isKeypad: false,
        isSystemKey: false,
      });
    }

    // Send keyup event
    await chrome.debugger.sendCommand(debuggee, 'Input.dispatchKeyEvent', {
      type: 'keyUp',
      key: mapping.key,
      code: mapping.code,
      windowsVirtualKeyCode: mapping.keyCode,
      nativeVirtualKeyCode: mapping.keyCode,
      text: '',
      unmodifiedText: '',
      autoRepeat: false,
      isKeypad: false,
      isSystemKey: false,
    });

    // Small delay to ensure key event is processed
    await new Promise(resolve => setTimeout(resolve, 50));
  }

  async cleanup(): Promise<void> {
    if (this.debugger) {
      if (this.ownsDebuggerAttachment) {
        try {
          await chrome.debugger.detach(this.debugger);
          console.log(`[PressKeyAction] Debugger detached`);
        } catch (error) {
          console.warn(`[PressKeyAction] Failed to detach debugger:`, error);
        }
      }
      this.debugger = null;
      this.ownsDebuggerAttachment = false;
    }
  }
}

// Singleton instance
export const pressKeyAction = new PressKeyAction();

// Integration with existing action system
export async function handlePressKey(step: any): Promise<any> {
  const key = step.key || (step.args && step.args[0]);
  const explicitTabId: number | undefined = step.tabId;
  const manageDebuggerLifecycle = step.manageDebuggerLifecycle !== false;
  
  if (!key) {
    throw new Error('press_key requires a key parameter');
  }

  let targetTabId: number | undefined = explicitTabId;
  if (!targetTabId) {
    targetTabId = await getActiveBrowserTabId('press_key');
  }

  const startedAt = Date.now();
  await pressKeyAction.execute(targetTabId, key, { manageDebuggerLifecycle });

  const result = {
    pressed: true,
    key: key,
    tabId: targetTabId,
  };

  return actionSuccess({
    action: 'press_key',
    result,
    tabId: targetTabId,
    duration_ms: Date.now() - startedAt,
    legacy: result,
  });
}
