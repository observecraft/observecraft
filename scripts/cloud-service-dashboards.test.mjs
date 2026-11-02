// A dashboard per AWS, Azure and Google Cloud service (scripts/cloud-service-dashboards.mjs), on the metrics the
// providers publish (scripts/import-cloud-metrics.mjs), installed for an org once the service's metrics arrive.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { gunzipSync } from 'node:zlib';
import { MOCK_SERVICES, buildServiceDashboards, metricOf } from './cloud-service-dashboards.mjs';

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

test('Azure metrics come from Azure Monitor’s supported metrics reference with their aggregation and unit', () => {
  assert.match(azure.source, /MicrosoftDocs\/azure-monitor-docs@[0-9a-f]{40} \(CC BY 4\.0\)/);
  const vm = azure.resource_types['Microsoft.Compute/virtualMachines'];
  assert.deepEqual(
    vm.metrics.find((m) => m.name === 'Percentage CPU'),
    { name: 'Percentage CPU', display: 'Percentage CPU', aggregation: 'Average', unit: 'percent' },
  );
  assert.equal(vm.metrics.find((m) => m.name === 'Network In Total').aggregation, 'Total');
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
  assert.equal(metricOf('AWS/RDS', 'FreeStorageSpace', 'Minimum'), 'aws.rds.free_storage_space', 'statistics we don’t crawl read the average');
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
  assert.match(text, /sum:aws\.lambda\.invocations\.sum\{\$account,\$region,\$functionname\} by \{functionname\}/);
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
  assert.match(JSON.stringify(vm.dashboards[0].widgets), /sum:azure\.vm\.network_in_total\.total\{/);
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
  assert.equal(units.get('azure.vm.network_in_total.total'), 'byte');
  assert.equal(units.get('gcp.gce.instance.cpu.utilization'), 'fraction');
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
  assert.equal(mock.units['azure.vm.percentage_cpu'], 'percent');
});
