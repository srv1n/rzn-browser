// Content-script-safe Input Synthesis Ladder
// This is a reduced ladder that only uses DOM + scripted events.
//
// The full ladder (DOM → scripted → CDP) depends on background-only APIs (chrome.debugger, chrome.tabs).
// Importing it in a content script can crash the script at load time.

import { ResolvedElement, InputRung, ResultEnvelope, createResultEnvelope } from '../types/targets';
import { DOMInputExecutor } from './rungs/dom';
import { ScriptedInputExecutor } from './rungs/scripted';

export interface InputAction {
  type: 'click' | 'fill' | 'key' | 'hover' | 'scroll';
  value?: string;
  key?: string;
  options?: {
    button?: 'left' | 'right' | 'middle';
    modifiers?: string[];
    force?: boolean;
  };
}

interface RungExecutor {
  canExecute(element: ResolvedElement, action: InputAction): boolean;
  execute(element: ResolvedElement, action: InputAction): Promise<boolean>;
}

export class ContentInputLadder {
  private executors: Array<{ rung: InputRung; exec: RungExecutor }>;

  constructor() {
    this.executors = [
      { rung: InputRung.DOM, exec: new DOMInputExecutor() as any },
      { rung: InputRung.SCRIPTED, exec: new ScriptedInputExecutor() as any },
    ];
  }

  async execute(element: ResolvedElement, action: InputAction): Promise<ResultEnvelope<boolean>> {
    const start = performance.now();
    let escalated = false;

    for (const { rung, exec } of this.executors) {
      try {
        if (!exec.canExecute(element, action)) {
          escalated = true;
          continue;
        }

        const ok = await exec.execute(element, action);
        const ms = performance.now() - start;
        if (ok) {
          return createResultEnvelope(true, rung, escalated, ms, element);
        }

        escalated = true;
      } catch (e: any) {
        escalated = true;
        const ms = performance.now() - start;
        // Continue to next rung; only return error if we exhaust all options.
        if (rung === InputRung.SCRIPTED) {
          return createResultEnvelope(false, rung, escalated, ms, element, e?.message || String(e));
        }
      }
    }

    const ms = performance.now() - start;
    return createResultEnvelope(false, InputRung.SCRIPTED, true, ms, element, 'No applicable input rung');
  }
}

export const inputLadder = new ContentInputLadder();

