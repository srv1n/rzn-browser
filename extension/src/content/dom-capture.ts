/**
 * DOM Capture Module - RZN Browser Extension
 * 
 * This module provides efficient DOM traversal and snapshot generation
 * for browser automation workflows. It implements cookbook-based optimizations
 * including spatial grouping, performance caps, and delta generation.
 */

// Interactive elements selector - focused list for better performance
export const INTERACTIVE_SELECTOR = [
  'input:not([type="hidden"])',
  'textarea',
  'select',
  'button',
  'a[href]',
  '[onclick]',
  '[onchange]',
  '[role="button"]',
  '[role="link"]',
  '[role="textbox"]',
  '[role="combobox"]',
  '[role="listbox"]',
  '[role="tab"]',
  '[role="menuitem"]',
  '[tabindex]:not([tabindex="-1"])',
  '[contenteditable="true"]'
].join(',');

/**
 * Check if element is visible using getBoundingClientRect
 */
export function visible(element: Element): boolean {
  if (!(element instanceof HTMLElement)) return false;
  
  const rect = element.getBoundingClientRect();
  const style = window.getComputedStyle(element);
  
  return rect.width > 0 && 
         rect.height > 0 && 
         style.visibility !== 'hidden' && 
         style.display !== 'none' &&
         rect.top < window.innerHeight &&
         rect.bottom > 0 &&
         rect.left < window.innerWidth &&
         rect.right > 0;
}

/**
 * Breadth-first DOM traversal generator
 */
export function* breadthFirst(root: Element = document.body): Generator<Element> {
  const queue: Element[] = [root];
  const visited = new Set<Element>();
  
  while (queue.length > 0) {
    const element = queue.shift()!;
    
    if (visited.has(element)) continue;
    visited.add(element);
    
    yield element;
    
    // Add children to queue
    for (const child of element.children) {
      if (!visited.has(child)) {
        queue.push(child);
      }
    }
  }
}

/**
 * Simplified element representation for snapshots
 */
export interface ElementStub {
  // Stable-ish element identifier for this document lifetime.
  // Format matches rzn_plan EncodedId conventions: "<frame_ordinal>:<numeric_id>".
  // Currently frame_ordinal is always 0 for the top frame snapshot.
  id: string;
  tag: string;
  text?: string;
  attributes: Record<string, string>;
  selector: string;
  spatial_info?: {
    x: number;
    y: number;
    width: number;
    height: number;
    area: number;
    viewport_position: 'top' | 'middle' | 'bottom';
  };
}

// Whitelist of important attributes to capture
export const WHITELIST_ATTRS = [
  'id', 'class', 'name', 'type', 'value', 'placeholder', 'title', 
  'alt', 'src', 'href', 'role', 'aria-label', 'aria-labelledby',
  'aria-describedby', 'data-testid', 'data-cy', 'data-test'
];

// Stable element IDs within the content-script lifetime.
// Avoids index-based ids that change between snapshots.
const elementIds = new WeakMap<Element, string>();
let nextElementId = 0;

function getStableElementId(element: Element): string {
  const existing = elementIds.get(element);
  if (existing) return existing;
  nextElementId += 1;
  const id = `0:${nextElementId}`;
  elementIds.set(element, id);
  return id;
}

/**
 * Generate best CSS selector for an element
 */
export function bestSelector(element: Element): string {
  // Priority order: id > data-testid > unique class > tag + position
  
  // 1. ID selector (most reliable)
  if (element.id) {
    return `#${element.id}`;
  }
  
  // 2. Test ID attributes
  const testId = element.getAttribute('data-testid') || 
                 element.getAttribute('data-cy') || 
                 element.getAttribute('data-test');
  if (testId) {
    const attr = element.getAttribute('data-testid') ? 'data-testid' : 
                  element.getAttribute('data-cy') ? 'data-cy' : 'data-test';
    return `[${attr}="${testId}"]`;
  }
  
  // 3. Unique class combination
  if (element.className && typeof element.className === 'string') {
    const classes = element.className.trim().split(/\s+/).filter(c => c.length > 0);
    if (classes.length > 0) {
      const classSelector = '.' + classes.join('.');
      try {
        if (document.querySelectorAll(classSelector).length === 1) {
          return classSelector;
        }
      } catch (e) {
        // Invalid class selector, continue
      }
    }
  }
  
  // 4. Tag + position as fallback
  const tagName = element.tagName.toLowerCase();
  const parent = element.parentElement;
  
  if (parent) {
    const siblings = Array.from(parent.children).filter(child => 
      child.tagName.toLowerCase() === tagName
    );
    
    if (siblings.length === 1) {
      return tagName;
    } else {
      const index = siblings.indexOf(element);
      return `${tagName}:nth-of-type(${index + 1})`;
    }
  }
  
  return tagName;
}

