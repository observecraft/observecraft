// The metrics AWS, Azure and Google Cloud services publish, for their dashboards (scripts/cloud-service-dashboards.mjs).
// AWS: aws-cdk-lib's generated canned metrics (Apache-2.0): every metric's namespace, name, statistic and dimensions.
// Azure: Azure Monitor's supported metrics reference (MicrosoftDocs/azure-monitor-docs, CC BY 4.0): every resource
// type's metrics, default aggregation and unit. Google Cloud: Cloud Monitoring's metrics list and monitored resources
// (cloud.google.com/monitoring/api, CC BY 4.0): every metric type, its kind, value type, unit and resource.
// Metric names, units and aggregations are facts; no descriptions are copied.
// node scripts/import-cloud-metrics.mjs --aws <extracted aws-cdk-lib package directory> [--aws-docs <a.json,b.json>]
//   --azure <supported-metrics directory> --azure-revision <commit>
//   --gcp <directory of the metrics_gcp_*.html, metrics_kubernetes.html and resources.html pages>
import { readFileSync, readdirSync, writeFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const OUT = new URL('../server/seeds/integrations/cloud/', import.meta.url);

/** One canned-metrics module: `{namespace: {metric: {statistic, dimension sets}}}` from its .js and .d.ts. */
export function cannedMetrics(js, dts) {
  const out = {};
  // static invocationsSum(dimensions){return{namespace:"AWS/Lambda",metricName:"Invocations",dimensionsMap:dimensions,statistic:"Sum"}}
  const methods = new Map();
  for (const [, method, namespace, name, statistic] of js.matchAll(/static (\w+)\(dimensions\)\{return\{namespace:"([^"]+)",metricName:"([^"]+)",dimensionsMap:dimensions,statistic:"([^"]+)"\}\}/g))
    methods.set(method, { namespace, name, statistic });
  // static invocationsSum(this: void, dimensions: { FunctionName: string; Resource: string; }): …
  const dims = new Map();
  for (const [, method, body] of dts.matchAll(/static (\w+)\(this: void, dimensions: \{([^}]*)\}\)/g)) {
    const keys = [...body.matchAll(/(\w+): string;/g)].map((m) => m[1]);
    if (!dims.has(method)) dims.set(method, []);
    dims.get(method).push(keys);
  }
  for (const [method, m] of methods) {
    out[m.namespace] ??= {};
    out[m.namespace][m.name] ??= { statistic: m.statistic, dimensionSets: [] };
    out[m.namespace][m.name].dimensionSets.push(...(dims.get(method) ?? []));
  }
  return out;
}

/** The namespace's resource dimension: the one its metrics are most often published by alone, else the one most
 *  of their smallest dimension sets start with (S3's BucketName, published with StorageType). */
function resourceDimension(metrics) {
  const tally = (pick) => {
    const counts = new Map();
    for (const m of Object.values(metrics)) {
      const key = pick(m.dimensionSets);
      if (key) counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    return [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))[0]?.[0] ?? null;
  };
  return (
    tally((sets) => sets.find((s) => s.length === 1)?.[0]) ??
    tally(
      (sets) =>
        sets
          .filter((s) => s.length)
          .sort((a, b) => a.length - b.length)[0]
          ?.slice()
          .sort()[0],
    )
  );
}

/** Namespaces aws-cdk-lib doesn't cover, from each service's CloudWatch metrics page of the AWS documentation
 *  (gathered into `{namespace: {url, dimension, metrics: [{name, statistic, dimensions}]}}` files): names only. */
function awsDocs(files) {
  const out = {};
  for (const f of files)
    for (const [ns, spec] of Object.entries(JSON.parse(readFileSync(f, 'utf8'))))
      if (spec.metrics?.length)
        out[ns] = {
          dimension: spec.dimension ?? null,
          doc: spec.url,
          metrics: spec.metrics
            .filter((m, i, a) => a.findIndex((o) => o.name === m.name) === i)
            .map((m) => ({ name: m.name, statistic: ['Sum', 'Maximum'].includes(m.statistic) ? m.statistic : 'Average', dimensions: m.dimensions ?? [] })),
        };
  return out;
}

export function awsMetrics(packageDir, docs = []) {
  const version = JSON.parse(readFileSync(join(packageDir, 'package.json'), 'utf8')).version;
  const namespaces = {};
  for (const dir of readdirSync(packageDir).filter((d) => d.startsWith('aws-'))) {
    const lib = join(packageDir, dir, 'lib');
    if (!existsSync(lib)) continue;
    for (const f of readdirSync(lib).filter((f) => f.endsWith('canned-metrics.generated.js'))) {
      const parsed = cannedMetrics(readFileSync(join(lib, f), 'utf8'), readFileSync(join(lib, f.replace(/\.js$/, '.d.ts')), 'utf8'));
      for (const [ns, metrics] of Object.entries(parsed)) Object.assign((namespaces[ns] ??= {}), metrics);
    }
  }
  const out = {};
  for (const ns of Object.keys(namespaces).sort()) {
    const metrics = namespaces[ns];
    const dimension = resourceDimension(metrics);
    out[ns] = {
      dimension,
      metrics: Object.entries(metrics)
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([name, m]) => ({
          name,
          statistic: m.statistic,
          // Its smallest set of dimensions naming a resource, the namespace's resource dimension's first.
          dimensions: m.dimensionSets.filter((d) => d.length).sort((a, b) => Number(b.includes(dimension)) - Number(a.includes(dimension)) || a.length - b.length)[0] ?? [],
        })),
    };
  }
  for (const [ns, spec] of Object.entries(awsDocs(docs))) out[ns] ??= spec;
  const sorted = Object.fromEntries(
    Object.keys(out)
      .sort()
      .map((ns) => [ns, out[ns]]),
  );
  return {
    source: `aws-cdk-lib@${version} (Apache-2.0), lib/*/lib/*-canned-metrics.generated.*${docs.length ? '; the other namespaces from their CloudWatch metrics pages of the AWS documentation (doc)' : ''}`,
    namespaces: sorted,
  };
}

/** Azure Monitor units → Datadog's (counts have none). */
const AZURE_UNITS = { Bytes: 'byte', BytesPerSecond: 'byte/second', BitsPerSecond: 'bit/second', Percent: 'percent', Milliseconds: 'millisecond', MilliSeconds: 'millisecond', Seconds: 'second' };

/** One supported-metrics page: `[{name, display, aggregation, unit}]` from its tables. */
export function azurePage(md) {
  const metrics = [];
  // |**Percentage CPU**<br><br>… |`Percentage CPU` | No | Percent |Average |\<none\>|PT1M |Yes|
  for (const row of md.split('\n').filter((l) => l.startsWith('|**'))) {
    const [, metric, rest, , unit, aggregations] = row.split('|').map((c) => c.trim());
    const [display, name] = [metric.match(/^\*\*(.+?)\*\*/)?.[1], rest.match(/^`([^`]+)`$/)?.[1]];
    // The default aggregation: the first listed (`Total (Sum), Average, …`).
    const aggregation = aggregations?.split(',')[0].replace(/\s*\(.*\)$/, '');
    if (display && name && aggregation && !metrics.some((m) => m.name === name)) metrics.push({ name, display, aggregation, ...(AZURE_UNITS[unit] ? { unit: AZURE_UNITS[unit] } : {}) });
  }
  return metrics;
}

export function azureMetrics(dir, revision) {
  const catalog = JSON.parse(readFileSync(new URL('../../azure_integration.json', OUT), 'utf8')).resource_types;
  const resource_types = {};
  for (const t of catalog) {
    const file = join(dir, `${t.type.toLowerCase().replace(/[/.]/g, '-')}-metrics.md`);
    if (existsSync(file)) resource_types[t.type] = { metrics: azurePage(readFileSync(file, 'utf8')) };
  }
  return { source: `MicrosoftDocs/azure-monitor-docs@${revision} (CC BY 4.0), articles/azure-monitor/reference/supported-metrics`, resource_types };
}

/** Cloud Monitoring units (UCUM) → Datadog's; dimensionless and annotated counts have none. */
const GCP_UNITS = {
  '10^2.%': 'fraction',
  '%': 'percent',
  By: 'byte',
  KiBy: 'kibibyte',
  MiBy: 'mebibyte',
  GiBy: 'gibibyte',
  'By/s': 'byte/second',
  'bit/s': 'bit/second',
  s: 'second',
  's{CPU}': 'second',
  ms: 'millisecond',
  us: 'microsecond',
  ns: 'nanosecond',
};

// &amp; last, so an escaped entity (&amp;lt;) stays text instead of being decoded twice.
const decode = (s) =>
  s
    .replace(/&#x27;/g, "'")
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&amp;/g, '&');

/** A metrics list page: `[{type, display, stage, kind, value, unit, resources}]`. */
export function gcpPage(html) {
  const out = [];
  for (const part of html.split('<tr class="met_type" id="').slice(1)) {
    const [id, body] = [part.slice(0, part.indexOf('"')), part];
    const [service, ...path] = id.split('/');
    const stage = body.match(/met_ban_(\w+)/)?.[1];
    const display = decode(body.match(/<br\/>([^<]+)\s*<\/td>/)?.[1].trim() ?? '');
    const [kind, value, unit] = [...(body.match(/<tr class="met_desc">\s*<td>([\s\S]*?)<br/)?.[1] ?? '').matchAll(/<code[^>]*>([^<]*)<\/code>/g)].map((m) => decode(m[1]));
    const resources = [...body.matchAll(/resources#tag_([\w./]+)"/g)].map((m) => m[1]);
    out.push({ type: `${service === 'kubernetes' ? 'kubernetes.io' : `${service}.googleapis.com`}/${path.join('/')}`, display, stage, kind, value, unit, resources });
  }
  return out;
}

/** The monitored resources page: each resource type's labels, in order. */
export function gcpResources(html) {
  const out = {};
  for (const [, type, body] of html.matchAll(/<h2 id="tag_([\w./]+)"[\s\S]*?<\/h2>([\s\S]*?)(?=<h2 |$)/g)) out[type] = [...body.matchAll(/<li><code[^>]*>(\w+)<\/code>/g)].map((m) => m[1]);
  return out;
}

/** A resource type's identifying label: its first beyond the project and the location (gce_instance → instance_id). */
const resourceLabel = (labels) => labels?.find((l) => !['project_id', 'location', 'zone', 'region', 'resource_container'].includes(l)) ?? null;

export function gcpMetrics(dir) {
  const catalog = JSON.parse(readFileSync(new URL('../../gcp_integration.json', OUT), 'utf8')).services;
  const pages = readdirSync(dir).filter((f) => /^metrics_(gcp_\w+|kubernetes)\.html$/.test(f));
  const all = pages.flatMap((f) => gcpPage(readFileSync(join(dir, f), 'utf8')));
  const labels = gcpResources(readFileSync(join(dir, 'resources.html'), 'utf8'));
  const services = {};
  for (const s of catalog) {
    // Released metrics with numbers, of the service's main resource (the one most of them are reported for).
    const metrics = all.filter((m) => m.type.startsWith(s.prefix) && ['GA', 'BETA'].includes(m.stage) && ['DOUBLE', 'INT64', 'BOOL', 'DISTRIBUTION'].includes(m.value));
    const counts = new Map();
    for (const m of metrics) for (const r of m.resources) counts.set(r, (counts.get(r) ?? 0) + 1);
    const resource = [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))[0]?.[0];
    if (!resource) continue;
    services[s.prefix] = {
      resource,
      dimension: resourceLabel(labels[resource]),
      metrics: metrics
        .filter((m) => m.resources.includes(resource))
        .filter((m, i, a) => a.findIndex((o) => o.type === m.type) === i)
        .map((m) => ({ type: m.type, display: m.display, kind: m.kind, value: m.value, ...(GCP_UNITS[m.unit] ? { unit: GCP_UNITS[m.unit] } : {}) })),
    };
  }
  return { source: 'cloud.google.com/monitoring/api Cloud Monitoring metrics list (CC BY 4.0), metrics_gcp_*, metrics_kubernetes and resources', services };
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const arg = (name) => (process.argv.includes(name) ? process.argv[process.argv.indexOf(name) + 1] : null);
  if (!arg('--aws') && !arg('--azure') && !arg('--gcp'))
    throw new Error('usage: node scripts/import-cloud-metrics.mjs [--aws <dir> [--aws-docs <files>]] [--azure <dir> --azure-revision <commit>] [--gcp <dir>]');
  if (arg('--aws')) {
    const result = awsMetrics(arg('--aws'), arg('--aws-docs')?.split(',') ?? []);
    writeFileSync(new URL('aws-metrics.json', OUT), JSON.stringify(result, null, 1) + '\n');
    console.log(`AWS: ${Object.keys(result.namespaces).length} namespaces`);
  }
  if (arg('--azure')) {
    const result = azureMetrics(arg('--azure'), arg('--azure-revision'));
    writeFileSync(new URL('azure-metrics.json', OUT), JSON.stringify(result, null, 1) + '\n');
    console.log(`Azure: ${Object.keys(result.resource_types).length} resource types`);
  }
  if (arg('--gcp')) {
    const result = gcpMetrics(arg('--gcp'));
    writeFileSync(new URL('gcp-metrics.json', OUT), JSON.stringify(result, null, 1) + '\n');
    console.log(`Google Cloud: ${Object.keys(result.services).length} services`);
  }
}
