// DOM pruning helper intended to be passed into chrome.scripting.executeScript({ func }).
//
// IMPORTANT: This function must be self-contained (no external references) because Chrome
// serializes only the function body when injecting into the page context.
export function pruneDOM(): string {
  const MAX_CHARS = 1_000_000;

  try {
    const root = document.documentElement.cloneNode(true) as HTMLElement;

    // Remove obviously noisy/heavy elements first.
    root
      .querySelectorAll(
        [
          'script',
          'style',
          'noscript',
          'svg',
          'canvas',
          'img',
          'video',
          'audio',
          'iframe',
          'object',
          'embed'
        ].join(',')
      )
      .forEach((el) => el.remove());

    // Strip inline handlers/styles to reduce size and avoid leaking dynamic code.
    root.querySelectorAll<HTMLElement>('*').forEach((el) => {
      for (const attr of Array.from(el.attributes)) {
        const name = attr.name.toLowerCase();
        if (name === 'style' || name.startsWith('on')) {
          el.removeAttribute(attr.name);
        }
      }
    });

    let html = root.outerHTML;
    if (html.length > MAX_CHARS) {
      html = html.slice(0, MAX_CHARS) + '<!-- truncated -->';
    }

    return html;
  } catch (e) {
    // Fallback: do not fail the caller; return raw outerHTML.
    try {
      const html = document.documentElement.outerHTML;
      if (html.length > MAX_CHARS) {
        return html.slice(0, MAX_CHARS) + '<!-- truncated -->';
      }
      return html;
    } catch {
      return '';
    }
  }
}

