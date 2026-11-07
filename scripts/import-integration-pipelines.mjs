// Import every catalog integration's integrations-core log pipelines (BSD-3-Clause, <integration>/assets/logs/*.yaml)
// into server/seeds/integrations/core/pipelines.json, which Logs → Pipelines offers to install, and the mock's subset.
// node scripts/import-integration-pipelines.mjs --source <integrations-core checkout at PINNED_REVISION> [--extras <integrations-extras checkout at EXTRAS_REVISION>]
import { createRequire } from 'node:module';
import { mkdir, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { EXTRAS_REVISION, PINNED_REVISION } from './import-integration-dashboards.mjs';

const yaml = createRequire(new URL('../web/package.json', import.meta.url))('js-yaml');

/** The pipelines the in-browser mock offers (web/src/api/pipelines/library.generated.json); the server offers them all. */
export const MOCK_PIPELINES = ['coredns', 'etcd', 'kube_scheduler', 'mysql', 'nginx', 'nginx_ingress_controller', 'postgres', 'redisdb'];

/** OCSF sub-pipelines map logs onto Datadog's security schema (schema-processor), which we don't have. */
const isOcsf = (p) => p.type === 'pipeline' && p.ocsf?.isOcsf === true;

/** Drops what doesn't apply here (OCSF sub-pipelines) from a processor list, recursively, in the API's shape. */
function processors(list) {
  return (list ?? [])
    .filter((p) => !isOcsf(p))
    .map((p) => (p.type === 'pipeline' ? { ...p, processors: processors(p.processors) } : p))
    .map((p) =>
      // YAML files write a lookup table as one block of `key,value` lines; Datadog's API stores a list of them.
      typeof p.lookupTable === 'string'
        ? {
            ...p,
            lookupTable: p.lookupTable
              .split('\n')
              .map((l) => l.trim())
              .filter(Boolean),
          }
        : p,
    );
}

/** An integrations-core pipeline document (parsed YAML) as an ObserveCraft integration pipeline. */
export function convertPipeline(doc, integration, path, revision, repository = 'DataDog/integrations-core') {
  const p = doc.pipeline;
  if (p?.type !== 'pipeline') throw new Error(`${path}: no pipeline`);
  return {
    id: integration,
    name: p.name,
    filter: p.filter?.query ?? '',
    processors: processors(p.processors),
    source: { repository, revision, path: `${integration}/${path}`, license: 'BSD-3-Clause' },
  };
}

/** Every catalog integration's pipelines into one library file (core/pipelines.json, next to its LICENSE). */
async function importAll(source, extras) {
  const { existsSync, readdirSync } = await import('node:fs');
  const { readFile } = await import('node:fs/promises');
  const { join } = await import('node:path');
  const catalog = JSON.parse(await readFile(new URL('../web/src/api/integrations/catalog.generated.json', import.meta.url), 'utf8'));
  const pipelines = [];
  const skipped = [];
  for (const c of catalog.integrations) {
    const fromExtras = c.repository === 'integrations-extras';
    if (fromExtras && !extras) continue;
    const dir = join(fromExtras ? extras : source, c.check, 'assets/logs');
    if (!existsSync(dir)) continue;
    const files = readdirSync(dir)
      .filter((f) => f.endsWith('.yaml') && !f.endsWith('_tests.yaml'))
      .sort();
    for (const file of files) {
      try {
        const p = convertPipeline(
          yaml.load(await readFile(join(dir, file), 'utf8')),
          c.check,
          `assets/logs/${file}`,
          fromExtras ? EXTRAS_REVISION : PINNED_REVISION,
          fromExtras ? 'DataDog/integrations-extras' : 'DataDog/integrations-core',
        );
        // One id per pipeline: the integration's for its own (its only file, or the one named after it), the file's for others.
        const stem = file.replace(/\.yaml$/, '');
        const own = files.length === 1 || stem.replace(/-/g, '_') === c.check;
        const { name, filter, processors, source } = p;
        pipelines.push({ id: own ? c.check : `${c.check}-${stem.replace(/[^a-z0-9]+/gi, '-').toLowerCase()}`, integration: c.id, name, filter, processors, source });
      } catch (e) {
        skipped.push(`${c.check}/${file}: ${e.message}`);
      }
    }
  }
  const target = new URL('../server/seeds/integrations/core/', import.meta.url);
  await mkdir(target, { recursive: true });
  await writeFile(new URL('pipelines.json', target), JSON.stringify(pipelines) + '\n');
  const mock = pipelines.filter((p) => MOCK_PIPELINES.includes(p.id));
  await writeFile(new URL('../web/src/api/pipelines/library.generated.json', import.meta.url), JSON.stringify(mock, null, 2) + '\n');
  console.log(`${pipelines.length} pipelines${skipped.length ? `; skipped:\n  ${skipped.join('\n  ')}` : ''}`);
}

async function main() {
  const source = process.argv[process.argv.indexOf('--source') + 1];
  if (!process.argv.includes('--source') || !source) throw new Error('usage: node scripts/import-integration-pipelines.mjs --source <integrations-core checkout>');
  const { readFile } = await import('node:fs/promises');
  const extras = process.argv.includes('--extras') ? process.argv[process.argv.indexOf('--extras') + 1] : null;
  for (const dir of [source, extras].filter(Boolean))
    if (!(await readFile(`${dir}/LICENSE`, 'utf8')).includes('Redistribution and use in source and binary forms')) throw new Error('Upstream license changed; review before importing');
  await importAll(source, extras);
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
