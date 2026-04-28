// Intelligent DOM Analyzer - Minimizes LLM context while maximizing capability

interface ActionableElement {
  selector: string;
  type: 'button' | 'input' | 'link' | 'select' | 'textarea' | 'editable';
  label: string;
  context: string;
  confidence: number;
  attributes: Record<string, string>;
  visual: {
    position: 'top' | 'center' | 'bottom';
    size: 'small' | 'medium' | 'large';
    prominence: number; // 0-1
  };
}

interface PageContext {
  url: string;
  title: string;
  type: 'search' | 'form' | 'article' | 'product' | 'dashboard' | 'unknown';
  actionableElements: ActionableElement[];
  patterns: {
    hasSearchBox: boolean;
    hasLoginForm: boolean;
    hasPrices: boolean;
    hasComments: boolean;
    hasNavigation: boolean;
  };
}

export class IntelligentDOMAnalyzer {
  private static instance: IntelligentDOMAnalyzer;
  private patternCache: Map<string, string> = new Map();
  
  // Common patterns for different sites
  private readonly PATTERNS = {
    searchBox: [
      'input[type="search"]',
      'input[placeholder*="search" i]',
      'input[aria-label*="search" i]',
      'textarea[placeholder*="search" i]',
      'textarea[aria-label*="search" i]',
      '#search-input',
      '.search-box input'
    ],
    submitButton: [
      'button[type="submit"]',
      'input[type="submit"]',
      'button.primary',
      'button[aria-label*="search" i]',
      'button:has-text("Search")',
      'button:has-text("Submit")',
      'button:has-text("Go")'
    ],
    loginForm: [
      'form[action*="login"]',
      'form[action*="signin"]',
      '#login-form',
      '.login-form',
      'form:has(input[type="password"])'
    ],
    prices: [
      // Generic patterns
      '[class*="price" i][class*="current" i]',
      '.price-text',
      '.stock-price',
      'span[class*="price" i]:has-text("$")',
      '[data-testid*="price" i]'
    ],
    stockSymbols: [
      '[data-symbol]',
      '.ticker-symbol',
      '.stock-symbol',
      '[class*="ticker" i]',
      'h1:has-text(/^[A-Z]{1,5}$/)'
    ],
    comments: [
      'textarea[placeholder*="comment" i]',
      'textarea[placeholder*="reply" i]',
      '[contenteditable="true"]',
      '.comment-box',
      '#comment-textarea'
    ]
  };

  static getInstance(): IntelligentDOMAnalyzer {
    if (!this.instance) {
      this.instance = new IntelligentDOMAnalyzer();
    }
    return this.instance;
  }

  // Main analysis function - returns compressed context for LLM
  async analyzeForTask(task: string): Promise<PageContext> {
    const context: PageContext = {
      url: window.location.href,
      title: document.title,
      type: this.detectPageType(),
      actionableElements: [],
      patterns: {
        hasSearchBox: false,
        hasLoginForm: false,
        hasPrices: false,
        hasComments: false,
        hasNavigation: false
      }
    };

    // Quick pattern detection
    context.patterns.hasSearchBox = this.findPattern('searchBox') !== null;
    context.patterns.hasLoginForm = this.findPattern('loginForm') !== null;
    context.patterns.hasPrices = this.findPattern('prices') !== null;
    context.patterns.hasComments = this.findPattern('comments') !== null;
    context.patterns.hasNavigation = !!document.querySelector('nav, [role="navigation"]');

    // Extract only relevant elements based on task
    context.actionableElements = this.extractRelevantElements(task);

    return context;
  }

  // Find elements using pattern matching
  findPattern(patternType: keyof typeof this.PATTERNS): string | null {
    const cacheKey = `${window.location.hostname}:${patternType}`;
    
    // Check cache first
    if (this.patternCache.has(cacheKey)) {
      const cached = this.patternCache.get(cacheKey)!;
      if (document.querySelector(cached)) {
        return cached;
      }
    }

    // Try each pattern
    const patterns = this.PATTERNS[patternType];
    for (const selector of patterns) {
      try {
        // Handle special pseudo-selectors
        const elements = this.querySelectorWithText(selector);
        if (elements.length > 0) {
          // Cache successful pattern
          const finalSelector = this.generateStableSelector(elements[0]);
          this.patternCache.set(cacheKey, finalSelector);
          return finalSelector;
        }
      } catch (e) {
        // Invalid selector, continue
      }
    }

    return null;
  }

