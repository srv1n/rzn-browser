// DOM Input Rung - Native DOM events (same-origin only)
// Most stealthy but limited to same-origin contexts

import { ResolvedElement, parseEncodedId } from '../../types/targets';
import { InputAction } from '../ladder';

export class DOMInputExecutor {
  /**
   * Check if DOM events can be used for this element/action combination
   */
  canExecute(element: ResolvedElement, action: InputAction): boolean {
    // DOM events only work for same-origin elements
    if (element.is_cross_origin) {
      return false;
    }

    // All basic actions are supported
    return ['click', 'fill', 'key', 'hover', 'scroll'].includes(action.type);
  }

  /**
   * Execute action using native DOM events
   */
  async execute(element: ResolvedElement, action: InputAction): Promise<boolean> {
    try {
      const domElement = await this.resolveElement(element);
      if (!domElement) {
        console.warn('[DOMRung] Element not found in DOM');
        return false;
      }

      // Ensure element is visible and interactable
      if (!this.isElementInteractable(domElement)) {
        console.debug('[DOMRung] Element is not interactable');
        return false;
      }

      // Execute the specific action
      switch (action.type) {
        case 'click':
          return this.executeClick(domElement, action);
        case 'fill':
          return this.executeFill(domElement, action);
        case 'key':
          return this.executeKey(domElement, action);
        case 'hover':
          return this.executeHover(domElement, action);
        case 'scroll':
          return this.executeScroll(domElement, action);
        default:
          console.warn(`[DOMRung] Unsupported action type: ${action.type}`);
          return false;
      }
    } catch (error) {
      console.error('[DOMRung] Execution error:', error);
      return false;
    }
  }

  private async resolveElement(resolved: ResolvedElement): Promise<Element | null> {
    // For DOM rung, we need to find the actual DOM element
    // This could use various methods depending on what we have available
    
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

    // Try to find by encoded ID (requires DOM augmentation or data attributes)
    const { backendNodeId } = parseEncodedId(resolved.encoded_id);
    const elementWithNodeId = document.querySelector(`[data-backend-node="${backendNodeId}"]`);
    if (elementWithNodeId) {
      return elementWithNodeId;
    }

    console.warn('[DOMRung] Could not resolve element to DOM node');
    return null;
  }

	  private executeClick(element: Element, action: InputAction): boolean {
	    try {
	      const eventInit: MouseEventInit = {
	        bubbles: true,
	        cancelable: true,
	        view: window,
	        button: this.getButtonNumber(action.options?.button || 'left')
	      };

      if (action.options?.modifiers) {
        Object.assign(eventInit, this.getModifierState(action.options.modifiers));
      }

	      // Create and dispatch a native click event
	      const clickEvent = new MouseEvent('click', eventInit);

	      // Note: dispatchEvent() returns false when a listener calls preventDefault().
	      // That does NOT mean the click failed (many SPAs preventDefault on click).
	      element.dispatchEvent(clickEvent);
	      
	      // For clickable elements, also try the native click method
	      if ('click' in element && typeof (element as any).click === 'function') {
	        (element as any).click();
	      }

	      return true;
	    } catch (error) {
	      console.error('[DOMRung] Click execution error:', error);
	      return false;
	    }
	  }

  private executeFill(element: Element, action: InputAction): boolean {
    try {
      if (!action.value) {
        console.warn('[DOMRung] No value provided for fill action');
        return false;
      }

      const inputElement = element as HTMLInputElement | HTMLTextAreaElement;
      
      // Check if it's a fillable element
      if (!('value' in inputElement)) {
        console.warn('[DOMRung] Element is not fillable');
        return false;
      }

      // Focus the element first
      inputElement.focus();

      // Clear existing value
      inputElement.value = '';

      // Set the new value
      inputElement.value = action.value;

      // Dispatch input and change events
      inputElement.dispatchEvent(new InputEvent('input', {
        data: action.value,
        bubbles: true,
        cancelable: true
      }));

      inputElement.dispatchEvent(new Event('change', {
        bubbles: true,
        cancelable: true
      }));

      return true;
    } catch (error) {
      console.error('[DOMRung] Fill execution error:', error);
      return false;
    }
  }

