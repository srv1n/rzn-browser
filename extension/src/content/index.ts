// Content script entry point
import { domAnalyzer } from './dom-analyzer';
// Import static actions for CSP compliance
import { RZN_ACTIONS } from './actions-static';
import { LLMDOMFormatter } from './llm-dom-formatter';
// Import compact snapshot system
import { snapshotIntegration } from './snapshot-integration';
// Import flight recorder for debugging
import { flightRecorder } from '../recorder/flightRecorder';
// Import enhanced action executor
import { enhancedActionExecutor } from './actions-enhanced';

// Create a logger for content scripts that sends logs to background
const logger = {
  debug: (message: string, metadata?: any) => sendLog('debug', message, metadata),
  info: (message: string, metadata?: any) => sendLog('info', message, metadata),
  warn: (message: string, metadata?: any) => sendLog('warn', message, metadata),
  error: (message: string, error?: any, metadata?: any) => {
    const errorMeta = error instanceof Error ? {
      errorName: error.name,
      errorMessage: error.message,
      errorStack: error.stack,
      ...metadata
    } : { error: String(error), ...metadata };
    sendLog('error', message, errorMeta);
  }
};

function sendLog(level: string, message: string, metadata?: any) {
  // Always log to console
  const consoleMethod = level === 'error' ? console.error :
                       level === 'warn' ? console.warn :
                       level === 'info' ? console.info :
                       console.log;
  
  consoleMethod(`[RZN:CS] ${message}`, metadata || '');
  
  // Send to background script
  chrome.runtime.sendMessage({
    type: 'CONTENT_LOG',
    level,
    message,
    timestamp: new Date().toISOString(),
    context: 'content/index.ts',
    metadata
  }).catch(() => {
    // Ignore errors - background might not be ready
  });
}

// Use static actions directly instead of handler class

logger.info('RZN Content Script loaded');

// Initialize flight recorder
(async () => {
  try {
    await flightRecorder.startRecording();
    console.log('[RZN] Flight recorder started - Press Ctrl+Shift+E to export debug data');
  } catch (error) {
    console.warn('[RZN] Flight recorder initialization failed:', error);
  }
})();

// Global keyboard shortcut for flight recorder export
document.addEventListener('keydown', async (event) => {
  // Ctrl+Shift+E to export flight recorder
  if (event.ctrlKey && event.shiftKey && event.key === 'E') {
    event.preventDefault();
    console.log('[RZN] Exporting flight recorder data...');
    
    try {
      const response = await new Promise<any>((resolve, reject) => {
        chrome.runtime.sendMessage({ cmd: 'export_flight_recorder' }, (response) => {
          if (chrome.runtime.lastError) {
            reject(chrome.runtime.lastError);
          } else {
            resolve(response);
          }
        });
      });
      
      if (response.success) {
        console.log(`[RZN] Flight recorder exported as ${response.filename}`);
        // Show temporary notification
        const notification = document.createElement('div');
        notification.textContent = `✅ Debug data exported: ${response.filename}`;
        notification.style.cssText = `
          position: fixed; top: 20px; right: 20px; z-index: 999999;
          background: #4CAF50; color: white; padding: 12px 20px;
          border-radius: 4px; font-family: monospace; font-size: 12px;
          box-shadow: 0 2px 8px rgba(0,0,0,0.2);
        `;
        document.body.appendChild(notification);
        setTimeout(() => notification.remove(), 3000);
      } else {
        console.error('[RZN] Flight recorder export failed:', response.error);
      }
    } catch (error) {
      console.error('[RZN] Flight recorder export failed:', error);
    }
  }
});

