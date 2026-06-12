// Static action runner - all primitives pre-registered for CSP compliance
// No dynamic code execution, no eval, no new Function

interface ActionResult {
  ok: boolean;
  data?: any;
  err?: string;
}

// Helper to find elements by index or selector
function findElement(sel: number | string): Element {
  const el = typeof sel === 'number' 
    ? document.querySelector(`[data-rzn-idx="${sel}"]`) 
    : document.querySelector(sel as string);
  
  if (!el) {
    throw new Error(`RZN_SELECTOR_NOT_FOUND:${sel}`);
  }
  return el;
}

// Helper for human-like delays
function delay(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

// Pre-registered action primitives
export const RZN_ACTIONS = {
  // Navigation
  navigate: (url: string) => {
    window.location.href = url;
    return true;
  },
  
  go_back: () => {
    window.history.back();
    return true;
  },
  
  go_forward: () => {
    window.history.forward();
    return true;
  },
  
  refresh: () => {
    window.location.reload();
    return true;
  },
  
  // Element interactions
  click: async (sel: number | string, opts: any = {}) => {
    const el = findElement(sel) as HTMLElement;
    
    // Scroll into view if needed
    el.scrollIntoView({ behavior: 'smooth', block: 'center' });
    await delay(100);
    
    // Simulate human-like click
    const rect = el.getBoundingClientRect();
    const x = rect.left + rect.width / 2;
    const y = rect.top + rect.height / 2;
    
    const events = [
      new MouseEvent('mouseenter', { bubbles: true, clientX: x, clientY: y }),
      new MouseEvent('mouseover', { bubbles: true, clientX: x, clientY: y }),
      new MouseEvent('mousedown', { bubbles: true, clientX: x, clientY: y }),
      new MouseEvent('mouseup', { bubbles: true, clientX: x, clientY: y }),
      new MouseEvent('click', { bubbles: true, clientX: x, clientY: y })
    ];
    
    for (const event of events) {
      el.dispatchEvent(event);
      await delay(20);
    }
    
    return true;
  },
  
  type: async (sel: number | string, text: string) => {
    const el = findElement(sel) as HTMLInputElement | HTMLTextAreaElement;
    
    // Focus element
    el.focus();
    await delay(100);
    
    // Clear existing value
    el.select();
    await delay(50);
    el.value = '';
    el.dispatchEvent(new Event('input', { bubbles: true }));
    await delay(50);
    
    // Type each character
    for (const char of text) {
      el.value += char;
      el.dispatchEvent(new Event('input', { bubbles: true }));
      await delay(50 + Math.random() * 100); // Human-like typing speed
    }
    
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return true;
  },
  
  press: async (key: string) => {
    const activeElement = document.activeElement || document.body;
    
    const keyEvent = new KeyboardEvent('keydown', {
      key: key,
      code: key === 'Enter' ? 'Enter' : key === 'Tab' ? 'Tab' : key === 'Escape' ? 'Escape' : `Key${key.toUpperCase()}`,
      bubbles: true,
      cancelable: true
    });
    
    activeElement.dispatchEvent(keyEvent);
    await delay(50);
    
    activeElement.dispatchEvent(new KeyboardEvent('keyup', {
      key: key,
      code: keyEvent.code,
      bubbles: true
    }));
    
    // Special handling for Enter on forms
    if (key === 'Enter' && activeElement instanceof HTMLInputElement) {
      const form = activeElement.closest('form');
      if (form) {
        form.requestSubmit();
      }
    }
    
    return true;
  },
  
  // Scrolling
  scroll: async (direction: 'up' | 'down', pages: number = 1) => {
    window.scrollBy({
      top: pages * window.innerHeight * (direction === 'down' ? 1 : -1),
      behavior: 'smooth'
    });
    await delay(500); // Wait for scroll to complete
    return true;
  },
  
  scroll_to_element: async (sel: number | string) => {
    const el = findElement(sel);
    el.scrollIntoView({ behavior: 'smooth', block: 'center' });
    await delay(500);
    return true;
  },
  
  // Data extraction
  get_text: (sel: number | string) => {
    const el = findElement(sel);
    return el.textContent?.trim() || '';
  },
  
  get_value: (sel: number | string) => {
    const el = findElement(sel) as HTMLInputElement;
    return el.value || '';
  },
  
  get_attr: (sel: number | string, name: string) => {
    const el = findElement(sel);
    return el.getAttribute(name);
  },
  
  get_href: (sel: number | string) => {
    const el = findElement(sel) as HTMLAnchorElement;
    return el.href || '';
  },
  
  get_src: (sel: number | string) => {
    const el = findElement(sel) as HTMLImageElement;
    return el.src || '';
  },
  
  // Element state
  is_visible: (sel: number | string) => {
    try {
      const el = findElement(sel) as HTMLElement;
      const rect = el.getBoundingClientRect();
      const style = window.getComputedStyle(el);
      
      return rect.width > 0 && 
             rect.height > 0 && 
             style.display !== 'none' && 
             style.visibility !== 'hidden' &&
             style.opacity !== '0';
    } catch {
      return false;
    }
  },
  
  is_enabled: (sel: number | string) => {
    try {
      const el = findElement(sel) as HTMLInputElement | HTMLButtonElement;
      return !el.disabled;
    } catch {
      return false;
    }
  },
  
  // Waiting
  wait: async (ms: number) => {
    await delay(ms);
    return true;
  },
  
  wait_for_element: async (sel: string, timeout: number = 5000) => {
    const startTime = Date.now();
    
    while (Date.now() - startTime < timeout) {
      try {
        const el = document.querySelector(sel);
        if (el) return true;
      } catch (e) {
        // Ignore and continue
      }
      await delay(100);
    }
    
    throw new Error(`RZN_TIMEOUT:Element ${sel} not found after ${timeout}ms`);
  },
  
  // Form operations
  select_option: async (sel: number | string, value: string) => {
    const select = findElement(sel) as HTMLSelectElement;
    select.value = value;
    select.dispatchEvent(new Event('change', { bubbles: true }));
    return true;
  },
  
  check: async (sel: number | string) => {
    const checkbox = findElement(sel) as HTMLInputElement;
    if (!checkbox.checked) {
      checkbox.click();
    }
    return true;
  },
  
  uncheck: async (sel: number | string) => {
    const checkbox = findElement(sel) as HTMLInputElement;
    if (checkbox.checked) {
      checkbox.click();
    }
    return true;
  },
  
  // Page info
  get_url: () => window.location.href,
  get_title: () => document.title,
  get_domain: () => window.location.hostname,
  
  // Complex extraction
  extract_structured_data: (rootSel: string, fields: Array<{name: string, selector: string, attribute?: string}>) => {
    const container = rootSel === 'body' ? document.body : document.querySelector(rootSel);
    if (!container) return { results: [] };
    
    const results: any[] = [];
    const items = rootSel === 'body' ? [container] : container.querySelectorAll(':scope > *');
    
    items.forEach(item => {
      const data: any = {};
      fields.forEach(field => {
        const el = item.querySelector(field.selector);
        if (el) {
          data[field.name] = field.attribute 
            ? el.getAttribute(field.attribute) 
            : el.textContent?.trim() || '';
        }
      });
      if (Object.keys(data).length > 0) {
        results.push(data);
      }
    });
    
    return { results };
  },
  
  extract_table: (sel: number | string) => {
    const table = findElement(sel) as HTMLTableElement;
    const rows = Array.from(table.querySelectorAll('tr'));
    
    return rows.map(row => {
      const cells = Array.from(row.querySelectorAll('td, th'));
      return cells.map(cell => cell.textContent?.trim() || '');
    });
  },
  
  extract_list: (sel: number | string) => {
    const list = findElement(sel);
    const items = Array.from(list.querySelectorAll('li'));
    return items.map(item => item.textContent?.trim() || '');
  },
  
  extract_links: (containerSel?: number | string) => {
    const container = containerSel ? findElement(containerSel) : document;
    const links = Array.from(container.querySelectorAll('a[href]'));
    
    return links.map(link => ({
      text: link.textContent?.trim() || '',
      href: (link as HTMLAnchorElement).href,
      index: link.getAttribute('data-rzn-idx')
    }));
  },
  
  extract_images: (containerSel?: number | string) => {
    const container = containerSel ? findElement(containerSel) : document;
    const images = Array.from(container.querySelectorAll('img[src]'));
    
    return images.map(img => ({
      alt: img.getAttribute('alt') || '',
      src: (img as HTMLImageElement).src,
      index: img.getAttribute('data-rzn-idx')
    }));
  },
  
  // Counting
  count_elements: (selector: string) => {
    return document.querySelectorAll(selector).length;
  },
  
  // Screenshot (via background script)
  take_screenshot: async () => {
    return new Promise((resolve) => {
      chrome.runtime.sendMessage({ cmd: 'take_screenshot' }, (response) => {
        resolve(response.dataUrl);
      });
    });
  }
};

// Message listener for executing actions
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg.rzn && msg.cmd in RZN_ACTIONS) {
    Promise.resolve()
      .then(() => (RZN_ACTIONS as any)[msg.cmd](...(msg.args || [])))
      .then(data => sendResponse({ ok: true, data }))
      .catch(e => {
        // Check for CSP errors
        if (/Content Security Policy/.test(e.message)) {
          sendResponse({ ok: false, err: 'RZN_CSP_BLOCKED', details: e.message });
        } else {
          sendResponse({ ok: false, err: e.message });
        }
      });
    
    return true; // Keep port open for async response
  }
});

// Export for TypeScript usage
export type ActionName = keyof typeof RZN_ACTIONS;
export type ActionArgs<T extends ActionName> = Parameters<typeof RZN_ACTIONS[T]>;