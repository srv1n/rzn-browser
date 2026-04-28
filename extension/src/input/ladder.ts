// Input Synthesis Ladder - Smart escalation from DOM events to CDP
// Determines minimum sufficient input method and tracks usage

import { ResolvedElement, InputRung, ResultEnvelope, createResultEnvelope } from '../types/targets';
import { DOMInputExecutor } from './rungs/dom';
import { ScriptedInputExecutor } from './rungs/scripted';
import { CDPInputExecutor } from './rungs/cdp';
import { frameRouter } from '../cdp/frameRouter';
import { getFlags } from '../config/flags';
import { recordActionMetric } from '../background';

export interface InputAction {
  type: 'click' | 'fill' | 'key' | 'hover' | 'scroll' | 'type_and_submit' | 'batch_actions';
  value?: string; // For fill and key actions
  text?: string;  // For type_and_submit
  key?: string;   // For key actions
  submit?: boolean; // For type_and_submit (default true)
  wait?: 'navigation' | 'networkIdle' | 'selector'; // For type_and_submit
  waitSelector?: string; // For selector wait
  frameId?: string; // Frame routing
  steps?: Array<{  // For batch_actions
    op: 'click' | 'insert_text' | 'press_key' | 'wait_selector' | 'scroll_by';
    selector?: string;
    encodedId?: string;
    text?: string;
    key?: string;
    waitSelector?: string;
    dx?: number;
    dy?: number;
  }>;
  options?: {
    button?: 'left' | 'right' | 'middle';
    modifiers?: string[]; // ctrl, shift, alt, meta
    force?: boolean; // Skip user activation checks
  };
}

export interface LadderConfig {
  // Maximum time to spend on each rung before escalating (ms)
  rung_timeout: number;
  
  // Whether to probe for user activation gates
  check_user_activation: boolean;
  
  // Whether to skip rungs based on cross-origin status
  smart_escalation: boolean;
  
  // Enable performance tracking
  track_performance: boolean;
}

interface RungExecutor {
  canExecute(element: ResolvedElement, action: InputAction): boolean;
  execute(element: ResolvedElement, action: InputAction): Promise<boolean>;
}

export class InputLadder {
  private config: LadderConfig;
  private executors: Map<InputRung, RungExecutor>;
  private performanceStats: Map<InputRung, { attempts: number; successes: number; avgTime: number }>;

  constructor(config: Partial<LadderConfig> = {}) {
    this.config = {
      rung_timeout: 5000,
      check_user_activation: true,
      smart_escalation: true,
      track_performance: true,
      ...config
    };

    // Initialize executors
    this.executors = new Map([
      [InputRung.DOM, new DOMInputExecutor()],
      [InputRung.SCRIPTED, new ScriptedInputExecutor()],
      [InputRung.CDP, new CDPInputExecutor()]
    ]);

    this.performanceStats = new Map();
    this.initializeStats();
  }

