// An integrations-core README section as the modal renders it: Markdown parts and the Host / Docker /
// Kubernetes tab groups Datadog's docs mark with `<!-- xxx tabs xxx -->` comments.

export type ReadmeSegment = { kind: 'markdown'; markdown: string } | { kind: 'tabs'; tabs: { name: string; markdown: string }[] };

const TABS_START = /<!-- xxx tabs xxx -->/;
const TABS_END = /<!-- xxz tabs xxx -->/;
const TAB = /<!-- xxx tab "([^"]+)" xxx -->([\s\S]*?)<!-- xxz tab xxx -->/g;

/** Datadog's app pages (`https://app.datadoghq.com/…`) as ours; others go to the home page. */
const APP_ROUTES: [RegExp, string][] = [
  [/^\/account\/settings\/agent/, '/infrastructure/add'],
  [/^\/logs\/livetail/, '/logs/livetail'],
  [/^\/logs\/pipelines/, '/logs/pipelines'],
  [/^\/logs/, '/logs'],
  [/^\/monitors/, '/monitors'],
  [/^\/dashboard/, '/dashboards'],
  [/^\/apm/, '/apm'],
  [/^\/metric/, '/metrics'],
  [/^\/process/, '/process'],
  [/^\/organization-settings\/api-keys/, '/organization-settings/api-keys'],
  [/^\/(account\/settings|integrations)/, '/integrations'],
];
const APP_URL = /https?:\/\/(?:app|us3|us5|ap1|ap2)\.datadoghq\.(?:com|eu)(\/[^\s)\]>"]*)?/g;

/**
 * A README's instructions as they apply here: links to Datadog's app become our pages, and "in Datadog", "the Datadog
 * app" and "your Datadog API key" name ObserveCraft (the Agent sends to it with an ingest key). The Datadog Agent and
 * other products keep their names; code (configuration, commands) is left as it is.
 */
export function forObserveCraft(md: string): string {
  const prose = (text: string) =>
    text
      .replace(APP_URL, (_, path = '/') => APP_ROUTES.find(([re]) => re.test(path))?.[1] ?? '/')
      .replace(/\b(in|to) (?:the |your )?Datadog (?:app|application|UI|web app|platform|site|account)\b(?! team)/g, '$1 ObserveCraft')
      .replace(/\bDatadog API [Kk]ey\b/g, 'ObserveCraft ingest key')
      .replace(/\b(in|to) (?:your )?Datadog\b(?! Agent| Cluster|’s|'s| account team)/gi, '$1 ObserveCraft');
  return md
    .split(/(```[\s\S]*?```|`[^`\n]*`)/)
    .map((part, i) => (i % 2 ? part : prose(part)))
    .join('');
}

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
    return forObserveCraft(text && definitions.length ? `${text}\n\n${definitions.join('\n')}` : text);
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
