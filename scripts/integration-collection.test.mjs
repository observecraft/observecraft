// How each catalog tile is collected and what the catalog offers for it (web/src/api/integrations/collection.ts,
// catalogFilter.ts, pages/integrations/cardHover.ts).
import assert from 'node:assert/strict';
import test from 'node:test';
import { availableHere, installable, ownIntegration, unavailableReason, webhookUrl } from '../web/src/api/integrations/collection.ts';
import { filterCatalog, showsUnavailable } from '../web/src/api/integrations/catalogFilter.ts';
import { cardHover } from '../web/src/pages/integrations/cardHover.ts';

test('Agent checks, log integrations, webhooks and senders that take any URL work with ObserveCraft', () => {
  for (const collection of ['agent', 'logs', 'webhook']) assert.equal(availableHere({ collection }), true, collection);
  assert.equal(availableHere({ collection: 'push-site', customEndpoint: true }), true);
  for (const collection of ['crawler', 'oauth', 'docs']) assert.equal(availableHere({ collection }), false, collection);
  assert.equal(availableHere({ collection: 'push-site' }), false, 'only sends to Datadog sites');
  assert.equal(availableHere({}), true, 'our own entries (OpenTelemetry templates, clouds)');
});

test('what Datadog collects with its own service is not installable, as the server refuses it', () => {
  for (const collection of ['crawler', 'oauth', 'docs']) assert.equal(installable(collection), false, collection);
  for (const collection of ['agent', 'logs', 'webhook', 'push-site', undefined]) assert.equal(installable(collection), true, String(collection));
});

test('an unavailable tile says why in one line', () => {
  assert.equal(unavailableReason('Asana', { collection: 'crawler' }), "Datadog collects this by calling Asana's API from its own service.");
  assert.match(unavailableReason('Vercel', { collection: 'oauth' }), /OAuth/);
  assert.match(unavailableReason('GKE', { collection: 'docs' }), /documentation/);
  assert.match(unavailableReason('Auth0', { collection: 'push-site' }), /Auth0 sends .* only to Datadog's own sites/);
});

test('a webhook posts its logs to the intake with the ingest key and its log source', () => {
  assert.equal(webhookUrl('https://intake.example.com:4318/', 'adyen'), 'https://intake.example.com:4318/api/v2/logs?dd-api-key=<INGEST_API_KEY>&ddsource=adyen');
});

test('tiles that ObserveCraft covers with its own integration link to it', () => {
  assert.deepEqual(ownIntegration('gke'), { to: '/integrations/kubernetes', label: 'Kubernetes' });
  assert.deepEqual(ownIntegration('slack'), { to: '/integrations/slack', label: 'Slack' });
  assert.equal(ownIntegration('asana'), undefined);
});

test('unavailable tiles are left out of the default listing, and found by search or their status', () => {
  const tile = (id, status) => ({ id, name: id, publisher: 'Datadog', logo: id, status, categories: ['Security'], description: '' });
  const items = [tile('asana', 'unavailable'), tile('redis', 'available')];
  const all = { query: '', status: 'all', category: null };
  assert.equal(showsUnavailable(all), false);
  assert.equal(showsUnavailable({ ...all, query: 'asa' }), true);
  assert.equal(showsUnavailable({ ...all, status: 'unavailable' }), true);
  assert.deepEqual(
    filterCatalog(items, { ...all, status: 'unavailable' }).map((s) => s.id),
    ['asana'],
  );
  assert.deepEqual(
    filterCatalog(items, { ...all, status: 'available' }).map((s) => s.id),
    ['redis'],
  );
});

test('an unavailable tile offers its details, not Install', () => {
  assert.deepEqual(cardHover('unavailable'), { band: 'Not available', action: 'details' });
});
