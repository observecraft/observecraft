// The countries of the geomap widget: Natural Earth's 1:110m admin-0 countries (public domain,
// https://www.naturalearthdata.com), reduced to each country's name and ISO 3166-1 alpha-2 code (the codes
// Datadog's geoip attributes carry) with coordinates rounded to 0.01°, without Antarctica.
// Run: node scripts/build-world-map.mjs [ne_110m_admin_0_countries.geojson]   (downloads it when not given)
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';

const SOURCE = 'https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson/ne_110m_admin_0_countries.geojson';

const round = (c) => (typeof c[0] === 'number' ? c.map((n) => Math.round(n * 100) / 100) : c.map(round));

const input = process.argv[2] ? readFileSync(process.argv[2], 'utf8') : await (await fetch(SOURCE)).text();
const source = JSON.parse(input);
// Antarctica is left out, as Datadog's geomap does: it would take the bottom of every map.
const features = source.features
  .filter((f) => f.properties.ISO_A2 !== 'AQ')
  .map((f) => {
    // ISO_A2 is -99 for a few countries (France, Norway): ISO_A2_EH has their code.
    const iso = [f.properties.ISO_A2, f.properties.ISO_A2_EH].find((c) => /^[A-Z]{2}$/.test(c ?? '')) ?? '';
    return { type: 'Feature', properties: { name: f.properties.NAME, iso }, geometry: { type: f.geometry.type, coordinates: round(f.geometry.coordinates) } };
  });
const out = new URL('../public/maps/', import.meta.url);
mkdirSync(out, { recursive: true });
writeFileSync(new URL('world.json', out), JSON.stringify({ type: 'FeatureCollection', source: 'Natural Earth 1:110m admin-0 countries (public domain), naturalearthdata.com', features }) + '\n');
console.log(`${features.length} countries`);
