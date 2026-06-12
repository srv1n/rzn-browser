const STATEMENT_TOKEN_RE = /(^|[\s;])(?:const|let|var|if|for|while|throw|try|await)\b/;

function expressionCandidate(source: string): string {
  return source.trim().replace(/^;+/, '').trimStart();
}

function isExpressionLike(source: string): boolean {
  return (
    source.startsWith('(') ||
    source.startsWith('[') ||
    source.startsWith('{') ||
    /^async\s*\(/.test(source) ||
    /^function\b/.test(source)
  );
}

export function buildScriptEvalBody(script: string): string {
  const source = String(script || '');
  const trimmed = source.trim();
  if (!trimmed) return '';
  if (/^return\b/.test(trimmed)) return source;

  const expressionSource = expressionCandidate(source);
  const expressionLike = isExpressionLike(expressionSource);
  const statementLike =
    !expressionLike &&
    (STATEMENT_TOKEN_RE.test(trimmed) ||
      trimmed.includes('\n') ||
      (trimmed.includes(';') && !expressionLike));

  return statementLike ? source : `return (${expressionSource});`;
}
