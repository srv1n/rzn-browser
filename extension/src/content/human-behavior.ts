/**
 * Human-like behavior simulation for bot detection avoidance
 */

export class HumanBehavior {
  /**
   * Random delay between min and max milliseconds
   */
  static async randomDelay(min: number = 100, max: number = 3000): Promise<void> {
    const delay = Math.floor(Math.random() * (max - min + 1)) + min;
    await new Promise(resolve => setTimeout(resolve, delay));
  }

  /**
   * Simulate human-like scrolling with natural patterns
   */
  static async naturalScroll(duration: number = 5000): Promise<void> {
    const startTime = Date.now();
    const scrollPatterns = [
      { direction: 1, speed: 2, pause: 500 },   // Slow down
      { direction: 1, speed: 5, pause: 300 },   // Normal down
      { direction: -1, speed: 3, pause: 700 },  // Back up a bit
      { direction: 1, speed: 8, pause: 200 },   // Fast down
      { direction: 0, speed: 0, pause: 1000 },  // Read pause
      { direction: -1, speed: 4, pause: 400 },  // Scroll up
    ];

    while (Date.now() - startTime < duration) {
      const pattern = scrollPatterns[Math.floor(Math.random() * scrollPatterns.length)];
      
      if (pattern.direction !== 0) {
        // Smooth scroll with easing
        const scrollAmount = pattern.direction * pattern.speed * (10 + Math.random() * 20);
        window.scrollBy({
          top: scrollAmount,
          behavior: 'smooth'
        });
      }
      
      // Natural pause between scrolls
      await this.randomDelay(pattern.pause * 0.7, pattern.pause * 1.3);
    }
  }

  /**
   * Simulate mouse movement
   */
  static generateMousePath(start: {x: number, y: number}, end: {x: number, y: number}): Array<{x: number, y: number}> {
    const points = [];
    const steps = 20 + Math.floor(Math.random() * 10);
    
    for (let i = 0; i <= steps; i++) {
      const t = i / steps;
      // Add bezier curve with slight randomness
      const x = start.x + (end.x - start.x) * t + (Math.random() - 0.5) * 5;
      const y = start.y + (end.y - start.y) * t + (Math.random() - 0.5) * 5;
      points.push({x: Math.round(x), y: Math.round(y)});
    }
    
    return points;
  }

  /**
   * Random micro-movements while "reading"
   */
  static async microMovements(duration: number = 2000): Promise<void> {
    const startTime = Date.now();
    
    while (Date.now() - startTime < duration) {
      // Small random scrolls like when reading
      const microScroll = (Math.random() - 0.5) * 50;
      window.scrollBy({
        top: microScroll,
        behavior: 'smooth'
      });
      
      await this.randomDelay(200, 800);
    }
  }

  /**
   * Simulate user reading time based on content length
   */
  static calculateReadTime(element: Element): number {
    const text = element.textContent || '';
    const wordCount = text.split(/\s+/).length;
    // Average reading speed: 200-250 words per minute
    const baseTime = (wordCount / 225) * 60 * 1000; // Convert to milliseconds
    // Add variance
    return baseTime * (0.8 + Math.random() * 0.4);
  }

  /**
   * Natural typing with mistakes and corrections
   */
  static async typeNaturally(element: HTMLInputElement | HTMLTextAreaElement, text: string): Promise<void> {
    element.focus();
    element.value = '';
    
    for (let i = 0; i < text.length; i++) {
      // 5% chance of typo
      if (Math.random() < 0.05 && i > 0 && i < text.length - 1) {
        // Make a typo
        const typoChar = String.fromCharCode(text.charCodeAt(i) + (Math.random() > 0.5 ? 1 : -1));
        element.value += typoChar;
        element.dispatchEvent(new Event('input', { bubbles: true }));
        
        await this.randomDelay(50, 150);
        
        // Realize mistake and backspace
        await this.randomDelay(100, 300);
        element.value = element.value.slice(0, -1);
        element.dispatchEvent(new Event('input', { bubbles: true }));
        await this.randomDelay(50, 100);
      }
      
      // Type the correct character
      element.value += text[i];
      element.dispatchEvent(new Event('input', { bubbles: true }));
      
      // Natural typing rhythm
      const baseDelay = 50;
      const variance = 100;
      await this.randomDelay(baseDelay, baseDelay + variance);
      
      // Occasional pause (thinking)
      if (Math.random() < 0.1) {
        await this.randomDelay(300, 800);
      }
    }
    
    element.dispatchEvent(new Event('change', { bubbles: true }));
  }

