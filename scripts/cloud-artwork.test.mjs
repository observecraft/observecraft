// The cloud overview dashboards' artwork (web/scripts/build-cloud-artwork.mjs): banners and service tiles.
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import test from 'node:test';
import { ARTWORK } from '../web/scripts/build-cloud-artwork.mjs';

const file = (name) => new URL(`../web/public/images/integrations/${name}`, import.meta.url);

test('every banner and tile is a generated SVG with a viewBox and its provenance', () => {
  assert.ok(ARTWORK.length >= 3 + 18, 'three banners and six tiles per cloud');
  for (const { file: name } of ARTWORK) {
    assert.ok(existsSync(file(name)), `${name} is generated (npm run artwork)`);
    const svg = readFileSync(file(name), 'utf8');
    assert.match(svg, /^<svg xmlns="http:\/\/www\.w3\.org\/2000\/svg" viewBox="0 0 \d+ \d+"/, name);
    assert.match(svg, /<!-- ObserveCraft artwork\./, `${name} says where it comes from`);
    assert.ok(svg.trimEnd().endsWith('</svg>'), name);
    assert.equal((svg.match(/<svg/g) ?? []).length, 1, `${name} has no nested documents`);
  }
});

test('the artwork is up to date with its generator', () => {
  for (const { file: name, svg } of ARTWORK) assert.equal(readFileSync(file(name), 'utf8'), svg, `${name}: run npm run artwork`);
});

test('banners and tiles name the provider’s trademarks they show', () => {
  for (const { file: name, svg } of ARTWORK.filter((a) => a.logos.length)) {
    assert.match(svg, /SVG Logos, CC0/, name);
    assert.match(svg, /trademark/, name);
  }
});
