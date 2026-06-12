// Enhanced Actions with Element Resolution and Input Synthesis Ladder
// Provides stable element references and smart input escalation

import { 
  TargetSpec, 
  ResolvedElement, 
  ResultEnvelope, 
  InputRung,
  createResultEnvelope 
} from '../types/targets';
import { elementResolver } from '../resolver/elementResolver';
import { inputLadder, InputAction } from '../input/ladder_content';
import { flightRecorder } from '../recorder/flightRecorder';

// Enhanced action types that support both legacy selectors and new TargetSpec
export interface EnhancedActionBase {
  // New targeting system (preferred)
  target_spec?: TargetSpec;
  
  // Selector compatibility support
  selector?: string;
  css?: string;
  xpath?: string;
  
  // Action-specific options
  timeout?: number;
  retry_count?: number;
  force?: boolean; // Skip user activation checks
}

export interface EnhancedClickAction extends EnhancedActionBase {
  type: 'click_element';
  button?: 'left' | 'right' | 'middle';
  modifiers?: string[];
}

export interface EnhancedFillAction extends EnhancedActionBase {
  type: 'fill_input_field';
  value: string;
  clear?: boolean; // Whether to clear existing value first
}

export interface EnhancedKeyAction extends EnhancedActionBase {
  type: 'press_special_key';
  key: string;
  modifiers?: string[];
}

export interface EnhancedHoverAction extends EnhancedActionBase {
  type: 'hover_element';
}

export interface EnhancedScrollAction extends EnhancedActionBase {
  type: 'scroll_element_into_view';
}

export interface EnhancedExtractAction extends EnhancedActionBase {
  type: 'extract_structured_data';
  fields: Array<{
    name: string;
    selector?: string;
    target_spec?: TargetSpec;
    attribute?: string;
  }>;
  // Optional hint (kept for compatibility); no site-specific fast paths in runtime.
  extraction_type?: string;
}

export interface EnhancedTextAction extends EnhancedActionBase {
  type: 'get_element_text';
}

export type EnhancedAction = 
  | EnhancedClickAction
  | EnhancedFillAction
  | EnhancedKeyAction
  | EnhancedHoverAction
  | EnhancedScrollAction
  | EnhancedExtractAction
  | EnhancedTextAction;

export class EnhancedActionExecutor {
  private retryAttempts = 3;
  private baseDelay = 1000;

  /**
   * Execute an enhanced action with automatic element resolution and input escalation
   */
  async execute(action: EnhancedAction): Promise<ResultEnvelope<any>> {
    const startTime = performance.now();
    
    try {
      // Convert legacy selectors to TargetSpec if needed
      const targetSpec = this.normalizeTargetSpec(action);
      if (!targetSpec) {
        const result = createResultEnvelope(
          null,
          InputRung.DOM,
          false,
          performance.now() - startTime,
          undefined,
          'No valid targeting method provided'
        );
        
        // Record failed action
        await flightRecorder.recordAction(
          action.type, 
          false, 
          performance.now() - startTime,
          'No valid targeting method provided',
          { action }
        );
        
        return result;
      }

      // Resolve element with retries
      let resolvedElement: ResolvedElement;
      let lastError: string | undefined;
      
      for (let attempt = 0; attempt < (action.retry_count || this.retryAttempts); attempt++) {
        try {
          resolvedElement = await elementResolver.resolve(targetSpec);
          break;
        } catch (error) {
          lastError = error instanceof Error ? error.message : 'Element resolution failed';
          console.warn(`[EnhancedActions] Resolution attempt ${attempt + 1} failed:`, error);
          
          if (attempt < (action.retry_count || this.retryAttempts) - 1) {
            await this.delay(this.baseDelay * Math.pow(1.5, attempt));
          }
        }
      }

      if (!resolvedElement!) {
        return createResultEnvelope(
          null,
          InputRung.DOM,
          false,
          performance.now() - startTime,
          undefined,
          lastError || 'Failed to resolve element after all retries'
        );
      }

      // Execute the action based on type
      const result = await this.executeActionOnElement(action, resolvedElement);
      
      const executionTime = performance.now() - startTime;
      
      // Record action in flight recorder
      await flightRecorder.recordAction(
        action.type,
        result.success,
        executionTime,
        result.error,
        {
          action,
          resolved_element: resolvedElement,
          result_rung: result.rung,
          escalated: result.escalated
        }
      );
      
      return {
        ...result,
        execution_time_ms: executionTime,
        resolved_element: resolvedElement
      };
      
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : 'Unknown error';
      return createResultEnvelope(
        null,
        InputRung.DOM,
        false,
        performance.now() - startTime,
        undefined,
        errorMessage
      );
    }
  }

