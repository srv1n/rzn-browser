/**
 * Comprehensive DOM Analyzer for RZN Browser Automation
 * Based on the reference project's sophisticated DOM analysis approach
 * Features:
 * - Smart interactive element detection using cursor styles, event listeners, and semantic analysis
 * - Performance caching with WeakMaps for boundingRects, computedStyles, clientRects
 * - Viewport-based filtering with expansion
 * - Text aggregation until next interactive element
 * - Shadow DOM and iframe handling
 * - Element hashing for change detection
 */

import {
  DOMElement,
  DOMTextNode,
  DOMState,
  DOMAnalysisOptions,
  DOMCache,
  ElementSelector,
  ViewportInfo,
  DOMMetrics,
  ElementScore,
  DEFAULT_ANALYSIS_OPTIONS,
  DEFAULT_INTERACTIVE_CONFIG,
  InteractiveElementConfig,
  TextAggregation,
  DOMDiff,
  DOMChangeInfo,
  ShadowDOMInfo
} from '../types/dom';

/**
 * Enhanced element finder that traverses closed shadow roots
 */
export function findElementEnhanced(selector: string): Element | null {
  // Try regular DOM first
  let element = document.querySelector(selector);
  if (element) return element;
  
  // Traverse all shadow roots including closed ones
  const traverse = (root: Document | ShadowRoot): Element | null => {
    // Query in current root
    const found = root.querySelector(selector);
    if (found) return found;
    
    // Find all potential shadow hosts
    const hosts = root.querySelectorAll('[data-rzn-shadow-host="true"], *');
    
    for (const host of hosts) {
      // Try to get shadow root (open or closed)
      let shadowRoot = host.shadowRoot; // Works for open
      
      // For closed shadows, use our instrumentation
      if (!shadowRoot && (window as any).__rznGetShadowRoot) {
        shadowRoot = (window as any).__rznGetShadowRoot(host);
      }
      
      if (shadowRoot) {
        const foundInShadow = traverse(shadowRoot);
        if (foundInShadow) return foundInShadow;
      }
    }
    
    return null;
  };
  
  return traverse(document);
}

/**
 * Find all elements matching selector, including in shadow DOMs
 */
export function findAllElementsEnhanced(selector: string): Element[] {
  const elements: Element[] = [];
  
  const traverse = (root: Document | ShadowRoot) => {
    // Query in current root
    const found = root.querySelectorAll(selector);
    elements.push(...Array.from(found));
    
    // Find all potential shadow hosts
    const hosts = root.querySelectorAll('[data-rzn-shadow-host="true"], *');
    
    for (const host of hosts) {
      // Try to get shadow root (open or closed)
      let shadowRoot = host.shadowRoot; // Works for open
      
      // For closed shadows, use our instrumentation
      if (!shadowRoot && (window as any).__rznGetShadowRoot) {
        shadowRoot = (window as any).__rznGetShadowRoot(host);
      }
      
      if (shadowRoot) {
        traverse(shadowRoot);
      }
    }
  };
  
  traverse(document);
  return elements;
}

/**
 * YouTube-specific selector helpers
 */
export const YOUTUBE_SELECTORS = {
  searchBox: 'input#search',
  searchButton: 'button#search-icon-legacy',
  searchForm: 'form#search-form',
  searchBoxContainer: 'ytd-searchbox',
  
  // Helper to find YouTube search reliably
  findYouTubeSearch(): HTMLInputElement | null {
    // YouTube nests search in ytd-searchbox > #shadow-root > input#search
    const searchBox = findElementEnhanced('ytd-searchbox');
    if (!searchBox) {
      console.log('[RZN] YouTube searchbox container not found');
      return null;
    }
    
    // Get shadow root (might be closed)
    const shadow = (searchBox as any).shadowRoot || 
                  (window as any).__rznGetShadowRoot?.(searchBox);
    
    if (!shadow) {
      console.log('[RZN] YouTube searchbox shadow root not accessible');
      // Fallback to regular search
      return findElementEnhanced('input#search') as HTMLInputElement;
    }
    
    const input = shadow.querySelector('input#search') as HTMLInputElement;
    console.log('[RZN] YouTube search input found in shadow DOM:', !!input);
    return input;
  },
  
  // Helper to find search button
  findYouTubeSearchButton(): HTMLButtonElement | null {
    // Try multiple selectors
    const selectors = [
      'button#search-icon-legacy',
      'ytd-searchbox button[aria-label*="Search"]',
      'button[id="search-icon-legacy"]'
    ];
    
    for (const selector of selectors) {
      const button = findElementEnhanced(selector) as HTMLButtonElement;
      if (button) return button;
    }
    
    return null;
  }
};

/**
 * Comprehensive DOM Analyzer implementing the reference project's approach
 */
export class DOMAnalyzer {
  private elementIdCounter = 0;
  private textNodeIdCounter = 0;
  private highlightIndex = 0;
  private processedElements = new WeakSet<Element>();
  private processedTextNodes = new WeakSet<Text>();
  
  // Performance optimizations
  private lastProcessTime = 0;
  private processingThrottle = 100; // ms
  private processingTimeout: number;
  
  // Configuration
  private interactiveConfig: InteractiveElementConfig;
  
  // Performance cache implementation following public reference
  private domCache: DOMCache = {
    boundingRects: new WeakMap(),
    clientRects: new WeakMap(),
    computedStyles: new WeakMap(),
    xpaths: new WeakMap(),
    selectors: new WeakMap(),
    textContent: new WeakMap(),
    eventListeners: new WeakMap(),
    clearCache: () => {
      this.domCache.boundingRects = new WeakMap();
      this.domCache.clientRects = new WeakMap();
      this.domCache.computedStyles = new WeakMap();
      this.domCache.xpaths = new WeakMap();
      this.domCache.selectors = new WeakMap();
      this.domCache.textContent = new WeakMap();
      this.domCache.eventListeners = new WeakMap();
    }
  };
  
  // Shadow DOM registry
  private shadowRoots = new Map<Element, ShadowDOMInfo>();
  
  // State for change detection
  private lastDOMState: DOMState | null = null;

