// Searching Logs → Pipelines → library (web/src/pages/logs/libraryFilter.ts).
import assert from 'node:assert/strict';
import test from 'node:test';
import { filterLibrary } from '../web/src/pages/logs/libraryFilter.ts';

const lib = [
  { id: 'nginx', name: 'Nginx', filter: 'source:nginx' },
  { id: 'cisco_asa', name: 'Cisco ASA', filter: 'source:cisco-asa' },
  { id: 'postgres', name: 'Postgresql', filter: 'source:postgresql' },
];

test('the library filters by name or source, ignoring case', () => {
  assert.deepEqual(
    filterLibrary(lib, 'CISCO').map((p) => p.id),
    ['cisco_asa'],
  );
  assert.deepEqual(
    filterLibrary(lib, 'source:postgres').map((p) => p.id),
    ['postgres'],
  );
  assert.deepEqual(
    filterLibrary(lib, '  ').map((p) => p.id),
    ['nginx', 'cisco_asa', 'postgres'],
  );
});