  /**
   * Execute specific action on resolved element
   */
  private async executeActionOnElement(
    action: EnhancedAction, 
    element: ResolvedElement
  ): Promise<ResultEnvelope<any>> {
    
    switch (action.type) {
      case 'click_element':
        return this.executeClick(action, element);
        
      case 'fill_input_field':
        return this.executeFill(action, element);
        
      case 'press_special_key':
        return this.executeKey(action, element);
        
      case 'hover_element':
        return this.executeHover(action, element);
        
      case 'scroll_element_into_view':
        return this.executeScroll(action, element);
        
      case 'extract_structured_data':
        return this.executeExtract(action, element);
        
      case 'get_element_text':
        return this.executeGetText(action, element);
        
      default:
        return createResultEnvelope(
          null,
          InputRung.DOM,
          false,
          0,
          element,
          `Unsupported action type: ${(action as any).type}`
        );
    }
  }

  private async executeClick(action: EnhancedClickAction, element: ResolvedElement): Promise<ResultEnvelope<boolean>> {
    const inputAction: InputAction = {
      type: 'click',
      options: {
        button: action.button || 'left',
        modifiers: action.modifiers || [],
        force: action.force || false
      }
    };

    return await inputLadder.execute(element, inputAction);
  }

  private async executeFill(action: EnhancedFillAction, element: ResolvedElement): Promise<ResultEnvelope<boolean>> {
    const inputAction: InputAction = {
      type: 'fill',
      value: action.value,
      options: {
        force: action.force || false
      }
    };

    return await inputLadder.execute(element, inputAction);
  }

  private async executeKey(action: EnhancedKeyAction, element: ResolvedElement): Promise<ResultEnvelope<boolean>> {
    const inputAction: InputAction = {
      type: 'key',
      key: action.key,
      options: {
        modifiers: action.modifiers || [],
        force: action.force || false
      }
    };

    return await inputLadder.execute(element, inputAction);
  }

  private async executeHover(action: EnhancedHoverAction, element: ResolvedElement): Promise<ResultEnvelope<boolean>> {
    const inputAction: InputAction = {
      type: 'hover'
    };

    return await inputLadder.execute(element, inputAction);
  }

  private async executeScroll(action: EnhancedScrollAction, element: ResolvedElement): Promise<ResultEnvelope<boolean>> {
    const inputAction: InputAction = {
      type: 'scroll'
    };

    return await inputLadder.execute(element, inputAction);
  }

