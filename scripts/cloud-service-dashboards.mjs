// A dashboard per AWS service, Azure resource type and Google Cloud service, as Datadog has one for each service of its cloud
// integrations: the service's icon, what it shows, its monitors and key numbers, then groups of charts. The most used AWS
// services have a layout designed for them (scripts/aws-service-layouts.mjs); the others chart every metric the service
// publishes (scripts/import-cloud-metrics.mjs) by the service's resource, grouped by what it measures. Our own
// dashboards: Datadog's aren't open.
// They join the integration dashboards an org gets once the service's metrics arrive (routes/integration_dashboards.rs).
// node scripts/cloud-service-dashboards.mjs
import { readFileSync, writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { gzipSync } from 'node:zlib';
import { AWS_LAYOUTS, queriesOver } from './aws-service-layouts.mjs';
import { convertDashboard } from './import-integration-dashboards.mjs';

const seeds = new URL('../server/seeds/', import.meta.url);
const json = (path) => JSON.parse(readFileSync(new URL(path, seeds), 'utf8'));

/** As the crawlers name things (server/crates/oc-api/src/cloud/naming.rs). */
const underscored = (s) =>
  s
    .replace(/[^A-Za-z0-9]+/g, '_')
    .toLowerCase()
    .replace(/^_+|_+$/g, '');
const snake = (m) => underscored(m.replace(/([a-z])([A-Z])/g, '$1_$2'));

/** The suffixes of the statistics the AWS crawler reads (aws/naming.rs STATS). The metric's own statistic is also its
 *  bare name (the Average's only), so a chart of a Sum or Maximum metric reads its suffixed name and the others the bare. */
const AWS_STATS = { Average: '', Sum: '.sum', Maximum: '.maximum' };

/** Namespaces Datadog doesn't name `aws.<namespace>.` (aws/naming.rs PREFIXES). */
const AWS_PREFIXES = { 'AWS/RDS/Proxy': 'aws.rds.proxy', 'ECS/ContainerInsights': 'ecs.containerinsights' };

/** The metric prefix of a CloudWatch namespace: `AWS/EC2` → `aws.ec2` (aws/naming.rs metric_prefix). */
export const awsPrefix = (namespace) => AWS_PREFIXES[namespace] ?? `aws.${underscored(namespace.toLowerCase().replace(/^\/?aws\//, ''))}`;

/**
 * A CloudWatch metric as our AWS crawler publishes it: `aws.<namespace>.<metric>[.sum|.maximum]`. In the namespaces
 * that follow Datadog (`datadog`, scripts/aws-datadog-metrics.json) the metric's statistic is its bare name's, and
 * Datadog's name (`datadog_name`) and dimension set's segment (`variant`: aws.ecs.service.*) are used.
 */
export function metricOf(namespace, metric, statistic, { datadog = false, datadog_name, variant } = {}) {
  const name = `${awsPrefix(namespace)}.${variant ? `${variant}.` : ''}${datadog_name ?? snake(metric)}`;
  return datadog ? name : `${name}${AWS_STATS[statistic] ?? ''}`;
}

/** `ConcurrentExecutions` → `Concurrent executions`. */
const words = (name) =>
  name
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/_/g, ' ')
    .replace(/^./, (c) => c.toUpperCase())
    .replace(/ ([A-Z])([a-z])/g, (_, a, b) => ` ${a.toLowerCase()}${b}`);

/** Metrics worth a number at the top: requests, errors and throttles first, then whatever comes first. */
const KEY =
  /^(Invocations|Errors|Throttles|Requests?|RequestCount|.*5XX.*|CPUUtilization|Count|Latency|Duration|ConsumedReadCapacityUnits|NumberOfMessagesPublished|NumberOfMessagesSent|IncomingRecords|DatabaseConnections|HealthyHostCount)$/;
