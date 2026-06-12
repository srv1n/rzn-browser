/**
 * LLM DOM Formatter
 * Creates a simplified, LLM-friendly representation of the DOM
 * Using index-based element references for CSP-safe automation
 */

import { DOMElement, DOMTextNode, DOMState } from '../types/dom';

export interface LLMDOMElement {
  idx: number;
  type: string;
  text?: string;
  value?: string;
  placeholder?: string;
  label?: string;
  href?: string;
  src?: string;
  alt?: string;
  checked?: boolean;
  selected?: boolean;
  options?: string[];
}

export interface LLMDOMRepresentation {
  url: string;
  title: string;
  elements: LLMDOMElement[];
  forms: {
    idx: number;
    fields: LLMDOMElement[];
  }[];
  links: {
    idx: number;
    text: string;
    href: string;
  }[];
  buttons: {
    idx: number;
    text: string;
    type?: string;
  }[];
  images: {
    idx: number;
    alt?: string;
    src: string;
  }[];
}

export class LLMDOMFormatter {
  /**
   * Convert DOM state to LLM-friendly representation
   */
  static formatForLLM(domState: DOMState): LLMDOMRepresentation {
    const elements: LLMDOMElement[] = [];
    const forms: Map<number, LLMDOMElement[]> = new Map();
    const links: LLMDOMRepresentation['links'] = [];
    const buttons: LLMDOMRepresentation['buttons'] = [];
    const images: LLMDOMRepresentation['images'] = [];
    
    // Process all elements
    Object.values(domState.tree).forEach(node => {
      if (node.type === 'element') {
        const element = node as DOMElement;
        
        // Only process elements with highlight index (interactive elements)
        if (element.highlightIndex !== undefined) {
          const llmElement = this.createLLMElement(element);
          if (llmElement) {
            elements.push(llmElement);
            
            // Categorize elements
            switch (element.tag.toLowerCase()) {
              case 'a':
                if (element.attributes.href) {
                  links.push({
                    idx: element.highlightIndex,
                    text: element.text || '',
                    href: element.attributes.href
                  });
                }
                break;
                
              case 'button':
                buttons.push({
                  idx: element.highlightIndex,
                  text: element.text || element.attributes.value || '',
                  type: element.attributes.type
                });
                break;
                
              case 'input':
                if (element.attributes.type === 'submit' || element.attributes.type === 'button') {
                  buttons.push({
                    idx: element.highlightIndex,
                    text: element.attributes.value || '',
                    type: element.attributes.type
                  });
                } else if (element.parent) {
                  // Group form fields
                  const form = this.findParentForm(element, domState);
                  if (form && form.highlightIndex !== undefined) {
                    if (!forms.has(form.highlightIndex)) {
                      forms.set(form.highlightIndex, []);
                    }
                    forms.get(form.highlightIndex)!.push(llmElement);
                  }
                }
                break;
                
              case 'img':
                if (element.attributes.src) {
                  images.push({
                    idx: element.highlightIndex,
                    alt: element.attributes.alt,
                    src: element.attributes.src
                  });
                }
                break;
                
              case 'select':
              case 'textarea':
                if (element.parent) {
                  const form = this.findParentForm(element, domState);
                  if (form && form.highlightIndex !== undefined) {
                    if (!forms.has(form.highlightIndex)) {
                      forms.set(form.highlightIndex, []);
                    }
                    forms.get(form.highlightIndex)!.push(llmElement);
                  }
                }
                break;
            }
          }
        }
      }
    });
    
    // Convert forms map to array
    const formsArray = Array.from(forms.entries()).map(([idx, fields]) => ({
      idx,
      fields
    }));
    
    return {
      url: domState.metadata.url,
      title: domState.metadata.title,
      elements,
      forms: formsArray,
      links,
      buttons,
      images
    };
  }
  