  constructor(config: Partial<InteractiveElementConfig> = {}) {
    this.interactiveConfig = { ...DEFAULT_INTERACTIVE_CONFIG, ...config };
    this.processingTimeout = 5000; // 5 second default timeout
    this.clearCache();
  }

  /**
   * Clear all caches and reset state
   */
  clearCache(): void {
    this.domCache.clearCache();
    this.processedElements = new WeakSet();
    this.processedTextNodes = new WeakSet();
    this.shadowRoots.clear();
    this.elementIdCounter = 0;
    this.textNodeIdCounter = 0;
    this.highlightIndex = 0;
    this.lastProcessTime = 0;
  }

  /**
   * Main method to analyze DOM and build comprehensive tree
   * Follows the reference project's buildDomTree approach
   */
  public analyzeDOMTree(options: Partial<DOMAnalysisOptions> = {}): DOMState {
    const opts = { ...DEFAULT_ANALYSIS_OPTIONS, ...options };
    const startTime = performance.now();
    
    // Check throttling
    if (this.shouldThrottleProcessing(opts)) {
      console.log('[RZN] DOM processing throttled');
      return this.createMinimalDOMState();
    }
    
    this.clearCache();
    
    const metrics: DOMMetrics = {
      totalNodes: 0,
      processedNodes: 0,
      interactiveNodes: 0,
      filteredNodes: 0,
      textNodes: 0,
      shadowRoots: 0,
      iframes: 0
    };
    
    const elementMap: Record<string, DOMElement | DOMTextNode> = {};
    
    // Process DOM starting from body
    const rootId = this.processNode(
      document.body,
      null, // parent iframe
      false, // is parent highlighted
      elementMap,
      metrics,
      opts
    );
    
    // Apply viewport prioritization if enabled
    let finalElementMap = elementMap;
    if (opts.prioritizeViewport && opts.maxElements && Object.keys(elementMap).length > opts.maxElements) {
      finalElementMap = this.applyViewportPrioritization(elementMap, opts);
    }
    
    const processingTime = performance.now() - startTime;
    metrics.processingTimeMs = processingTime;
    
    // Log performance metrics if enabled
    if (opts.logPerformanceMetrics) {
      this.logPerformanceMetrics(metrics, processingTime);
    }
    
    const domState: DOMState = {
      rootId: rootId || 'body',
      elementMap: finalElementMap,
      metrics,
      viewport: this.getViewportInfo(),
      url: window.location.href,
      title: document.title,
      timestamp: Date.now()
    };
    
    this.lastDOMState = domState;
    return domState;
  }

  /**
   * Process a single node (element or text) recursively
   * Core logic from the reference project's processNode
   */
  private processNode(
    node: Node,
    parentIframe: HTMLIFrameElement | null,
    isParentHighlighted: boolean,
    elementMap: Record<string, DOMElement | DOMTextNode>,
    metrics: DOMMetrics,
    options: DOMAnalysisOptions
  ): string | null {
    metrics.totalNodes++;
    
    // Early filtering
    if (!node || (node.nodeType !== Node.ELEMENT_NODE && node.nodeType !== Node.TEXT_NODE)) {
      return null;
    }
    
    // Handle text nodes
    if (node.nodeType === Node.TEXT_NODE) {
      if (!options.includeTextNodes) return null;
      return this.processTextNode(node as Text, elementMap, metrics, options);
    }
    
    const element = node as Element;
    
    // Skip if already processed or not accepted
    if (this.processedElements.has(element) || !this.isElementAccepted(element)) {
      return null;
    }
    
    this.processedElements.add(element);
    metrics.processedNodes++;
    
    // Early viewport filtering (public reference optimization)
    if (options.viewportExpansion !== -1 && !this.isElementInExpandedViewport(element, options.viewportExpansion)) {
      const style = this.getCachedComputedStyle(element);
      const isFixedOrSticky = style && (style.position === 'fixed' || style.position === 'sticky');
      
      if (!isFixedOrSticky) {
        metrics.filteredNodes++;
        return null;
      }
    }
    
    // Create DOM element
    const domElement = this.createDOMElement(element, parentIframe, options);
    if (!domElement) return null;
    
    const elementId = `elem-${this.elementIdCounter++}`;
    domElement.id = elementId;
    
    // Handle highlighting (public reference approach)
    let nodeWasHighlighted = false;
    if (domElement.isVisible && (domElement.isTopElement || this.isMenuContainer(element))) {
      if (domElement.isInteractive) {
        nodeWasHighlighted = this.handleHighlighting(
          domElement,
          element,
          isParentHighlighted,
          options
        );
        
        if (nodeWasHighlighted) {
          metrics.interactiveNodes++;
        }
      }
    }
    
    // Handle text aggregation
    if (options.aggregateTextUntilNextInteractive) {
      domElement.aggregatedText = this.aggregateTextUntilNextInteractive(
        element,
        options.maxTextAggregationLength || 500
      );
    }
    
    // Process children
    this.processElementChildren(
      element,
      elementId,
      parentIframe,
      nodeWasHighlighted || isParentHighlighted,
      domElement,
      elementMap,
      metrics,
      options
    );
    
    elementMap[elementId] = domElement;
    return elementId;
  }

  /**
   * Process text node following public reference approach
   */
  private processTextNode(
    textNode: Text,
    elementMap: Record<string, DOMElement | DOMTextNode>,
    metrics: DOMMetrics,
    options: DOMAnalysisOptions
  ): string | null {
    if (this.processedTextNodes.has(textNode)) return null;
    
    const textContent = textNode.textContent?.trim();
    if (!textContent || textContent.length === 0) return null;
    
    const parentElement = textNode.parentElement;
    if (!parentElement || parentElement.tagName.toLowerCase() === 'script') {
      return null;
    }
    
    this.processedTextNodes.add(textNode);
    metrics.textNodes++;
    
    const textId = `text-${this.textNodeIdCounter++}`;
    
    const domTextNode: DOMTextNode = {
      type: 'TEXT_NODE',
      id: textId,
      text: textContent.substring(0, options.maxTextLength || 100),
      isVisible: this.isTextNodeVisible(textNode, options.viewportExpansion),
      parentId: '', // Will be set by parent
      position: this.getTextNodePosition(textNode)
    };
    
    elementMap[textId] = domTextNode;
    return textId;
  }