  /**
   * Execute input action with automatic escalation
   */
  async execute(element: ResolvedElement, action: InputAction): Promise<ResultEnvelope<boolean>> {
    const startTime = performance.now();
    const host = location.hostname;
    
    try {
      // Get current flags for this domain
      const flags = await getFlags(host);
      
      // Fast-path macro: type_and_submit does everything in one CDP macro
      if (action.type === 'type_and_submit') {
        if (!element) {
          const executionTime = performance.now() - startTime;
          recordActionMetric(host, false, executionTime, false);
          return createResultEnvelope(false, InputRung.CDP, false, executionTime, element, 'No element resolved for type_and_submit');
        }
        
        // Check if type_and_submit is disabled by flags
        if (!flags.typeAndSubmitRequired) {
          console.log('[InputLadder] type_and_submit disabled by flags, using fallback');
          // Could fallback to separate type + press_key actions here
        }
        
        // Ensure CDP is attached and extend lease
        await frameRouter.ensureAttachedForFrame(element.frameId);
        const cdpExecutor = this.executors.get(InputRung.CDP) as CDPInputExecutor;
        const result = await cdpExecutor.executeTypeAndSubmit(element, action);
        
        const executionTime = performance.now() - startTime;
        this.updateStats(InputRung.CDP, result.success, executionTime);
        recordActionMetric(host, result.success, executionTime, false);
        
        return createResultEnvelope(
          result.success,
          InputRung.CDP,
          false, // No escalation for macro
          executionTime,
          element,
          result.error
        );
      }

      // Fast-path batch actions: execute multiple steps in one CDP lease
      if (action.type === 'batch_actions') {
        const batchAction = action as any; // Cast to access batch-specific fields
        
        // Check if batch actions are disabled by circuit breaker
        if (!flags.batchActionsEnabled) {
          const executionTime = performance.now() - startTime;
          recordActionMetric(host, false, executionTime, false);
          return createResultEnvelope(false, InputRung.CDP, false, executionTime, element, 'batch_actions disabled by circuit breaker');
        }
        
        if (!batchAction.steps || !Array.isArray(batchAction.steps)) {
          const executionTime = performance.now() - startTime;
          recordActionMetric(host, false, executionTime, false);
          return createResultEnvelope(false, InputRung.CDP, false, executionTime, element, 'batch_actions requires steps array');
        }
        
        // Respect maxMacroSteps limit
        const steps = batchAction.steps.slice(0, flags.maxMacroSteps);
        if (steps.length < batchAction.steps.length) {
          console.log(`[InputLadder] Truncated batch from ${batchAction.steps.length} to ${steps.length} steps due to maxMacroSteps`);
        }
        
        const startFrameId = element?.frameId || batchAction.frameId || 'main';
        
        // Ensure CDP is attached and extend lease
        await frameRouter.ensureAttachedForFrame(startFrameId);
        const cdpExecutor = this.executors.get(InputRung.CDP) as CDPInputExecutor;
        const result = await cdpExecutor.executeBatchActions(startFrameId, steps);
        
        const executionTime = performance.now() - startTime;
        this.updateStats(InputRung.CDP, result.success, executionTime);
        recordActionMetric(host, result.success, executionTime, false);
        
        return createResultEnvelope(
          result.success,
          InputRung.CDP,
          false, // No escalation for batch macro
          executionTime,
          element,
          result.error
        );
      }

      // Normal escalation path for other actions
      const startingRung = this.determineStartingRung(element, action);
      let currentRung = startingRung;
      let escalated = false;
      let lastError: string | undefined;

      console.log(`[InputLadder] Starting with rung ${currentRung} for ${action.type} action`);

      // Try each rung until success or exhaustion
      while (currentRung <= InputRung.CDP) {
        try {
          const executor = this.executors.get(currentRung);
          if (!executor) {
            throw new Error(`No executor found for rung ${currentRung}`);
          }

          // Check if executor can handle this combination
          if (!executor.canExecute(element, action)) {
            console.log(`[InputLadder] Rung ${currentRung} cannot execute ${action.type}, escalating`);
            currentRung++;
            escalated = true;
            continue;
          }

          // Ensure CDP attachment if needed
          if (currentRung === InputRung.CDP) {
            // Check if CDP is disabled by circuit breaker
            if (!flags.cdpEnable) {
              console.log('[InputLadder] CDP disabled by circuit breaker, cannot escalate further');
              const executionTime = performance.now() - startTime;
              recordActionMetric(host, false, executionTime, true);
              return createResultEnvelope(false, currentRung, escalated, executionTime, element, 'CDP disabled by circuit breaker');
            }
            await frameRouter.ensureAttachedForFrame(element.frameId);
          }

          // Check user activation if required
          if (this.config.check_user_activation && currentRung === InputRung.DOM) {
            const needsActivation = await this.checkUserActivationRequired(element, action);
            if (needsActivation) {
              console.log('[InputLadder] User activation required, escalating to scripted events');
              currentRung = InputRung.SCRIPTED;
              escalated = true;
              continue;
            }
          }

          // Execute with timeout
          const success = await this.executeWithTimeout(
            executor,
            element,
            action,
            this.config.rung_timeout
          );

          const executionTime = performance.now() - startTime;

          if (success) {
            // Track success
            this.updateStats(currentRung, true, executionTime);
            recordActionMetric(host, true, executionTime, currentRung === InputRung.CDP && !flags.cdpEnable);
            
            console.log(`[InputLadder] Success with rung ${currentRung} in ${executionTime.toFixed(2)}ms`);
            
            return createResultEnvelope(
              true,
              currentRung,
              escalated,
              executionTime,
              element
            );
          } else {
            // Track failure and try next rung
            this.updateStats(currentRung, false, executionTime);
            lastError = `Rung ${currentRung} failed to execute ${action.type}`;
            
            console.log(`[InputLadder] Rung ${currentRung} failed, trying next rung`);
            currentRung++;
            escalated = true;
          }
          
        } catch (error) {
          const errorMessage = error instanceof Error ? error.message : 'Unknown error';
          lastError = `Rung ${currentRung} error: ${errorMessage}`;
          
          console.warn(`[InputLadder] Rung ${currentRung} threw error:`, error);
          currentRung++;
          escalated = true;
        }
      }

      // All rungs failed
      const totalTime = performance.now() - startTime;
      console.error('[InputLadder] All rungs failed');
      
      return createResultEnvelope(
        false,
        currentRung - 1, // Last attempted rung
        escalated,
        totalTime,
        element,
        lastError || 'All input rungs failed'
      );
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : 'Unknown error';
      const totalTime = performance.now() - startTime;
      return createResultEnvelope(
        false,
        InputRung.CDP,
        false,
        totalTime,
        element,
        `Input execution error: ${errorMessage}`
      );
    }
  }

