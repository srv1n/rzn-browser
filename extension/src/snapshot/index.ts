/**
 * Compact Snapshot System - Main Entry Point
 * Provides unified interface for snapshot generation and management
 */

import { snapshotBuilder } from './builder';
import { snapshotReducer } from './reducer';
import { memoryTracker } from './memory';
import {
  CompactSnapshot,
  CompactElement,
  ActionMemory,
  SNAPSHOT_CONFIG,
  ACTION_TYPES
} from './types';

export class SnapshotManager {
  private lastSnapshot: CompactSnapshot | null = null;
  
  /**
   * Generate compact snapshot for LLM consumption
   */
  async generateSnapshot(maxSizeKB: number = SNAPSHOT_CONFIG.MAX_SIZE_KB): Promise<CompactSnapshot> {
    console.log('[RZN Snapshot] Generating compact snapshot...');
    
    try {
      // Build initial snapshot
      let snapshot = await snapshotBuilder.buildSnapshot();
      
      // Reduce to target size if needed
      if (snapshot.sizeKB > maxSizeKB) {
        snapshot = snapshotReducer.reduce(snapshot, maxSizeKB);
      }
      
      console.log('[RZN Snapshot] Generated snapshot:', {
        elements: snapshot.elements.length,
        sizeKB: snapshot.sizeKB,
        compression: snapshot.compressionLevel
      });
      
      this.lastSnapshot = snapshot;
      return snapshot;
    } catch (error) {
      console.error('[RZN Snapshot] Failed to generate snapshot:', error);
      throw error;
    }
  }

  /**
   * Generate LLM-optimized prompt from snapshot
   */
  generatePrompt(snapshot: CompactSnapshot, includeMemory: boolean = true): string {
    let prompt = this.buildBasePrompt(snapshot);
    
    if (includeMemory) {
      const memorySummary = memoryTracker.getRecentSummary();
      if (memorySummary !== 'No recent actions') {
        prompt += '\n\n' + memorySummary;
      }
    }
    
    return prompt;
  }

  /**
   * Find element by encoded ID
   */
  findElement(encodedId: string): Element | null {
    return snapshotBuilder.getElementById(encodedId);
  }

  /**
   * Track action in memory
   */
  trackAction(actionType: string, targetId?: string, success: boolean = true, details?: any): void {
    const action: ActionMemory = {
      action: actionType,
      targetId,
      summary: this.generateActionSummary(actionType, targetId, success, details),
      timestamp: Date.now(),
      success
    };
    
    memoryTracker.addAction(action);
  }

  /**
   * Get memory statistics
   */
  getMemoryStats() {
    return memoryTracker.getStats();
  }

  /**
   * Clear action memory
   */
  clearMemory(): void {
    memoryTracker.clear();
  }

  /**
   * Build base prompt from snapshot
   */
  private buildBasePrompt(snapshot: CompactSnapshot): string {
    const lines: string[] = [];
    
    // Header
    lines.push(`Page: ${snapshot.title} (${this.formatUrl(snapshot.url)})`);
    lines.push(`Viewport: ${snapshot.viewport.width}x${snapshot.viewport.height}`);
    
    if (snapshot.frames.length > 0) {
      lines.push(`Frames: ${snapshot.frames.length} (${snapshot.frames.filter(f => f.accessible).length} accessible)`);
    }
    
    lines.push(''); // Empty line
    
    // Interactive elements
    if (snapshot.elements.length > 0) {
      lines.push('Interactive Elements:');
      
      const groupedElements = this.groupElementsByType(snapshot.elements);
      
      for (const [type, elements] of Object.entries(groupedElements)) {
        if (elements.length > 0) {
          lines.push(`\n${type.toUpperCase()} (${elements.length}):`);
          
          for (const element of elements) {
            const line = this.formatElementForPrompt(element);
            lines.push(`  ${line}`);
          }
        }
      }
    } else {
      lines.push('No interactive elements found.');
    }
    
    // Footer
    lines.push('');
    lines.push(`Generated: ${new Date(snapshot.timestamp).toLocaleTimeString()}`);
    lines.push(`Size: ${snapshot.sizeKB}KB, Compression: ${snapshot.compressionLevel}`);
    
    return lines.join('\n');
  }

  /**
   * Format URL for compact display
   */
  private formatUrl(url: string): string {
    try {
      const urlObj = new URL(url);
      const domain = urlObj.hostname.replace(/^www\./, '');
      return domain + (urlObj.pathname !== '/' ? urlObj.pathname : '');
    } catch {
      return url.length > 50 ? url.slice(0, 47) + '...' : url;
    }
  }

