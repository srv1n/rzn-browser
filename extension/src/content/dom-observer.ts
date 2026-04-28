/**
 * DOM Observer - Handles dynamic content detection and waiting strategies
 */

export interface DOMChange {
  type: 'added' | 'removed' | 'modified' | 'text';
  selector?: string;
  element?: Element;
  oldValue?: string;
  newValue?: string;
  timestamp: number;
}

export interface ObservationResult {
  hasSignificantChanges: boolean;
  changes: DOMChange[];
  newInteractiveElements: number;
  domStabilized: boolean;
  observationDuration: number;
}

export interface WaitStrategy {
  waitForSelector?: string;
  waitForText?: string;
  waitForStability?: number; // ms without DOM changes
  maxWait?: number;
  checkInterval?: number;
}

/**
 * Observe DOM changes after an action
 */
export async function observeDOM(duration: number = 500): Promise<ObservationResult> {
  const startTime = Date.now();
  const changes: DOMChange[] = [];
  let lastChangeTime = startTime;
  let newInteractiveElements = 0;

  return new Promise((resolve) => {
    const observer = new MutationObserver((mutations) => {
      const currentTime = Date.now();
      lastChangeTime = currentTime;

      for (const mutation of mutations) {
        if (mutation.type === 'childList') {
          // Track added nodes
          mutation.addedNodes.forEach((node) => {
            if (node.nodeType === Node.ELEMENT_NODE) {
              const element = node as Element;
              
              // Check if it's an interactive element
              if (isInteractiveElement(element)) {
                newInteractiveElements++;
                changes.push({
                  type: 'added',
                  selector: getBestSelector(element),
                  element,
                  timestamp: currentTime
                });
              }
              
              // Also check descendants
              const interactiveDescendants = element.querySelectorAll(
                'a, button, input, textarea, select, [onclick], [role="button"]'
              );
              newInteractiveElements += interactiveDescendants.length;
            }
          });

          // Track removed nodes
          mutation.removedNodes.forEach((node) => {
            if (node.nodeType === Node.ELEMENT_NODE) {
              changes.push({
                type: 'removed',
                element: node as Element,
                timestamp: currentTime
              });
            }
          });
        } else if (mutation.type === 'attributes') {
          changes.push({
            type: 'modified',
            element: mutation.target as Element,
            oldValue: mutation.oldValue,
            newValue: (mutation.target as Element).getAttribute(mutation.attributeName!),
            timestamp: currentTime
          });
        } else if (mutation.type === 'characterData') {
          changes.push({
            type: 'text',
            element: mutation.target.parentElement!,
            oldValue: mutation.oldValue,
            newValue: mutation.target.textContent,
            timestamp: currentTime
          });
        }
      }
    });

    // Start observing
    observer.observe(document.body, {
      childList: true,
      attributes: true,
      characterData: true,
      subtree: true,
      attributeOldValue: true,
      characterDataOldValue: true
    });

    // Check periodically for stability
    const checkInterval = setInterval(() => {
      const now = Date.now();
      const timeSinceLastChange = now - lastChangeTime;
      const totalDuration = now - startTime;

      // Consider DOM stabilized if no changes for 100ms
      const domStabilized = timeSinceLastChange > 100;

      if (totalDuration >= duration || (domStabilized && totalDuration > 200)) {
        clearInterval(checkInterval);
        observer.disconnect();

        resolve({
          hasSignificantChanges: newInteractiveElements > 0 || changes.length > 5,
          changes: changes.slice(0, 50), // Limit to prevent memory issues
          newInteractiveElements,
          domStabilized,
          observationDuration: totalDuration
        });
      }
    }, 50);
  });
}

/**
 * Wait for specific conditions with timeout
 */
