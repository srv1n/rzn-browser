/**
 * Compact Snapshot System Types
 * Optimized for LLM consumption with 2-8KB size limits
 */

export interface CompactElement {
  /** Encoded ID for element selection (e.g., "btn_1", "inp_2") */
  encodedId: string;
  
  /** Element tag name */
  tag: string;
  
  /** Primary selector for element */
  selector: string;
  
  /** Text content (truncated) */
  text?: string;
  
  /** Element role from AX tree */
  role?: string;
  
  /** Element name from AX tree */
  name?: string;
  
  /** Distance from viewport center (for sorting) */
  viewportDistance: number;
  
  /** Key attributes */
  attrs?: Record<string, string>;
  
  /** Frame information for cross-origin awareness */
  frame?: string;
  
  /** Element type (button, input, link, etc) */
  type?: string;
  
  /** Interaction hints */
  actions?: string[];
}

export interface FrameContext {
  /** Frame identifier */
  id: string;
  
  /** Frame URL */
  url: string;
  
  /** Frame origin */
  origin: string;
  
  /** Whether frame is accessible */
  accessible: boolean;
  
  /** Number of elements in this frame */
  elementCount: number;
}

export interface CompactSnapshot {
  /** Current page URL */
  url: string;
  
  /** Page title (truncated) */
  title: string;
  
  /** Viewport dimensions */
  viewport: {
    width: number;
    height: number;
    scrollX: number;
    scrollY: number;
  };
  
  /** Interactive elements sorted by viewport proximity */
  elements: CompactElement[];
  
  /** Frame context information */
  frames: FrameContext[];
  
  /** Estimated size in KB */
  sizeKB: number;
  
  /** Generation timestamp */
  timestamp: number;
  
  /** Compression level used */
  compressionLevel: 'none' | 'light' | 'aggressive';
}

export interface ActionMemory {
  /** Action type performed */
  action: string;
  
  /** Target element encoded ID */
  targetId?: string;
  
  /** One-line summary of what happened */
  summary: string;
  
  /** Timestamp */
  timestamp: number;
  
  /** Success/failure status */
  success: boolean;
}

export interface SnapshotBuilder {
  /** Build complete snapshot from current DOM */
  buildSnapshot(): Promise<CompactSnapshot>;
  
  /** Build snapshot from accessibility tree */
  buildFromAccessibilityTree(): Promise<CompactSnapshot>;
  
  /** Add frame context */
  addFrameContext(frames: HTMLIFrameElement[]): FrameContext[];
  
  /** Get element with encoded ID */
  getElementById(encodedId: string): Element | null;
}

export interface SnapshotReducer {
  /** Reduce snapshot to target size */
  reduce(snapshot: CompactSnapshot, maxKB: number): CompactSnapshot;
  
  /** Estimate snapshot size */
  estimateSize(snapshot: CompactSnapshot): number;
  
  /** Apply compression strategies */
  compress(snapshot: CompactSnapshot, level: 'light' | 'aggressive'): CompactSnapshot;
}

export interface MemoryTracker {
  /** Add action to memory */
  addAction(action: ActionMemory): void;
  
  /** Get recent actions summary */
  getRecentSummary(count?: number): string;
  
  /** Clear old memories */
  cleanup(maxAge?: number): void;
  
  /** Get all memories for context */
  getAllMemories(): ActionMemory[];
}

// Configuration constants
export const SNAPSHOT_CONFIG = {
  /** Maximum snapshot size in KB */
  MAX_SIZE_KB: 8,
  
  /** Target size for aggressive compression */
  TARGET_SIZE_KB: 4,
  
  /** Maximum elements before compression kicks in */
  MAX_ELEMENTS: 50,
  
  /** Maximum text length per element */
  MAX_TEXT_LENGTH: 40,
  
  /** Maximum attributes per element */
  MAX_ATTRIBUTES: 3,
  
  /** Maximum memory items to keep */
  MAX_MEMORY_ITEMS: 10,
  
  /** Memory cleanup interval in ms */
  MEMORY_CLEANUP_INTERVAL: 5 * 60 * 1000, // 5 minutes
} as const;

// Element type mappings for compact representation
export const ELEMENT_TYPE_MAPPING = {
  'button': 'btn',
  'input': 'inp',
  'textarea': 'txt',
  'select': 'sel', 
  'option': 'opt',
  'a': 'lnk',
  'form': 'frm',
  'div': 'div',
  'span': 'spn',
  'img': 'img',
  'video': 'vid',
  'audio': 'aud',
  'iframe': 'ifr',
  'canvas': 'cvs'
} as const;

// Common action types for memory tracking
export const ACTION_TYPES = {
  CLICK: 'click',
  TYPE: 'type', 
  SELECT: 'select',
  SCROLL: 'scroll',
  NAVIGATE: 'navigate',
  WAIT: 'wait',
  EXTRACT: 'extract'
} as const;