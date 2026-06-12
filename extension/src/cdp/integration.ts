// Integration layer - Connects CDP modules to existing RZN actions
import { executeAction, type ActionParams } from './inputLadder';
import { reduceForLLM } from './domReducer';
import { strategy, ExecutionTier, type StrategyConfig } from './executionStrategy';
import { cdp } from './cdpHelper';

// Map RZN actions to new tiered system
export async function executeRZNAction(
  tabId: number,
  action: any
): Promise<{ success: boolean; error?: string; data?: any }> {
  console.log(`[Integration] Executing action: ${action.type}`);
  
  // Map RZN action types to new system
  const actionMap: Record<string, ActionParams['type']> = {
    'click_element': 'click',
    'fill_input_field': 'setValue',
    'type_text': 'type',
    'select_option': 'select',
    'scroll_element': 'scroll'
  };
  
  const mappedType = actionMap[action.type];
  if (!mappedType) {
    // Fall back to existing implementation for unmapped actions
    return { success: false, error: `Action ${action.type} not mapped to tiered system` };
  }
  
  // Build params
  const params: ActionParams = {
    type: mappedType,
    selector: action.selector,
    text: action.text,
    value: action.value || action.option_value,
    options: {
      // Detect if we need trusted events
      trusted: action.require_trusted || detectTrustedRequirement(action),
      // Detect cross-origin frames
      crossOrigin: action.cross_origin || detectCrossOrigin(action)
    }
  };
  
  // Execute with automatic tier selection
  const result = await executeAction(tabId, params);
  
  if (!result.success && result.escalated) {
    console.log(`[Integration] Action escalated to ${result.tier} due to: ${result.error}`);
  }
  
  return {
    success: result.success,
    error: result.error,
    data: {
      tier: result.tier,
      method: result.method,
      escalated: result.escalated
    }
  };
}

// Get DOM context for LLM
export async function getDOMContext(
  tabId: number,
  options?: {
    maxBytes?: number;
    preferCDP?: boolean;
  }
): Promise<any> {
  const context = await reduceForLLM(tabId, options);
  
  // Convert to RZN format
  return {
    summary: context.summary,
    elements: context.elements.map(el => ({
      token: el.id,
      selector: el.id,
      css_selector: el.selector || el.id,
      type: el.role,
      name: el.name,
      text: el.name || el.text,
      value: el.value,
      visible: el.visible,
      clickable: el.clickable,
      editable: el.editable,
      frame_id: el.frameId,
      frame_ordinal: el.frameOrdinal,
      actions: el.actions,
      attributes: el.attributes
    })),
    metadata: {
      ...context.metadata,
      extraction_method: strategy.config.tier,
      cdp_available: cdp.isAttached(tabId)
    }
  };
}

// Configure execution strategy
export function configureStrategy(config: Partial<StrategyConfig>) {
  strategy.updateConfig(config);
  console.log(`[Integration] Strategy updated:`, config);
}

// Get current strategy configuration
export function getStrategyConfig(): StrategyConfig {
  return strategy.exportConfig();
}

// Message handler for broker communication
export async function handleBrokerMessage(
  message: any,
  sender: chrome.runtime.MessageSender
): Promise<any> {
  const tabId = sender.tab?.id || message.tabId;
  
  switch (message.type) {
    case 'execute_action':
      return executeRZNAction(tabId, message.action);
      
    case 'get_dom_context':
      return getDOMContext(tabId, message.options);
      
    case 'configure_strategy':
      configureStrategy(message.config);
      return { success: true };
      
    case 'get_strategy':
      return getStrategyConfig();
      
    case 'set_tier':
      // Quick tier switch
      strategy.updateConfig({ tier: message.tier as ExecutionTier });
      return { success: true, tier: message.tier };
      
    case 'escalate':
      strategy.escalate();
      return { success: true, tier: strategy.exportConfig().tier };
      
    case 'deescalate':
      strategy.deescalate();
      return { success: true, tier: strategy.exportConfig().tier };
      
    default:
      return { success: false, error: 'Unknown message type' };
  }
}

// Auto-detection helpers
function detectTrustedRequirement(action: any): boolean {
  // Common patterns that require trusted events
  const trustedPatterns = [
    'file', 'upload', 'download',
    'payment', 'checkout', 'submit',
    'captcha', 'verify'
  ];
  
  const selector = action.selector?.toLowerCase() || '';
  const text = action.text?.toLowerCase() || '';
  
  return trustedPatterns.some(pattern => 
    selector.includes(pattern) || text.includes(pattern)
  );
}

function detectCrossOrigin(action: any): boolean {
  // Detect iframe selectors
  const selector = action.selector || '';
  return selector.includes('iframe') || 
         selector.includes('frame') ||
         action.frame_id !== undefined;
}

// Initialize with safe defaults
export function initialize() {
  // Set conservative defaults
  configureStrategy({
    tier: ExecutionTier.CDPFallback,
    features: {
      cdpForCrossOriginFrames: true,
      cdpForTrustedEvents: true,
      cdpForScreenshots: false,
      cdpWatchdogs: true
    },
    stealth: {
      actionDelayRange: [40, 160],
      humanizeMousePaths: true,
      humanizeTyping: true,
      cdpOnDemandOnly: false
    }
  });
  
  console.log('[Integration] Initialized with safe defaults');
}

// Export for use in service worker
export const cdpIntegration = {
  executeAction: executeRZNAction,
  getDOMContext,
  configureStrategy,
  getStrategyConfig,
  handleMessage: handleBrokerMessage,
  initialize
};
