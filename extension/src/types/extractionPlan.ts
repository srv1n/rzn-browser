import { z } from 'zod';

// A small, validated DSL for deterministic extraction without arbitrary JS execution.
// This is intentionally conservative: CSS/XPath only (no eval), and field selectors are
// resolved relative to the chosen scope/item container.

export const ExtractionScopeSchema = z.object({
  css: z.string().min(1).optional(),
  xpath: z.string().min(1).optional(),
}).refine((v) => !!(v.css || v.xpath), {
  message: 'scope must include css or xpath',
});

export const ExtractionFieldSchema = z.object({
  name: z.string().min(1),
  selector: z.string().min(1),
  attribute: z.string().min(1).optional(),
  optional: z.boolean().optional(),
});

export const ExtractionPlanV1Schema = z.object({
  version: z.literal(1),
  mode: z.enum(['single', 'list']),
  scope: ExtractionScopeSchema.optional(),
  // For list-mode extraction: selector for repeated items, relative to scope.
  item_selector: z.string().min(1).optional(),
  limit: z.number().int().min(1).max(2000).optional(),
  fields: z.array(ExtractionFieldSchema).min(1),
}).superRefine((plan, ctx) => {
  if (plan.mode === 'list' && !plan.item_selector) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      message: 'item_selector is required for list mode',
      path: ['item_selector'],
    });
  }
});

export type ExtractionPlanV1 = z.infer<typeof ExtractionPlanV1Schema>;