  /**
   * Create comprehensive DOM element structure
   */
  private createDOMElement(
    element: Element,
    parentIframe: HTMLIFrameElement | null,
    options: DOMAnalysisOptions
  ): DOMElement | null {
    const tagName = element.tagName.toLowerCase();
    const rect = this.getCachedBoundingRect(element);
    const style = this.getCachedComputedStyle(element);
    
    const isVisible = this.isElementVisible(element);
    const isInteractive = this.isElementInteractive(element);
    const isTopElement = this.isTopElement(element, options.viewportExpansion);
    const isInViewport = this.isElementInExpandedViewport(element, options.viewportExpansion);
    
    const domElement: DOMElement = {
      id: '', // Will be set by caller
      tagName,
      xpath: this.getElementXPath(element),
      selector: this.generateElementSelector(element),
      attributes: this.getRelevantAttributes(element),
      text: this.getElementText(element, options.maxTextLength),
      children: [],
      
      // State flags
      isVisible,
      isInteractive,
      isTopElement,
      isInViewport,
      isDistinctInteraction: this.isElementDistinctInteraction(element),
      
      // Positioning
      position: {
        top: Math.round(rect.top),
        left: Math.round(rect.left),
        width: Math.round(rect.width),
        height: Math.round(rect.height)
      },
      
      // Classification
      cursor: style.cursor,
      role: element.getAttribute('role') || '',
      type: (element as HTMLInputElement).type || tagName,
      
      // Shadow DOM
      shadowRoot: !!element.shadowRoot,
      
      // Change detection
      hash: this.generateElementHash(element),
      
      // Event listeners
      hasEventListeners: options.detectEventListeners ? 
        this.detectEventListeners(element) : undefined,
      
      // Scoring
      semanticScore: options.enableSemanticAnalysis ? 
        this.calculateSemanticScore(element) : undefined,
      interactionScore: options.calculateInteractionScores ? 
        this.calculateInteractionScore(element) : undefined
    };
    
    return domElement;
  }

  /**
   * Handle element highlighting following public reference logic
   */
  private handleHighlighting(
    domElement: DOMElement,
    element: Element,
    isParentHighlighted: boolean,
    options: DOMAnalysisOptions
  ): boolean {
    if (!domElement.isInteractive) return false;
    
    let shouldHighlight = false;
    
    if (!isParentHighlighted) {
      shouldHighlight = true;
    } else {
      // Parent was highlighted - only highlight if distinct interaction
      if (domElement.isDistinctInteraction) {
        shouldHighlight = true;
      }
    }
    
    if (shouldHighlight && (domElement.isInViewport || options.viewportExpansion === -1)) {
      domElement.highlightIndex = this.highlightIndex++;
      
      // Set data-rzn-idx attribute for LLM-based element selection
      element.setAttribute('data-rzn-idx', domElement.highlightIndex.toString());
      
      // Actually highlight if requested
      if (options.highlightElements) {
        if (options.focusHighlightIndex >= 0) {
          if (options.focusHighlightIndex === domElement.highlightIndex) {
            this.highlightElement(element, domElement.highlightIndex);
          }
        } else {
          this.highlightElement(element, domElement.highlightIndex);
        }
      }
      
      return true;
    }
    
    return false;
  }

  /**
   * Process element children including shadow DOM and iframes
   */
  private processElementChildren(
    element: Element,
    elementId: string,
    parentIframe: HTMLIFrameElement | null,
    isParentHighlighted: boolean,
    domElement: DOMElement,
    elementMap: Record<string, DOMElement | DOMTextNode>,
    metrics: DOMMetrics,
    options: DOMAnalysisOptions
  ): void {
    const tagName = element.tagName.toLowerCase();
    
    // Handle iframes if enabled
    if (tagName === 'iframe' && options.includeIframes) {
      try {
        const iframe = element as HTMLIFrameElement;
        const iframeDoc = iframe.contentDocument || iframe.contentWindow?.document;
        if (iframeDoc && iframeDoc.body) {
          metrics.iframes++;
          const childId = this.processNode(
            iframeDoc.body,
            iframe,
            false,
            elementMap,
            metrics,
            options
          );
          if (childId) {
            domElement.children.push(childId);
          }
        }
      } catch (e) {
        if (options.debugMode) {
          console.warn('[RZN] Unable to access iframe:', e);
        }
      }
      return;
    }
    
    // Handle shadow DOM if enabled
    if (options.includeShadowDOM && element.shadowRoot) {
      metrics.shadowRoots++;
      this.registerShadowRoot(element, element.shadowRoot);
      
      for (const child of element.shadowRoot.childNodes) {
        const childId = this.processNode(
          child,
          parentIframe,
          isParentHighlighted,
          elementMap,
          metrics,
          options
        );
        if (childId) {
          domElement.children.push(childId);
          
          // Set parent reference for text nodes
          const childNode = elementMap[childId];
          if (childNode && childNode.type === 'TEXT_NODE') {
            (childNode as DOMTextNode).parentId = elementId;
          }
        }
      }
    }
    
    // Handle regular children
    for (const child of element.childNodes) {
      const childId = this.processNode(
        child,
        parentIframe,
        isParentHighlighted,
        elementMap,
        metrics,
        options
      );
      if (childId) {
        domElement.children.push(childId);
        
        // Set parent reference for text nodes
        const childNode = elementMap[childId];
        if (childNode && childNode.type === 'TEXT_NODE') {
          (childNode as DOMTextNode).parentId = elementId;
        }
      }
    }
  }

  // ============ CORE UTILITY METHODS (public reference Implementation) ============

  /**
   * Get cached bounding rect for performance
   */
  private getCachedBoundingRect(element: Element): DOMRect {
    if (this.domCache.boundingRects.has(element)) {
      return this.domCache.boundingRects.get(element)!;
    }
    const rect = element.getBoundingClientRect();
    this.domCache.boundingRects.set(element, rect);
    return rect;
  }

