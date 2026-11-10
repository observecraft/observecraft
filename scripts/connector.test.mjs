// Connector packages' built files are current: dist/dashboards.json is what `node scripts/connector.mjs build` makes
// of assets/dashboards, the web's mock catalog lists every bundled package, and NetBird's dashboard asset is what
// scripts/netbird-dashboard.mjs writes.
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { basename, join } from 'node:path';
import test from 'node:test';
import { buildDashboards, bundledPackages, readManifest } from './connector.mjs';
import { datadogDashboard } from './netbird-dashboard.mjs';

for (const dir of bundledPackages()) {
  test(`${basename(dir)}: dist/dashboards.json is built from its assets`, () => {
    const built = buildDashboards(dir);
    const dist = join(dir, 'dist/dashboards.json');
    if (built.length === 0) return assert.ok(!existsSync(dist) || JSON.parse(readFileSync(dist, 'utf8')).length === 0);
    assert.deepEqual(JSON.parse(readFileSync(dist, 'utf8')), built, 'run node scripts/connector.mjs build');
    const m = readManifest(dir);
    for (const d of built) assert.equal(d.logo, m.logo ?? m.tiles?.[0] ?? m.id, 'gated by the package logo');
  });
}

test('the web mock catalog lists every bundled package', () => {
  const generated = JSON.parse(readFileSync(new URL('../web/src/api/connectors/packages.generated.json', import.meta.url), 'utf8'));
  assert.deepEqual(
    generated.map((p) => p.entry.id).sort(),
    bundledPackages()
      .map((d) => readManifest(d).id)
      .sort(),
    'run node scripts/connector.mjs build',
  );
});

test("NetBird's dashboard asset is the script's", () => {
  const asset = JSON.parse(readFileSync(new URL('../integrations/connectors/netbird/assets/dashboards/overview.json', import.meta.url), 'utf8'));
  assert.deepEqual(asset, datadogDashboard(), 'run node scripts/netbird-dashboard.mjs');
});