  private executeKey(element: Element, action: InputAction): boolean {
    try {
      if (!action.key) {
        console.warn('[DOMRung] No key provided for key action');
        return false;
      }

      // Focus the element
      if ('focus' in element) {
        (element as any).focus();
      }

      const eventInit: KeyboardEventInit = {
        key: action.key,
        bubbles: true,
        cancelable: true
      };

      if (action.options?.modifiers) {
        Object.assign(eventInit, this.getModifierState(action.options.modifiers));
      }

      // Create key events
      const keyDownEvent = new KeyboardEvent('keydown', eventInit);
      const keyUpEvent = new KeyboardEvent('keyup', eventInit);

      // Dispatch events
      const downResult = element.dispatchEvent(keyDownEvent);
      const upResult = element.dispatchEvent(keyUpEvent);

      return downResult && upResult;
    } catch (error) {
      console.error('[DOMRung] Key execution error:', error);
      return false;
    }
  }

  private executeHover(element: Element, action: InputAction): boolean {
    try {
      // Create mouse events for hover
      const mouseOverEvent = new MouseEvent('mouseover', {
        bubbles: true,
        cancelable: true,
        view: window
      });

      const mouseEnterEvent = new MouseEvent('mouseenter', {
        bubbles: false, // mouseenter doesn't bubble
        cancelable: true,
        view: window
      });

      // Dispatch hover events
      const overResult = element.dispatchEvent(mouseOverEvent);
      const enterResult = element.dispatchEvent(mouseEnterEvent);

      return overResult && enterResult;
    } catch (error) {
      console.error('[DOMRung] Hover execution error:', error);
      return false;
    }
  }

  private executeScroll(element: Element, action: InputAction): boolean {
    try {
      // Scroll element into view
      element.scrollIntoView({
        behavior: 'smooth',
        block: 'center',
        inline: 'center'
      });

      // Dispatch scroll event
      const scrollEvent = new Event('scroll', {
        bubbles: true,
        cancelable: true
      });

      window.dispatchEvent(scrollEvent);

      return true;
    } catch (error) {
      console.error('[DOMRung] Scroll execution error:', error);
      return false;
    }
  }

  private isElementInteractable(element: Element): boolean {
    // Check if element is visible
    const style = window.getComputedStyle(element);
    if (style.display === 'none' || style.visibility === 'hidden') {
      return false;
    }

    // Check if element has size
    const rect = element.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) {
      return false;
    }

    // Check if element is in viewport (at least partially)
    if (rect.bottom < 0 || rect.right < 0 || 
        rect.top > window.innerHeight || rect.left > window.innerWidth) {
      return false;
    }

    // Check if element is not covered by another element
    const centerX = rect.left + rect.width / 2;
    const centerY = rect.top + rect.height / 2;
    const topElement = document.elementFromPoint(centerX, centerY);
    
    if (topElement && !element.contains(topElement) && !topElement.contains(element)) {
      // Element might be covered, but this could be a false positive
      // for complex layouts, so we'll still allow interaction
      console.debug('[DOMRung] Element might be covered by another element');
    }

    return true;
  }

  private getButtonNumber(button: string): number {
    switch (button) {
      case 'left': return 0;
      case 'middle': return 1;
      case 'right': return 2;
      default: return 0;
    }
  }

  private getModifierState(modifiers: string[]): Partial<MouseEvent | KeyboardEvent> {
    return {
      ctrlKey: modifiers.includes('ctrl'),
      shiftKey: modifiers.includes('shift'),
      altKey: modifiers.includes('alt'),
      metaKey: modifiers.includes('meta')
    };
  }
}
