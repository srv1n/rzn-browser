// Scripted Input Rung - Enhanced MouseEvent/KeyboardEvent with realistic sequences
// Better compatibility with complex UI frameworks, still same-origin

import { ResolvedElement, parseEncodedId } from '../../types/targets';
import { InputAction } from '../ladder';

export class ScriptedInputExecutor {
  /**
   * Check if scripted events can be used for this element/action combination
   */
  canExecute(element: ResolvedElement, action: InputAction): boolean {
    // Scripted events work for same-origin elements
    // Can also handle some cross-origin cases if element is accessible
    return ['click', 'fill', 'key', 'hover', 'scroll'].includes(action.type);
  }

  /**
   * Execute action using enhanced scripted events with realistic behavior
   */
  async execute(element: ResolvedElement, action: InputAction): Promise<boolean> {
    try {
      const domElement = await this.resolveElement(element);
      if (!domElement) {
        console.warn('[ScriptedRung] Element not found in DOM');
        return false;
      }

      // Ensure element is visible and interactable
      if (!this.isElementInteractable(domElement)) {
        console.debug('[ScriptedRung] Element is not interactable');
        return false;
      }

      // Execute the specific action with enhanced event sequences
      switch (action.type) {
        case 'click':
          return this.executeRealisticClick(domElement, action);
        case 'fill':
          return this.executeRealisticFill(domElement, action);
        case 'key':
          return this.executeRealisticKey(domElement, action);
        case 'hover':
          return this.executeRealisticHover(domElement, action);
        case 'scroll':
          return this.executeRealisticScroll(domElement, action);
        default:
          console.warn(`[ScriptedRung] Unsupported action type: ${action.type}`);
          return false;
      }
    } catch (error) {
      console.error('[ScriptedRung] Execution error:', error);
      return false;
    }
  }

  private async resolveElement(resolved: ResolvedElement): Promise<Element | null> {
    // Same logic as DOM rung but with additional fallbacks
    if (resolved.target_spec.css) {
      return document.querySelector(resolved.target_spec.css);
    }
    
    if (resolved.target_spec.xpath) {
      const result = document.evaluate(
        resolved.target_spec.xpath,
        document,
        null,
        XPathResult.FIRST_ORDERED_NODE_TYPE,
        null
      );
      return result.singleNodeValue as Element | null;
    }

    // Try to find by encoded ID
    const { backendNodeId } = parseEncodedId(resolved.encoded_id);
    const elementWithNodeId = document.querySelector(`[data-backend-node="${backendNodeId}"]`);
    if (elementWithNodeId) {
      return elementWithNodeId;
    }

    console.warn('[ScriptedRung] Could not resolve element to DOM node');
    return null;
  }

	  private async executeRealisticClick(element: Element, action: InputAction): Promise<boolean> {
	    try {
	      const rect = element.getBoundingClientRect();
	      const centerX = rect.left + rect.width / 2;
	      const centerY = rect.top + rect.height / 2;

      // Add small random offset for more realistic behavior
      const offsetX = centerX + (Math.random() - 0.5) * Math.min(rect.width * 0.1, 10);
      const offsetY = centerY + (Math.random() - 0.5) * Math.min(rect.height * 0.1, 10);

      const eventOptions = {
        bubbles: true,
        cancelable: true,
        view: window,
        clientX: offsetX,
        clientY: offsetY,
        screenX: offsetX + window.screenX,
        screenY: offsetY + window.screenY,
        button: this.getButtonNumber(action.options?.button || 'left'),
        buttons: this.getButtonFlags(action.options?.button || 'left')
      };

      // Add modifier states
      if (action.options?.modifiers) {
        Object.assign(eventOptions, this.getModifierState(action.options.modifiers));
      }

      // Realistic mouse event sequence
      const events = [
        new MouseEvent('mousedown', eventOptions),
        new MouseEvent('mouseup', eventOptions),
        new MouseEvent('click', eventOptions)
      ];

      // Also dispatch mouseover/mouseenter if element wasn't already hovered
      const mouseOverEvent = new MouseEvent('mouseover', eventOptions);
      const mouseEnterEvent = new MouseEvent('mouseenter', {
        ...eventOptions,
        bubbles: false // mouseenter doesn't bubble
      });

      // Execute sequence with small delays for realism
      element.dispatchEvent(mouseOverEvent);
      element.dispatchEvent(mouseEnterEvent);
      
	      await this.delay(10 + Math.random() * 20); // 10-30ms delay
	      
	      // Note: dispatchEvent() returns false when a listener calls preventDefault().
	      // That does NOT mean the click failed (many SPAs preventDefault on click).
	      for (const event of events) {
	        element.dispatchEvent(event);
	        await this.delay(5 + Math.random() * 10); // 5-15ms between events
	      }

	      // For certain elements, also trigger the native click
	      if (this.shouldTriggerNativeClick(element)) {
	        (element as any).click();
	      }

	      return true;
	    } catch (error) {
	      console.error('[ScriptedRung] Click execution error:', error);
	      return false;
	    }
	  }

