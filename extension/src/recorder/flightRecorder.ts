// Flight Recorder - Debug trace capture for 1-click repro export
// Records screenshots, metrics, logs, and DOM snapshots for debugging

import { getFlags } from '../config/flags';

export interface RecorderEvent {
  timestamp: number;
  type: 'action' | 'error' | 'metric' | 'dom_change' | 'screenshot' | 'navigation';
  data: any;
  url: string;
  frame_id?: string;
}

export interface RecorderMetrics {
  action_count: number;
  success_rate: number;
  avg_action_time: number;
  total_time: number;
  cdp_escalations: number;
  errors: string[];
}

export interface FlightRecorderState {
  session_id: string;
  start_time: number;
  events: RecorderEvent[];
  metrics: RecorderMetrics;
  screenshots: Blob[];
  dom_snapshots: string[];
  max_events: number;
  max_screenshots: number;
}

class FlightRecorder {
  private state: FlightRecorderState;
  private isRecording: boolean = false;
  private screenshotInterval?: number;

  constructor() {
    this.state = this.createNewSession();
  }

  private createNewSession(): FlightRecorderState {
    return {
      session_id: this.generateSessionId(),
      start_time: Date.now(),
      events: [],
      metrics: {
        action_count: 0,
        success_rate: 0,
        avg_action_time: 0,
        total_time: 0,
        cdp_escalations: 0,
        errors: []
      },
      screenshots: [],
      dom_snapshots: [],
      max_events: 500,
      max_screenshots: 20
    };
  }

