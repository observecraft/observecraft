// A dashboard per AWS service, Azure resource type and Google Cloud service, as Datadog has one for each service of its cloud
// integrations: the service's icon and what it shows, its key numbers, then every metric the service publishes
// (scripts/import-cloud-metrics.mjs) over time by the service's resource. Our own dashboards: Datadog's aren't open.
// They join the integration dashboards an org gets once the service's metrics arrive (routes/integration_dashboards.rs).
// node scripts/cloud-service-dashboards.mjs
import { readFileSync, writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { gzipSync } from 'node:zlib';
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

const AGGREGATION = { sum: ' (sum)', max: ' (max)', avg: '' };

/**
 * A service's dashboard (Datadog's widget format, then converted): its icon and what it shows, an overview of its
 * key numbers, then each metric over time by the service's resource.
 * charts: [{ title, metric, agg: 'sum' | 'avg' | 'max', grouped, key: a key metric (true, or its rank) or not }]
 */
function serviceDashboard({ cloud, service, note, variables, tag, charts, color, metricsSource }) {
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
  let id = 0;
  const widget = (definition, width, height) => ({ id: ++id, definition, layout: { x: 0, y: 0, width, height } });
  const timeseries = charts.map((c) =>
    widget(
      {
        type: 'timeseries',
        title: `${c.title}${AGGREGATION[c.agg]}${c.grouped ? ` by ${c.tag ?? tag}` : ''}`,
        requests: [
          { response_format: 'timeseries', queries: [{ data_source: 'metrics', name: 'query1', query: query(c) }], formulas: [{ formula: 'query1' }], display_type: c.agg === 'sum' ? 'bars' : 'line' },
        ],
        show_legend: false,
      },
      6,
      3,
    ),
  );
  // Ordered layout: widgets flow in rows in this order (x is each one's place in its row).
  const flow = (ws) => {
    let x = 0;
    return ws.map((w) => {
      if (x + w.layout.width > 12) x = 0;
      w.layout.x = x;
      x += w.layout.width;
      return w;
    });
  };
  const doc = {
    title: service.name,
    layout_type: 'ordered',
    template_variables: variables.map((v) => ({ name: v.name, prefix: v.prefix, default: '*' })),
    widgets: flow([
      widget({ type: 'image', url: '' }, 3, 3),
      widget({ type: 'note', content: `## ${service.name}\n\n${note}`, background_color: 'white', font_size: '14', text_align: 'left' }, 9, 3),
      widget(
        {
          type: 'group',
          title: 'Overview',
          background_color: color,
          layout_type: 'ordered',
          widgets: flow(
            keys.map((c) =>
              widget(
                {
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
                3,
                2,
              ),
            ),
          ),
        },
        12,
        3,
      ),
      widget({ type: 'group', title: 'Metrics', background_color: color, layout_type: 'ordered', widgets: flow(timeseries) }, 12, 3 * Math.ceil(timeseries.length / 2)),
    ]),
  };
  const d = convertDashboard(doc, cloud, `services/${service.id}.json`, 'observecraft');
  // Our icon of the service in place of the image; our dashboard, not integrations-core's.
  const icon = (ws) => ws.map((w) => (w.definition.type === 'image' ? { ...w, definition: { type: 'image', title: service.name, logo: `tile:${service.id}`, background: '#ffffff' } } : w));
  return {
    ...d,
    id: `${service.id}-overview`,
    description: `${service.name}: the metrics ${{ aws: 'CloudWatch', azure: 'Azure Monitor', gcp: 'Cloud Monitoring' }[cloud]} publishes for it.`,
    logo: `tile:${service.id}`,
    widgets: icon(d.widgets),
    source: { repository: 'observecraft', path: 'scripts/cloud-service-dashboards.mjs', metrics: metricsSource },
  };
}

function awsDashboard(service, spec, source) {
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
  });
}

/** Azure Monitor's default aggregations and what our crawler publishes for them (azure/naming.rs AGGREGATIONS):
 *  totals and counts as `.total`, maxima as `.maximum`, the rest averaged. */
const AZURE_AGGREGATIONS = { Total: ['sum', '.total'], Count: ['sum', '.total'], Maximum: ['max', '.maximum'] };

/** `Percentage CPU` of a VM → `azure.vm.percentage_cpu` (azure/naming.rs metric_name). */
export const azureMetricOf = (prefix, metric, aggregation) => `${prefix}.${snake(metric)}${(AZURE_AGGREGATIONS[aggregation] ?? ['avg', ''])[1]}`;

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
      metric: azureMetricOf(type.prefix, m.name, m.aggregation),
      agg: (AZURE_AGGREGATIONS[m.aggregation] ?? ['avg'])[0],
      grouped: true,
      key: keyRank(m.display),
    })),
    metricsSource: source,
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
  for (const [type, spec] of Object.entries(json('integrations/cloud/azure-metrics.json').resource_types)) {
    const prefix = json('azure_integration.json').resource_types.find((t) => t.type === type).prefix;
    for (const m of spec.metrics) if (m.unit) for (const suffix of ['', '.total', '.maximum']) units.set(`${prefix}.${snake(m.name)}${suffix}`, m.unit);
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
