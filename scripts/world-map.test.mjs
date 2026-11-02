// The world map of the geomap widget (web/scripts/build-world-map.mjs, Natural Earth, public domain).
import assert from 'node:assert/strict';
import { readFileSync, statSync } from 'node:fs';
import test from 'node:test';

const file = new URL('../web/public/maps/world.json', import.meta.url);

test('every country carries its ISO 3166-1 alpha-2 code, France and Norway included', () => {
  const map = JSON.parse(readFileSync(file, 'utf8'));
  const iso = new Set(map.features.map((f) => f.properties.iso));
  for (const code of ['US', 'FR', 'NO', 'DE', 'RO', 'BR', 'IN', 'JP', 'AU']) assert.ok(iso.has(code), code);
  assert.ok(!iso.has('AQ'), 'Antarctica is left out, as Datadog’s geomap does');
  for (const f of map.features) assert.deepEqual(Object.keys(f.properties).sort(), ['iso', 'name'], f.properties.name);
  assert.match(JSON.stringify(map), /Natural Earth/, 'says where it comes from');
});

test('the map stays small: it loads with the first geomap', () => {
  assert.ok(statSync(file).size < 350_000, `${statSync(file).size} bytes`);
});
