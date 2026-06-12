export type TabNavigationState = {
  url: string;
  pendingUrl: string;
  status: string;
};

export function tabNavigationChanged(before: TabNavigationState, after: TabNavigationState): boolean {
  const beforeUrl = before.pendingUrl || before.url;
  const afterUrl = after.pendingUrl || after.url;
  return !!afterUrl && afterUrl !== beforeUrl;
}
