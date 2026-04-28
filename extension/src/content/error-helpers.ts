/**
 * Content script helpers for error context capture
 */

export interface ViewportInfo {
  width: number;
  height: number;
  devicePixelRatio: number;
  scrollX: number;
  scrollY: number;
}

/**
 * Get current viewport information
 */
export function getViewportInfo(): ViewportInfo {
  return {
    width: window.innerWidth,
    height: window.innerHeight,
    devicePixelRatio: window.devicePixelRatio || 1,
    scrollX: window.scrollX,
    scrollY: window.scrollY,
  };
}

/**
 * Get DOM snapshot around a selector
 */
export function getDomSnapshot(selector?: string, maxDepth: number = 3): string {
  try {
    let targetElement: Element | null = null;
    
    if (selector) {
      try {
        targetElement = document.querySelector(selector);
      } catch (e) {
        // Invalid selector
        return `Invalid selector: ${selector}`;
      }
    }
    
    // If no element found with selector, capture around viewport center
    if (!targetElement) {
      const centerX = window.innerWidth / 2;
      const centerY = window.innerHeight / 2;
      targetElement = document.elementFromPoint(centerX, centerY);
    }
    
    if (!targetElement) {
      return 'No element found';
    }
    
    // Capture DOM tree around the element
    const snapshot = captureDomTree(targetElement, maxDepth);
    return JSON.stringify(snapshot, null, 2);
  } catch (error) {
    return `Error capturing DOM: ${error}`;
  }
}

/**
 * Recursively capture DOM tree structure
 */
function captureDomTree(element: Element, maxDepth: number, currentDepth: number = 0): any {
  if (currentDepth >= maxDepth) {
    return { tag: element.tagName.toLowerCase(), children: '...' };
  }
  
  const node: any = {
    tag: element.tagName.toLowerCase(),
    id: element.id || undefined,
    classes: element.className ? element.className.split(' ').filter(c => c) : undefined,
    text: element.textContent?.trim().substring(0, 50),
    attributes: {},
  };
  
  // Capture important attributes
  const importantAttrs = ['href', 'src', 'alt', 'title', 'name', 'type', 'value', 'placeholder'];
  for (const attr of importantAttrs) {
    const value = element.getAttribute(attr);
    if (value) {
      node.attributes[attr] = value;
    }
  }
  
  // Capture children
  if (element.children.length > 0) {
    node.children = Array.from(element.children)
      .slice(0, 5) // Limit to 5 children
      .map(child => captureDomTree(child, maxDepth, currentDepth + 1));
    
    if (element.children.length > 5) {
      node.children.push({ tag: '...', moreChildren: element.children.length - 5 });
    }
  }
  
  return node;
}

/**
 * Suggest alternative selectors for an element
 */
export function suggestAlternativeSelectors(originalSelector: string): string[] {
  const suggestions: string[] = [];
  
  try {
    // Try to find the element with the original selector
    const elements = document.querySelectorAll(originalSelector);
    
    if (elements.length === 0) {
      // Try to find similar elements
      suggestions.push(...findSimilarSelectors(originalSelector));
    } else if (elements.length > 1) {
      // Multiple elements found, suggest more specific selectors
      elements.forEach((el, index) => {
        if (index < 3) { // Limit to 3 suggestions
          suggestions.push(...generateSelectorsForElement(el, index));
        }
      });
    } else {
      // Single element found, suggest alternative ways to select it
      suggestions.push(...generateSelectorsForElement(elements[0]));
    }
  } catch (e) {
    // Invalid selector, try to fix common issues
    suggestions.push(...fixCommonSelectorIssues(originalSelector));
  }
  
  // Remove duplicates and the original selector
  return [...new Set(suggestions)].filter(s => s !== originalSelector).slice(0, 5);
}

/**
 * Find similar selectors when original doesn't match
 */
