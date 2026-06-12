/**
 * Action Memory Tracker
 * Tracks action history with one-line summaries for LLM context
 */

import {
  ActionMemory,
  MemoryTracker as IMemoryTracker,
  SNAPSHOT_CONFIG,
  ACTION_TYPES
} from './types';

export class MemoryTracker implements IMemoryTracker {
  private memories: ActionMemory[] = [];
  private cleanupTimer?: number;

  constructor() {
    this.startCleanupTimer();
  }

  /**
   * Add action to memory with automatic summarization
   */
  addAction(action: ActionMemory): void {
    // Generate summary if not provided
    if (!action.summary) {
      action.summary = this.generateActionSummary(action);
    }

    this.memories.unshift(action); // Add to beginning for recency
    
    // Limit memory size
    if (this.memories.length > SNAPSHOT_CONFIG.MAX_MEMORY_ITEMS) {
      this.memories = this.memories.slice(0, SNAPSHOT_CONFIG.MAX_MEMORY_ITEMS);
    }
    
    console.log('[RZN Memory] Added action:', action.summary);
  }

  /**
   * Get recent actions summary for LLM context
   */
  getRecentSummary(count: number = 5): string {
    if (this.memories.length === 0) {
      return 'No recent actions';
    }

    const recentActions = this.memories.slice(0, count);
    const summaries = recentActions.map((action, index) => {
      const timeAgo = this.getTimeAgo(action.timestamp);
      const status = action.success ? '✓' : '✗';
      return `${index + 1}. ${status} ${action.summary} (${timeAgo})`;
    });

    return 'Recent actions:\n' + summaries.join('\n');
  }

  /**
   * Cleanup old memories
   */
  cleanup(maxAge: number = SNAPSHOT_CONFIG.MEMORY_CLEANUP_INTERVAL): void {
    const cutoff = Date.now() - maxAge;
    const initialLength = this.memories.length;
    
    this.memories = this.memories.filter(memory => memory.timestamp >= cutoff);
    
    if (this.memories.length !== initialLength) {
      console.log('[RZN Memory] Cleaned up', initialLength - this.memories.length, 'old memories');
    }
  }

  /**
   * Get all memories for context
   */
  getAllMemories(): ActionMemory[] {
    return [...this.memories];
  }

  /**
   * Generate automatic action summary
   */
  private generateActionSummary(action: ActionMemory): string {
    const { action: actionType, targetId, success } = action;
    
    switch (actionType) {
      case ACTION_TYPES.CLICK:
        return targetId ? 
          `Clicked ${targetId}` : 
          'Clicked element';
      
      case ACTION_TYPES.TYPE:
        return targetId ? 
          `Typed in ${targetId}` : 
          'Typed text';
      
      case ACTION_TYPES.SELECT:
        return targetId ? 
          `Selected option in ${targetId}` : 
          'Selected option';
      
      case ACTION_TYPES.SCROLL:
        return 'Scrolled page';
      
      case ACTION_TYPES.NAVIGATE:
        return 'Navigated to new page';
      
      case ACTION_TYPES.WAIT:
        return targetId ? 
          `Waited for ${targetId}` : 
          'Waited for element';
      
      case ACTION_TYPES.EXTRACT:
        return targetId ? 
          `Extracted data from ${targetId}` : 
          'Extracted data';
      
      default:
        return targetId ? 
          `Performed ${actionType} on ${targetId}` : 
          `Performed ${actionType}`;
    }
  }

  /**
   * Get human-readable time ago string
   */
  private getTimeAgo(timestamp: number): string {
    const seconds = Math.floor((Date.now() - timestamp) / 1000);
    
    if (seconds < 60) {
      return `${seconds}s ago`;
    }
    
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) {
      return `${minutes}m ago`;
    }
    
    const hours = Math.floor(minutes / 60);
    if (hours < 24) {
      return `${hours}h ago`;
    }
    