  /**
   * Get cached computed style for performance
   */
  private getCachedComputedStyle(element: Element): CSSStyleDeclaration {
    if (this.domCache.computedStyles.has(element)) {
      return this.domCache.computedStyles.get(element)!;
    }
    const style = window.getComputedStyle(element);
    this.domCache.computedStyles.set(element, style);
    return style;
  }

  /**
   * Get cached client rects for performance
   */
  private getCachedClientRects(element: Element): DOMRectList {
    if (this.domCache.clientRects.has(element)) {
      return this.domCache.clientRects.get(element)!;
    }
    const rects = element.getClientRects();
    this.domCache.clientRects.set(element, rects);
    return rects;
  }

  /**
   * Check if element should be accepted for processing
   */
  private isElementAccepted(element: Element): boolean {
    if (!element || !element.tagName) return false;
    
    const alwaysAccept = new Set([
      'body', 'div', 'main', 'article', 'section', 'nav', 'header', 'footer',
      'aside', 'ul', 'ol', 'li', 'p', 'span', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6'
    ]);
    
    const tagName = element.tagName.toLowerCase();
    
    if (alwaysAccept.has(tagName)) return true;
    
    const denyList = new Set([
      'svg', 'script', 'style', 'link', 'meta', 'noscript', 'template',
      'head', 'title', 'base'
    ]);
    
    return !denyList.has(tagName);
  }

  /**
   * Check if element is visible using public reference criteria
   */
  private isElementVisible(element: Element): boolean {
    const style = this.getCachedComputedStyle(element);
    const htmlElement = element as HTMLElement;
    
    return (
      htmlElement.offsetWidth > 0 &&
      htmlElement.offsetHeight > 0 &&
      style.visibility !== 'hidden' &&
      style.display !== 'none' &&
      parseFloat(style.opacity) > 0
    );
  }

  /**
   * Check if element is interactive using public reference methodology
   */
  private isElementInteractive(element: Element): boolean {
    if (element.nodeType !== Node.ELEMENT_NODE) return false;
    
    const tagName = element.tagName.toLowerCase();
    const style = this.getCachedComputedStyle(element);
    const role = element.getAttribute('role');
    
    // Check cursor style - most reliable indicator
    if (this.interactiveConfig.interactiveCursors.has(style.cursor)) return true;
    
    // Check for non-interactive cursors
    if (this.interactiveConfig.nonInteractiveCursors.has(style.cursor)) return false;
    
    // Check tag name
    if (this.interactiveConfig.interactiveTags.has(tagName)) {
      // Skip disabled elements
      const htmlElement = element as HTMLInputElement;
      if (htmlElement.disabled || 
          element.getAttribute('aria-disabled') === 'true' ||
          element.getAttribute('disabled') !== null) {
        return false;
      }
      return true;
    }
    
    // Check role
    if (role && this.interactiveConfig.interactiveRoles.has(role)) return true;
    
    // Check contenteditable
    const htmlElement = element as HTMLElement;
    if (htmlElement.isContentEditable || 
        element.getAttribute('contenteditable') === 'true') {
      return true;
    }
    
    // Check for interaction indicators
    if (element.hasAttribute('onclick') ||
        element.hasAttribute('data-action') ||
        element.classList.contains('clickable') ||
        element.classList.contains('btn') ||
        element.classList.contains('button')) {
      return true;
    }
    
    // Check tabindex
    const tabIndex = element.getAttribute('tabindex');
    if (tabIndex && tabIndex !== '-1') return true;
    
    return false;
  }

  /**
   * Check if element is topmost at its position (public reference approach)
   */
  private isTopElement(element: Element, viewportExpansion: number): boolean {
    if (viewportExpansion === -1) return true;
    
    const rects = this.getCachedClientRects(element);
    if (!rects || rects.length === 0) return false;
    
    // Check if any rect is in viewport
    let isAnyRectInViewport = false;
    for (const rect of rects) {
      if (rect.width > 0 && rect.height > 0 && 
          !this.isRectOutsideViewport(rect, viewportExpansion)) {
        isAnyRectInViewport = true;
        break;
      }
    }
    
    if (!isAnyRectInViewport) return false;
    
    // Check if element is topmost at its center point
    const rect = rects[Math.floor(rects.length / 2)];
    const centerX = rect.left + rect.width / 2;
    const centerY = rect.top + rect.height / 2;
    
    try {
      const topElement = document.elementFromPoint(centerX, centerY);
      if (!topElement) return false;
      
      // Check if our element is in the hierarchy of the top element
      let current = topElement;
      while (current && current !== document.documentElement) {
        if (current === element) return true;
        current = current.parentElement;
      }
      return false;
    } catch (e) {
      return true; // Default to true if check fails
    }
  }

  /**
   * Check if element is in expanded viewport
   */
  private isElementInExpandedViewport(element: Element, expansion: number): boolean {
    if (expansion === -1) return true;
    
    const rects = this.getCachedClientRects(element);
    if (!rects || rects.length === 0) {
      const boundingRect = this.getCachedBoundingRect(element);
      if (!boundingRect || boundingRect.width === 0 || boundingRect.height === 0) {
        return false;
      }
      return !this.isRectOutsideViewport(boundingRect, expansion);
    }
    
    // Check if any client rect is within viewport
    for (const rect of rects) {
      if (rect.width === 0 || rect.height === 0) continue;
      if (!this.isRectOutsideViewport(rect, expansion)) {
        return true;
      }
    }
    
    return false;
  }

  /**
   * Check if rect is outside viewport with expansion
   */
  private isRectOutsideViewport(rect: DOMRect, expansion: number): boolean {
    return (
      rect.bottom < -expansion ||
      rect.top > window.innerHeight + expansion ||
      rect.right < -expansion ||
      rect.left > window.innerWidth + expansion
    );
  }