function findSimilarSelectors(selector: string): string[] {
  const suggestions: string[] = [];
  
  // Extract parts from the selector
  const idMatch = selector.match(/#([^\s\[\.]+)/);
  const classMatch = selector.match(/\.([^\s\[#]+)/);
  const tagMatch = selector.match(/^([a-zA-Z]+)/);
  const attrMatch = selector.match(/\[([^\]]+)\]/);
  
  // Try partial matches
  if (idMatch) {
    const id = idMatch[1];
    // Try partial ID match
    const elements = document.querySelectorAll(`[id*="${id}"]`);
    elements.forEach(el => {
      if (el.id) suggestions.push(`#${el.id}`);
    });
  }
  
  if (classMatch) {
    const className = classMatch[1];
    // Try elements with similar class names
    const elements = document.querySelectorAll(`[class*="${className}"]`);
    elements.forEach(el => {
      const classes = Array.from(el.classList).filter(c => c.includes(className));
      if (classes.length > 0) {
        suggestions.push(`.${classes[0]}`);
      }
    });
  }
  
  if (tagMatch && attrMatch) {
    const tag = tagMatch[1];
    const attr = attrMatch[1];
    // Try tag with different attribute values
    const elements = document.querySelectorAll(tag);
    elements.forEach(el => {
      const attrs = Array.from(el.attributes)
        .filter(a => a.name.includes(attr.split('=')[0]))
        .map(a => `${tag}[${a.name}="${a.value}"]`);
      suggestions.push(...attrs);
    });
  }
  
  return suggestions;
}

/**
 * Generate alternative selectors for a specific element
 */
function generateSelectorsForElement(element: Element, index?: number): string[] {
  const selectors: string[] = [];
  
  // ID selector
  if (element.id) {
    selectors.push(`#${element.id}`);
  }
  
  // Class selector
  if (element.className) {
    const classes = element.className.split(' ').filter(c => c && !c.includes(':'));
    if (classes.length > 0) {
      selectors.push(`.${classes.join('.')}`);
      // Also try individual classes
      classes.forEach(c => selectors.push(`.${c}`));
    }
  }
  
  // Tag + attribute selectors
  const tag = element.tagName.toLowerCase();
  
  // Common attributes
  ['name', 'type', 'role', 'data-testid', 'data-test', 'aria-label'].forEach(attr => {
    const value = element.getAttribute(attr);
    if (value) {
      selectors.push(`${tag}[${attr}="${value}"]`);
    }
  });
  
  // Text content selector (for buttons, links)
  if (['button', 'a', 'span', 'div'].includes(tag) && element.textContent) {
    const text = element.textContent.trim();
    if (text.length > 0 && text.length < 50) {
      selectors.push(`${tag}:contains("${text}")`);
    }
  }
  
  // nth-child selector if index provided
  if (index !== undefined && element.parentElement) {
    const parent = element.parentElement;
    const parentSelector = parent.id ? `#${parent.id}` : parent.className ? `.${parent.className.split(' ')[0]}` : parent.tagName.toLowerCase();
    selectors.push(`${parentSelector} > ${tag}:nth-child(${index + 1})`);
  }
  
  // XPath as last resort
  selectors.push(getXPath(element));
  
  return selectors;
}

/**
 * Fix common selector syntax issues
 */
function fixCommonSelectorIssues(selector: string): string[] {
  const suggestions: string[] = [];
  
  // Fix missing quotes in attribute selectors
  const fixedQuotes = selector.replace(/\[([^=]+)=([^\]'"]+)\]/g, '[$1="$2"]');
  if (fixedQuotes !== selector) suggestions.push(fixedQuotes);
  
  // Fix space issues
  if (selector.includes('  ')) {
    suggestions.push(selector.replace(/\s+/g, ' '));
  }
  
  // Try without pseudo-classes
  if (selector.includes(':')) {
    suggestions.push(selector.replace(/:[^:\s]+/g, ''));
  }
  
  // Try simpler versions
  if (selector.includes(' ')) {
    const parts = selector.split(' ');
    // Try last part only
    suggestions.push(parts[parts.length - 1]);
    // Try first and last part
    if (parts.length > 2) {
      suggestions.push(`${parts[0]} ${parts[parts.length - 1]}`);
    }
  }
  
  return suggestions;
}

/**
 * Get XPath for an element
 */
function getXPath(element: Element): string {
  if (element.id) {
    return `//*[@id="${element.id}"]`;
  }
  
  const parts: string[] = [];
  let current: Element | null = element;
  
  while (current && current.nodeType === Node.ELEMENT_NODE) {
    let index = 1;
    let sibling = current.previousElementSibling;
    
    while (sibling) {
      if (sibling.tagName === current.tagName) {
        index++;
      }
      sibling = sibling.previousElementSibling;
    }
    
    const tagName = current.tagName.toLowerCase();
    const part = index > 1 ? `${tagName}[${index}]` : tagName;
    parts.unshift(part);
    
    current = current.parentElement;
  }
  
  return `//${parts.join('/')}`;
}

// Listen for messages from the background script
chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
  switch (request.action) {
    case 'getViewportInfo':
      sendResponse(getViewportInfo());
      break;
      
    case 'getDomSnapshot':
      sendResponse(getDomSnapshot(request.selector));
      break;
      
    case 'suggestSelectors':
      sendResponse(suggestAlternativeSelectors(request.selector));
      break;
      
    default:
      sendResponse(null);
  }
  
  return true; // Keep the message channel open for async response
});