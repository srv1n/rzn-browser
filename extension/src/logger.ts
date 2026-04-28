// Logger module for RZN Browser Extension
// Sends log messages through the native messaging channel to the runtime bridge

interface LogMessage {
  action: 'extension_log';
  component: 'ext';
  level: 'debug' | 'info' | 'warn' | 'error';
  message: string;
  timestamp: string;
  context?: string;
  metadata?: Record<string, any>;
}

// Check if we're in development mode
const isDevelopment = () => {
  // Check various ways to determine development mode
  try {
    // Chrome extensions don't have process.env, so check for debug flag in storage
    return (
      chrome.runtime.getManifest().version_name?.includes('dev') ||
      // Or check a custom flag in local storage (only available in content scripts)
      (typeof localStorage !== 'undefined' && localStorage.getItem('RZN_DEBUG') === 'true') ||
      // Default to true for now to enable logging
      true
    );
  } catch {
    // Default to true if any checks fail
    return true;
  }
};

// Store reference to native port from background script
let nativePort: chrome.runtime.Port | null = null;

// Queue for messages before native port is available
const messageQueue: LogMessage[] = [];
const MAX_QUEUE_SIZE = 100;

// Set the native port reference
export function setNativePort(port: chrome.runtime.Port | null) {
  nativePort = port;
  
  // Flush queued messages if we just connected
  if (port && messageQueue.length > 0) {
    const messages = messageQueue.splice(0, messageQueue.length);
    messages.forEach(msg => sendLogMessage(msg));
  }
}

// Format log message for UDP logger compatibility
function formatLogMessage(
  level: LogMessage['level'], 
  message: string, 
  context?: string,
  metadata?: Record<string, any>
): LogMessage {
  return {
    action: 'extension_log',
    component: 'ext',
    level,
    message,
    timestamp: new Date().toISOString(),
    context: context || getCallerContext(),
    metadata
  };
}

// Get caller context from stack trace
function getCallerContext(): string {
  try {
    const stack = new Error().stack;
    if (!stack) return 'unknown';
    
    const lines = stack.split('\n');
    // Skip Error line and current function, find actual caller
    const callerLine = lines[3] || lines[2];
    
    // Extract file and line info
    const match = callerLine.match(/\s+at\s+(?:.*?\s+\()?(.+?):(\d+):(\d+)/);
    if (match) {
      const [, file, line] = match;
      const filename = file.split('/').pop() || file;
      return `${filename}:${line}`;
    }
    
    return 'unknown';
  } catch {
    return 'unknown';
  }
}

// Send log message through native messaging
function sendLogMessage(logMsg: LogMessage) {
  // Always log to console as fallback
  const consoleMethod = logMsg.level === 'error' ? console.error :
                       logMsg.level === 'warn' ? console.warn :
                       logMsg.level === 'info' ? console.info :
                       console.log;
  
  const consolePrefix = `[RZN:${logMsg.context}]`;
  if (logMsg.metadata) {
    consoleMethod(consolePrefix, logMsg.message, logMsg.metadata);
  } else {
    consoleMethod(consolePrefix, logMsg.message);
  }
  
  // Only send to broker in development mode
  if (!isDevelopment()) {
    return;
  }
  
  // Send through native messaging if available
  if (nativePort) {
    try {
      // Check if port is still connected before sending
      if (nativePort.onDisconnect) {
        nativePort.postMessage(logMsg);
      } else {
        // Port is disconnected, clear it
        nativePort = null;
      }
    } catch (error) {
      // Native port might be disconnected
      // Only log error in debug mode to avoid spam
      if (isDevelopment()) {
        console.debug('[RZN:Logger] Native port disconnected, queueing message');
      }
      nativePort = null;
      
      // Queue the message if space available
      if (messageQueue.length < MAX_QUEUE_SIZE) {
        messageQueue.push(logMsg);
      }
    }
  } else {
    // Queue the message if native port not yet available
    if (messageQueue.length < MAX_QUEUE_SIZE) {
      messageQueue.push(logMsg);
    }
  }
}

// Public logging functions
export function logDebug(message: string, metadata?: Record<string, any>) {
  const logMsg = formatLogMessage('debug', message, undefined, metadata);
  sendLogMessage(logMsg);
}

export function logInfo(message: string, metadata?: Record<string, any>) {
  const logMsg = formatLogMessage('info', message, undefined, metadata);
  sendLogMessage(logMsg);
}

export function logWarn(message: string, metadata?: Record<string, any>) {
  const logMsg = formatLogMessage('warn', message, undefined, metadata);
  sendLogMessage(logMsg);
}

export function logError(message: string, error?: Error | unknown, metadata?: Record<string, any>) {
  const errorMetadata = {
    ...metadata,
    ...(error instanceof Error ? {
      errorName: error.name,
      errorMessage: error.message,
      errorStack: error.stack
    } : error ? {
      error: String(error)
    } : {})
  };
  
  const logMsg = formatLogMessage('error', message, undefined, errorMetadata);
  sendLogMessage(logMsg);
}

// Helper for logging with custom context
export function createLogger(context: string) {
  return {
    debug: (message: string, metadata?: Record<string, any>) => {
      const logMsg = formatLogMessage('debug', message, context, metadata);
      sendLogMessage(logMsg);
    },
    info: (message: string, metadata?: Record<string, any>) => {
      const logMsg = formatLogMessage('info', message, context, metadata);
      sendLogMessage(logMsg);
    },
    warn: (message: string, metadata?: Record<string, any>) => {
      const logMsg = formatLogMessage('warn', message, context, metadata);
      sendLogMessage(logMsg);
    },
    error: (message: string, error?: Error | unknown, metadata?: Record<string, any>) => {
      const errorMetadata = {
        ...metadata,
        ...(error instanceof Error ? {
          errorName: error.name,
          errorMessage: error.message,
          errorStack: error.stack
        } : error ? {
          error: String(error)
        } : {})
      };
      
      const logMsg = formatLogMessage('error', message, context, errorMetadata);
      sendLogMessage(logMsg);
    }
  };
}

// Export function to enable/disable debug mode at runtime
export function setDebugMode(enabled: boolean) {
  if (typeof localStorage !== 'undefined') {
    if (enabled) {
      localStorage.setItem('RZN_DEBUG', 'true');
    } else {
      localStorage.removeItem('RZN_DEBUG');
    }
  }
}

// Initialize logger after Chrome APIs are available
if (typeof chrome !== 'undefined' && chrome.runtime) {
  // Log initialization
  logInfo('RZN Logger initialized', {
    debugMode: isDevelopment(),
    context: 'logger.ts'
  });
}
