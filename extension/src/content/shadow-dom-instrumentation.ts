// Instrument Element.attachShadow to capture closed shadow roots
// This must run before any page scripts

export function instrumentShadowDOM() {
  // Check if already instrumented
  if ((window as any).__rznShadowInstrumented) {
    return;
  }

  const debug = Boolean((window as any).__rznShadowDebug);

  // Store references to closed shadow roots
  const shadowRootRegistry = new WeakMap<Element, ShadowRoot>();

  // Also keep a best-effort ordered list of all shadow roots created on the page.
  // This enables shadow-aware querying from isolated-world content scripts without
  // scanning the entire DOM tree repeatedly.
  const shadowRoots: ShadowRoot[] = Array.isArray((window as any).__rznShadowRoots)
    ? (window as any).__rznShadowRoots
    : [];
  (window as any).__rznShadowRoots = shadowRoots;
  
  // Save original attachShadow
  const originalAttachShadow = Element.prototype.attachShadow;
  
  // Monkey-patch attachShadow
  Element.prototype.attachShadow = function(init: ShadowRootInit) {
    const shadow = originalAttachShadow.call(this, init);
    
    // Store reference even for closed shadows
    shadowRootRegistry.set(this, shadow);

    // Track shadow roots for fast iteration.
    if (!shadowRoots.includes(shadow)) {
      shadowRoots.push(shadow);
    }

    if (debug) {
      console.debug('[RZN] Shadow DOM attached:', {
        element: this.tagName,
        mode: init.mode,
        closed: init.mode === 'closed'
      });
    }
    
    return shadow;
  };
  
  // Expose getter for our extension
  (window as any).__rznGetShadowRoot = (element: Element) => {
    return shadowRootRegistry.get(element);
  };

  // Mark as instrumented
  (window as any).__rznShadowInstrumented = true;
  
  if (debug) {
    console.debug('[RZN] Shadow DOM instrumentation installed');
  }
}

// Auto-run on script load if in page context
if (typeof window !== 'undefined' && !window.chrome?.runtime) {
  instrumentShadowDOM();
}