  private async executeRealisticFill(element: Element, action: InputAction): Promise<boolean> {
    try {
      if (!action.value) {
        console.warn('[ScriptedRung] No value provided for fill action');
        return false;
      }

      const inputElement = element as HTMLInputElement | HTMLTextAreaElement;
      
      if (!('value' in inputElement)) {
        console.warn('[ScriptedRung] Element is not fillable');
        return false;
      }

      // Focus the element first
      inputElement.focus();
      
      // Dispatch focus events
      inputElement.dispatchEvent(new FocusEvent('focusin', { bubbles: true }));
      inputElement.dispatchEvent(new FocusEvent('focus', { bubbles: false }));

      await this.delay(50 + Math.random() * 100); // Realistic pause

      // Clear existing value with realistic key events
      if (inputElement.value.length > 0) {
        await this.clearFieldRealistic(inputElement);
      }

      // Type each character with realistic timing
      for (let i = 0; i < action.value.length; i++) {
        const char = action.value[i];
        await this.typeCharacterRealistic(inputElement, char, i === action.value.length - 1);
        
        // Random typing delay (50-150ms per character)
        await this.delay(50 + Math.random() * 100);
      }

      // Final change event
      inputElement.dispatchEvent(new Event('change', {
        bubbles: true,
        cancelable: true
      }));

      return true;
    } catch (error) {
      console.error('[ScriptedRung] Fill execution error:', error);
      return false;
    }
  }

  private async executeRealisticKey(element: Element, action: InputAction): Promise<boolean> {
    try {
      if (!action.key) {
        console.warn('[ScriptedRung] No key provided for key action');
        return false;
      }

      // Focus the element
      if ('focus' in element) {
        (element as any).focus();
      }

      const keyEventOptions = {
        key: action.key,
        code: this.getKeyCode(action.key),
        bubbles: true,
        cancelable: true,
        view: window
      };

      // Add modifier states
      if (action.options?.modifiers) {
        Object.assign(keyEventOptions, this.getModifierState(action.options.modifiers));
      }

      // Realistic key event sequence
      const keydownEvent = new KeyboardEvent('keydown', keyEventOptions);
      const keypressEvent = new KeyboardEvent('keypress', keyEventOptions);
      const keyupEvent = new KeyboardEvent('keyup', keyEventOptions);

      // Execute sequence
      const downResult = element.dispatchEvent(keydownEvent);
      await this.delay(10 + Math.random() * 20);
      
      const pressResult = element.dispatchEvent(keypressEvent);
      await this.delay(50 + Math.random() * 50);
      
      const upResult = element.dispatchEvent(keyupEvent);

      return downResult && pressResult && upResult;
    } catch (error) {
      console.error('[ScriptedRung] Key execution error:', error);
      return false;
    }
  }

