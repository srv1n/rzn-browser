// LRU Cache for resolved elements
// Maintains stable element references with automatic cleanup

import { ResolvedElement, EncodedId } from '../types/targets';

interface CacheNode {
  key: EncodedId;
  value: ResolvedElement;
  prev?: CacheNode;
  next?: CacheNode;
  accessCount: number;
  lastAccessed: number;
}

interface CacheStats {
  size: number;
  hitRate: number;
  totalRequests: number;
  hits: number;
  misses: number;
}

export class ElementCache {
  private maxSize: number;
  private cache: Map<EncodedId, CacheNode>;
  private head?: CacheNode;
  private tail?: CacheNode;
  private stats: CacheStats;
  private cleanupInterval: number | null = null;

  constructor(maxSize: number = 1000, cleanupIntervalMs: number = 60000) {
    this.maxSize = maxSize;
    this.cache = new Map();
    this.stats = {
      size: 0,
      hitRate: 0,
      totalRequests: 0,
      hits: 0,
      misses: 0
    };

    // Start automatic cleanup
    this.startCleanup(cleanupIntervalMs);
  }

  /**
   * Get element from cache
   */
  get(key: EncodedId): ResolvedElement | null {
    this.stats.totalRequests++;
    
    const node = this.cache.get(key);
    if (!node) {
      this.stats.misses++;
      this.updateHitRate();
      return null;
    }

    // Check if element is still fresh
    if (this.isExpired(node.value)) {
      this.delete(key);
      this.stats.misses++;
      this.updateHitRate();
      return null;
    }

    // Move to head (most recently used)
    this.moveToHead(node);
    node.accessCount++;
    node.lastAccessed = Date.now();
    
    this.stats.hits++;
    this.updateHitRate();
    
    return node.value;
  }

  /**
   * Set element in cache
   */
  set(key: EncodedId, value: ResolvedElement): void {
    const existingNode = this.cache.get(key);
    
    if (existingNode) {
      // Update existing node
      existingNode.value = value;
      existingNode.lastAccessed = Date.now();
      existingNode.accessCount++;
      this.moveToHead(existingNode);
      return;
    }

    // Create new node
    const newNode: CacheNode = {
      key,
      value,
      accessCount: 1,
      lastAccessed: Date.now()
    };

    // Add to cache
    this.cache.set(key, newNode);
    this.addToHead(newNode);
    this.stats.size++;

    // Check if we need to evict
    if (this.cache.size > this.maxSize) {
      const evicted = this.removeTail();
      if (evicted) {
        this.cache.delete(evicted.key);
        this.stats.size--;
      }
    }
  }

  /**
   * Delete element from cache
   */
  delete(key: EncodedId): boolean {
    const node = this.cache.get(key);
    if (!node) {
      return false;
    }

    this.removeNode(node);
    this.cache.delete(key);
    this.stats.size--;
    return true;
  }

  /**
   * Check if key exists in cache
   */
  has(key: EncodedId): boolean {
    const node = this.cache.get(key);
    return node !== undefined && !this.isExpired(node.value);
  }

  /**
   * Clear all cache entries
   */
  clear(): void {
    this.cache.clear();
    this.head = undefined;
    this.tail = undefined;
    this.stats.size = 0;
  }

  /**
   * Get cache statistics
   */
  getStats(): CacheStats {
    return { ...this.stats };
  }

  /**
   * Get all cache keys
   */
  keys(): EncodedId[] {
    return Array.from(this.cache.keys());
  }

  /**
   * Get cache size
   */
  size(): number {
    return this.cache.size;
  }

  /**
   * Clean up expired entries
   */
  cleanup(): number {
    let removed = 0;
    const now = Date.now();
    const expiredThreshold = 5 * 60 * 1000; // 5 minutes

    for (const [key, node] of this.cache) {
      // Remove if expired or too old
      if (this.isExpired(node.value) || 
          (now - node.lastAccessed) > expiredThreshold) {
        this.delete(key);
        removed++;
      }
    }

    console.log(`[ElementCache] Cleaned up ${removed} expired entries`);
    return removed;
  }

  /**
   * Get cache performance metrics
   */
  getPerformanceMetrics(): {
    hitRate: number;
    averageAccessCount: number;
    oldestEntry: number;
    newestEntry: number;
  } {
    if (this.cache.size === 0) {
      return {
        hitRate: 0,
        averageAccessCount: 0,
        oldestEntry: 0,
        newestEntry: 0
      };
    }

    let totalAccessCount = 0;
    let oldestTime = Date.now();
    let newestTime = 0;

    for (const node of this.cache.values()) {
      totalAccessCount += node.accessCount;
      oldestTime = Math.min(oldestTime, node.value.resolved_at);
      newestTime = Math.max(newestTime, node.value.resolved_at);
    }

    return {
      hitRate: this.stats.hitRate,
      averageAccessCount: totalAccessCount / this.cache.size,
      oldestEntry: oldestTime,
      newestEntry: newestTime
    };
  }

  /**
   * Export cache contents for debugging
   */
  exportContents(): Array<{
    key: EncodedId;
    element: ResolvedElement;
    accessCount: number;
    lastAccessed: number;
    age: number;
  }> {
    const now = Date.now();
    return Array.from(this.cache.entries()).map(([key, node]) => ({
      key,
      element: node.value,
      accessCount: node.accessCount,
      lastAccessed: node.lastAccessed,
      age: now - node.value.resolved_at
    }));
  }

  // Private methods

  private addToHead(node: CacheNode): void {
    node.prev = undefined;
    node.next = this.head;

    if (this.head) {
      this.head.prev = node;
    }

    this.head = node;

    if (!this.tail) {
      this.tail = node;
    }
  }

  private removeNode(node: CacheNode): void {
    if (node.prev) {
      node.prev.next = node.next;
    } else {
      this.head = node.next;
    }

    if (node.next) {
      node.next.prev = node.prev;
    } else {
      this.tail = node.prev;
    }
  }

  private moveToHead(node: CacheNode): void {
    this.removeNode(node);
    this.addToHead(node);
  }

  private removeTail(): CacheNode | undefined {
    const last = this.tail;
    if (last) {
      this.removeNode(last);
    }
    return last;
  }

  private isExpired(element: ResolvedElement): boolean {
    const maxAge = element.is_cross_origin ? 30000 : 60000; // 30s cross-origin, 60s same-origin
    return Date.now() - element.resolved_at > maxAge;
  }

  private updateHitRate(): void {
    this.stats.hitRate = this.stats.totalRequests > 0 
      ? this.stats.hits / this.stats.totalRequests 
      : 0;
  }

  private startCleanup(intervalMs: number): void {
    this.cleanupInterval = window.setInterval(() => {
      this.cleanup();
    }, intervalMs);
  }

  /**
   * Stop automatic cleanup
   */
  destroy(): void {
    if (this.cleanupInterval !== null) {
      clearInterval(this.cleanupInterval);
      this.cleanupInterval = null;
    }
    this.clear();
  }
}