/** The same for Azure's and Google Cloud's display names, most telling first: a chart's key is its rank (lower first). */
const KEY_WORDS = [
  /^(percentage cpu|cpu (utilization|percent(age)?))\b/i,
  /\brequests?( count)?$/i,
  /\b(errors?|failed|5xx)\b/i,
  /\b(latency|response time)\b/i,
  /^(network (in|out)( total)?|(bytes|data) (received|sent))\b/i,
  /\b(invocations|executions)\b/i,
  /\bconnections\b/i,
  /\b(memory|disk) (utilization|percent(age)?)\b/i,
  /\b(throttled|availability)\b/i,
  /\b(cpu|utilization)\b/i,
];
const keyRank = (display) => {
  if (/\((deprecated|preview)\)|semi-managed|ecc/i.test(display)) return undefined;
  const i = KEY_WORDS.findIndex((r) => r.test(display));
  return i < 0 ? undefined : i;
};

/** At most this many charts per dashboard: the services with hundreds of metrics show their key ones, then their first. */
const MAX_CHARTS = 48;

const AGGREGATION = { sum: ' (sum)', max: ' (max)', min: ' (min)', avg: '' };

/** What a metric measures, from its name: the groups of a generated dashboard, in this order (first match wins). */
export const CATEGORIES = [
  ['Errors', /error|fail|fault|throttl|reject|drop|denied|timed ?out|timeout|5xx|4xx|unhealthy|dead ?letter|exceeded|abort|invalid/i],
  ['Latency', /latency|duration|response ?time|\btime\b|[a-z](?<!up)time$|\bage\b|[a-z]age$(?<!percentage|usage|storage)|\blag\b|delay|wait/i],
  ['Saturation', /utili[sz]ation|usage|percent|pressure|balance|credit|concurren|queue|load|memory|cpu|swap|quota|capacity used/i],
  ['Throughput', /request|invocation|execution|bytes|packets|records|messages|connection|ops\b|iops|throughput|sent|received|transfer|calls|\bin\b|\bout\b|reads?\b|writes?\b|hits|miss/i],
  ['Capacity', /size|storage|count|capacity|objects|items|space|instances|nodes|shards|documents|available/i],
];
const categoryOf = (title) => CATEGORIES.find(([, r]) => r.test(title))?.[0] ?? 'Other metrics';

// ---------------------------------------------------------------- layout

/** Widgets placed in a group's columns, each where it rises least (left first), so smaller ones stack beside a tall one. */
function pack(items, cols, next) {
  const sky = Array(cols).fill(0);
  return items.map((item) => {
    const w = Math.min(item.width, cols);
    let at = { x: 0, y: Infinity };
    for (let x = 0; x + w <= cols; x++) {
      const y = Math.max(...sky.slice(x, x + w));
      if (y < at.y) at = { x, y };
    }
    for (let x = at.x; x < at.x + w; x++) sky[x] = at.y + item.height;
    return { id: next(), definition: item.definition, layout: { x: at.x, y: at.y, width: w, height: item.height } };
  });
}

/**
 * A service's dashboard in Datadog's widget format, converted to ours: an About group (its icon, what it shows, its
 * monitors and key numbers), then its themed groups ({ group, title, widgets: sized widgets, color, width }) and its
 * logs. Our icon of the service in place of the image; our dashboard, not integrations-core's.
 */
