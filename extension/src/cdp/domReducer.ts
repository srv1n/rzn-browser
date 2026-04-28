// Hybrid DOM/AX Reducer - Works with CDP or Pure JS extraction
import { strategy } from './executionStrategy';
import type { UnifiedSnapshot, NodeSummary } from './uft';

export interface ReducedContext {
  summary: string;           // Human-readable summary for LLM
  elements: ElementInfo[];   // Actionable elements
  metadata: {
    frameCount: number;
    elementCount: number;
    hasIframes: boolean;
    hasCrossOriginContent: boolean;
    viewport?: { width: number; height: number };
  };
  raw?: string;              // Optional raw text for debugging
}

export interface ElementInfo {
  id: string;                // EncodedId or selector token
  role: string;              // button, link, input, etc.
  name?: string;             // Label or aria-label
  text?: string;             // Visible text
  value?: string;            // Current value (for inputs)
  actions: string[];         // Available actions
  visible: boolean;          // In viewport
  clickable: boolean;        // Can be clicked
  editable: boolean;         // Can accept text
  selector?: string;         // Best-effort CSS selector from CDP
  frameId?: string;          // Frame identifier (for cross-origin targeting)
  frameOrdinal?: number;     // Frame ordinal index
  attributes?: Record<string, string>;
}

// Reduce CDP snapshot for LLM
export function reduceSnapshot(
  snapshot: UnifiedSnapshot,
  options: {
    maxBytes?: number;
    focusViewport?: boolean;
    includeHidden?: boolean;
  } = {}
): ReducedContext {
  const { 
    maxBytes = 4096,
    focusViewport = true,
    includeHidden = false 
  } = options;
  
  // Filter and prioritize elements
  let elements = snapshot.nodes
    .filter(n => {
      if (!includeHidden && n.visible === false) return false;
      if (!n.role && !n.clickable) return false;
      return true;
    })
    .map(n => nodeToElementInfo(n))
    .sort((a, b) => {
      // Prioritize by importance
      const scoreA = getElementScore(a);
      const scoreB = getElementScore(b);
      return scoreB - scoreA;
    });
  
  // Focus on viewport if requested
  if (focusViewport && snapshot.viewportSize) {
    const vw = snapshot.viewportSize.width;
    const vh = snapshot.viewportSize.height;
    
    elements = elements.filter(el => {
      // Keep elements likely in viewport (simplified check)
      // In production, use actual box coordinates
      return el.visible;
    });
  }
  
  // Build summary
  const lines: string[] = [];
  lines.push(`Page: ${snapshot.frames[0]?.url || 'unknown'}`);
  lines.push(`Frames: ${snapshot.frames.length}`);
  
  if (snapshot.frames.length > 1) {
    lines.push('Has iframes: yes');
  }
  
  // Group elements by role
  const byRole = new Map<string, ElementInfo[]>();
  for (const el of elements) {
    const role = el.role || 'unknown';
    if (!byRole.has(role)) byRole.set(role, []);
    byRole.get(role)!.push(el);
  }
  
  // Add elements to summary
  for (const [role, items] of byRole) {
    lines.push(`\n${role}s (${items.length}):`);
    for (const item of items.slice(0, 5)) {
      const name = item.name || item.text || item.value || '';
      const truncated = name.length > 50 ? name.slice(0, 47) + '...' : name;
      lines.push(`  [${item.id}] ${truncated}`);
      if (item.actions.length > 0) {
        lines.push(`    Actions: ${item.actions.join(', ')}`);
      }
    }
  }
  
  // Trim to budget
  let summary = lines.join('\n');
  if (summary.length > maxBytes) {
    summary = summary.slice(0, maxBytes - 20) + '\n... (truncated)';
  }
  
  return {
    summary,
    elements: elements.slice(0, 100), // Keep top 100 elements
    metadata: {
      frameCount: snapshot.frames.length,
      elementCount: snapshot.nodes.length,
      hasIframes: snapshot.frames.length > 1,
      hasCrossOriginContent: snapshot.frames.some(f => 
        f.url && !f.url.startsWith(snapshot.frames[0].url?.split('/').slice(0, 3).join('/') || '')
      ),
      viewport: snapshot.viewportSize
    }
  };
}