/**
 * Build DOM snapshot with performance optimizations
 */
export function buildSnapshot(maxElements: number = 120): ElementStub[] {
  const elements: ElementStub[] = [];
  const viewport = {
    height: window.innerHeight,
    width: window.innerWidth
  };
  
  // Find all interactive elements first
  const interactiveElements = document.querySelectorAll(INTERACTIVE_SELECTOR);
  const processedElements = new Set<Element>();
  
  // Process interactive elements with priority
  for (const element of interactiveElements) {
    if (elements.length >= maxElements) break;
    if (!visible(element) || processedElements.has(element)) continue;
    
    processedElements.add(element);
    const rect = element.getBoundingClientRect();
    
    // Determine viewport position for spatial grouping
    let viewportPosition: 'top' | 'middle' | 'bottom';
    const centerY = rect.top + rect.height / 2;
    if (centerY < viewport.height * 0.33) {
      viewportPosition = 'top';
    } else if (centerY < viewport.height * 0.67) {
      viewportPosition = 'middle';
    } else {
      viewportPosition = 'bottom';
    }
    
    // Extract text content (first 100 chars)
    const textContent = element.textContent?.trim().slice(0, 100) || '';
    
    // Build attributes object
    const attributes: Record<string, string> = {};
    for (const attr of WHITELIST_ATTRS) {
      const value = element.getAttribute(attr);
      if (value !== null) {
        attributes[attr] = value;
      }
    }
    
    const stub: ElementStub = {
      id: getStableElementId(element),
      tag: element.tagName.toLowerCase(),
      text: textContent || undefined,
      attributes,
      selector: bestSelector(element),
      spatial_info: {
        x: Math.round(rect.left),
        y: Math.round(rect.top),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
        area: Math.round(rect.width * rect.height),
        viewport_position: viewportPosition
      }
    };
    
    elements.push(stub);
  }
  
  // Fill remaining slots with other visible elements using breadth-first traversal
  if (elements.length < maxElements) {
    for (const element of breadthFirst()) {
      if (elements.length >= maxElements) break;
      if (processedElements.has(element) || !visible(element)) continue;
      
      // Skip if already processed or not interesting
      const tagName = element.tagName.toLowerCase();
      if (['script', 'style', 'meta', 'link', 'title'].includes(tagName)) {
        continue;
      }
      
      processedElements.add(element);
      const rect = element.getBoundingClientRect();
      
      // Determine viewport position
      let viewportPosition: 'top' | 'middle' | 'bottom';
      const centerY = rect.top + rect.height / 2;
      if (centerY < viewport.height * 0.33) {
        viewportPosition = 'top';
      } else if (centerY < viewport.height * 0.67) {
        viewportPosition = 'middle';
      } else {
        viewportPosition = 'bottom';
      }
      
      const textContent = element.textContent?.trim().slice(0, 100) || '';
      
      // Build attributes object
      const attributes: Record<string, string> = {};
      for (const attr of WHITELIST_ATTRS) {
        const value = element.getAttribute(attr);
        if (value !== null) {
          attributes[attr] = value;
        }
      }
      
      const stub: ElementStub = {
        id: getStableElementId(element),
        tag: tagName,
        text: textContent || undefined,
        attributes,
        selector: bestSelector(element),
        spatial_info: {
          x: Math.round(rect.left),
          y: Math.round(rect.top),
          width: Math.round(rect.width),
          height: Math.round(rect.height),
          area: Math.round(rect.width * rect.height),
          viewport_position: viewportPosition
        }
      };
      
      elements.push(stub);
    }
  }
  
  return elements;
}

/**
 * Serialize DOM snapshot to prompt-friendly format
 */
