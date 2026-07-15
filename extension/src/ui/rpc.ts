import { z } from 'zod';

export class SupervisorUnreachable extends Error {
  constructor(message = 'Supervisor unreachable') {
    super(message);
    this.name = 'SupervisorUnreachable';
  }
}

export class SupervisorApplicationError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'SupervisorApplicationError';
  }
}

const run = z.object({
  run_id: z.string(),
  workflow_id: z.string(),
  origin: z.string(),
  started_at: z.number(),
  ended_at: z.number().nullish(),
  status: z.string(),
}).passthrough();
const settings = z.object({
  run_retention_count: z.number(),
  run_retention_days: z.number(),
  notifications_enabled: z.boolean(),
  notify_on: z.string(),
  fleet_keep_window_on_failure: z.boolean(),
}).passthrough();

const schemas: Record<string, z.ZodTypeAny> = {
  'status.snapshot': z.object({
    supervisor_version: z.string(),
    native_host_connected: z.boolean(),
    extension_connected: z.boolean(),
    paused: z.boolean(),
    now_running: z.record(z.unknown()).nullable(),
    fleet: z.record(z.unknown()).nullable(),
    recent_runs: z.array(run),
    flagged_workflows: z.number(),
  }).passthrough(),
  'runs.list': z.object({ ok: z.boolean(), total: z.number(), runs: z.array(run) }).passthrough(),
  'runs.get': z.discriminatedUnion('ok', [
    z.object({ ok: z.literal(true), record: run, result: z.record(z.unknown()) }).passthrough(),
    z.object({ ok: z.literal(false), error: z.string(), run_id: z.string() }).passthrough(),
  ]),
  'runs.replay': z.object({ run_id: z.string().optional() }).passthrough(),
  'runs.start': z.object({ run_id: z.string().optional() }).passthrough(),
  'runs.cancel': z.object({ ok: z.boolean(), cancel_requested: z.boolean() }).passthrough(),
  'automation.pause': z.object({
    ok: z.boolean(), paused: z.literal(true), cancel_current: z.boolean(),
  }).passthrough(),
  'automation.resume': z.object({ ok: z.boolean(), paused: z.literal(false) }).passthrough(),
  'runs.get_failure_context': z.object({
    console_tail: z.string().optional(), screenshot_b64: z.string().optional(),
    dom_excerpt: z.string().optional(), capture_unavailable: z.string().optional(),
  }).passthrough(),
  'workflows.health': z.object({ workflows: z.array(z.record(z.unknown())) }).passthrough(),
  'workflows.list': z.object({ workflows: z.array(z.object({
    workflow_id: z.string(),
    name: z.string().optional(),
    source: z.string(),
    health: z.record(z.unknown()).nullish(),
  }).passthrough()) }).passthrough(),
  'fleet.status': z.object({ state: z.string() }).passthrough(),
  'fleet.enroll': z.record(z.unknown()),
  'fleet.unenroll': z.record(z.unknown()),
  'settings.get': settings,
  'settings.set': settings,
  'diagnostics.export': z.object({ path: z.string() }).passthrough(),
  'logs.tail': z.object({ entries: z.array(z.record(z.unknown())) }).passthrough(),
};

// The native host owns a 10s control deadline. The UI must wait longer so the
// host's real error wins instead of inventing an earlier unreachable state.
export const DEFAULT_RPC_TIMEOUT_MS = 12_000;

export async function rpc<T = unknown>(
  method: string,
  params: Record<string, unknown> = {},
  timeoutMs = DEFAULT_RPC_TIMEOUT_MS,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let response: any;
  try {
    response = await Promise.race([
      chrome.runtime.sendMessage({ cmd: 'supervisor_rpc', payload: { method, params } }),
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error('timeout')), timeoutMs);
      }),
    ]);
  } catch (error: any) {
    throw new SupervisorUnreachable(error?.message || String(error));
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
  if (!response?.success) {
    if (typeof response?.error === 'string' && response.error) {
      throw new SupervisorApplicationError(response.error);
    }
    throw new SupervisorUnreachable('No supervisor response');
  }
  const schema = schemas[method];
  return (schema ? schema.parse(response.result) : response.result) as T;
}
