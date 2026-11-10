// A dashboard per AWS, Azure and Google Cloud service (scripts/cloud-service-dashboards.mjs), on the metrics the
// providers publish (scripts/import-cloud-metrics.mjs), installed for an org once the service's metrics arrive.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { gunzipSync } from 'node:zlib';
import { MOCK_SERVICES, azurePublished, azurePublishedNames, buildServiceDashboards, gcpNamesOf, gcpPublishedNames, metricOf, publishedNames } from './cloud-service-dashboards.mjs';
import { AWS_LAYOUTS } from './aws-service-layouts.mjs';
import { AZURE_LAYOUTS } from './azure-service-layouts.mjs';
import { GCP_LAYOUTS } from './gcp-service-layouts.mjs';
import { gcpPage, withDatadogNamespaces } from './import-cloud-metrics.mjs';

/** `s` matched literally in a RegExp. */
const escapeRegExp = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

const seed = (path) => JSON.parse(readFileSync(new URL(`../server/seeds/${path}`, import.meta.url), 'utf8'));
const aws = seed('integrations/cloud/aws-metrics.json');
const azure = seed('integrations/cloud/azure-metrics.json');
const gcp = seed('integrations/cloud/gcp-metrics.json');

test('AWS metrics come from aws-cdk-lib with their statistic and dimensions', () => {
  assert.match(aws.source, /aws-cdk-lib@\d+\.\d+\.\d+ \(Apache-2\.0\)/);
  const lambda = aws.namespaces['AWS/Lambda'];
  assert.equal(lambda.dimension, 'FunctionName');
  assert.deepEqual(
    lambda.metrics.find((m) => m.name === 'Invocations'),
    { name: 'Invocations', statistic: 'Sum', dimensions: ['FunctionName'] },
  );
  assert.ok(Object.keys(aws.namespaces).length >= 125);
  // Namespaces aws-cdk-lib doesn't cover come from the AWS documentation's metric pages.
  const glue = aws.namespaces.Glue;
  assert.equal(glue.dimension, 'JobName');
  assert.match(glue.doc, /^https:\/\/docs\.aws\.amazon\.com\//);
});

test('Datadog’s statistics, percentiles and daily periods override aws-cdk-lib’s (scripts/aws-datadog-metrics.json overrides)', () => {
  const metric = (ns, name) => aws.namespaces[ns].metrics.find((m) => m.name === name);
  // Counts are sums under their bare name, as Datadog publishes them; gauges averages.
  assert.equal(metric('AWS/SQS', 'NumberOfMessagesSent').statistic, 'Sum');
  assert.equal(metric('AWS/SQS', 'ApproximateNumberOfMessagesVisible').statistic, 'Average');
  assert.equal(metric('AWS/Lambda', 'ConcurrentExecutions').statistic, 'Average');
  assert.equal(metric('AWS/EBS', 'VolumeReadBytes').statistic, 'Average');
  // Latencies with CloudWatch's extended statistics.
  assert.deepEqual(metric('AWS/ApplicationELB', 'TargetResponseTime').percentiles, ['p50', 'p90', 'p95', 'p99']);
  assert.deepEqual(metric('AWS/ApiGateway', 'Latency').percentiles, ['p50', 'p75', 'p90', 'p95', 'p99']);
  assert.deepEqual(metric('AWS/Lambda', 'Duration').percentiles, ['p50', 'p80', 'p95', 'p99', 'p99.9']);
  // S3: daily storage metrics, and the request metrics of a bucket's filters.
  assert.equal(metric('AWS/S3', 'BucketSizeBytes').period, 86400);
  assert.deepEqual(metric('AWS/S3', 'AllRequests'), { name: 'AllRequests', statistic: 'Sum', dimensions: ['BucketName', 'FilterId'] });
  assert.equal(metric('AWS/S3', 'FirstByteLatency').percentiles.at(-1), 'p99.99');
  assert.equal(aws.namespaces['AWS/S3/Storage-Lens'].period, 86400);
});

test('overrides patch a namespace’s metrics, add the ones it lacks and set namespace fields', () => {
  const result = {
    source: 'aws-cdk-lib@2.0.0 (Apache-2.0)',
    namespaces: { 'AWS/SQS': { dimension: 'QueueName', metrics: [{ name: 'NumberOfMessagesSent', statistic: 'Average', dimensions: ['QueueName'] }] } },
  };
  const overrides = { 'AWS/SQS': { period: 60, metrics: { NumberOfMessagesSent: { statistic: 'Sum' }, NumberOfEmptyReceives: { statistic: 'Sum', dimensions: ['QueueName'] } } } };
  const out = withDatadogNamespaces(result, {}, overrides);
  assert.deepEqual(out.namespaces['AWS/SQS'], {
    dimension: 'QueueName',
    period: 60,
    metrics: [
      { name: 'NumberOfMessagesSent', statistic: 'Sum', dimensions: ['QueueName'] },
      { name: 'NumberOfEmptyReceives', statistic: 'Sum', dimensions: ['QueueName'] },
    ],
  });
  assert.throws(() => withDatadogNamespaces(result, {}, { 'AWS/SQS': { metrics: { Unknown: { percentiles: ['p99'] } } } }), /Unknown.*statistic and dimensions/);
});

test('Azure metrics come from Azure Monitor’s supported metrics reference with their aggregations, unit, dimensions and grain', () => {
  assert.match(azure.source, /MicrosoftDocs\/azure-monitor-docs@[0-9a-f]{40} \(CC BY 4\.0\)/);
  const vm = azure.resource_types['Microsoft.Compute/virtualMachines'];
  assert.deepEqual(
    vm.metrics.find((m) => m.name === 'Percentage CPU'),
    { name: 'Percentage CPU', display: 'Percentage CPU', aggregation: 'Average', aggregations: ['Average'], unit: 'percent' },
  );
  assert.equal(vm.metrics.find((m) => m.name === 'Network In Total').aggregation, 'Total');
  assert.deepEqual(azure.resource_types['Microsoft.Web/sites'].metrics.find((m) => m.name === 'AppConnections').aggregations, ['Average', 'Count', 'Maximum', 'Minimum']);
  // Function apps are App Service sites of kind `functionapp`: the same metrics, named as Datadog's Azure Functions.
  assert.deepEqual(azure.resource_types['Microsoft.Web/sites (functionapp)'].metrics, azure.resource_types['Microsoft.Web/sites'].metrics);
  assert.deepEqual(vm.metrics.find((m) => m.name === 'Data Disk Latency').dimensions, ['LUN']);
  // Storage capacity is hourly.
  const storage = azure.resource_types['Microsoft.Storage/storageAccounts'];
  assert.equal(storage.metrics.find((m) => m.name === 'UsedCapacity').grain, 'PT1H');
  assert.deepEqual(storage.metrics.find((m) => m.name === 'Transactions').dimensions.slice(0, 2), ['ResponseType', 'GeoType']);
  // Storage accounts' services are their own resource types, as Datadog's Blob, File, Queue and Table Storage.
  const blobs = azure.resource_types['Microsoft.Storage/storageAccounts/blobServices'];
  assert.deepEqual(blobs.metrics.find((m) => m.name === 'BlobCapacity').dimensions, ['BlobType', 'Tier']);
  for (const s of ['fileServices', 'queueServices', 'tableServices'])
    assert.ok(
      azure.resource_types[`Microsoft.Storage/storageAccounts/${s}`].metrics.some((m) => m.name === 'Transactions'),
      s,
    );
  assert.ok(Object.keys(azure.resource_types).length >= 50);
});

test('Google Cloud metrics come from Cloud Monitoring’s metrics list with their kind, unit and resource', () => {
  assert.match(gcp.source, /Cloud Monitoring metrics list \(CC BY 4\.0\)/);
  const gce = gcp.services['compute.googleapis.com/'];
  assert.deepEqual([gce.resource, gce.dimension], ['gce_instance', 'instance_id']);
  assert.deepEqual(
    gce.metrics.find((m) => m.type === 'compute.googleapis.com/instance/cpu/utilization'),
    { type: 'compute.googleapis.com/instance/cpu/utilization', display: 'CPU utilization', kind: 'GAUGE', value: 'DOUBLE', unit: 'fraction' },
  );
  assert.equal(gcp.services['cloudsql.googleapis.com/'].dimension, 'database_id');
  assert.ok(Object.keys(gcp.services).length >= 35);
});

test('a metric is named as our AWS crawler publishes it', () => {
  assert.equal(metricOf('AWS/Lambda', 'Invocations', 'Sum'), 'aws.lambda.invocations.sum');
  assert.equal(metricOf('AWS/EC2', 'CPUUtilization', 'Average'), 'aws.ec2.cpuutilization');
  assert.equal(metricOf('AWS/ApplicationELB', 'HTTPCode_Target_5XX_Count', 'Sum'), 'aws.applicationelb.httpcode_target_5xx_count.sum');
  assert.equal(metricOf('AWS/RDS', 'FreeStorageSpace', 'Minimum'), 'aws.rds.free_storage_space', 'other statistics chart the bare name, which carries the metric’s own');
  // Namespaces that follow Datadog: the bare name carries the metric's statistic, with Datadog's names and prefixes.
  assert.equal(metricOf('AWS/CloudFront', 'Requests', 'Sum', { datadog: true }), 'aws.cloudfront.requests');
  assert.equal(metricOf('AWS/CloudFront', '401ErrorRate', 'Average', { datadog: true, datadog_name: '401_error_rate' }), 'aws.cloudfront.401_error_rate');
  assert.equal(metricOf('AWS/ECS', 'CPUUtilization', 'Average', { datadog: true, variant: 'service' }), 'aws.ecs.service.cpuutilization');
  assert.equal(metricOf('ECS/ContainerInsights', 'RunningTaskCount', 'Average', { datadog: true }), 'ecs.containerinsights.running_task_count');
  assert.equal(metricOf('AWS/RDS/Proxy', 'ClientConnections', 'Average', { datadog: true }), 'aws.rds.proxy.client_connections');
});

test('CloudFront, ECS, RDS and WAF dashboards chart Datadog’s names, split by the resource each metric is published for', () => {
  const lines = bundle();
  const of = (logo) => lines.find((l) => l.logo === logo);
  const text = (logo) => JSON.stringify(of(logo).dashboards[0].widgets);
  assert.equal(of('tile:aws-cloudfront').prefix, 'aws.cloudfront.');
  assert.match(text('tile:aws-cloudfront'), /sum:aws\.cloudfront\.requests\{\$account,\$region,\$distributionid,distributionid:\*\} by \{distributionid\}/);
  assert.match(text('tile:aws-cloudfront'), /avg:aws\.cloudfront\.401_error_rate\{/);
  // WAF counts each web ACL's totals as rule:ALL beside its rules: totals keep rule:ALL, the rules leave it out.
  assert.match(text('tile:aws-wafv2'), /sum:aws\.wafv2\.blocked_requests\{\$account,\$region,\$webacl,rule:ALL,webacl:\*\} by \{webacl\}/);
  assert.match(text('tile:aws-wafv2'), /sum:aws\.wafv2\.blocked_requests\{\$account,\$region,\$webacl,!rule:ALL,rule:\*\} by \{rule\}/);
  assert.match(text('tile:aws-waf'), /sum:aws\.waf\.allowed_requests\{/);
  assert.match(text('tile:aws-ecs'), /avg:aws\.ecs\.cluster\.cpuutilization\{[^}]*\} by \{clustername\}/);
  assert.match(text('tile:aws-ecs'), /avg:aws\.ecs\.service\.memory_utilization\{[^}]*\} by \{servicename\}/);
  assert.equal(of('tile:aws-ecs-containerinsights').prefix, 'ecs.containerinsights.');
  assert.match(text('tile:aws-ecs-containerinsights'), /sum:ecs\.containerinsights\.running_task_count\{[^}]*servicename:\*\} by \{servicename\}/);
  assert.match(text('tile:aws-ecs-containerinsights'), /ecs\.containerinsights\.family\.cpu_utilized\{[^}]*\} by \{taskdefinitionfamily\}/);
  const rds = of('tile:aws-rds').dashboards[0];
  assert.deepEqual(
    rds.templateVariables.map((v) => v.prefix),
    ['aws_account', 'region', 'dbinstanceidentifier', 'dbclusteridentifier'],
  );
  assert.match(text('tile:aws-rds'), /avg:aws\.rds\.cpuutilization\{[^}]*dbinstanceidentifier:\*\} by \{dbinstanceidentifier\}/);
  assert.match(text('tile:aws-rds'), /avg:aws\.rds\.volume_bytes_used\{[^}]*dbclusteridentifier:\*\} by \{dbclusteridentifier\}/);
  assert.match(text('tile:aws-rds'), /avg:aws\.rds\.dbload\{/);
  assert.doesNotMatch(text('tile:aws-rds'), /aws\.rds\.database_connections\.sum/);
  assert.equal(of('tile:aws-rds-proxy').prefix, 'aws.rds.proxy.');
});

