/**
 * Compact Snapshot Builder
 * Builds unified snapshots from AX tree and DOM with EncodedIds
 */

import {
  CompactSnapshot,
  CompactElement,
  FrameContext,
  SnapshotBuilder as ISnapshotBuilder,
  SNAPSHOT_CONFIG,
  ELEMENT_TYPE_MAPPING
} from './types';
import { domAnalyzer } from '../content/dom-analyzer';

export class SnapshotBuilder implements ISnapshotBuilder {
  private elementIdMap = new Map<string, Element>();
  private idCounter = 0;

  /**
   * Build complete snapshot from current DOM state
   */
  async buildSnapshot(): Promise<CompactSnapshot> {
    console.log('[RZN Snapshot] Building compact snapshot...');
    
    // Reset state
    this.elementIdMap.clear();
    this.idCounter = 0;

    // Get DOM analysis
    const domState = domAnalyzer.analyzeDOMTree({
      prioritizeViewport: true,
      viewportExpansion: 100, // Slightly beyond viewport
      maxElements: SNAPSHOT_CONFIG.MAX_ELEMENTS * 2, // Start with more for filtering
      includeTextNodes: false,
      includeShadowDOM: true,
      calculateInteractionScores: true
    });

    // Extract interactive elements
    const interactiveElements = Object.values(domState.elementMap)
      .filter(node => node.type !== 'TEXT_NODE')
      .map(node => node as any)
      .filter(elem => elem.isInteractive && elem.isVisible);

    console.log('[RZN Snapshot] Found', interactiveElements.length, 'interactive elements');

    // Sort by viewport proximity
    const sortedElements = this.sortByViewportProximity(interactiveElements);

    // Build compact elements
    const compactElements: CompactElement[] = [];
    for (const element of sortedElements.slice(0, SNAPSHOT_CONFIG.MAX_ELEMENTS)) {
      const compactElement = await this.buildCompactElement(element);
      if (compactElement) {
        compactElements.push(compactElement);
      }
    }

    // Get frame context
    const frames = this.addFrameContext(Array.from(document.querySelectorAll('iframe')));

    // Create initial snapshot
    const snapshot: CompactSnapshot = {
      url: window.location.href,
      title: this.truncateText(document.title, 60),
      viewport: {
        width: window.innerWidth,
        height: window.innerHeight,
        scrollX: window.scrollX,
        scrollY: window.scrollY
      },
      elements: compactElements,
      frames,
      sizeKB: 0,
      timestamp: Date.now(),
      compressionLevel: 'none'
    };

    // Calculate size and apply compression if needed
    snapshot.sizeKB = this.estimateSnapshotSize(snapshot);
    console.log('[RZN Snapshot] Initial size:', snapshot.sizeKB, 'KB');

    return snapshot;
  }

  /**
   * Build snapshot using accessibility tree (experimental)
   */
  async buildFromAccessibilityTree(): Promise<CompactSnapshot> {
    console.log('[RZN Snapshot] Building from accessibility tree...');
    
    // Fallback to regular DOM for now
    // In the future, this could use Chrome DevTools Protocol to get AX tree
    return this.buildSnapshot();
  }

  /**
   * Sort elements by viewport proximity
   */
  private sortByViewportProximity(elements: any[]): any[] {
    const viewportCenterX = window.innerWidth / 2;
    const viewportCenterY = window.innerHeight / 2;

    return elements
      .map(elem => ({
        ...elem,
        viewportDistance: this.calculateViewportDistance(elem.position, viewportCenterX, viewportCenterY)
      }))
      .sort((a, b) => {
        // First sort by viewport status (in viewport elements first)
        if (a.isInViewport !== b.isInViewport) {
          return a.isInViewport ? -1 : 1;
        }
        // Then by distance from viewport center
        return a.viewportDistance - b.viewportDistance;
      });
  }

  /**
   * Calculate distance from viewport center
   */
  private calculateViewportDistance(position: any, centerX: number, centerY: number): number {
    const elementCenterX = position.left + (position.width / 2);
    const elementCenterY = position.top + (position.height / 2);
    
    const deltaX = Math.abs(elementCenterX - centerX);
    const deltaY = Math.abs(elementCenterY - centerY);
    
    return Math.sqrt(deltaX * deltaX + deltaY * deltaY);
  }

