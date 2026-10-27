// Block structure of the Note widgets' Markdown subset: #/##/### headings, "- " / "1. " lists, "---" rules, paragraphs.

export type MarkdownBlock = { type: 'heading'; level: number; text: string } | { type: 'list'; ordered: boolean; items: string[] } | { type: 'paragraph'; text: string } | { type: 'rule' };

const ENTITIES: Record<string, string> = { nbsp: '\u00a0', lt: '<', gt: '>', quot: '"', apos: "'", amp: '&' };

/** The named HTML entities notes use, decoded to text (React renders it as text, never as HTML). */
const decode = (line: string) => line.replace(/&(nbsp|lt|gt|quot|apos|amp);/g, (_, name: string) => ENTITIES[name]);

export function markdownBlocks(source: string): MarkdownBlock[] {
  const blocks: MarkdownBlock[] = [];
  let para: string[] = [];
  let list: { ordered: boolean; items: string[] } | undefined;
  const flush = () => {
    if (para.length) blocks.push({ type: 'paragraph', text: para.join(' ') });
    if (list) blocks.push({ type: 'list', ...list });
    para = [];
    list = undefined;
  };
  for (const raw of source.split('\n')) {
    const line = decode(raw.trimEnd());
    const heading = /^(#{1,3})\s+(.*)$/.exec(line);
    const item = /^(?:([-*])|\d+[.)])\s+(.*)$/.exec(line);
    if (heading) {
      flush();
      blocks.push({ type: 'heading', level: heading[1].length, text: heading[2] });
    } else if (item) {
      const ordered = !item[1];
      if (para.length || list?.ordered !== ordered) flush();
      list ??= { ordered, items: [] };
      list.items.push(item[2]);
    } else if (/^---+$/.test(line)) {
      flush();
      blocks.push({ type: 'rule' });
    } else if (line === '') flush();
    else {
      if (list) flush();
      para.push(line);
    }
  }
  flush();
  return blocks;
}
