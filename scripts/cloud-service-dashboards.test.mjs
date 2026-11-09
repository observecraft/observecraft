// A dashboard per AWS, Azure and Google Cloud service (scripts/cloud-service-dashboards.mjs), on the metrics the
// providers publish (scripts/import-cloud-metrics.mjs), installed for an org once the service's metrics arrive.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { gunzipSync } from 'node:zlib';
import { MOCK_SERVICES, buildServiceDashboards, metricOf, publishedNames } from './cloud-service-dashboards.mjs';
import { AWS_LAYOUTS } from './aws-service-layouts.mjs';
import { gcpPage, withDatadogNamespaces } from './import-cloud-metrics.mjs';

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
  assert.deepEqual(vm.metrics.find((m) => m.name === 'Data Disk Latency').dimensions, ['LUN']);
  // Storage capacity is hourly.
  const storage = azure.resource_types['Microsoft.Storage/storageAccounts'];
  assert.equal(storage.metrics.find((m) => m.name === 'UsedCapacity').grain, 'PT1H');
  assert.deepEqual(storage.metrics.find((m) => m.name === 'Transactions').dimensions.slice(0, 2), ['ResponseType', 'GeoType']);
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
  assert.match(JSON.stringify(gce.dashboards[0].widgets), /avg:gcp\.gce\.instance\.cpu\.utilization\{\$project_id,\$instance_id\} by \{instance_id\}/);
  assert.ok(lines.filter((l) => l.logo.startsWith('tile:azure-')).length >= 50);
  assert.ok(lines.filter((l) => l.logo.startsWith('tile:gcp-')).length >= 35);
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
  const [d] = awsLine('Glue').dashboards;
  const titles = d.widgets.map((w) => w.definition.title);
  assert.ok(!titles.includes('Metrics'), 'not one flat grid');
  assert.ok(
    titles.some((t) => /^(Errors|Latency|Throughput|Saturation|Capacity)$/.test(t)),
    titles.join(', '),
  );
  const [about] = d.widgets;
  assert.ok(about.definition.widgets.some((w) => w.definition.type === 'monitor_summary'));
  assert.ok(about.definition.widgets.filter((w) => kind(w.definition) === 'query_value').length >= 2);
  const vm = bundle().find((l) => l.logo === 'tile:azure-compute-virtualmachines').dashboards[0];
  assert.ok(vm.widgets.filter((w) => w.definition.type === 'group').length >= 3);
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
