// Template variables in the searches of dashboard widgets (logs, signals, events, monitors) and in metric queries,
// as Datadog applies them: each `$name` where it is written, `prefix:value`, dropped while it is `*`.
import assert from 'node:assert/strict';
import test from 'node:test';
import { parseMetricQuery, resolveQuery, resolveSearchQuery, resolveTitle } from '../web/src/lib/metricQuery.ts';

const vault = [
  { name: 'vault_cluster', prefix: 'vault_cluster', default: '*' },
  { name: 'logs.auth.display_name', prefix: '@auth.display_name', default: '*' },
  { name: 'logs.request.path', prefix: '@request.path', default: '*' },
];

test('a search applies the variables it names, in place, and drops those set to *', () => {
  const query = 'source:vault $logs.auth.display_name $logs.request.path $vault_cluster';
  assert.equal(resolveSearchQuery(query, vault, {}), 'source:vault');
  assert.equal(resolveSearchQuery(query, vault, { 'logs.auth.display_name': 'approle', vault_cluster: 'vault-prod' }), 'source:vault @auth.display_name:approle vault_cluster:vault-prod');
});

test("`@$name` reads the variable as an attribute, and drops with it while it is * (Argo Rollouts' log stream)", () => {
  const rollouts = [{ name: 'argo_rollouts_namespace', prefix: 'argo_rollouts_namespace', default: '*' }];
  const query = 'source:argo_rollouts @$argo_rollouts_namespace ';
  assert.equal(resolveSearchQuery(query, rollouts, {}), 'source:argo_rollouts');
  assert.equal(resolveSearchQuery(query, rollouts, { argo_rollouts_namespace: 'checkout' }), 'source:argo_rollouts @argo_rollouts_namespace:checkout');
});

test("`$name.value` in a search is the value alone, `*` while unset (Istio's log patterns)", () => {
  const istio = [
    { name: 'istiod_image', prefix: 'short_image', default: 'pilot' },
    { name: 'cluster_name', prefix: 'kube_cluster_name', default: '*' },
  ];
  const query = 'source:$istiod_image.value kube_cluster_name:$cluster_name.value ';
  assert.equal(resolveSearchQuery(query, istio, {}), 'source:pilot kube_cluster_name:*');
  assert.equal(resolveSearchQuery(query, istio, { cluster_name: 'shop-prod-eks' }), 'source:pilot kube_cluster_name:shop-prod-eks');
});

test('a search leaves the variables it does not name alone', () => {
  const gatekeeper = [
    { name: 'cluster_name', prefix: 'cluster_name', default: '*' },
    { name: 'kube_namespace', prefix: 'kube_namespace', default: '*' },
  ];
  assert.equal(resolveSearchQuery('source:gatekeeper $cluster_name', gatekeeper, { cluster_name: 'shop-prod', kube_namespace: 'gatekeeper-system' }), 'source:gatekeeper cluster_name:shop-prod');
});

test('a search naming no variable (our own widgets) gets every variable that is set', () => {
  const apm = [
    { name: 'env', prefix: 'env', default: '*' },
    { name: 'service', prefix: 'service', default: '*' },
  ];
  assert.equal(resolveSearchQuery('status:error', apm, { env: 'prod', service: '*' }), 'status:error env:prod');
});

test('a search value with spaces or colons is quoted', () => {
  assert.equal(resolveSearchQuery('source:vault $logs.request.path', vault, { 'logs.request.path': 'auth/token/lookup self' }), 'source:vault @request.path:"auth/token/lookup self"');
});

test('a longer variable name wins over a shorter one it starts with', () => {
  const vars = [
    { name: 'host', prefix: 'host', default: '*' },
    { name: 'host_group', prefix: 'host_group', default: '*' },
  ];
  assert.equal(resolveSearchQuery('service:web $host_group $host', vars, { host_group: 'edge', host: 'i-0a1b2c3d4e5f60001' }), 'service:web host_group:edge host:i-0a1b2c3d4e5f60001');
});

