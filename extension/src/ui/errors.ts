import { SupervisorApplicationError, SupervisorUnreachable } from './rpc';

/** What a person sees for an error: a title and one plain sentence. Raw detail goes to the console only. */
export type Friendly = { kind: 'unreachable' | 'outdated' | 'failed'; title: string; line: string };

// A helper older than this extension refuses methods or settings it does not know.
const OUTDATED = /method not allowed|unknown supervisor method|method not found|unknown setting|-32601/i;
// Text that is ours to show as is: a sentence with no RPC, JSON or code in it.
const PLAIN = /^[^{}[\]<>]*$/;

export function friendly(error: unknown, where = 'RZN'): Friendly {
  console.warn(`[${where}]`, error);
  const message = error instanceof Error ? error.message : String(error ?? '');
  if (error instanceof SupervisorUnreachable) return { kind: 'unreachable', title: "RZN isn't responding", line: 'The RZN helper on this computer didn’t answer. Make sure RZN is running, then try again.' };
  if (OUTDATED.test(message)) return { kind: 'outdated', title: 'RZN needs an update', line: 'The RZN helper on this computer is older than this extension. Update RZN, then try again.' };
  if (error instanceof SupervisorApplicationError) {
    // The helper's own validation messages are short sentences; anything else stays in the console.
    const sentence = message.length <= 140 && PLAIN.test(message) && !/rpc|native|supervisor|json/i.test(message);
    return { kind: 'failed', title: 'Something went wrong', line: sentence ? message : 'That didn’t work. Try again, or restart RZN if it keeps happening.' };
  }
  return { kind: 'failed', title: 'Something went wrong', line: message && message.length <= 140 && PLAIN.test(message) ? message : 'That didn’t work. Try again, or restart RZN if it keeps happening.' };
}