// Listen for messages from background
// Set up message listener
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  // Handle ping for connection check
  if (message.type === 'PING') {
    sendResponse({ success: true });
    return false;
  }
  
  // Handle DOM analysis request
  if (message.type === 'ANALYZE_DOM') {
    try {
      const analysis = domAnalyzer.getSimplifiedDom(message.options || {});
      sendResponse({ success: true, data: analysis });
    } catch (error) {
      sendResponse({ success: false, error: error.message });
    }
    return false;
  }
  
  // Handle enhanced DOM analysis request
  if (message.type === 'ENHANCED_DOM_ANALYSIS') {
    try {
      const options = message.options || {
        highlightElements: true,
        focusHighlightIndex: -1,
        viewportExpansion: 0,
        debugMode: false
      };
      
      // Placeholder enhanced DOM analysis (module under development)
      const domTree = { 
        metrics: { processedNodes: 0, interactiveNodes: 0 },
        tree: {} 
      };
      sendResponse({ success: true, data: domTree });
      
      logger.debug('Enhanced DOM analysis completed', {
        elementCount: domTree.metrics.processedNodes,
        interactiveCount: domTree.metrics.interactiveNodes
      });
    } catch (error) {
      logger.error('Enhanced DOM analysis failed', error);
      sendResponse({ success: false, error: error.message });
    }
    return false;
  }
  
  // Handle find element by RZN ID
  if (message.type === 'FIND_ELEMENT_BY_ID') {
    try {
      const element = domAnalyzer.findElementById(message.rznId);
      if (element) {
        // Return the selector that can be used to find the element
        sendResponse({ success: true, selector: domAnalyzer.generateSelector(element) });
      } else {
        sendResponse({ success: false, error: 'Element not found' });
      }
    } catch (error) {
      sendResponse({ success: false, error: error.message });
    }
    return false;
  }
  
  // Handle compact snapshot generation request
  if (message.type === 'GENERATE_SNAPSHOT') {
    snapshotIntegration.handleSnapshotRequest(message)
      .then(snapshot => sendResponse({ success: true, data: snapshot }))
      .catch(error => {
        logger.error('Snapshot generation failed', error);
        sendResponse({ success: false, error: error.message });
      });
    
    return true; // Keep channel open for async response
  }

  // Handle element action with encoded ID
  if (message.type === 'ELEMENT_ACTION') {
    snapshotIntegration.handleElementAction(
      message.action, 
      message.encodedId, 
      message.details
    )
      .then(success => sendResponse({ success }))
      .catch(error => {
        logger.error('Element action failed', error);
        sendResponse({ success: false, error: error.message });
      });
    
    return true; // Keep channel open for async response
  }

  // Handle prompt generation request
  if (message.type === 'GENERATE_PROMPT') {
    snapshotIntegration.generatePrompt(message.includeMemory !== false)
      .then(prompt => sendResponse({ success: true, data: prompt }))
      .catch(error => {
        logger.error('Prompt generation failed', error);
        sendResponse({ success: false, error: error.message });
      });
    
    return true; // Keep channel open for async response
  }

  // Handle memory statistics request
  if (message.type === 'GET_MEMORY_STATS') {
    try {
      const stats = snapshotIntegration.getMemoryStats();
      sendResponse({ success: true, data: stats });
    } catch (error) {
      sendResponse({ success: false, error: error.message });
    }
    return false;
  }

  // Handle clear memory request
  if (message.type === 'CLEAR_MEMORY') {
    try {
      snapshotIntegration.clearMemory();
      sendResponse({ success: true });
    } catch (error) {
      sendResponse({ success: false, error: error.message });
    }
    return false;
  }

  // Handle flight recorder export request
  if (message.cmd === 'export_recorder_data') {
    (async () => {
      try {
        const { flightRecorder } = await import('../recorder/flightRecorder');
        const exportBlob = await flightRecorder.exportSession();
        const state = flightRecorder.getState();
        
        // Convert blob to JSON string for transfer
        const exportText = await exportBlob.text();
        
        sendResponse({
          success: true,
          data: exportText,
          session_id: state.session_id
        });
      } catch (error: any) {
        logger.error('Flight recorder export failed', error);
        sendResponse({ 
          success: false, 
          error: error.message || 'Failed to export flight recorder data'
        });
      }
    })();
    
    return true; // Keep channel open for async response
  }

  // Handle action execution
  if (message.type === 'EXECUTE_ACTION') {
    enhancedActionExecutor.execute(message.action)
      .then(sendResponse)
      .catch(error => {
        logger.error('Action execution error', error);
        sendResponse({
          success: false,
          error: error.message || 'Unknown error'
        });
      });
    
    // Keep message channel open for async response
    return true;
  }
  
  // Handle new static action format for CSP compliance
  if (message.rzn && message.cmd in RZN_ACTIONS) {
    Promise.resolve()
      .then(() => (RZN_ACTIONS as any)[message.cmd](...(message.args || [])))
      .then(data => sendResponse({ success: true, data }))
      .catch(error => {
        // Check for CSP errors
        if (/Content Security Policy/.test(error.message)) {
          sendResponse({ success: false, error: 'RZN_CSP_BLOCKED', details: error.message });
        } else {
          sendResponse({ success: false, error: error.message });
        }
      });
    
    return true; // Keep port open for async response
  }
  
  // Handle LLM DOM request
  if (message.type === 'GET_LLM_DOM') {
    try {
      // Get DOM analysis with indexing
      const domState = domAnalyzer.analyze({
        highlightElements: false,
        includeInvisible: false,
        viewportExpansion: 0
      });
      
      // Convert to LLM format
      const llmDom = LLMDOMFormatter.formatForLLM(domState);
      const actionPrompt = LLMDOMFormatter.generateActionPrompt(llmDom);
      
      sendResponse({
        success: true,
        data: {
          dom: llmDom,
          prompt: actionPrompt,
          elementCount: llmDom.elements.length
        }
      });
    } catch (error) {
      logger.error('Failed to generate LLM DOM', error);
      sendResponse({
        success: false,
        error: error.message || 'Failed to generate LLM DOM'
      });
    }
    return false;
  }
  
  // Step execution adapter removed
  
  // Handle DOM pruning request (legacy)
  if (message.cmd === 'get_pruned_dom') {
    getPrunedDOM(message).then(sendResponse);
    return true;
  }
  
  return false;
});

