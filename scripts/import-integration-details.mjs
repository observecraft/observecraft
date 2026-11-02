// Import every catalog integration's details from integrations-core (BSD-3-Clause, kept as they are): the README
// split into the modal's tabs, the Dive Deeper links, metadata.csv, the log pipeline's facets, service checks,
// monitor templates, dashboard titles and the latest CHANGELOG entries, into
// web/src/api/integrations/details/<id>.json (one file per integration, loaded when its modal opens).
// node scripts/import-integration-details.mjs --source <integrations-core checkout at PINNED_REVISION>
import { existsSync } from 'node:fs';
import { mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { PINNED_REVISION } from './import-integration-dashboards.mjs';

const yaml = createRequire(new URL('../web/package.json', import.meta.url))('js-yaml');

/** The README's `[n]: url` link definitions used by a section, appended to it so its links resolve on their own. */
function withLinks(section, definitions) {
  const used = new Set([...section.matchAll(/\]\[([^\]]+)\]/g)].map((m) => m[1]));
  const lines = [...used].filter((id) => definitions.has(id)).map((id) => `[${id}]: ${definitions.get(id)}`);
  return [section.trim(), lines.join('\n')].filter(Boolean).join('\n\n');
}

/** The README's top-level (`##`) sections the modal shows: Overview, Setup (Configure), Troubleshooting and Further Reading (Support). */
export function readmeSections(readme) {
  const definitions = new Map();
  const body = readme
    .replace(/^\[([^\]]+)\]:\s*(\S+).*$/gm, (_, id, url) => {
      definitions.set(id, url);
      return '';
    })
    .replace(/^# .*\n/, '');
  const parts = body.split(/^## +/m);
  if (parts.length === 1) return { overview: withLinks(body, definitions), setup: '', troubleshooting: '', furtherReading: '' };
  const section = (pattern) => {
    const part = parts.find((p) => pattern.test(p.split('\n')[0]));
    return part ? withLinks(part.slice(part.indexOf('\n') + 1), definitions) : '';
  };
  return { overview: section(/^Overview/i), setup: section(/^Setup/i), troubleshooting: section(/^Troubleshooting/i), furtherReading: section(/^Further Reading/i) };
}

/** Datadog's documentation shortcodes (Hugo) as plain Markdown: US-site content, our site in place of Datadog's. */
export function resolveShortcodes(md) {
  return md
    .replace(/\{\{< site-region region="([^"]*)" >\}\}\n?([\s\S]*?)\n?\{\{< \/site-region >\}\}/g, (_, regions, content) => (regions.split(',').includes('us') ? content : ''))
    .replace(/\{\{% site-region region="([^"]*)" %\}\}\n?([\s\S]*?)\n?\{\{% \/site-region %\}\}/g, (_, regions, content) => (regions.split(',').includes('us') ? content : ''))
    .replace(/\{\{< region-param key="?dd_site"?[^>]*>\}\}/g, '`<OBSERVECRAFT_SITE>`')
    .replace(/\{\{< region-param [^>]*>\}\}/g, '`<OBSERVECRAFT_SITE>`')
    .replace(/\{\{< get-metrics-from-git [^>]*>\}\}/g, 'See the Data Collected tab.')
    .replace(/\{\{% collapse-content title="([^"]*)"[^%]*%\}\}\n?/g, '**$1**\n\n')
    .replace(/\n?\{\{[<%] \/[\w-]+ [>%]\}\}/g, '')
    .replace(/\{\{[<%] [\w-]+[^>%]*[>%]\}\}\n?/g, '');
}

/** The HTML some READMEs use (alerts, links, images, line breaks) as Markdown; code spans and blocks stay as they are. */
export function htmlToMarkdown(md) {
  const convert = (text) =>
    text
      .replace(/<div class="alert alert-(\w+)">\s*([\s\S]*?)\s*<\/div>/g, (_, kind, body) => `> **${kind === 'warning' || kind === 'danger' ? 'Warning' : 'Note'}:** ${body.replace(/\s*\n\s*/g, ' ')}`)
      .replace(/<a [^>]*href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/g, '[$2]($1)')
      .replace(/<img\s[^>]*>/g, (tag) => `![${/alt="([^"]*)"/.exec(tag)?.[1] ?? ''}](${/src="([^"]+)"/.exec(tag)?.[1] ?? ''})`)
      .replace(/<br\s*\/?>/g, '  \n')
      .replace(/<\/?(strong|b)>/g, '**')
      .replace(/<\/?(em|i)>/g, '_')
      .replace(/<\/?code>/g, '`')
      .replace(/<\/?(p|div|span|center|sup|sub)(\s[^>]*)?>/g, '');
  // Leave fenced blocks and inline code untouched.
  return md
    .split(/(```[\s\S]*?```|`[^`\n]*`)/)
    .map((part, i) => (i % 2 ? part : convert(part)))
    .join('');
}

/** Splits one CSV line set into rows of fields (quoted fields may hold commas, quotes and line breaks). */
function csvRows(text) {
  const rows = [];
  let row = [];
  let field = '';
  let quoted = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quoted) {
      if (c === '"' && text[i + 1] === '"') ((field += '"'), i++);
      else if (c === '"') quoted = false;
      else field += c;
    } else if (c === '"') quoted = true;
    else if (c === ',') (row.push(field), (field = ''));
    else if (c === '\n' || c === '\r') {
      if (c === '\r' && text[i + 1] === '\n') i++;
      row.push(field);
      rows.push(row);
      row = [];
      field = '';
    } else field += c;
  }
  if (field || row.length) (row.push(field), rows.push(row));
  return rows.filter((r) => r.some(Boolean));
}

/** metadata.csv: every metric's name, type, unit (and per-unit) and description. */
export function metadataCsv(text) {
  const [header, ...rows] = csvRows(text.replace(/^﻿/, ''));
  const col = (name) => header.indexOf(name);
  return rows.map((r) => {
    const m = { name: r[col('metric_name')], type: r[col('metric_type')] };
    if (r[col('unit_name')]) m.unit = r[col('unit_name')];
    if (r[col('per_unit_name')]) m.perUnit = r[col('per_unit_name')];
    m.description = r[col('description')] ?? '';
    return m;
  });
}

/** The facets an integration's log pipeline declares (assets/logs/<source>.yaml). */
export function logFacets(text) {
  const doc = yaml.load(text);
  return (doc?.facets ?? []).map((f) => ({ name: f.name, path: f.path, facetType: f.facetType ?? 'list', group: f.groups?.[0] ?? '' }));
}

/** A recommended monitor (assets/monitors/*.json) with the type Datadog's modal names. */
export function monitorTemplate(doc) {
  const { type, query = '' } = doc.definition ?? {};
  const kind = /anomalies\(/.test(query)
    ? 'Anomaly Monitor'
    : /forecast\(/.test(query)
      ? 'Forecast Monitor'
      : /outliers\(/.test(query)
        ? 'Outlier Monitor'
        : ({
            'query alert': 'Metric Monitor',
            'log alert': 'Log Monitor',
            'service check': 'Service Check Monitor',
            'event-v2 alert': 'Event Monitor',
            'event alert': 'Event Monitor',
            'process alert': 'Process Monitor',
            'trace-analytics alert': 'APM Monitor',
            composite: 'Composite Monitor',
          }[type] ?? 'Monitor');
  return { title: doc.title ?? doc.definition?.name ?? '', description: doc.description ?? '', type: kind, query };
}

/** The latest `limit` releases of a CHANGELOG.md: version, date and notes (Markdown). */
export function changelog(md, limit) {
  const entries = md.split(/^## +/m).slice(1);
  return entries.slice(0, limit).map((e) => {
    const [title, ...rest] = e.split('\n');
    const [version, date = ''] = title.split(' / ').map((x) => x.trim());
    return { version, date, notes: rest.join('\n').trim() };
  });
}

const read = (path) => readFile(path, 'utf8').catch(() => undefined);
const json = async (path) => {
  const text = await read(path);
  return text === undefined ? undefined : JSON.parse(text);
};

/** One integration directory of the checkout → its details. */
export async function integrationDetails(dir, id, revision = PINNED_REVISION) {
  const manifest = (await json(join(dir, 'manifest.json'))) ?? {};
  const assets = manifest.assets ?? {};
  const readme = htmlToMarkdown(resolveShortcodes((await read(join(dir, 'README.md'))) ?? ''));
  const sections = readmeSections(readme);
  const metadata = await read(join(dir, assets.integration?.metrics?.metadata_path ?? 'metadata.csv'));
  const facets = [];
  const logsDir = join(dir, 'assets/logs');
  if (existsSync(logsDir))
    for (const f of (await readdir(logsDir)).filter((f) => f.endsWith('.yaml') && !f.endsWith('_tests.yaml')).sort()) facets.push(...logFacets(await readFile(join(logsDir, f), 'utf8')));
  const dashboards = [];
  for (const [key, path] of Object.entries(assets.dashboards ?? {})) {
    const doc = await json(join(dir, path));
    if (doc) dashboards.push({ key, title: doc.title ?? key, file: path });
  }
  const monitors = [];
  for (const path of Object.values(assets.monitors ?? {})) {
    const doc = await json(join(dir, path));
    if (doc) monitors.push(monitorTemplate(doc));
  }
  const checks = (await json(join(dir, assets.integration?.service_checks?.metadata_path ?? 'assets/service_checks.json'))) ?? [];
  return {
    source: `DataDog/integrations-core@${revision} ${dir.split('/').pop()}`,
    overview: sections.overview,
    setup: sections.setup,
    troubleshooting: sections.troubleshooting,
    furtherReading: sections.furtherReading,
    resources: (manifest.tile?.resources ?? []).map((r) => ({ type: r.resource_type, url: r.url })),
    metrics: metadata ? metadataCsv(metadata) : [],
    logFacets: facets,
    serviceChecks: checks.map((c) => ({ check: c.check, name: c.name, statuses: c.statuses ?? [], description: c.description ?? '' })),
    monitors,
    dashboards,
    releaseNotes: changelog((await read(join(dir, 'CHANGELOG.md'))) ?? '', 15),
  };
}

async function main() {
  const args = process.argv.slice(2);
  const source = args[args.indexOf('--source') + 1];
  if (!args.includes('--source') || !source) throw new Error('usage: node scripts/import-integration-details.mjs --source <integrations-core checkout>');
  const catalog = JSON.parse(await readFile(new URL('../web/src/api/integrations/catalog.generated.json', import.meta.url), 'utf8'));
  const out = new URL('../web/src/api/integrations/details/', import.meta.url);
  await rm(out, { recursive: true, force: true });
  await mkdir(out, { recursive: true });
  // Catalog ids are the integrations-core directory names with dashes (manifest app_id); the directory is the check.
  for (const c of catalog.integrations) {
    const dir = join(source, c.check);
    if (!existsSync(join(dir, 'manifest.json'))) continue;
    await writeFile(new URL(`${c.id}.json`, out), `${JSON.stringify(await integrationDetails(dir, c.id))}\n`);
  }
  console.log(`Details of ${catalog.integrations.length} integrations written to ${out.pathname}`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) await main();