  /**
   * Check if element represents distinct interaction
   */
  private isElementDistinctInteraction(element: Element): boolean {
    const tagName = element.tagName.toLowerCase();
    const role = element.getAttribute('role');
    
    // Check distinct interactive tags
    if (this.interactiveConfig.distinctInteractiveTags.has(tagName)) return true;
    
    // Check interactive roles
    if (role && this.interactiveConfig.interactiveRoles.has(role)) return true;
    
    // Check contenteditable
    const htmlElement = element as HTMLElement;
    if (htmlElement.isContentEditable || 
        element.getAttribute('contenteditable') === 'true') {
      return true;
    }
    
    // Check for explicit interaction handlers
    if (element.hasAttribute('onclick') ||
        element.hasAttribute('data-testid') ||
        element.hasAttribute('data-cy') ||
        element.hasAttribute('data-test')) {
      return true;
    }
    
    return false;
  }

  /**
   * Check if element is a menu container
   */
  private isMenuContainer(element: Element): boolean {
    const role = element.getAttribute('role');
    return role === 'menu' || role === 'menubar' || role === 'listbox';
  }

  /**
   * Generate robust element selector
   */
  private generateElementSelector(element: Element): ElementSelector {
    if (this.domCache.selectors.has(element)) {
      return this.domCache.selectors.get(element)!;
    }
    
    const selector: ElementSelector = {};
    
    // Prefer ID
    if (element.id) {
      selector.css = `#${CSS.escape(element.id)}`;
    }
    
    // Use data-testid if available
    const testId = element.getAttribute('data-testid') || 
                   element.getAttribute('data-test') ||
                   element.getAttribute('data-cy');
    if (testId) {
      selector.testId = testId;
      selector.css = selector.css || `[data-testid="${CSS.escape(testId)}"]`;
    }
    
    // Use aria-label if available
    const ariaLabel = element.getAttribute('aria-label');
    if (ariaLabel) {
      selector.ariaLabel = ariaLabel;
    }
    
    // Build CSS selector if not already set
    if (!selector.css) {
      const tagName = element.tagName.toLowerCase();
      let css = tagName;
      
      // Add classes (filtering out utility classes)
      if (element.className) {
        const classes = (element.className as string).split(' ')
          .filter(c => c && !c.match(/^(w-|h-|p-|m-|text-|bg-|flex-|grid-|col-|row-)/))
          .slice(0, 2);
        
        if (classes.length > 0) {
          css += classes.map(c => `.${CSS.escape(c)}`).join('');
        }
      }
      
      // Add unique attributes
      const attrs = ['name', 'type', 'role'];
      for (const attr of attrs) {
        const value = element.getAttribute(attr);
        if (value) {
          css += `[${attr}="${CSS.escape(value)}"]`;
          break;
        }
      }
      
      selector.css = css;
    }
    
    // Generate XPath
    selector.xpath = this.getElementXPath(element);
    
    // Add text content if meaningful
    const text = this.getElementText(element, 50);
    if (text && text.length > 0 && text.length < 50) {
      selector.text = text;
    }
    
    this.domCache.selectors.set(element, selector);
    return selector;
  }

  /**
   * Get element XPath with caching
   */
  private getElementXPath(element: Element): string {
    if (this.domCache.xpaths.has(element)) {
      return this.domCache.xpaths.get(element)!;
    }
    
    const segments: string[] = [];
    let currentElement: Element | null = element;
    
    while (currentElement && currentElement.nodeType === Node.ELEMENT_NODE) {
      // Stop at shadow root or iframe boundary
      if (currentElement.parentNode instanceof ShadowRoot ||
          currentElement.parentNode instanceof HTMLIFrameElement) {
        break;
      }
      
      const position = this.getElementPosition(currentElement);
      const tagName = currentElement.nodeName.toLowerCase();
      const xpathIndex = position > 0 ? `[${position}]` : '';
      segments.unshift(`${tagName}${xpathIndex}`);
      
      currentElement = currentElement.parentElement;
    }
    
    const result = segments.length > 0 ? segments.join('/') : element.tagName.toLowerCase();
    this.domCache.xpaths.set(element, result);
    return result;
  }

  /**
   * Get element position among siblings of same tag
   */
  private getElementPosition(element: Element): number {
    if (!element.parentElement) return 0;
    
    const tagName = element.nodeName.toLowerCase();
    const siblings = Array.from(element.parentElement.children)
      .filter(sib => sib.nodeName.toLowerCase() === tagName);
    
    if (siblings.length === 1) return 0;
    
    return siblings.indexOf(element) + 1;
  }

  /**
   * Get relevant attributes for element
   */
  private getRelevantAttributes(element: Element): Record<string, string> {
    const relevantAttrs = [
      'id', 'class', 'name', 'type', 'role', 'aria-label', 'placeholder',
      'value', 'href', 'src', 'alt', 'title', 'data-testid', 'data-test', 'data-cy',
      'aria-describedby', 'aria-expanded', 'aria-selected', 'aria-checked'
    ];
    
    const attributes: Record<string, string> = {};
    
    for (const attr of relevantAttrs) {
      const value = element.getAttribute(attr);
      if (value && value.trim()) {
        attributes[attr] = value.trim();
      }
    }
    
    return attributes;
  }

  /**
   * Get element text content with caching
   */
  private getElementText(element: Element, maxLength = 100): string {
    if (this.domCache.textContent.has(element)) {
      return this.domCache.textContent.get(element)!;
    }
    
    const tagName = element.tagName.toLowerCase();
    let text = '';
    
    if (tagName === 'input' || tagName === 'textarea') {
      const input = element as HTMLInputElement;
      text = input.placeholder || input.value || '';
    } else {
      // Get direct text content only (not from children)
      const textNodes = Array.from(element.childNodes)
        .filter(n => n.nodeType === Node.TEXT_NODE);
      
      text = textNodes
        .map(n => n.textContent?.trim() || '')
        .join(' ');
    }
    
    const result = text.substring(0, maxLength);
    this.domCache.textContent.set(element, result);
    return result;
  }