    const days = Math.floor(hours / 24);
    return `${days}d ago`;
  }

  /**
   * Start automatic cleanup timer
   */
  private startCleanupTimer(): void {
    this.cleanupTimer = window.setInterval(() => {
      this.cleanup();
    }, SNAPSHOT_CONFIG.MEMORY_CLEANUP_INTERVAL);
  }

  /**
   * Stop cleanup timer
   */
  stopCleanupTimer(): void {
    if (this.cleanupTimer) {
      clearInterval(this.cleanupTimer);
      this.cleanupTimer = undefined;
    }
  }

  /**
   * Add click action to memory
   */
  addClickAction(targetId: string, success: boolean, details?: string): void {
    this.addAction({
      action: ACTION_TYPES.CLICK,
      targetId,
      summary: details || `Clicked ${targetId}`,
      timestamp: Date.now(),
      success
    });
  }

  /**
   * Add type action to memory
   */
  addTypeAction(targetId: string, text: string, success: boolean): void {
    const truncatedText = text.length > 20 ? text.slice(0, 17) + '...' : text;
    this.addAction({
      action: ACTION_TYPES.TYPE,
      targetId,
      summary: `Typed "${truncatedText}" in ${targetId}`,
      timestamp: Date.now(),
      success
    });
  }

  /**
   * Add select action to memory
   */
  addSelectAction(targetId: string, option: string, success: boolean): void {
    const truncatedOption = option.length > 20 ? option.slice(0, 17) + '...' : option;
    this.addAction({
      action: ACTION_TYPES.SELECT,
      targetId,
      summary: `Selected "${truncatedOption}" in ${targetId}`,
      timestamp: Date.now(),
      success
    });
  }

  /**
   * Add navigation action to memory
   */
  addNavigationAction(url: string, success: boolean): void {
    const truncatedUrl = this.truncateUrl(url);
    this.addAction({
      action: ACTION_TYPES.NAVIGATE,
      summary: `Navigated to ${truncatedUrl}`,
      timestamp: Date.now(),
      success
    });
  }

  /**
   * Add scroll action to memory
   */
  addScrollAction(direction: 'up' | 'down' | 'left' | 'right', success: boolean): void {
    this.addAction({
      action: ACTION_TYPES.SCROLL,
      summary: `Scrolled ${direction}`,
      timestamp: Date.now(),
      success
    });
  }

  /**
   * Add wait action to memory
   */
  addWaitAction(targetId: string, success: boolean, timeout?: number): void {
    const timeoutStr = timeout ? ` (${timeout}ms)` : '';
    this.addAction({
      action: ACTION_TYPES.WAIT,
      targetId,
      summary: `Waited for ${targetId}${timeoutStr}`,
      timestamp: Date.now(),
      success
    });
  }

  /**
   * Add extraction action to memory
   */
  addExtractionAction(targetId: string, itemCount: number, success: boolean): void {
    this.addAction({
      action: ACTION_TYPES.EXTRACT,
      targetId,
      summary: `Extracted ${itemCount} items from ${targetId}`,
      timestamp: Date.now(),
      success
    });
  }

  /**
   * Truncate URL for compact display
   */
  private truncateUrl(url: string): string {
    try {
      const urlObj = new URL(url);
      const domain = urlObj.hostname.replace(/^www\./, '');
      const path = urlObj.pathname;
      
      if (path === '/' || !path) {
        return domain;
      }
      
      const truncatedPath = path.length > 20 ? path.slice(0, 17) + '...' : path;
      return `${domain}${truncatedPath}`;
    } catch {
      return url.length > 30 ? url.slice(0, 27) + '...' : url;
    }
  }

  /**
   * Get success rate for recent actions
   */
  getSuccessRate(count: number = 10): number {
    if (this.memories.length === 0) return 0;
    
    const recentActions = this.memories.slice(0, count);
    const successCount = recentActions.filter(action => action.success).length;
    
    return Math.round((successCount / recentActions.length) * 100);
  }

  /**
   * Get most common failed actions
   */
  getFailurePatterns(): string[] {
    const failedActions = this.memories.filter(action => !action.success);
    const actionCounts: Record<string, number> = {};
    
    failedActions.forEach(action => {
      actionCounts[action.action] = (actionCounts[action.action] || 0) + 1;
    });
    
    return Object.entries(actionCounts)
      .sort(([, a], [, b]) => b - a)
      .slice(0, 3)
      .map(([action, count]) => `${action} (${count} failures)`);
  }

  /**
   * Clear all memories
   */
  clear(): void {
    this.memories = [];
    console.log('[RZN Memory] Cleared all memories');
  }

  /**
   * Get memory statistics
   */
  getStats(): {
    totalActions: number;
    successRate: number;
    recentSuccessRate: number;
    failurePatterns: string[];
  } {
    return {
      totalActions: this.memories.length,
      successRate: this.getSuccessRate(this.memories.length),
      recentSuccessRate: this.getSuccessRate(5),
      failurePatterns: this.getFailurePatterns()
    };
  }
}

// Export singleton instance
export const memoryTracker = new MemoryTracker();