  /**
   * Move mouse to element with natural path
   */
  static async moveToElement(element: Element): Promise<void> {
    const rect = element.getBoundingClientRect();
    const targetX = rect.left + rect.width / 2 + (Math.random() - 0.5) * rect.width * 0.3;
    const targetY = rect.top + rect.height / 2 + (Math.random() - 0.5) * rect.height * 0.3;
    
    // Create visual indicator (for debugging) - commented out to avoid process error
    // const indicator = document.createElement('div');
    // indicator.style.position = 'fixed';
    // indicator.style.width = '10px';
    // indicator.style.height = '10px';
    // indicator.style.borderRadius = '50%';
    // indicator.style.backgroundColor = 'red';
    // indicator.style.pointerEvents = 'none';
    // indicator.style.zIndex = '99999';
    // indicator.style.left = targetX + 'px';
    // indicator.style.top = targetY + 'px';
    // document.body.appendChild(indicator);
    // setTimeout(() => indicator.remove(), 1000);
    
    // Simulate thinking time before moving
    await this.randomDelay(100, 400);
  }

  /**
   * Hover over element naturally
   */
  static async hoverElement(element: Element, duration: number = 500): Promise<void> {
    const mouseEnter = new MouseEvent('mouseenter', {
      view: window,
      bubbles: true,
      cancelable: true
    });
    element.dispatchEvent(mouseEnter);
    
    await this.randomDelay(duration * 0.8, duration * 1.2);
    
    const mouseLeave = new MouseEvent('mouseleave', {
      view: window,
      bubbles: true,
      cancelable: true
    });
    element.dispatchEvent(mouseLeave);
  }

  /**
   * Check if we should act more cautiously (e.g., on sensitive sites)
   */
  static shouldBeExtraCautious(): boolean {
    // Avoid domain-tuned rules. Use a conservative heuristic based on page signals
    // that commonly indicate sensitive flows (auth, payments, uploads).
    try {
      const hasPassword = !!document.querySelector('input[type="password"]');
      const hasOtp = !!document.querySelector('input[autocomplete="one-time-code"]');
      const hasFileUpload = !!document.querySelector('input[type="file"]');

      const hasCcAutocomplete = !!document.querySelector(
        'input[autocomplete^="cc-"], input[autocomplete*="cc-" i]'
      );
      const hasCcKeywords = !!document.querySelector(
        'input[name*="card" i], input[placeholder*="card" i], input[name*="cvv" i], input[placeholder*="cvv" i]'
      );

      const hasAuthKeywords = !!document.querySelector(
        'form[action*="login" i], form[action*="signin" i], form[action*="auth" i]'
      );

      return (
        hasPassword ||
        hasOtp ||
        hasFileUpload ||
        hasCcAutocomplete ||
        hasCcKeywords ||
        hasAuthKeywords
      );
    } catch {
      return false;
    }
  }

  /**
   * Add natural behavior before action
   */
  static async beforeAction(actionType: 'click' | 'type' | 'scroll'): Promise<void> {
    const extraCautious = this.shouldBeExtraCautious();
    
    switch (actionType) {
      case 'click':
        await this.randomDelay(
          extraCautious ? 200 : 100,
          extraCautious ? 1000 : 500
        );
        break;
      case 'type':
        await this.randomDelay(
          extraCautious ? 300 : 150,
          extraCautious ? 1500 : 700
        );
        break;
      case 'scroll':
        await this.randomDelay(
          extraCautious ? 100 : 50,
          extraCautious ? 500 : 300
        );
        break;
    }
  }
}