  /**
   * Generate element hash for change detection
   */
  private generateElementHash(element: Element): string {
    const tagName = element.tagName.toLowerCase();
    const attributes = this.getRelevantAttributes(element);
    const xpath = this.getElementXPath(element);
    const rect = this.getCachedBoundingRect(element);
    const text = this.getElementText(element, 50);
    
    // Create a stable hash based on element characteristics
    const hashData = `${tagName}|${xpath}|${JSON.stringify(attributes)}|${text}|${rect.top}|${rect.left}|${rect.width}|${rect.height}`;
    
    // Simple but effective hash function
    let hash = 0;
    for (let i = 0; i < hashData.length; i++) {
      const char = hashData.charCodeAt(i);
      hash = ((hash << 5) - hash) + char;
      hash = hash & hash; // Convert to 32-bit integer
    }
    
    return Math.abs(hash).toString(36);
  }

  /**
   * Check if text node is visible
   */
  private isTextNodeVisible(textNode: Text, viewportExpansion: number): boolean {
    const parentElement = textNode.parentElement;
    if (!parentElement) return false;
    
    if (viewportExpansion === -1) {
      try {
        return (parentElement as any).checkVisibility?.({
          checkOpacity: true,
          checkVisibilityCSS: true
        }) ?? this.isElementVisible(parentElement);
      } catch (e) {
        return this.isElementVisible(parentElement);
      }
    }
    
    try {
      const range = document.createRange();
      range.selectNodeContents(textNode);
      const rects = range.getClientRects();
      
      if (!rects || rects.length === 0) return false;
      
      for (const rect of rects) {
        if (rect.width > 0 && rect.height > 0 && 
            !this.isRectOutsideViewport(rect, viewportExpansion)) {
          return true;
        }
      }
      
      return false;
    } catch (e) {
      return this.isElementVisible(parentElement);
    }
  }

  /**
   * Get text node position
   */
  private getTextNodePosition(textNode: Text): ElementPosition | undefined {
    try {
      const range = document.createRange();
      range.selectNodeContents(textNode);
      const rect = range.getBoundingClientRect();
      
      return {
        top: Math.round(rect.top),
        left: Math.round(rect.left),
        width: Math.round(rect.width),
        height: Math.round(rect.height)
      };
    } catch (e) {
      return undefined;
    }
  }

  /**
   * Aggregate text until next interactive element (public reference feature)
   */
  private aggregateTextUntilNextInteractive(element: Element, maxLength: number): string {
    const textParts: string[] = [];
    let currentLength = 0;
    
    const collectText = (node: Node): boolean => {
      if (currentLength >= maxLength) return false;
      
      if (node.nodeType === Node.TEXT_NODE) {
        const text = node.textContent?.trim();
        if (text) {
          const remainingLength = maxLength - currentLength;
          const textToAdd = text.length > remainingLength ? 
            text.substring(0, remainingLength) + '...' : text;
          
          textParts.push(textToAdd);
          currentLength += textToAdd.length;
          
          if (currentLength >= maxLength) return false;
        }
      } else if (node.nodeType === Node.ELEMENT_NODE) {
        const elem = node as Element;
        
        // Stop at next interactive element
        if (elem !== element && this.isElementInteractive(elem)) {
          return false;
        }
        
        // Continue with children
        for (const child of elem.childNodes) {
          if (!collectText(child)) return false;
        }
      }
      
      return true;
    };
    
    // Start from the element's children
    for (const child of element.childNodes) {
      if (!collectText(child)) break;
    }
    
    return textParts.join(' ');
  }

  /**
   * Detect event listeners on element (performance intensive)
   */
  private detectEventListeners(element: Element): boolean {
    if (this.domCache.eventListeners.has(element)) {
      return this.domCache.eventListeners.get(element)!;
    }
    
    // Check for common event handler attributes
    const eventAttributes = [
      'onclick', 'onchange', 'onsubmit', 'onkeydown', 'onkeyup', 'onmousedown',
      'onmouseup', 'onmouseover', 'onmouseout', 'onfocus', 'onblur'
    ];
    
    for (const attr of eventAttributes) {
      if (element.hasAttribute(attr)) {
        this.domCache.eventListeners.set(element, true);
        return true;
      }
    }
    
    // Check for framework-specific attributes
    const frameworkAttributes = [
      'ng-click', 'v-on:click', 'data-action', '@click'
    ];
    
    for (const attr of frameworkAttributes) {
      if (element.hasAttribute(attr)) {
        this.domCache.eventListeners.set(element, true);
        return true;
      }
    }
    
    this.domCache.eventListeners.set(element, false);
    return false;
  }

  /**
   * Calculate semantic score for element
   */
  private calculateSemanticScore(element: Element): number {
    let score = 0;
    
    const tagName = element.tagName.toLowerCase();
    const role = element.getAttribute('role');
    const ariaLabel = element.getAttribute('aria-label');
    const text = this.getElementText(element, 100);
    
    // Base score by tag name
    const tagScores: Record<string, number> = {
      'button': 10, 'a': 9, 'input': 8, 'select': 7, 'textarea': 7,
      'h1': 6, 'h2': 5, 'h3': 4, 'nav': 6, 'main': 5
    };
    
    score += tagScores[tagName] || 1;
    
    // Role score
    if (role) {
      const roleScores: Record<string, number> = {
        'button': 8, 'link': 7, 'search': 9, 'navigation': 6,
        'main': 5, 'banner': 4, 'complementary': 3
      };
      score += roleScores[role] || 2;
    }
    
    // Aria-label indicates importance
    if (ariaLabel) {
      score += 3;
    }
    
    // Text content relevance
    if (text) {
      // Keywords that indicate important interactive elements
      const importantKeywords = [
        'submit', 'send', 'search', 'login', 'register', 'buy', 'add',
        'save', 'delete', 'edit', 'cancel', 'confirm', 'next', 'previous'
      ];
      
      const lowerText = text.toLowerCase();
      for (const keyword of importantKeywords) {
        if (lowerText.includes(keyword)) {
          score += 2;
          break;
        }
      }
    }
    
    return score;
  }

