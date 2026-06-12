/**
 * Snapshot Reducer - Reduces snapshots to 2-8KB for LLM consumption
 */

import {
  CompactSnapshot,
  CompactElement,
  SnapshotReducer as ISnapshotReducer,
  SNAPSHOT_CONFIG
} from './types';

export class SnapshotReducer implements ISnapshotReducer {
  
  /**
   * Reduce snapshot to target size
   */
  reduce(snapshot: CompactSnapshot, maxKB: number = SNAPSHOT_CONFIG.MAX_SIZE_KB): CompactSnapshot {
    console.log('[RZN Reducer] Reducing snapshot from', snapshot.sizeKB, 'KB to max', maxKB, 'KB');
    
    let reduced = { ...snapshot };
    
    // First pass: light compression
    if (reduced.sizeKB > maxKB) {
      reduced = this.compress(reduced, 'light');
      reduced.sizeKB = this.estimateSize(reduced);
      console.log('[RZN Reducer] After light compression:', reduced.sizeKB, 'KB');
    }
    
    // Second pass: aggressive compression if still too large
    if (reduced.sizeKB > maxKB) {
      reduced = this.compress(reduced, 'aggressive');
      reduced.sizeKB = this.estimateSize(reduced);
      console.log('[RZN Reducer] After aggressive compression:', reduced.sizeKB, 'KB');
    }
    
    // Final pass: element reduction if still too large
    if (reduced.sizeKB > maxKB) {
      reduced = this.reduceElements(reduced, maxKB);
      reduced.sizeKB = this.estimateSize(reduced);
      console.log('[RZN Reducer] After element reduction:', reduced.sizeKB, 'KB');
    }
    
    return reduced;
  }

  /**
   * Estimate snapshot size in KB
   */
  estimateSize(snapshot: CompactSnapshot): number {
    try {
      const jsonString = JSON.stringify(snapshot);
      return Math.round((new Blob([jsonString]).size) / 1024 * 100) / 100;
    } catch (error) {
      console.warn('[RZN Reducer] Failed to estimate size:', error);
      return 0;
    }
  }

  /**
   * Apply compression strategies
   */
  compress(snapshot: CompactSnapshot, level: 'light' | 'aggressive'): CompactSnapshot {
    const compressed = { ...snapshot };
    compressed.compressionLevel = level;
    
    if (level === 'light') {
      compressed.elements = this.applyLightCompression(snapshot.elements);
    } else {
      compressed.elements = this.applyAggressiveCompression(snapshot.elements);
    }
    
    // Compress other fields
    compressed.title = this.truncateText(compressed.title, level === 'light' ? 40 : 20);
    compressed.url = this.compressUrl(compressed.url, level === 'light');
    
    return compressed;
  }

  /**
   * Apply light compression to elements
   */
  private applyLightCompression(elements: CompactElement[]): CompactElement[] {
    return elements.map(elem => {
      const compressed = { ...elem };
      
      // Truncate text content
      if (compressed.text) {
        compressed.text = this.truncateText(compressed.text, 30);
      }
      
      if (compressed.name && compressed.name !== compressed.text) {
        compressed.name = this.truncateText(compressed.name, 25);
      }
      
      // Remove redundant role if it matches tag
      if (compressed.role === compressed.tag) {
        delete compressed.role;
      }
      
      // Compress attributes
      if (compressed.attrs) {
        compressed.attrs = this.compressAttributes(compressed.attrs, 2);
      }
      
      // Compress selector
      compressed.selector = this.compressSelector(compressed.selector, false);
      
      return compressed;
    });
  }

  /**
   * Apply aggressive compression to elements
   */
  private applyAggressiveCompression(elements: CompactElement[]): CompactElement[] {
    return elements.map(elem => {
      const compressed: CompactElement = {
        encodedId: elem.encodedId,
        tag: this.abbreviateTag(elem.tag),
        selector: this.compressSelector(elem.selector, true),
        viewportDistance: Math.round(elem.viewportDistance)
      };
      
      // Only include most essential text
      if (elem.text && elem.text.length > 5) {
        compressed.text = this.truncateText(elem.text, 15);
      } else if (elem.name && elem.name.length > 5) {
        compressed.text = this.truncateText(elem.name, 15);
      }
      
      // Only include type if different from tag
      if (elem.type && elem.type !== elem.tag) {
        compressed.type = elem.type.slice(0, 4); // Abbreviate type
      }
      
      // Include only most critical attributes
      if (elem.attrs) {
        const criticalAttrs = this.getCriticalAttributes(elem.attrs);
        if (Object.keys(criticalAttrs).length > 0) {
          compressed.attrs = criticalAttrs;
        }
      }
      
      // Include only primary action
      if (elem.actions && elem.actions.length > 0) {
        compressed.actions = [elem.actions[0]];
      }
      
      return compressed;
    });
  }

