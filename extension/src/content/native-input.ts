// Native Input Layer 2 - Communication with broker for OS-level automation
// This module handles escalation when DOM events fail (React/Vue SPA issues)

import { logInfo, logError } from '../content-logger';
import { getFlags } from '../config/flags';

// Message types for extension ⇄ broker communication
// Must match broker's NativeInputRequest format
export interface NativeInputRequest {
  action: 'click' | 'type_text' | 'press_key';
  x?: number;
  y?: number;
  text?: string;
  key?: string;
  // Enhanced typing options for natural simulation
  typing_delay_ms?: number;
  natural_variance?: boolean;
}

export interface NativeInputResponse {
  ok: boolean;
  error?: string;
}

export class NativeInputHandler {
  private isEnabled = false;
  private availabilityChecked = false;
  private availabilityPromise: Promise<boolean> | null = null;

  constructor() {
    // Don't check availability on every page load - only check when needed
    // this.checkAvailability();
  }

  private async checkAvailability(force = false): Promise<boolean> {
    try {
      if (!force) {
        const flags = await getFlags(window.location.hostname);
        if (!flags.nativeInputEnabled) {
          this.isEnabled = false;
          logInfo('Native input Layer 2 disabled by flags', { feature: 'native_input' });
          return false;
        }
      }

      // Send a test message to see if broker supports native input
      const response = await this.sendToBroker({
        action: 'type_text',
        text: '' // Empty test message
      });
      this.isEnabled = response?.ok === true;
      if (this.isEnabled) {
        logInfo('Native input Layer 2 available', { feature: 'native_input' });
      }
      return this.isEnabled;
    } catch (e) {
      this.isEnabled = false;
      logInfo('Native input Layer 2 not available', { reason: 'broker_unavailable' });
      return false;
    }
  }

  /**
   * Check if native input escalation is available
   */
  public get available(): boolean {
    // Check availability on first use
    if (!this.availabilityChecked) {
      this.availabilityChecked = true;
      this.checkAvailability(false).catch(() => {
        console.log('Native input availability check failed');
      });
    }
    
    return this.isEnabled;
  }

  /**
   * Await broker availability. When force=true, explicit workflow opt-in can bypass
   * the storage flag gate and probe the native host directly.
   */
  public async ensureAvailable(options?: { force?: boolean }): Promise<boolean> {
    if (this.isEnabled) return true;

    const force = options?.force === true;
    if (this.availabilityPromise) {
      return this.availabilityPromise;
    }

    this.availabilityChecked = true;
    const probe = this.checkAvailability(force)
      .catch(() => false)
      .finally(() => {
        this.availabilityPromise = null;
      });
    this.availabilityPromise = probe;
    return probe;
  }

  /**
   * Send native input request to broker
   */
  private async sendToBroker(request: NativeInputRequest): Promise<NativeInputResponse | null> {
    try {
      // Use the existing message passing to broker
      const response = await new Promise<any>((resolve, reject) => {
        const messageId = `native_input_${Date.now()}_${Math.random()}`;
        
        const message = {
          cmd: 'native_input',
          req_id: messageId,
          payload: request
        };

        // Send to background script, which forwards to broker
        chrome.runtime.sendMessage(message, (response) => {
          if (chrome.runtime.lastError) {
            reject(new Error(chrome.runtime.lastError.message));
          } else {
            resolve(response);
          }
        });
      });

      return response as NativeInputResponse;
    } catch (error) {
      logError('Failed to send native input request', { error: error.message });
      return null;
    }
  }

  /**
   * Convert DOM coordinates to screen coordinates
   * Handles browser zoom, multi-monitor, and window positioning
   */
  private async domToScreenCoords(element: Element): Promise<{ x: number; y: number }> {
    const rect = element.getBoundingClientRect();
    
    // Calculate center point of element
    const domX = rect.left + rect.width / 2;
    const domY = rect.top + rect.height / 2;
    
    // Account for page scroll
    const pageX = domX + window.scrollX;
    const pageY = domY + window.scrollY;
    
    // Account for device pixel ratio (browser zoom)
    const screenX = Math.round(pageX * window.devicePixelRatio);
    const screenY = Math.round(pageY * window.devicePixelRatio);
    
    // Note: In a full implementation, we'd need to add window position offset
    // This requires chrome.windows.getCurrent() from background script
    
    return { x: screenX, y: screenY };
  }

