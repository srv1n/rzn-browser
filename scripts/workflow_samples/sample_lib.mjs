// Runs a workflow's output step (the step result.output_selector names) against the fake page the
// caller installed on globalThis, and writes or checks workflows/<system>/samples/<stem>.sample.json.
// `params` is what the extension hands the script (for `captured_responses: true`, the bodies a
// capture_responses step kept, as params.captured_responses).
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
let stale = 0;

export async function sample(rel, args, builder, { params = {}, note } = {}) {
  const manifest = JSON.parse(readFileSync(join(root, rel), 'utf8'));
  const step = manifest.steps.find((s) => s.id === manifest.result.output_selector.step_id);
  const names = args.map((_, i) => 'arg' + i);
  const output = await new AsyncFunction(...names, 'params', '"use strict";' + step.action.inputs.script)(...args, params);
  const samplePath = join(root, dirname(rel), 'samples', rel.split('/').pop().replace(/\.json$/, '.sample.json'));
  const text = JSON.stringify({ sample_of: manifest.id, source: 'hand-built',
    note: note || `Built by scripts/workflow_samples/${builder} from the workflow's own mapping step and invented page data; replace with a recorded run.`,
    output }, null, 2) + '\n';
  if (process.argv.includes('--write')) writeFileSync(samplePath, text);
  else if (readFileSync(samplePath, 'utf8') !== text) { console.error(`stale: ${samplePath}`); stale += 1; }
}

export function finish() {
  if (stale) process.exit(1);
  console.log('ok');
}