export function toPrompt(elements: ElementStub[]): string {
  const lines: string[] = [];
  
  lines.push('NOTE: Elements are labeled with idx (0-based) and ref (@eN where N=idx+1).');
  lines.push('Prefer using ref in step selectors when possible (e.g., selector="@e3").');

  // Group by viewport position for better spatial understanding
  const indexed = elements.map((element, idx) => ({ element, idx }));
  const grouped = {
    top: indexed.filter(e => e.element.spatial_info?.viewport_position === 'top'),
    middle: indexed.filter(e => e.element.spatial_info?.viewport_position === 'middle'),
    bottom: indexed.filter(e => e.element.spatial_info?.viewport_position === 'bottom')
  };
  
  for (const [position, positionElements] of Object.entries(grouped)) {
    if (positionElements.length === 0) continue;
    
    lines.push(`\n=== ${position.toUpperCase()} OF PAGE ===`);
    
    for (const { element, idx } of positionElements) {
      const ref = `@e${idx + 1}`;
      const parts: string[] = [
        `<${element.tag}`,
        `idx="${idx}"`,
        `ref="${ref}"`,
        `eid="${element.id}"`,
      ];
      
      // Add important attributes
      for (const [key, value] of Object.entries(element.attributes)) {
        if (['id', 'class', 'name', 'type', 'role'].includes(key)) {
          parts.push(`${key}="${value}"`);
        }
      }
      
      parts.push(`selector="${element.selector}"`);
      
      // Add spatial info
      if (element.spatial_info) {
        const spatial = element.spatial_info;
        parts.push(`pos="${spatial.x},${spatial.y}" size="${spatial.width}x${spatial.height}"`);
      }
      
      const openTag = parts.join(' ') + '>';
      
      if (element.text && element.text.length > 0) {
        lines.push(`${openTag}${element.text}</${element.tag}>`);
      } else {
        lines.push(`${openTag}</${element.tag}>`);
      }
    }
  }
  
  return lines.join('\n');
}

/**
 * Generate hash of DOM state for loop detection
 */
export function domHash(): string {
  const snapshot = buildSnapshot(50); // Smaller snapshot for hashing
  const simplified = snapshot.map(el => ({
    tag: el.tag,
    selector: el.selector,
    text: el.text?.slice(0, 50) || '',
    key_attrs: Object.entries(el.attributes)
      .filter(([key]) => ['id', 'class', 'name'].includes(key))
      .map(([key, value]) => `${key}:${value}`)
      .join('|')
  }));
  
  const content = JSON.stringify(simplified);
  
  // Simple hash function
  let hash = 0;
  for (let i = 0; i < content.length; i++) {
    const char = content.charCodeAt(i);
    hash = ((hash << 5) - hash) + char;
    hash = hash & hash; // Convert to 32-bit integer
  }
  
  return Math.abs(hash).toString(16);
}

/**
 * Generate delta between two DOM snapshots
 */
export function diff(oldElements: ElementStub[], newElements: ElementStub[]): {
  added: ElementStub[];
  removed: ElementStub[];
  modified: ElementStub[];
} {
  const oldMap = new Map<string, ElementStub>();
  const newMap = new Map<string, ElementStub>();
  
  // Build maps using selector as key
  for (const el of oldElements) {
    oldMap.set(el.selector, el);
  }
  for (const el of newElements) {
    newMap.set(el.selector, el);
  }
  
  const added: ElementStub[] = [];
  const removed: ElementStub[] = [];
  const modified: ElementStub[] = [];
  
  // Find added elements
  for (const [selector, element] of newMap) {
    if (!oldMap.has(selector)) {
      added.push(element);
    }
  }
  
  // Find removed and modified elements
  for (const [selector, oldElement] of oldMap) {
    const newElement = newMap.get(selector);
    
    if (!newElement) {
      removed.push(oldElement);
    } else {
      // Check if element was modified
      const oldStr = JSON.stringify({
        text: oldElement.text,
        attributes: oldElement.attributes
      });
      const newStr = JSON.stringify({
        text: newElement.text,
        attributes: newElement.attributes
      });
      
      if (oldStr !== newStr) {
        modified.push(newElement);
      }
    }
  }
  
  return { added, removed, modified };
}

/**
 * Export convenience function for getting current page snapshot
 */
export function captureCurrentDOM(maxElements: number = 120): {
  elements: ElementStub[];
  hash: string;
  prompt: string;
  metadata: {
    timestamp: number;
    url: string;
    title: string;
    viewport: { width: number; height: number };
  };
} {
  const elements = buildSnapshot(maxElements);
  const hash = domHash();
  const prompt = toPrompt(elements);
  
  return {
    elements,
    hash,
    prompt,
    metadata: {
      timestamp: Date.now(),
      url: window.location.href,
      title: document.title,
      viewport: {
        width: window.innerWidth,
        height: window.innerHeight
      }
    }
  };
}
