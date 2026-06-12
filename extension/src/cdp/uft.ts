// Unified Frame Tree - Solves iframe traversal similar to the public reference implementation
import { cdp } from './cdpHelper';

export type EncodedId = `${number}:${number}`; // frameOrdinal:backendNodeId

export interface NodeSummary {
  id: EncodedId;
  frameId: string;
  frameOrdinal: number;
  backendNodeId: number;
  role?: string;
  name?: string;
  url?: string;
  visible?: boolean;
  clickable?: boolean;
  box?: { x: number; y: number; w: number; h: number };
  deepXPath?: string;
  selector?: string;
  text?: string;
  value?: string;
  attributes?: Record<string, string>;
}

export interface UnifiedSnapshot {
  frames: Array<{ 
    frameId: string; 
    parentId?: string; 
    url?: string; 
    ordinal: number;
    sessionId?: string;
  }>;
  nodes: NodeSummary[];
  createdAt: number;
  viewportSize?: { width: number; height: number };
}

function makeId(frameOrdinal: number, backendNodeId: number): EncodedId {
  return `${frameOrdinal}:${backendNodeId}`;
}

// Build complete snapshot across all frames including OOPIFs
export async function buildUnifiedSnapshot(tabId: number): Promise<UnifiedSnapshot> {
  return cdp.with(tabId, async () => {
    console.log('[UFT] Building unified snapshot');
    
    // Ensure domains
    await cdp.enable(tabId, ['Page', 'DOM', 'Accessibility', 'CSS']);

    // Get frame tree
    const frameTree = await cdp.send<{ frameTree: any }>(tabId, 'Page.getFrameTree');
    const frames: UnifiedSnapshot['frames'] = [];
    const ordinals = new Map<string, number>();

    let ordinal = 0;
    function dfs(node: any, parentId?: string) {
      const frameId = node.frame.id as string;
      frames.push({ 
        frameId, 
        parentId, 
        url: node.frame.url || '', 
        ordinal 
      });
      ordinals.set(frameId, ordinal);
      ordinal++;
      if (node.childFrames) {
        for (const ch of node.childFrames) dfs(ch, frameId);
      }
    }
    dfs(frameTree.frameTree);

    const nodes: NodeSummary[] = [];

    // Process each frame
    for (const f of frames) {
      try {
        // Get AX tree for semantic info
        const ax = await cdp.send<any>(
          tabId, 
          'Accessibility.getFullAXTree', 
          {}, 
          cdp.routeForFrame(f.frameId)
        );

        // Build backendNodeId -> semantic info map
        const axMap = new Map<number, { 
          role?: string; 
          name?: string; 
          description?: string;
          value?: any;
        }>();
        
        if (ax?.nodes) {
          for (const n of ax.nodes) {
            if (typeof n.backendDOMNodeId === 'number') {
              axMap.set(n.backendDOMNodeId, {
                role: n.role?.value,
                name: n.name?.value,
                description: n.description?.value,
                value: n.value?.value
              });
            }
          }
        }

        // Get DOM for structure
        const doc = await cdp.send<any>(
          tabId, 
          'DOM.getDocument', 
          { depth: -1, pierce: true }, 
          cdp.routeForFrame(f.frameId)
        );

        // Find interactable elements
        const interactableSelectors = [
          'a', 'button', 'input', 'textarea', 'select',
          '[role=button]', '[role=link]', '[role=textbox]',
          '[onclick]', '[contenteditable]', 'summary'
        ].join(',');

        const result = await cdp.send<any>(
          tabId,
          'DOM.querySelectorAll',
          { nodeId: doc.root.nodeId, selector: interactableSelectors },
          cdp.routeForFrame(f.frameId)
        );

        const interactables = result.nodeIds || [];

        // Process each interactable node
        for (const nodeId of interactables) {
          try {
            // Get node details
            const desc = await cdp.send<any>(
              tabId,
              'DOM.describeNode',
              { nodeId, depth: 0 },
              cdp.routeForFrame(f.frameId)
            );
            
            const backendNodeId = desc.node.backendNodeId as number;
            const nodeName = desc.node.nodeName?.toLowerCase();
            const attrs = desc.node.attributes || [];
            
            // Parse attributes
            const attributes: Record<string, string> = {};
            for (let i = 0; i < attrs.length; i += 2) {
              attributes[attrs[i]] = attrs[i + 1];
            }

            // Get box model for positioning
            let box: NodeSummary['box'] | undefined;
            try {
              const boxModel = await cdp.send<any>(
                tabId,
                'DOM.getBoxModel',
                { nodeId },
                cdp.routeForFrame(f.frameId)
              );
              if (boxModel?.model) {
                const [x1, y1, x2, y2] = bboxFromQuads(
                  boxModel.model.border || boxModel.model.content
                );
                box = { x: x1, y: y1, w: x2 - x1, h: y2 - y1 };
              }
            } catch {}

            // Get text content
            let text: string | undefined;
            try {
              const outer = await cdp.send<any>(
                tabId,
                'DOM.getOuterHTML',
                { nodeId },
                cdp.routeForFrame(f.frameId)
              );
              // Simple text extraction
              text = outer.outerHTML
                ?.replace(/<[^>]*>/g, ' ')
                .replace(/\s+/g, ' ')
                .trim()
                .slice(0, 100);
            } catch {}

            // Merge with AX info
            const axInfo = axMap.get(backendNodeId) || {};

            // Determine if clickable
            const clickable = 
              nodeName === 'button' || 
              nodeName === 'a' || 
              axInfo.role === 'button' ||
              axInfo.role === 'link' ||
              !!attributes.onclick;

            nodes.push({
              id: makeId(f.ordinal, backendNodeId),
              frameId: f.frameId,
              frameOrdinal: f.ordinal,
              backendNodeId,
              role: axInfo.role || nodeName,
              name: axInfo.name || attributes['aria-label'] || attributes.alt || attributes.title,
              url: attributes.href,
              visible: box ? box.w > 0 && box.h > 0 : undefined,
              clickable,
              box,
              selector: buildSelector(nodeName, attributes),
              text: text || axInfo.value?.toString(),
              value: attributes.value || axInfo.value,
              attributes: Object.keys(attributes).length > 0 ? attributes : undefined
            });
          } catch (e) {
            console.warn(`[UFT] Failed to process node ${nodeId}:`, e);
          }
        }
      } catch (e) {
        console.warn(`[UFT] Failed to process frame ${f.frameId}:`, e);
      }
    }

    // Get viewport size
    let viewportSize: UnifiedSnapshot['viewportSize'];
    try {
      const metrics = await cdp.send<any>(tabId, 'Page.getLayoutMetrics');
      viewportSize = {
        width: metrics.visualViewport.clientWidth,
        height: metrics.visualViewport.clientHeight
      };
    } catch {}

    console.log(`[UFT] Built snapshot: ${frames.length} frames, ${nodes.length} nodes`);
    return { frames, nodes, createdAt: Date.now(), viewportSize };
  }, /* keepAttachedMs */ 0);
}

