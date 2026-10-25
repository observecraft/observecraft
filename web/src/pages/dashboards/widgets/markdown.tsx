import type { ReactNode } from 'react';

// Tiny, safe Markdown subset for Note widgets: #/##/### headings, **bold**, *italic*, `code`,
// [links](http…) (http/https only), "- " lists, "---" rules, paragraphs. No raw HTML is ever rendered.

function inline(text: string, key: string): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /(\*\*[^*]+\*\*)|(\*[^*]+\*)|(`[^`]+`)|(\[[^\]]+\]\([^)\s]+\))/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let i = 0;
  while ((m = re.exec(text))) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const tok = m[0];
    const k = `${key}-${i++}`;
    if (tok.startsWith('**')) out.push(<strong key={k}>{tok.slice(2, -2)}</strong>);
    else if (tok.startsWith('`')) out.push(<code key={k}>{tok.slice(1, -1)}</code>);
    else if (tok.startsWith('[')) {
      const label = tok.slice(1, tok.indexOf(']'));
      const href = tok.slice(tok.indexOf('(') + 1, -1);
      out.push(
        /^https?:\/\//.test(href) ? (
          <a key={k} href={href} target="_blank" rel="noreferrer noopener">
            {label}
          </a>
        ) : (
          label
        ),
      );
    } else out.push(<em key={k}>{tok.slice(1, -1)}</em>);
    last = m.index + tok.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

export function Markdown({ source }: { source: string }) {
  const blocks: ReactNode[] = [];
  const lines = source.split('\n');
  let para: string[] = [];
  let list: string[] = [];
  const flushPara = () => {
    if (para.length) blocks.push(<p key={`p${blocks.length}`}>{inline(para.join(' '), `p${blocks.length}`)}</p>);
    para = [];
  };
  const flushList = () => {
    if (list.length)
      blocks.push(
        <ul key={`u${blocks.length}`}>
          {list.map((li, i) => (
            <li key={i}>{inline(li, `l${blocks.length}-${i}`)}</li>
          ))}
        </ul>,
      );
    list = [];
  };
  for (const raw of lines) {
    const line = raw.trimEnd();
    const h = /^(#{1,3})\s+(.*)$/.exec(line);
    if (h) {
      flushPara();
      flushList();
      const level = h[1].length;
      const content = inline(h[2], `h${blocks.length}`);
      blocks.push(level === 1 ? <h1 key={`h${blocks.length}`}>{content}</h1> : level === 2 ? <h2 key={`h${blocks.length}`}>{content}</h2> : <h3 key={`h${blocks.length}`}>{content}</h3>);
    } else if (/^[-*]\s+/.test(line)) {
      flushPara();
      list.push(line.replace(/^[-*]\s+/, ''));
    } else if (/^---+$/.test(line)) {
      flushPara();
      flushList();
      blocks.push(<hr key={`r${blocks.length}`} />);
    } else if (line === '') {
      flushPara();
      flushList();
    } else {
      flushList();
      para.push(line);
    }
  }
  flushPara();
  flushList();
  return <>{blocks}</>;
}
