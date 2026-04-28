/**
 * DOM Analysis Types for RZN Browser Automation
 * Based on the reference project's approach with TypeScript type safety
 */

// ============ Core DOM Types ============

export interface ElementPosition {
  top: number;
  left: number;
  width: number;
  height: number;
}

export interface ViewportInfo {
  width: number;
  height: number;
  scrollX: number;
  scrollY: number;
}

export interface ElementSelector {
  css?: string;
  xpath?: string;
  text?: string;
  testId?: string;
  ariaLabel?: string;
}

// ============ DOM Element Types ============

export interface DOMElement {
  id: string;
  tagName: string;
  xpath: string;
  selector: ElementSelector;
  attributes: Record<string, string>;
  text: string;
  children: string[];
  
  // State flags
  isVisible: boolean;
  isInteractive: boolean;
  isTopElement: boolean;
  isInViewport: boolean;
  isDistinctInteraction: boolean;
  
  // Positioning
  position: ElementPosition;
  
  // Classification
  cursor: string;
  role: string;
  type: string;
  
  // Shadow DOM
  shadowRoot?: boolean;
  
  // Change detection
  hash: string;
  
  // Highlighting
  highlightIndex?: number;
  
  // Text aggregation
  aggregatedText?: string;
  
  // Event listeners detection
  hasEventListeners?: boolean;
  
  // Semantic analysis
  semanticScore?: number;
  interactionScore?: number;
}

export interface DOMTextNode {
  type: 'TEXT_NODE';
  id: string;
  text: string;
  isVisible: boolean;
  parentId: string;
  position?: ElementPosition;
}

export interface DOMNode extends DOMElement {
  textNodes: DOMTextNode[];
}

// ============ DOM Tree Structure ============

export interface DOMState {
  rootId: string;
  elementMap: Record<string, DOMElement | DOMTextNode>;
  metrics: DOMMetrics;
  viewport: ViewportInfo;
  url: string;
  title: string;
  timestamp: number;
}

export interface DOMMetrics {
  totalNodes: number;
  processedNodes: number;
  interactiveNodes: number;
  filteredNodes: number;
  textNodes: number;
  shadowRoots: number;
  iframes: number;
  processingTimeMs?: number;
  cacheHitRate?: number;
}

// ============ Analysis Options ============

export interface DOMAnalysisOptions {
  // Highlighting options
  highlightElements?: boolean;
  focusHighlightIndex?: number;
  
  // Viewport filtering
  viewportExpansion?: number; // pixels to expand viewport (-1 for no filtering)
  prioritizeViewport?: boolean;
  
  // Element limits
  maxElements?: number;
  maxTextLength?: number;
  
  // Performance options
  enableCaching?: boolean;
  batchSize?: number;
  processingTimeout?: number;
  
  // Analysis depth
  includeTextNodes?: boolean;
  includeShadowDOM?: boolean;
  includeIframes?: boolean;
  includeHiddenElements?: boolean;
  
  // Text aggregation
  aggregateTextUntilNextInteractive?: boolean;
  maxTextAggregationLength?: number;
  
  // Event listener detection
  detectEventListeners?: boolean;
  
  // Semantic analysis
  enableSemanticAnalysis?: boolean;
  calculateInteractionScores?: boolean;
  
  // Debug options
  debugMode?: boolean;
  logPerformanceMetrics?: boolean;
}

// ============ Cache Types ============

export interface DOMCache {
  boundingRects: WeakMap<Element, DOMRect>;
  clientRects: WeakMap<Element, DOMRectList>;
  computedStyles: WeakMap<Element, CSSStyleDeclaration>;
  xpaths: WeakMap<Element, string>;
  selectors: WeakMap<Element, ElementSelector>;
  textContent: WeakMap<Element, string>;
  eventListeners: WeakMap<Element, boolean>;
  clearCache(): void;
}

// ============ Interactive Element Detection ============

export interface InteractiveElementConfig {
  interactiveTags: Set<string>;
  interactiveRoles: Set<string>;
  interactiveCursors: Set<string>;
  nonInteractiveCursors: Set<string>;
  distinctInteractiveTags: Set<string>;
  textInputTypes: Set<string>;
}

// ============ Element Scoring ============

