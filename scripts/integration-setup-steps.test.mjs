// The "Sending to ObserveCraft" steps of the Configure tab (web/src/api/integrations/setupSteps.ts, mock and live mode):
// tools that send their logs through a local Datadog Agent say how, in our words.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
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

test('JMX checks (Kafka, Confluent Platform, …) say the Agent needs its JMX image and the JVM its JMX port', () => {
  const specs = new URL('../web/src/api/integrations/agentSpecs/', import.meta.url);
  const jmx = catalog.filter(
    (c) =>
      c.collection === 'agent' &&
      /"name":"is_jmx"/.test(
        (() => {
          try {
            return readFileSync(new URL(`${c.check}.json`, specs), 'utf8');
          } catch {
            return '';
          }
        })(),
      ),
  );
  assert.ok(jmx.some((c) => c.id === 'kafka') && jmx.some((c) => c.id === 'confluent-platform'), 'kafka and confluent_platform are JMX checks');
  for (const c of jmx) assert.match(coreSteps(c).join('\n'), /JMX image/, c.id);
  assert.doesNotMatch(stepsOf('kafka-consumer'), /JMX image/, 'kafka_consumer reads the brokers over the Kafka protocol');
});

// The tiles' details as the UI builds them (live and mock mode): bundled, as the web build does.
const require = createRequire(new URL('../web/package.json', import.meta.url));
const { outputFiles } = require('esbuild').buildSync({
  stdin: { contents: "export { integrationMetadata, getIntegration } from './api/mock/integrations';", resolveDir: fileURLToPath(new URL('../web/src', import.meta.url)), loader: 'ts' },
  bundle: true,
  platform: 'node',
  format: 'esm',
  write: false,
  // Vite's lazy imports (agent specs, details) aren't needed here.
  define: { 'import.meta.env': '{}', 'import.meta.glob': 'globalThis.__noGlob' },
  logLevel: 'error',
});
globalThis.__noGlob = () => ({});
const { integrationMetadata, getIntegration } = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString('base64')}`);

test('a curated OpenTelemetry tile linked to an Agent check (Kafka) gives the Agent steps first, then the Collector alternative', async () => {
  for (const steps of [integrationMetadata('kafka').setupSteps, (await getIntegration('kafka')).setupSteps].map((s) => s.join('\n'))) {
    assert.match(steps, /dd_url/);
    assert.match(steps, /conf\.d\/kafka\.d\/conf\.yaml/);
    assert.match(steps, /JMX image/);
    assert.match(steps, /OpenTelemetry Collector/);
    assert.match(steps, /kafkametrics/);
  }
});

test('no tile promises a fleet manager deploying collector configuration (there is none)', async () => {
  for (const id of ['kafka', 'mongodb', 'rabbitmq']) assert.doesNotMatch((await getIntegration(id)).setupSteps.join('\n'), /fleet manager/i, id);
});
