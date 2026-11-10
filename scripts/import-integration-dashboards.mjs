// Import integrations-core dashboards (BSD-3-Clause): widgets, notes, colors and the images embedded
// in them. Images only linked from them are hosted by Datadog, not part of the repository, and are
// replaced by our own artwork (web/public/images/integrations/<integration>-banner.svg).
// node scripts/import-integration-dashboards.mjs [integration] [commit SHA]
// node scripts/import-integration-dashboards.mjs --all --source <integrations-core checkout> [--extras <integrations-extras checkout>]
// node scripts/import-integration-dashboards.mjs --mock   (only the mock subset, from the imported bundle)
import { existsSync } from 'node:fs';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { gunzipSync, gzipSync } from 'node:zlib';
import { pathToFileURL } from 'node:url';
import { formulaQueries, parseMetricQuery } from '../web/src/lib/metricQuery.ts';
// The last revision with every integration: from 9afe6c8 (2026-10-30) Datadog moves integrations to a private
// repository (integrations-internal-core), so later revisions lose catalog entries Datadog still offers.
export const PINNED_REVISION = '1e7420ec635aafdd33db6a1e69586de7d2e24c21';
// integrations-extras (community and partner integrations, BSD-3-Clause) at its last revision before the same move
// (d6ad10b, 2026-10-30, drops the listings Datadog migrated to its publishing platform).
export const EXTRAS_REVISION = '20d3ceeaf342b8c9f6211dc9df9ea3d382dbb692';

/** Units of the Agent's own host metrics (they have no integrations-core metadata.csv); memory is in MiB. */
const AGENT_UNITS = {
  kubernetes: [
    ['system.cpu.idle', 'percent'],
    ['system.cpu.iowait', 'percent'],
    ['system.cpu.system', 'percent'],
    ['system.cpu.user', 'percent'],
    ['system.cpu.num_cores', 'core'],
    ['system.mem.free', 'mebibyte'],
    ['system.mem.pct_usable', 'fraction'],
    ['system.mem.total', 'mebibyte'],
    ['system.mem.usable', 'mebibyte'],
    ['system.mem.used', 'mebibyte'],
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

export function convertDashboard(doc, integration, path, revision, repository = 'DataDog/integrations-core') {
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
          // Plain colors are pale headers with dark text; vivid_* (and no color) are saturated.
          ...(d.background_color && !d.background_color.startsWith('vivid_') ? { light: true } : {}),
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
      } else if (d.type === 'list_stream' && d.requests?.[0]?.query?.data_source === 'logs_stream') {
        // Log lists are our log stream (the integration's logs, newest first).
        definition = { type: 'log_stream', title: d.title ?? '', query: d.requests[0].query.query_string ?? '', limit: 50 };
      } else if (d.type === 'log_stream') {
        definition = { type: 'log_stream', title: d.title ?? '', query: d.query ?? '', limit: 50 };
      } else if (d.type === 'manage_status') {
        definition = { type: 'monitor_summary', title: d.title ?? '', query: d.query ?? '' };
      } else {
        // Preserve requests, formulas, units, thresholds and visualization options verbatim.
        const { banner_img, image_url, ...original } = d;
        definition = { type: 'integration', title: d.title ?? '', original };
      }
      const layout = item.layout;
      let geometry;
      if (layout) {
        const scale = cols / extent;
        // Both edges rounded, so widgets side by side keep meeting (3 + 5 of 8 columns is 5 + 7 of 12, not 5 + 8).
        const x = Math.round(layout.x * scale);
        geometry = {
          x,
          y: Math.round(layout.y * rowScale),
          w: Math.max(1, Math.round((layout.x + layout.width) * scale) - x),
          h: Math.max(4, Math.round(layout.height * rowScale)),
        };
      } else {
        const h = d.type === 'group' ? Math.max(8, Math.max(0, ...definition.widgets.map((w) => w.layout.y + w.layout.h)) + 2) : d.type === 'note' ? 3 : 6;
        geometry = { x: 0, y: cursor, w: cols, h };
      }
      if (definition.type === 'group') {
        // The rows the dashboard draws the group in (web/src/pages/dashboards/tree.ts groupHeight): its header, then its widgets.
        const header = definition.showTitle === false ? 1 : definition.compact ? 2 : 3;
        geometry.h = Math.max(geometry.h, header + Math.max(2, ...definition.widgets.map((w) => w.layout.y + w.layout.h)));
      }
      cursor = Math.max(cursor, geometry.y + geometry.h);
      return { id: `upstream-${item.id ?? ++seq}`, definition, layout: geometry };
    });
    // Group headers add space that was implicit in the upstream layout. Move
    // subsequent rows below those headers instead of relying on runtime overlap
    // correction, which otherwise changes the layout after the first paint.
    if (!free && !nested) {
      // Datadog draws an ordered dashboard in its list order, row after row: a widget joins the row when it starts
      // after the previous one and fits, else it starts a new row. The upstream y is for its column view.
      let row = [];
      let rowY = 0;
      let x = 0;
      const closeRow = () => {
        if (row.length > 1 && row.every((item) => item.definition.type === 'group')) {
          const h = Math.max(...row.map((item) => item.layout.h));
          for (const item of row) item.layout.h = h;
        }
        rowY = Math.max(rowY, ...row.map((item) => item.layout.y + item.layout.h));
        row = [];
        x = 0;
      };
      for (const item of converted) {
        const l = item.layout;
        if (row.length && (l.x < x || l.x + l.w > cols)) closeRow();
        l.y = rowY;
        x = l.x + l.w;
        row.push(item);
      }
      closeRow();
    } else if (nested) {
      // Inside a group, row by row (widgets sharing an upstream y), so a row moves down as one and stays aligned.
      // Widgets of one upstream y that overlap each other (after a column break) start a further row.
      const rows = [];
      for (const item of [...converted].sort((a, b) => a.layout.y - b.layout.y)) {
        const row = rows.findLast((r) => r.start === item.layout.y && !r.items.some(({ layout: b }) => item.layout.x < b.x + b.w && item.layout.x + item.layout.w > b.x));
        if (row) row.items.push(item);
        else rows.push({ start: item.layout.y, items: [item] });
      }
      const placed = [];
      const overlaps = (a, y, b) => a.x < b.x + b.w && a.x + a.w > b.x && y < b.y + b.h && y + a.h > b.y;
      for (const { start, items: row } of rows) {
        let y = start;
        for (;;) {
          const blocking = placed.filter(({ layout: b }) => row.some(({ layout: a }) => overlaps(a, y, b)));
          if (!blocking.length) break;
          y = Math.max(...blocking.map(({ layout: b }) => b.y + b.h));
        }
        for (const item of row) item.layout.y = y;
        // Groups of a row share its height (as Datadog draws them): the dashboard's upward compaction can't
        // pull the next row's left part above its right part.
        if (row.length > 1 && row.every((item) => item.definition.type === 'group')) {
          const h = Math.max(...row.map((item) => item.layout.h));
          for (const item of row) item.layout.h = h;
        }
        placed.push(...row);
      }
    }
    return converted;
  }
  return {
    id: `${integration}-${path
      .split('/')
      .at(-1)
      .replace(/\.json$/, '')
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-|-$/g, '')}`,
    title: doc.title,
    description: `Built-in ${integration} dashboard from integrations-core.`,
    kind: 'integration',
    logo: integration,
    teams: [],
    layoutType: free ? 'free' : 'grid',
    templateVariables: (doc.template_variables ?? []).map((v) => ({ name: v.name, prefix: v.prefix ?? '', default: v.default ?? '*' })),
    widgets: widgets(doc.widgets ?? [], undefined),
    source: { repository, revision, path: `${integration}/${path}`, license: 'BSD-3-Clause' },
  };
}

