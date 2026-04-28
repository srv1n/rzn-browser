// Element Resolution System with Stable EncodedId References
// Converts TargetSpec to ResolvedElement with frameOrdinal:backendNodeId

import { 
  TargetSpec, 
  ResolvedElement, 
  EncodedId, 
  createEncodedId, 
  parseEncodedId,
  requiresCrossOriginHandling 
} from '../types/targets';
import { ElementCache } from './cache';

// CDP integration for getting backend node IDs
declare global {
  interface Window {
    __RZN_CDP_AVAILABLE__: boolean;
  }
}

class ElementResolver {
  private cache: ElementCache;
  
  constructor() {
    this.cache = new ElementCache();
  }

  /**
   * Resolves a TargetSpec to a ResolvedElement with stable EncodedId
   */
  async resolve(target: TargetSpec): Promise<ResolvedElement> {
    // Check cache first if we have an encoded_id
    if (target.encoded_id) {
      const cached = this.cache.get(target.encoded_id);
      if (cached && this.isElementStillValid(cached)) {
        return cached;
      }
    }

    // Determine frame context
    const frameOrdinal = target.frame_ordinal ?? 0;
    const isCrossOrigin = requiresCrossOriginHandling(target, 0);

    let element: Element | null = null;
    let backendNodeId: number;

    // Find the element using available targeting methods
    if (target.encoded_id) {
      element = await this.findByEncodedId(target.encoded_id);
    } else if (target.css) {
      element = await this.findByCss(target.css, frameOrdinal);
    } else if (target.xpath) {
      element = await this.findByXPath(target.xpath, frameOrdinal);
    } else if (target.role_name) {
      element = await this.findByRole(target.role_name, frameOrdinal);
    } else if (target.text_near) {
      element = await this.findByTextNear(target.text_near, frameOrdinal);
    } else {
      throw new Error('No valid targeting method provided in TargetSpec');
    }

    if (!element) {
      throw new Error(`Element not found using target spec: ${JSON.stringify(target)}`);
    }

    // Get backend node ID from CDP
    backendNodeId = await this.getBackendNodeId(element, frameOrdinal);
    
    if (!backendNodeId) {
      throw new Error('Failed to get backend node ID from CDP');
    }

    // Get element bounds
    const bounds = element.getBoundingClientRect();
    
    // Create stable encoded ID
    const encodedId = createEncodedId(frameOrdinal, backendNodeId);

    // Create resolved element
    const resolved: ResolvedElement = {
      encoded_id: encodedId,
      frame_ordinal: frameOrdinal,
      backend_node_id: backendNodeId,
      bounds: {
        x: bounds.left,
        y: bounds.top,
        width: bounds.width,
        height: bounds.height
      },
      is_cross_origin: isCrossOrigin,
      target_spec: target,
      resolved_at: Date.now()
    };

    // Cache the resolved element
    this.cache.set(encodedId, resolved);

    return resolved;
  }

  /**
   * Resolves element by existing EncodedId
   */
  private async findByEncodedId(encodedId: EncodedId): Promise<Element | null> {
    const { frameOrdinal, backendNodeId } = parseEncodedId(encodedId);
    
    try {
      // In content script, we can't use CDP directly
      // Try to find element by its position in the DOM (for pseudo backend node IDs)
      if (backendNodeId >= 1000000) {
        // This is a pseudo backend node ID we generated
        const index = backendNodeId - 1000000;
        const allElements = document.querySelectorAll('*');
        if (index < allElements.length) {
          return allElements[index] as Element;
        }
      }
      console.warn('Cannot resolve encoded_id directly without proper CDP integration');
      return null;
    } catch (error) {
      console.warn(`Failed to resolve encoded_id ${encodedId}:`, error);
      return null;
    }
  }

  /**
   * Find element by CSS selector
   */
  private async findByCss(css: string, frameOrdinal: number): Promise<Element | null> {
    if (frameOrdinal === 0) {
      // Same-origin, use direct DOM access
      return document.querySelector(css);
    } else {
      // Cross-origin, need CDP
      return await this.findElementByCDPSelector(css, frameOrdinal);
    }
  }

  /**
   * Find element by XPath
   */
  private async findByXPath(xpath: string, frameOrdinal: number): Promise<Element | null> {
    if (frameOrdinal === 0) {
      // Same-origin, use DOM XPath evaluation
      const result = document.evaluate(
        xpath,
        document,
        null,
        XPathResult.FIRST_ORDERED_NODE_TYPE,
        null
      );
      return result.singleNodeValue as Element | null;
    } else {
      // Cross-origin, need CDP
      return await this.findElementByCDPXPath(xpath, frameOrdinal);
    }
  }

  /**
   * Find element by accessibility role
   */
  private async findByRole(roleName: string, frameOrdinal: number): Promise<Element | null> {
    if (frameOrdinal === 0) {
      // Same-origin, use DOM query
      const selector = `[role="${roleName}"]`;
      return document.querySelector(selector);
    } else {
      // Cross-origin, need CDP
      const selector = `[role="${roleName}"]`;
      return await this.findElementByCDPSelector(selector, frameOrdinal);
    }
  }

