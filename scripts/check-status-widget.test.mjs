// Datadog's check status widget over our service checks: the `service_check.status` gauge (0 OK, 1 WARNING,
// 2 CRITICAL, 3 UNKNOWN) tagged `check:<name>`, one series per check run (host and tags).
import assert from 'node:assert/strict';
import test from 'node:test';
import { checkStatusQuery, checkStatusTiles } from '../web/src/api/integrations/checkStatus.ts';

const keep = (q) => q;

test('a cluster check reads every run of the check, per host', () => {
  assert.equal(
    checkStatusQuery({ type: 'check_status', check: 'gatekeeper.health', grouping: 'cluster', group_by: [], tags: [] }, keep),
    'max:service_check.status{check:gatekeeper.health} by {host}',
  );
});

test('tags filter the runs; * and template variables in them resolve like any filter', () => {
  const def = { type: 'check_status', check: 'vault.unsealed', grouping: 'cluster', group_by: [], tags: ['*', '$vault_cluster', '$api_url'] };
  assert.equal(checkStatusQuery(def, keep), 'max:service_check.status{check:vault.unsealed,$vault_cluster,$api_url} by {host}');
});

test('group_by splits the cluster into a status per tag value; variables written there filter instead', () => {
  const traefik = { type: 'check_status', check: 'traefik_mesh.controller.ready', grouping: 'cluster', group_by: ['controller_endpoint', 'host'], tags: [] };
  assert.equal(checkStatusQuery(traefik, keep), 'max:service_check.status{check:traefik_mesh.controller.ready} by {host,controller_endpoint}');
  const vault = { type: 'check_status', check: 'vault.initialized', grouping: 'cluster', group_by: ['$vault_cluster', '$api_url'], tags: ['*'] };
  assert.equal(checkStatusQuery(vault, keep), 'max:service_check.status{check:vault.initialized,$vault_cluster,$api_url} by {host}');
});

test('a host in group_by is a tile per host', () => {
  const tiles = checkStatusTiles({ grouping: 'cluster', group_by: ['controller_endpoint', 'host'] }, [
    series('host:a, controller_endpoint:mesh:9000', 0),
    series('host:b, controller_endpoint:mesh:9000', 1),
  ]);
  assert.deepEqual(
    tiles.map((t) => t.label),
    ['controller_endpoint:mesh:9000, host:b', 'controller_endpoint:mesh:9000, host:a'],
  );
});

test('a check grouping reads the one group it names', () => {
  const def = { type: 'check_status', check: 'vault.can_connect', grouping: 'check', group: 'host:i-0a1b2c3d4e5f60001,api_url:http://vault:8200', tags: [] };
  assert.equal(checkStatusQuery(def, keep), 'max:service_check.status{check:vault.can_connect,host:i-0a1b2c3d4e5f60001,api_url:http://vault:8200} by {host}');
});

const series = (name, status) => ({
  name,
  points: [
    [0, 0],
    [60_000, status],
  ],
});

test('a cluster shows its worst status and how many runs are in each', () => {
  const tiles = checkStatusTiles({ grouping: 'cluster', group_by: [] }, [series('host:a', 0), series('host:b', 1), series('host:c', 0)]);
  assert.deepEqual(tiles, [{ label: '', status: 1, counts: [2, 1, 0, 0] }]);
  assert.equal(checkStatusTiles({ grouping: 'cluster', group_by: [] }, [series('host:a', 3), series('host:b', 2)])[0].status, 2, 'critical is worse than unknown');
  assert.equal(checkStatusTiles({ grouping: 'cluster', group_by: [] }, [series('host:a', 0), series('host:b', 3)])[0].status, 3, 'unknown is worse than OK');
});

test('a cluster split by group_by has a tile per value, worst first', () => {
  const tiles = checkStatusTiles({ grouping: 'cluster', group_by: ['endpoint'] }, [
    series('host:a, endpoint:http://mesh-a:8080/metrics', 0),
    series('host:b, endpoint:http://mesh-a:8080/metrics', 0),
    series('host:a, endpoint:http://mesh-b:8080/metrics', 2),
  ]);
  assert.deepEqual(tiles, [
    { label: 'endpoint:http://mesh-b:8080/metrics', status: 2, counts: [0, 0, 1, 0] },
    { label: 'endpoint:http://mesh-a:8080/metrics', status: 0, counts: [2, 0, 0, 0] },
  ]);
});

test('no runs, no tiles', () => {
  assert.deepEqual(checkStatusTiles({ grouping: 'cluster', group_by: [] }, []), []);
});
