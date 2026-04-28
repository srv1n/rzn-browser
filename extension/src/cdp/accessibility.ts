// Accessibility Tree via CDP - Get semantic element information without DOM pollution
// This is CRITICAL: use CDP Accessibility domain, NOT DOM parsing, to avoid CSP issues

import { cdpClient, CDPTarget } from './cdpClient';
import { frameRouter } from './frameRouter';
import { Accessibility, DOM, ElementInfo, UnifiedSnapshot } from './types';

export interface AccessibleElement {
  id: string; // Encoded as frameOrdinal:backendNodeId
  frameId: string;
  frameOrdinal: number;
  backendNodeId: DOM.BackendNodeId;
  axNodeId?: Accessibility.AXNodeId;
  role: string;
  name?: string;
  description?: string;
  value?: string;
  url?: string;
  visible: boolean;
  clickable: boolean;
  focusable: boolean;
  editable: boolean;
  required: boolean;
  disabled: boolean;
  checked?: boolean;
  expanded?: boolean;
  selected?: boolean;
  multiline?: boolean;
  readonly?: boolean;
  invalid?: boolean;
  level?: number;
  posInSet?: number;
  setSize?: number;
  bounds?: DOM.Rect;
  attributes?: Record<string, string>;
}

export class AccessibilityService {
  /**
   * Get complete accessibility snapshot across all frames
   * This is the main method for extracting semantic element information
   */
  async getAccessibilitySnapshot(tabId: number): Promise<UnifiedSnapshot> {
    console.log('[AccessibilityService] Building accessibility snapshot');
    
    // Ensure CDP is attached
    if (!frameRouter.isAttachedToTab(tabId)) {
      await frameRouter.attachToTab(tabId);
    }
    
    const target: CDPTarget = { tabId };
    
    // Enable required domains
    await cdpClient.enableDomains(target, ['Page', 'DOM', 'Accessibility']);
    
    // Get frame tree first
    const frameTree = await frameRouter.getFrameTree(tabId);
    const elements: ElementInfo[] = [];
    
    // Process each frame
    for (let ordinal = 0; ordinal < frameTree.length; ordinal++) {
      const frame = frameTree[ordinal];
      
      try {
        console.log(`[AccessibilityService] Processing frame ${ordinal}: ${frame.frameId}`);
        
        // Get full accessibility tree for this frame
        const axTree = await cdpClient.getFullAccessibilityTree(target, frame.frameId);
        
        if (!axTree?.nodes) {
          console.warn(`[AccessibilityService] No AX nodes in frame ${frame.frameId}`);
          continue;
        }
        
        // Process each accessibility node
        for (const axNode of axTree.nodes) {
          try {
            const element = await this.processAccessibilityNode(
              target,
              axNode,
              frame.frameId,
              ordinal
            );
            
            if (element) {
              elements.push(element);
            }
          } catch (error) {
            console.warn(`[AccessibilityService] Failed to process AX node:`, error);
          }
        }
      } catch (error) {
        console.warn(`[AccessibilityService] Failed to process frame ${frame.frameId}:`, error);
      }
    }
    
    // Get viewport size
    let viewportSize: UnifiedSnapshot['viewportSize'];
    try {
      const metrics = await cdpClient.getLayoutMetrics(target);
      viewportSize = {
        width: metrics.visualViewport.clientWidth,
        height: metrics.visualViewport.clientHeight
      };
    } catch (error) {
      console.warn('[AccessibilityService] Failed to get viewport size:', error);
    }
    
    console.log(`[AccessibilityService] Built snapshot: ${frameTree.length} frames, ${elements.length} elements`);
    
    return {
      frames: frameTree.map((f, i) => ({
        frameId: f.frameId,
        parentId: f.parentFrameId,
        url: f.url,
        ordinal: i,
        sessionId: f.sessionId,
        securityOrigin: f.securityOrigin
      })),
      elements,
      createdAt: Date.now(),
      viewportSize
    };
  }
  
  /**
   * Get accessible elements by role
   */
  async getElementsByRole(tabId: number, role: string): Promise<AccessibleElement[]> {
    const snapshot = await this.getAccessibilitySnapshot(tabId);
    return snapshot.elements
      .filter(el => el.role === role)
      .map(el => this.convertToAccessibleElement(el));
  }
  
  /**
   * Get all interactive elements (buttons, links, inputs, etc.)
   */
  async getInteractiveElements(tabId: number): Promise<AccessibleElement[]> {
    const snapshot = await this.getAccessibilitySnapshot(tabId);
    return snapshot.elements
      .filter(el => el.clickable || this.isInteractiveRole(el.role))
      .map(el => this.convertToAccessibleElement(el));
  }
  
