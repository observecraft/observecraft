// An integrations-core README section as the modal renders it: Markdown parts and the Host / Docker /
// Kubernetes tab groups Datadog's docs mark with `<!-- xxx tabs xxx -->` comments.

export type ReadmeSegment = { kind: 'markdown'; markdown: string } | { kind: 'tabs'; tabs: { name: string; markdown: string }[] };

const TABS_START = /<!-- xxx tabs xxx -->/;
const TABS_END = /<!-- xxz tabs xxx -->/;
const TAB = /<!-- xxx tab "([^"]+)" xxx -->([\s\S]*?)<!-- xxz tab xxx -->/g;

export function readmeSegments(source: string): ReadmeSegment[] {
  // Reference link definitions are shared by the whole section: each part gets them.
  const definitions = source.match(/^\[[^\]]+\]:\s*\S+.*$/gm) ?? [];
  const body = source.replace(/^\[[^\]]+\]:\s*\S+.*$/gm, '');
  const part = (markdown: string) => {
    // Repeated until stable: removing one comment can join the text around it into another.
    let text = markdown;
    for (let previous = ''; previous !== text;) {
      previous = text;
      text = text.replace(/<!--(?! xx[xz] tab)[\s\S]*?-->/g, '');
    }
    text = text.trim();
    return text && definitions.length ? `${text}\n\n${definitions.join('\n')}` : text;
  };
  const segments: ReadmeSegment[] = [];
  let rest = body;
  while (rest) {
    const start = TABS_START.exec(rest);
    const end = start ? TABS_END.exec(rest.slice(start.index)) : null;
    if (!start || !end) {
      const markdown = part(rest);
      if (markdown) segments.push({ kind: 'markdown', markdown });
      break;
    }
    const before = part(rest.slice(0, start.index));
    if (before) segments.push({ kind: 'markdown', markdown: before });
    const group = rest.slice(start.index, start.index + end.index);
    const tabs = [...group.matchAll(TAB)].map(([, name, markdown]) => ({ name, markdown: part(markdown) }));
    if (tabs.length) segments.push({ kind: 'tabs', tabs });
    rest = rest.slice(start.index + end.index + end[0].length);
  }
  return segments;
}