function serviceDoc({ cloud, service, note, variables, monitors, kpis, groups, logs = [], color, description, source }) {
  let id = 0;
  const next = () => ++id;
  const about = [
    { definition: { type: 'image', url: '' }, width: 3, height: 2 },
    { definition: { type: 'note', content: `## ${service.name}\n\n${note}`, background_color: 'white', font_size: '14', text_align: 'left' }, width: 5, height: 2 },
    { definition: { type: 'manage_status', title: 'Monitors', query: monitors, summary_type: 'monitors', display_format: 'countsAndList', sort: 'status,asc' }, width: 4, height: 2 },
    // Wide enough for their titles: up to four in a row, else rows of three.
    ...kpis.slice(0, 6).map((k) => ({ ...k, width: kpis.length <= 4 ? 12 / Math.max(kpis.length, 1) : 4 })),
  ];
  const withLogs = logs.length
    ? [
        ...groups,
        { group: true, title: 'Logs', color: 'gray', width: 12, widgets: logs.map(([title, query]) => ({ definition: { type: 'log_stream', title, query }, width: 12 / logs.length, height: 4 })) },
      ]
    : groups;
  const all = [{ group: true, title: 'Overview', color, width: 12, widgets: about }, ...withLogs];
  let x = 0;
  const widgets = all.map((g) => {
    const children = pack(g.widgets, g.width, next);
    const height = Math.max(...children.map((c) => c.layout.y + c.layout.height));
    if (x + g.width > 12) x = 0;
    const w = { id: next(), definition: { type: 'group', title: g.title, background_color: g.color, layout_type: 'ordered', widgets: children }, layout: { x, y: 0, width: g.width, height } };
    x += g.width;
    return w;
  });
  const doc = { title: service.name, layout_type: 'ordered', template_variables: variables.map((v) => ({ name: v.name, prefix: v.prefix, default: '*' })), widgets };
  const d = convertDashboard(doc, cloud, `services/${service.id}.json`, 'observecraft');
  const icon = (ws) =>
    ws.map((w) =>
      w.definition.type === 'image'
        ? { ...w, definition: { type: 'image', title: service.name, logo: `tile:${service.id}`, background: '#ffffff' } }
        : w.definition.type === 'group'
          ? { ...w, definition: { ...w.definition, widgets: icon(w.definition.widgets) } }
          : w,
    );
  return { ...d, id: `${service.id}-overview`, description, logo: `tile:${service.id}`, widgets: icon(d.widgets), source };
}

/**
 * A service's generated dashboard, for the services without a designed one: its key numbers, then each metric over
 * time by the service's resource, grouped by what it measures (errors, latency, throughput, saturation, capacity).
 * charts: [{ title, metric, agg: 'sum' | 'avg' | 'max', grouped, key: a key metric (true, or its rank) or not }]
 */
function serviceDashboard({ cloud, service, note, variables, tag, charts, color, metricsSource, prefix }) {
  const vars = variables.map((v) => `$${v.name}`);
  // A chart may split by its own tag and keep tags of its own (filters: `rule:ALL`, `dbinstanceidentifier:*`).
  const query = (c, grouped = true) => `${c.agg}:${c.metric}{${[...vars, ...(c.filters ?? [])].join(',')}}${grouped && c.grouped ? ` by {${c.tag ?? tag}}` : ''}`;
  // Key charts by rank (`true` ranks first), then the others in their order.
  const ranked = [
    ...charts.filter((c) => c.key !== undefined && c.key !== false).sort((a, b) => Number(a.key === true ? -1 : a.key) - Number(b.key === true ? -1 : b.key)),
    ...charts.filter((c) => c.key === undefined || c.key === false),
  ];
  const kept = new Set(ranked.slice(0, MAX_CHARTS));
  charts = charts.filter((c) => kept.has(c));
  // One number per kind of key metric first (CPU, requests, errors, latency…), then the next ones.
  const firstOfRank = ranked.filter((c, i) => c.key !== undefined && c.key !== false && (c.key === true || ranked.findIndex((o) => o.key === c.key) === i));
  const keys = [...new Set([...firstOfRank, ...ranked])].filter((c) => kept.has(c)).slice(0, 4);
  const kpis = keys.map((c) => ({
    definition: {
      type: 'query_value',
      title: `${c.title}${AGGREGATION[c.agg]}`,
      requests: [
        {
          response_format: 'scalar',
          queries: [{ data_source: 'metrics', name: 'query1', query: query(c, false), aggregator: c.time ?? (c.agg === 'sum' ? 'sum' : 'avg') }],
          formulas: [{ formula: 'query1' }],
        },
      ],
      precision: 2,
    },
    width: 3,
    height: 2,
  }));
  const chart = (c, width) => ({
    definition: {
      type: 'timeseries',
      title: `${c.title}${AGGREGATION[c.agg]}${c.grouped ? ` by ${c.tag ?? tag}` : ''}`,
      requests: [
        { response_format: 'timeseries', queries: [{ data_source: 'metrics', name: 'query1', query: query(c) }], formulas: [{ formula: 'query1' }], display_type: c.agg === 'sum' ? 'bars' : 'line' },
      ],
      show_legend: false,
    },
    width,
    height: 2,
  });
  const byCategory = new Map([...CATEGORIES.map(([name]) => [name, []]), ['Other metrics', []]]);
  for (const c of charts) byCategory.get(categoryOf(c.title)).push(c);
  const groups = [...byCategory]
    .filter(([, cs]) => cs.length)
    .map(([title, cs]) => ({ group: true, title, color: color.replace(/^vivid_/, ''), width: 12, widgets: cs.map((c) => chart(c, cs.length === 1 ? 12 : cs.length === 2 ? 6 : 4)) }));
  return serviceDoc({
    cloud,
    service,
    note,
    variables,
    monitors: prefix,
    kpis,
    groups,
    color,
    description: `${service.name}: the metrics ${{ aws: 'CloudWatch', azure: 'Azure Monitor', gcp: 'Cloud Monitoring' }[cloud]} publishes for it.`,
    source: { repository: 'observecraft', path: 'scripts/cloud-service-dashboards.mjs', metrics: metricsSource },
  });
}