export async function waitForCondition(strategy: WaitStrategy): Promise<boolean> {
  const startTime = Date.now();
  const maxWait = strategy.maxWait || 5000;
  const checkInterval = strategy.checkInterval || 100;

  return new Promise((resolve) => {
    const checkConditions = () => {
      const elapsed = Date.now() - startTime;
      
      if (elapsed >= maxWait) {
        resolve(false);
        return;
      }

      let conditionMet = false;

      // Check for selector
      if (strategy.waitForSelector) {
        const element = document.querySelector(strategy.waitForSelector);
        if (element && isVisible(element as HTMLElement)) {
          conditionMet = true;
        }
      }

      // Check for text
      if (strategy.waitForText && !conditionMet) {
        const bodyText = document.body.textContent || '';
        if (bodyText.includes(strategy.waitForText)) {
          conditionMet = true;
        }
      }

      // Check for stability
      if (strategy.waitForStability && !conditionMet) {
        // This would need to be implemented with a separate observer
        // For now, we'll use a simple approach
        observeDOM(strategy.waitForStability).then(result => {
          if (result.domStabilized) {
            resolve(true);
          } else {
            setTimeout(checkConditions, checkInterval);
          }
        });
        return;
      }

      if (conditionMet) {
        resolve(true);
      } else {
        setTimeout(checkConditions, checkInterval);
      }
    };

    checkConditions();
  });
}

/**
 * Execute action with observation
 */
export async function executeWithObservation(
  action: () => Promise<any>,
  observationTime: number = 500
): Promise<{ result: any; observations: ObservationResult }> {
  // Execute the action
  const result = await action();
  
  // Observe DOM changes
  const observations = await observeDOM(observationTime);
  
  return { result, observations };
}

/**
 * Smart wait that adapts based on the action type
 */
export async function smartWait(actionType: string, context?: any): Promise<void> {
  switch (actionType) {
    case 'click_element':
      // After click, wait for DOM changes or stability
      await observeDOM(300);
      break;
      
    case 'fill_input_field':
      // After typing, wait a bit for autocomplete/suggestions
      await new Promise(resolve => setTimeout(resolve, 200));
      const result = await observeDOM(300);
      
      // If we see new elements (like autocomplete), wait a bit more
      if (result.newInteractiveElements > 0) {
        await new Promise(resolve => setTimeout(resolve, 200));
      }
      break;
      
    case 'navigate_to_url':
      // For navigation, we typically wait in the background script
      // But we can wait for initial DOM stability here
      await waitForCondition({
        waitForStability: 500,
        maxWait: 10000
      });
      break;
      
    case 'submit_input':
      // After form submission, wait for significant changes
      await waitForCondition({
        waitForStability: 300,
        maxWait: 5000
      });
      break;
      
    default:
      // Default wait
      await new Promise(resolve => setTimeout(resolve, 100));
  }
}

// Helper functions
function isInteractiveElement(element: Element): boolean {
  const interactiveTags = ['A', 'BUTTON', 'INPUT', 'TEXTAREA', 'SELECT'];
  const hasInteractiveRole = element.getAttribute('role') === 'button' || 
                            element.getAttribute('role') === 'link';
  const hasClickHandler = element.hasAttribute('onclick') || 
                         element.hasAttribute('ng-click') ||
                         element.hasAttribute('@click');
  
  return interactiveTags.includes(element.tagName) || 
         hasInteractiveRole || 
         hasClickHandler ||
         element.hasAttribute('tabindex');
}

function getBestSelector(element: Element): string {
  if (element.id) return `#${element.id}`;
  
  const testId = element.getAttribute('data-testid');
  if (testId) return `[data-testid="${testId}"]`;
  
  const ariaLabel = element.getAttribute('aria-label');
  if (ariaLabel) return `[aria-label="${ariaLabel}"]`;
  
  const className = element.classList[0];
  return className ? `${element.tagName.toLowerCase()}.${className}` : element.tagName.toLowerCase();
}

function isVisible(element: HTMLElement): boolean {
  const rect = element.getBoundingClientRect();
  const style = window.getComputedStyle(element);
  
  return (
    rect.width > 0 &&
    rect.height > 0 &&
    style.visibility !== 'hidden' &&
    style.display !== 'none' &&
    style.opacity !== '0'
  );
}