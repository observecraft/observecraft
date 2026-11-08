import { memo, useState, type ReactNode } from 'react';
import { Check, Copy } from 'lucide-react';
import { highlightLine, type CodeLanguage } from './highlight';
import './codeBlock.css';

interface Props {
  code: string;
  filename?: string;
  badge?: ReactNode;
  /** YAML (default) or JSON highlighting. */
  language?: CodeLanguage;
}

export const CodeBlock = memo(function CodeBlock({ code, filename, badge, language = 'yaml' }: Props) {
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
            <span>{highlightLine(line, language)}</span>
          </div>
        ))}
      </pre>
    </figure>
  );
});