const COMPARATORS = { '>': 'above', '>=': 'above_or_equal', '<': 'below', '<=': 'below_or_equal' };

/**
 * A recommended monitor (assets/monitors/*.json `definition`) as a monitor template, or null when its
 * query is neither a metric query nor arithmetic between metric queries (`a - b`, `a * 100 / b`).
 */
export function convertMonitor(def, integration, title) {
  const m = /^(?:change\((\w+)\(last_(\d+)m\),last_(\d+)m\)|(\w+)\(last_(\d+)m\)):(.+?)\s*(>=|<=|>|<)\s*(-?[\d.]+)$/.exec(def.query?.trim() ?? '');
  if (!m) return null;
  const [, changeAgg, changeWindow, changeShift, agg, window, rawExpr, op] = m;
  // default_zero() around one query: no data counts as 0, which a threshold above zero treats like no data
  // anyway. Around a formula it stays (a ratio's missing values matter there).
  const single = rawExpr.replace(/^default_zero\((.*)\)$/, '$1').trim();
  const expr = parseMetricQuery(single) ? single : rawExpr.trim();
  if (!formulaQueries(expr)) return null;
  const t = def.options?.thresholds ?? {};
  const change = Boolean(changeAgg);
  return {
    id: `${integration}-${title
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-|-$/g, '')}`,
    source: integration.toUpperCase().replace(/_/g, ' '),
    title,
    description: `Recommended ${integration} monitor from integrations-core.`,
    type: 'metric',
    name: def.name,
    query: expr,
    options: {
      detection: change ? 'change' : 'threshold',
      timeAggregation: change ? changeAgg : agg,
      windowMinutes: Number(change ? changeWindow : window),
      comparator: COMPARATORS[op],
      thresholds: { critical: t.critical ?? null, warning: t.warning ?? null, criticalRecovery: t.critical_recovery ?? null, warningRecovery: t.warning_recovery ?? null },
      notifyNoData: false,
      noDataMinutes: 10,
      changeMode: change ? 'change' : null,
      changeMinutes: change ? Number(changeShift) : null,
    },
    message: def.message ?? '',
    keywords: [
      integration,
      ...title
        .toLowerCase()
        .split(/\W+/)
        .filter((w) => w.length > 2),
    ],
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

/** Integrations with a bundle of their own (Kubernetes, the clouds): not repeated in the core bundle. */
const OWN_BUNDLES = new Set(['kubernetes', 'aws', 'azure', 'gcp']);
/** Integrations the mock shows installed: their dashboards are bundled with the UI for `make web-mock`. */
const MOCK_INTEGRATIONS = [
  'postgres',
  'mysql',
  'redisdb',
  'nginx',
  'kafka',
  'mongo',
  'elastic',
  'rabbitmq',
  'cisco-asa',
  'traefik-mesh',
  'bind9',
  'gitea',
  'emqx',
  'vault',
  'gatekeeper',
  'external-secrets',
  'argocd',
  'cert-manager',
  'supply-chain-firewall',
];

/** Our banner for the integration (web/public/images/integrations/<id>-banner.svg), or null. */
export function bannerFor(id) {
  const banner = `/images/integrations/${id}-banner.svg`;
  return existsSync(new URL(`../web/public${banner}`, import.meta.url)) ? banner : null;
}

/** Points the converted dashboard's images at our banner, or at the integration's logo (the image widget draws it). */
function withArtwork(widgets, id, name) {
  const banner = bannerFor(id);
  return widgets.map((w) => {
    const d = w.definition;
    if (d.type === 'group') return { ...w, definition: { ...d, widgets: withArtwork(d.widgets, id, name) } };
    if (d.type !== 'image' || d.src?.startsWith('data:')) return w;
    return { ...w, definition: banner ? { ...d, src: banner } : { type: 'image', title: name, logo: id, background: '#ffffff' } };
  });
}

/** Every catalog integration's dashboards from a checkout: one entry per integration with its metric prefix. */
export async function coreBundle(source, catalog, revision = PINNED_REVISION, extras = null) {
  const bundle = [];
  for (const c of catalog.integrations) {
    if (OWN_BUNDLES.has(c.id)) continue;
    const fromExtras = c.repository === 'integrations-extras';
    if (fromExtras && !extras) continue;
    const dir = join(fromExtras ? extras : source, c.check);
    const [rev, repository] = fromExtras ? [EXTRAS_REVISION, 'DataDog/integrations-extras'] : [revision, 'DataDog/integrations-core'];
    if (!existsSync(join(dir, 'manifest.json'))) continue;
    const manifest = JSON.parse(await readFile(join(dir, 'manifest.json'), 'utf8'));
    const dashboards = [];
    for (const path of new Set(Object.values(manifest.assets?.dashboards ?? {}).map((p) => p.replace(/^\.\//, '')))) {
      if (!path.startsWith('assets/dashboards/') || !path.endsWith('.json') || path.includes('..') || !existsSync(join(dir, path))) continue;
      const d = convertDashboard(JSON.parse(await readFile(join(dir, path), 'utf8')), c.check, path, rev, repository);
      dashboards.push({ ...d, logo: c.id, widgets: withArtwork(d.widgets, c.id, c.name) });
    }
    if (dashboards.length) bundle.push({ logo: c.id, prefix: c.metricPrefix ?? '', dashboards });
  }
  return bundle.sort((a, b) => a.logo.localeCompare(b.logo));
}

/** The dashboards of the integrations the mock shows installed, with the units of the metrics they chart (the server reads units.csv). */
async function writeMockSubset(bundle, units) {
  const dashboards = bundle.filter((b) => MOCK_INTEGRATIONS.includes(b.logo)).flatMap((b) => b.dashboards);
  const charted = new Set([...JSON.stringify(dashboards).matchAll(/[a-z]+:([a-z][a-z0-9_]*\.[a-z0-9_.]+)\{/g)].map((m) => m[1]));
  const mockUnits = Object.fromEntries(
    [...charted]
      .filter((m) => units.has(m))
      .sort()
      .map((m) => [m, units.get(m)]),
  );
  await writeFile(new URL('../web/src/api/integrations/coreDashboards.generated.json', import.meta.url), JSON.stringify({ dashboards, units: mockUnits }) + '\n');
}

/** `--mock`: the mock subset again from the imported bundle and units.csv (after changing MOCK_INTEGRATIONS), without a checkout. */
async function importMock() {
  const target = new URL('../server/seeds/integrations/core/', import.meta.url);
  const bundle = gunzipSync(await readFile(new URL('dashboards.ndjson.gz', target)))
    .toString('utf8')
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line));
  const units = new Map(
    (await readFile(new URL('units.csv', target), 'utf8'))
      .split('\n')
      .filter((l) => l && !l.startsWith('#'))
      .map((l) => [l.slice(0, l.indexOf(',')), l.slice(l.indexOf(',') + 1)]),
  );
  await writeMockSubset(bundle, units);
}

/** `--all --source <checkout> [--extras <checkout>]`: the bundle (gzip NDJSON), the units of every metric and the mock subset. */
async function importAll(source, revision, extras) {
  const license = await readFile(join(source, 'LICENSE'), 'utf8');
  const extrasLicense = extras ? await readFile(join(extras, 'LICENSE'), 'utf8') : null;
  for (const l of [license, extrasLicense].filter(Boolean)) if (!l.includes('Redistribution and use in source and binary forms')) throw new Error('Upstream license changed; review before importing');
  const catalog = JSON.parse(await readFile(new URL('../web/src/api/integrations/catalog.generated.json', import.meta.url), 'utf8'));
  const bundle = await coreBundle(source, catalog, revision, extras);
  const target = new URL('../server/seeds/integrations/core/', import.meta.url);
  await mkdir(target, { recursive: true });
  await writeFile(new URL('dashboards.ndjson.gz', target), gzipSync(bundle.map((b) => JSON.stringify(b)).join('\n') + '\n', { level: 9 }));
  const units = new Map();
  for (const c of catalog.integrations) {
    if (c.repository === 'integrations-extras' && !extras) continue;
    const csv = join(c.repository === 'integrations-extras' ? extras : source, c.check, 'metadata.csv');
    if (existsSync(csv)) for (const [metric, unit] of metricUnits(await readFile(csv, 'utf8'))) if (!units.has(metric) && /^[\w.]+$/.test(metric)) units.set(metric, unit);
  }
  await writeFile(
    new URL('units.csv', target),
    `# Metric units from DataDog/integrations-core@${revision}${extras ? ` and DataDog/integrations-extras@${EXTRAS_REVISION}` : ''} (every integration's metadata.csv); BSD-3-Clause, see LICENSE${extras ? ' and LICENSE-integrations-extras' : ''}.\n` +
      [...units]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([m, u]) => `${m},${u}`)
        .join('\n') +
      '\n',
  );
  await writeFile(new URL('LICENSE', target), license);
  if (extrasLicense) await writeFile(new URL('LICENSE-integrations-extras', target), extrasLicense);
  await writeMockSubset(bundle, units);
  console.log(`Imported ${bundle.reduce((n, b) => n + b.dashboards.length, 0)} dashboards of ${bundle.length} integrations and ${units.size} metric units.`);
}

async function main() {
  if (process.argv.includes('--mock')) return importMock();
  if (process.argv.includes('--all')) {
    const source = process.argv[process.argv.indexOf('--source') + 1];
    if (!process.argv.includes('--source') || !source) throw new Error('usage: --all --source <integrations-core checkout> [--extras <integrations-extras checkout>]');
    const extras = process.argv.includes('--extras') ? process.argv[process.argv.indexOf('--extras') + 1] : null;
    return importAll(source, PINNED_REVISION, extras);
  }
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
  const monitors = [];
  const skipped = [];
  for (const [title, path] of Object.entries(manifest.assets?.monitors ?? {})) {
    if (!/^assets\/monitors\/[a-zA-Z0-9_-]+\.json$/.test(path)) throw new Error(`Unexpected asset path: ${path}`);
    const template = convertMonitor(JSON.parse(await read(`${integration}/${path}`)).definition, integration, title);
    if (template) monitors.push(template);
    else skipped.push(title);
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
      [...descriptions]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([m, d]) => `${m}\t${d}`)
        .join('\n') +
      '\n',
  );
  await writeFile(new URL('LICENSE', target), license);
  const templates = new URL(`../web/src/pages/monitors/${integration}Templates.generated.ts`, import.meta.url);
  await writeFile(
    templates,
    `// GENERATED by scripts/import-integration-dashboards.mjs from DataDog/integrations-core@${revision}\n` +
      `// (${integration}/assets/monitors, BSD-3-Clause, see server/seeds/integrations/${integration}/LICENSE). Do not edit.\n` +
      (skipped.length ? `// Not expressible as one monitor query (arithmetic between queries): ${skipped.join('; ')}.\n` : '') +
      `import type { MonitorTemplate } from './templates';\n\nexport const ${integration.toUpperCase()}_TEMPLATES: MonitorTemplate[] = ${JSON.stringify(monitors, null, 2)};\n`,
  );
  console.log(`Imported ${dashboards.length} upstream dashboards, ${monitors.length} monitors (${skipped.length} skipped) and ${units.size} metric units.`);
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