  /**
   * Find element by nearby text content
   */
  private async findByTextNear(textNear: string, frameOrdinal: number): Promise<Element | null> {
    if (frameOrdinal === 0) {
      // Same-origin, use DOM traversal
      const walker = document.createTreeWalker(
        document.body,
        NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT
      );

      let node: Node | null;
      while (node = walker.nextNode()) {
        if (node.nodeType === Node.TEXT_NODE) {
          const textContent = node.textContent?.trim();
          if (textContent && textContent.includes(textNear)) {
            // Found text, look for nearest interactive element
            let parent = node.parentElement;
            while (parent) {
              if (this.isInteractiveElement(parent)) {
                return parent;
              }
              parent = parent.parentElement;
            }
          }
        }
      }
      return null;
    } else {
      // Cross-origin, need CDP
      return await this.findElementByCDPText(textNear, frameOrdinal);
    }
  }

  /**
   * Get backend node ID using CDP
   */
  private async getBackendNodeId(element: Element, frameOrdinal: number): Promise<number> {
    // In content script context, we can't directly use CDP
    // Generate a pseudo backend node ID based on element position in DOM
    // This is a temporary solution until we properly route CDP calls through the background script
    const allElements = document.querySelectorAll('*');
    const index = Array.from(allElements).indexOf(element);
    if (index === -1) {
      throw new Error('Element not found in DOM for backend node ID generation');
    }
    // Use a high number range to avoid conflicts with real backend node IDs
    return 1000000 + index;

    try {
      // This would interface with CDP to get the backend node ID
      // For now, we'll simulate this with a placeholder
      // In real implementation, this would call CDP DOM.requestNode or similar
      const mockBackendNodeId = this.generateMockBackendNodeId(element);
      return mockBackendNodeId;
    } catch (error) {
      throw new Error(`Failed to get backend node ID: ${error}`);
    }
  }

  /**
   * CDP-based element finding (placeholder implementation)
   */
  private async findElementByCDP(backendNodeId: number, frameOrdinal: number): Promise<Element | null> {
    // This would use CDP DOM.resolveNode to get the element
    console.warn('CDP element resolution not fully implemented');
    return null;
  }

  private async findElementByCDPSelector(selector: string, frameOrdinal: number): Promise<Element | null> {
    // This would use CDP DOM.querySelector in the specified frame
    console.warn('CDP selector resolution not fully implemented');
    return null;
  }

  private async findElementByCDPXPath(xpath: string, frameOrdinal: number): Promise<Element | null> {
    // This would use CDP DOM.performSearch with xpath
    console.warn('CDP XPath resolution not fully implemented');
    return null;
  }

  private async findElementByCDPText(text: string, frameOrdinal: number): Promise<Element | null> {
    // This would use CDP DOM.performSearch to find text
    console.warn('CDP text resolution not fully implemented');
    return null;
  }

  /**
   * Check if element is interactive (clickable, fillable, etc.)
   */
  private isInteractiveElement(element: Element): boolean {
    const tagName = element.tagName.toLowerCase();
    const role = element.getAttribute('role');
    
    // Standard interactive elements
    const interactiveTags = ['a', 'button', 'input', 'select', 'textarea'];
    if (interactiveTags.includes(tagName)) {
      return true;
    }

    // Elements with click handlers or interactive roles
    if (element.hasAttribute('onclick') || 
        element.hasAttribute('onmousedown') ||
        role && ['button', 'link', 'textbox', 'combobox'].includes(role)) {
      return true;
    }

    // Elements with tabindex
    if (element.hasAttribute('tabindex')) {
      return true;
    }

    return false;
  }

  /**
   * Check if resolved element is still valid (exists in DOM)
   */
  private isElementStillValid(resolved: ResolvedElement): boolean {
    try {
      const { frameOrdinal, backendNodeId } = parseEncodedId(resolved.encoded_id);
      
      // For same-origin frames, we can check DOM directly
      if (frameOrdinal === 0) {
        // This is a simplified check - real implementation would verify the actual element
        const mockElement = document.querySelector(`[data-backend-node="${backendNodeId}"]`);
        return mockElement !== null;
      }
      
      // For cross-origin frames, assume valid for now (CDP would verify)
      // Real implementation would use CDP to check element existence
      const cacheAge = Date.now() - resolved.resolved_at;
      return cacheAge < 30000; // 30 second validity for cross-origin
    } catch {
      return false;
    }
  }

  /**
   * Generate mock backend node ID (placeholder for real CDP integration)
   */
  private generateMockBackendNodeId(element: Element): number {
    // In real implementation, this would come from CDP
    // For now, generate a stable ID based on element properties
    let hash = 0;
    const str = element.tagName + (element.id || '') + (element.className || '');
    for (let i = 0; i < str.length; i++) {
      const char = str.charCodeAt(i);
      hash = ((hash << 5) - hash) + char;
      hash = hash & hash; // Convert to 32-bit integer
    }
    return Math.abs(hash);
  }

  /**
   * Clear element cache
   */
  clearCache(): void {
    this.cache.clear();
  }

  /**
   * Get cache statistics
   */
  getCacheStats(): { size: number; hitRate: number; totalRequests: number } {
    return this.cache.getStats();
  }
}

// Export singleton instance
export const elementResolver = new ElementResolver();