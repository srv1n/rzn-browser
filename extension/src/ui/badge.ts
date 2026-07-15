export type BadgeState = {
  reachable: boolean;
  paused?: boolean;
  runningCount?: number;
  lastRunFailed?: boolean;
  flaggedWorkflows?: number;
};

export function badgeForState(state: BadgeState): { text: string; color: string } {
  if (!state.reachable) return { text: '•', color: '#7b8496' };
  if (state.paused) return { text: '⏸', color: '#8b6fcb' };
  if (state.lastRunFailed || (state.flaggedWorkflows ?? 0) > 0) {
    return { text: '!', color: '#d74455' };
  }
  if ((state.runningCount ?? 0) > 0) {
    return { text: String(state.runningCount), color: '#3478f6' };
  }
  return { text: '', color: '#3478f6' };
}
