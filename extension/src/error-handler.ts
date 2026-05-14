/**
 * Error handling module for the RZN browser extension
 * Provides structured error types, retry logic, and recovery mechanisms
 */

export enum ErrorCode {
  // Network errors
  NETWORK_TIMEOUT = 'NETWORK_TIMEOUT',
  NETWORK_ERROR = 'NETWORK_ERROR',
  
  // DOM errors
  ELEMENT_NOT_FOUND = 'ELEMENT_NOT_FOUND',
  STALE_ELEMENT = 'STALE_ELEMENT',
  ELEMENT_NOT_INTERACTABLE = 'ELEMENT_NOT_INTERACTABLE',
  ELEMENT_OBSCURED = 'ELEMENT_OBSCURED',
  MULTIPLE_ELEMENTS_FOUND = 'MULTIPLE_ELEMENTS_FOUND',
  INVALID_SELECTOR = 'INVALID_SELECTOR',
  
  // Permission errors
  PERMISSION_DENIED = 'PERMISSION_DENIED',
  DEBUGGER_ACCESS_DENIED = 'DEBUGGER_ACCESS_DENIED',
  
  // Validation errors
  MISSING_PARAMETER = 'MISSING_PARAMETER',
  INVALID_TYPE = 'INVALID_TYPE',
  
  // Execution errors
  SCRIPT_ERROR = 'SCRIPT_ERROR',
  TIMEOUT = 'TIMEOUT',
  USER_CANCELLED = 'USER_CANCELLED',
  
  // System errors
  BROKER_UNAVAILABLE = 'BROKER_UNAVAILABLE',
  EXTENSION_NOT_RESPONDING = 'EXTENSION_NOT_RESPONDING',
}

export interface ErrorContext {
  stepId?: string;
  selector?: string;
  url?: string;
  screenshot?: string;
  domSnapshot?: string;
  suggestedSelectors?: string[];
  viewport?: {
    width: number;
    height: number;
    devicePixelRatio: number;
  };
}

export interface RznError {
  code: ErrorCode;
  message: string;
  context?: ErrorContext;
  recoverable: boolean;
  retryable: boolean;
}

export class ErrorHandler {
  private static instance: ErrorHandler;
  private breadcrumbs: string[] = [];
  private maxBreadcrumbs = 50;
  
  static getInstance(): ErrorHandler {
    if (!ErrorHandler.instance) {
      ErrorHandler.instance = new ErrorHandler();
    }
    return ErrorHandler.instance;
  }
  
  addBreadcrumb(message: string): void {
    const timestamp = new Date().toISOString();
    this.breadcrumbs.push(`[${timestamp}] ${message}`);
    
    // Keep only the last N breadcrumbs
    if (this.breadcrumbs.length > this.maxBreadcrumbs) {
      this.breadcrumbs.shift();
    }
  }
  
  getBreadcrumbs(): string[] {
    return [...this.breadcrumbs];
  }
  
  clearBreadcrumbs(): void {
    this.breadcrumbs = [];
  }
  
  /**
   * Create a structured error from a DOM operation
   */
  async createDomError(
    code: ErrorCode,
    message: string,
    selector?: string,
    tabId?: number
  ): Promise<RznError> {
    const context: ErrorContext = { selector };
    
    // Try to capture additional context
    if (tabId) {
      try {
        // Capture screenshot
        const screenshot = await this.captureScreenshot(tabId);
        if (screenshot) {
          context.screenshot = screenshot;
        }
        
        // Get viewport info
        const viewport = await this.getViewportInfo(tabId);
        if (viewport) {
          context.viewport = viewport;
        }
        
        // Get DOM snapshot
        const domSnapshot = await this.getDomSnapshot(tabId, selector);
        if (domSnapshot) {
          context.domSnapshot = domSnapshot;
        }
        
        // Generate alternative selectors
        if (selector && code === ErrorCode.ELEMENT_NOT_FOUND) {
          const suggestions = await this.suggestAlternativeSelectors(tabId, selector);
          if (suggestions.length > 0) {
            context.suggestedSelectors = suggestions;
          }
        }
      } catch (error) {
        this.addBreadcrumb(`Failed to capture error context: ${error}`);
      }
    }
    
    return {
      code,
      message,
      context,
      recoverable: code === ErrorCode.ELEMENT_NOT_FOUND || code === ErrorCode.STALE_ELEMENT,
      retryable: code === ErrorCode.STALE_ELEMENT,
    };
  }
  
  /**
   * Create a network error
   */
  createNetworkError(message: string, url?: string): RznError {
    return {
      code: ErrorCode.NETWORK_ERROR,
      message,
      context: { url },
      recoverable: true,
      retryable: true,
    };
  }
  