const bundle = () =>
  gunzipSync(readFileSync(new URL('../server/seeds/integrations/cloud/services.ndjson.gz', import.meta.url)))
    .toString('utf8')
    .trim()
    .split('\n')
    .map((l) => JSON.parse(l));

test('one dashboard per AWS service with metrics, installed once the service’s metrics arrive', () => {
  const lines = bundle();
  const lambda = lines.find((l) => l.logo === 'tile:aws-lambda');
  assert.equal(lambda.prefix, 'aws.lambda.');
  const [d] = lambda.dashboards;
  assert.equal(d.title, 'AWS Lambda');
  assert.deepEqual(
    d.templateVariables.map((v) => v.name),
    ['account', 'region', 'functionname'],
  );
  const text = JSON.stringify(d.widgets);
  assert.match(text, /sum:aws\.lambda\.invocations\{\$account,\$region,\$functionname\} by \{functionname\}/);
  assert.ok(lines.filter((l) => l.logo.startsWith('tile:aws-')).length >= 110, 'every AWS service of the catalog with documented metrics');
  assert.deepEqual(
    buildServiceDashboards().map((l) => l.logo),
    lines.map((l) => l.logo),
    'run node scripts/cloud-service-dashboards.mjs',
  );
});

test('one dashboard per Azure resource type and Google Cloud service, named as their crawlers publish', () => {
  const lines = bundle();
  const vm = lines.find((l) => l.logo === 'tile:azure-compute-virtualmachines');
  assert.equal(vm.prefix, 'azure.vm.');
  assert.equal(vm.dashboards[0].title, 'Azure Virtual Machine');
  assert.deepEqual(
    vm.dashboards[0].templateVariables.map((v) => v.name),
    ['subscription_id', 'resource_group', 'region', 'name'],
  );
  assert.match(JSON.stringify(vm.dashboards[0].widgets), /avg:azure\.vm\.percentage_cpu\{\$subscription_id,\$resource_group,\$region,\$name\} by \{name\}/);
  // The bare name is the primary aggregation (a total, summed); maxima are `.max`.
  assert.match(JSON.stringify(vm.dashboards[0].widgets), /sum:azure\.vm\.network_in_total\{/);
  assert.doesNotMatch(JSON.stringify(lines.filter((l) => l.logo.startsWith('tile:azure-'))), /azure\.[a-z0-9_.]+\.(total|maximum)\{/);
  const gce = lines.find((l) => l.logo === 'tile:gcp-gce');
  assert.equal(gce.prefix, 'gcp.gce.');
  const memcache = lines.find((l) => l.logo === 'tile:gcp-memcache');
  assert.match(JSON.stringify(memcache.dashboards[0].widgets), /max:gcp\.memcache\.node\.memory\.utilization\{\$project_id,\$location,\$instance_id\} by \{node_id\}/);
  assert.ok(lines.filter((l) => l.logo.startsWith('tile:azure-')).length >= 50);
  assert.ok(lines.filter((l) => l.logo.startsWith('tile:gcp-')).length >= 60);
  // Google Cloud: distributions are charted by their mean (.avg), cumulative counters by their change (.delta), as the crawler names them.
  const gcpLines = JSON.stringify(lines.filter((l) => l.logo.startsWith('tile:gcp-')));
  assert.match(gcpLines, /avg:gcp\.loadbalancing\.https\.total_latencies\.avg\{/);
  assert.match(gcpLines, /sum:gcp\.gke\.container\.restart_count\.delta\{/);
  assert.match(gcpLines, /:gcp\.storage\.storage\.total_bytes\{/);
  assert.doesNotMatch(gcpLines, /:gcp\.gcs\.[a-z0-9_.]+\{/);
});

test('Google Cloud metrics are published under Datadog’s names for their kind', () => {
  const m = (type, kind, value) => ({ type, kind, value });
  assert.deepEqual(gcpNamesOf('gce', m('compute.googleapis.com/instance/cpu/utilization', 'GAUGE', 'DOUBLE')), ['gcp.gce.instance.cpu.utilization']);
  assert.deepEqual(gcpNamesOf('gke', m('kubernetes.io/container/restart_count', 'CUMULATIVE', 'INT64')), ['gcp.gke.container.restart_count', 'gcp.gke.container.restart_count.delta']);
  assert.deepEqual(gcpNamesOf('pubsub', m('pubsub.googleapis.com/topic/send_request_latencies', 'DELTA', 'DISTRIBUTION')), [
    'gcp.pubsub.topic.send_request_latencies.avg',
    'gcp.pubsub.topic.send_request_latencies.samplecount',
    'gcp.pubsub.topic.send_request_latencies.sumsqdev',
  ]);
  // Strings and money aren't collected (gcp/naming.rs stats).
  assert.deepEqual(gcpNamesOf('gce', m('compute.googleapis.com/instance/uptime_total', 'GAUGE', 'STRING')), []);
  assert.deepEqual(gcpNamesOf('gce', m('compute.googleapis.com/instance/disk/performance_status', 'GAUGE', 'BOOL')), ['gcp.gce.instance.disk.performance_status']);
  assert.deepEqual(gcpNamesOf('loadbalancing', m('loadbalancing.googleapis.com/https/total_latencies', 'DELTA', 'DISTRIBUTION')).slice(3), [
    'gcp.loadbalancing.https.total_latencies.p95',
    'gcp.loadbalancing.https.total_latencies.p99',
  ]);
});

test('service metrics carry their units for the server', () => {
  const units = new Map(
    readFileSync(new URL('../server/seeds/integrations/cloud/service-units.csv', import.meta.url), 'utf8')
      .split('\n')
      .filter((l) => l && !l.startsWith('#'))
      .map((l) => l.split(',')),
  );
  assert.equal(units.get('azure.vm.percentage_cpu'), 'percent');
  assert.equal(units.get('azure.vm.network_in_total'), 'byte');
  assert.equal(units.get('azure.vm.percentage_cpu.max'), 'percent');
  assert.equal(units.get('gcp.gce.instance.cpu.utilization'), 'fraction');
  assert.equal(units.get('gcp.loadbalancing.https.total_latencies.avg'), 'millisecond');
  assert.equal(units.get('gcp.loadbalancing.https.total_latencies.p99'), 'millisecond');
  assert.equal(units.get('gcp.loadbalancing.https.total_latencies.samplecount'), undefined);
  assert.equal(units.get('gcp.storage.storage.total_bytes'), 'byte');
  assert.equal(units.get('aws.cloudfront.bytes_downloaded'), 'byte');
  assert.equal(units.get('aws.rds.read_latency'), 'second');
  assert.equal(units.get('aws.ecs.service.cpuutilization'), 'percent');
  assert.equal(units.get('ecs.containerinsights.memory_utilized'), 'mebibyte');
});

test('service dashboards fit the grid without overlapping', () => {
  for (const l of bundle())
    for (const d of l.dashboards) {
      const check = (ws) => {
        for (const [i, a] of ws.entries()) {
          assert.ok(a.layout.x >= 0 && a.layout.x + a.layout.w <= 12, `${d.id}: ${a.id}`);
          for (const b of ws.slice(i + 1)) {
            const [p, r] = [a.layout, b.layout];
            assert.ok(p.x + p.w <= r.x || r.x + r.w <= p.x || p.y + p.h <= r.y || r.y + r.h <= p.y, `${d.id}: ${a.id} overlaps ${b.id}`);
          }
          if (a.definition.type === 'group') check(a.definition.widgets);
        }
      };
      check(d.widgets);
    }
});

test('mock mode shows the dashboards of the services its demo account collects', async () => {
  const mock = JSON.parse(readFileSync(new URL('../web/src/api/integrations/cloudServiceDashboards.generated.json', import.meta.url), 'utf8'));
  assert.deepEqual(mock.dashboards.map((d) => d.logo).sort(), [...MOCK_SERVICES].sort());
  assert.ok(MOCK_SERVICES.includes('tile:aws-sqs'));
  for (const id of ['aws-autoscaling', 'aws-docdb', 'azure-sql-servers-elasticpools', 'azure-cdn-profiles', 'gcp-alloydb', 'gcp-artifactregistry']) assert.ok(MOCK_SERVICES.includes(`tile:${id}`), id);
  for (const id of ['gcp-gce', 'gcp-run', 'gcp-cloudsql', 'gcp-gcs', 'gcp-pubsub']) assert.ok(MOCK_SERVICES.includes(`tile:${id}`), id);
  assert.equal(mock.units['azure.vm.percentage_cpu'], 'percent');
});

test('Cloud Monitoring’s escaped text is decoded once (&amp;lt; stays the text &lt;)', () => {
  const [m] = gcpPage(
    '<tr class="met_type" id="compute/a/b"><td><code>a/b</code><br/>Bytes &amp;lt; limit &amp; more</td></tr><tr class="met_desc"><td><code>GAUGE</code><code>INT64</code><code>By</code><br/></td></tr>',
  );
  assert.equal(m.display, 'Bytes &lt; limit & more');
});

// ---------------------------------------------------------------- hand-designed AWS dashboards (scripts/aws-service-layouts.mjs)

const widgetsOf = (d) => {
  const out = [];
  const walk = (ws, group) => {
    for (const w of ws) {
      if (w.definition.type === 'group') walk(w.definition.widgets, w.definition.title);
      else out.push({ ...w.definition, group });
    }
  };
  walk(d.widgets);
  return out;
};
const kind = (w) => (w.type === 'integration' ? w.original.type : w.type);
const awsLine = (namespace) => {
  const id = seed('aws_integration.json').namespaces.find((n) => n.namespace === namespace).id;
  return bundle().find((l) => l.logo === `tile:${id}`);
};

test('the most used AWS services have a dashboard designed for them', () => {
  for (const ns of [
    'AWS/EC2',
    'AWS/EBS',
    'AWS/S3',
    'AWS/RDS',
    'AWS/ApplicationELB',
    'AWS/NetworkELB',
    'AWS/ELB',
    'AWS/Lambda',
    'AWS/DynamoDB',
    'AWS/ElastiCache',
    'AWS/SQS',
    'AWS/SNS',
    'AWS/ApiGateway',
    'AWS/CloudFront',
    'AWS/ECS',
    'ECS/ContainerInsights',
    'AWS/NATGateway',
    'AWS/Route53',
    'AWS/WAFV2',
    'AWS/Kinesis',
    'AWS/Firehose',
    'AWS/States',
    'AWS/EFS',
    'AWS/ES',
    'AWS/Redshift',
    'AWS/Kafka',
    'AWS/KMS',
    'AWS/Bedrock',
    'AWS/AppRunner',
    'AWS/Backup',
    'AWS/CertificateManager',
    'AWS/PrivateLinkEndpoints',
    'AWS/PrivateLinkServices',
    'AWS/VPN',
    'AWS/Events',
    'AWS/Logs',
    'AWS/Billing',
    'AWS/AutoScaling',
    'AWS/DocDB',
    'AWS/MemoryDB',
    'Glue',
    'AWS/ElasticMapReduce',
    '/aws/sagemaker/Endpoints',
    '/aws/sagemaker/TrainingJobs',
    '/aws/sagemaker/ProcessingJobs',
    'AWS/TransitGateway',
    'AWS/DDoSProtection',
    'AWS/RDS/Proxy',
    'WAF',
    'AWS/ElasticBeanstalk',
    'AWS/Athena',
    'AWS/AmazonMQ',
    'AWS/Neptune',
    'AWS/Cassandra',
    'AWS/AOSS',
    'AWS/Route53Resolver',
    'AWS/GlobalAccelerator',
    'AWS/DX',
    'AWS/AppSync',
    'AWS/CodeBuild',
    'AWS/StorageGateway',
    'AWS/FSx',
    'AWS/TrustedAdvisor',
    'AWS/Usage',
    'AWS/ElasticInference',
    'AWS/ElasticTranscoder',
  ])
    assert.ok(AWS_LAYOUTS[ns], `${ns} has a layout`);
  for (const ns of Object.keys(AWS_LAYOUTS)) {
    const [d] = awsLine(ns).dashboards;
    const [about, ...groups] = d.widgets;
    assert.equal(about.definition.type, 'group', `${ns}: starts with its About group`);
    const inAbout = about.definition.widgets.map((w) => w.definition);
    assert.ok(
      inAbout.some((w) => w.type === 'image' && w.logo === d.logo),
      `${ns}: its logo`,
    );
    assert.ok(
      inAbout.some((w) => w.type === 'note' && w.content.length > 40),
      `${ns}: what it shows`,
    );
    assert.ok(inAbout.filter((w) => kind(w) === 'query_value').length >= 3, `${ns}: a row of key numbers`);
    const summary = inAbout.find((w) => w.type === 'monitor_summary');
    assert.match(summary?.query ?? '', /tag:\(integration:amazon_[a-z0-9_]+\) OR aws\.|ecs\./, `${ns}: its monitors`);
    assert.ok(groups.filter((g) => g.definition.type === 'group').length >= 2, `${ns}: themed groups`);
    assert.ok(new Set(widgetsOf(d).map(kind)).size >= 3, `${ns}: more than one kind of widget`);
  }
});

test('designed dashboards follow their service’s model, not one template', () => {
  const titles = (ns) =>
    awsLine(ns)
      .dashboards[0].widgets.slice(1)
      .map((g) => g.definition.title);
  const all = ['AWS/EC2', 'AWS/S3', 'AWS/ApplicationELB', 'AWS/Lambda', 'AWS/SQS'].map((ns) => titles(ns).join('|'));
  assert.equal(new Set(all).size, all.length);
  const text = (ns) => JSON.stringify(awsLine(ns).dashboards[0]);
  const vars = (ns) => awsLine(ns).dashboards[0].templateVariables.map((v) => v.prefix);
  // S3: daily storage metrics over a week; requests, errors and latency percentiles per bucket.
  const s3 = widgetsOf(awsLine('AWS/S3').dashboards[0]);
  assert.ok(
    s3.some((w) => /bucket_size_bytes/.test(JSON.stringify(w)) && w.original?.time?.live_span),
    'S3 storage reads a longer span',
  );
  assert.match(text('AWS/S3'), /aws\.s3\.first_byte_latency\.p99/);
  assert.deepEqual(vars('AWS/S3'), ['aws_account', 'region', 'bucketname', 'filterid']);
  // ALB: healthy and unhealthy targets, p95/p99 response time, responses by source.
  assert.match(text('AWS/ApplicationELB'), /aws\.applicationelb\.un_healthy_host_count/);
  assert.match(text('AWS/ApplicationELB'), /aws\.applicationelb\.target_response_time\.p95/);
  assert.match(text('AWS/ApplicationELB'), /aws\.applicationelb\.httpcode_elb_5xx_count\{/);
  assert.deepEqual(vars('AWS/ApplicationELB'), ['aws_account', 'region', 'loadbalancer', 'targetgroup']);
  // Lambda: duration percentiles and maximum, errors, throttles, concurrency, its logs.
  assert.match(text('AWS/Lambda'), /aws\.lambda\.duration\.p95/);
  assert.match(text('AWS/Lambda'), /aws\.lambda\.duration\.maximum/);
  assert.match(text('AWS/Lambda'), /aws\.lambda\.concurrent_executions/);
  assert.ok(widgetsOf(awsLine('AWS/Lambda').dashboards[0]).some((w) => w.type === 'log_stream' && /source:lambda/.test(w.query)));
  // SQS: message states, throughput, age of the oldest message.
  assert.match(text('AWS/SQS'), /aws\.sqs\.approximate_age_of_oldest_message/);
  assert.match(text('AWS/SQS'), /aws\.sqs\.approximate_number_of_messages_not_visible/);
  assert.deepEqual(vars('AWS/SQS'), ['aws_account', 'region', 'queuename']);
  // EC2: status checks and CPU per instance.
  assert.match(text('AWS/EC2'), /aws\.ec2\.status_check_failed_system/);
  assert.deepEqual(vars('AWS/EC2'), ['aws_account', 'region', 'instanceid']);
});

test('every AWS service dashboard charts only names the crawler publishes (bare name, .sum/.maximum/.minimum, percentiles)', () => {
  assert.ok(publishedNames('AWS/Lambda').has('aws.lambda.duration.p99.9'));
  assert.ok(publishedNames('AWS/Lambda').has('aws.lambda.invocations'));
  assert.ok(publishedNames('AWS/Lambda').has('aws.lambda.invocations.minimum'));
  assert.ok(!publishedNames('AWS/Lambda').has('aws.lambda.duration.p95.sum'));
  assert.ok(publishedNames('AWS/ECS').has('aws.ecs.service.cpuutilization'));
  assert.ok(publishedNames('ECS/ContainerInsights').has('ecs.containerinsights.family.cpu_utilized'));
  assert.ok(!publishedNames('ECS/ContainerInsights').has('ecs.containerinsights.cpu_utilized.family'));
  const catalog = seed('aws_integration.json').namespaces;
  for (const l of bundle().filter((x) => x.logo.startsWith('tile:aws-'))) {
    const ns = catalog.find((n) => `tile:${n.id}` === l.logo).namespace;
    const names = publishedNames(ns);
    for (const [, name] of JSON.stringify(l.dashboards[0].widgets).matchAll(/[a-z]+:([a-z][a-z0-9_]*\.[a-z0-9_.]+)\{/g)) assert.ok(names.has(name), `${ns}: ${name}`);
  }
});

test('services without a design get their key numbers and their metrics grouped by what they measure', () => {
  const [d] = awsLine('AWS/AppStream').dashboards;
  const titles = d.widgets.map((w) => w.definition.title);
  assert.ok(!titles.includes('Metrics'), 'not one flat grid');
  assert.ok(
    titles.some((t) => /^(Errors|Latency|Throughput|Saturation|Capacity)$/.test(t)),
    titles.join(', '),
  );
  const [about] = d.widgets;
  assert.ok(about.definition.widgets.some((w) => w.definition.type === 'monitor_summary'));
  assert.ok(about.definition.widgets.filter((w) => kind(w.definition) === 'query_value').length >= 2);
  const mediatailor = awsLine('AWS/MediaTailor').dashboards[0];
  assert.ok(mediatailor.widgets.filter((w) => w.definition.type === 'group').length >= 3);
});

test('designed dashboards’ metrics carry their units', () => {
  const units = new Map(
    readFileSync(new URL('../server/seeds/integrations/cloud/service-units.csv', import.meta.url), 'utf8')
      .split('\n')
      .filter((l) => l && !l.startsWith('#'))
      .map((l) => l.split(',')),
  );
  assert.equal(units.get('aws.sqs.approximate_age_of_oldest_message'), 'second');
  assert.equal(units.get('aws.lambda.duration.p95'), 'millisecond');
  assert.equal(units.get('aws.s3.bucket_size_bytes'), 'byte');
  assert.equal(units.get('aws.applicationelb.target_response_time.p99'), 'second');
  assert.equal(units.get('aws.ec2.cpuutilization.maximum'), 'percent');
});

// ---------------------------------------------------------------- hand-designed Azure dashboards (scripts/azure-service-layouts.mjs)

const azureLine = (type) => {
  const id = seed('azure_integration.json').resource_types.find((t) => t.type === type).id;
  return bundle().find((l) => l.logo === `tile:${id}`);
};
const DESIGNED_AZURE = [
  'Microsoft.Compute/virtualMachines',
  'Microsoft.Compute/virtualMachineScaleSets',
  'Microsoft.ContainerService/managedClusters',
  'Microsoft.Web/sites',
  'Microsoft.Web/sites (functionapp)',
  'Microsoft.Web/serverFarms',
  'Microsoft.Sql/servers/databases',
  'Microsoft.Sql/managedInstances',
  'Microsoft.DBforPostgreSQL/flexibleServers',
  'Microsoft.DBforMySQL/flexibleServers',
  'Microsoft.DocumentDB/databaseAccounts',
  'Microsoft.Storage/storageAccounts',
  'Microsoft.Storage/storageAccounts/blobServices',
  'Microsoft.Storage/storageAccounts/fileServices',
  'Microsoft.Storage/storageAccounts/queueServices',
  'Microsoft.Storage/storageAccounts/tableServices',
  'Microsoft.KeyVault/vaults',
  'Microsoft.Cache/redis',
  'Microsoft.Cache/redisEnterprise',
  'Microsoft.Network/loadBalancers',
  'Microsoft.Network/applicationGateways',
  'Microsoft.Network/frontDoors',
  'Microsoft.Network/dnszones',
  'Microsoft.Network/privateDnsZones',
  'Microsoft.Network/natGateways',
  'Microsoft.Network/publicIPAddresses',
  'Microsoft.Network/virtualNetworkGateways',
  'Microsoft.Network/expressRouteCircuits',
  'Microsoft.Network/azureFirewalls',
  'Microsoft.EventHub/namespaces',
  'Microsoft.ServiceBus/namespaces',
  'Microsoft.EventGrid/topics',
  'Microsoft.EventGrid/systemTopics',
  'Microsoft.ApiManagement/service',
  'Microsoft.ContainerRegistry/registries',
  'Microsoft.App/containerApps',
  'Microsoft.ContainerInstance/containerGroups',
  'Microsoft.OperationalInsights/workspaces',
  'Microsoft.Insights/components',
  'Microsoft.RecoveryServices/vaults',
  'Microsoft.DataProtection/backupVaults',
  'Microsoft.CognitiveServices/accounts',
  'Microsoft.Search/searchServices',
  'Microsoft.MachineLearningServices/workspaces',
  'Microsoft.Synapse/workspaces',
  'Microsoft.DataFactory/factories',
  'Microsoft.Insights/dataCollectionRules',
  'Microsoft.Compute/disks',
  'Microsoft.Web/hostingEnvironments',
  'Microsoft.Sql/servers/elasticPools',
  'Microsoft.Cdn/profiles',
  'Microsoft.Network/trafficManagerProfiles',
  'Microsoft.Logic/workflows',
  'Microsoft.Automation/automationAccounts',
  'Microsoft.HDInsight/clusters',
  'Microsoft.StreamAnalytics/streamingjobs',
  'Microsoft.AppConfiguration/configurationStores',
  'Microsoft.SignalRService/SignalR',
  'Microsoft.Devices/IotHubs',
  'Microsoft.Batch/batchAccounts',
  'Microsoft.NetApp/netAppAccounts/capacityPools/volumes',
  'Microsoft.AnalysisServices/servers',
  'Microsoft.PowerBIDedicated/capacities',
  'Microsoft.Relay/namespaces',
];

test('the most used Azure services have a dashboard designed for them', () => {
  for (const type of DESIGNED_AZURE) assert.ok(AZURE_LAYOUTS[type], `${type} has a layout`);
  for (const type of Object.keys(AZURE_LAYOUTS)) {
    const prefix = seed('azure_integration.json').resource_types.find((t) => t.type === type).prefix;
    const [d] = azureLine(type).dashboards;
    const [about, ...groups] = d.widgets;
    assert.equal(about.definition.type, 'group', `${type}: starts with its About group`);
    const inAbout = about.definition.widgets.map((w) => w.definition);
    assert.ok(
      inAbout.some((w) => w.type === 'image' && w.logo === d.logo),
      `${type}: its logo`,
    );
    assert.ok(
      inAbout.some((w) => w.type === 'note' && w.content.length > 40),
      `${type}: what it shows`,
    );
    const kpis = inAbout.filter((w) => kind(w) === 'query_value');
    assert.ok(kpis.length >= 3, `${type}: a row of key numbers`);
    // Its resources by Resource Health's status, as Datadog's Azure dashboards count them.
    assert.ok(
      kpis.some((w) => JSON.stringify(w).includes(`${prefix}.count{`)),
      `${type}: counts its resources`,
    );
    const summary = inAbout.find((w) => w.type === 'monitor_summary');
    assert.match(summary?.query ?? '', new RegExp(`^tag:\\(integration:azure_[a-z0-9_]+\\) OR ${escapeRegExp(prefix)}\\.$`), `${type}: its monitors`);
    assert.ok(groups.filter((g) => g.definition.type === 'group').length >= 3, `${type}: themed groups`);
    assert.ok(new Set(widgetsOf(d).map(kind)).size >= 3, `${type}: more than one kind of widget`);
    assert.deepEqual(
      d.templateVariables.slice(0, 4).map((v) => v.prefix),
      ['subscription_id', 'resource_group', 'region', 'name'],
      `${type}: filters by the crawler's resource tags`,
    );
  }
});

test('designed Azure dashboards follow their service’s model, not one template, and not AWS’s', () => {
  const titles = (type) =>
    azureLine(type)
      .dashboards[0].widgets.slice(1)
      .map((g) => g.definition.title)
      .join('|');
  const all = DESIGNED_AZURE.map(titles);
  assert.equal(new Set(all).size, all.length, 'each service has its own groups');
  const aws = Object.keys(AWS_LAYOUTS).map((ns) =>
    awsLine(ns)
      .dashboards[0].widgets.slice(1)
      .map((g) => g.definition.title)
      .join('|'),
  );
  for (const t of all) assert.ok(!aws.includes(t), t);
  const text = (type) => JSON.stringify(azureLine(type).dashboards[0]);
  const vars = (type) => azureLine(type).dashboards[0].templateVariables.map((v) => v.prefix);
  // VMs: CPU, memory, disks, network and their availability.
  assert.match(text('Microsoft.Compute/virtualMachines'), /avg:azure\.vm\.percentage_cpu\{\$subscription_id,\$resource_group,\$region,\$name\} by \{name\}/);
  assert.match(text('Microsoft.Compute/virtualMachines'), /azure\.vm\.available_memory_percentage/);
  assert.match(text('Microsoft.Compute/virtualMachines'), /sum:azure\.vm\.count\{[^}]*\} by \{status\}/);
  // App Service: requests and status codes per instance, response time, function executions.
  assert.match(text('Microsoft.Web/sites'), /sum:azure\.app_services\.http5xx\{/);
  assert.match(text('Microsoft.Web/sites'), /azure\.app_services\.http_response_time/);
  assert.match(text('Microsoft.Web/sites'), /by \{instance\}/);
  assert.doesNotMatch(text('Microsoft.Web/sites'), /function_execution/, 'function apps are Azure Functions');
  // Functions: executions and execution units by function app, HTTP status codes, instances' memory.
  assert.match(text('Microsoft.Web/sites (functionapp)'), /sum:azure\.functions\.function_execution_count\{[^}]*\} by \{name\}/);
  assert.match(text('Microsoft.Web/sites (functionapp)'), /azure\.functions\.function_execution_units/);
  assert.match(text('Microsoft.Web/sites (functionapp)'), /sum:azure\.functions\.http5xx\{/);
  assert.match(text('Microsoft.Web/sites (functionapp)'), /sum:azure\.functions\.count\{[^}]*\} by \{status\}/);
  assert.deepEqual(vars('Microsoft.Web/sites'), ['subscription_id', 'resource_group', 'region', 'name', 'instance']);
  // Storage: transactions by API and response type, end-to-end against server latency, capacity.
  assert.match(text('Microsoft.Storage/storageAccounts'), /sum:azure\.storage\.transactions\{[^}]*\} by \{apiname\}/);
  assert.match(text('Microsoft.Storage/storageAccounts'), /by \{responsetype\}/);
  assert.match(text('Microsoft.Storage/storageAccounts'), /azure\.storage\.success_e2elatency/);
  assert.match(text('Microsoft.Storage/storageAccounts'), /azure\.storage\.used_capacity/);
  // Blob, File, Queue and Table Storage: each service's transactions by API, its capacity, its own objects.
  assert.match(text('Microsoft.Storage/storageAccounts/blobServices'), /sum:azure\.storage_storageaccounts_blobservices\.transactions\{[^}]*\} by \{apiname\}/);
  assert.match(text('Microsoft.Storage/storageAccounts/blobServices'), /sum:azure\.storage_storageaccounts_blobservices\.blob_capacity\{[^}]*\} by \{tier\}/);
  assert.match(text('Microsoft.Storage/storageAccounts/fileServices'), /azure\.storage_storageaccounts_fileservices\.percent_file_share_utilization\{[^}]*\} by \{fileshare\}/);
  assert.match(text('Microsoft.Storage/storageAccounts/queueServices'), /azure\.storage_storageaccounts_queueservices\.queue_message_count/);
  assert.match(text('Microsoft.Storage/storageAccounts/tableServices'), /azure\.storage_storageaccounts_tableservices\.table_entity_count/);
  assert.deepEqual(vars('Microsoft.Storage/storageAccounts/fileServices'), ['subscription_id', 'resource_group', 'region', 'name', 'apiname', 'responsetype', 'fileshare']);
  // Cosmos DB: requests by status code, throttled 429s, normalized RU consumption.
  assert.match(text('Microsoft.DocumentDB/databaseAccounts'), /sum:azure\.cosmosdb\.total_requests\{[^}]*\} by \{statuscode\}/);
  assert.match(text('Microsoft.DocumentDB/databaseAccounts'), /statuscode:429/);
  assert.match(text('Microsoft.DocumentDB/databaseAccounts'), /azure\.cosmosdb\.normalized_ruconsumption/);
  // Key Vault: API hits by activity type, availability, latency, saturation.
  assert.match(text('Microsoft.KeyVault/vaults'), /azure\.keyvault_vaults\.service_api_hit\{[^}]*\} by \{activitytype\}/);
  assert.match(text('Microsoft.KeyVault/vaults'), /azure\.keyvault_vaults\.saturation_shoebox/);
  // Redis: hits and misses, server load by shard, memory.
  assert.match(text('Microsoft.Cache/redis'), /azure\.cache_redis\.cachemisses/);
  assert.match(text('Microsoft.Cache/redis'), /max:azure\.cache_redis\.server_load\{[^}]*\} by \{shardid\}/);
  // Event Hubs and Service Bus: messages in and out per entity, throttled requests.
  assert.match(text('Microsoft.EventHub/namespaces'), /sum:azure\.eventhub_namespaces\.incoming_messages\{[^}]*\} by \{entityname\}/);
  assert.match(text('Microsoft.ServiceBus/namespaces'), /azure\.servicebus_namespaces\.deadlettered_messages/);
  // Backup: jobs by health state.
  assert.match(text('Microsoft.RecoveryServices/vaults'), /sum:azure\.recoveryservices_vaults\.backup_health_event\{[^}]*\} by \{healthstatus\}/);
  assert.match(text('Microsoft.DataProtection/backupVaults'), /azure\.dataprotection_backupvaults\.restore_health_event/);
});