  /**
   * Get all form elements
   */
  async getFormElements(tabId: number): Promise<AccessibleElement[]> {
    const snapshot = await this.getAccessibilitySnapshot(tabId);
    return snapshot.elements
      .filter(el => this.isFormRole(el.role))
      .map(el => this.convertToAccessibleElement(el));
  }
  
  /**
   * Find elements by accessible name
   */
  async findByAccessibleName(tabId: number, name: string): Promise<AccessibleElement[]> {
    const snapshot = await this.getAccessibilitySnapshot(tabId);
    const searchName = name.toLowerCase();
    
    return snapshot.elements
      .filter(el => 
        el.name?.toLowerCase().includes(searchName) ||
        el.text?.toLowerCase().includes(searchName)
      )
      .map(el => this.convertToAccessibleElement(el));
  }
  
  /**
   * Process individual accessibility node
   */
  private async processAccessibilityNode(
    target: CDPTarget,
    axNode: Accessibility.AXNode,
    frameId: string,
    frameOrdinal: number
  ): Promise<ElementInfo | null> {
    // Skip ignored nodes
    if (axNode.ignored) {
      return null;
    }
    
    // Must have backend DOM node ID
    if (!axNode.backendDOMNodeId) {
      return null;
    }
    
    const backendNodeId = axNode.backendDOMNodeId;
    const elementId = `${frameOrdinal}:${backendNodeId}`;
    
    // Extract role
    const role = this.extractRole(axNode);
    if (!role) {
      return null;
    }
    
    // Extract name, description, value
    const name = this.extractStringValue(axNode.name);
    const description = this.extractStringValue(axNode.description);
    const value = this.extractValue(axNode.value);
    
    // Extract properties
    const properties = this.extractProperties(axNode.properties || []);
    
    // Determine interaction capabilities
    const clickable = this.isClickableRole(role) || properties.focusable === true;
    const focusable = properties.focusable === true;
    const editable = properties.editable === true;
    const visible = !properties.hidden && !properties.hiddenRoot;
    
    // Get bounding box if possible
    let bounds: DOM.Rect | undefined;
    try {
      // Push to frontend to get nodeId
      const pushResult = await cdpClient.pushNodesByBackendIds(
        target, 
        [backendNodeId], 
        frameId
      );
      
      if (pushResult?.nodeIds?.[0]) {
        const nodeId = pushResult.nodeIds[0];
        
        // Get box model
        const boxModel = await cdpClient.getBoxModel(target, nodeId, frameId);
        if (boxModel?.model) {
          bounds = this.calculateBounds(boxModel.model.border || boxModel.model.content);
        }
      }
    } catch (error) {
      // Box model might not be available for all elements
      console.debug(`[AccessibilityService] Could not get bounds for ${elementId}:`, error);
    }
    
    // Extract URL for links
    let url: string | undefined;
    if (role === 'link') {
      // Try to get href from properties or description
      url = this.extractUrl(axNode.properties || []);
    }
    
    return {
      id: elementId,
      frameId,
      frameOrdinal,
      backendNodeId,
      role,
      name,
      text: name || description, // Use name as text fallback
      value: value?.toString(),
      url,
      visible,
      clickable,
      box: bounds,
      attributes: this.buildAttributesFromProperties(properties)
    };
  }
  
  /**
   * Extract role from AX node
   */
  private extractRole(axNode: Accessibility.AXNode): string | null {
    if (axNode.role?.value) {
      return axNode.role.value;
    }
    
    if (axNode.chromeRole?.value) {
      return axNode.chromeRole.value;
    }
    
    return null;
  }
  
  /**
   * Extract string value from AX value
   */
  private extractStringValue(axValue?: Accessibility.AXValue): string | undefined {
    if (!axValue) return undefined;
    
    if (axValue.type === 'string' || axValue.type === 'computedString') {
      return axValue.value?.toString();
    }
    
    return undefined;
  }
  
  /**
   * Extract value from AX value (can be any type)
   */
  private extractValue(axValue?: Accessibility.AXValue): any {
    return axValue?.value;
  }
  
  /**
   * Extract properties map from AX properties
   */
  private extractProperties(axProperties: Accessibility.AXProperty[]): Record<string, any> {
    const properties: Record<string, any> = {};
    
    for (const prop of axProperties) {
      properties[prop.name] = prop.value.value;
    }
    
    return properties;
  }
  