  // Extract only relevant elements for the task
  private extractRelevantElements(task: string): ActionableElement[] {
    const elements: ActionableElement[] = [];
    const taskKeywords = task.toLowerCase().split(' ');

    // Determine what to look for based on task
    const lookFor = {
      search: taskKeywords.some(k => ['search', 'find', 'look'].includes(k)),
      click: taskKeywords.some(k => ['click', 'press', 'tap'].includes(k)),
      fill: taskKeywords.some(k => ['fill', 'type', 'enter', 'write'].includes(k)),
      extract: taskKeywords.some(k => ['extract', 'get', 'find', 'price', 'value'].includes(k)),
      login: taskKeywords.some(k => ['login', 'signin', 'authenticate'].includes(k))
    };

    // Smart element extraction based on task
    if (lookFor.search) {
      let searchBox: Element | null = null;
      const fallbackSelector = this.findPattern('searchBox');
      if (fallbackSelector) {
        searchBox = document.querySelector(fallbackSelector);
      }
      
      if (searchBox) {
        elements.push(this.analyzeElement(searchBox as HTMLElement, 'input'));
      }
    }

    if (lookFor.extract && taskKeywords.some(k => ['price', 'stock', 'value'].includes(k))) {
      const priceElements = this.findPriceElements();
      elements.push(...priceElements);
    }

    if (lookFor.click || lookFor.fill) {
      // Get interactive elements
      const interactive = this.findInteractiveElements();
      elements.push(...interactive);
    }

    // Rank by relevance to task
    return this.rankByRelevance(elements, task);
  }

  // Find price elements intelligently
  private findPriceElements(): ActionableElement[] {
    const elements: ActionableElement[] = [];
    
    // Try known patterns first
    for (const selector of this.PATTERNS.prices) {
      try {
        const found = document.querySelectorAll(selector);
        found.forEach(el => {
          // Verify it contains price-like content
          const text = el.textContent || '';
          if (/\$?\d+\.?\d*/.test(text)) {
            elements.push(this.analyzeElement(el, 'price'));
          }
        });
      } catch (e) {
        // Continue with next pattern
      }
    }

    // Fallback: Find elements with price-like text
    if (elements.length === 0) {
      const allElements = document.querySelectorAll('span, div, p');
      allElements.forEach(el => {
        const text = el.textContent || '';
        // Match price patterns: $123.45, 123.45 USD, etc.
        if (/^\$?\d{1,3}(,\d{3})*(\.\d{2})?$/.test(text.trim())) {
          elements.push(this.analyzeElement(el, 'price'));
        }
      });
    }

    return elements;
  }

  // Find interactive elements
  private findInteractiveElements(): ActionableElement[] {
    const elements: ActionableElement[] = [];
    const selectors = [
      'button',
      'a[href]',
      'input:not([type="hidden"])',
      'select',
      'textarea',
      '[role="button"]',
      '[onclick]',
      '[contenteditable="true"]'
    ];

    selectors.forEach(selector => {
      const found = document.querySelectorAll(selector);
      found.forEach(el => {
        if (this.isVisible(el)) {
          elements.push(this.analyzeElement(el, this.getElementType(el)));
        }
      });
    });

    return elements;
  }

  // Analyze individual element
  private analyzeElement(element: Element, hint?: string): ActionableElement {
    const rect = element.getBoundingClientRect();
    const styles = window.getComputedStyle(element);
    
    return {
      selector: this.generateStableSelector(element),
      type: this.getElementType(element),
      label: this.getElementLabel(element),
      context: this.getElementContext(element),
      confidence: this.calculateConfidence(element, hint),
      attributes: this.getRelevantAttributes(element),
      visual: {
        position: rect.top < window.innerHeight / 3 ? 'top' : 
                  rect.top < window.innerHeight * 2 / 3 ? 'center' : 'bottom',
        size: this.getElementSize(rect),
        prominence: this.calculateProminence(rect, styles)
      }
    };
  }

