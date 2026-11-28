// The RZN mark and letter avatars, shared by the popup and the Settings pages.

/** An indigo tile with three index lines, then the wordmark. */
export const markSvg = (size = 22) => `<svg width="${size}" height="${size}" viewBox="0 0 24 24" aria-hidden="true"><rect width="24" height="24" rx="6.5" fill="currentColor"/><path d="M7 8.5h10M7 12h7M7 15.5h4" stroke="#fff" stroke-width="2" stroke-linecap="round"/></svg>`;
export const mark = (size = 22) => `${markSvg(size)}<span>RZN</span>`;

const escape = (value: unknown) => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]!));

/** The first letter of a name or site, on one of a few soft tints picked from the name. */
export function avatar(name: string, extra = ''): string {
  const letter = (name.replace(/^www\./, '').match(/[a-z0-9]/i)?.[0] || '?').toUpperCase();
  const tint = [...name].reduce((sum, c) => sum + c.charCodeAt(0), 0) % 5;
  return `<span class="avatar ${extra} ${tint ? `tint-${tint}` : ''}" aria-hidden="true">${escape(letter)}</span>`;
}