test('every Azure service dashboard charts only names and tags the crawler publishes', () => {
  assert.ok(azurePublishedNames('Microsoft.Web/sites').has('azure.app_services.cpu_time.max'));
  assert.ok(azurePublishedNames('Microsoft.Web/sites').has('azure.app_services.count'));
  assert.ok(azurePublishedNames('Microsoft.Web/sites (functionapp)').has('azure.functions.function_execution_units'));
  assert.ok(azurePublishedNames('Microsoft.Web/sites (functionapp)').has('azure.functions.count'));
  assert.deepEqual(azurePublished('Microsoft.Storage/storageAccounts/blobServices').get('azure.storage_storageaccounts_blobservices.transactions'), ['apiname', 'responsetype', 'tier']);
  assert.ok(azurePublishedNames('Microsoft.Storage/storageAccounts/tableServices').has('azure.storage_storageaccounts_tableservices.count'));
  assert.deepEqual(azurePublished('Microsoft.Web/sites (functionapp)').get('azure.functions.requests'), ['instance']);
  assert.ok(!azurePublishedNames('Microsoft.Web/sites').has('azure.app_services.http5xx.max'), 'Http5xx has no maximum');
  assert.ok(azurePublishedNames('Microsoft.Network/dnszones').has('azure.network_dnszones.record_set_count'), 'no primary aggregation: averaged');
  for (const t of seed('azure_integration.json').resource_types) {
    const line = bundle().find((l) => l.logo === `tile:${t.id}`);
    if (!line) continue;
    const names = azurePublishedNames(t.type);
    for (const [, name] of JSON.stringify(line.dashboards[0].widgets).matchAll(/[a-z]+:(azure\.[a-z0-9_.]+)\{/g)) assert.ok(names.has(name), `${t.type}: ${name}`);
  }
});

// ---------------------------------------------------------------- hand-designed Google Cloud dashboards (scripts/gcp-service-layouts.mjs)

const gcpLine = (prefix) => {
  const id = seed('gcp_integration.json').services.find((s) => s.prefix === prefix).id;
  return bundle().find((l) => l.logo === `tile:${id}`);
};
const DESIGNED_GCP = [
  'compute.googleapis.com/',
  'kubernetes.io/',
  'run.googleapis.com/',
  'cloudfunctions.googleapis.com/',
  'appengine.googleapis.com/',
  'cloudsql.googleapis.com/',
  'spanner.googleapis.com/',
  'bigtable.googleapis.com/',
  'bigquery.googleapis.com/',
  'firestore.googleapis.com/',
  'datastore.googleapis.com/',
  'redis.googleapis.com/',
  'memorystore.googleapis.com/',
  'pubsub.googleapis.com/',
  'storage.googleapis.com/',
  'loadbalancing.googleapis.com/',
  'edgecache.googleapis.com/',
  'dns.googleapis.com/',
  'router.googleapis.com/',
  'vpn.googleapis.com/',
  'interconnect.googleapis.com/',
  'networksecurity.googleapis.com/',
  'file.googleapis.com/',
  'dataflow.googleapis.com/',
  'dataproc.googleapis.com/',
  'composer.googleapis.com/',
  'aiplatform.googleapis.com/',
  'serviceruntime.googleapis.com/',
  'logging.googleapis.com/',
  'cloudtasks.googleapis.com/',
  'cloudkms.googleapis.com/',
  'eventarc.googleapis.com/',
  'monitoring.googleapis.com/',
  'alloydb.googleapis.com/',
  'artifactregistry.googleapis.com/',
  'cloudbuild.googleapis.com/',
  'workflows.googleapis.com/',
  'managedkafka.googleapis.com/',
  'memcache.googleapis.com/',
  'iam.googleapis.com/',
  'backupdr.googleapis.com/',
  'certificatemanager.googleapis.com/',
  'firewallinsights.googleapis.com/',
  'datastream.googleapis.com/',
  'datafusion.googleapis.com/',
  'dataplex.googleapis.com/',
  'bigquerystorage.googleapis.com/',
  'bigquerydatatransfer.googleapis.com/',
  'storagetransfer.googleapis.com/',
  'tpu.googleapis.com/',
  'pubsublite.googleapis.com/',
  'privateca.googleapis.com/',
  'recaptchaenterprise.googleapis.com/',
  'gkebackup.googleapis.com/',
  'parallelstore.googleapis.com/',
  'lustre.googleapis.com/',
  'firebasedatabase.googleapis.com/',
  'firebasehosting.googleapis.com/',
];

test('the most used Google Cloud services have a dashboard designed for them', () => {
  for (const prefix of DESIGNED_GCP) assert.ok(GCP_LAYOUTS[prefix], `${prefix} has a layout`);
  for (const prefix of Object.keys(GCP_LAYOUTS)) {
    const short = seed('gcp_integration.json').services.find((s) => s.prefix === prefix).short;
    const [d] = gcpLine(prefix).dashboards;
    const [about, ...groups] = d.widgets;
    assert.equal(about.definition.type, 'group', `${prefix}: starts with its About group`);
    const inAbout = about.definition.widgets.map((w) => w.definition);
    assert.ok(
      inAbout.some((w) => w.type === 'image' && w.logo === d.logo),
      `${prefix}: its logo`,
    );
    assert.ok(
      inAbout.some((w) => w.type === 'note' && w.content.length > 40),
      `${prefix}: what it shows`,
    );
    assert.ok(inAbout.filter((w) => kind(w) === 'query_value').length >= 3, `${prefix}: a row of key numbers`);
    const summary = inAbout.find((w) => w.type === 'monitor_summary');
    assert.match(summary?.query ?? '', new RegExp(`^tag:\\(integration:google_[a-z0-9_]+\\) OR gcp\\.${short}\\.$`), `${prefix}: its monitors`);
    assert.ok(groups.filter((g) => g.definition.type === 'group').length >= 2, `${prefix}: themed groups`);
    assert.ok(new Set(widgetsOf(d).map(kind)).size >= 3, `${prefix}: more than one kind of widget`);
    assert.equal(d.templateVariables[0].prefix, 'project_id', `${prefix}: filters by project first`);
    assert.ok(d.templateVariables.length >= 2, `${prefix}: and by the service's resource`);
  }
});

test('designed Google Cloud dashboards follow their service’s model, not one template, and not AWS’s or Azure’s', () => {
  const titles = (line) =>
    line.dashboards[0].widgets
      .slice(1)
      .map((g) => g.definition.title)
      .join('|');
  const all = DESIGNED_GCP.map((p) => titles(gcpLine(p)));
  assert.equal(new Set(all).size, all.length, 'each service has its own groups');
  const others = bundle()
    .filter((l) => /^tile:(aws|azure)-/.test(l.logo))
    .map(titles);
  for (const t of all) assert.ok(!others.includes(t), t);
  const text = (p) => JSON.stringify(gcpLine(p).dashboards[0]);
  const vars = (p) => gcpLine(p).dashboards[0].templateVariables.map((v) => v.prefix);
  // Compute Engine: CPU by instance name, disks, network, firewall drops; its logs.
  assert.match(text('compute.googleapis.com/'), /avg:gcp\.gce\.instance\.cpu\.utilization\{[^}]*\} by \{instance_name\}/);
  assert.match(text('compute.googleapis.com/'), /gcp\.gce\.instance\.disk\.write_bytes_count/);
  assert.match(text('compute.googleapis.com/'), /gcp\.gce\.firewall\.dropped_packets_count/);
  assert.match(text('compute.googleapis.com/'), /source:gcp\.gce\.instance/);
  assert.deepEqual(vars('compute.googleapis.com/'), ['project_id', 'region', 'zone', 'instance_name']);
  // Cloud Run: requests by response code class, latency percentiles, instances, utilization distributions, billable time.
  assert.match(text('run.googleapis.com/'), /sum:gcp\.run\.request_count\{[^}]*\} by \{response_code_class\}/);
  assert.match(text('run.googleapis.com/'), /gcp\.run\.request_latencies\.p99/);
  assert.match(text('run.googleapis.com/'), /gcp\.run\.container\.cpu\.utilizations\.p95/);
  assert.match(text('run.googleapis.com/'), /gcp\.run\.container\.billable_instance_time/);
  assert.deepEqual(vars('run.googleapis.com/'), ['project_id', 'location', 'service_name', 'revision_name']);
  // Cloud SQL: one dashboard for MySQL, PostgreSQL and SQL Server, with replication lag and Query Insights.
  assert.match(text('cloudsql.googleapis.com/'), /gcp\.cloudsql\.database\.replication\.replica_lag/);
  assert.match(text('cloudsql.googleapis.com/'), /gcp\.cloudsql\.database\.mysql\.queries/);
  assert.match(text('cloudsql.googleapis.com/'), /gcp\.cloudsql\.database\.postgresql\.insights\.aggregate\.latencies\.p99/);
  assert.match(text('cloudsql.googleapis.com/'), /gcp\.cloudsql\.database\.sqlserver\./);
  // GKE: allocatable against used, container restarts by their change, pod volumes.
  assert.match(text('kubernetes.io/'), /gcp\.gke\.node\.cpu\.allocatable_utilization/);
  assert.match(text('kubernetes.io/'), /sum:gcp\.gke\.container\.restart_count\.delta\{/);
  assert.match(text('kubernetes.io/'), /gcp\.gke\.pod\.volume\.utilization/);
  // Load Balancing: requests by response code class, total and backend latency percentiles, CDN cache results.
  assert.match(text('loadbalancing.googleapis.com/'), /gcp\.loadbalancing\.https\.total_latencies\.p95/);
  assert.match(text('loadbalancing.googleapis.com/'), /gcp\.loadbalancing\.https\.backend_latencies\.p99/);
  assert.match(text('loadbalancing.googleapis.com/'), /by \{cache_result\}/);
  assert.match(text('loadbalancing.googleapis.com/'), /gcp\.loadbalancing\.tcp_ssl_proxy\.open_connections/);
  // Pub/Sub: undelivered messages, oldest unacked age, acks and dead letters.
  assert.match(text('pubsub.googleapis.com/'), /gcp\.pubsub\.subscription\.num_undelivered_messages/);
  assert.match(text('pubsub.googleapis.com/'), /gcp\.pubsub\.subscription\.oldest_unacked_message_age/);
  assert.match(text('pubsub.googleapis.com/'), /gcp\.pubsub\.topic\.send_request_count/);
  assert.deepEqual(vars('pubsub.googleapis.com/'), ['project_id', 'topic_id', 'subscription_id']);
  // Cloud Storage: daily storage over a week, requests by method and response code.
  assert.ok(
    widgetsOf(gcpLine('storage.googleapis.com/').dashboards[0]).some((w) => /storage\.total_bytes/.test(JSON.stringify(w)) && w.original?.time?.live_span),
    'Cloud Storage reads its storage over a longer span',
  );
  assert.match(text('storage.googleapis.com/'), /sum:gcp\.storage\.api\.request_count\{[^}]*\} by \{method\}/);
});