  // Generate stable selector for element
  private generateStableSelector(element: Element): string {
    // Priority: id > data-testid > unique attributes > class + text
    
    if (element.id) {
      return `#${element.id}`;
    }

    if (element.getAttribute('data-testid')) {
      return `[data-testid="${element.getAttribute('data-testid')}"]`;
    }

    if (element.getAttribute('name')) {
      return `[name="${element.getAttribute('name')}"]`;
    }

    // Generate unique selector
    const tag = element.tagName.toLowerCase();
    const classes = Array.from(element.classList).slice(0, 2).join('.');
    const text = element.textContent?.slice(0, 20);
    
    if (classes) {
      return `${tag}.${classes}`;
    }

    if (text) {
      return `${tag}:contains("${text}")`;
    }

    // Fallback to nth-child
    const parent = element.parentElement;
    if (parent) {
      const index = Array.from(parent.children).indexOf(element);
      return `${this.generateStableSelector(parent)} > ${tag}:nth-child(${index + 1})`;
    }

    return tag;
  }

  // Helper methods
  private detectPageType(): PageContext['type'] {
    const url = window.location.href;
    const title = document.title.toLowerCase();

    if (url.includes('search') || title.includes('search')) return 'search';
    if (document.querySelector('form[action*="login"], form[action*="signin"]')) return 'form';
    if (document.querySelector('[itemtype*="Product"], .product-page')) return 'product';
    if (document.querySelector('article, [role="article"]')) return 'article';
    if (document.querySelector('.dashboard, #dashboard')) return 'dashboard';
    
    return 'unknown';
  }

  private getElementType(element: Element): ActionableElement['type'] {
    const tag = element.tagName.toLowerCase();
    
    if (tag === 'button' || element.getAttribute('role') === 'button') return 'button';
    if (tag === 'a') return 'link';
    if (tag === 'input') return 'input';
    if (tag === 'select') return 'select';
    if (tag === 'textarea') return 'textarea';
    if (element.getAttribute('contenteditable') === 'true') return 'editable';
    
    return 'button'; // Default
  }

  private getElementLabel(element: Element): string {
    // Try multiple sources for label
    return element.getAttribute('aria-label') ||
           element.getAttribute('title') ||
           element.textContent?.trim().slice(0, 50) ||
           element.getAttribute('placeholder') ||
           element.getAttribute('value') ||
           '';
  }

  private getElementContext(element: Element): string {
    // Get surrounding context
    const parent = element.parentElement;
    if (!parent) return '';

    const form = element.closest('form');
    if (form) {
      return `form: ${form.getAttribute('name') || form.getAttribute('id') || 'unnamed'}`;
    }

    const section = element.closest('section, article, nav, header, footer');
    if (section) {
      return `${section.tagName.toLowerCase()}: ${section.getAttribute('aria-label') || ''}`;
    }

    return parent.tagName.toLowerCase();
  }

  private calculateConfidence(element: Element, hint?: string): number {
    let confidence = 0.5;

    // Boost confidence for stable selectors
    if (element.id) confidence += 0.2;
    if (element.getAttribute('data-testid')) confidence += 0.2;
    if (element.getAttribute('aria-label')) confidence += 0.1;

    // Boost for matching hint
    if (hint) {
      const text = element.textContent?.toLowerCase() || '';
      if (text.includes(hint.toLowerCase())) confidence += 0.2;
    }

    // Boost for visible and prominent elements
    if (this.isVisible(element)) confidence += 0.1;

    return Math.min(confidence, 1.0);
  }

  private getRelevantAttributes(element: Element): Record<string, string> {
    const relevant = ['href', 'action', 'method', 'type', 'name', 'value', 'placeholder'];
    const attrs: Record<string, string> = {};

    relevant.forEach(attr => {
      const value = element.getAttribute(attr);
      if (value) attrs[attr] = value;
    });

    return attrs;
  }

  private isVisible(element: Element): boolean {
    const rect = element.getBoundingClientRect();
    const styles = window.getComputedStyle(element);

    return rect.width > 0 && 
           rect.height > 0 && 
           styles.display !== 'none' && 
           styles.visibility !== 'hidden' && 
           styles.opacity !== '0';
  }