  /**
   * Calculate interaction score for element
   */
  private calculateInteractionScore(element: Element): number {
    let score = 0;
    
    // Base interaction score
    if (this.isElementInteractive(element)) {
      score += 10;
    }
    
    // Visibility bonus
    if (this.isElementVisible(element)) {
      score += 5;
    }
    
    // Viewport bonus
    if (this.isElementInExpandedViewport(element, 0)) {
      score += 15;
    }
    
    // Size considerations
    const rect = this.getCachedBoundingRect(element);
    if (rect.width > 0 && rect.height > 0) {
      if (rect.width >= 30 && rect.height >= 20) {
        score += 5; // Good size
      } else if (rect.width < 10 || rect.height < 10) {
        score -= 5; // Too small
      }
    }
    
    // Cursor style bonus
    const style = this.getCachedComputedStyle(element);
    if (this.interactiveConfig.interactiveCursors.has(style.cursor)) {
      score += 8;
    }
    
    // Event listeners bonus
    if (this.detectEventListeners(element)) {
      score += 3;
    }
    
    return Math.max(0, score);
  }

  /**
   * Register shadow root for tracking
   */
  private registerShadowRoot(host: Element, shadowRoot: ShadowRoot): void {
    const info: ShadowDOMInfo = {
      host,
      shadowRoot,
      mode: shadowRoot.mode,
      delegatesFocus: shadowRoot.delegatesFocus
    };
    
    this.shadowRoots.set(host, info);
  }

  /**
   * Apply viewport prioritization to reduce element count
   */
  private applyViewportPrioritization(
    elementMap: Record<string, DOMElement | DOMTextNode>,
    options: DOMAnalysisOptions
  ): Record<string, DOMElement | DOMTextNode> {
    // Extract and score interactive elements
    const scoredElements: Array<{
      key: string;
      element: DOMElement;
      score: number;
    }> = [];
    
    for (const [key, node] of Object.entries(elementMap)) {
      if (node.type !== 'TEXT_NODE') {
        const element = node as DOMElement;
        if (element.isInteractive && element.highlightIndex !== undefined) {
          const score = this.calculateViewportScore(element, options.viewportExpansion);
          scoredElements.push({ key, element, score });
        }
      }
    }
    
    // Sort by score (higher is better)
    scoredElements.sort((a, b) => b.score - a.score);
    
    // Build filtered map
    const filteredMap: Record<string, DOMElement | DOMTextNode> = {};
    const maxElements = options.maxElements || 200;
    const selectedElements = scoredElements.slice(0, maxElements);
    
    // Always include body/root element
    for (const [key, node] of Object.entries(elementMap)) {
      if (node.type !== 'TEXT_NODE') {
        const element = node as DOMElement;
        if (element.tagName === 'body') {
          filteredMap[key] = element;
          break;
        }
      }
    }
    
    // Add selected high-priority elements
    for (const { key, element } of selectedElements) {
      filteredMap[key] = element;
      
      // Also include text children to preserve context
      for (const childId of element.children) {
        if (elementMap[childId] && elementMap[childId].type === 'TEXT_NODE') {
          filteredMap[childId] = elementMap[childId];
        }
      }
    }
    
    return filteredMap;
  }

  /**
   * Calculate viewport priority score for an element
   */
  private calculateViewportScore(element: DOMElement, viewportExpansion: number): number {
    let score = 0;
    
    // Base score for being interactive
    score += 10;
    
    // High priority for viewport elements
    if (element.isInViewport) {
      score += 50;
    } else if (viewportExpansion > 0) {
      // Calculate distance from viewport
      const distance = this.calculateViewportDistance(element.position);
      if (distance < 100) {
        score += Math.max(0, 20 - (distance / 5));
      }
    }
    
    // Priority by element type
    const typeScores: Record<string, number> = {
      'button': 20, 'input': 18, 'select': 16, 'textarea': 17,
      'a': 15, 'summary': 12, 'details': 10
    };
    score += typeScores[element.tagName] || 5;
    
    // Priority by role
    const roleScores: Record<string, number> = {
      'button': 10, 'link': 10, 'search': 15, 'submit': 15,
      'menu': 8, 'menuitem': 8
    };
    if (element.role) {
      score += roleScores[element.role] || 5;
    }
    
    // Text content bonus
    if (element.text && element.text.length > 0) {
      score += 5;
    }
    
    // Distinct interaction bonus
    if (element.isDistinctInteraction) {
      score += 10;
    }
    
    // Size considerations
    if (element.position.width < 20 || element.position.height < 20) {
      score -= 10; // Penalty for very small elements
    } else if (element.position.width >= 30 && element.position.height >= 20) {
      score += 5; // Bonus for reasonable size
    }
    
    // Interaction score bonus
    if (element.interactionScore) {
      score += element.interactionScore * 0.1; // Scale down interaction score
    }
    
    return Math.max(0, score);
  }

  /**
   * Calculate distance from viewport center
   */
  private calculateViewportDistance(position: ElementPosition): number {
    const viewportCenterX = window.innerWidth / 2;
    const viewportCenterY = window.innerHeight / 2;
    
    const elementCenterX = position.left + (position.width / 2);
    const elementCenterY = position.top + (position.height / 2);
    
    const deltaX = Math.abs(elementCenterX - viewportCenterX);
    const deltaY = Math.abs(elementCenterY - viewportCenterY);
    
    return Math.sqrt(deltaX * deltaX + deltaY * deltaY);
  }

  /**
   * Get current viewport information
   */
  private getViewportInfo(): ViewportInfo {
    return {
      width: window.innerWidth,
      height: window.innerHeight,
      scrollX: window.scrollX,
      scrollY: window.scrollY
    };
  }

  /**
   * Check if processing should be throttled
   */
  private shouldThrottleProcessing(options: DOMAnalysisOptions): boolean {
    const now = performance.now();
    if (now - this.lastProcessTime < this.processingThrottle) {
      return true;
    }
    this.lastProcessTime = now;
    return false;
  }

  /**
   * Create minimal DOM state for throttled requests
   */
  private createMinimalDOMState(): DOMState {
    return {
      rootId: 'body',
      elementMap: {},
      metrics: {
        totalNodes: 0,
        processedNodes: 0,
        interactiveNodes: 0,
        filteredNodes: 0,
        textNodes: 0,
        shadowRoots: 0,
        iframes: 0
      },
      viewport: this.getViewportInfo(),
      url: window.location.href,
      title: document.title,
      timestamp: Date.now()
    };
  }

