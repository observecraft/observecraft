// Import dashboard data from integrations-core without proprietary artwork or documentation prose.
// node scripts/import-integration-dashboards.mjs [integration] [commit SHA]
import { mkdir, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { dashboardNote } from './integration-dashboard-copy.mjs';
export const PINNED_REVISION = 'c7dbc72cac1946294acda77f6491ecd081d9b498';

export function convertDashboard(doc, integration, path, revision) {
  const free = doc.layout_type === 'free';
  let seq = 0;
  function widgets(items, sourceWidth, nested = false, section = '') {
    // Ordered groups have their own coordinate system (e.g. an 8-column group),
    // but the nested ObserveCraft canvas always has 12 columns.
    const cols = free && !nested ? 48 : 12;
    const extent = sourceWidth ?? Math.max(free ? 1 : 12, ...items.map((w) => (w.layout?.x ?? 0) + (w.layout?.width ?? 0)));
    // Legacy free-layout coordinates are grid units, not pixels. Dividing them
    // by ten collapses a 14-unit chart into a two-row strip.
    const rowScale = free && !nested ? 1 : 2;
    let cursor = 0;
    const converted = items.map((item) => {
      const d = item.definition;
      let definition;
      if (d.type === 'group') {
        definition = {
          type: 'group',
          title: d.title || (d.banner_img ? doc.title : ''),
          color: 'blue',
          compact: true,
          ...(d.banner_img ? { logo: integration } : {}),
          widgets: widgets(d.widgets ?? [], item.layout?.width, true, d.title ?? ''),
        };
      } else if (d.type === 'note') {
        // Do not republish Datadog documentation or marketing copy.
        definition = {
          type: 'note',
          content: dashboardNote(d.content ?? '', doc.title, section),
          background: d.background_color === 'gray' ? 'gray' : 'white',
          align: 'left',
          fontSize: 'md',
        };
      } else if (d.type === 'image') {
        definition = { type: 'image', title: 'Kubernetes resource model', src: '/images/dashboard-kubernetes-capacity.svg' };
      } else {
        // Preserve requests, formulas, units, thresholds and visualization options verbatim.
        const { banner_img, image_url, ...original } = d;
        definition = { type: 'integration', title: d.title ?? '', original };
      }
      const layout = item.layout;
      let geometry;
      if (layout) {
        const scale = cols / extent;
        geometry = {
          x: Math.round(layout.x * scale),
          y: Math.round(layout.y * rowScale),
          w: Math.max(1, Math.round(layout.width * scale)),
          h: Math.max(4, Math.round(layout.height * rowScale)),
        };
      } else {
        const h = d.type === 'group' ? Math.max(8, Math.max(0, ...definition.widgets.map((w) => w.layout.y + w.layout.h)) + 2) : d.type === 'note' ? 3 : 6;
        geometry = { x: 0, y: cursor, w: cols, h };
      }
      if (definition.type === 'group') {
        geometry.h = Math.max(geometry.h, 2 + Math.max(0, ...definition.widgets.map((w) => w.layout.y + w.layout.h)));
      }
      cursor = Math.max(cursor, geometry.y + geometry.h);
      return { id: `upstream-${item.id ?? ++seq}`, definition, layout: geometry };
    });
    // Group headers add space that was implicit in the upstream layout. Move
    // subsequent rows below those headers instead of relying on runtime overlap
    // correction, which otherwise changes the layout after the first paint.
    if (!free || nested) {
      const placed = [];
      for (const item of [...converted].sort((a, b) => a.layout.y - b.layout.y || a.layout.x - b.layout.x)) {
        for (;;) {
          const a = item.layout;
          const collisions = placed.filter(({ layout: b }) => a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y);
          if (!collisions.length) break;
          a.y = Math.max(...collisions.map(({ layout: b }) => b.y + b.h));
        }
        placed.push(item);
      }
    }
    return converted;
  }
  return {
    id: `${integration}-${path
      .split('/')
      .at(-1)
      .replace(/\.json$/, '')
      .replace(/_/g, '-')}`,
    title: doc.title,
    description: `Built-in ${integration} dashboard from integrations-core.`,
    kind: 'integration',
    logo: integration,
    teams: [],
    layoutType: free ? 'free' : 'grid',
    templateVariables: (doc.template_variables ?? []).map((v) => ({ name: v.name, prefix: v.prefix ?? '', default: v.default ?? '*' })),
    widgets: widgets(doc.widgets ?? [], undefined),
    source: { repository: 'DataDog/integrations-core', revision, path: `${integration}/${path}`, license: 'BSD-3-Clause' },
  };
}

async function main() {
  const integration = process.argv[2] ?? 'kubernetes';
  const revision = process.argv[3] ?? PINNED_REVISION;
  if (!/^[a-z0-9_]+$/.test(integration) || !/^[a-f0-9]{40}$/.test(revision)) throw new Error('Expected integration directory and full commit SHA');
  const base = `https://raw.githubusercontent.com/DataDog/integrations-core/${revision}/`;
  async function read(path) {
    const response = await fetch(base + path);
    if (!response.ok) throw new Error(`${path}: HTTP ${response.status}`);
    return response.text();
  }
  const license = await read('LICENSE');
  if (!license.includes('Neither the name of integrations-core') || !license.includes('Redistribution and use in source and binary forms'))
    throw new Error('Upstream license changed; review before importing');
  const manifest = JSON.parse(await read(`${integration}/manifest.json`));
  const dashboards = [];
  for (const path of new Set(Object.values(manifest.assets?.dashboards ?? {}))) {
    if (!/^assets\/dashboards\/[a-zA-Z0-9_-]+\.json$/.test(path)) throw new Error(`Unexpected asset path: ${path}`);
    dashboards.push(convertDashboard(JSON.parse(await read(`${integration}/${path}`)), integration, path, revision));
  }
  const target = new URL(`../server/seeds/integrations/${integration}/`, import.meta.url);
  await mkdir(target, { recursive: true });
  await writeFile(new URL('dashboards.json', target), JSON.stringify(dashboards, null, 2) + '\n');
  await writeFile(new URL('LICENSE', target), license);
  console.log(`Imported ${dashboards.length} upstream dashboards with original metric widget definitions.`);
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