/** Every name the AWS crawler publishes for a metric of the catalog (aws/naming.rs series_names): its bare name (and
 *  the Datadog variant of its dimension set) with no suffix (its own statistic), `.sum`, `.maximum`, `.minimum` and
 *  its percentiles. */
export function namesOf(namespace, m) {
  const prefix = awsPrefix(namespace);
  const leaf = m.datadog_name ?? snake(m.name);
  const has = (d) => m.dimensions.includes(d);
  const bases =
    namespace === 'AWS/ECS' && has('ServiceName')
      ? [`${prefix}.${leaf}`, `${prefix}.service.${leaf}`]
      : namespace === 'AWS/ECS' && m.dimensions.length === 1 && has('ClusterName')
        ? [`${prefix}.${leaf}`, `${prefix}.cluster.${leaf}`]
        : namespace === 'ECS/ContainerInsights' && has('TaskDefinitionFamily')
          ? [`${prefix}.family.${leaf}`]
          : [`${prefix}.${leaf}`];
  const own = m.statistic ?? 'Average';
  const suffixes = ['', '.sum', '.maximum', '.minimum', ...(['Average', 'Sum', 'Maximum', 'Minimum'].includes(own) ? [] : [`.${own.toLowerCase()}`]), ...(m.percentiles ?? []).map((p) => `.${p}`)];
  return bases.flatMap((b) => suffixes.map((s) => `${b}${s}`));
}

/** Every metric name the AWS crawler publishes for a namespace of the catalog. */
export const publishedNames = (namespace) => new Set((json('integrations/cloud/aws-metrics.json').namespaces[namespace]?.metrics ?? []).flatMap((m) => namesOf(namespace, m)));

