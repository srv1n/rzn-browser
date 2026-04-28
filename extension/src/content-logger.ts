// Content-safe logger for content scripts
// Uses chrome.runtime.sendMessage instead of native ports

interface LogMessage {
  type: 'CONTENT_LOG';
  level: 'debug' | 'info' | 'warn' | 'error';
  message: string;
  timestamp: string;
  context?: string;
  metadata?: Record<string, any>;
}

function getContext(): string {
  try {
    const stack = new Error().stack;
    if (!stack) return 'unknown';
    const lines = stack.split('\n');
    const relevantLine = lines[3] || lines[2];
    const match = relevantLine.match(/\s+at\s+(?:.*?\s+\()?(.*?):(\d+):(\d+)/);
    if (match) {
      const [, file, line] = match;
      return `${file.split('/').pop() || file}:${line}`;
    }
    return 'unknown';
  } catch {
    return 'unknown';
  }
}

function sendLog(level: string, message: string, metadata?: any) {
  // Always log to console
  const consoleMethod = level === 'error' ? console.error :
                       level === 'warn' ? console.warn :
                       level === 'info' ? console.info :
                       console.log;
  
  consoleMethod(`[RZN:CS] ${message}`, metadata || '');
  
  // Try to send to background script (best effort, don't fail if it can't)
  try {
    chrome.runtime.sendMessage({
      type: 'CONTENT_LOG',
      level,
      message,
      timestamp: new Date().toISOString(),
      context: getContext(),
      metadata
    }).catch(() => {
      // Ignore errors - background might not be ready
    });
  } catch {
    // Ignore - extension context might not be available
  }
}

export function logDebug(message: string, metadata?: any) {
  sendLog('debug', message, metadata);
}

export function logInfo(message: string, metadata?: any) {
  sendLog('info', message, metadata);
}

export function logWarn(message: string, metadata?: any) {
  sendLog('warn', message, metadata);
}

export function logError(message: string, error?: any, metadata?: any) {
  const errorMeta = error instanceof Error ? {
    errorName: error.name,
    errorMessage: error.message,
    errorStack: error.stack,
    ...metadata
  } : { error: String(error), ...metadata };
  sendLog('error', message, errorMeta);
}

// Stub for compatibility - content scripts don't manage native ports
export function setNativePort(port: any) {
  // No-op in content scripts
}