  /**
   * Reduce number of elements to fit size constraint
   */
  private reduceElements(snapshot: CompactSnapshot, maxKB: number): CompactSnapshot {
    const reduced = { ...snapshot };
    
    // Sort elements by priority score
    const scoredElements = reduced.elements.map(elem => ({
      element: elem,
      score: this.calculateElementPriority(elem)
    }));
    
    scoredElements.sort((a, b) => b.score - a.score);
    
    // Binary search to find optimal element count
    let low = 1;
    let high = scoredElements.length;
    let bestElements: CompactElement[] = [];
    
    while (low <= high) {
      const mid = Math.floor((low + high) / 2);
      const testElements = scoredElements.slice(0, mid).map(s => s.element);
      
      const testSnapshot = { ...reduced, elements: testElements };
      const testSize = this.estimateSize(testSnapshot);
      
      if (testSize <= maxKB) {
        bestElements = testElements;
        low = mid + 1;
      } else {
        high = mid - 1;
      }
    }
    
    reduced.elements = bestElements;
    console.log('[RZN Reducer] Reduced to', bestElements.length, 'elements');
    
    return reduced;
  }

  /**
   * Calculate element priority for reduction
   */
  private calculateElementPriority(element: CompactElement): number {
    let score = 0;
    
    // Viewport proximity (closer = higher priority)
    score += Math.max(0, 100 - element.viewportDistance / 10);
    
    // Element type priority
    const typePriority: Record<string, number> = {
      'button': 25, 'input': 25, 'select': 20, 'textarea': 20,
      'a': 15, 'form': 10, 'div': 5, 'span': 3
    };
    score += typePriority[element.tag] || 5;
    
    // Action availability
    if (element.actions && element.actions.length > 0) {
      score += 10 + element.actions.length * 2;
    }
    
    // Text content (meaningful text is valuable)
    if (element.text && element.text.length > 3) {
      score += Math.min(10, element.text.length / 2);
    }
    
    // Accessibility info (role/name)
    if (element.role) score += 5;
    if (element.name) score += 5;
    
    // Critical attributes
    if (element.attrs) {
      const criticalAttrs = ['data-testid', 'id', 'name', 'type'];
      for (const attr of criticalAttrs) {
        if (element.attrs[attr]) {
          score += 8;
          break;
        }
      }
    }
    
    return score;
  }

  /**
   * Compress URL for size reduction
   */
  private compressUrl(url: string, isLight: boolean): string {
    try {
      const urlObj = new URL(url);
      
      if (isLight) {
        // Light compression: remove fragment and some params
        urlObj.hash = '';
        const paramsToRemove = ['utm_source', 'utm_medium', 'utm_campaign', 'fbclid', 'gclid'];
        for (const param of paramsToRemove) {
          urlObj.searchParams.delete(param);
        }
        return urlObj.toString();
      } else {
        // Aggressive compression: keep only origin and pathname
        return urlObj.origin + urlObj.pathname;
      }
    } catch {
      return url.slice(0, isLight ? 80 : 40);
    }
  }

  /**
   * Compress element attributes
   */
  private compressAttributes(attrs: Record<string, string>, maxCount: number): Record<string, string> {
    const priority = ['data-testid', 'id', 'name', 'type', 'placeholder', 'href'];
    const compressed: Record<string, string> = {};
    
    let count = 0;
    for (const key of priority) {
      if (count >= maxCount) break;
      if (attrs[key]) {
        compressed[key] = this.truncateText(attrs[key], 20);
        count++;
      }
    }
    
    // Add other attributes if space allows
    for (const [key, value] of Object.entries(attrs)) {
      if (count >= maxCount) break;
      if (!priority.includes(key)) {
        compressed[key] = this.truncateText(value, 15);
        count++;
      }
    }
    
    return compressed;
  }

  /**
   * Get only critical attributes for aggressive compression
   */
  private getCriticalAttributes(attrs: Record<string, string>): Record<string, string> {
    const critical: Record<string, string> = {};
    const criticalKeys = ['data-testid', 'id', 'name', 'type'];
    
    for (const key of criticalKeys) {
      if (attrs[key]) {
        critical[key] = this.truncateText(attrs[key], 15);
        break; // Only keep the first critical attribute found
      }
    }
    
    return critical;
  }

  /**
   * Compress CSS selector for size reduction
   */
  private compressSelector(selector: string, aggressive: boolean): string {
    if (!aggressive) {
      return selector.length > 50 ? selector.slice(0, 47) + '...' : selector;
    }
    
    // Aggressive compression: keep only most specific part
    if (selector.includes('#')) {
      const idMatch = selector.match(/#[\w-]+/);
      if (idMatch) return idMatch[0];
    }
    
    if (selector.includes('[data-testid')) {
      const testIdMatch = selector.match(/\[data-testid="[^"]+"\]/);
      if (testIdMatch) return testIdMatch[0];
    }
    
    if (selector.includes('[name')) {
      const nameMatch = selector.match(/\[name="[^"]+"\]/);
      if (nameMatch) return nameMatch[0];
    }
    
    // Return first class or tag
    const parts = selector.split(' ')[0];
    return parts.length > 25 ? parts.slice(0, 22) + '...' : parts;
  }

  /**
   * Abbreviate HTML tag names for compression
   */
  private abbreviateTag(tag: string): string {
    const abbreviations: Record<string, string> = {
      'button': 'btn',
      'input': 'inp',
      'textarea': 'txt',
      'select': 'sel',
      'option': 'opt',
      'anchor': 'a'
    };
    
    return abbreviations[tag] || tag.slice(0, 3);
  }

  /**
   * Truncate text to specified length
   */
  private truncateText(text: string, maxLength: number): string {
    if (text.length <= maxLength) {
      return text;
    }
    return text.substring(0, maxLength - 3) + '...';
  }
}

// Export singleton instance
export const snapshotReducer = new SnapshotReducer();