  private async executeExtract(action: EnhancedExtractAction, element: ResolvedElement): Promise<ResultEnvelope<any>> {
    const startTime = performance.now();
    
    try {
      const results: Record<string, any> = {};
      
      // For same-origin elements, we can use DOM methods
      if (!element.is_cross_origin) {
        // Find the actual DOM element
        const domElement = await this.resolveToDOMElement(element);
        if (!domElement) {
          return createResultEnvelope(
            null,
            InputRung.DOM,
            false,
            0,
            element,
            'Could not resolve to DOM element for extraction'
          );
        }

        // Extract each field
        for (const field of action.fields) {
          try {
            let fieldElement = domElement;
            
            // If field has its own selector, find that element
            if (field.selector || field.target_spec) {
              const fieldTarget = field.target_spec || { css: field.selector };
              const fieldResolved = await elementResolver.resolve(fieldTarget);
              const fieldDOM = await this.resolveToDOMElement(fieldResolved);
              if (fieldDOM) {
                fieldElement = fieldDOM;
              }
            }

            // Extract the value
            if (field.attribute) {
              results[field.name] = fieldElement.getAttribute(field.attribute);
            } else {
              results[field.name] = fieldElement.textContent?.trim() || '';
            }
            
          } catch (error) {
            console.warn(`Failed to extract field ${field.name}:`, error);
            results[field.name] = null;
          }
        }

        return createResultEnvelope(results, InputRung.DOM, false, 0, element);
        
      } else {
        // Cross-origin extraction would require CDP
        return createResultEnvelope(
          null,
          InputRung.CDP,
          false,
          0,
          element,
          'Cross-origin extraction not yet implemented'
        );
      }
      
    } catch (error) {
      return createResultEnvelope(
        null,
        InputRung.DOM,
        false,
        0,
        element,
        error instanceof Error ? error.message : 'Extraction failed'
      );
    }
  }

  private async executeGetText(action: EnhancedTextAction, element: ResolvedElement): Promise<ResultEnvelope<string>> {
    try {
      if (!element.is_cross_origin) {
        const domElement = await this.resolveToDOMElement(element);
        if (!domElement) {
          return createResultEnvelope(
            '',
            InputRung.DOM,
            false,
            0,
            element,
            'Could not resolve to DOM element'
          );
        }

        const text = domElement.textContent?.trim() || '';
        return createResultEnvelope(text, InputRung.DOM, false, 0, element);
        
      } else {
        // Cross-origin text extraction would require CDP
        return createResultEnvelope(
          '',
          InputRung.CDP,
          false,
          0,
          element,
          'Cross-origin text extraction not yet implemented'
        );
      }
      
    } catch (error) {
      return createResultEnvelope(
        '',
        InputRung.DOM,
        false,
        0,
        element,
        error instanceof Error ? error.message : 'Text extraction failed'
      );
    }
  }

  /**
   * Convert legacy selectors to TargetSpec
   */
  private normalizeTargetSpec(action: EnhancedActionBase): TargetSpec | null {
    // If we already have a target_spec, use it
    if (action.target_spec) {
      return action.target_spec;
    }

    // Convert legacy selectors
    if (action.css || action.selector) {
      return { css: action.css || action.selector };
    }

    if (action.xpath) {
      return { xpath: action.xpath };
    }

    return null;
  }

  /**
   * Resolve ResolvedElement to actual DOM element (same-origin only)
   */
  private async resolveToDOMElement(element: ResolvedElement): Promise<Element | null> {
    if (element.is_cross_origin) {
      return null; // Can't access cross-origin DOM directly
    }

    // Try various resolution methods
    if (element.target_spec.css) {
      return document.querySelector(element.target_spec.css);
    }

    if (element.target_spec.xpath) {
      const result = document.evaluate(
        element.target_spec.xpath,
        document,
        null,
        XPathResult.FIRST_ORDERED_NODE_TYPE,
        null
      );
      return result.singleNodeValue as Element | null;
    }

    // As a fallback, try to find by encoded ID (if we have data attributes)
    const elementWithNodeId = document.querySelector(`[data-backend-node="${element.backend_node_id}"]`);
    if (elementWithNodeId) {
      return elementWithNodeId;
    }

    return null;
  }

  private delay(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
  }

  /**
   * Get performance statistics from the input ladder
   */
  getPerformanceStats() {
    return inputLadder.getPerformanceStats();
  }

  /**
   * Get element cache statistics
   */
  getCacheStats() {
    return elementResolver.getCacheStats();
  }

  /**
   * Clear caches
   */
  clearCaches(): void {
    elementResolver.clearCache();
  }
}

// Export singleton instance
export const enhancedActionExecutor = new EnhancedActionExecutor();