  /**
   * Click element using native OS input
   */
  public async nativeClick(element: Element): Promise<boolean> {
    if (!(await this.ensureAvailable())) {
      logError('Native input not available for click', { element: element.tagName });
      return false;
    }

    try {
      const coords = await this.domToScreenCoords(element);
      
      logInfo('Attempting native click', { 
        coords, 
        element: element.tagName,
        feature: 'native_input' 
      });

      const response = await this.sendToBroker({
        action: 'click',
        x: coords.x,
        y: coords.y
      });

      if (response?.ok) {
        logInfo('Native click succeeded', { coords, feature: 'native_input' });
        return true;
      } else {
        logError('Native click failed', { 
          error: response?.error || 'Unknown error',
          coords 
        });
        return false;
      }
    } catch (error) {
      logError('Native click exception', { error: error.message });
      return false;
    }
  }

  /**
   * Type text using native OS input
   */
  public async nativeType(text: string, options?: { typing_delay_ms?: number; natural_variance?: boolean }): Promise<boolean> {
    if (!(await this.ensureAvailable())) {
      logError('Native input not available for typing', { textLength: text.length });
      return false;
    }

    try {
      const finalOptions = options || {};
      
      logInfo('Attempting native type', { 
        textLength: text.length,
        feature: 'native_input',
        options: finalOptions
      });

      const response = await this.sendToBroker({
        action: 'type_text',
        text: text,
        ...finalOptions
      });

      if (response?.ok) {
        logInfo('Native type succeeded', { 
          textLength: text.length, 
          feature: 'native_input',
          natural: !!finalOptions.typing_delay_ms
        });
        return true;
      } else {
        logError('Native type failed', { 
          error: response?.error || 'Unknown error',
          textLength: text.length 
        });
        return false;
      }
    } catch (error) {
      logError('Native type exception', { error: error.message });
      return false;
    }
  }

  /**
   * Type text naturally for YouTube and other SPAs that need human-like input
   */
  public async nativeTypeNatural(text: string, typingSpeed: 'slow' | 'medium' | 'fast' = 'medium'): Promise<boolean> {
    const delays = {
      slow: 200,
      medium: 100,
      fast: 50
    };

    return this.nativeType(text, {
      typing_delay_ms: delays[typingSpeed],
      natural_variance: true
    });
  }

  /**
   * Press key using native OS input
   */
  public async nativeKey(key: string): Promise<boolean> {
    if (!(await this.ensureAvailable())) {
      logError('Native input not available for key press', { key });
      return false;
    }

    try {
      logInfo('Attempting native key press', { 
        key,
        feature: 'native_input' 
      });

      const response = await this.sendToBroker({
        action: 'press_key',
        key: key
      });

      if (response?.ok) {
        logInfo('Native key press succeeded', { key, feature: 'native_input' });
        return true;
      } else {
        logError('Native key press failed', { 
          error: response?.error || 'Unknown error',
          key 
        });
        return false;
      }
    } catch (error) {
      logError('Native key press exception', { error: error.message });
      return false;
    }
  }

