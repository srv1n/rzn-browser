// Tiered Execution Strategy - CDP is optional and only used as fallback
// Minimizes bot detection by defaulting to pure JS

export enum ExecutionTier {
  // Pure JavaScript via content scripts (default, most stealthy)
  PureJS = 'pure-js',
  
  // Enhanced JS with more sophisticated event simulation
  EnhancedJS = 'enhanced-js',
  
  // CDP only when absolutely necessary (cross-origin iframes, trusted events)
  CDPFallback = 'cdp-fallback',
  
  // Full CDP mode (only for debugging or specific requirements)
  CDPAlways = 'cdp-always'
}

export interface StrategyConfig {
  // Global execution tier
  tier: ExecutionTier;
  
  // Feature flags
  features: {
    // Use CDP for cross-origin iframe access
    cdpForCrossOriginFrames: boolean;
    
    // Use CDP for trusted event requirements
    cdpForTrustedEvents: boolean;
    
    // Use CDP for accurate screenshots
    cdpForScreenshots: boolean;
    
    // Enable CDP watchdogs (crash detection, etc)
    cdpWatchdogs: boolean;
  };
  
  // Per-site overrides
  siteOverrides: Map<string, {
    tier?: ExecutionTier;
    features?: Partial<StrategyConfig['features']>;
  }>;
  
  // Escalation settings
  escalation: {
    // Auto-escalate to next tier on failure
    autoEscalate: boolean;
    
    // Max retries before escalation
    maxRetriesPerTier: number;
    
    // Track failed selectors for smart escalation
    trackFailures: boolean;
  };
  
  // Stealth settings
  stealth: {
    // Random delays between actions (ms)
    actionDelayRange: [number, number];
    
    // Simulate human-like mouse movements
    humanizeMousePaths: boolean;
    
    // Randomize typing speed
    humanizeTyping: boolean;
    
    // Never attach CDP unless explicitly needed
    cdpOnDemandOnly: boolean;
  };
}

// Default configuration - maximum stealth
export const DEFAULT_STRATEGY: StrategyConfig = {
  tier: ExecutionTier.PureJS,
  
  features: {
    cdpForCrossOriginFrames: false, // Only enable when needed
    cdpForTrustedEvents: false,      // Try JS first
    cdpForScreenshots: false,        // Use DOM capture by default
    cdpWatchdogs: false              // No CDP by default
  },
  
  // No built-in site overrides in runtime. If a caller needs overrides,
  // set them explicitly via configureStrategy(...).
  siteOverrides: new Map(),
  
  escalation: {
    autoEscalate: true,
    maxRetriesPerTier: 2,
    trackFailures: true
  },
  
  stealth: {
    actionDelayRange: [50, 200],
    humanizeMousePaths: true,
    humanizeTyping: true,
    cdpOnDemandOnly: true
  }
};

// Runtime strategy manager
export class ExecutionStrategy {
  private config: StrategyConfig;
  private failureCache = new Map<string, number>();
  
  constructor(config: Partial<StrategyConfig> = {}) {
    this.config = { ...DEFAULT_STRATEGY, ...config };
  }
  
  // Get effective tier for a URL
  getTierForUrl(url: string): ExecutionTier {
    const hostname = new URL(url).hostname;
    
    // Check site overrides
    for (const [pattern, override] of this.config.siteOverrides) {
      if (this.matchesPattern(hostname, pattern)) {
        return override.tier || this.config.tier;
      }
    }
    
    return this.config.tier;
  }
  
  // Check if CDP should be used for a specific feature
  shouldUseCDP(feature: keyof StrategyConfig['features'], url?: string): boolean {
    // Never use CDP in pure-js mode unless explicitly overridden
    if (this.config.tier === ExecutionTier.PureJS) {
      if (!url) return false;
      
      // Check site-specific overrides
      const hostname = url ? new URL(url).hostname : '';
      for (const [pattern, override] of this.config.siteOverrides) {
        if (this.matchesPattern(hostname, pattern)) {
          return override.features?.[feature] ?? this.config.features[feature];
        }
      }
      
      return this.config.features[feature];
    }
    
    // CDP tiers
    if (this.config.tier === ExecutionTier.CDPAlways) return true;
    if (this.config.tier === ExecutionTier.CDPFallback) {
      return this.config.features[feature];
    }
    
    return false;
  }
  
  // Record action failure for smart escalation
  recordFailure(selector: string, tier: ExecutionTier) {
    if (!this.config.escalation.trackFailures) return;
    
    const key = `${tier}:${selector}`;
    const count = (this.failureCache.get(key) || 0) + 1;
    this.failureCache.set(key, count);
    
    // Auto-escalate if threshold reached
    if (count >= this.config.escalation.maxRetriesPerTier) {
      this.escalate();
    }
  }
  
  // Escalate to next tier
  escalate() {
    if (!this.config.escalation.autoEscalate) return;
    
    const tiers = [
      ExecutionTier.PureJS,
      ExecutionTier.EnhancedJS,
      ExecutionTier.CDPFallback,
      ExecutionTier.CDPAlways
    ];
    
    const currentIndex = tiers.indexOf(this.config.tier);
    if (currentIndex < tiers.length - 1) {
      const newTier = tiers[currentIndex + 1];
      console.log(`[Strategy] Escalating from ${this.config.tier} to ${newTier}`);
      this.config.tier = newTier;
    }
  }
  
  // De-escalate back to safer tier
  deescalate() {
    const tiers = [
      ExecutionTier.PureJS,
      ExecutionTier.EnhancedJS,
      ExecutionTier.CDPFallback,
      ExecutionTier.CDPAlways
    ];
    
    const currentIndex = tiers.indexOf(this.config.tier);
    if (currentIndex > 0) {
      const newTier = tiers[currentIndex - 1];
      console.log(`[Strategy] De-escalating from ${this.config.tier} to ${newTier}`);
      this.config.tier = newTier;
    }
  }
  
  // Get random delay for human-like behavior
  getActionDelay(): number {
    if (!this.config.stealth.humanizeMousePaths) return 0;
    
    const [min, max] = this.config.stealth.actionDelayRange;
    return Math.floor(Math.random() * (max - min + 1)) + min;
  }
  
  // Update configuration
  updateConfig(updates: Partial<StrategyConfig>) {
    this.config = { ...this.config, ...updates };
  }
  
  private matchesPattern(hostname: string, pattern: string): boolean {
    const regex = pattern
      .replace(/\./g, '\\.')
      .replace(/\*/g, '.*');
    return new RegExp(`^${regex}$`).test(hostname);
  }
  
  // Export current config for persistence
  exportConfig(): StrategyConfig {
    return { ...this.config };
  }
}

// Global instance
export const strategy = new ExecutionStrategy();