test('every Google Cloud service dashboard charts only names the crawler publishes, cumulative counters by their change', () => {
  assert.ok(gcpPublishedNames('kubernetes.io/').has('gcp.gke.node.cpu.allocatable_utilization'), 'other resources’ metrics too');
  assert.ok(gcpPublishedNames('cloudsql.googleapis.com/').has('gcp.cloudsql.database.postgresql.insights.aggregate.latencies.p99'));
  assert.ok(!gcpPublishedNames('run.googleapis.com/').has('gcp.run.request_count.delta'), 'deltas have no .delta');
  const gcpSeed = seed('integrations/cloud/gcp-metrics.json').services;
  for (const s of seed('gcp_integration.json').services) {
    const line = bundle().find((l) => l.logo === `tile:${s.id}`);
    if (!line) continue;
    const names = gcpPublishedNames(s.prefix);
    const cumulative = new Set(
      [...gcpSeed[s.prefix].metrics, ...(gcpSeed[s.prefix].others ?? [])].filter((m) => m.kind === 'CUMULATIVE' && m.value !== 'DISTRIBUTION').map((m) => gcpNamesOf(s.short, m)[0]),
    );
    for (const [, name] of JSON.stringify(line.dashboards[0].widgets).matchAll(/[a-z]+:(gcp\.[a-z0-9_.]+)\{/g)) {
      assert.ok(names.has(name), `${s.prefix}: ${name}`);
      assert.ok(!cumulative.has(name), `${s.prefix}: ${name} is a running total; chart its .delta`);
    }
  }
});
