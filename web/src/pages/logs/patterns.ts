// Log patterns (Group into Patterns): from a pattern back to a search for its logs.

/** The search for a pattern's logs: its service and its longest fixed text (between `*` and quotes). */
export function patternQuery(p: { pattern: string; service: string }, query: string): string {
  const text = p.pattern
    .split(/[*"]/)
    .map((s) => s.trim())
    .reduce((longest, s) => (s.length > longest.length ? s : longest), '');
  return [query.trim(), p.service && `service:${p.service}`, text.length >= 3 && `"${text}"`].filter(Boolean).join(' ');
}