// Legacy adapter removed
function convertLegacyStep(step: any): any {
  const typeMapping: Record<string, string> = {
    'navigate_to_url': 'navigate_to_url',
    'click_element': 'click_element',
    'fill_input_field': 'fill_input_field',
    'submit_input': 'submit_input',
    'press_special_key': 'press_special_key',
    'wait_for_element': 'wait_for_element',
    'wait_for_timeout': 'wait_for_timeout',
    'extract_structured_data': 'extract_structured_data',
    'get_element_text': 'get_element_text',
    'scroll_window_to': 'scroll_window_to',
    'take_screenshot': 'take_screenshot',
    'get_page_source': 'get_page_source'
  };
  
  const actionType = typeMapping[step.type];
  console.log('Step conversion - Input type:', step.type, 'Mapped to:', actionType);
  if (!actionType) {
    logger.warn('Unknown legacy step type', { stepType: step.type });
    return null;
  }
  
  // Build action object
  const action: any = { type: actionType };
  
  // Adapter removed: return null to disable legacy path
  return null;
}

// DOM pruning utility
async function getPrunedDOM(message: any): Promise<any> {
  try {
    // Get full DOM
    const fullHTML = document.documentElement.outerHTML;
    
    // Basic pruning - remove scripts, styles, and comments
    const parser = new DOMParser();
    const doc = parser.parseFromString(fullHTML, 'text/html');
    
    // Remove script and style elements
    doc.querySelectorAll('script, style, link[rel="stylesheet"]').forEach(el => el.remove());
    
    // Remove comments
    const removeComments = (node: Node) => {
      for (let i = node.childNodes.length - 1; i >= 0; i--) {
        const child = node.childNodes[i];
        if (child.nodeType === Node.COMMENT_NODE) {
          child.remove();
        } else if (child.nodeType === Node.ELEMENT_NODE) {
          removeComments(child);
        }
      }
    };
    removeComments(doc.documentElement);
    
    // Remove data attributes
    doc.querySelectorAll('*').forEach(el => {
      const attrs = Array.from(el.attributes);
      attrs.forEach(attr => {
        if (attr.name.startsWith('data-')) {
          el.removeAttribute(attr.name);
        }
      });
    });
    
    const prunedHTML = doc.documentElement.outerHTML;
    
    return {
      req_id: message.req_id,
      success: true,
      result: {
        html: prunedHTML,
        url: window.location.href,
        title: document.title
      }
    };
  } catch (error) {
    return {
      req_id: message.req_id,
      success: false,
      error_msg: error instanceof Error ? error.message : 'Failed to prune DOM'
    };
  }
}

// Notify background that content script is ready
chrome.runtime.sendMessage({ type: 'CONTENT_READY' }).catch(() => {
  // Ignore errors if background is not ready
});

// Log that content script is loaded
logger.info('RZN Content Script loaded on page', { url: window.location.href });