  /**
   * Create a permission error
   */
  createPermissionError(message: string, resource: string): RznError {
    return {
      code: ErrorCode.PERMISSION_DENIED,
      message,
      context: { url: resource },
      recoverable: false,
      retryable: false,
    };
  }
  
  /**
   * Capture a screenshot of the current tab
   */
  private async captureScreenshot(tabId: number): Promise<string | null> {
    try {
      const tab = await chrome.tabs.get(tabId);
      if (tab.windowId === undefined || tab.windowId === chrome.windows.WINDOW_ID_NONE) {
        return null;
      }
      const dataUrl = await chrome.tabs.captureVisibleTab(
        tab.windowId,
        { format: 'png' }
      );
      return dataUrl;
    } catch (error) {
      this.addBreadcrumb(`Screenshot capture failed: ${error}`);
      return null;
    }
  }
  
  /**
   * Get viewport information
   */
  private async getViewportInfo(tabId: number): Promise<any> {
    try {
      const result = await chrome.tabs.sendMessage(tabId, {
        action: 'getViewportInfo',
      });
      return result;
    } catch (error) {
      return null;
    }
  }
  
  /**
   * Get DOM snapshot around the selector
   */
  private async getDomSnapshot(tabId: number, selector?: string): Promise<string | null> {
    try {
      const result = await chrome.tabs.sendMessage(tabId, {
        action: 'getDomSnapshot',
        selector,
      });
      return result;
    } catch (error) {
      return null;
    }
  }
  
  /**
   * Suggest alternative selectors using various strategies
   */
  private async suggestAlternativeSelectors(
    tabId: number,
    originalSelector: string
  ): Promise<string[]> {
    try {
      const result = await chrome.tabs.sendMessage(tabId, {
        action: 'suggestSelectors',
        selector: originalSelector,
      });
      return result || [];
    } catch (error) {
      return [];
    }
  }
  
  /**
   * Retry an operation with exponential backoff
   */
  async retryWithBackoff<T>(
    operation: () => Promise<T>,
    maxAttempts: number = 3,
    initialDelay: number = 1000,
    maxDelay: number = 30000,
    backoffFactor: number = 2
  ): Promise<T> {
    let lastError: any;
    
    for (let attempt = 0; attempt < maxAttempts; attempt++) {
      try {
        this.addBreadcrumb(`Retry attempt ${attempt + 1}/${maxAttempts}`);
        return await operation();
      } catch (error) {
        lastError = error;
        
        if (attempt < maxAttempts - 1) {
          const delay = Math.min(
            initialDelay * Math.pow(backoffFactor, attempt),
            maxDelay
          );
          
          // Add jitter (up to 10% of delay)
          const jitter = Math.random() * delay * 0.1;
          const totalDelay = delay + jitter;
          
          this.addBreadcrumb(`Waiting ${totalDelay}ms before retry`);
          await new Promise(resolve => setTimeout(resolve, totalDelay));
        }
      }
    }
    
    throw lastError;
  }
  
  /**
   * Convert error to response format
   */
  toResponse(error: RznError, taskId: string): any {
    return {
      action: 'execute_workflow',
      task_id: taskId,
      success: false,
      error_code: error.code,
      error_msg: error.message,
      error_context: error.context,
      breadcrumbs: this.getBreadcrumbs(),
      recovery_suggestions: this.getRecoverySuggestions(error),
    };
  }
  
  /**
   * Get recovery suggestions for an error
   */
  private getRecoverySuggestions(error: RznError): string[] {
    const suggestions: string[] = [];
    
    switch (error.code) {
      case ErrorCode.ELEMENT_NOT_FOUND:
        if (error.context?.suggestedSelectors?.length) {
          suggestions.push('Try using one of the suggested alternative selectors');
        }
        suggestions.push('Verify the page has fully loaded');
        suggestions.push('Check if the element is inside an iframe');
        break;
        
      case ErrorCode.STALE_ELEMENT:
        suggestions.push('Re-query the element before interacting');
        suggestions.push('Add a wait condition before the action');
        break;
        
      case ErrorCode.PERMISSION_DENIED:
        suggestions.push('Grant the required permissions to the extension');
        suggestions.push('Check if the site is in the extension\'s allowed origins');
        break;
        
      case ErrorCode.NETWORK_TIMEOUT:
        suggestions.push('Check your internet connection');
        suggestions.push('Increase the timeout duration');
        suggestions.push('Verify the URL is correct');
        break;
        
      case ErrorCode.DEBUGGER_ACCESS_DENIED:
        suggestions.push('Enable debugger permission for the extension');
        suggestions.push('Close any existing DevTools windows');
        break;
    }
    
    return suggestions;
  }
}

// Export singleton instance
export const errorHandler = ErrorHandler.getInstance();