test('a log query handed to the metric resolver resolves as a search', () => {
  assert.equal(resolveQuery('source:vault $logs.request.path', vault, { 'logs.request.path': 'sys/health' }), 'source:vault @request.path:sys/health');
  assert.equal(resolveQuery('status:critical source:vault', vault, { vault_cluster: 'vault-prod' }), 'status:critical source:vault', 'upstream panels apply only what they name');
});

test('a metric query filters by the tag of a variable whose prefix is a log attribute (@host)', () => {
  const traefik = [{ name: 'host', prefix: '@host', default: '*' }];
  assert.equal(resolveQuery('avg:traefik_mesh.config.reloads.total{$host}', traefik, { host: 'i-0a1b2c3d4e5f60001' }), 'avg:traefik_mesh.config.reloads.total{host:i-0a1b2c3d4e5f60001}');
});

test('boolean tag filters read as our filters: AND as a comma, OR and IN between values of one tag', () => {
  const traefik = [
    { name: 'host', prefix: '@host', default: '*' },
    { name: 'entrypoint', prefix: 'entrypoint', default: '*' },
  ];
  const q = 'sum:traefik_mesh.entrypoint.requests.count{$host AND $entrypoint AND code:4* OR code:5*} by {entrypoint,code}';
  assert.equal(resolveQuery(q, traefik, {}), 'sum:traefik_mesh.entrypoint.requests.count{code:(4* OR 5*)} by {entrypoint,code}');
  assert.equal(resolveQuery(q, traefik, { entrypoint: 'web' }), 'sum:traefik_mesh.entrypoint.requests.count{code:(4* OR 5*),entrypoint:web} by {entrypoint,code}');
  assert.equal(resolveQuery('avg:system.load.1{env IN (prod, qa) AND NOT service:billing}', [], {}), 'avg:system.load.1{env:(prod OR qa),!service:billing}');
  assert.equal(resolveQuery('avg:system.load.1{env NOT IN (qa)}', [], {}), 'avg:system.load.1{!env:(qa)}');
  // A bare tag no series has (cert-manager's `(status:400 OR s)`) adds nothing to an OR.
  assert.equal(
    resolveQuery('sum:cert_manager.http_acme_client.request.count{(status:400 OR s) AND $cluster_name} by {status}', [{ name: 'cluster_name', prefix: 'cluster_name', default: '*' }], {}),
    'sum:cert_manager.http_acme_client.request.count{status:(400)} by {status}',
  );
});

test('an OR between different tags is not one of our filters', () => {
  assert.equal(parseMetricQuery('avg:system.load.1{env:prod OR service:billing}'), null);
});

test('a search reads `$name.value` as the variable’s value, `*` while it is unset (Postgres’ log stream)', () => {
  const vars = [{ name: 'host', prefix: 'host', default: '*' }];
  const query = 'source:postgresql (host:$host.value OR hostname:$host.value)';
  assert.equal(resolveSearchQuery(query, vars, {}), 'source:postgresql (host:* OR hostname:*)');
  assert.equal(resolveSearchQuery(query, vars, { host: 'i-0a1b2c3d4e5f60001' }), 'source:postgresql (host:i-0a1b2c3d4e5f60001 OR hostname:i-0a1b2c3d4e5f60001)');
});

test('a widget title shows the variables it names: `$name` as prefix:value, `$name.value` as the value, * while unset', () => {
  const kafka = [
    { name: 'env', prefix: 'env', default: '*' },
    { name: 'topic', prefix: 'topic', default: '*' },
  ];
  assert.equal(resolveTitle('Kafka Topology for $topic, $env', kafka, {}), 'Kafka Topology for *, *');
  assert.equal(resolveTitle('Kafka Topology for $topic, $env', kafka, { topic: 'orders.created', env: 'prod' }), 'Kafka Topology for topic:orders.created, env:prod');
  assert.equal(resolveTitle('Lag of $topic.value', kafka, { topic: 'orders.created' }), 'Lag of orders.created');
  assert.equal(resolveTitle('Cost in $ of $unknown', kafka, {}), 'Cost in $ of $unknown', 'not a variable of the dashboard');
});