  /**
   * Get performance statistics for optimization
   */
  getPerformanceStats(): Record<InputRung, { attempts: number; successRate: number; avgTime: number }> {
    const stats: Record<number, { attempts: number; successRate: number; avgTime: number }> = {};
    
    for (const [rung, data] of this.performanceStats) {
      stats[rung] = {
        attempts: data.attempts,
        successRate: data.attempts > 0 ? data.successes / data.attempts : 0,
        avgTime: data.avgTime
      };
    }
    
    return stats;
  }

  /**
   * Reset performance statistics
   */
  resetStats(): void {
    this.initializeStats();
  }

  /**
   * Update ladder configuration
   */
  updateConfig(updates: Partial<LadderConfig>): void {
    this.config = { ...this.config, ...updates };
  }

  // Private methods

  private determineStartingRung(element: ResolvedElement, action: InputAction): InputRung {
    // Mandatory CDP conditions
    const requiresTrusted = 
      element.is_cross_origin ||
      element.frameId !== undefined ||
      this.isContentEditableHeavyTyping(element, action) ||
      this.isFileUpload(element) ||
      action.type === 'key' ||
      this.hasShadowDOMQuirks(element);
    
    if (requiresTrusted) {
      console.log('[InputLadder] Starting at CDP due to trust requirements');
      return InputRung.CDP;
    }

    // Prefer SCRIPTED for interactive elements; only use DOM for simple clicks
    if (this.isLikelyInteractive(element)) {
      return InputRung.SCRIPTED;
    }
    
    // Fall back to DOM for very simple elements
    return InputRung.DOM;
  }
  
  private isContentEditableHeavyTyping(element: ResolvedElement, action: InputAction): boolean {
    return element.target_spec?.css?.includes('contenteditable') && 
           action.type === 'fill' && 
           (action.value?.length || 0) > 80;
  }
  
  private isFileUpload(element: ResolvedElement): boolean {
    return element.target_spec?.css?.includes('input[type="file"]') || false;
  }
  
  private hasShadowDOMQuirks(element: ResolvedElement): boolean {
    return element.target_spec?.css?.includes('shadow') || false;
  }

  private async executeWithTimeout(
    executor: RungExecutor,
    element: ResolvedElement,
    action: InputAction,
    timeoutMs: number
  ): Promise<boolean> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(new Error(`Timeout after ${timeoutMs}ms`));
      }, timeoutMs);

      executor.execute(element, action)
        .then(result => {
          clearTimeout(timer);
          resolve(result);
        })
        .catch(error => {
          clearTimeout(timer);
          reject(error);
        });
    });
  }

  private async checkUserActivationRequired(element: ResolvedElement, action: InputAction): Promise<boolean> {
    // Check if action requires user activation (like opening new tabs, fullscreen, etc.)
    if (action.type === 'click') {
      try {
        // Test if we have user activation by attempting a restricted operation
        await navigator.permissions.query({ name: 'notifications' as any });
        return false; // If no error, we likely have activation
      } catch {
        // Permission API not available or restricted, assume we need activation
        return true;
      }
    }

    return false; // Other actions typically don't need user activation
  }

  private isModernInputField(element: ResolvedElement): boolean {
    // Check if element is a modern input field that might need scripted events
    // This would be determined by analyzing the element's properties
    // For now, we'll use a simple heuristic
    return element.target_spec.css?.includes('input[type=') || false;
  }

  private isLikelyInteractive(element: ResolvedElement): boolean {
    // Check if element is likely to need scripted events
    return this.isModernInputField(element) || this.isComplexUIComponent(element);
  }

  private isComplexUIComponent(element: ResolvedElement): boolean {
    // Check if element is part of a complex UI component (React, Vue, etc.)
    // This would be determined by analyzing class names, data attributes, etc.
    const css = element.target_spec.css || '';
    return css.includes('react-') || 
           css.includes('vue-') || 
           css.includes('ng-') ||
           css.includes('-component');
  }

  private updateStats(rung: InputRung, success: boolean, time: number): void {
    if (!this.config.track_performance) return;

    const stats = this.performanceStats.get(rung)!;
    stats.attempts++;
    
    if (success) {
      stats.successes++;
    }
    
    // Update rolling average
    stats.avgTime = (stats.avgTime * (stats.attempts - 1) + time) / stats.attempts;
  }

  private initializeStats(): void {
    this.performanceStats.clear();
    
    for (const rung of [InputRung.DOM, InputRung.SCRIPTED, InputRung.CDP]) {
      this.performanceStats.set(rung, {
        attempts: 0,
        successes: 0,
        avgTime: 0
      });
    }
  }
}

// Export singleton instance
export const inputLadder = new InputLadder();
