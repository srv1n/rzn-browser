// CDP Frame Router - CRITICAL infrastructure for cross-origin iframe support
// This is the new clean CDP implementation based on Sam's feedback

// === Core Frame Routing Infrastructure ===
export { frameRouter, type FrameInfo, type TargetInfo } from './frameRouter';
export { cdpClient, type CDPCommand, type CDPResult, type CDPTarget } from './cdpClient';
export { accessibilityService, type AccessibleElement } from './accessibility';

// === Complete CDP Type System ===
export * from './types';

// === Legacy CDP module wrappers removed in favor of Frame Router ===
export { cdp, CDP } from './cdpHelper';
export { 
  buildUnifiedSnapshot, 
  resolveElement,
  type UnifiedSnapshot,
  type NodeSummary,
  type EncodedId 
} from './uft';
export { 
  executeAction,
  type ActionParams,
  type ActionResult 
} from './inputLadder';
export {
  reduceForLLM,
  reduceDOMContent,
  reduceSnapshot,
  type ReducedContext,
  type ElementInfo
} from './domReducer';
export {
  ExecutionTier,
  ExecutionStrategy,
  strategy,
  DEFAULT_STRATEGY,
  type StrategyConfig
} from './executionStrategy';
export { cdpIntegration } from './integration';

// === Primary API - Use this for new code ===

/**
 * Attach CDP to tab with OOPIF support
 * KEY: Uses Target.setAutoAttach with flatten=true for cross-origin frames
 */
export async function attachToTab(tabId: number): Promise<void> {
  return frameRouter.attachToTab(tabId);
}

/**
 * Detach CDP from tab and clean up
 */
export async function detachFromTab(tabId: number): Promise<void> {
  return frameRouter.detachFromTab(tabId);
}

/**
 * Get sessionId for routing commands to specific frame
 * This is the core routing functionality
 */
export function routeForFrame(frameId?: string): { sessionId?: string } {
  return frameRouter.routeForFrame(frameId);
}

/**
 * Get complete accessibility snapshot across all frames
 * Preferred method for extracting semantic element information
 */
export async function getAccessibilitySnapshot(tabId: number) {
  return accessibilityService.getAccessibilitySnapshot(tabId);
}

/**
 * Get all interactive elements (buttons, links, inputs, etc.)
 */
export async function getInteractiveElements(tabId: number) {
  return accessibilityService.getInteractiveElements(tabId);
}

/**
 * Send CDP command with automatic frame routing
 */
export async function sendCommand<T = any>(
  tabId: number,
  method: string,
  params?: any,
  options?: { frameId?: string; sessionId?: string; timeout?: number }
): Promise<T> {
  return cdpClient.sendCommand<T>({ tabId }, method, params, options);
}

// === Utility Functions ===

/**
 * Check if CDP is attached to tab
 */
export function isAttachedToTab(tabId: number): boolean {
  return frameRouter.isAttachedToTab(tabId);
}

/**
 * Get all attached tabs
 */
export function getAttachedTabs(): number[] {
  return frameRouter.getAttachedTabs();
}

/**
 * Get frame tree with routing information
 */
export async function getFrameTree(tabId: number) {
  return frameRouter.getFrameTree(tabId);
}

// Back-compat helpers removed; use primary API above.
