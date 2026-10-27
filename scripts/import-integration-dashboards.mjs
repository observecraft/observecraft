// Import integrations-core dashboards (BSD-3-Clause): widgets, notes, colors and the images embedded
// in them. Images only linked from them are hosted by Datadog, not part of the repository, and are
// replaced by our own artwork (web/public/images/integrations/<integration>-banner.svg).
// node scripts/import-integration-dashboards.mjs [integration] [commit SHA]
import { mkdir, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
export const PINNED_REVISION = 'c7dbc72cac1946294acda77f6491ecd081d9b498';

/** Units of the Agent's own host metrics (they have no integrations-core metadata.csv). */
const AGENT_UNITS = {
  kubernetes: [
    ['system.cpu.idle', 'percent'],
    ['system.cpu.iowait', 'percent'],
    ['system.cpu.system', 'percent'],
    ['system.cpu.user', 'percent'],
    ['system.cpu.num_cores', 'core'],
    ['system.mem.free', 'byte'],
    ['system.mem.pct_usable', 'fraction'],
    ['system.mem.total', 'byte'],
    ['system.mem.usable', 'byte'],
    ['system.mem.used', 'byte'],
  ],
};

/** Integrations whose metadata.csv gives the units of the metrics an integration's dashboards use. */
const UNIT_SOURCES = {
  kubernetes: [
    'kubelet',
    'kubernetes_state_core',
    'kubernetes_state',
    'etcd',
    'kube_apiserver_metrics',
    'kube_controller_manager',
    'kube_scheduler',
    'container',
    'system_core',
    'disk',
    'network',
    'tls',
  ],
};

/** Our colors for integrations-core's note and group colors (vivid_* are the saturated variants). */
const NOTE_COLORS = { white: 'white', transparent: 'white', gray: 'gray', blue: 'blue', purple: 'purple', pink: 'purple', yellow: 'yellow', orange: 'yellow', green: 'green', red: 'red' };
const GROUP_COLORS = { blue: 'blue', purple: 'purple', pink: 'purple', green: 'green', gray: 'gray', white: 'gray', orange: 'orange', yellow: 'orange', red: 'orange' };
const color = (map, name, fallback) => map[(name ?? '').replace(/^vivid_/, '')] ?? fallback;
/** Font size in px → our note sizes. */
const noteSize = (px) => (!px ? 'md' : px <= 14 ? 'sm' : px <= 16 ? 'md' : px <= 24 ? 'lg' : 'xl');
/** Rows a group banner takes above the group's widgets. */
const BANNER_ROWS = 5;

export function convertDashboard(doc, integration, path, revision) {
  const banner = { type: 'image', title: integration.charAt(0).toUpperCase() + integration.slice(1).replace(/_/g, ' '), src: `/images/integrations/${integration}-banner.svg` };
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
        const children = widgets(d.widgets ?? [], item.layout?.width, true, d.title ?? '');
        if (d.banner_img) {
          // The banner spans the group above its widgets.
          for (const w of children) w.layout.y += BANNER_ROWS;
          // A banner embedded in the file is part of it; a linked one is Datadog's and becomes ours.
          const image = d.banner_img.startsWith('data:image/') ? { ...banner, src: d.banner_img } : banner;
          children.unshift({ id: `upstream-${item.id ?? ++seq}-banner`, definition: image, layout: { x: 0, y: 0, w: 12, h: BANNER_ROWS } });
        }
        definition = {
          type: 'group',
          title: d.title || (d.banner_img ? doc.title : ''),
          color: color(GROUP_COLORS, d.background_color, 'blue'),
          compact: false,
          ...(d.show_title === false ? { showTitle: false } : {}),
          widgets: children,
        };
      } else if (d.type === 'note') {
        definition = {
          type: 'note',
          content: d.content ?? '',
          background: color(NOTE_COLORS, d.background_color, 'white'),
          align: d.text_align === 'center' ? 'center' : 'left',
          fontSize: noteSize(Number(d.font_size)),
          ...(d.show_tick ? { pointer: true } : {}),
        };
      } else if (d.type === 'image') {
        // Embedded images are part of the BSD-licensed file; linked ones are Datadog's.
        definition = d.url?.startsWith('data:image/') ? { type: 'image', title: d.title ?? '', src: d.url } : banner;
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
        // Two rows for the group's header, unless it has none.
        const header = definition.showTitle === false ? 0 : 2;
        geometry.h = Math.max(geometry.h, header + Math.max(0, ...definition.widgets.map((w) => w.layout.y + w.layout.h)));
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

/** One CSV record's fields (RFC 4180 quoting: commas and doubled quotes inside quotes). */
function csvFields(line) {
  const fields = [];
  let field = '';
  let quoted = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (quoted) {
      if (c === '"' && line[i + 1] === '"') ((field += '"'), i++);
      else if (c === '"') quoted = false;
      else field += c;
    } else if (c === '"') quoted = true;
    else if (c === ',') (fields.push(field), (field = ''));
    else field += c;
  }
  fields.push(field);
  return fields;
}

/** [metric, unit] for every metric of a metadata.csv that has a unit; per-unit as "unit/per" (byte/second). */
export function metricUnits(csv) {
  const [header, ...rows] = csv.trim().split('\n');
  const cols = csvFields(header);
  const [name, unit, per] = ['metric_name', 'unit_name', 'per_unit_name'].map((c) => cols.indexOf(c));
  return rows
    .map(csvFields)
    .filter((f) => f[unit])
    .map((f) => [f[name], f[per] ? `${f[unit]}/${f[per]}` : f[unit]]);
}

/** [metric, description] for every metric of a metadata.csv with a description (its first line). */
export function metricDescriptions(csv) {
  const [header, ...rows] = csv.trim().split('\n');
  const cols = csvFields(header);
  const [name, description] = ['metric_name', 'description'].map((c) => cols.indexOf(c));
  return rows
    .map(csvFields)
    .filter((f) => /^[\w.]+$/.test(f[name] ?? '') && f[description]?.trim())
    .map((f) => [f[name], f[description].trim().replace(/\s+/g, ' ')]);
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
  const units = new Map(AGENT_UNITS[integration] ?? []);
  const descriptions = new Map();
  for (const source of UNIT_SOURCES[integration] ?? [integration]) {
    const metadata = await read(`${source}/metadata.csv`);
    for (const [metric, unit] of metricUnits(metadata)) if (!units.has(metric)) units.set(metric, unit);
    for (const [metric, text] of metricDescriptions(metadata)) if (!descriptions.has(metric)) descriptions.set(metric, text);
  }
  const target = new URL(`../server/seeds/integrations/${integration}/`, import.meta.url);
  await mkdir(target, { recursive: true });
  await writeFile(new URL('dashboards.json', target), JSON.stringify(dashboards, null, 2) + '\n');
  const sources = (UNIT_SOURCES[integration] ?? [integration]).map((s) => `${s}/metadata.csv`).join(', ');
  const header = `# Metric units from DataDog/integrations-core@${revision} (${sources}); BSD-3-Clause, see LICENSE. Agent host metrics (system.*) added here.\n`;
  await writeFile(
    new URL('units.csv', target),
    header +
      [...units]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([m, u]) => `${m},${u}`)
        .join('\n') +
      '\n',
  );
  await writeFile(
    new URL('descriptions.tsv', target),
    `# Metric descriptions from DataDog/integrations-core@${revision} (${sources}); BSD-3-Clause, see LICENSE.\n` +
      [...descriptions].sort(([a], [b]) => a.localeCompare(b)).map(([m, d]) => `${m}\t${d}`).join('\n') +
      '\n',
  );
  await writeFile(new URL('LICENSE', target), license);
  console.log(`Imported ${dashboards.length} upstream dashboards and ${units.size} metric units.`);
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
