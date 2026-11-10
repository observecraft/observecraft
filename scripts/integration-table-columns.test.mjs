// Table widgets written as Datadog's newer table requests (`request_type: table`): their rows' and compute columns'
// aliases and formats (External Secrets' "ExternalSecrets With Errors").
import assert from 'node:assert/strict';
import test from 'node:test';
import { tableColumns } from '../web/src/api/integrations/tableColumns.ts';

test('a table request names its group columns and compute columns by their aliases, with their formats', () => {
  const request = {
    request_type: 'table',
    queries: [{ name: 'query1', data_source: 'metrics', metric_name: 'external_secrets.externalsecret.status_condition', filter: 'condition:ready,status:false' }],
    rows: {
      group_by: [
        { name: 'groupBy0', group_keys: [{ query: 'query1', group_key: 'kube_cluster_name' }], alias: 'Cluster' },
        { name: 'groupBy1', group_keys: [{ query: 'query1', group_key: 'name' }] },
      ],
    },
    columns: [
      { name: 'column1', type: 'compute', compute: { query: 'query1', aggregation: 'max' }, format: { conditional_formats: [{ comparator: '>', value: 0, palette: 'white_on_red' }] }, alias: 'Error' },
    ],
  };
  const { tags, computes } = tableColumns(request);
  assert.deepEqual([...tags], [['kube_cluster_name', 'Cluster']]);
  assert.deepEqual(computes.get('query1'), { label: 'Error', conditional_formats: [{ comparator: '>', value: 0, palette: 'white_on_red' }] });
  assert.deepEqual(tableColumns({ formulas: [{ formula: 'query1' }] }), { tags: new Map(), computes: new Map() });
});