export interface ElementScore {
  baseScore: number;
  viewportScore: number;
  typeScore: number;
  roleScore: number;
  sizeScore: number;
  textScore: number;
  interactionScore: number;
  totalScore: number;
}

// ============ Shadow DOM Types ============

export interface ShadowDOMInfo {
  host: Element;
  shadowRoot: ShadowRoot;
  mode: 'open' | 'closed';
  delegatesFocus: boolean;
}

// ============ Event Listener Types ============

export interface EventListenerInfo {
  type: string;
  capture: boolean;
  passive: boolean;
  once: boolean;
}

// ============ Text Aggregation Types ============

export interface TextAggregation {
  text: string;
  source: 'direct' | 'children' | 'aggregated';
  length: number;
  truncated: boolean;
}

// ============ Change Detection ============

export interface DOMChangeInfo {
  elementId: string;
  changeType: 'added' | 'removed' | 'modified' | 'moved';
  oldHash?: string;
  newHash?: string;
  timestamp: number;
}

export interface DOMDiff {
  changes: DOMChangeInfo[];
  addedElements: string[];
  removedElements: string[];
  modifiedElements: string[];
  timestamp: number;
}

// ============ Error Types ============

export interface DOMAnalysisError {
  type: 'timeout' | 'memory' | 'access_denied' | 'invalid_element' | 'unknown';
  message: string;
  elementId?: string;
  stack?: string;
}

// ============ Utility Types ============

export type ElementFilter = (element: Element) => boolean;
export type TextNodeFilter = (textNode: Text) => boolean;
export type ElementComparator = (a: DOMElement, b: DOMElement) => number;

// ============ Export all types ============

// Default analysis options
export const DEFAULT_ANALYSIS_OPTIONS: Required<DOMAnalysisOptions> = {
  highlightElements: false,
  focusHighlightIndex: -1,
  viewportExpansion: 0,
  prioritizeViewport: true,
  maxElements: 200,
  maxTextLength: 100,
  enableCaching: true,
  batchSize: 50,
  processingTimeout: 5000,
  includeTextNodes: true,
  includeShadowDOM: true,
  includeIframes: false, // Often restricted by CORS
  includeHiddenElements: false,
  aggregateTextUntilNextInteractive: true,
  maxTextAggregationLength: 500,
  detectEventListeners: false, // Performance intensive
  enableSemanticAnalysis: false, // Performance intensive
  calculateInteractionScores: true,
  debugMode: false,
  logPerformanceMetrics: false
};

// Default interactive element configuration
export const DEFAULT_INTERACTIVE_CONFIG: InteractiveElementConfig = {
  interactiveTags: new Set([
    'a', 'button', 'input', 'select', 'textarea', 'details', 'summary',
    'label', 'option', 'optgroup', 'fieldset', 'legend'
  ]),
  
  interactiveRoles: new Set([
    'button', 'link', 'menuitem', 'menuitemradio', 'menuitemcheckbox',
    'radio', 'checkbox', 'tab', 'switch', 'slider', 'spinbutton',
    'combobox', 'searchbox', 'textbox', 'listbox', 'option', 'scrollbar'
  ]),
  
  interactiveCursors: new Set([
    'pointer', 'move', 'text', 'grab', 'grabbing', 'cell', 'copy',
    'alias', 'all-scroll', 'col-resize', 'context-menu', 'crosshair',
    'e-resize', 'ew-resize', 'help', 'n-resize', 'ne-resize', 'nesw-resize',
    'ns-resize', 'nw-resize', 'nwse-resize', 'row-resize', 's-resize',
    'se-resize', 'sw-resize', 'vertical-text', 'w-resize', 'zoom-in', 'zoom-out'
  ]),
  
  nonInteractiveCursors: new Set([
    'not-allowed', 'no-drop', 'wait', 'progress', 'initial', 'inherit'
  ]),
  
  distinctInteractiveTags: new Set([
    'a', 'button', 'input', 'select', 'textarea', 'summary', 'details', 
    'label', 'option'
  ]),
  
  textInputTypes: new Set([
    'text', 'email', 'password', 'search', 'tel', 'url', 'number',
    'date', 'datetime-local', 'month', 'time', 'week'
  ])
};