  /**
   * Build compact element with encoded ID
   */
  private async buildCompactElement(domElement: any): Promise<CompactElement | null> {
    try {
      // Find the actual DOM element
      const actualElement = this.findActualElement(domElement);
      if (!actualElement) {
        return null;
      }

      // Generate encoded ID
      const encodedId = this.generateEncodedId(actualElement);
      this.elementIdMap.set(encodedId, actualElement);

      // Get primary selector
      const selector = domElement.selector?.css || this.generateFallbackSelector(actualElement);

      // Build compact element
      const compact: CompactElement = {
        encodedId,
        tag: domElement.tagName,
        selector,
        viewportDistance: domElement.viewportDistance || 0,
        type: this.getElementType(actualElement),
        actions: this.getActionHints(actualElement)
      };

      // Add text content if meaningful
      const text = this.extractMeaningfulText(actualElement);
      if (text) {
        compact.text = this.truncateText(text, SNAPSHOT_CONFIG.MAX_TEXT_LENGTH);
      }

      // Add role and name from accessibility
      const role = actualElement.getAttribute('role') || this.inferRole(actualElement);
      if (role && role !== domElement.tagName) {
        compact.role = role;
      }

      const name = this.getAccessibleName(actualElement);
      if (name && name !== text) {
        compact.name = this.truncateText(name, SNAPSHOT_CONFIG.MAX_TEXT_LENGTH);
      }

      // Add key attributes
      const attrs = this.getKeyAttributes(actualElement);
      if (Object.keys(attrs).length > 0) {
        compact.attrs = attrs;
      }

      return compact;
    } catch (error) {
      console.warn('[RZN Snapshot] Failed to build compact element:', error);
      return null;
    }
  }

  /**
   * Find actual DOM element from DOM analyzer element
   */
  private findActualElement(domElement: any): Element | null {
    if (domElement.selector?.css) {
      return document.querySelector(domElement.selector.css);
    }
    
    // Fallback: try to find by xpath or tag
    if (domElement.xpath) {
      const result = document.evaluate(
        '//' + domElement.xpath,
        document,
        null,
        XPathResult.FIRST_ORDERED_NODE_TYPE,
        null
      );
      return result.singleNodeValue as Element;
    }
    
    return null;
  }

  /**
   * Generate encoded ID for element
   */
  private generateEncodedId(element: Element): string {
    const tagName = element.tagName.toLowerCase();
    const shortType = ELEMENT_TYPE_MAPPING[tagName as keyof typeof ELEMENT_TYPE_MAPPING] || tagName.slice(0, 3);
    return `${shortType}_${++this.idCounter}`;
  }

  /**
   * Generate fallback selector for element
   */
  private generateFallbackSelector(element: Element): string {
    if (element.id) {
      return `#${element.id}`;
    }
    
    if (element.getAttribute('data-testid')) {
      return `[data-testid="${element.getAttribute('data-testid')}"]`;
    }
    
    if (element.className) {
      const classes = element.className.toString().split(' ')
        .filter(c => c && !c.match(/^(w-|h-|p-|m-|text-|bg-)/))
        .slice(0, 2);
      if (classes.length > 0) {
        return element.tagName.toLowerCase() + classes.map(c => `.${c}`).join('');
      }
    }
    
    return element.tagName.toLowerCase();
  }

  /**
   * Extract meaningful text from element
   */
  private extractMeaningfulText(element: Element): string {
    const tagName = element.tagName.toLowerCase();
    
    if (tagName === 'input') {
      const input = element as HTMLInputElement;
      return input.placeholder || input.value || '';
    }
    
    if (tagName === 'textarea') {
      const textarea = element as HTMLTextAreaElement;
      return textarea.placeholder || textarea.value || '';
    }
    
    if (tagName === 'button' || tagName === 'a') {
      return element.textContent?.trim() || '';
    }
    
    // For other elements, get first meaningful text node
    const textNodes = Array.from(element.childNodes)
      .filter(node => node.nodeType === Node.TEXT_NODE);
    
    if (textNodes.length > 0) {
      const text = textNodes[0].textContent?.trim() || '';
      return text;
    }
    
    return element.textContent?.trim() || '';
  }

  /**
   * Get element type for compact representation
   */
  private getElementType(element: Element): string {
    const tagName = element.tagName.toLowerCase();
    
    if (tagName === 'input') {
      const input = element as HTMLInputElement;
      return input.type || 'text';
    }
    
    return tagName;
  }

