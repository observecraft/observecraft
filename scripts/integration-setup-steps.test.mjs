// The "Sending to ObserveCraft" steps of the Configure tab (web/src/api/integrations/setupSteps.ts, mock and live mode):
// tools that send their logs through a local Datadog Agent say how, in our words.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { coreSteps } from '../web/src/api/integrations/setupSteps.ts';

const catalog = JSON.parse(readFileSync(new URL('../web/src/api/integrations/catalog.generated.json', import.meta.url), 'utf8')).integrations;
const stepsOf = (id) => coreSteps(catalog.find((c) => c.id === id)).join('\n');

test('Supply Chain Firewall: through the local Agent pointed at ObserveCraft, not its API key option', () => {
  const steps = stepsOf('supply-chain-firewall');
  assert.match(steps, /logs_dd_url/);
  assert.match(steps, /scfw configure/);
  assert.match(steps, /conf\.d\/scfw\.d/);
  assert.match(steps, /API key/, 'says the API key option sends to Datadog only');
  assert.match(steps, /version 4|v4|4\.x/i, 'says what the Go rewrite does');
  assert.doesNotMatch(steps, /integrations-core documentation/, 'not the generic Agent check steps');
});

test('IDE Shepherd: the extension configures the local Agent', () => {
  const steps = stepsOf('ide-shepherd');
  assert.match(steps, /logs_dd_url/);
  assert.match(steps, /Telemetry/);
  assert.match(steps, /conf\.d\/ide-shepherd\.d/);
});

test('GuardDog, an Agent check, keeps the check steps and needs log collection', () => {
  const steps = stepsOf('guarddog');
  assert.match(steps, /conf\.d\/guarddog\.d\/conf\.yaml/);
  assert.match(steps, /logs_enabled/);
});

test('the Agent steps point every signal at the intake, never at a Datadog site', () => {
  for (const c of catalog.filter((c) => ['agent', 'logs'].includes(c.collection))) {
    const steps = coreSteps(c).join('\n');
    assert.match(steps, /dd_url/, c.id);
    assert.doesNotMatch(steps, /DD_SITE/, c.id);
  }
});

test('PostgreSQL: the check’s user, and Database Monitoring said to be unsupported (and kept off)', () => {
  const steps = stepsOf('postgres');
  assert.match(steps, /conf\.d\/postgres\.d\/conf\.yaml/);
  assert.match(steps, /pg_monitor/);
  assert.match(steps, /Database Monitoring/);
  assert.match(steps, /dbm: false|leave dbm off|dbm off/i);
  assert.match(steps, /not supported|isn't supported|does not receive/i);
});

test('Redis: the redisdb check, Valkey included', () => {
  const steps = stepsOf('redis');
  assert.match(steps, /conf\.d\/redisdb\.d\/conf\.yaml/);
  assert.match(steps, /Valkey/);
});

test('ClickHouse: the HTTP interface and the system tables, no Database Monitoring', () => {
  assert.match(stepsOf('clickhouse'), /8123/);
  assert.match(stepsOf('clickhouse'), /system/);
  assert.match(stepsOf('clickhouse'), /Leave dbm off/);
});

test('PgBouncer: the admin console with a stats user', () => {
  assert.match(stepsOf('pgbouncer'), /stats_users/);
  assert.match(stepsOf('pgbouncer'), /database_url|pgbouncer database/);
});