  /**
   * Log performance metrics
   */
  private logPerformanceMetrics(metrics: DOMMetrics, processingTime: number): void {
    console.log('[RZN] DOM Analysis Performance Metrics:', {
      ...metrics,
      processingTimeMs: processingTime.toFixed(2),
      elementsPerMs: (metrics.processedNodes / processingTime).toFixed(2),
      efficiency: `${((metrics.interactiveNodes / metrics.totalNodes) * 100).toFixed(1)}%`
    });
  }

  /**
   * Highlight element visually
   */
  private highlightElement(element: Element, index: number): void {
    // Remove existing highlights for this element
    const existingHighlight = document.querySelector(`[data-rzn-highlight="${index}"]`);
    if (existingHighlight) {
      existingHighlight.remove();
    }
    
    const overlay = document.createElement('div');
    overlay.setAttribute('data-rzn-highlight', index.toString());
    overlay.style.cssText = `
      position: fixed;
      pointer-events: none;
      border: 2px solid #ff0000;
      background: rgba(255, 0, 0, 0.1);
      z-index: 10000;
      font-size: 12px;
      color: white;
      padding: 2px 4px;
      box-sizing: border-box;
    `;
    
    const rect = element.getBoundingClientRect();
    overlay.style.top = `${rect.top}px`;
    overlay.style.left = `${rect.left}px`;
    overlay.style.width = `${rect.width}px`;
    overlay.style.height = `${rect.height}px`;
    
    const label = document.createElement('div');
    label.textContent = index.toString();
    label.style.cssText = `
      position: absolute;
      top: -2px;
      right: -2px;
      background: #ff0000;
      color: white;
      padding: 1px 3px;
      font-size: 10px;
      border-radius: 2px;
      min-width: 14px;
      text-align: center;
    `;
    
    overlay.appendChild(label);
    document.body.appendChild(overlay);
    
    // Auto-remove after 30 seconds
    setTimeout(() => {
      if (overlay.parentNode) {
        overlay.parentNode.removeChild(overlay);
      }
    }, 30000);
  }

  // ============ PUBLIC API METHODS ============

  /**
   * Get simplified DOM representation
   */
  public getSimplifiedDom(options: { maxElements?: number } = {}): any {
    const domState = this.analyzeDOMTree({
      maxElements: options.maxElements || 100,
      prioritizeViewport: true,
      viewportExpansion: 0,
      debugMode: false
    });
    
    const interactiveElements = Object.values(domState.elementMap)
      .filter(node => node.type !== 'TEXT_NODE')
      .map(node => node as DOMElement)
      .filter(elem => elem.isInteractive);
    
    // Group by type for summary
    const byType: Record<string, number> = {};
    interactiveElements.forEach(elem => {
      byType[elem.type] = (byType[elem.type] || 0) + 1;
    });
    
    return {
      total: interactiveElements.length,
      inViewport: interactiveElements.filter(e => e.isInViewport).length,
      byType: byType,
      elements: interactiveElements.slice(0, options.maxElements || 100)
    };
  }

  /**
   * Find element by RZN ID
   */
  public findElementById(rznId: string): Element | null {
    if (!this.lastDOMState) {
      this.analyzeDOMTree();
    }
    
    const domElement = this.lastDOMState?.elementMap[rznId] as DOMElement;
    if (domElement && domElement.type !== 'TEXT_NODE') {
      // Try to find the actual DOM element using the selector
      if (domElement.selector.css) {
        return findElementEnhanced(domElement.selector.css);
      }
    }
    
    return null;
  }

  /**
   * Generate selector for element
   */
  public generateSelector(element: Element): string {
    const selector = this.generateElementSelector(element);
    return selector.css || element.tagName.toLowerCase();
  }

  /**
   * Detect DOM changes since last analysis
   */
  public detectChanges(): DOMDiff | null {
    if (!this.lastDOMState) return null;
    
    const currentState = this.analyzeDOMTree();
    const changes: DOMChangeInfo[] = [];
    const timestamp = Date.now();
    
    // Find added and modified elements
    for (const [id, currentElement] of Object.entries(currentState.elementMap)) {
      if (currentElement.type === 'TEXT_NODE') continue;
      
      const current = currentElement as DOMElement;
      const previous = this.lastDOMState.elementMap[id] as DOMElement;
      
      if (!previous) {
        changes.push({
          elementId: id,
          changeType: 'added',
          newHash: current.hash,
          timestamp
        });
      } else if (previous.hash !== current.hash) {
        changes.push({
          elementId: id,
          changeType: 'modified',
          oldHash: previous.hash,
          newHash: current.hash,
          timestamp
        });
      }
    }
    
    // Find removed elements
    for (const [id, previousElement] of Object.entries(this.lastDOMState.elementMap)) {
      if (previousElement.type === 'TEXT_NODE') continue;
      
      if (!currentState.elementMap[id]) {
        changes.push({
          elementId: id,
          changeType: 'removed',
          oldHash: (previousElement as DOMElement).hash,
          timestamp
        });
      }
    }
    
    return {
      changes,
      addedElements: changes.filter(c => c.changeType === 'added').map(c => c.elementId),
      removedElements: changes.filter(c => c.changeType === 'removed').map(c => c.elementId),
      modifiedElements: changes.filter(c => c.changeType === 'modified').map(c => c.elementId),
      timestamp
    };
  }

  /**
   * Find all interactive elements
   */
  public findInteractiveElements(root: Element = document.body): any[] {
    const domState = this.analyzeDOMTree({
      prioritizeViewport: false,
      viewportExpansion: -1
    });
    
    return Object.values(domState.elementMap)
      .filter(node => node.type !== 'TEXT_NODE')
      .map(node => node as DOMElement)
      .filter(elem => elem.isInteractive)
      .map(elem => ({
        id: elem.id,
        tagName: elem.tagName,
        selector: elem.selector.css || '',
        text: elem.text,
        type: elem.type,
        role: elem.role,
        position: elem.position,
        isVisible: elem.isVisible,
        isInteractive: elem.isInteractive,
        isInViewport: elem.isInViewport,
        cursor: elem.cursor
      }));
  }
}

// Export singleton instance
export const domAnalyzer = new DOMAnalyzer();