/** A designed AWS service dashboard (scripts/aws-service-layouts.mjs); every metric it charts must be published. */
function designedDashboard(service, layout, source) {
  const extra = layout.vars;
  const variables = [{ name: 'account', prefix: 'aws_account' }, { name: 'region', prefix: 'region' }, ...extra.map((v) => ({ name: v, prefix: v }))];
  const f = variables.map((v) => `$${v.name}`).join(',');
  const { kpis, groups } = layout.build({ ...queriesOver(f), f });
  const prefix = awsPrefix(service.namespace);
  const d = serviceDoc({
    cloud: 'aws',
    service,
    note: `${layout.note} Collected by the [AWS integration](/integrations/aws).`,
    variables,
    monitors: `tag:(integration:${layout.monitor}) OR ${prefix}.`,
    kpis,
    groups,
    logs: layout.logs,
    color: 'vivid_orange',
    description: `${service.name}: ${layout.note.split(':')[0].toLowerCase()} and their health, from CloudWatch.`,
    source: { repository: 'observecraft', path: 'scripts/aws-service-layouts.mjs', metrics: source },
  });
  const published = publishedNames(service.namespace);
  for (const [, name] of JSON.stringify(d.widgets).matchAll(/[a-z]+:([a-z][a-z0-9_]*\.[a-z0-9_.]+)\{/g))
    if (!published.has(name)) throw new Error(`${service.namespace}: ${name} isn't a name the AWS crawler publishes (aws-metrics.json)`);
  return d;
}

function awsDashboard(service, spec, source) {
  if (AWS_LAYOUTS[service.namespace]) return designedDashboard(service, AWS_LAYOUTS[service.namespace], source);
  const dim = spec.dimension;
  const tag = dim ? underscored(dim) : null;
  const agg = (m) => m.space ?? (m.statistic === 'Sum' ? 'sum' : m.statistic === 'Maximum' ? 'max' : 'avg');
  // Datadog-following namespaces (scripts/aws-datadog-metrics.json) name the dimension each chart splits by and the
  // other resources to filter by; a chart split by a resource keeps only its series (RDS publishes CPU per instance and
  // per cluster: an instance chart leaves the cluster's out).
  const groupOf = (m) => m.group ?? (dim && m.dimensions.includes(dim) ? dim : null);
  const extra = (spec.variables ?? []).map(underscored);
  const variables = [...(tag ? [tag] : []), ...extra];
  return serviceDashboard({
    cloud: 'aws',
    service,
    color: 'vivid_orange',
    note: `The ${service.name} metrics CloudWatch publishes (namespace \`${service.namespace}\`), collected by the [AWS integration](/integrations/aws). Filter by account, region${variables.length ? ` and ${variables.join(', ')}` : ''} above; each chart splits ${tag ? `by ${tag}${spec.datadog ? ' or the resource it is published for' : ''}` : 'the whole service'}.`,
    variables: [{ name: 'account', prefix: 'aws_account' }, { name: 'region', prefix: 'region' }, ...variables.map((v) => ({ name: v, prefix: v }))],
    tag,
    charts: spec.metrics.map((m) => {
      const group = groupOf(m);
      const filters = m.filter ? [m.filter] : [];
      return {
        title: words(m.name),
        metric: metricOf(service.namespace, m.name, m.statistic, { datadog: spec.datadog, datadog_name: m.datadog_name, variant: m.variant }),
        agg: agg(m),
        grouped: !!group,
        ...(spec.datadog ? { tag: group ? underscored(group) : undefined, filters: [...filters, ...(group ? [`${underscored(group)}:*`] : [])], time: m.space ? 'avg' : undefined } : {}),
        key: m.key ?? (!spec.datadog && KEY.test(m.name)),
      };
    }),
    metricsSource: source,
    prefix: `${awsPrefix(service.namespace)}.`,
  });
}

/** How a metric's series combine, by its primary aggregation: our crawler publishes that aggregation under the bare
 *  name, as Datadog does (azure/naming.rs `aggregations`), so totals and counts add up, maxima and minima stay
 *  extremes and the rest is averaged. */
const AZURE_AGGREGATIONS = { Total: 'sum', Count: 'sum', Maximum: 'max', Minimum: 'min' };

/** `Percentage CPU` of a VM → `azure.vm.percentage_cpu` (azure/naming.rs metric_name; the primary aggregation). */
export const azureMetricOf = (prefix, metric) => `${prefix}.${snake(metric)}`;

function azureDashboard(type, spec, source) {
  return serviceDashboard({
    cloud: 'azure',
    service: type,
    color: 'vivid_blue',
    note: `The ${type.name} metrics Azure Monitor publishes (resource type \`${type.type}\`), collected by the [Azure integration](/integrations/azure). Filter by subscription, resource group, region and name above; each chart splits by resource.`,
    variables: ['subscription_id', 'resource_group', 'region', 'name'].map((n) => ({ name: n, prefix: n })),
    tag: 'name',
    charts: spec.metrics.map((m) => ({
      title: m.display,
      metric: azureMetricOf(type.prefix, m.name),
      agg: AZURE_AGGREGATIONS[m.aggregation] ?? 'avg',
      grouped: true,
      key: keyRank(m.display),
    })),
    metricsSource: source,
    prefix: `${type.prefix}.`,
  });
}

/** `compute.googleapis.com/instance/cpu/utilization` → `gcp.gce.instance.cpu.utilization` (gcp/naming.rs metric_name). */
export const gcpMetricOf = (short, type) => `gcp.${short}.${type.split('/').slice(1).map(underscored).filter(Boolean).join('.')}`;

function gcpDashboard(service, spec, source) {
  const tag = spec.dimension;
  // As the crawler aligns them (gcp/api.rs aligner): gauges averaged, deltas and cumulative counts summed per minute.
  const agg = (m) => (m.value !== 'DISTRIBUTION' && (m.kind === 'DELTA' || m.kind === 'CUMULATIVE') ? 'sum' : 'avg');
  return serviceDashboard({
    cloud: 'gcp',
    service,
    color: 'vivid_blue',
    note: `The ${service.name} metrics Cloud Monitoring publishes (\`${service.prefix}\`, resource \`${spec.resource}\`), collected by the [Google Cloud integration](/integrations/gcp). Filter by project${tag ? ` and ${tag}` : ''} above; each chart splits ${tag ? `by ${tag}` : 'the whole service'}.`,
    variables: [{ name: 'project_id', prefix: 'project_id' }, ...(tag ? [{ name: tag, prefix: tag }] : [])],
    tag,
    charts: spec.metrics.map((m) => ({ title: m.display || words(m.type.split('/').at(-1)), metric: gcpMetricOf(service.short, m.type), agg: agg(m), grouped: !!tag, key: keyRank(m.display) })),
    metricsSource: source,
    prefix: `gcp.${service.short}.`,
  });
}

/** Every service's bundle line: its logo (the key), the metric prefix that installs it and its dashboard. */
export function buildServiceDashboards() {
  const aws = json('integrations/cloud/aws-metrics.json');
  const azure = json('integrations/cloud/azure-metrics.json');
  const gcp = json('integrations/cloud/gcp-metrics.json');
  const lines = [
    ...json('aws_integration.json')
      .namespaces.filter((s) => aws.namespaces[s.namespace]?.metrics.length)
      .map((s) => ({ logo: `tile:${s.id}`, prefix: `${awsPrefix(s.namespace)}.`, dashboards: [awsDashboard(s, aws.namespaces[s.namespace], aws.source)] })),
    ...json('azure_integration.json')
      .resource_types.filter((t) => azure.resource_types[t.type]?.metrics.length)
      .map((t) => ({ logo: `tile:${t.id}`, prefix: `${t.prefix}.`, dashboards: [azureDashboard(t, azure.resource_types[t.type], azure.source)] })),
    ...json('gcp_integration.json')
      .services.filter((s) => gcp.services[s.prefix]?.metrics.length)
      .map((s) => ({ logo: `tile:${s.id}`, prefix: `gcp.${s.short}.`, dashboards: [gcpDashboard(s, gcp.services[s.prefix], gcp.source)] })),
  ];
  return lines.sort((a, b) => a.logo.localeCompare(b.logo));
}

/** The units of the metrics the service dashboards chart (Datadog unit names), for the server: the crawlers send none. */
export function serviceUnits() {
  const units = new Map();
  // AWS: the units of the namespaces that follow Datadog (scripts/aws-datadog-metrics.json), on every name the crawler gives them.
  for (const [ns, spec] of Object.entries(json('integrations/cloud/aws-metrics.json').namespaces))
    if (spec.datadog)
      for (const m of spec.metrics)
        if (m.unit) {
          for (const variant of [m.variant, undefined]) {
            const name = metricOf(ns, m.name, m.statistic, { datadog: true, datadog_name: m.datadog_name, variant });
            for (const suffix of ['', '.sum', '.maximum', '.minimum']) units.set(`${name}${suffix}`, m.unit);
          }
        }
  // AWS: the units the designed dashboards give their metrics (scripts/aws-service-layouts.mjs), on every name.
  const awsMetrics = json('integrations/cloud/aws-metrics.json').namespaces;
  for (const [ns, layout] of Object.entries(AWS_LAYOUTS))
    for (const [metric, unit] of Object.entries(layout.units ?? {})) {
      const specs = awsMetrics[ns].metrics.filter((m) => m.name === metric);
      if (!specs.length) throw new Error(`${ns}: no metric ${metric} for its unit`);
      for (const m of specs) for (const name of namesOf(ns, m)) units.set(name, unit);
    }
  for (const [type, spec] of Object.entries(json('integrations/cloud/azure-metrics.json').resource_types)) {
    const prefix = json('azure_integration.json').resource_types.find((t) => t.type === type).prefix;
    for (const m of spec.metrics) if (m.unit) for (const suffix of ['', '.max']) units.set(`${azureMetricOf(prefix, m.name)}${suffix}`, m.unit);
  }
  const short = new Map(json('gcp_integration.json').services.map((s) => [s.prefix, s.short]));
  for (const [prefix, spec] of Object.entries(json('integrations/cloud/gcp-metrics.json').services))
    for (const m of spec.metrics) if (m.unit) units.set(gcpMetricOf(short.get(prefix), m.type), m.unit);
  return [...units].sort(([a], [b]) => a.localeCompare(b));
}

/** The services the mock's demo accounts collect (web/src/api/mock/integrations.ts DEMO_AWS, DEMO_AZURE, DEMO_GCP):
 *  their dashboards and their metrics' units are in the UI bundle. */
export const MOCK_SERVICES = [
  'tile:aws-apigateway',
  'tile:aws-applicationelb',
  'tile:aws-cloudfront',
  'tile:aws-dynamodb',
  'tile:aws-ec2',
  'tile:aws-ecs',
  'tile:aws-ecs-containerinsights',
  'tile:aws-lambda',
  'tile:aws-rds',
  'tile:aws-s3',
  'tile:aws-sqs',
  'tile:aws-wafv2',
  'tile:azure-compute-virtualmachines',
  'tile:azure-storage-storageaccounts',
  'tile:azure-web-sites',
  'tile:gcp-cloudsql',
  'tile:gcp-gce',
  'tile:gcp-gcs',
  'tile:gcp-run',
];

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const lines = buildServiceDashboards();
  writeFileSync(new URL('integrations/cloud/services.ndjson.gz', seeds), gzipSync(lines.map((l) => JSON.stringify(l)).join('\n') + '\n', { level: 9 }));
  const dashboards = lines.filter((l) => MOCK_SERVICES.includes(l.logo)).flatMap((l) => l.dashboards);
  const charted = JSON.stringify(dashboards);
  const units = Object.fromEntries(serviceUnits().filter(([m]) => charted.includes(`:${m}{`)));
  writeFileSync(new URL('../web/src/api/integrations/cloudServiceDashboards.generated.json', import.meta.url), JSON.stringify({ dashboards, units }) + '\n');
  writeFileSync(
    new URL('integrations/cloud/service-units.csv', seeds),
    `# Units of the AWS, Azure and Google Cloud service dashboards' metrics (scripts/cloud-service-dashboards.mjs; the crawlers send none).\n` +
      serviceUnits()
        .map(([m, u]) => `${m},${u}`)
        .join('\n') +
      '\n',
  );
  console.log(`${lines.length} service dashboards`);
}