  private async executeRealisticHover(element: Element, action: InputAction): Promise<boolean> {
    try {
      const rect = element.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) {
        return false;
      }

      const clampPoint = (x: number, y: number) => ({
        x: Math.max(1, Math.min(window.innerWidth - 1, x)),
        y: Math.max(1, Math.min(window.innerHeight - 1, y)),
      });

      const finalPoint = clampPoint(
        rect.left + rect.width / 2 + (Math.random() - 0.5) * Math.min(rect.width * 0.16, 16),
        rect.top + rect.height / 2 + (Math.random() - 0.5) * Math.min(rect.height * 0.16, 16),
      );
      const startPoint = clampPoint(
        Math.max(2, rect.left - Math.min(48, rect.width * 0.2)),
        Math.max(2, rect.top + Math.min(rect.height * 0.2, 20)),
      );

      const buildEventOptions = (x: number, y: number) => ({
        bubbles: true,
        cancelable: true,
        view: window,
        clientX: x,
        clientY: y,
        screenX: x + window.screenX,
        screenY: y + window.screenY,
      });

      const dispatch = (target: EventTarget | null, type: string, options: MouseEventInit, bubble = true) => {
        if (!target || !(target instanceof Element || target === document || target === window)) {
          return;
        }
        const ctor = type.startsWith('pointer') && typeof (window as any).PointerEvent === 'function'
          ? (window as any).PointerEvent
          : MouseEvent;
        try {
          target.dispatchEvent(new ctor(type, { ...options, bubbles: bubble }));
        } catch {}
      };

      const dispatchEnter = (targets: Element[], options: MouseEventInit) => {
        for (const target of targets) {
          dispatch(target, 'pointerover', options);
          dispatch(target, 'mouseover', options);
        }
        for (const target of targets) {
          dispatch(target, 'pointerenter', options, false);
          dispatch(target, 'mouseenter', options, false);
        }
      };

      const dispatchMove = (targets: Element[], options: MouseEventInit) => {
        dispatch(document, 'pointermove', options);
        dispatch(document, 'mousemove', options);
        dispatch(window, 'pointermove', options);
        dispatch(window, 'mousemove', options);
        for (const target of targets) {
          dispatch(target, 'pointermove', options);
          dispatch(target, 'mousemove', options);
        }
      };

      const sameTargets = (left: Element[], right: Element[]) =>
        left.length === right.length && left.every((item, index) => item === right[index]);

      const steps = Math.max(8, Math.min(18, Math.round(Math.max(rect.width, rect.height) / 28)));
      let previousTargets: Element[] = [];

      for (let i = 0; i <= steps; i++) {
        const progress = i / steps;
        const eased = progress < 0.5
          ? 4 * progress * progress * progress
          : 1 - Math.pow(-2 * progress + 2, 3) / 2;
        const currentPoint = clampPoint(
          startPoint.x + (finalPoint.x - startPoint.x) * eased,
          startPoint.y + (finalPoint.y - startPoint.y) * eased,
        );
        const options = buildEventOptions(currentPoint.x, currentPoint.y);
        const targets = document
          .elementsFromPoint(currentPoint.x, currentPoint.y)
          .filter((node): node is Element => node instanceof Element)
          .slice(0, 6);

        if (!sameTargets(previousTargets, targets)) {
          dispatchEnter(targets, options);
        }
        dispatchMove(targets, options);
        previousTargets = targets;
        await this.delay(18 + Math.random() * 26);
      }

      const finalOptions = buildEventOptions(finalPoint.x, finalPoint.y);
      const finalTargets = document
        .elementsFromPoint(finalPoint.x, finalPoint.y)
        .filter((node): node is Element => node instanceof Element)
        .slice(0, 6);
      dispatchEnter(finalTargets, finalOptions);
      dispatchMove(finalTargets, finalOptions);
      await this.delay(160 + Math.random() * 140);

      return true;
    } catch (error) {
      console.error('[ScriptedRung] Hover execution error:', error);
      return false;
    }
  }

  private async executeRealisticScroll(element: Element, action: InputAction): Promise<boolean> {
    try {
      // Get current and target positions
      const rect = element.getBoundingClientRect();
      const targetY = rect.top + rect.height / 2 - window.innerHeight / 2;
      const currentY = window.scrollY;

      // Smooth scroll simulation
      const distance = targetY - currentY;
      const steps = Math.max(10, Math.abs(distance) / 50); // Adjust steps based on distance
      
      for (let i = 0; i < steps; i++) {
        const progress = this.easeInOutCubic(i / steps);
        const currentScroll = currentY + distance * progress;
        
        window.scrollTo(0, currentScroll);
        
        // Dispatch scroll events
        window.dispatchEvent(new Event('scroll', { bubbles: true }));
        
        await this.delay(16); // ~60fps
      }

      // Ensure we're at the exact target
      window.scrollTo(0, currentY + distance);
      window.dispatchEvent(new Event('scroll', { bubbles: true }));

      return true;
    } catch (error) {
      console.error('[ScriptedRung] Scroll execution error:', error);
      return false;
    }
  }

  // Helper methods

  private async clearFieldRealistic(input: HTMLInputElement | HTMLTextAreaElement): Promise<void> {
    // Select all text
    input.select();
    
    // Simulate Ctrl+A then Delete
    const selectAllEvent = new KeyboardEvent('keydown', {
      key: 'a',
      code: 'KeyA',
      ctrlKey: true,
      bubbles: true
    });
    
    const deleteEvent = new KeyboardEvent('keydown', {
      key: 'Delete',
      code: 'Delete',
      bubbles: true
    });

    input.dispatchEvent(selectAllEvent);
    await this.delay(10);
    input.dispatchEvent(deleteEvent);
    
    input.value = '';
    input.dispatchEvent(new InputEvent('input', {
      data: null,
      inputType: 'deleteContent',
      bubbles: true
    }));
  }

  private async typeCharacterRealistic(
    input: HTMLInputElement | HTMLTextAreaElement,
    char: string,
    isLast: boolean
  ): Promise<void> {
    const keyEventOptions = {
      key: char,
      code: this.getKeyCode(char),
      bubbles: true,
      cancelable: true
    };

    // Key events
    input.dispatchEvent(new KeyboardEvent('keydown', keyEventOptions));
    input.dispatchEvent(new KeyboardEvent('keypress', keyEventOptions));
    
    // Update value
    input.value += char;
    
    // Input event with realistic properties
    input.dispatchEvent(new InputEvent('input', {
      data: char,
      inputType: 'insertText',
      bubbles: true,
      cancelable: true
    }));
    
    input.dispatchEvent(new KeyboardEvent('keyup', keyEventOptions));
  }

  private shouldTriggerNativeClick(element: Element): boolean {
    const tagName = element.tagName.toLowerCase();
    return ['button', 'a', 'input'].includes(tagName) ||
           element.hasAttribute('onclick') ||
           element.getAttribute('role') === 'button';
  }

  private getButtonNumber(button: string): number {
    switch (button) {
      case 'left': return 0;
      case 'middle': return 1;
      case 'right': return 2;
      default: return 0;
    }
  }

  private getButtonFlags(button: string): number {
    switch (button) {
      case 'left': return 1;
      case 'right': return 2;
      case 'middle': return 4;
      default: return 1;
    }
  }

  private getKeyCode(key: string): string {
    // Map common keys to their codes
    const keyCodeMap: Record<string, string> = {
      'Enter': 'Enter',
      'Tab': 'Tab',
      'Escape': 'Escape',
      'Space': 'Space',
      'Delete': 'Delete',
      'Backspace': 'Backspace'
    };

    if (keyCodeMap[key]) {
      return keyCodeMap[key];
    }

    // For letter keys
    if (/^[a-zA-Z]$/.test(key)) {
      return `Key${key.toUpperCase()}`;
    }

    // For number keys
    if (/^[0-9]$/.test(key)) {
      return `Digit${key}`;
    }

    return key;
  }

  private getModifierState(modifiers: string[]): Partial<MouseEvent | KeyboardEvent> {
    return {
      ctrlKey: modifiers.includes('ctrl'),
      shiftKey: modifiers.includes('shift'),
      altKey: modifiers.includes('alt'),
      metaKey: modifiers.includes('meta')
    };
  }

  private isElementInteractable(element: Element): boolean {
    // Same logic as DOM rung
    const style = window.getComputedStyle(element);
    if (style.display === 'none' || style.visibility === 'hidden') {
      return false;
    }

    const rect = element.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) {
      return false;
    }

    return true;
  }

  private delay(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
  }

  private easeInOutCubic(t: number): number {
    return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
  }
}