  /**
   * Wait for a condition to be met, used to detect if Layer 1 succeeded
   * Returns true if condition is met within timeout
   */
  public async waitForSuccess(
    condition: () => boolean, 
    timeoutMs: number = 800
  ): Promise<boolean> {
    const startTime = Date.now();
    
    while (Date.now() - startTime < timeoutMs) {
      if (condition()) {
        return true;
      }
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    
    return false;
  }

  /**
   * Smart typing with automatic Layer 2 escalation
   * Tries DOM first, escalates to native input if needed
   */
  public async smartType(
    element: HTMLElement, 
    text: string,
    options: {
      checkSuccess?: () => boolean;
      waitTimeMs?: number;
      focusFirst?: boolean;
    } = {}
  ): Promise<boolean> {
    const { 
      checkSuccess, 
      waitTimeMs = 800, 
      focusFirst = true 
    } = options;

    try {
      // Layer 1: DOM events
      if (focusFirst) {
        element.focus();
        await new Promise(resolve => setTimeout(resolve, 100));
      }

      // Clear and type using DOM
      if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
        element.value = text;
        element.dispatchEvent(new InputEvent('input', { 
          data: text, 
          bubbles: true, 
          cancelable: true 
        }));
      } else if (element.contentEditable === 'true') {
        element.textContent = text;
        element.dispatchEvent(new InputEvent('input', { 
          data: text, 
          bubbles: true, 
          cancelable: true 
        }));
      }

      // Check if Layer 1 succeeded
      if (checkSuccess) {
        const layer1Success = await this.waitForSuccess(checkSuccess, waitTimeMs);
        if (layer1Success) {
          logInfo('Layer 1 typing succeeded', { 
            text: text.substring(0, 20) + '...',
            layer: 'DOM',
            feature: 'smart_typing' 
          });
          return true;
        }
      } else {
        // No success check provided, assume Layer 1 worked
        return true;
      }

      // Layer 2: Native input escalation
      if (!this.available) {
        logError('Layer 1 failed and Layer 2 unavailable', { 
          text: text.substring(0, 20) + '...' 
        });
        return false;
      }

      logInfo('Layer 1 failed, escalating to Layer 2', { 
        text: text.substring(0, 20) + '...',
        layer: 'NATIVE',
        feature: 'smart_typing' 
      });

      // Focus element using native click
      const clickSuccess = await this.nativeClick(element);
      if (!clickSuccess) {
        return false;
      }

      // Type using native input
      const typeSuccess = await this.nativeType(text);
      return typeSuccess;

    } catch (error) {
      logError('Smart typing failed', { error: error.message });
      return false;
    }
  }

  /**
   * Smart key press with automatic Layer 2 escalation
   */
  public async smartKeyPress(
    element: HTMLElement,
    key: string,
    options: {
      checkSuccess?: () => boolean;
      waitTimeMs?: number;
      focusFirst?: boolean;
    } = {}
  ): Promise<boolean> {
    const { 
      checkSuccess, 
      waitTimeMs = 800, 
      focusFirst = true 
    } = options;

    try {
      // Layer 1: DOM events
      if (focusFirst) {
        element.focus();
        await new Promise(resolve => setTimeout(resolve, 100));
      }

      // Press key using DOM
      const keyEvent = new KeyboardEvent('keydown', {
        key: key,
        code: key,
        bubbles: true,
        cancelable: true
      });
      element.dispatchEvent(keyEvent);

      // Check if Layer 1 succeeded
      if (checkSuccess) {
        const layer1Success = await this.waitForSuccess(checkSuccess, waitTimeMs);
        if (layer1Success) {
          logInfo('Layer 1 key press succeeded', { 
            key,
            layer: 'DOM',
            feature: 'smart_key_press' 
          });
          return true;
        }
      } else {
        // No success check provided, assume Layer 1 worked
        return true;
      }

      // Layer 2: Native input escalation
      if (!this.available) {
        logError('Layer 1 failed and Layer 2 unavailable', { key });
        return false;
      }

      logInfo('Layer 1 failed, escalating to Layer 2', { 
        key,
        layer: 'NATIVE',
        feature: 'smart_key_press' 
      });

      // Press key using native input
      const keySuccess = await this.nativeKey(key);
      return keySuccess;

    } catch (error) {
      logError('Smart key press failed', { error: error.message, key });
      return false;
    }
  }
}

// Global instance
export const nativeInput = new NativeInputHandler();
