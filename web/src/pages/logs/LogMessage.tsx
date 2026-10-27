import { memo, type ReactNode } from 'react';
import { stripAnsi } from '../../lib/format';

// Highlighting rules for raw log lines, matching how the log list colours them:
// leading timestamps and [TAGS] muted; URLs, paths, `key=value` and `word:` labels in link blue.
const RULES =
  /(?<date>^\d{4}[/-]\d{2}[/-]\d{2}[ T]\d{2}:\d{2}:\d{2}(?:\.\d+)?Z?)|(?<url>https?:\/\/[^\s'"]+)|(?<kv>\b[\w.]+=(?:'[^']*'|"[^"]*"|[^\s,]+))|(?<bracket>\[[\w-]+\])|(?<label>\b[A-Za-z_][\w.]*:(?=\s|$))|(?<path>(?<=^|\s)\/[\w\-./]+)/g;

const CLASS: Record<string, string> = {
  date: 'hl-muted',
  bracket: 'hl-muted',
  url: 'hl-link',
  path: 'hl-link',
  kv: 'hl-link',
  label: 'hl-link',
};

export function highlight(message: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  for (const m of message.matchAll(RULES)) {
    const group = Object.entries(m.groups ?? {}).find(([, v]) => v !== undefined)?.[0];
    if (!group || m.index === undefined) continue;
    if (m.index > last) out.push(message.slice(last, m.index));
    out.push(
      <span key={m.index} className={CLASS[group]}>
        {m[0]}
      </span>,
    );
    last = m.index + m[0].length;
  }
  if (last < message.length) out.push(message.slice(last));
  return out;
}

export const LogMessage = memo(function LogMessage({ message }: { message: string }) {
  return <>{highlight(stripAnsi(message))}</>;
});
