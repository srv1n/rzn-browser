// AX-first DOM capture - Compact accessibility tree for LLM consumption
// Replaces raw HTML scraping with semantic role/name/state extraction

export interface AXNodeStub {
  role: string;
  name?: string;
  frameId: string;
  backendNodeId: number;
  bounds?: { x: number; y: number; w: number; h: number };
  actionable?: boolean;
  focused?: boolean;
  value?: string;
  checked?: boolean;
  selected?: boolean;
}

export interface AXSliceOptions {
  maxNodes?: number;
  viewportOnly?: boolean;
  frameIds?: string[];
}

/**
 * Build a compact AX slice for LLM consumption
 * Routes to background script for CDP Accessibility tree access
 */
export async function buildAXSlice(options: AXSliceOptions = {}): Promise<AXNodeStub[]> {
  const { maxNodes = 150, viewportOnly = true, frameIds } = options;
  
  try {
    const response = await chrome.runtime.sendMessage({
      cmd: 'rzn_fetch_ax_slice',
      maxNodes,
      viewportOnly,
      frameIds
    });
    
    if (!response.ok) {
      console.warn('Failed to fetch AX slice:', response.error);
      return [];
    }
    
    return response.nodes as AXNodeStub[];
  } catch (error) {
    console.error('AX slice request failed:', error);
    return [];
  }
}

/**
 * Convert AX nodes to compact prompt format
 */
export function toPromptFromAX(nodes: AXNodeStub[]): string {
  const lines: string[] = [];
  const viewport = { w: window.innerWidth, h: window.innerHeight };
  
  // Group by viewport position for spatial understanding
  const grouped = {
    top: nodes.filter(n => n.bounds && n.bounds.y < viewport.h * 0.33),
    middle: nodes.filter(n => n.bounds && n.bounds.y >= viewport.h * 0.33 && n.bounds.y < viewport.h * 0.67),
    bottom: nodes.filter(n => n.bounds && n.bounds.y >= viewport.h * 0.67),
    hidden: nodes.filter(n => !n.bounds)
  };
  
  for (const [position, positionNodes] of Object.entries(grouped)) {
    if (positionNodes.length === 0) continue;
    
    lines.push(`\\n=== ${position.toUpperCase()} OF PAGE ===`);
    
    for (const node of positionNodes.slice(0, 50)) { // Limit per section
      const id = `${node.frameId}:${node.backendNodeId}`;
      const box = node.bounds ? 
        ` @(${Math.round(node.bounds.x)},${Math.round(node.bounds.y)},${Math.round(node.bounds.w)}x${Math.round(node.bounds.h)})` : 
        '';
      
      // Build role/name string
      let roleStr = node.role;
      if (node.name) {
        roleStr += `: "${truncate(node.name, 40)}"`; 
      }
      
      // Add state indicators
      const states: string[] = [];
      if (node.actionable) states.push('actionable');
      if (node.focused) states.push('focused');
      if (node.checked !== undefined) states.push(node.checked ? 'checked' : 'unchecked');
      if (node.selected) states.push('selected');
      if (node.value) states.push(`value="${truncate(node.value, 20)}"`);
      
      const stateStr = states.length > 0 ? ` [${states.join(',')}]` : '';
      
      lines.push(`- ${roleStr} #${id}${box}${stateStr}`);
    }
  }
  
  return lines.join('\\n');
}

/**
 * Detect list extraction candidates from AX nodes
 */
export function detectListCandidates(nodes: AXNodeStub[]): Array<{
  role: string;
  count: number;
  sampleItems: AXNodeStub[];
  confidence: number;
}> {
  // Count nodes by role to find repeated patterns
  const roleCounts = new Map<string, AXNodeStub[]>();
  
  for (const node of nodes) {
    if (!node.actionable) continue; // Only actionable items are interesting
    
    const role = node.role.toLowerCase();
    if (!roleCounts.has(role)) {
      roleCounts.set(role, []);
    }
    roleCounts.get(role)!.push(node);
  }
  
  const candidates: Array<{
    role: string;
    count: number; 
    sampleItems: AXNodeStub[];
    confidence: number;
  }> = [];
  
  for (const [role, items] of roleCounts.entries()) {
    if (items.length >= 3) { // Need at least 3 similar items to be a list
      const confidence = calculateListConfidence(role, items);
      
      candidates.push({
        role,
        count: items.length,
        sampleItems: items.slice(0, 3), // First 3 as samples
        confidence
      });
    }
  }
  
  // Sort by confidence * count
  return candidates
    .sort((a, b) => (b.confidence * b.count) - (a.confidence * a.count))
    .slice(0, 5); // Top 5 candidates
}

function calculateListConfidence(role: string, items: AXNodeStub[]): number {
  let confidence = 0.5; // Base confidence
  
  // Higher confidence for list-like roles
  const listRoles = ['listitem', 'link', 'button', 'article', 'option'];
  if (listRoles.includes(role)) {
    confidence += 0.3;
  }
  
  // Boost for consistent naming patterns
  const names = items.map(item => item.name).filter(Boolean);
  if (names.length > 0) {
    const avgLength = names.reduce((sum, name) => sum + (name?.length || 0), 0) / names.length;
    if (avgLength > 10 && avgLength < 100) { // Good length for content
      confidence += 0.2;
    }
  }
  
  // Boost for spatial clustering (items close together)
  const withBounds = items.filter(item => item.bounds);
  if (withBounds.length >= 2) {
    const yPositions = withBounds.map(item => item.bounds!.y).sort((a, b) => a - b);
    const avgSpacing = (yPositions[yPositions.length - 1] - yPositions[0]) / (yPositions.length - 1);
    if (avgSpacing > 20 && avgSpacing < 200) { // Reasonable list spacing
      confidence += 0.2;
    }
  }
  
  return Math.min(confidence, 1.0);
}

function truncate(s: string, n: number): string {
  return s.length > n ? s.slice(0, n - 1) + '…' : s;
}

/**
 * Cache list candidates by domain+path for reuse
 */
export class ListCandidateCache {
  private cache = new Map<string, any>();
  
  private getCacheKey(): string {
    const url = new URL(window.location.href);
    return `${url.hostname}${url.pathname}`;
  }
  
  getCached(): any {
    return this.cache.get(this.getCacheKey());
  }
  
  setCached(candidates: any): void {
    this.cache.set(this.getCacheKey(), candidates);
  }
  
  clear(): void {
    this.cache.clear();
  }
}

export const listCandidateCache = new ListCandidateCache();