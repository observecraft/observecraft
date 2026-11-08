import { memo, type ReactNode } from 'react';
import './codeBlock.css';

// Light syntax highlighting for the YAML and JSON the UI shows (manifests, configuration, payloads): keys, strings,
// numbers, booleans and null, comments, ${env:VARS}. Line based and dependency free; no full parser.
export type CodeLanguage = 'yaml' | 'json';

const YAML_TOKEN = /(?<comment>(?:^|\s)#.*$)|(?<env>\$\{env:[^}]+\})|(?<str>"(?:[^"\\]|\\.)*"|'[^']*')|(?<lit>(?<![\w.-])(?:true|false|null|~|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)(?![\w.:-]))/g;
const JSON_TOKEN = /(?<key>"(?:[^"\\]|\\.)*")(?=\s*:)|(?<str>"(?:[^"\\]|\\.)*")|(?<lit>\b(?:true|false|null)\b|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)/g;
const CLASS: Record<string, string> = { comment: 'cb-comment', env: 'cb-env', str: 'cb-str', lit: 'cb-lit', key: 'cb-key' };

function tokens(text: string, re: RegExp, out: ReactNode[], keyBase: string) {
  let last = 0;
  for (const m of text.matchAll(re)) {
    const group = Object.entries(m.groups ?? {}).find(([, v]) => v !== undefined)?.[0];
    if (!group || m.index === undefined) continue;
    if (m.index > last) out.push(text.slice(last, m.index));
    out.push(
      <span key={`${keyBase}${m.index}`} className={CLASS[group]}>
        {m[0]}
      </span>,
    );
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
}

/** One line of `language`, as React nodes. */
export function highlightLine(line: string, language: CodeLanguage = 'yaml'): ReactNode[] {
  const out: ReactNode[] = [];
  if (language === 'json') {
    tokens(line, JSON_TOKEN, out, 'j');
    return out;
  }
  // `key:` (also `- key:` in a list, and quoted keys) at the start of the line.
  const key = /^(\s*(?:-\s+)*)("[^"]*"|'[^']*'|[^\s#'"{}[\],][^:#'"{}[\],]*?)(:)(?=\s|$)/.exec(line);
  let rest = line;
  if (key) {
    out.push(
      key[1],
      <span key="k" className="cb-key">
        {key[2]}
      </span>,
      key[3],
    );
    rest = line.slice(key[0].length);
  } else {
    // `- value` in a list.
    const item = /^(\s*(?:-\s+)+)/.exec(line);
    if (item) {
      out.push(item[1]);
      rest = line.slice(item[0].length);
    }
  }
  // A plain (unquoted) scalar value is a string, as editors show it; literals, quotes and comments keep their colors.
  const plain = /^(\s*)([^\s#'"[\]{}&*!|>][^#]*?)(\s+#.*)?$/.exec(rest);
  if (plain && (key || rest !== line) && !LITERAL.test(plain[2]) && !plain[2].startsWith('${env:')) {
    out.push(
      plain[1],
      <span key="v" className="cb-str">
        {plain[2]}
      </span>,
    );
    if (plain[3]) tokens(plain[3], YAML_TOKEN, out, 'c');
    return out;
  }
  tokens(rest, YAML_TOKEN, out, 'y');
  return out;
}

const LITERAL = /^(?:true|false|null|~|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)$/;

/** Highlighted code for a `<pre>` the page styles itself: one line per row. */
export const HighlightedCode = memo(function HighlightedCode({ code, language }: { code: string; language: CodeLanguage }) {
  return (
    <>
      {code.split('\n').map((line, i, all) => (
        <span key={i}>
          {highlightLine(line, language)}
          {i < all.length - 1 && '\n'}
        </span>
      ))}
    </>
  );
});