// Reduce pure DOM extraction (no CDP)
export async function reduceDOMContent(
  tabId: number,
  options: {
    maxBytes?: number;
    selector?: string;
  } = {}
): Promise<ReducedContext> {
  const { maxBytes = 4096 } = options;
  
  // Extract from content scripts (always main frame)
  const results = await chrome.scripting.executeScript({
    target: { tabId, allFrames: false },
    func: () => {
      const container = document.body || document.documentElement;
      if (!container) return null;
      
      // Find interactable elements
      const elements: any[] = [];
      const selectors = [
        'a[href]',
        'button',
        'input',
        'textarea',
        'select',
        '[role=button]',
        '[role=link]',
        '[onclick]',
        '[contenteditable]'
      ];
      
      container.querySelectorAll(selectors.join(',')).forEach((el, index) => {
        const rect = el.getBoundingClientRect();
        const visible = rect.width > 0 && rect.height > 0 && 
                        rect.top < window.innerHeight && 
                        rect.bottom > 0;
        
        // Build selector
        let selector = el.tagName.toLowerCase();
        if (el.id) {
          selector = `#${el.id}`;
        } else if (el.className) {
          selector += `.${el.className.split(' ').join('.')}`;
        }
        
        elements.push({
          id: selector || `element-${index}`,
          role: el.getAttribute('role') || el.tagName.toLowerCase(),
          name: el.getAttribute('aria-label') || 
                el.getAttribute('title') || 
                (el as any).innerText?.slice(0, 50),
          value: (el as any).value,
          visible,
          clickable: el.tagName === 'A' || el.tagName === 'BUTTON' || 
                     el.getAttribute('onclick') !== null,
          editable: el.tagName === 'INPUT' || el.tagName === 'TEXTAREA',
          href: (el as any).href
        });
      });
      
      return {
        url: window.location.href,
        title: document.title,
        elements,
        hasIframes: document.querySelectorAll('iframe').length > 0
      };
    },
    world: 'MAIN'
  });
  
  const data = results[0]?.result as any;
  if (!data) {
    return {
      summary: 'Failed to extract page content',
      elements: [],
      metadata: {
        frameCount: 1,
        elementCount: 0,
        hasIframes: false,
        hasCrossOriginContent: false
      }
    };
  }
  
  // Convert to ElementInfo format
  const elements: ElementInfo[] = data.elements.map((e: any) => ({
    id: e.id,
    role: e.role,
    name: e.name,
    text: e.name,
    value: e.value,
    actions: determineActions(e),
    visible: e.visible,
    clickable: e.clickable,
    editable: e.editable,
    selector: e.id,
    frameOrdinal: 0,
    attributes: e.href ? { href: e.href } : undefined
  }));
  
  // Build summary
  const lines: string[] = [];
  lines.push(`Page: ${data.title || 'Untitled'}`);
  lines.push(`URL: ${data.url}`);
  lines.push(`Elements: ${elements.length}`);
  
  if (data.hasIframes) {
    lines.push('Has iframes: yes (limited access in pure-JS mode)');
  }
  
  // Group by role
  const byRole = new Map<string, ElementInfo[]>();
  for (const el of elements) {
    const role = el.role || 'unknown';
    if (!byRole.has(role)) byRole.set(role, []);
    byRole.get(role)!.push(el);
  }
  
  for (const [role, items] of byRole) {
    if (items.length === 0) continue;
    lines.push(`\n${role}s (${items.length}):`);
    for (const item of items.slice(0, 3)) {
      const desc = item.name || item.text || '';
      lines.push(`  [${item.id}] ${desc.slice(0, 50)}`);
    }
  }
  
  let summary = lines.join('\n');
  if (summary.length > maxBytes) {
    summary = summary.slice(0, maxBytes - 20) + '\n... (truncated)';
  }
  
  return {
    summary,
    elements,
    metadata: {
      frameCount: 1,
      elementCount: elements.length,
      hasIframes: data.hasIframes,
      hasCrossOriginContent: false // Can't detect in pure-JS
    }
  };
}

// Hybrid reducer - automatically chooses best method
export async function reduceForLLM(
  tabId: number,
  options: {
    maxBytes?: number;
    preferCDP?: boolean;
  } = {}
): Promise<ReducedContext> {
  const url = await getCurrentUrl(tabId);
  const shouldUseCDP = options.preferCDP || 
                       strategy.shouldUseCDP('cdpForCrossOriginFrames', url);
  
  if (shouldUseCDP) {
    try {
      // Try CDP snapshot
      const { buildUnifiedSnapshot } = await import('./uft');
      const snapshot = await buildUnifiedSnapshot(tabId);
      return reduceSnapshot(snapshot, options);
    } catch (e) {
      console.warn('[Reducer] CDP snapshot failed, falling back to DOM:', e);
    }
  }
  
  // Use pure DOM extraction
  return reduceDOMContent(tabId, options);
}

// Helper functions
function nodeToElementInfo(node: NodeSummary): ElementInfo {
  return {
    id: node.id,
    role: node.role || 'element',
    name: node.name,
    text: node.text,
    value: node.value,
    actions: determineNodeActions(node),
    visible: node.visible !== false,
    clickable: node.clickable || false,
    editable: node.role === 'textbox' || node.role === 'textarea',
    selector: node.selector,
    frameId: node.frameId,
    frameOrdinal: node.frameOrdinal,
    attributes: node.attributes
  };
}

function determineActions(element: any): string[] {
  const actions: string[] = [];
  
  if (element.clickable) actions.push('click');
  if (element.editable) actions.push('type', 'clear');
  if (element.role === 'select') actions.push('select');
  if (element.href) actions.push('navigate');
  
  return actions;
}

function determineNodeActions(node: NodeSummary): string[] {
  const actions: string[] = [];
  
  if (node.clickable) actions.push('click');
  if (node.role === 'textbox' || node.role === 'textarea') {
    actions.push('type', 'clear', 'setValue');
  }
  if (node.role === 'combobox' || node.role === 'listbox') {
    actions.push('select');
  }
  if (node.url) actions.push('navigate');
  
  actions.push('scroll');
  
  return actions;
}

function getElementScore(element: ElementInfo): number {
  let score = 0;
  
  // Prioritize by role
  const rolePriority: Record<string, number> = {
    button: 10,
    link: 8,
    textbox: 7,
    input: 7,
    select: 6,
    combobox: 6,
    checkbox: 5,
    radio: 5
  };
  
  score += rolePriority[element.role] || 0;
  
  // Bonus for visible elements
  if (element.visible) score += 5;
  
  // Bonus for actionable elements
  score += element.actions.length * 2;
  
  // Bonus for labeled elements
  if (element.name) score += 3;
  
  return score;
}

async function getCurrentUrl(tabId: number): Promise<string> {
  try {
    const tab = await chrome.tabs.get(tabId);
    return tab.url || '';
  } catch {
    return '';
  }
}
