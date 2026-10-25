import { memo, useState, type ReactNode } from 'react';
import { Check, Copy } from 'lucide-react';
import './codeBlock.css';

// Minimal YAML highlighting: comments, keys, ${env:VARS}, quoted strings. Enough for config snippets.
const TOKEN = /(?<comment>#.*$)|(?<env>\$\{env:[^}]+\})|(?<str>"[^"]*"|'[^']*')/g;

function highlightLine(line: string): ReactNode[] {
  const out: ReactNode[] = [];
  const key = /^(\s*-?\s*)([\w./-]+)(:)(?=\s|$)/.exec(line);
  let rest = line;
  if (key) {
    out.push(key[1], <span key="k" className="cb-key">{key[2]}</span>, key[3]);
    rest = line.slice(key[0].length);
  }
  let last = 0;
  for (const m of rest.matchAll(TOKEN)) {
    const g = Object.entries(m.groups ?? {}).find(([, v]) => v !== undefined)?.[0];
    if (!g || m.index === undefined) continue;
    if (m.index > last) out.push(rest.slice(last, m.index));
    out.push(
      <span key={m.index} className={`cb-${g}`}>
        {m[0]}
      </span>,
    );
    last = m.index + m[0].length;
  }
  if (last < rest.length) out.push(rest.slice(last));
  return out;
}

interface Props {
  code: string;
  filename?: string;
  badge?: ReactNode;
}

export const CodeBlock = memo(function CodeBlock({ code, filename, badge }: Props) {
  const [copied, setCopied] = useState(false);
  return (
    <figure className="code-block">
      <figcaption>
        {badge}
        {filename && <span className="cb-filename mono">{filename}</span>}
        <button
          className="cb-copy"
          title="Copy"
          onClick={() => {
            navigator.clipboard?.writeText(code);
            setCopied(true);
            setTimeout(() => setCopied(false), 1200);
          }}
        >
          {copied ? <Check size={14} /> : <Copy size={14} />}
          {copied ? 'Copied' : 'Copy'}
        </button>
      </figcaption>
      <pre className="mono">
        {code.split('\n').map((line, i) => (
          <div key={i} className="cb-line">
            <span className="cb-ln">{i + 1}</span>
            <span>{highlightLine(line)}</span>
          </div>
        ))}
      </pre>
    </figure>
  );
});