  /**
   * Calculate bounding rectangle from CDP quad points
   */
  private calculateBounds(quad: number[]): DOM.Rect {
    if (quad.length < 8) {
      return { x: 0, y: 0, width: 0, height: 0 };
    }
    
    const xs = [quad[0], quad[2], quad[4], quad[6]];
    const ys = [quad[1], quad[3], quad[5], quad[7]];
    
    const x = Math.min(...xs);
    const y = Math.min(...ys);
    const width = Math.max(...xs) - x;
    const height = Math.max(...ys) - y;
    
    return { x, y, width, height };
  }
  
  /**
   * Extract URL from AX properties (for links)
   */
  private extractUrl(axProperties: Accessibility.AXProperty[]): string | undefined {
    // Look for URL in various properties
    for (const prop of axProperties) {
      if (prop.name === 'url' && prop.value.value) {
        return prop.value.value.toString();
      }
    }
    
    return undefined;
  }
  
  /**
   * Build HTML attributes from AX properties
   */
  private buildAttributesFromProperties(properties: Record<string, any>): Record<string, string> {
    const attributes: Record<string, string> = {};
    
    // Map relevant AX properties to HTML attributes
    const propMapping: Record<string, string> = {
      'required': 'required',
      'disabled': 'disabled',
      'readonly': 'readonly',
      'multiline': 'multiline',
      'invalid': 'aria-invalid',
      'checked': 'checked',
      'expanded': 'aria-expanded',
      'selected': 'aria-selected',
      'level': 'aria-level',
      'posInSet': 'aria-posinset',
      'setSize': 'aria-setsize'
    };
    
    for (const [axProp, htmlAttr] of Object.entries(propMapping)) {
      if (properties[axProp] !== undefined) {
        attributes[htmlAttr] = properties[axProp].toString();
      }
    }
    
    return attributes;
  }
  
  /**
   * Check if role is clickable
   */
  private isClickableRole(role: string): boolean {
    const clickableRoles = [
      'button', 'link', 'menuitem', 'tab', 'option', 'radio', 'checkbox',
      'switch', 'treeitem', 'gridcell', 'columnheader', 'rowheader',
      'menuitemcheckbox', 'menuitemradio', 'listitem'
    ];
    
    return clickableRoles.includes(role.toLowerCase());
  }
  
  /**
   * Check if role is interactive
   */
  private isInteractiveRole(role: string): boolean {
    const interactiveRoles = [
      'button', 'link', 'textbox', 'combobox', 'listbox', 'menu', 'menubar',
      'tab', 'tabpanel', 'tree', 'grid', 'slider', 'spinbutton', 'progressbar',
      'scrollbar', 'searchbox', 'switch', 'option', 'radio', 'checkbox'
    ];
    
    return interactiveRoles.includes(role.toLowerCase());
  }
  
  /**
   * Check if role is form-related
   */
  private isFormRole(role: string): boolean {
    const formRoles = [
      'textbox', 'combobox', 'listbox', 'radio', 'checkbox', 'button',
      'searchbox', 'spinbutton', 'slider', 'switch', 'option'
    ];
    
    return formRoles.includes(role.toLowerCase());
  }
  
  /**
   * Convert ElementInfo to AccessibleElement
   */
  private convertToAccessibleElement(el: ElementInfo): AccessibleElement {
    return {
      id: el.id,
      frameId: el.frameId,
      frameOrdinal: el.frameOrdinal,
      backendNodeId: el.backendNodeId,
      role: el.role || 'unknown',
      name: el.name,
      value: el.value,
      url: el.url,
      visible: el.visible || false,
      clickable: el.clickable || false,
      focusable: this.isInteractiveRole(el.role || ''),
      editable: el.role === 'textbox' || el.attributes?.contenteditable === 'true',
      required: el.attributes?.required === 'true',
      disabled: el.attributes?.disabled === 'true',
      checked: el.attributes?.checked === 'true',
      expanded: el.attributes?.['aria-expanded'] === 'true',
      selected: el.attributes?.['aria-selected'] === 'true',
      multiline: el.attributes?.multiline === 'true',
      readonly: el.attributes?.readonly === 'true',
      invalid: el.attributes?.['aria-invalid'] === 'true',
      level: el.attributes?.['aria-level'] ? parseInt(el.attributes['aria-level']) : undefined,
      posInSet: el.attributes?.['aria-posinset'] ? parseInt(el.attributes['aria-posinset']) : undefined,
      setSize: el.attributes?.['aria-setsize'] ? parseInt(el.attributes['aria-setsize']) : undefined,
      bounds: el.box,
      attributes: el.attributes
    };
  }
}

// Singleton instance
export const accessibilityService = new AccessibilityService();