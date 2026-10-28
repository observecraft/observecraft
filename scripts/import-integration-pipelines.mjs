// Import integrations-core log pipelines (BSD-3-Clause, <integration>/assets/logs/<source>.yaml) into
// server/seeds/integrations/<integration>/pipeline.json, which Logs → Pipelines offers to install.
// node scripts/import-integration-pipelines.mjs [integration …]   (default: every one listed below)
import { createRequire } from 'node:module';
import { mkdir, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { PINNED_REVISION } from './import-integration-dashboards.mjs';

const yaml = createRequire(new URL('../web/package.json', import.meta.url))('js-yaml');

/** Integration directory → its pipeline file under assets/logs/. */
export const PIPELINE_SOURCES = {
  nginx: 'nginx.yaml',
  nginx_ingress_controller: 'nginx-ingress-controller.yaml',
  redisdb: 'redis.yaml',
  postgres: 'postgresql.yaml',
  mysql: 'mysql.yaml',
  etcd: 'etcd.yaml',
  coredns: 'coredns.yaml',
  kube_scheduler: 'kube_scheduler.yaml',
};

/** OCSF sub-pipelines map logs onto Datadog's security schema (schema-processor), which we don't have. */
const isOcsf = (p) => p.type === 'pipeline' && p.ocsf?.isOcsf === true;

/** Drops what doesn't apply here (OCSF sub-pipelines) from a processor list, recursively. */
function processors(list) {
  return (list ?? []).filter((p) => !isOcsf(p)).map((p) => (p.type === 'pipeline' ? { ...p, processors: processors(p.processors) } : p));
}

/** An integrations-core pipeline document (parsed YAML) as an ObserveCraft integration pipeline. */
export function convertPipeline(doc, integration, path, revision) {
  const p = doc.pipeline;
  if (p?.type !== 'pipeline') throw new Error(`${path}: no pipeline`);
  return {
    id: integration,
    name: p.name,
    filter: p.filter?.query ?? '',
    processors: processors(p.processors),
    source: { repository: 'DataDog/integrations-core', revision, path: `${integration}/${path}`, license: 'BSD-3-Clause' },
  };
}

async function main() {
  const wanted = process.argv.length > 2 ? process.argv.slice(2) : Object.keys(PIPELINE_SOURCES);
  const base = `https://raw.githubusercontent.com/DataDog/integrations-core/${PINNED_REVISION}/`;
  async function read(path) {
    const response = await fetch(base + path);
    if (!response.ok) throw new Error(`${path}: HTTP ${response.status}`);
    return response.text();
  }
  const license = await read('LICENSE');
  if (!license.includes('Neither the name of integrations-core') || !license.includes('Redistribution and use in source and binary forms'))
    throw new Error('Upstream license changed; review before importing');
  for (const integration of wanted) {
    const file = PIPELINE_SOURCES[integration];
    if (!file) throw new Error(`No pipeline source listed for ${integration}`);
    const path = `assets/logs/${file}`;
    const pipeline = convertPipeline(yaml.load(await read(`${integration}/${path}`)), integration, path, PINNED_REVISION);
    const target = new URL(`../server/seeds/integrations/${integration}/`, import.meta.url);
    await mkdir(target, { recursive: true });
    await writeFile(new URL('pipeline.json', target), JSON.stringify(pipeline, null, 2) + '\n');
    await writeFile(new URL('LICENSE', target), license);
    console.log(`${integration}: ${pipeline.processors.length} processors`);
  }
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
