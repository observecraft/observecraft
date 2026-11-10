// Template variables in the searches of dashboard widgets (logs, signals, events, monitors) and in metric queries,
// as Datadog applies them: each `$name` where it is written, `prefix:value`, dropped while it is `*`.
import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveQuery, resolveSearchQuery } from '../web/src/lib/metricQuery.ts';

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
