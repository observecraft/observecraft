import assert from 'node:assert/strict';
import test from 'node:test';

const { recipientLogo, TOOL_LOGOS } = await import('../web/src/lib/channelLogos.ts');
const { BRAND_LOGOS } = await import('../web/src/assets/brandLogos.generated.ts');

test('notification recipients map to their brand logo', () => {
  assert.equal(recipientLogo('slack-checkout-alerts'), 'slack');
  assert.equal(recipientLogo('pagerduty-payments'), 'pagerduty');
  assert.equal(recipientLogo('opsgenie-platform'), 'opsgenie');
  assert.equal(recipientLogo('webhook-incident-bot'), 'webhook');
  assert.equal(recipientLogo('teams-sre'), 'teams');
  assert.equal(recipientLogo('jane@example.com'), null);
  assert.equal(recipientLogo('team-checkout'), null);
});

test('every tool logo the app names exists in the generated logo set', () => {
  for (const key of [...new Set([...TOOL_LOGOS, 'slack', 'pagerduty', 'opsgenie', 'webhook', 'teams'])]) assert.ok(BRAND_LOGOS[key], `missing logo ${key}`);
});