  /**
   * Create simplified element representation
   */
  private static createLLMElement(element: DOMElement): LLMDOMElement | null {
    if (element.highlightIndex === undefined) return null;
    
    const tag = element.tag.toLowerCase();
    const attrs = element.attributes;
    
    const llmElement: LLMDOMElement = {
      idx: element.highlightIndex,
      type: tag
    };
    
    // Add relevant attributes based on element type
    switch (tag) {
      case 'input':
        llmElement.type = attrs.type || 'text';
        if (attrs.value) llmElement.value = attrs.value;
        if (attrs.placeholder) llmElement.placeholder = attrs.placeholder;
        if (attrs.type === 'checkbox' || attrs.type === 'radio') {
          llmElement.checked = attrs.checked === 'true';
        }
        break;
        
      case 'textarea':
        if (element.text) llmElement.value = element.text;
        if (attrs.placeholder) llmElement.placeholder = attrs.placeholder;
        break;
        
      case 'select':
        llmElement.options = this.extractSelectOptions(element);
        if (attrs.value) llmElement.value = attrs.value;
        break;
        
      case 'a':
        if (attrs.href) llmElement.href = attrs.href;
        if (element.text) llmElement.text = element.text;
        break;
        
      case 'button':
        if (element.text) llmElement.text = element.text;
        if (attrs.type) llmElement.type = attrs.type;
        break;
        
      case 'img':
        if (attrs.src) llmElement.src = attrs.src;
        if (attrs.alt) llmElement.alt = attrs.alt;
        break;
        
      default:
        if (element.text) llmElement.text = element.text;
    }
    
    // Try to find associated label
    const label = this.findLabel(element);
    if (label) llmElement.label = label;
    
    return llmElement;
  }
  
  /**
   * Extract options from select element
   */
  private static extractSelectOptions(element: DOMElement): string[] {
    // This would need to be enhanced to properly extract options
    // For now, return empty array
    return [];
  }
  
  /**
   * Find parent form element
   */
  private static findParentForm(element: DOMElement, domState: DOMState): DOMElement | null {
    let current = element.parent;
    while (current) {
      const parent = domState.tree[current];
      if (parent && parent.type === 'element' && (parent as DOMElement).tag.toLowerCase() === 'form') {
        return parent as DOMElement;
      }
      current = parent?.parent;
    }
    return null;
  }
  
  /**
   * Find associated label for form element
   */
  private static findLabel(element: DOMElement): string | null {
    // Simple heuristic - look for aria-label or nearby text
    if (element.attributes['aria-label']) {
      return element.attributes['aria-label'];
    }
    
    if (element.attributes['title']) {
      return element.attributes['title'];
    }
    
    // More sophisticated label finding would require DOM traversal
    return null;
  }
  
  /**
   * Generate a text prompt describing available actions
   */
  static generateActionPrompt(dom: LLMDOMRepresentation): string {
    const parts: string[] = [];
    
    parts.push(`Current page: ${dom.title} (${dom.url})`);
    parts.push('');
    
    // Forms
    if (dom.forms.length > 0) {
      parts.push('Forms:');
      dom.forms.forEach(form => {
        parts.push(`  Form [${form.idx}]:`);
        form.fields.forEach(field => {
          const label = field.label || field.placeholder || field.type;
          const value = field.value ? ` (current: "${field.value}")` : '';
          parts.push(`    - ${label} [${field.idx}]${value}`);
        });
      });
      parts.push('');
    }
    
    // Links
    if (dom.links.length > 0) {
      parts.push('Links:');
      dom.links.slice(0, 10).forEach(link => {
        parts.push(`  [${link.idx}] ${link.text || link.href}`);
      });
      if (dom.links.length > 10) {
        parts.push(`  ... and ${dom.links.length - 10} more links`);
      }
      parts.push('');
    }
    
    // Buttons
    if (dom.buttons.length > 0) {
      parts.push('Buttons:');
      dom.buttons.forEach(button => {
        parts.push(`  [${button.idx}] ${button.text || button.type || 'Button'}`);
      });
      parts.push('');
    }
    
    // Interactive elements summary
    parts.push(`Total interactive elements: ${dom.elements.length}`);
    
    return parts.join('\n');
  }
}