  private generateSessionId(): string {
    return `rzn_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
  }

  /**
   * Start recording session
   */
  async startRecording(): Promise<void> {
    const flags = await getFlags(window.location.hostname);
    if (!flags.flightRecorder) {
      console.log('[FlightRecorder] Disabled by flags');
      return;
    }

    this.isRecording = true;
    this.state = this.createNewSession();
    
    console.log(`[FlightRecorder] Started session ${this.state.session_id}`);
    
    // Record initial state
    await this.recordEvent({
      timestamp: Date.now(),
      type: 'navigation',
      data: { 
        url: window.location.href,
        title: document.title,
        user_agent: navigator.userAgent
      },
      url: window.location.href
    });

    // Auto-screenshot every 5 seconds during active recording
    this.screenshotInterval = window.setInterval(async () => {
      if (this.isRecording) {
        await this.captureScreenshot();
      }
    }, 5000);
  }

  /**
   * Stop recording session
   */
  stopRecording(): void {
    this.isRecording = false;
    if (this.screenshotInterval) {
      clearInterval(this.screenshotInterval);
      this.screenshotInterval = undefined;
    }
    
    this.state.metrics.total_time = Date.now() - this.state.start_time;
    console.log(`[FlightRecorder] Stopped session ${this.state.session_id} after ${this.state.metrics.total_time}ms`);
  }

  /**
   * Record an event during the session
   */
  async recordEvent(event: Omit<RecorderEvent, 'timestamp'> & { timestamp?: number }): Promise<void> {
    if (!this.isRecording) return;

    const recordedEvent: RecorderEvent = {
      timestamp: event.timestamp || Date.now(),
      type: event.type,
      data: event.data,
      url: event.url,
      frame_id: event.frame_id
    };

    this.state.events.push(recordedEvent);
    
    // Update metrics
    if (event.type === 'action') {
      this.state.metrics.action_count++;
      if (event.data.success === false) {
        this.state.metrics.errors.push(event.data.error || 'Unknown action failure');
      }
    } else if (event.type === 'error') {
      this.state.metrics.errors.push(event.data.message || 'Unknown error');
    } else if (event.type === 'metric' && event.data.cdp_escalation) {
      this.state.metrics.cdp_escalations++;
    }

    // Trim old events to stay within limits
    if (this.state.events.length > this.state.max_events) {
      this.state.events = this.state.events.slice(-this.state.max_events);
    }

    console.log(`[FlightRecorder] Recorded ${event.type} event`);
  }

  /**
   * Record action execution
   */
  async recordAction(actionType: string, success: boolean, duration: number, error?: string, metadata?: any): Promise<void> {
    await this.recordEvent({
      type: 'action',
      data: {
        action_type: actionType,
        success,
        duration_ms: duration,
        error,
        metadata
      },
      url: window.location.href
    });
  }

  /**
   * Record error
   */
  async recordError(error: Error | string, context?: any): Promise<void> {
    await this.recordEvent({
      type: 'error',
      data: {
        message: typeof error === 'string' ? error : error.message,
        stack: error instanceof Error ? error.stack : undefined,
        context
      },
      url: window.location.href
    });
  }

  private async requestScreenshotFromBackground(): Promise<string> {
    const runtime = (globalThis as any).chrome?.runtime as
      | {
          sendMessage?: (
            message: unknown,
            callback?: (response?: { success?: boolean; dataUrl?: string; error?: string }) => void
          ) => void;
          lastError?: { message?: string };
        }
      | undefined;
    if (!runtime?.sendMessage) {
      throw new Error('Extension runtime unavailable for screenshot capture');
    }

    return await new Promise<string>((resolve, reject) => {
      runtime.sendMessage(
        { cmd: 'take_screenshot', format: 'png' },
        (response?: { success?: boolean; dataUrl?: string; error?: string }) => {
          if (runtime.lastError) {
            reject(new Error(runtime.lastError.message));
            return;
          }

          if (!response?.success || typeof response.dataUrl !== 'string') {
            reject(new Error(response?.error || 'Screenshot capture failed'));
            return;
          }

          resolve(response.dataUrl);
        }
      );
    });
  }

  private async dataUrlToBlob(dataUrl: string): Promise<Blob> {
    const response = await fetch(dataUrl);
    if (!response.ok) {
      throw new Error(`Invalid screenshot data: ${response.status}`);
    }
    return await response.blob();
  }

  /**
   * Capture screenshot
   */
  private async captureScreenshot(): Promise<void> {
    try {
      const screenshotDataUrl = await this.requestScreenshotFromBackground();
      const screenshotBlob = await this.dataUrlToBlob(screenshotDataUrl);

      this.state.screenshots.push(screenshotBlob);

      if (this.state.screenshots.length > this.state.max_screenshots) {
        this.state.screenshots = this.state.screenshots.slice(-this.state.max_screenshots);
      }
    } catch (error) {
      await this.recordEvent({
        type: 'screenshot',
        data: {
          width: window.innerWidth,
          height: window.innerHeight,
          error: error instanceof Error ? error.message : String(error)
        },
        url: window.location.href
      });
      console.warn('[FlightRecorder] Screenshot capture failed:', error);
    }
  }

  /**
   * Capture DOM snapshot
   */
  async captureDOMSnapshot(): Promise<void> {
    if (!this.isRecording) return;

    try {
      // Create simplified DOM representation
      const snapshot = this.createDOMSnapshot(document.documentElement);
      this.state.dom_snapshots.push(JSON.stringify(snapshot));
      
      // Trim old snapshots (keep last 5)
      if (this.state.dom_snapshots.length > 5) {
        this.state.dom_snapshots = this.state.dom_snapshots.slice(-5);
      }

      await this.recordEvent({
        type: 'dom_change',
        data: { snapshot_index: this.state.dom_snapshots.length - 1 },
        url: window.location.href
      });
    } catch (error) {
      console.warn('[FlightRecorder] DOM snapshot failed:', error);
    }
  }

  private createDOMSnapshot(element: Element, depth: number = 0): any {
    if (depth > 5) return { truncated: true }; // Limit depth

    return {
      tag: element.tagName.toLowerCase(),
      attributes: Array.from(element.attributes).reduce((attrs, attr) => {
        attrs[attr.name] = attr.value;
        return attrs;
      }, {} as Record<string, string>),
      text: element.childNodes.length === 1 && element.childNodes[0].nodeType === 3 
        ? element.textContent?.trim().slice(0, 100) 
        : undefined,
      children: Array.from(element.children)
        .slice(0, 10) // Limit children
        .map(child => this.createDOMSnapshot(child, depth + 1))
    };
  }

  /**
   * Get current recorder state
   */
  getState(): FlightRecorderState {
    // Calculate success rate
    const successfulActions = this.state.events.filter(e => 
      e.type === 'action' && e.data.success === true
    ).length;
    
    this.state.metrics.success_rate = this.state.metrics.action_count > 0 
      ? successfulActions / this.state.metrics.action_count 
      : 0;

    // Calculate average action time
    const actionTimes = this.state.events
      .filter(e => e.type === 'action' && e.data.duration_ms)
      .map(e => e.data.duration_ms);
    
    this.state.metrics.avg_action_time = actionTimes.length > 0
      ? actionTimes.reduce((a, b) => a + b, 0) / actionTimes.length
      : 0;

    return { ...this.state };
  }

  /**
   * Export session data as ZIP for repro
   */
  async exportSession(): Promise<Blob> {
    const sessionData = this.getState();
    
    // Create export package
    const exportData = {
      session_id: sessionData.session_id,
      start_time: new Date(sessionData.start_time).toISOString(),
      total_time: sessionData.metrics.total_time,
      url: window.location.href,
      user_agent: navigator.userAgent,
      
      // Events log
      events: sessionData.events.map(event => ({
        timestamp: new Date(event.timestamp).toISOString(),
        type: event.type,
        data: event.data,
        url: event.url
      })),
      
      // Metrics summary
      metrics: sessionData.metrics,
      
      // DOM snapshots
      dom_snapshots: sessionData.dom_snapshots.map((snapshot, index) => ({
        index,
        data: JSON.parse(snapshot)
      })),
      
      // Screenshot count (actual images handled separately)
      screenshot_count: sessionData.screenshots.length
    };

    // Create JSON blob
    const jsonBlob = new Blob([JSON.stringify(exportData, null, 2)], {
      type: 'application/json'
    });

    return jsonBlob;
  }

  /**
   * Clear current session
   */
  clearSession(): void {
    this.stopRecording();
    this.state = this.createNewSession();
    console.log('[FlightRecorder] Session cleared');
  }
}

// Export singleton instance
export const flightRecorder = new FlightRecorder();
