import assert from 'node:assert/strict';
import test from 'node:test';
import { formatMetricValue } from '../web/src/api/integrations/units.ts';
test('physical units have explicit CPU, memory and throughput labels', () => {
  assert.equal(formatMetricValue(2600000000, 'n{core}'), '2.6 cores');
  assert.equal(formatMetricValue(500, 'millicore'), '500 mCPU');
  assert.equal(formatMetricValue(5300000000, 'By'), '4.94 GiB');
  assert.equal(formatMetricValue(353000, 'By/s'), '344.73 KiB/s');
  assert.equal(formatMetricValue(0, 'By/s'), '0 B/s');
  assert.equal(formatMetricValue(-1024, 'By'), '-1 KiB');
  assert.equal(formatMetricValue(1, 'GiBy'), '1 GiB');
});
