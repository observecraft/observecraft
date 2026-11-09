// What a catalog tile shows when hovered (web/src/pages/integrations/cardHover.ts).
import assert from 'node:assert/strict';
import test from 'node:test';
import { cardHover } from '../web/src/pages/integrations/cardHover.ts';

test('installed integrations are configured, under a status band', () => {
  assert.deepEqual(cardHover('installed'), { band: 'Installed', action: 'configure' });
  assert.deepEqual(cardHover('warning'), { band: 'Missing data', action: 'configure' });
});

test('integrations not installed yet are installed, detected ones say so', () => {
  assert.deepEqual(cardHover('detected'), { band: 'Detected', action: 'install' });
  assert.deepEqual(cardHover('available'), { band: null, action: 'install' });
});

test('an installed tile with no data yet says so, and opens its configuration', () => {
  assert.deepEqual(cardHover('installed', true), { band: 'Installed · Waiting for data', action: 'configure' });
  assert.deepEqual(cardHover('installed', false), { band: 'Installed', action: 'configure' });
});
