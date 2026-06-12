/**
 * Integration between content script and compact snapshot system
 * Handles snapshot generation requests from the runtime bridge
 */

import { snapshotManager, CompactSnapshot, ACTION_TYPES } from '../snapshot';
import { domAnalyzer } from './dom-analyzer';

export class SnapshotIntegration {
  private isEnabled: boolean = true;

  /**
   * Handle snapshot generation request from the runtime bridge
   */
  async handleSnapshotRequest(message: any): Promise<CompactSnapshot> {
    console.log('[RZN Snapshot Integration] Generating snapshot...');
    
    try {
      // Generate compact snapshot
      const snapshot = await snapshotManager.generateSnapshot();
      
      console.log('[RZN Snapshot Integration] Generated snapshot:', {
        elements: snapshot.elements.length,
        sizeKB: snapshot.sizeKB,
        compression: snapshot.compressionLevel
      });
      
      return snapshot;
    } catch (error) {
      console.error('[RZN Snapshot Integration] Failed to generate snapshot:', error);
      throw error;
    }
  }

  /**
   * Handle element interaction and track in memory
   */
  async handleElementAction(action: string, encodedId?: string, details?: any): Promise<boolean> {
    if (!this.isEnabled) return false;
    
    try {
      let success = true;
      
      // Find element using encoded ID
      let element: Element | null = null;
      if (encodedId) {
        element = snapshotManager.findElement(encodedId);
        if (!element) {
          console.warn('[RZN Snapshot Integration] Element not found:', encodedId);
          success = false;
        }
      }
      
      // Perform the action
      if (success && element) {
        success = await this.performElementAction(action, element, details);
      }
      
      // Track action in memory
      snapshotManager.trackAction(action, encodedId, success, details);
      
      return success;
    } catch (error) {
      console.error('[RZN Snapshot Integration] Action failed:', error);
      snapshotManager.trackAction(action, encodedId, false, { error: error.message });
      return false;
    }
  }

  /**
   * Perform specific element action
   */
  private async performElementAction(action: string, element: Element, details?: any): Promise<boolean> {
    switch (action) {
      case ACTION_TYPES.CLICK:
        return this.performClick(element);
      
      case ACTION_TYPES.TYPE:
        return this.performType(element, details?.text || '');
      
      case ACTION_TYPES.SELECT:
        return this.performSelect(element, details?.option || '');
      
      default:
        console.warn('[RZN Snapshot Integration] Unknown action:', action);
        return false;
    }
  }

  /**
   * Perform click action
   */
  private async performClick(element: Element): Promise<boolean> {
    try {
      // Scroll element into view
      element.scrollIntoView({ behavior: 'smooth', block: 'center' });
      
      // Wait a bit for scroll
      await new Promise(resolve => setTimeout(resolve, 100));
      
      // Create and dispatch click event
      const clickEvent = new MouseEvent('click', {
        bubbles: true,
        cancelable: true,
        view: window
      });
      
      element.dispatchEvent(clickEvent);
      
      // Also trigger native click if it's a button or link
      if (element instanceof HTMLButtonElement || element instanceof HTMLAnchorElement) {
        (element as HTMLElement).click();
      }
      
      return true;
    } catch (error) {
      console.error('[RZN Snapshot Integration] Click failed:', error);
      return false;
    }
  }

  /**
   * Perform type action
   */
  private async performType(element: Element, text: string): Promise<boolean> {
    try {
      if (!(element instanceof HTMLInputElement) && !(element instanceof HTMLTextAreaElement)) {
        console.error('[RZN Snapshot Integration] Element is not typeable');
        return false;
      }
      
      // Focus the element
      element.focus();
      
      // Clear existing content
      element.select();
      
      // Type the text character by character for realism
      element.value = '';
      for (let i = 0; i < text.length; i++) {
        const char = text[i];
        
        // Dispatch keydown event
        const keyDownEvent = new KeyboardEvent('keydown', {
          key: char,
          bubbles: true,
          cancelable: true
        });
        element.dispatchEvent(keyDownEvent);
        
        // Update value
        element.value += char;
        
        // Dispatch input event
        const inputEvent = new InputEvent('input', {
          data: char,
          bubbles: true,
          cancelable: true
        });
        element.dispatchEvent(inputEvent);
        
        // Small delay between characters
        await new Promise(resolve => setTimeout(resolve, 50 + Math.random() * 50));
      }
      
      // Dispatch change event
      const changeEvent = new Event('change', {
        bubbles: true,
        cancelable: true
      });
      element.dispatchEvent(changeEvent);
      
      return true;
    } catch (error) {
      console.error('[RZN Snapshot Integration] Type failed:', error);
      return false;
    }
  }

  /**
   * Perform select action
   */
  private async performSelect(element: Element, optionText: string): Promise<boolean> {
    try {
      if (!(element instanceof HTMLSelectElement)) {
        console.error('[RZN Snapshot Integration] Element is not a select');
        return false;
      }
      
      // Find option by text or value
      let targetOption: HTMLOptionElement | null = null;
      for (const option of element.options) {
        if (option.text.trim() === optionText || option.value === optionText) {
          targetOption = option;
          break;
        }
      }
      
      if (!targetOption) {
        console.error('[RZN Snapshot Integration] Option not found:', optionText);
        return false;
      }
      
      // Select the option
      element.selectedIndex = targetOption.index;
      
      // Dispatch change event
      const changeEvent = new Event('change', {
        bubbles: true,
        cancelable: true
      });
      element.dispatchEvent(changeEvent);
      
      return true;
    } catch (error) {
      console.error('[RZN Snapshot Integration] Select failed:', error);
      return false;
    }
  }

  /**
   * Generate prompt for LLM from current page state
   */
  async generatePrompt(includeMemory: boolean = true): Promise<string> {
    try {
      const snapshot = await snapshotManager.generateSnapshot();
      return snapshotManager.generatePrompt(snapshot, includeMemory);
    } catch (error) {
      console.error('[RZN Snapshot Integration] Failed to generate prompt:', error);
      return 'Error: Unable to generate page snapshot';
    }
  }

  /**
   * Get memory statistics
   */
  getMemoryStats() {
    return snapshotManager.getMemoryStats();
  }

  /**
   * Clear action memory
   */
  clearMemory(): void {
    snapshotManager.clearMemory();
  }

  /**
   * Enable/disable snapshot integration
   */
  setEnabled(enabled: boolean): void {
    this.isEnabled = enabled;
    console.log('[RZN Snapshot Integration] Integration', enabled ? 'enabled' : 'disabled');
  }

  /**
   * Get last snapshot statistics
   */
  getLastSnapshotStats() {
    return snapshotManager.getStats();
  }
}

// Export singleton instance
export const snapshotIntegration = new SnapshotIntegration();