  /**
   * Get action hints for element
   */
  private getActionHints(element: Element): string[] {
    const hints: string[] = [];
    const tagName = element.tagName.toLowerCase();
    
    switch (tagName) {
      case 'button':
        hints.push('click');
        break;
      case 'input':
        const input = element as HTMLInputElement;
        if (input.type === 'text' || input.type === 'email' || input.type === 'password') {
          hints.push('fill', 'clear');
        } else if (input.type === 'checkbox' || input.type === 'radio') {
          hints.push('check');
        } else if (input.type === 'submit') {
          hints.push('click', 'submit');
        }
        break;
      case 'textarea':
        hints.push('fill', 'clear');
        break;
      case 'select':
        hints.push('select');
        break;
      case 'a':
        hints.push('click');
        break;
      default:
        if (element.getAttribute('onclick') || element.getAttribute('role') === 'button') {
          hints.push('click');
        }
    }
    
    return hints;
  }

  /**
   * Infer role from element characteristics
   */
  private inferRole(element: Element): string | null {
    const tagName = element.tagName.toLowerCase();
    
    const roleMapping: Record<string, string> = {
      'button': 'button',
      'a': 'link',
      'input': 'textbox',
      'textarea': 'textbox',
      'select': 'combobox',
      'form': 'form',
      'nav': 'navigation',
      'main': 'main',
      'aside': 'complementary',
      'header': 'banner',
      'footer': 'contentinfo'
    };
    
    return roleMapping[tagName] || null;
  }

  /**
   * Get accessible name for element
   */
  private getAccessibleName(element: Element): string {
    // Try aria-label first
    const ariaLabel = element.getAttribute('aria-label');
    if (ariaLabel) return ariaLabel;
    
    // Try aria-labelledby
    const labelledBy = element.getAttribute('aria-labelledby');
    if (labelledBy) {
      const labelElement = document.getElementById(labelledBy);
      if (labelElement) {
        return labelElement.textContent?.trim() || '';
      }
    }
    
    // Try associated label
    if (element.id) {
      const label = document.querySelector(`label[for="${element.id}"]`);
      if (label) {
        return label.textContent?.trim() || '';
      }
    }
    
    // Try title attribute
    const title = element.getAttribute('title');
    if (title) return title;
    
    // For buttons and links, use text content
    const tagName = element.tagName.toLowerCase();
    if (tagName === 'button' || tagName === 'a') {
      return element.textContent?.trim() || '';
    }
    
    return '';
  }

  /**
   * Get key attributes for element
   */
  private getKeyAttributes(element: Element): Record<string, string> {
    const attrs: Record<string, string> = {};
    
    const keyAttributes = [
      'type', 'name', 'placeholder', 'value', 'href', 'src', 'alt',
      'data-testid', 'data-test', 'aria-expanded', 'aria-selected'
    ];
    
    let attrCount = 0;
    for (const attr of keyAttributes) {
      if (attrCount >= SNAPSHOT_CONFIG.MAX_ATTRIBUTES) break;
      
      const value = element.getAttribute(attr);
      if (value && value.trim()) {
        attrs[attr] = this.truncateText(value.trim(), 30);
        attrCount++;
      }
    }
    
    return attrs;
  }

  /**
   * Add frame context information
   */
  addFrameContext(frames: HTMLIFrameElement[]): FrameContext[] {
    const frameContexts: FrameContext[] = [];
    
    for (let i = 0; i < frames.length; i++) {
      const frame = frames[i];
      
      try {
        const context: FrameContext = {
          id: `frame_${i}`,
          url: frame.src || 'about:blank',
          origin: new URL(frame.src || window.location.href).origin,
          accessible: false,
          elementCount: 0
        };
        
        // Try to access frame content
        try {
          const doc = frame.contentDocument || frame.contentWindow?.document;
          if (doc) {
            context.accessible = true;
            context.elementCount = doc.querySelectorAll('button, input, select, textarea, a').length;
          }
        } catch (error) {
          // Frame is cross-origin, can't access
        }
        
        frameContexts.push(context);
      } catch (error) {
        console.warn('[RZN Snapshot] Failed to analyze frame:', error);
      }
    }
    
    return frameContexts;
  }

  /**
   * Get element by encoded ID
   */
  getElementById(encodedId: string): Element | null {
    return this.elementIdMap.get(encodedId) || null;
  }

  /**
   * Estimate snapshot size in KB
   */
  private estimateSnapshotSize(snapshot: CompactSnapshot): number {
    const jsonString = JSON.stringify(snapshot);
    return Math.round((new Blob([jsonString]).size) / 1024 * 100) / 100;
  }

  /**
   * Truncate text to maximum length
   */
  private truncateText(text: string, maxLength: number): string {
    if (text.length <= maxLength) {
      return text;
    }
    return text.substring(0, maxLength - 3) + '...';
  }
}

// Export singleton instance
export const snapshotBuilder = new SnapshotBuilder();