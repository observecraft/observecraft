import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';

const { ALIASES, loadLibraries, matchIntegrationLogo } = await import('../web/scripts/integrationLogoMatch.mjs');
const { INTEGRATION_LOGOS } = await import('../web/src/assets/integrationLogos.generated.ts');
const { GENERIC_LOGOS } = await import('../web/src/pages/integrations/genericLogos.ts');
const catalog = JSON.parse(readFileSync(new URL('../web/src/api/integrations/catalog.generated.json', import.meta.url))).integrations;
const libraries = loadLibraries();
const match = (id) => {
  const integration = catalog.find((c) => c.id === id);
  assert.ok(integration, `${id} is not in the catalog`);
  const m = matchIntegrationLogo(integration, libraries);
  return m && `${m.source}:${m.name}`;
};

test('integrations find their brand logo, full colour first', () => {
  assert.equal(match('redis'), 'logos:redis');
  assert.equal(match('mongodb'), 'logos:mongodb-icon');
  assert.equal(match('kafka'), 'logos:kafka-icon');
  assert.equal(match('flink'), 'logos:apache-flink-icon');
  assert.equal(match('scylla'), 'simple-icons:scylladb');
});

test('brands named differently are reached through aliases', () => {
  assert.equal(match('postgres'), 'logos:postgresql');
  assert.equal(match('kafka-consumer'), 'logos:kafka-icon');
  assert.equal(match('cockroachdb'), 'simple-icons:cockroachlabs');
  assert.equal(match('vsphere'), 'logos:vmware');
  assert.equal(match('hugging-face-tgi'), 'logos:hugging-face-icon');
  // Hive is Apache Hive, not the blockchain of the same name.
  assert.equal(match('hive'), 'simple-icons:apachehive');
  const ids = new Set(catalog.map((c) => c.id));
  for (const id of Object.keys(ALIASES)) assert.ok(ids.has(id), `alias for unknown integration ${id}`);
});

test("a vendor's products take the vendor's logo", () => {
  assert.equal(match('cisco-asa'), 'simple-icons:cisco');
  assert.equal(match('ibm-mq'), 'logos:ibm');
});

test('a shared word is not a brand', () => {
  // "Cloud Foundry" is not Google Cloud; generic checks are not brands.
  assert.equal(match('cloud-foundry-api'), 'simple-icons:cloudfoundry');
  for (const id of ['disk', 'battery', 'directory', 'container', 'process']) assert.equal(match(id), null, id);
  // Obsidian Security is not the Obsidian notes app; Prisma Cloud (Palo Alto Networks) is not the Prisma ORM.
  assert.equal(match('obsidian-security'), null);
  assert.equal(match('twistlock'), 'simple-icons:paloaltonetworks');
});

test("Datadog's logo is not ours to show", () => {
  for (const [id, logo] of Object.entries(INTEGRATION_LOGOS)) assert.ok(!/datadog/i.test(logo.body), `${id} draws Datadog's logo`);
  for (const id of catalog.map((c) => c.id).filter((id) => id.startsWith('datadog-'))) assert.equal(match(id), null, id);
});

test('the generated logos cover the catalog, and system checks have an icon', () => {
  const ids = new Set(catalog.map((c) => c.id));
  for (const id of Object.keys(GENERIC_LOGOS)) assert.ok(ids.has(id), `generic icon for unknown integration ${id}`);
  for (const id of Object.keys(INTEGRATION_LOGOS)) assert.ok(ids.has(id), `logo for unknown integration ${id}`);
  for (const [id, logo] of Object.entries(INTEGRATION_LOGOS)) assert.ok(logo.body.length > 0 && logo.width > 0 && logo.height > 0, id);
  const covered = catalog.filter((c) => INTEGRATION_LOGOS[c.id] || GENERIC_LOGOS[c.id]).length;
  assert.ok(covered >= 260, `${covered} of ${catalog.length} integrations have a logo`);
  console.log(`${covered} of ${catalog.length} integrations have a logo`);
});
