import type { ReactNode } from 'react';
import { markdownBlocks } from './markdownBlocks';

// Tiny, safe Markdown subset for Note widgets: #/##/### headings, **bold**, *italic*, `code`,
// [links](http…) (http/https only), "- " and "1. " lists, "---" rules, paragraphs. No raw HTML is ever rendered.

function inline(text: string, key: string): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /(\*\*[^*]+\*\*)|(\*[^*]+\*)|((?<!\w)_[^_\n]+_(?!\w))|(`[^`]+`)|(\[[^\]]+\]\([^)\s]+\))/g;
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
            {inline(label, k)}
          </a>
        ) : (
          inline(label, k)
        ),
      );
    } else out.push(<em key={k}>{tok.slice(1, -1)}</em>);
    last = m.index + tok.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

export function Markdown({ source }: { source: string }) {
  return (
    <>
      {markdownBlocks(source).map((block, b) => {
        const key = `b${b}`;
        if (block.type === 'rule') return <hr key={key} />;
        if (block.type === 'paragraph') return <p key={key}>{inline(block.text, key)}</p>;
        if (block.type === 'heading') {
          const Heading = (['h1', 'h2', 'h3'] as const)[block.level - 1];
          return <Heading key={key}>{inline(block.text, key)}</Heading>;
        }
        const List = block.ordered ? 'ol' : 'ul';
        return (
          <List key={key}>
            {block.items.map((li, i) => (
              <li key={i}>{inline(li, `${key}-${i}`)}</li>
            ))}
          </List>
        );
      })}
    </>
  );
}