  /**
   * Group elements by type for organized display
   */
  private groupElementsByType(elements: CompactElement[]): Record<string, CompactElement[]> {
    const groups: Record<string, CompactElement[]> = {
      inputs: [],
      buttons: [],
      links: [],
      selects: [],
      other: []
    };
    
    for (const element of elements) {
      if (element.tag === 'input' || element.tag === 'textarea') {
        groups.inputs.push(element);
      } else if (element.tag === 'button' || (element.role === 'button')) {
        groups.buttons.push(element);
      } else if (element.tag === 'a' || element.role === 'link') {
        groups.links.push(element);
      } else if (element.tag === 'select' || element.role === 'combobox') {
        groups.selects.push(element);
      } else {
        groups.other.push(element);
      }
    }
    
    // Remove empty groups
    Object.keys(groups).forEach(key => {
      if (groups[key].length === 0) {
        delete groups[key];
      }
    });
    
    return groups;
  }

  /**
   * Format element for LLM prompt
   */
  private formatElementForPrompt(element: CompactElement): string {
    const parts: string[] = [];
    
    // Encoded ID (most important for LLM)
    parts.push(element.encodedId);
    
    // Selector
    parts.push(`[${element.selector}]`);
    
    // Text content
    if (element.text) {
      parts.push(`"${element.text}"`);
    } else if (element.name) {
      parts.push(`"${element.name}"`);
    }
    
    // Actions
    if (element.actions && element.actions.length > 0) {
      parts.push(`(${element.actions.join(', ')})`);
    }
    
    // Key attributes
    if (element.attrs) {
      const keyAttrs = this.getDisplayAttributes(element.attrs);
      if (keyAttrs.length > 0) {
        parts.push(`{${keyAttrs.join(', ')}}`)
      }
    }
    
    // Type if different from tag
    if (element.type && element.type !== element.tag) {
      parts.push(`type:${element.type}`);
    }
    
    return parts.join(' ');
  }

  /**
   * Get key attributes for display
   */
  private getDisplayAttributes(attrs: Record<string, string>): string[] {
    const display: string[] = [];
    const priority = ['data-testid', 'id', 'name', 'type', 'placeholder'];
    
    for (const key of priority) {
      if (attrs[key]) {
        display.push(`${key}=${attrs[key]}`);
        break; // Only show one key attribute
      }
    }
    
    return display;
  }

  /**
   * Generate action summary for memory
   */
  private generateActionSummary(actionType: string, targetId?: string, success: boolean = true, details?: any): string {
    const target = targetId ? ` on ${targetId}` : '';
    const status = success ? 'Successfully' : 'Failed to';
    
    switch (actionType) {
      case ACTION_TYPES.CLICK:
        return `${status} clicked${target}`;
      
      case ACTION_TYPES.TYPE:
        const text = details?.text ? ` "${details.text.slice(0, 20)}${details.text.length > 20 ? '...' : ''}"` : '';
        return `${status} typed${text}${target}`;
      
      case ACTION_TYPES.SELECT:
        const option = details?.option ? ` "${details.option}"` : '';
        return `${status} selected${option}${target}`;
      
      case ACTION_TYPES.SCROLL:
        const direction = details?.direction ? ` ${details.direction}` : '';
        return `${status} scrolled${direction}`;
      
      case ACTION_TYPES.NAVIGATE:
        const url = details?.url ? ` to ${this.formatUrl(details.url)}` : '';
        return `${status} navigated${url}`;
      
      case ACTION_TYPES.WAIT:
        return `${status} waited${target}`;
      
      case ACTION_TYPES.EXTRACT:
        const count = details?.count ? ` (${details.count} items)` : '';
        return `${status} extracted data${target}${count}`;
      
      default:
        return `${status} performed ${actionType}${target}`;
    }
  }

  /**
   * Get last generated snapshot
   */
  getLastSnapshot(): CompactSnapshot | null {
    return this.lastSnapshot;
  }

  /**
   * Get snapshot statistics
   */
  getStats() {
    if (!this.lastSnapshot) {
      return null;
    }
    
    return {
      elements: this.lastSnapshot.elements.length,
      sizeKB: this.lastSnapshot.sizeKB,
      compression: this.lastSnapshot.compressionLevel,
      frames: this.lastSnapshot.frames.length,
      accessibleFrames: this.lastSnapshot.frames.filter(f => f.accessible).length,
      generatedAt: new Date(this.lastSnapshot.timestamp).toISOString()
    };
  }
}

// Export types and utilities
export * from './types';
export { snapshotBuilder, snapshotReducer, memoryTracker };

// Export singleton instance
export const snapshotManager = new SnapshotManager();