function bboxFromQuads(quads: number[] | undefined): [number, number, number, number] {
  if (!quads || quads.length < 8) return [0, 0, 0, 0];
  const xs = [quads[0], quads[2], quads[4], quads[6]];
  const ys = [quads[1], quads[3], quads[5], quads[7]];
  const x1 = Math.min(...xs), y1 = Math.min(...ys);
  const x2 = Math.max(...xs), y2 = Math.max(...ys);
  return [x1, y1, x2, y2];
}

function buildSelector(nodeName: string | undefined, attrs: Record<string, string>): string {
  if (!nodeName) return '';
  
  let selector = nodeName;
  if (attrs.id) {
    selector = `#${attrs.id}`;
  } else if (attrs.class) {
    selector += '.' + attrs.class.split(' ').join('.');
  }
  
  // Add key attributes for specificity
  if (attrs.type) selector += `[type="${attrs.type}"]`;
  if (attrs.name) selector += `[name="${attrs.name}"]`;
  
  return selector;
}

// Resolve element by EncodedId
export async function resolveElement(
  tabId: number, 
  encodedId: EncodedId
): Promise<{ frameId: string; backendNodeId: number; nodeId?: number } | null> {
  const [frameOrdinal, backendNodeId] = encodedId.split(':').map(Number);
  
  return cdp.with(tabId, async () => {
    // Get current frame tree
    const frameTree = await cdp.send<{ frameTree: any }>(tabId, 'Page.getFrameTree');
    
    // Find frame by ordinal
    let targetFrame: { id: string } | null = null;
    let currentOrdinal = 0;
    
    function findFrame(node: any): boolean {
      if (currentOrdinal === frameOrdinal) {
        targetFrame = { id: node.frame.id };
        return true;
      }
      currentOrdinal++;
      if (node.childFrames) {
        for (const ch of node.childFrames) {
          if (findFrame(ch)) return true;
        }
      }
      return false;
    }
    
    findFrame(frameTree.frameTree);
    
    if (!targetFrame) return null;
    
    // Push node to frontend to get nodeId
    try {
      const result = await cdp.send<any>(
        tabId,
        'DOM.pushNodesByBackendIdsToFrontend',
        { backendNodeIds: [backendNodeId] },
        cdp.routeForFrame(targetFrame.id)
      );
      
      return {
        frameId: targetFrame.id,
        backendNodeId,
        nodeId: result.nodeIds?.[0]
      };
    } catch {
      return null;
    }
  });
}
