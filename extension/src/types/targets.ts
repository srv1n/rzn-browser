// Element targeting and resolution types for RZN Browser Native
// Supports stable EncodedId references and Input Synthesis Ladder

import { z } from 'zod';

// Encoded element identifier: "frameOrdinal:backendNodeId"
// Provides stable reference that survives DOM changes
export type EncodedId = string;

// Input synthesis rungs in escalation order
export enum InputRung {
  DOM = 1,        // Native DOM events (same-origin only)
  SCRIPTED = 2,   // Scripted MouseEvent/KeyboardEvent  
  CDP = 3         // Chrome DevTools Protocol (works everywhere)
}

// Target specification - multiple ways to identify an element
export interface TargetSpec {
  // Stable element identifier (preferred)
  encoded_id?: EncodedId;
  
  // CSS selector
  css?: string;
  
  // XPath expression
  xpath?: string;
  
  // Accessibility role name
  role_name?: string;
  
  // Text content or nearby text
  text_near?: string;
  
  // Optional frame context
  frame_ordinal?: number;
}

// Resolved element with stable identifier
export interface ResolvedElement {
  // Stable encoded identifier
  encoded_id: EncodedId;
  
  // Frame ordinal where element exists
  frame_ordinal: number;
  
  // Backend node ID from CDP
  backend_node_id: number;
  
  // Element's bounding box
  bounds: {
    x: number;
    y: number;
    width: number;
    height: number;
  };
  
  // Whether element is cross-origin
  is_cross_origin: boolean;
  
  // Original target spec used to find this element
  target_spec: TargetSpec;
  
  // Cache timestamp
  resolved_at: number;
}

// Result envelope that tracks which input rung was used
export interface ResultEnvelope<T = any> {
  // The actual result data
  result: T;
  
  // Which input rung was used (1=DOM, 2=SCRIPTED, 3=CDP)
  rung_used: InputRung;
  
  // Whether input escalated from a lower rung
  escalated: boolean;
  
  // Success/failure status
  success: boolean;
  
  // Error message if failed
  error?: string;
  
  // Performance metrics
  execution_time_ms: number;
  
  // Resolved element used (if applicable)
  resolved_element?: ResolvedElement;
}

// Zod schemas for runtime validation
export const EncodedIdSchema = z.string().regex(/^\d+:\d+$/, 'EncodedId must be frameOrdinal:backendNodeId');

export const InputRungSchema = z.nativeEnum(InputRung);

export const TargetSpecSchema = z.object({
  encoded_id: EncodedIdSchema.optional(),
  css: z.string().optional(),
  xpath: z.string().optional(),
  role_name: z.string().optional(),
  text_near: z.string().optional(),
  frame_ordinal: z.number().int().min(0).optional()
}).refine(
  (data) => Object.values(data).some(v => v !== undefined),
  { message: "At least one targeting method must be provided" }
);

export const ResolvedElementSchema = z.object({
  encoded_id: EncodedIdSchema,
  frame_ordinal: z.number().int().min(0),
  backend_node_id: z.number().int().positive(),
  bounds: z.object({
    x: z.number(),
    y: z.number(),
    width: z.number().min(0),
    height: z.number().min(0)
  }),
  is_cross_origin: z.boolean(),
  target_spec: TargetSpecSchema,
  resolved_at: z.number().int().positive()
});

export const ResultEnvelopeSchema = z.object({
  result: z.any(),
  rung_used: InputRungSchema,
  escalated: z.boolean(),
  success: z.boolean(),
  error: z.string().optional(),
  execution_time_ms: z.number().min(0),
  resolved_element: ResolvedElementSchema.optional()
});

// Type guards
export function isValidEncodedId(id: string): id is EncodedId {
  return /^\d+:\d+$/.test(id);
}

export function parseEncodedId(encoded_id: EncodedId): { frameOrdinal: number; backendNodeId: number } {
  const [frameOrdinal, backendNodeId] = encoded_id.split(':').map(Number);
  return { frameOrdinal, backendNodeId };
}

export function createEncodedId(frameOrdinal: number, backendNodeId: number): EncodedId {
  return `${frameOrdinal}:${backendNodeId}`;
}

// Helper to determine if target requires cross-origin handling
export function requiresCrossOriginHandling(target: TargetSpec, currentFrameOrdinal: number = 0): boolean {
  return target.frame_ordinal !== undefined && target.frame_ordinal !== currentFrameOrdinal;
}

// Helper to create result envelope
export function createResultEnvelope<T>(
  result: T,
  rung_used: InputRung,
  escalated: boolean = false,
  execution_time_ms: number = 0,
  resolved_element?: ResolvedElement,
  error?: string
): ResultEnvelope<T> {
  return {
    result,
    rung_used,
    escalated,
    success: !error,
    error,
    execution_time_ms,
    resolved_element
  };
}