  private getElementSize(rect: DOMRect): 'small' | 'medium' | 'large' {
    const area = rect.width * rect.height;
    if (area < 2000) return 'small';
    if (area < 10000) return 'medium';
    return 'large';
  }

  private calculateProminence(rect: DOMRect, styles: CSSStyleDeclaration): number {
    let prominence = 0;

    // Size factor
    const area = rect.width * rect.height;
    const viewportArea = window.innerWidth * window.innerHeight;
    prominence += Math.min(area / viewportArea * 10, 0.3);

    // Position factor (center is more prominent)
    const centerX = rect.left + rect.width / 2;
    const centerY = rect.top + rect.height / 2;
    const distFromCenter = Math.sqrt(
      Math.pow(centerX - window.innerWidth / 2, 2) +
      Math.pow(centerY - window.innerHeight / 2, 2)
    );
    prominence += Math.max(0, 0.3 - distFromCenter / 1000);

    // Visual weight
    if (styles.fontWeight === 'bold') prominence += 0.1;
    if (styles.fontSize.includes('large')) prominence += 0.1;
    if (styles.backgroundColor !== 'transparent') prominence += 0.1;
    if (styles.border !== 'none') prominence += 0.1;

    return Math.min(prominence, 1.0);
  }

  private rankByRelevance(elements: ActionableElement[], task: string): ActionableElement[] {
    const keywords = task.toLowerCase().split(' ');

    return elements.sort((a, b) => {
      let scoreA = a.confidence;
      let scoreB = b.confidence;

      // Boost score for keyword matches
      keywords.forEach(keyword => {
        if (a.label.toLowerCase().includes(keyword)) scoreA += 0.2;
        if (b.label.toLowerCase().includes(keyword)) scoreB += 0.2;
        if (a.context.toLowerCase().includes(keyword)) scoreA += 0.1;
        if (b.context.toLowerCase().includes(keyword)) scoreB += 0.1;
      });

      // Consider visual prominence
      scoreA += a.visual.prominence * 0.3;
      scoreB += b.visual.prominence * 0.3;

      return scoreB - scoreA;
    });
  }

  // Handle special pseudo-selectors
  private querySelectorWithText(selector: string): Element[] {
    // Handle :has-text() pseudo-selector
    const hasTextMatch = selector.match(/:has-text\("([^"]+)"\)/);
    if (hasTextMatch) {
      const baseSelector = selector.replace(/:has-text\("[^"]+"\)/, '');
      const text = hasTextMatch[1];
      const elements = document.querySelectorAll(baseSelector || '*');
      return Array.from(elements).filter(el => 
        el.textContent?.toLowerCase().includes(text.toLowerCase())
      );
    }

    // Handle :contains() pseudo-selector
    const containsMatch = selector.match(/:contains\("([^"]+)"\)/);
    if (containsMatch) {
      const baseSelector = selector.replace(/:contains\("[^"]+"\)/, '');
      const text = containsMatch[1];
      const elements = document.querySelectorAll(baseSelector || '*');
      return Array.from(elements).filter(el => 
        el.textContent?.toLowerCase().includes(text.toLowerCase())
      );
    }

    // Standard selector
    return Array.from(document.querySelectorAll(selector));
  }

  // Public API for getting minimal context
  async getMinimalContext(task: string): Promise<string> {
    const context = await this.analyzeForTask(task);
    
    // Format as minimal string for LLM
    const summary = [
      `Page: ${context.type} - ${context.title}`,
      `URL: ${context.url}`,
      context.patterns.hasSearchBox && 'Has search',
      context.patterns.hasLoginForm && 'Has login',
      context.patterns.hasPrices && 'Has prices',
      `Found ${context.actionableElements.length} relevant elements:`
    ].filter(Boolean).join('\n');

    const elements = context.actionableElements.slice(0, 5).map(el => 
      `- ${el.type}: "${el.label.slice(0, 30)}" (${el.selector})`
    ).join('\n');

    return `${summary}\n${elements}`;
  }
}

// Export singleton instance
export const intelligentDOM = IntelligentDOMAnalyzer.getInstance();
